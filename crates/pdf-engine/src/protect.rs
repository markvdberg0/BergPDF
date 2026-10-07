//! Password-protected documents.
//!
//! A protected file is *unlocked once* when it is opened: the whole document is decrypted and written as a
//! plain working copy, and everything else in the engine (rendering, editing, undo, text extraction) works on
//! that copy and knows nothing about encryption. The [`Protection`] kept beside it remembers how the file was
//! protected, so that saving can encrypt it again **with the same passwords** (nothing is silently stripped),
//! unless the person explicitly removes the protection.
//!
//! What the permission flags of the file mean here: opened with the owner password (or a document without a
//! separate owner password) everything is allowed; opened with the user password only, the flags decide. Editing
//! needs "modify", printing needs "print", copying text needs "copy" (see [`Rights`]).

use crate::error::{EngineError, Result, sanitize};
use lopdf::encryption::crypt_filters::{Aes256CryptFilter, CryptFilter};
use lopdf::{Document, EncryptionState, EncryptionVersion, Object, Permissions};
use md5::{Digest, Md5};
use std::collections::BTreeMap;
use std::sync::Arc;

/// What the document's author allows (the permission flags of the encryption dictionary).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rights {
    /// Print the document.
    pub print: bool,
    /// Change the content (text, images, pages' content).
    pub modify: bool,
    /// Copy text and graphics.
    pub copy: bool,
    /// Add or change annotations.
    pub annotate: bool,
    /// Fill in form fields.
    pub fill: bool,
    /// Insert, delete, rotate and reorder pages.
    pub assemble: bool,
}

impl Rights {
    /// Everything allowed.
    pub const ALL: Rights = Rights {
        print: true,
        modify: true,
        copy: true,
        annotate: true,
        fill: true,
        assemble: true,
    };

    fn from_permissions(p: Permissions) -> Self {
        Rights {
            print: p.contains(Permissions::PRINTABLE),
            modify: p.contains(Permissions::MODIFIABLE),
            copy: p.contains(Permissions::COPYABLE),
            annotate: p.contains(Permissions::ANNOTABLE),
            fill: p.contains(Permissions::FILLABLE),
            assemble: p.contains(Permissions::ASSEMBLABLE),
        }
    }

    fn to_permissions(self) -> Permissions {
        let mut p =
            Permissions::COPYABLE_FOR_ACCESSIBILITY | Permissions::PRINTABLE_IN_HIGH_QUALITY;
        for (on, flag) in [
            (self.print, Permissions::PRINTABLE),
            (self.modify, Permissions::MODIFIABLE),
            (self.copy, Permissions::COPYABLE),
            (self.annotate, Permissions::ANNOTABLE),
            (self.fill, Permissions::FILLABLE),
            (self.assemble, Permissions::ASSEMBLABLE),
        ] {
            if on {
                p |= flag;
            }
        }
        p
    }
}

/// The encryption method of a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cipher {
    /// RC4, 40-bit key (old and weak).
    Rc4_40,
    /// RC4, up to 128-bit key.
    Rc4,
    /// AES, 128-bit key.
    Aes128,
    /// AES, 256-bit key.
    Aes256,
}

impl Cipher {
    /// Name for the user interface.
    pub fn label(self) -> &'static str {
        match self {
            Cipher::Rc4_40 => "RC4 40-bit",
            Cipher::Rc4 => "RC4 128-bit",
            Cipher::Aes128 => "AES 128-bit",
            Cipher::Aes256 => "AES 256-bit",
        }
    }
}

/// What the interface needs to know about the protection of the open document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtectionInfo {
    /// Method used when the file is saved.
    pub cipher: Cipher,
    /// Opened with the owner password (nothing is restricted).
    pub owner: bool,
    /// What may be done with this document right now (everything when `owner`).
    pub rights: Rights,
}

/// How the open document was protected and what it takes to protect it again.
#[derive(Clone)]
pub(crate) struct Protection {
    state: EncryptionState,
    /// The password the document was opened with; used to check what was written.
    password: String,
    /// The encrypted bytes it was opened from (to unlock with another password).
    source: Option<Arc<Vec<u8>>>,
    owner: bool,
    /// The flags stored in the file.
    stored: Rights,
    cipher: Cipher,
}

impl std::fmt::Debug for Protection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Protection")
            .field("cipher", &self.cipher)
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

impl Protection {
    /// Capture the protection of a document lopdf has just decrypted.
    pub(crate) fn from_decrypted(
        doc: &Document,
        password: &str,
        source: Arc<Vec<u8>>,
    ) -> Option<Self> {
        let state = doc.encryption_state.clone()?;
        let stored = Rights::from_permissions(state.permissions());
        let cipher = cipher_of(&state);
        let owner = authenticates_as_owner(&state, doc, password);
        Some(Protection {
            state,
            password: password.to_string(),
            source: Some(source),
            owner,
            stored,
            cipher,
        })
    }

    pub(crate) fn info(&self) -> ProtectionInfo {
        ProtectionInfo {
            cipher: self.cipher,
            owner: self.owner,
            rights: if self.owner { Rights::ALL } else { self.stored },
        }
    }

    pub(crate) fn password(&self) -> &str {
        &self.password
    }

    pub(crate) fn source(&self) -> Option<&Arc<Vec<u8>>> {
        self.source.as_ref()
    }

    /// Encrypt plain document bytes the way the original was protected.
    pub(crate) fn seal(&self, plain: &[u8]) -> Result<Vec<u8>> {
        let mut doc = Document::load_mem(plain)?;
        doc.encrypt(&self.state)
            .map_err(|e| EngineError::Save(sanitize(&format!("encryption failed: {e}"))))?;
        let mut out = Vec::new();
        doc.save_to(&mut out)
            .map_err(|e| EngineError::Save(sanitize(&e.to_string())))?;
        Ok(out)
    }

    /// New AES-256 protection (revision 6) with the given passwords and rights. The caller opens it as owner.
    pub(crate) fn new_aes256(user: &str, owner: &str, rights: Rights) -> Result<Self> {
        let mut key = [0u8; 32];
        getrandom::fill(&mut key)
            .map_err(|e| EngineError::Save(format!("no random numbers available: {e}")))?;
        let filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
        let state = EncryptionState::try_from(EncryptionVersion::V5 {
            encrypt_metadata: true,
            crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
            file_encryption_key: &key,
            stream_filter: b"StdCF".to_vec(),
            string_filter: b"StdCF".to_vec(),
            owner_password: owner,
            user_password: user,
            permissions: rights.to_permissions(),
        })
        .map_err(|e| EngineError::InvalidArgument(sanitize(&e.to_string())))?;
        Ok(Protection {
            state,
            password: user.to_string(),
            source: None,
            owner: true,
            stored: rights,
            cipher: Cipher::Aes256,
        })
    }
}

fn cipher_of(state: &EncryptionState) -> Cipher {
    match state.version() {
        1 => Cipher::Rc4_40,
        2 | 3 => Cipher::Rc4,
        4 => match state.get_stream_filter().method() {
            b"AESV2" => Cipher::Aes128,
            _ => Cipher::Rc4,
        },
        _ => Cipher::Aes256,
    }
}

/// Whether `password` is the owner password. lopdf can only check a document that still carries its
/// `/Encrypt` dictionary, so a small stand-in with the same dictionary and `/ID` is checked.
fn authenticates_as_owner(state: &EncryptionState, doc: &Document, password: &str) -> bool {
    let Ok(dict) = state.encode() else {
        return false;
    };
    let mut probe = Document::new();
    probe.objects.insert((1, 0), Object::Dictionary(dict));
    probe.trailer.set("Encrypt", Object::Reference((1, 0)));
    if let Ok(id) = doc.trailer.get(b"ID") {
        probe.trailer.set("ID", id.clone());
    }
    probe.authenticate_owner_password(password).is_ok()
}

/// Open `bytes` with `password` as a plain working copy.
///
/// Returns `Ok(None)` for a file that is not encrypted. `Err(PasswordRequired)` when no password was given and the
/// empty one does not work, `Err(WrongPassword)` when the given one does not.
pub(crate) fn unlock(
    bytes: &[u8],
    max_decompressed_size: usize,
    password: &str,
) -> Result<Option<(Vec<u8>, Protection)>> {
    // lopdf's loader decrypts while it parses and needs a password that works as a *user* password: it takes an
    // owner password for the user password, which gives the wrong key for RC4 and AES-128 files. So a first
    // parse (which only reads the encryption dictionary when no password fits) tells which password we hold.
    let load = |password: Option<String>| lopdf::LoadOptions {
        max_decompressed_size: Some(max_decompressed_size),
        password,
        ..Default::default()
    };
    let parse = |o| {
        Document::load_mem_with_options(bytes, o)
            .map_err(|e| EngineError::Parse(sanitize(&e.to_string())))
    };
    let probe = parse(load(None))?;
    let doc = if probe.is_encrypted() {
        if password.is_empty() {
            return Err(EngineError::PasswordRequired);
        }
        let effective = user_password_for(&probe, password)?;
        match parse(load(Some(effective))) {
            Ok(d) if !d.is_encrypted() => d,
            Ok(_) | Err(EngineError::Parse(_)) => return Err(EngineError::WrongPassword),
            Err(e) => return Err(e),
        }
    } else if probe.was_encrypted() {
        probe
    } else {
        return Ok(None);
    };
    let protection = Protection::from_decrypted(&doc, password, Arc::new(bytes.to_vec()))
        .ok_or_else(|| EngineError::Parse("the encryption settings could not be read".into()))?;
    let mut plain = Vec::new();
    let mut doc = doc;
    doc.save_to(&mut plain)
        .map_err(|e| EngineError::Parse(sanitize(&e.to_string())))?;
    Ok(Some((plain, protection)))
}

/// The password that makes lopdf's loader derive the right file key: the user password itself, or the user
/// password recovered with the owner password. `WrongPassword` when `password` is neither.
fn user_password_for(probe: &Document, password: &str) -> Result<String> {
    if probe.authenticate_user_password(password).is_ok() {
        return Ok(password.to_string());
    }
    if probe.authenticate_owner_password(password).is_err() {
        return Err(EngineError::WrongPassword);
    }
    let revision = probe
        .get_encrypted()
        .ok()
        .and_then(|d| d.get(b"R").ok())
        .and_then(|r| r.as_i64().ok())
        .unwrap_or(0);
    if revision >= 5 {
        // Newer handlers keep the file key encrypted under either password.
        return Ok(password.to_string());
    }
    // Older ones derive the key from the user password, which is stored under the owner password
    // (ISO 32000-1, algorithm 7). A user password that is not plain text cannot be handed on.
    let padded =
        user_password_from_owner(probe, password, revision).ok_or(EngineError::WrongPassword)?;
    let n = (0..=32)
        .find(|&n| padded[n..] == PAD[..32 - n])
        .unwrap_or(32);
    String::from_utf8(padded[..n].to_vec()).map_err(|_| EngineError::WrongPassword)
}

/// The padding string every password is extended with (ISO 32000-1, algorithm 2).
const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

/// The (padded) user password, recovered from the `/O` entry with the owner password: algorithm 7 of
/// ISO 32000-1, for security handlers of revision 2 to 4.
fn user_password_from_owner(doc: &Document, owner: &str, revision: i64) -> Option<Vec<u8>> {
    let dict = doc.get_encrypted().ok()?;
    let o = dict.get(b"O").ok()?.as_str().ok()?;
    if o.len() < 32 {
        return None;
    }
    let key_bytes = if revision == 2 {
        5
    } else {
        usize::try_from(dict.get(b"Length").and_then(Object::as_i64).unwrap_or(128)).ok()? / 8
    };
    if !(5..=16).contains(&key_bytes) {
        return None;
    }
    // Passwords are PDFDocEncoding text, which for what people type is Latin-1.
    let mut pw: Vec<u8> = owner
        .chars()
        .map(|c| u8::try_from(u32::from(c)).ok())
        .collect::<Option<_>>()?;
    pw.truncate(32);
    let mut padded = pw.clone();
    padded.extend_from_slice(&PAD[..32 - pw.len()]);
    let mut hash = Md5::digest(&padded).to_vec();
    if revision >= 3 {
        for _ in 0..50 {
            hash = Md5::digest(&hash).to_vec();
        }
    }
    let key = &hash[..key_bytes];
    let mut data = o[..32].to_vec();
    if revision == 2 {
        rc4(key, &mut data);
    } else {
        for i in (0..20u8).rev() {
            let k: Vec<u8> = key.iter().map(|b| b ^ i).collect();
            rc4(&k, &mut data);
        }
    }
    Some(data)
}

/// RC4 in place (it is its own inverse).
fn rc4(key: &[u8], data: &mut [u8]) {
    let mut s: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut j = 0usize;
    for i in 0..256 {
        j = (j + usize::from(s[i]) + usize::from(key[i % key.len()])) & 255;
        s.swap(i, j);
    }
    let (mut i, mut j) = (0usize, 0usize);
    for b in data {
        i = (i + 1) & 255;
        j = (j + usize::from(s[i])) & 255;
        s.swap(i, j);
        *b ^= s[(usize::from(s[i]) + usize::from(s[j])) & 255];
    }
}
