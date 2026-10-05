//! Optical character recognition: turns scanned pages into searchable pages by adding an
//! invisible text layer. Recognition runs on a background thread with progress and cancel.
//!
//! The recognition models are not bundled (see `pdf-ocr` and docs/DEPENDENCIES.md); when they are
//! missing the dialog says exactly where to put them instead of failing silently.

use crate::dialogs::modal;
use crate::state::*;
use editor_core::ocr::{OcrPageSpec, recognize_pages};
use egui::RichText;
use pdf_engine::doc::PageId;
use pdf_engine::pagecontent::{OcrWord, add_ocr_text_layers, page_has_text};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};

/// Where the model files are looked for: `BERG_OCR_MODELS` if set, else the data directory.
pub fn model_dir() -> std::path::PathBuf {
    std::env::var_os("BERG_OCR_MODELS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(platform::dirs::ocr_models_dir)
}

type OcrResult = Result<Vec<(PageId, Vec<OcrWord>)>, String>;

/// A running recognition.
pub struct OcrJob {
    rx: mpsc::Receiver<OcrResult>,
    done: Arc<AtomicUsize>,
    total: usize,
    cancel: Arc<AtomicBool>,
}

/// Which pages to recognise.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OcrScope {
    Current,
    Selected,
    All,
}

/// State of the OCR dialog.
pub struct OcrDialogState {
    pub scope: OcrScope,
    pub include_pages_with_text: bool,
    pub models_ok: bool,
    pub error: Option<String>,
}

impl App {
    /// Open the OCR dialog.
    pub fn open_ocr_dialog(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        if !self.tabs[self.active].session.doc().capabilities().can_edit {
            self.notify_error("This document cannot be modified, so text cannot be added to it.");
            return;
        }
        if self.ocr_job.is_some() {
            self.notify("Text recognition is already running.");
            return;
        }
        self.dialog = Some(Dialog::Ocr(Box::new(OcrDialogState {
            scope: OcrScope::All,
            include_pages_with_text: false,
            models_ok: pdf_ocr::models_present(&model_dir()),
            error: None,
        })));
    }

    /// The OCR dialog. Returns whether it stays open.
    pub fn dialog_ocr(&mut self, ctx: &egui::Context, st: &mut OcrDialogState) -> bool {
        let mut start = false;
        let mut cancel = false;
        let dir = model_dir();
        modal(ctx, "ocr_dialog", |ui| {
            ui.heading("Recognize text (OCR)");
            ui.label(
                RichText::new("Makes scanned pages searchable and their text selectable by adding an invisible text layer. How the pages look does not change.")
                    .size(12.0)
                    .color(self.pal.text_dim),
            );
            ui.add_space(6.0);
            if !st.models_ok {
                ui.colored_label(
                    self.pal.danger,
                    "The text-recognition models are not installed.",
                );
                ui.label(
                    RichText::new(
                        "BergPDF works offline and does not download anything by itself. Put the two model files \
                         (about 12 MB) in this folder:",
                    )
                    .size(12.0),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(dir.display().to_string())
                            .monospace()
                            .size(12.0),
                    )
                    .selectable(true),
                );
                ui.label(
                    RichText::new(format!(
                        "  {}\n  {}",
                        pdf_ocr::MODEL_FILES.0,
                        pdf_ocr::MODEL_FILES.1
                    ))
                    .monospace()
                    .size(12.0),
                );
                ui.label(
                    RichText::new("Developers: `cargo xtask fetch-ocr-models` downloads and verifies them. Review the models' licence (see docs/DEPENDENCIES.md) before distributing them.")
                        .size(11.5)
                        .color(self.pal.text_dim),
                );
                if ui.button("Copy folder path").clicked() {
                    ctx.copy_text(dir.display().to_string());
                }
                ui.add_space(6.0);
                if ui.button("Check again").clicked() {
                    st.models_ok = pdf_ocr::models_present(&dir);
                }
            } else {
                ui.label("Pages");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut st.scope, OcrScope::All, "All pages");
                    ui.radio_value(&mut st.scope, OcrScope::Current, "Current page");
                    ui.radio_value(&mut st.scope, OcrScope::Selected, "Selected thumbnails");
                });
                ui.checkbox(
                    &mut st.include_pages_with_text,
                    "Also recognise pages that already contain text",
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Works for printed text in the Latin alphabet. Accented letters (é, ë, ü…) are not in the \
                         recognition alphabet and come out as plain or wrong letters; handwriting, very small print \
                         and rotated pages do poorly. Check important numbers by eye.",
                    )
                    .size(11.5)
                    .color(self.pal.text_dim),
                );
            }
            if let Some(e) = &st.error {
                ui.add_space(4.0);
                ui.colored_label(self.pal.danger, e);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(st.models_ok, egui::Button::new("Start"))
                    .clicked()
                {
                    start = true;
                }
                if ui.button("Close").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
        });
        if cancel {
            return false;
        }
        if start {
            match self.start_ocr(st) {
                Ok(()) => return false,
                Err(e) => st.error = Some(e),
            }
        }
        true
    }

    fn start_ocr(&mut self, st: &OcrDialogState) -> Result<(), String> {
        let ti = self.active;
        let selected = self.selected_pages();
        let tab = self.tabs.get_mut(ti).ok_or("No document is open.")?;
        let infos = tab.session.pages().map_err(|e| e.to_string())?;
        let cur = tab.session.view.current_page;
        let mut specs: Vec<OcrPageSpec> = Vec::new();
        for (i, p) in infos.iter().enumerate() {
            let wanted = match st.scope {
                OcrScope::All => true,
                OcrScope::Current => i == cur,
                OcrScope::Selected => selected.contains(&p.id),
            };
            if !wanted {
                continue;
            }
            if !st.include_pages_with_text && page_has_text(tab.session.doc().lopdf(), p.id) {
                continue;
            }
            specs.push(OcrPageSpec {
                page: p.id,
                index: i,
                geometry: p.geometry,
            });
        }
        if specs.is_empty() {
            return Err("There is nothing to recognise: every chosen page already has text. Tick “Also recognise pages that already contain text” to run anyway.".into());
        }
        let snap = tab.session.snapshot().map_err(|e| e.to_string())?;
        let dir = model_dir();
        let (tx, rx) = mpsc::channel();
        let done = Arc::new(AtomicUsize::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let total = specs.len();
        let (d2, c2) = (done.clone(), cancel.clone());
        std::thread::spawn(move || {
            let result: OcrResult = (|| {
                let engine = pdf_ocr::Engine::load(&dir).map_err(|e| e.to_string())?;
                recognize_pages(&engine, snap.bytes.clone(), &specs, &mut |n, _| {
                    d2.store(n, Ordering::Relaxed);
                    !c2.load(Ordering::Relaxed)
                })
                .map_err(|e| e.to_string())
            })();
            let _ = tx.send(result);
        });
        self.ocr_job = Some(OcrJob {
            rx,
            done,
            total,
            cancel,
        });
        self.dialog = Some(Dialog::OcrProgress);
        Ok(())
    }

    /// Progress dialog while recognition runs.
    pub fn dialog_ocr_progress(&mut self, ctx: &egui::Context) -> bool {
        let Some(job) = &self.ocr_job else {
            return false;
        };
        let (done, total) = (job.done.load(Ordering::Relaxed), job.total);
        let mut stop = false;
        modal(ctx, "ocr_progress", |ui| {
            ui.heading("Recognizing text…");
            ui.add(
                egui::ProgressBar::new(done as f32 / total.max(1) as f32)
                    .desired_width(320.0)
                    .text(format!("page {} of {}", (done + 1).min(total), total)),
            );
            ui.label(
                RichText::new("This runs on your computer and can take a few seconds per page.")
                    .size(12.0)
                    .color(self.pal.text_dim),
            );
            if ui.button("Cancel").clicked() {
                stop = true;
            }
        });
        if stop {
            job.cancel.store(true, Ordering::Relaxed);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(150));
        true
    }

    /// Collect a finished recognition and add the text layers (called every frame).
    pub fn poll_ocr(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.ocr_job else { return };
        let outcome = match job.rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(150));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Recognition stopped unexpectedly.".into())
            }
        };
        let cancelled = job.cancel.load(Ordering::Relaxed);
        self.ocr_job = None;
        if matches!(self.dialog, Some(Dialog::OcrProgress)) {
            self.dialog = None;
        }
        match outcome {
            Err(e) => {
                self.dialog = Some(Dialog::Error {
                    title: "Text recognition failed".into(),
                    detail: e,
                });
            }
            Ok(pages) => {
                let Some(tab) = self.tabs.get_mut(self.active) else {
                    return;
                };
                // Pages may have been deleted while recognition ran.
                let alive: Vec<PageId> = tab.session.doc().page_ids().unwrap_or_default();
                let pages: Vec<_> = pages
                    .into_iter()
                    .filter(|(p, _)| alive.contains(p))
                    .collect();
                let n_pages = pages.iter().filter(|(_, w)| !w.is_empty()).count();
                if pages.is_empty() {
                    self.notify(if cancelled {
                        "Cancelled."
                    } else {
                        "No pages were recognised."
                    });
                    return;
                }
                match tab
                    .session
                    .execute("Recognize text", |tx| add_ocr_text_layers(tx, &pages))
                {
                    Ok(words) if words > 0 => self.notify(format!(
                        "Recognised {words} words on {n_pages} page(s){}. Search and copy now work; the page looks the same.",
                        if cancelled { " before cancelling" } else { "" }
                    )),
                    Ok(_) => self.notify("No text was found on those pages."),
                    Err(e) => self.notify_error(e.to_string()),
                }
            }
        }
    }
}
