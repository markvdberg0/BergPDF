//! BergPDF desktop entry point.
// Release builds on Windows are GUI-subsystem programs, so no console window opens beside the app.
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]
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
mod gpu;
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
    let prefs = platform::dirs::read_text(&platform::dirs::prefs_file())
        .and_then(|s| editor_core::prefs::Preferences::from_toml(&s).ok())
        .unwrap_or_default();
    let options = |prefer_dx12: bool| eframe::NativeOptions {
        wgpu_options: gpu::configuration(&prefs.graphics, prefer_dx12),
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
    let first_files = files.clone();
    let result = eframe::run_native(
        "BergPDF",
        options(true),
        Box::new(move |cc| Ok(Box::new(state::App::new_app(cc, first_files)))),
    );
    // If DirectX 12 could not be started, try again with the library's own choice.
    if result.is_err()
        && prefs.graphics.backend == editor_core::prefs::GfxBackend::Auto
        && gpu::dx12_is_default()
    {
        tracing::warn!("DirectX 12 could not be started; trying the default graphics API");
        return eframe::run_native(
            "BergPDF",
            options(false),
            Box::new(move |cc| Ok(Box::new(state::App::new_app(cc, files)))),
        );
    }
    result
}
