//! Download a file and verify its SHA-256 before it is used.
//!
//! Used for the OCR models only, and only when the person pressed "Download". The file is written
//! to `<dest>.partial`, hashed while it arrives, compared with the expected hash and only then
//! renamed into place; a wrong hash deletes it. HTTPS is required (plain http only for a loopback
//! server, which is how the tests run). Sizes are capped.

use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Hard cap on a download (bytes). The OCR models are ~10 MB.
pub const MAX_BYTES: u64 = 200 * 1024 * 1024;

/// What can go wrong.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DownloadError {
    /// The address is not acceptable.
    #[error("{0}")]
    Url(String),
    /// Connection or transfer problem.
    #[error("Download failed: {0}")]
    Network(String),
    /// The server answered with an error.
    #[error("The server answered HTTP {0}.")]
    Status(u16),
    /// The file is bigger than allowed.
    #[error("The file is larger than the allowed {0} MB.")]
    TooLarge(u64),
    /// The hash differs, so the file was discarded.
    #[error("The downloaded file did not match its expected checksum and was discarded.")]
    BadChecksum,
    /// Writing the file failed.
    #[error("Could not write the file: {0}")]
    Io(String),
    /// The user cancelled.
    #[error("Cancelled.")]
    Cancelled,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn loopback(url: &str) -> bool {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or("")
        .split(['/', ':'])
        .next()
        .unwrap_or("");
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

/// SHA-256 of `bytes` as lower-case hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// Whether `path` exists and has the expected hash.
pub fn file_matches(path: &Path, sha256: &str) -> bool {
    std::fs::read(path).is_ok_and(|b| sha256_hex(&b) == sha256)
}

/// Download `url` to `dest`, calling `progress(bytes_so_far, total_if_known)` as data arrives.
pub fn fetch_verified(
    url: &str,
    sha256: &str,
    dest: &Path,
    progress: &dyn Fn(u64, Option<u64>),
    cancel: &AtomicBool,
) -> Result<(), DownloadError> {
    if !(url.starts_with("https://") || (url.starts_with("http://") && loopback(url))) {
        return Err(DownloadError::Url(
            "Downloads must use https:// addresses.".into(),
        ));
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| DownloadError::Io(e.to_string()))?;
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(300)))
        .http_status_as_error(false)
        .https_only(!loopback(url))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .user_agent("BergPDF")
        .build()
        .into();
    let mut resp = agent
        .get(url)
        .call()
        .map_err(|e| DownloadError::Network(e.to_string()))?;
    let code = resp.status().as_u16();
    if !(200..300).contains(&code) {
        return Err(DownloadError::Status(code));
    }
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if total.is_some_and(|t| t > MAX_BYTES) {
        return Err(DownloadError::TooLarge(MAX_BYTES / 1024 / 1024));
    }
    let tmp = dest.with_extension("partial");
    let result = (|| {
        let mut out = std::fs::File::create(&tmp).map_err(|e| DownloadError::Io(e.to_string()))?;
        let mut hasher = Sha256::new();
        let mut body = resp.body_mut().as_reader();
        let (mut buf, mut got) = (vec![0u8; 64 * 1024], 0u64);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(DownloadError::Cancelled);
            }
            let n = body
                .read(&mut buf)
                .map_err(|e| DownloadError::Network(e.to_string()))?;
            if n == 0 {
                break;
            }
            got += n as u64;
            if got > MAX_BYTES {
                return Err(DownloadError::TooLarge(MAX_BYTES / 1024 / 1024));
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n])
                .map_err(|e| DownloadError::Io(e.to_string()))?;
            progress(got, total);
        }
        out.sync_all()
            .map_err(|e| DownloadError::Io(e.to_string()))?;
        if hex(&hasher.finalize()) != sha256 {
            return Err(DownloadError::BadChecksum);
        }
        Ok(())
    })();
    match result {
        Ok(()) => std::fs::rename(&tmp, dest).map_err(|e| DownloadError::Io(e.to_string())),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}
