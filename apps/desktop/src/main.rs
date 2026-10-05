//! BergPDF desktop entry point.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod app;
mod canvas;
mod chrome;
mod contentedit;
mod copilot_ui;
mod ctxmenu;
mod dialogs;
mod docops_ui;
mod exec;
mod fontpick;
mod forms_ui;
mod icons;
mod interaction;
mod measure_ui;
mod ocr_ui;
mod optimize_ui;
mod pagefilter;
mod pdfa_ui;
mod sidebar;
mod sign_ui;
mod snap_ui;
mod state;
mod theme;
mod translate_ui;

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
        .filter(|p| {
            p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
                || platform::dialogs::is_image_path(p)
        })
        .collect();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("BergPDF")
            .with_icon(egui::IconData {
                rgba: brand::render_rgba(256, true),
                width: 256,
                height: 256,
            })
            .with_inner_size([1360.0, 880.0])
            .with_min_inner_size([760.0, 520.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "BergPDF",
        options,
        Box::new(move |cc| Ok(Box::new(state::App::new_app(cc, files)))),
    )
}
