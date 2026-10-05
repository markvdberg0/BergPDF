//! Which operating system's conventions to present.

/// Desktop operating systems we present native conventions for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OsKind {
    /// Windows 11 / 10.
    Windows,
    /// macOS.
    MacOs,
    /// Linux and other Unix (experimental target).
    Linux,
}

impl OsKind {
    /// The OS this binary was compiled for.
    pub const fn current() -> Self {
        if cfg!(target_os = "windows") {
            OsKind::Windows
        } else if cfg!(target_os = "macos") {
            OsKind::MacOs
        } else {
            OsKind::Linux
        }
    }

    /// Whether the primary modifier is ⌘ (true) or Ctrl (false).
    pub const fn uses_command_key(self) -> bool {
        matches!(self, OsKind::MacOs)
    }
}
