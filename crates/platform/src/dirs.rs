//! Platform-appropriate per-user directories (never next to the executable).

use std::path::PathBuf;

const APP: &str = "BergPDF";

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Directory for the preferences file.
pub fn config_dir() -> PathBuf {
    base(Kind::Config).join(APP)
}

/// Directory for application data (recovery files live under it).
pub fn data_dir() -> PathBuf {
    base(Kind::Data).join(APP)
}

/// Directory for disposable caches.
pub fn cache_dir() -> PathBuf {
    base(Kind::Cache).join(APP)
}

#[derive(Clone, Copy)]
enum Kind {
    Config,
    Data,
    Cache,
}

#[cfg(target_os = "windows")]
fn base(kind: Kind) -> PathBuf {
    match kind {
        Kind::Config | Kind::Data => env_path("APPDATA"),
        Kind::Cache => env_path("LOCALAPPDATA"),
    }
    .unwrap_or_else(std::env::temp_dir)
}

#[cfg(target_os = "macos")]
fn base(kind: Kind) -> PathBuf {
    let home = env_path("HOME").unwrap_or_else(std::env::temp_dir);
    match kind {
        Kind::Config | Kind::Data => home.join("Library/Application Support"),
        Kind::Cache => home.join("Library/Caches"),
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn base(kind: Kind) -> PathBuf {
    let home = env_path("HOME").unwrap_or_else(std::env::temp_dir);
    match kind {
        Kind::Config => env_path("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config")),
        Kind::Data => env_path("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share")),
        Kind::Cache => env_path("XDG_CACHE_HOME").unwrap_or_else(|| home.join(".cache")),
    }
}

/// Path of the preferences file.
pub fn prefs_file() -> PathBuf {
    config_dir().join("preferences.toml")
}

/// Read a text file if it exists.
pub fn read_text(path: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Write a small text file atomically (temp + rename), creating parent directories.
pub fn write_text_atomic(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirs_are_under_a_named_app_folder_and_not_beside_the_exe() {
        for d in [config_dir(), data_dir(), cache_dir()] {
            assert_eq!(d.file_name().and_then(|n| n.to_str()), Some(APP));
        }
        assert!(prefs_file().ends_with("preferences.toml"));
    }

    #[test]
    fn atomic_text_write() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub/x.toml");
        write_text_atomic(&p, "a = 1").unwrap();
        assert_eq!(read_text(&p).as_deref(), Some("a = 1"));
        write_text_atomic(&p, "a = 2").unwrap();
        assert_eq!(read_text(&p).as_deref(), Some("a = 2"));
        assert!(!p.with_extension("tmp").exists());
    }
}
