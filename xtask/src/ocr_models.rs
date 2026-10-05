//! `cargo xtask fetch-ocr-models [dir]`: download the two OCR model files and verify them against
//! pinned SHA-256 hashes. This is the *only* place in the project that downloads anything, and it
//! only runs when a developer asks for it. The files are not committed or bundled.
//!
//! The models are the stock `ocrs` models (https://github.com/robertknight/ocrs-models), trained on
//! the HierText dataset (CC BY-SA 4.0). Their own licence terms have not been reviewed for
//! redistribution — see docs/DEPENDENCIES.md before shipping them to anyone.

use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::process::Command;

const BASE: &str = "https://ocrs-models.s3-accelerate.amazonaws.com/";
/// `(file name, SHA-256 of the file as downloaded on 2026-10-05)`.
const MODELS: [(&str, &str); 2] = [
    (
        "text-detection.rten",
        "f15cfb56bd02c4bf478a20343986504a1f01e1665c2b3a0ad66340f054b1b5ca",
    ),
    (
        "text-recognition.rten",
        "e484866d4cce403175bd8d00b128feb08ab42e208de30e42cd9889d8f1735a6e",
    ),
];

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn fetch(dir: Option<PathBuf>) -> Result<(), String> {
    let dir = dir.unwrap_or_else(platform::dirs::ocr_models_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    for (name, want) in MODELS {
        let dest = dir.join(name);
        if let Ok(existing) = std::fs::read(&dest)
            && hex(&Sha256::digest(&existing)) == want
        {
            println!("{name}: already present and verified");
            continue;
        }
        let tmp = dir.join(format!("{name}.partial"));
        println!("downloading {BASE}{name} …");
        let status = Command::new("curl")
            .args([
                "--fail",
                "--location",
                "--silent",
                "--show-error",
                "--retry",
                "2",
                "-o",
            ])
            .arg(&tmp)
            .arg(format!("{BASE}{name}"))
            .status()
            .map_err(|e| format!("could not run curl (needed for the download): {e}"))?;
        if !status.success() {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("download of {name} failed ({status})"));
        }
        let got = hex(&Sha256::digest(
            std::fs::read(&tmp).map_err(|e| e.to_string())?,
        ));
        if got != want {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!(
                "{name}: SHA-256 mismatch (expected {want}, got {got}); the file was discarded"
            ));
        }
        std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
        println!("{name}: verified");
    }
    println!("OCR models are in {}", dir.display());
    Ok(())
}
