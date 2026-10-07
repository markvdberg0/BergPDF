//! How this copy of BergPDF was installed.
//!
//! A copy installed from the Microsoft Store is updated by the Store, so the program must not run its own update
//! check or point people at a download page (Microsoft Store policy; see docs/MICROSOFT_STORE.md). Every other
//! copy (installer, portable folder, `cargo run`) is *standalone*.
//!
//! `cargo xtask msix` puts a small marker file next to the program inside the package, so the very same binary
//! serves both channels and nothing depends on a Windows API. `BERG_DISTRIBUTION=store` (or `standalone`)
//! overrides the detection for trying things out.

use std::path::Path;
use std::sync::OnceLock;

/// Name of the marker file `cargo xtask msix` writes next to the program.
pub const STORE_MARKER: &str = "microsoft-store.marker";

/// Where the running copy came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    /// Installer, program folder or a developer build.
    Standalone,
    /// The Microsoft Store package (MSIX).
    Store,
}

/// The channel of the running program (decided once).
pub fn channel() -> Channel {
    static CHANNEL: OnceLock<Channel> = OnceLock::new();
    *CHANNEL.get_or_init(|| {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf));
        detect(
            exe_dir.as_deref(),
            std::env::var("BERG_DISTRIBUTION").ok().as_deref(),
        )
    })
}

/// Whether the Microsoft Store looks after updates of this copy.
pub fn is_store() -> bool {
    channel() == Channel::Store
}

/// The decision itself: an explicit override first, then the marker file beside the program.
pub fn detect(exe_dir: Option<&Path>, override_value: Option<&str>) -> Channel {
    match override_value
        .map(|v| v.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("store") => return Channel::Store,
        Some("standalone") => return Channel::Standalone,
        _ => {}
    }
    if exe_dir.is_some_and(|d| d.join(STORE_MARKER).is_file()) {
        Channel::Store
    } else {
        Channel::Standalone
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_folder_is_standalone() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect(Some(dir.path()), None), Channel::Standalone);
        assert_eq!(detect(None, None), Channel::Standalone);
    }

    #[test]
    fn the_marker_file_makes_it_a_store_copy() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(STORE_MARKER), "").unwrap();
        assert_eq!(detect(Some(dir.path()), None), Channel::Store);
    }

    #[test]
    fn the_override_wins_over_the_marker() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect(Some(dir.path()), Some("Store")), Channel::Store);
        std::fs::write(dir.path().join(STORE_MARKER), "").unwrap();
        assert_eq!(
            detect(Some(dir.path()), Some("standalone")),
            Channel::Standalone
        );
        assert_eq!(detect(Some(dir.path()), Some("nonsense")), Channel::Store);
    }
}
