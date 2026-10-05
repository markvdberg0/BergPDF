//! Digital signatures: created with the `pdf-sign` crate, checked by independent tools
//! (poppler `pdfsig`, OpenSSL `cms -verify`) as well as by our own inspector.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pdf_engine::doc::{OpenOptions, PdfDocument};
use pdf_engine::geom::Rect;
use pdf_engine::sign::{SignOptions, SignatureStatus, list_signatures};
use pdf_sign::Identity;
use test_support::fixtures::*;
use test_support::*;

fn open(bytes: Vec<u8>) -> PdfDocument {
    PdfDocument::open(bytes, &OpenOptions::default()).unwrap()
}

fn identity(file: &str) -> Identity {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../pdf-sign/tests/fixtures")
        .join(file);
    Identity::from_pkcs12(&std::fs::read(p).unwrap(), "test123").unwrap()
}

fn signed(file: &str, opts: &SignOptions) -> (Vec<u8>, Vec<u8>) {
    let original = helvetica_lines();
    let mut doc = open(original.clone());
    let bytes = doc.sign(&identity(file), opts).unwrap();
    // Optional: keep the artefacts for manual inspection (`BERG_DUMP_DIR=/tmp/x cargo test …`).
    if let Ok(dir) = std::env::var("BERG_DUMP_DIR") {
        let name = file.replace(".p12", "");
        std::fs::write(
            std::path::Path::new(&dir).join(format!("signed-{name}.pdf")),
            &bytes,
        )
        .unwrap();
    }
    (original, bytes)
}

/// Cut the CMS blob and the signed content out of signed bytes (independent of our own parser).
fn cms_and_content(bytes: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let d = lopdf::Document::load_mem(bytes).unwrap();
    let v = d
        .objects
        .values()
        .filter_map(|o| o.as_dict().ok())
        .find(|d| d.get(b"Type").ok().and_then(|t| t.as_name().ok()) == Some(b"Sig"))
        .unwrap();
    let br: Vec<usize> = v
        .get(b"ByteRange")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o.as_i64().unwrap() as usize)
        .collect();
    let mut content = bytes[..br[1]].to_vec();
    content.extend_from_slice(&bytes[br[2]..br[2] + br[3]]);
    let raw = v.get(b"Contents").unwrap().as_str().unwrap().to_vec();
    // Trim the zero padding using the DER length.
    let (hdr, len) = if raw[1] & 0x80 == 0 {
        (2, usize::from(raw[1]))
    } else {
        let n = usize::from(raw[1] & 0x7F);
        (
            2 + n,
            raw[2..2 + n]
                .iter()
                .fold(0usize, |a, b| (a << 8) | usize::from(*b)),
        )
    };
    (raw[..hdr + len].to_vec(), content)
}

#[test]
fn invisible_rsa_signature_is_valid_for_every_checker() {
    let (original, bytes) = signed("rsa-aes.p12", &SignOptions::default());
    // Incremental: the original file is an exact prefix.
    assert_eq!(&bytes[..original.len()], &original[..]);
    // Our inspector.
    let doc = open(bytes.clone());
    let sigs = list_signatures(&doc);
    assert_eq!(sigs.len(), 1);
    assert_eq!(
        sigs[0].status,
        SignatureStatus::IntegrityOk,
        "{:?}",
        sigs[0]
    );
    assert!(sigs[0].covers_whole_file);
    assert_eq!(sigs[0].signer, "BergPDF Test RSA");
    assert_eq!(sigs[0].sub_filter, "adbe.pkcs7.detached");
    // OpenSSL.
    let (cms, content) = cms_and_content(&bytes);
    if let Some(ok) = openssl_cms_verify(&cms, &content) {
        assert!(ok, "openssl rejected our signature");
    }
    // poppler pdfsig.
    match poppler_pdfsig(&bytes) {
        Some(out) => {
            assert!(out.contains("Signature is Valid"), "pdfsig said:\n{out}");
            assert!(out.contains("BergPDF Test RSA"), "{out}");
        }
        None => assert!(!oracles_required(), "pdfsig required"),
    }
}

#[test]
fn ecdsa_and_chain_identities_also_verify() {
    for (file, cn) in [
        ("ec-aes.p12", "BergPDF Test EC"),
        ("chain-aes.p12", "Mark Test Signer"),
    ] {
        let (_, bytes) = signed(file, &SignOptions::default());
        let sigs = list_signatures(&open(bytes.clone()));
        assert_eq!(sigs[0].status, SignatureStatus::IntegrityOk, "{file}");
        assert_eq!(sigs[0].signer, cn);
        if let Some(out) = poppler_pdfsig(&bytes) {
            assert!(out.contains("Signature is Valid"), "{file}: {out}");
        } else {
            assert!(!oracles_required(), "pdfsig required");
        }
    }
}

#[test]
fn tampering_with_signed_bytes_is_detected() {
    let (_, mut bytes) = signed("rsa-aes.p12", &SignOptions::default());
    // Alter a content byte inside the signed range (a text character in the page content).
    let pos = bytes
        .windows(3)
        .position(|w| w == b"Tj\n" || w == b" Tj")
        .unwrap();
    bytes[pos - 3] ^= 0x01;
    let sigs = list_signatures(&open(bytes.clone()));
    assert_eq!(
        sigs[0].status,
        SignatureStatus::DigestMismatch,
        "{:?}",
        sigs[0]
    );
    if let Some(out) = poppler_pdfsig(&bytes) {
        assert!(
            !out.contains("Signature is Valid"),
            "pdfsig accepted tampered file:\n{out}"
        );
    }
}

#[test]
fn visible_signature_has_an_appearance_and_the_field_metadata() {
    let mut doc = open(helvetica_lines());
    let page = doc.page_ids().unwrap()[0];
    let opts = SignOptions {
        reason: "I approve".into(),
        location: "Eindhoven".into(),
        contact: "mark@example.invalid".into(),
        page: Some(page),
        rect: Some(Rect::new(300.0, 80.0, 560.0, 160.0)),
    };
    let bytes = doc.sign(&identity("rsa-aes.p12"), &opts).unwrap();
    let again = open(bytes.clone());
    let s = &list_signatures(&again)[0];
    assert_eq!(
        (s.reason.as_str(), s.location.as_str()),
        ("I approve", "Eindhoven")
    );
    assert_eq!(s.page, Some(page));
    assert_eq!(s.status, SignatureStatus::IntegrityOk);
    if let Some((w, h, px)) = poppler_render(&bytes, 1, 72) {
        // The box is at x 300..560, y 80..160 of an 842-pt-high page: look for ink inside it.
        let mut dark = 0;
        for y in (842 - 160)..(842 - 80) {
            for x in 300..560 {
                if px[((y as u32 * w + x as u32) * 3) as usize] < 120 {
                    dark += 1;
                }
            }
        }
        assert!(
            dark > 80,
            "signature appearance missing ({dark} dark pixels, {w}x{h})"
        );
    } else {
        assert!(!oracles_required(), "poppler required");
    }
    if let Some(out) = poppler_pdfsig(&bytes) {
        assert!(out.contains("Signature is Valid"), "{out}");
    }
}

#[test]
fn edits_after_signing_keep_the_signature_valid_but_not_whole_file() {
    let (_, bytes) = signed("rsa-aes.p12", &SignOptions::default());
    let mut doc = open(bytes);
    let page = doc.page_ids().unwrap()[0];
    doc.transact(|tx| {
        let spec =
            pdf_engine::annot::AnnotationSpec::new(pdf_engine::annot::AnnotationKind::Rectangle {
                rect: Rect::new(10.0, 10.0, 60.0, 40.0),
            });
        pdf_engine::annot::add_annotation(tx, page, &spec)
    })
    .unwrap();
    let after = doc.snapshot_bytes().unwrap();
    let s = &list_signatures(&open(after))[0];
    assert_eq!(
        s.status,
        SignatureStatus::IntegrityOk,
        "an appended revision must not break the earlier signature"
    );
    assert!(
        !s.covers_whole_file,
        "…but the signature no longer covers the whole file"
    );
}

#[test]
fn two_signatures_can_coexist() {
    let (_, bytes) = signed("rsa-aes.p12", &SignOptions::default());
    let mut doc = open(bytes);
    let second = doc
        .sign(&identity("ec-aes.p12"), &SignOptions::default())
        .unwrap();
    let sigs = list_signatures(&open(second.clone()));
    assert_eq!(sigs.len(), 2);
    assert!(
        sigs.iter()
            .all(|s| s.status == SignatureStatus::IntegrityOk),
        "{sigs:?}"
    );
    assert_eq!(sigs.iter().filter(|s| s.covers_whole_file).count(), 1);
    assert_ne!(sigs[0].field, sigs[1].field);
}

#[test]
fn unsigned_documents_report_no_signatures() {
    assert!(list_signatures(&open(helvetica_lines())).is_empty());
    assert!(list_signatures(&open(form_rich())).is_empty());
}

#[test]
fn a_form_document_keeps_its_fields_when_signed() {
    let mut doc = open(form_rich());
    let bytes = doc
        .sign(&identity("rsa-aes.p12"), &SignOptions::default())
        .unwrap();
    let again = open(bytes);
    // 5 original fields + the signature field.
    assert_eq!(pdf_engine::forms::read_form(&again).fields.len(), 5 + 1);
    assert_eq!(list_signatures(&again).len(), 1);
}
