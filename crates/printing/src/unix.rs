//! macOS and Linux: CUPS through its command line tools (`lpstat`, `lp`).

use crate::{PrintError, PrintRequest, Printer, lp_args};
use std::process::Command;

pub fn printers() -> Vec<Printer> {
    let list = Command::new("lpstat").arg("-e").output();
    let Ok(list) = list else {
        return Vec::new();
    };
    let default = Command::new("lpstat")
        .arg("-d")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .and_then(|s| {
            // "system default destination: Office"
            s.split_once(':').map(|(_, n)| n.trim().to_string())
        })
        .unwrap_or_default();
    String::from_utf8_lossy(&list.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|name| Printer {
            name: name.to_string(),
            is_default: name == default,
        })
        .collect()
}

pub fn print(
    bytes: &[u8],
    req: &PrintRequest,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<(), PrintError> {
    if req.output_file.is_some() {
        return Err(PrintError::Unavailable(
            "printing to a file is only available on Windows".into(),
        ));
    }
    // CUPS reads the file while the job is queued; it is removed once `lp` returned.
    let dir = std::env::temp_dir();
    let path = dir.join(format!("bergpdf-print-{}.pdf", std::process::id()));
    std::fs::write(&path, bytes).map_err(|e| PrintError::System(e.to_string()))?;
    let out = Command::new("lp").args(lp_args(req)).arg(&path).output();
    let _ = std::fs::remove_file(&path);
    let out = out.map_err(|e| {
        PrintError::Unavailable(format!("the `lp` command could not be started: {e}"))
    })?;
    if out.status.success() {
        progress(req.pages.len(), req.pages.len());
        Ok(())
    } else {
        Err(PrintError::System(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}
