//! Thin OS-integration layer. The rest of the application never touches OS APIs directly.
//!
//! What lives here: per-user config/data/recovery directories, native file dialogs,
//! private recovery files, and opening *confirmed* external links. Printing, file
//! associations, taskbar/dock integration and Finder "open with" events need real
//! OS-specific code that cannot be exercised in the Linux CI sandbox; they are tracked in
//! docs/PLATFORM_CHECKLIST.md rather than stubbed behind buttons.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod dialogs;
pub mod dirs;
pub mod instance;
pub mod links;
pub mod recovery;
pub mod secrets;
