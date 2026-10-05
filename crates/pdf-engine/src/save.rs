//! Safe saving: temp file in the destination directory → flush → fsync → re-open and
//! validate → atomic replace. The original file is never truncated; any failure leaves it
//! byte-for-byte intact and removes the temporary file.

use crate::doc::{OpenOptions, PdfDocument};
use crate::error::{EngineError, Result};
use std::fs::{self, File, OpenOptions as FsOpen};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Size + modification time of a file, used to detect external modification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStamp {
    /// Length in bytes.
    pub len: u64,
    /// Last modification time, when the platform reports it.
    pub modified: Option<SystemTime>,
}

impl FileStamp {
    /// Stamp a file on disk.
    pub fn of(path: &Path) -> std::io::Result<Self> {
        let m = fs::metadata(path)?;
        Ok(Self {
            len: m.len(),
            modified: m.modified().ok(),
        })
    }
}

/// Failure injection for tests (never set in production code).
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub enum InjectedFailure {
    /// Fail with a "disk full" style error after this many bytes were written.
    WriteAfter(usize),
    /// Fail right before the final rename.
    BeforeRename,
}

/// Options for [`write_atomic`].
#[derive(Clone, Debug, Default)]
pub struct SaveOptions {
    /// If set and the destination exists, it must still match this stamp.
    pub expected_stamp: Option<FileStamp>,
    /// Expected page count for post-write validation (skipped when `None`).
    pub expected_pages: Option<usize>,
    /// Test-only failure injection.
    #[doc(hidden)]
    pub inject: Option<InjectedFailure>,
}

fn tmp_path(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map_or_else(|| "document".into(), |n| n.to_string_lossy().into_owned());
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    dest.with_file_name(format!(".{name}.{}.{nanos}.berg-tmp", std::process::id()))
}

/// Atomically write `bytes` to `dest`. Returns the new file stamp.
pub fn write_atomic(dest: &Path, bytes: &[u8], opts: &SaveOptions) -> Result<FileStamp> {
    if let (Some(expected), true) = (opts.expected_stamp, dest.exists()) {
        let cur = FileStamp::of(dest)?;
        if cur != expected {
            return Err(EngineError::ExternallyModified);
        }
    }
    let tmp = tmp_path(dest);
    let result = write_and_replace(dest, &tmp, bytes, opts);
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn write_and_replace(
    dest: &Path,
    tmp: &Path,
    bytes: &[u8],
    opts: &SaveOptions,
) -> Result<FileStamp> {
    {
        let mut f = FsOpen::new().write(true).create_new(true).open(tmp)?;
        if let Some(InjectedFailure::WriteAfter(n)) = opts.inject {
            let n = n.min(bytes.len());
            f.write_all(&bytes[..n])?;
            return Err(EngineError::Save(
                "No space left on device (injected)".into(),
            ));
        }
        f.write_all(bytes)?;
        f.flush()?;
        f.sync_all()?;
    }
    // Validate by re-reading what is actually on disk.
    let written = fs::read(tmp)?;
    if written != bytes {
        return Err(EngineError::Save(
            "verification failed: written bytes differ".into(),
        ));
    }
    let reopened = PdfDocument::open(written, &OpenOptions::default())
        .map_err(|e| EngineError::Save(format!("saved file failed validation: {e}")))?;
    if let Some(n) = opts.expected_pages
        && reopened.page_count() != n
    {
        return Err(EngineError::Save(format!(
            "saved file has {} pages, expected {n}",
            reopened.page_count()
        )));
    }
    // Keep the existing file's permissions.
    if let Ok(meta) = fs::metadata(dest) {
        let _ = fs::set_permissions(tmp, meta.permissions());
    }
    if let Some(InjectedFailure::BeforeRename) = opts.inject {
        return Err(EngineError::Save("replacement failed (injected)".into()));
    }
    // `rename` replaces atomically on POSIX and uses MoveFileExW(REPLACE_EXISTING) on Windows.
    fs::rename(tmp, dest).map_err(|e| {
        EngineError::Save(format!(
            "could not replace the destination (is it open in another program?): {e}"
        ))
    })?;
    sync_dir(dest);
    Ok(FileStamp::of(dest)?)
}

#[cfg(unix)]
fn sync_dir(dest: &Path) {
    if let Some(dir) = dest.parent()
        && let Ok(d) = File::open(if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir
        })
    {
        let _ = d.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir(_dest: &Path) {}
