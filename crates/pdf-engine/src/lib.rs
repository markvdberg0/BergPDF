//! BergPDF engine: parsing, preservation, rendering adapter, text extraction,
//! annotations, page/content editing and safe serialization.
//!
//! The editable representation is a [`lopdf::Document`]; rendering goes through
//! hayro using *byte snapshots* serialised from that single authoritative state.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod annot;
pub mod caps;
pub mod compare;
pub mod content;
pub mod doc;
pub mod error;
pub mod fontembed;
pub mod formedit;
pub mod forms;
pub mod formscripts;
pub mod geom;
pub mod icc;
pub mod imagedoc;
pub mod imageembed;
pub mod inlinetr;
pub mod measure;
pub mod meta;
pub mod nav;
pub mod objutil;
pub mod optimize;
pub mod outline;
pub mod pagecontent;
pub mod pagelabels;
pub mod pagemarks;
pub mod pageops;
pub mod pdfa;
pub mod protect;
pub mod redact;
pub mod render;
pub mod save;
pub mod serialize;
pub mod sign;
pub mod snap;
pub mod stampimage;
pub mod sysfonts;
pub mod text;
pub mod textdoc;
pub mod textfont;

pub use error::{EngineError, Result};
/// PDF object identifier (re-exported so the UI need not depend on the PDF library).
pub use lopdf::ObjectId;
