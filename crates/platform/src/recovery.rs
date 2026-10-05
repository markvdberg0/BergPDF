//! Crash-recovery files: private, bounded and removable.
//!
//! Recovery files contain document contents, so they are created with owner-only permissions
//! where the OS supports it, never logged, capped in number and total size, and deleted when a
//! document is saved or closed cleanly.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Maximum recovery files kept.
pub const MAX_FILES: usize = 24;
/// Maximum total bytes kept.
pub const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// A recovery entry found at startup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryEntry {
    /// Path of the recovered bytes.
    pub pdf: PathBuf,
    /// Original file path, if the document had one.
    pub original: Option<String>,
    /// When the recovery file was last written.
    pub modified: std::time::SystemTime,
    /// Size in bytes.
    pub len: u64,
}

fn pdf_path(dir: &Path, token: &str) -> PathBuf {
    dir.join(format!("{token}.recovery.pdf"))
}

fn meta_path(dir: &Path, token: &str) -> PathBuf {
    dir.join(format!("{token}.recovery.meta"))
}

#[cfg(unix)]
fn private_open(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn private_open(path: &Path) -> std::io::Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}

/// Write (or refresh) the recovery file for a document.
pub fn write(dir: &Path, token: &str, original: Option<&str>, bytes: &[u8]) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{token}.recovery.tmp"));
    {
        let mut f = private_open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, pdf_path(dir, token))?;
    let mut m = private_open(&meta_path(dir, token))?;
    m.write_all(original.unwrap_or("").as_bytes())?;
    prune(dir);
    Ok(())
}

/// Remove a document's recovery files.
pub fn remove(dir: &Path, token: &str) {
    let _ = fs::remove_file(pdf_path(dir, token));
    let _ = fs::remove_file(meta_path(dir, token));
}

/// List recoverable documents.
pub fn list(dir: &Path) -> Vec<RecoveryEntry> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if let Some(token) = name.strip_suffix(".recovery.pdf") {
            let original = fs::read_to_string(meta_path(dir, token))
                .ok()
                .filter(|s| !s.is_empty());
            let md = e.metadata().ok();
            out.push(RecoveryEntry {
                pdf: p,
                original,
                modified: md
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .unwrap_or(std::time::UNIX_EPOCH),
                len: md.map_or(0, |m| m.len()),
            });
        }
    }
    out.sort_by(|a, b| a.pdf.cmp(&b.pdf));
    out
}

/// Entries that no running instance can still be writing: untouched for at least `min_age`.
/// (A live instance rewrites its recovery file at least every 30 seconds while a document is
/// modified, so a recent file belongs to another running instance, not to a crash.)
pub fn list_stale(dir: &Path, min_age: std::time::Duration) -> Vec<RecoveryEntry> {
    let now = std::time::SystemTime::now();
    list(dir)
        .into_iter()
        .filter(|e| {
            now.duration_since(e.modified)
                .is_ok_and(|age| age >= min_age)
        })
        .collect()
}

/// Delete the oldest files beyond the count and size caps.
pub fn prune(dir: &Path) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, PathBuf, u64)> = rd
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".recovery.pdf"))
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, e.path(), m.len()))
        })
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0)); // newest first
    let mut total = 0u64;
    for (i, (_, path, len)) in files.iter().enumerate() {
        total += len;
        if i >= MAX_FILES || total > MAX_TOTAL_BYTES {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            if let Some(token) = name.strip_suffix(".recovery.pdf") {
                remove(dir, token);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_list_remove() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "doc1", Some("/a/b.pdf"), b"%PDF-1.4 x").unwrap();
        write(d.path(), "doc2", None, b"%PDF-1.4 y").unwrap();
        let l = list(d.path());
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].original.as_deref(), Some("/a/b.pdf"));
        assert_eq!(l[1].original, None);
        assert_eq!(fs::read(&l[0].pdf).unwrap(), b"%PDF-1.4 x");
        remove(d.path(), "doc1");
        assert_eq!(list(d.path()).len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "p", None, b"secret").unwrap();
        let mode = fs::metadata(pdf_path(d.path(), "p"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o077,
            0,
            "recovery file must not be group/world accessible"
        );
    }

    #[test]
    fn count_cap_prunes_oldest() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..(MAX_FILES + 5) {
            write(d.path(), &format!("d{i:03}"), None, b"x").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(list(d.path()).len() <= MAX_FILES);
    }
}

#[cfg(test)]
mod stale_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn recent_files_belong_to_a_live_instance_and_are_not_offered() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "live", Some("/a.pdf"), b"%PDF-1.4 x").unwrap();
        assert_eq!(list(d.path()).len(), 1);
        assert!(list_stale(d.path(), Duration::from_secs(90)).is_empty());
        let e = list_stale(d.path(), Duration::ZERO);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].len, 10);
    }
}
