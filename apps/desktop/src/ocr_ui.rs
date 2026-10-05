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

/// Where the model files are looked for, in order: `BERG_OCR_MODELS`; an `ocr-models` folder next
/// to the program (how a distribution that bundles them ships them); `../Resources/ocr-models`
/// inside a macOS `.app`; the per-user data directory.
fn candidate_dirs() -> Vec<std::path::PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = std::env::var_os("BERG_OCR_MODELS") {
        v.push(std::path::PathBuf::from(d));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        v.push(dir.join("ocr-models"));
        v.push(dir.join("../Resources/ocr-models"));
    }
    v.push(platform::dirs::ocr_models_dir());
    v
}

/// First candidate that holds both model files, else `fallback`.
fn pick_dir(
    candidates: &[std::path::PathBuf],
    fallback: std::path::PathBuf,
    present: impl Fn(&std::path::Path) -> bool,
) -> std::path::PathBuf {
    candidates
        .iter()
        .find(|d| present(d))
        .cloned()
        .unwrap_or(fallback)
}

/// The folder the models are loaded from (or, when there are none yet, where they would go).
pub fn model_dir() -> std::path::PathBuf {
    pick_dir(&candidate_dirs(), install_dir(), pdf_ocr::models_present)
}

/// Where a download puts the models: the `BERG_OCR_MODELS` folder if set, else the per-user data
/// directory (always writable, unlike the program folder).
pub fn install_dir() -> std::path::PathBuf {
    std::env::var_os("BERG_OCR_MODELS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(platform::dirs::ocr_models_dir)
}

/// A running model download.
pub struct ModelDownload {
    rx: mpsc::Receiver<Result<(), String>>,
    bytes: Arc<std::sync::atomic::AtomicU64>,
    total: Arc<std::sync::atomic::AtomicU64>,
    cancel: Arc<AtomicBool>,
}

/// Download both model files into `dir` (verified; files that are already right are kept).
/// `progress(bytes_so_far, total_known_so_far)` covers both files together.
fn download_models(
    dir: &std::path::Path,
    progress: &dyn Fn(u64, u64),
    cancel: &AtomicBool,
) -> Result<(), String> {
    let mut done_before = 0u64;
    for (name, sha) in pdf_ocr::MODEL_SOURCES {
        let dest = dir.join(name);
        if ai_client::download::file_matches(&dest, sha) {
            continue;
        }
        let base = done_before;
        let last = std::sync::atomic::AtomicU64::new(0);
        ai_client::download::fetch_verified(
            &format!("{}{name}", pdf_ocr::MODEL_BASE_URL),
            sha,
            &dest,
            &|got, tot| {
                last.store(got, Ordering::Relaxed);
                // The other file is counted once its size is known.
                progress(base + got, base + tot.unwrap_or(got));
            },
            cancel,
        )
        .map_err(|e| e.to_string())?;
        done_before = base + last.load(Ordering::Relaxed);
    }
    // Attribution travels with the files.
    let _ = std::fs::write(dir.join("NOTICE-OCR.txt"), pdf_ocr::MODEL_NOTICE);
    Ok(())
}

/// `bergpdf --download-ocr-models`: used by the Windows installer; no window, exit code says how it went.
pub fn download_models_blocking() -> Result<(), String> {
    download_models(&install_dir(), &|_, _| {}, &AtomicBool::new(false))
}

fn start_model_download() -> ModelDownload {
    let (tx, rx) = mpsc::channel();
    let bytes = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let total = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    let (b2, t2, c2) = (bytes.clone(), total.clone(), cancel.clone());
    let dir = install_dir();
    std::thread::spawn(move || {
        let r = std::panic::catch_unwind(|| {
            download_models(
                &dir,
                &|got, tot| {
                    b2.store(got, Ordering::Relaxed);
                    t2.store(tot, Ordering::Relaxed);
                },
                &c2,
            )
        });
        let _ = tx.send(r.unwrap_or_else(|_| Err("The download stopped unexpectedly.".into())));
    });
    ModelDownload {
        rx,
        bytes,
        total,
        cancel,
    }
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
    pub download: Option<ModelDownload>,
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
            download: None,
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
        let mut begin_download = false;
        let mut cancel_download = false;
        // Collect the result of a running download.
        if let Some(d) = &st.download {
            match d.rx.try_recv() {
                Ok(r) => {
                    st.download = None;
                    match r {
                        Ok(()) => {
                            st.models_ok = pdf_ocr::models_present(&model_dir());
                            st.error = None;
                            self.notify("OCR models installed.");
                        }
                        Err(e) => st.error = Some(e),
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    st.download = None;
                    st.error = Some("The download stopped unexpectedly.".into());
                }
            }
        }
        let dir = model_dir();
        let host = pdf_ocr::MODEL_BASE_URL
            .split("://")
            .nth(1)
            .unwrap_or("")
            .trim_end_matches('/')
            .to_string();
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
                match &st.download {
                    Some(d) => {
                        let (got, tot) = (
                            d.bytes.load(Ordering::Relaxed),
                            d.total.load(Ordering::Relaxed).max(1),
                        );
                        ui.add(
                            egui::ProgressBar::new((got as f32 / tot as f32).min(1.0))
                                .text(format!("{:.1} MB", got as f64 / 1_048_576.0)),
                        );
                        if ui.button("Cancel download").clicked() {
                            cancel_download = true;
                        }
                    }
                    None => {
                        ui.label(
                            RichText::new(format!(
                                "One click installs them: BergPDF will connect to {host}, download the two model files (about 12 MB) and check them against built-in checksums. Nothing else is sent. After that OCR works offline."
                            ))
                            .size(12.0),
                        );
                        if ui.button("Download the OCR models").clicked() {
                            begin_download = true;
                        }
                    }
                }
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "Or install them yourself (no network needed): put the two model files in this folder:",
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
        if begin_download {
            st.error = None;
            st.download = Some(start_model_download());
        }
        if cancel_download && let Some(d) = st.download.take() {
            d.cancel.store(true, Ordering::Relaxed);
        }
        if cancel {
            if let Some(d) = st.download.take() {
                d.cancel.store(true, Ordering::Relaxed);
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn the_first_folder_with_both_models_wins() {
        let c = [
            PathBuf::from("/a"),
            PathBuf::from("/b"),
            PathBuf::from("/c"),
        ];
        let fb = PathBuf::from("/install");
        assert_eq!(
            pick_dir(&c, fb.clone(), |d| d == std::path::Path::new("/b")),
            PathBuf::from("/b")
        );
        assert_eq!(pick_dir(&c, fb.clone(), |_| true), PathBuf::from("/a"));
        assert_eq!(pick_dir(&c, fb.clone(), |_| false), fb);
    }

    #[test]
    fn a_bundle_next_to_the_program_is_among_the_candidates() {
        let c = candidate_dirs();
        assert!(c.iter().any(|d| d.ends_with("ocr-models")));
        // The per-user directory is always the last resort.
        assert_eq!(c.last().unwrap(), &platform::dirs::ocr_models_dir());
    }
}
