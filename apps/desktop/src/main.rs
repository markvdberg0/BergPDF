//! Ferrum PDF desktop entry point.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod app;
mod canvas;
mod chrome;
mod contentedit;
mod dialogs;
mod docops_ui;
mod exec;
mod forms_ui;
mod icons;
mod interaction;
mod measure_ui;
mod pagefilter;
mod sidebar;
mod state;
mod theme;

use std::path::PathBuf;

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();
    let files: Vec<PathBuf> = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")))
        .collect();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Ferrum PDF")
            .with_inner_size([1360.0, 880.0])
            .with_min_inner_size([760.0, 520.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "Ferrum PDF",
        options,
        Box::new(move |cc| Ok(Box::new(state::App::new_app(cc, files)))),
    )
}
