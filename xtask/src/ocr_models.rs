//! `cargo xtask fetch-ocr-models [dir]`: download the two OCR model files and verify them against
//! the SHA-256 hashes pinned in `pdf-ocr`. Besides the in-app download button this is the only
//! place that downloads anything, and it only runs when a developer asks for it (or `dist
//! --with-ocr-models` does). The files are not committed to the repository.
//!
//! The models are the stock `ocrs` models (https://github.com/robertknight/ocrs-models), trained on
//! the HierText dataset (CC BY-SA 4.0). Their redistribution terms have not been verified — see
//! docs/DEPENDENCIES.md and D-023 before shipping them to anyone.

use ai_client::download::{self, DownloadError};
use pdf_ocr::{MODEL_BASE_URL, MODEL_SOURCES};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

pub fn fetch(dir: Option<PathBuf>) -> Result<(), String> {
    let dir = dir.unwrap_or_else(platform::dirs::ocr_models_dir);
    fetch_into(&dir)?;
    println!("OCR models are in {}", dir.display());
    Ok(())
}

/// Make sure both verified model files are in `dir`.
pub fn fetch_into(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let cancel = AtomicBool::new(false);
    for (name, want) in MODEL_SOURCES {
        let dest = dir.join(name);
        if download::file_matches(&dest, want) {
            println!("{name}: already present and verified");
            continue;
        }
        println!("downloading {MODEL_BASE_URL}{name} …");
        download::fetch_verified(
            &format!("{MODEL_BASE_URL}{name}"),
            want,
            &dest,
            &|_, _| {},
            &cancel,
        )
        .map_err(|e| match e {
            DownloadError::BadChecksum => {
                format!("{name}: SHA-256 mismatch; the file was discarded")
            }
            other => format!("download of {name} failed: {other}"),
        })?;
        println!("{name}: verified");
    }
    Ok(())
}
