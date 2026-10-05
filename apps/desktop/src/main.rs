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
mod i18n;
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
mod update_ui;

use std::path::PathBuf;

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();
    // Used by the Windows installer: download and verify the OCR models, then exit (no window).
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if let Some(i) = args.iter().position(|a| a == "--download-ocr-models") {
        // An optional folder follows (the installer passes one for all-users installs).
        let dir = args.get(i + 1).map(PathBuf::from);
        return match ocr_ui::download_models_blocking(dir) {
            Ok(()) => Ok(()),
            Err(e) => {
                tracing::warn!("OCR models were not downloaded: {e}");
                std::process::exit(1)
            }
        };
    }
    let files: Vec<PathBuf> = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .filter(|p| {
            p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
                || platform::dialogs::is_image_path(p)
        })
        // Absolute, because a running instance has another working directory.
        .map(|p| std::fs::canonicalize(&p).unwrap_or(p))
        .collect();
    // One BergPDF at a time: a second start hands its files to the first and quits. `--new-instance`
    // (or BERG_MULTI_INSTANCE=1) skips this, e.g. for testing.
    let ctx_cell: std::sync::Arc<std::sync::OnceLock<egui::Context>> = std::sync::Arc::default();
    let mut instance_rx = None;
    if !args.iter().any(|a| a == "--new-instance")
        && std::env::var_os("BERG_MULTI_INSTANCE").is_none()
    {
        let cell = ctx_cell.clone();
        let wake: std::sync::Arc<dyn Fn() + Send + Sync> = std::sync::Arc::new(move || {
            if let Some(c) = cell.get() {
                c.request_repaint();
            }
        });
        match platform::instance::start(&files, wake) {
            platform::instance::Start::Forwarded => return Ok(()),
            platform::instance::Start::Primary(server) => instance_rx = Some(server.rx),
        }
    }
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
        Box::new({
            let cell = ctx_cell.clone();
            move |cc| {
                let _ = cell.set(cc.egui_ctx.clone());
                Ok(Box::new(state::App::new_app(cc, first_files, instance_rx)))
            }
        }),
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
            Box::new(move |cc| {
                let _ = ctx_cell.set(cc.egui_ctx.clone());
                // The first attempt took the receiver; the retry opens just the files.
                Ok(Box::new(state::App::new_app(cc, files, None)))
            }),
        );
    }
    result
}
