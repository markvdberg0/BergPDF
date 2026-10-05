//! Optional AI features for BergPDF: PDF Copilot and translation.
//!
//! This is the only crate in the workspace that opens network connections, and only when a
//! function in [`chat`], [`download`] or [`update`] is called in answer to something the user did. It has no UI and no
//! dependency on the rest of the application.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod chat;
pub mod docqa;
pub mod download;
pub mod text;
pub mod translate;
pub mod update;

pub use chat::{AiError, Config, Message, Provider, Reply, Request, Role, chat};
pub use docqa::{Answer, Ask, Mode, Point, ask};
pub use text::{Document, PageText, build_document};
