//! Password-protected documents: opened with the right password, edited, saved encrypted again with the same
//! passwords, and checked with an independent reader (poppler's `pdftotext -upw/-opw`) and by looking at the
//! bytes (nothing readable is left in an encrypted file).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lopdf::encryption::crypt_filters::{
    Aes128CryptFilter, Aes256CryptFilter, CryptFilter, Rc4CryptFilter,
};
use lopdf::{Document, EncryptionState, EncryptionVersion, Object, Permissions};
use pdf_engine::EngineError;
use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::pageops;
use pdf_engine::protect::{Cipher, Rights};
use std::collections::BTreeMap;
use std::process::Command;
use std::sync::Arc;
use test_support::fixtures::{helvetica_lines, signature_structure_only};
use test_support::*;

const USER: &str = "open-sesame";
const OWNER: &str = "boss-key";
const WORDS: &str = "Second line of text";

#[derive(Clone, Copy, Debug)]
enum Method {
    Rc4_40,
    Rc4_128,
    Aes128,
    Aes256,
}

/// The fixture, encrypted with `method`. `permissions` limits what the user password allows.
fn encrypted(method: Method, user: &str, owner: &str, permissions: Permissions) -> Vec<u8> {
    encrypt_bytes(&helvetica_lines(), method, user, owner, permissions)
}

fn encrypt_bytes(
    plain: &[u8],
    method: Method,
    user: &str,
    owner: &str,
    permissions: Permissions,
) -> Vec<u8> {
    let mut doc = Document::load_mem(plain).unwrap();
    // Encrypted files always carry a file identifier (the older key derivations use it).
    let id = Object::string_literal(b"0123456789abcdef".to_vec());
    doc.trailer.set("ID", Object::Array(vec![id.clone(), id]));
    let state = match method {
        Method::Rc4_40 => EncryptionState::try_from(EncryptionVersion::V1 {
            document: &doc,
            owner_password: owner,
            user_password: user,
            permissions,
        }),
        Method::Rc4_128 => EncryptionState::try_from(EncryptionVersion::V2 {
            document: &doc,
            owner_password: owner,
            user_password: user,
            key_length: 128,
            permissions,
        }),
        Method::Aes128 => {
            let f: Arc<dyn CryptFilter> = Arc::new(Aes128CryptFilter);
            EncryptionState::try_from(EncryptionVersion::V4 {
                document: &doc,
                encrypt_metadata: true,
                crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), f)]),
                stream_filter: b"StdCF".to_vec(),
                string_filter: b"StdCF".to_vec(),
                owner_password: owner,
                user_password: user,
                permissions,
            })
        }
        Method::Aes256 => {
            let f: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
            EncryptionState::try_from(EncryptionVersion::V5 {
                encrypt_metadata: true,
                crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), f)]),
                file_encryption_key: &[9u8; 32],
                stream_filter: b"StdCF".to_vec(),
                string_filter: b"StdCF".to_vec(),
                owner_password: owner,
                user_password: user,
                permissions,
            })
        }
    }
    .unwrap();
    doc.encrypt(&state).unwrap();
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

fn open_with(bytes: &[u8], password: &str) -> Result<PdfDocument, EngineError> {
    PdfDocument::open_with_password(bytes.to_vec(), &OpenOptions::default(), password)
}

fn contains(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

/// Text of page 1 as an independent reader sees it: `pdftotext` with a password.
fn pdftotext(bytes: &[u8], pw_flag: &str, password: &str) -> Option<String> {
    if !have_tool("pdftotext") {
        assert!(
            !oracles_required(),
            "poppler is required for this check (BERG_REQUIRE_ORACLES=1)"
        );
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("in.pdf");
    std::fs::write(&p, bytes).unwrap();
    let out = Command::new("pdftotext")
        .args(if pw_flag.is_empty() {
            vec!["-enc", "UTF-8"]
        } else {
            vec![pw_flag, password, "-enc", "UTF-8"]
        })
        .arg(&p)
        .arg("-")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "pdftotext failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn the_fixture_really_is_encrypted_and_readable_only_with_the_password() {
    for m in [
        Method::Rc4_40,
        Method::Rc4_128,
        Method::Aes128,
        Method::Aes256,
    ] {
        let bytes = encrypted(m, USER, OWNER, Permissions::all());
        assert!(contains(&helvetica_lines(), WORDS), "fixture sanity");
        assert!(!contains(&bytes, WORDS), "{m:?}: text is readable in the file");
        assert!(matches!(
            open_with(&bytes, ""),
            Err(EngineError::PasswordRequired)
        ));
        assert!(
            matches!(open_with(&bytes, "wrong"), Err(EngineError::WrongPassword)),
            "{m:?}"
        );
        for (pw, owner) in [(USER, false), (OWNER, true)] {
            let doc = open_with(&bytes, pw).unwrap_or_else(|e| panic!("{m:?} {pw}: {e}"));
            let info = doc.protection().expect("protected");
            assert_eq!(info.owner, owner, "{m:?} opened with {pw}");
            assert_eq!(doc.page_count(), 1);
            assert!(doc.capabilities().encrypted);
            // Whichever password opened it, the text came out right (a wrong file key gives garbage).
            if let Some(t) = pdftotext(doc.original_bytes(), "", "") {
                assert!(t.contains(WORDS), "{m:?} opened with {pw}: {t:?}");
            }
        }
    }
}

#[test]
fn the_cipher_is_reported() {
    let cases = [
        (Method::Rc4_40, Cipher::Rc4_40),
        (Method::Rc4_128, Cipher::Rc4),
        (Method::Aes128, Cipher::Aes128),
        (Method::Aes256, Cipher::Aes256),
    ];
    for (m, c) in cases {
        let doc = open_with(&encrypted(m, USER, OWNER, Permissions::all()), USER).unwrap();
        assert_eq!(doc.protection().unwrap().cipher, c, "{m:?}");
    }
    let _ = Rc4CryptFilter; // (the RC4 filters are what V1/V2 use internally)
}

#[test]
fn edit_save_encrypted_again_and_read_it_with_another_reader() {
    for m in [
        Method::Rc4_40,
        Method::Rc4_128,
        Method::Aes128,
        Method::Aes256,
    ] {
        let bytes = encrypted(m, USER, OWNER, Permissions::all());
        let mut doc = open_with(&bytes, OWNER).unwrap();
        assert!(doc.capabilities().can_edit);
        doc.transact(|tx| pageops::insert_blank_page(tx, 1, 300.0, 400.0))
            .unwrap();
        let plain = doc.snapshot_bytes().unwrap();
        if let Some(t) = pdftotext(&plain, "", "") {
            assert!(t.contains(WORDS), "the working copy is plain: {t:?}");
        }
        let sealed = doc.seal(&plain).unwrap().into_owned();
        assert!(!contains(&sealed, WORDS), "{m:?}: saved file is not encrypted");
        assert!(sealed.windows(8).any(|w| w == b"/Encrypt"));

        // Both passwords still open it, the edit is there.
        for pw in [USER, OWNER] {
            let again = open_with(&sealed, pw).unwrap_or_else(|e| panic!("{m:?} {pw}: {e}"));
            assert_eq!(again.page_count(), 2, "{m:?}: the edit was lost");
        }
        assert!(matches!(
            open_with(&sealed, "wrong"),
            Err(EngineError::WrongPassword)
        ));

        // An independent reader agrees.
        if let Some(t) = pdftotext(&sealed, "-upw", USER) {
            assert!(t.contains(WORDS), "{m:?} pdftotext(user): {t:?}");
        }
        if let Some(t) = pdftotext(&sealed, "-opw", OWNER) {
            assert!(t.contains(WORDS), "{m:?} pdftotext(owner): {t:?}");
        }
    }
}

#[test]
fn what_the_author_forbids_is_blocked_until_the_owner_password_is_given() {
    let only_print = Permissions::PRINTABLE | Permissions::COPYABLE;
    let bytes = encrypted(Method::Aes128, USER, OWNER, only_print);

    let mut doc = open_with(&bytes, USER).unwrap();
    let info = doc.protection().unwrap();
    assert!(!info.owner);
    assert!(info.rights.print && info.rights.copy);
    assert!(!info.rights.modify && !info.rights.assemble);
    assert!(!doc.capabilities().can_edit);
    assert!(
        doc.capabilities()
            .edit_blocker_summary()
            .contains("owner password")
    );
    assert!(
        doc.transact(|tx| pageops::insert_blank_page(tx, 1, 300.0, 400.0))
            .is_err()
    );
    assert!(doc.remove_protection().is_err());
    assert!(doc.set_protection("a", "b", Rights::ALL).is_err());

    // The owner password unlocks everything.
    let owner_doc = doc
        .unlock_as_owner(OWNER, &OpenOptions::default())
        .unwrap();
    assert!(owner_doc.protection().unwrap().owner);
    assert!(owner_doc.capabilities().can_edit);
    assert!(owner_doc.protection().unwrap().rights.modify);
    // The user password is not the owner password.
    assert!(matches!(
        doc.unlock_as_owner(USER, &OpenOptions::default()),
        Err(EngineError::WrongPassword)
    ));
}

#[test]
fn a_file_with_only_an_owner_password_opens_without_asking() {
    let bytes = encrypted(Method::Aes128, "", OWNER, Permissions::PRINTABLE);
    let doc = PdfDocument::open(bytes.clone(), &OpenOptions::default()).unwrap();
    let info = doc.protection().unwrap();
    assert!(!info.owner, "opened without the owner password");
    assert!(!info.rights.modify);
    assert!(!doc.capabilities().can_edit);
    let owner = open_with(&bytes, OWNER).unwrap();
    assert!(owner.protection().unwrap().owner);
    assert!(owner.capabilities().can_edit);
}

#[test]
fn protection_can_be_removed_and_added() {
    let bytes = encrypted(Method::Rc4_128, USER, OWNER, Permissions::all());
    let mut doc = open_with(&bytes, OWNER).unwrap();
    doc.remove_protection().unwrap();
    assert!(doc.protection().is_none());
    assert!(!doc.capabilities().encrypted);
    let plain = doc.snapshot_bytes().unwrap();
    let out = doc.seal(&plain).unwrap().into_owned();
    assert!(!out.windows(8).any(|w| w == b"/Encrypt"), "no longer encrypted");
    if let Some(t) = pdftotext(&out, "", "") {
        assert!(t.contains(WORDS), "{t:?}");
    }
    assert!(PdfDocument::open(out, &OpenOptions::default()).is_ok());

    // A plain document gets AES-256 protection.
    let mut doc = PdfDocument::open(helvetica_lines(), &OpenOptions::default()).unwrap();
    assert!(doc.protection().is_none());
    let only_view = Rights {
        print: true,
        copy: false,
        modify: false,
        annotate: false,
        fill: false,
        assemble: false,
    };
    doc.set_protection("reader", "", only_view).unwrap();
    assert_eq!(doc.protection().unwrap().cipher, Cipher::Aes256);
    let plain = doc.snapshot_bytes().unwrap();
    let sealed = doc.seal(&plain).unwrap().into_owned();
    assert!(!contains(&sealed, WORDS));
    // The empty owner password became the user password: that opens everything.
    assert!(open_with(&sealed, "reader").unwrap().protection().unwrap().owner);
    assert!(matches!(
        open_with(&sealed, ""),
        Err(EngineError::PasswordRequired)
    ));
    if let Some(t) = pdftotext(&sealed, "-upw", "reader") {
        assert!(t.contains(WORDS), "{t:?}");
    }
    // With a separate owner password the restrictions apply to the user password.
    let mut doc = PdfDocument::open(helvetica_lines(), &OpenOptions::default()).unwrap();
    doc.set_protection("reader", "chief", only_view).unwrap();
    let plain = doc.snapshot_bytes().unwrap();
    let sealed = doc.seal(&plain).unwrap().into_owned();
    let as_reader = open_with(&sealed, "reader").unwrap();
    assert!(!as_reader.protection().unwrap().owner);
    assert!(!as_reader.protection().unwrap().rights.copy);
    assert!(!as_reader.capabilities().can_edit);
    assert!(open_with(&sealed, "chief").unwrap().capabilities().can_edit);
}

#[test]
fn an_unprotected_document_is_written_as_it_is() {
    let doc = PdfDocument::open(helvetica_lines(), &OpenOptions::default()).unwrap();
    let bytes = helvetica_lines();
    assert_eq!(doc.seal(&bytes).unwrap().as_ref(), bytes.as_slice());
    assert!(doc.protection().is_none());
    assert!(matches!(
        doc.unlock_as_owner("x", &OpenOptions::default()),
        Err(EngineError::InvalidArgument(_))
    ));
}

#[test]
fn a_protected_and_signed_document_warns_about_the_signature() {
    let bytes = encrypt_bytes(
        &signature_structure_only(),
        Method::Rc4_128,
        USER,
        OWNER,
        Permissions::all(),
    );
    let opened = open_with(&bytes, OWNER).unwrap();
    assert!(opened.capabilities().has_signatures);
    assert!(
        opened
            .capabilities()
            .warnings
            .iter()
            .any(|w| w.0.contains("no longer verify"))
    );
}
