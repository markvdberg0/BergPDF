//! Where the user's AI provider API key lives on this computer.
//!
//! The key is stored in its own small file in the per-user config directory, **not** in the
//! preferences file, so preferences can be shared or backed up without leaking it. On Unix the
//! file is created readable by the owner only (mode 0600). On Windows it inherits the ACL of the
//! user's profile folder. The key is **not encrypted**: any program running as the same user
//! can read it. An operating-system key store (Keychain, Credential Manager, Secret Service)
//! would be better and is on the roadmap; until then the preferences dialog says so.
//!
//! The environment variable `BERGPDF_AI_KEY`, when set, takes precedence and is never written
//! anywhere.

use std::io::Write;
use std::path::{Path, PathBuf};

const FILE: &str = "ai-key";
const ENV: &str = "BERGPDF_AI_KEY";

/// Where a key came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeySource {
    /// `BERGPDF_AI_KEY`.
    Environment,
    /// The key file.
    File,
    /// No key.
    None,
}

/// Path of the key file.
pub fn key_path() -> PathBuf {
    crate::dirs::config_dir().join(FILE)
}

/// Read a key from `path`, trimmed. Empty files count as "no key".
pub fn load_from(path: &Path) -> Option<String> {
    let s = std::fs::read_to_string(path).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// Write `key` to `path` (owner-only on Unix), replacing any previous key atomically.
pub fn save_to(path: &Path, key: &str) -> std::io::Result<()> {
    let key = key.trim();
    if key.is_empty() || key.contains(['\n', '\r']) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the key is empty or contains a line break",
        ));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(key.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// Remove the key file (no error when it does not exist).
pub fn delete_at(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// The key to use now: the environment variable, else the key file.
pub fn load_ai_key() -> Option<String> {
    if let Some(k) = std::env::var(ENV).ok().map(|k| k.trim().to_string())
        && !k.is_empty()
    {
        return Some(k);
    }
    load_from(&key_path())
}

/// Where the current key comes from.
pub fn key_source() -> KeySource {
    if std::env::var(ENV).is_ok_and(|k| !k.trim().is_empty()) {
        KeySource::Environment
    } else if load_from(&key_path()).is_some() {
        KeySource::File
    } else {
        KeySource::None
    }
}

/// Save the key to the standard location.
pub fn save_ai_key(key: &str) -> std::io::Result<()> {
    save_to(&key_path(), key)
}

/// Delete the stored key (the environment variable, if any, is untouched).
pub fn delete_ai_key() -> std::io::Result<()> {
    delete_at(&key_path())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn round_trip_replace_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub/ai-key");
        assert_eq!(load_from(&p), None);
        save_to(&p, "  sk-test-123\n").unwrap();
        assert_eq!(load_from(&p).as_deref(), Some("sk-test-123"));
        save_to(&p, "sk-other").unwrap();
        assert_eq!(load_from(&p).as_deref(), Some("sk-other"));
        assert!(!p.with_extension("tmp").exists());
        delete_at(&p).unwrap();
        assert_eq!(load_from(&p), None);
        delete_at(&p).unwrap(); // idempotent
    }

    #[test]
    fn rejects_empty_and_multiline_keys() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("k");
        assert!(save_to(&p, "   ").is_err());
        assert!(save_to(&p, "a\nb").is_err());
        assert!(!p.exists());
    }

    #[cfg(unix)]
    #[test]
    fn key_file_is_private_to_the_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("k");
        save_to(&p, "sk-secret").unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "mode was {mode:o}");
        // Replacing keeps it private too.
        save_to(&p, "sk-secret-2").unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
