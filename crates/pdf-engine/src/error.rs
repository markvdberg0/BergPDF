//! Structured engine errors. Messages never include document-controlled bytes
//! beyond short, sanitised identifiers, so they are safe to log and display.

use std::io;

/// Result alias used across the engine.
pub type Result<T> = std::result::Result<T, EngineError>;

/// Errors produced by the PDF engine.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// Underlying I/O failure.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// The file could not be parsed as a PDF.
    #[error("cannot parse PDF: {0}")]
    Parse(String),
    /// The document is encrypted and no (correct) password was supplied.
    #[error("the document is encrypted and requires a password")]
    PasswordRequired,
    /// A password was supplied but it does not open the document.
    #[error("that password does not open the document")]
    WrongPassword,
    /// The document structure is damaged in a way that blocks the operation.
    #[error("malformed document: {0}")]
    Malformed(String),
    /// The requested operation is outside the supported subset.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// A page index or id does not exist.
    #[error("page {0} does not exist")]
    NoSuchPage(usize),
    /// An object id referenced by the caller does not exist.
    #[error("object {0} {1} does not exist")]
    NoSuchObject(u32, u16),
    /// A resource limit (size, depth, time) was hit.
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
    /// The text contains characters the font cannot show (and that were not silently dropped).
    #[error("the font “{font}” has no glyph for: {chars}")]
    MissingGlyphs {
        /// Display name of the font.
        font: String,
        /// The characters, space separated.
        chars: String,
    },
    /// The referenced page object changed since it was listed.
    #[error("the page content changed since this object was selected")]
    StaleReference,
    /// Invalid argument supplied by the caller.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// The file on disk changed since it was opened or last saved.
    #[error("the file was modified by another program since it was opened")]
    ExternallyModified,
    /// Saving failed; the original file was left untouched.
    #[error("save failed: {0}")]
    Save(String),
    /// The rendering backend failed.
    #[error("render failed: {0}")]
    Render(String),
}

impl From<lopdf::Error> for EngineError {
    fn from(e: lopdf::Error) -> Self {
        EngineError::Malformed(sanitize(&e.to_string()))
    }
}

/// Strip control characters and cap the length of externally-derived text
/// before it is embedded in an error message.
pub fn sanitize(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(200).collect()
}
