//! Editor core: document sessions, commands, history, view layout, tools and background jobs.
//! No UI-toolkit types appear here, so everything is unit-testable without a window.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod command;
pub mod jobs;
pub mod platform_kind;
pub mod prefs;
pub mod search;
pub mod selection;
pub mod session;
pub mod tiles;
pub mod tools;
pub mod view;
