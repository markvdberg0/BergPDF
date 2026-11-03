//! File ▸ Compare Documents: pick the older and the newer version, compare their text on a
//! background thread, and open a copy of the newer version with the changes marked (green: inserted,
//! orange: replaced, red notes: deleted). The comments panel then lists every change.

use crate::dialogs::modal;
use crate::i18n::tr;
use crate::state::*;
use crate::tf;
use egui::RichText;
use pdf_engine::compare::{self, Comparison};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

/// The Compare dialog.
pub struct CompareState {
    older: Option<PathBuf>,
    /// The newer version is the open document (`None`) or another file.
    newer: Option<PathBuf>,
    error: Option<String>,
}

/// A running comparison.
pub struct CompareJob {
    rx: mpsc::Receiver<Result<Comparison, String>>,
    cancel: Arc<AtomicBool>,
    newer: Arc<Vec<u8>>,
    title: String,
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn read_pdf(p: &std::path::Path) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(p).map_err(|e| tf!("Could not read {}: {}", p.display(), e))?;
    match pdf_engine::doc::PdfDocument::open(
        bytes.clone(),
        &pdf_engine::doc::OpenOptions::default(),
    ) {
        Ok(_) => Ok(bytes),
        Err(pdf_engine::EngineError::PasswordRequired) => Err(tf!(
            "{} is password-protected. Open it, remove the password and save a copy to compare it.",
            file_name(p)
        )),
        Err(e) => Err(tf!("{} cannot be read: {}", file_name(p), e)),
    }
}

impl App {
    /// File ▸ Compare Documents…
    pub fn open_compare(&mut self) {
        self.dialog = Some(Dialog::Compare(Box::new(CompareState {
            older: None,
            newer: None,
            error: None,
        })));
    }

    pub fn dialog_compare(&mut self, ctx: &egui::Context, st: &mut CompareState) -> bool {
        let dim = self.pal.text_dim;
        let running = self.compare_job.is_some();
        let mut choice: Option<u8> = None;
        let current = self.active_tab().map(|t| t.session.title.clone());
        modal(ctx, "compare", |ui| {
            ui.heading(tr("Compare documents"));
            ui.label(
                RichText::new(tr("Finds the text that was inserted, deleted or replaced between two versions. The result is a copy of the newer version with the changes marked; your files are not changed. Scanned pages need OCR first."))
                    .size(12.0)
                    .color(dim),
            );
            ui.add_space(4.0);
            crate::ui_kit::form_row(ui, tr("Older"), |ui| {
                if ui.button(tr("Choose file…")).clicked() {
                    choice = Some(1);
                }
                ui.label(
                    st.older
                        .as_deref()
                        .map(file_name)
                        .unwrap_or_else(|| tr("(none chosen)").into()),
                );
            });
            crate::ui_kit::form_row(ui, tr("Newer"), |ui| {
                if let Some(t) = &current
                    && ui
                        .radio(st.newer.is_none(), tf!("This document ({})", t))
                        .clicked()
                {
                    st.newer = None;
                }
                if ui.button(tr("Other file…")).clicked() {
                    choice = Some(2);
                }
                if let Some(p) = &st.newer {
                    ui.label(file_name(p));
                }
            });
            if let Some(e) = &st.error {
                ui.colored_label(self.pal.danger, e);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if running {
                    ui.spinner();
                    ui.label(tr("Comparing…"));
                    if ui.button(tr("Cancel")).clicked() {
                        choice = Some(4);
                    }
                } else {
                    let ready = st.older.is_some() && (st.newer.is_some() || current.is_some());
                    if crate::ui_kit::primary_enabled(ui, ready, tr("Compare")).clicked() {
                        choice = Some(3);
                    }
                    if ui.button(tr("Close")).clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        choice = Some(0);
                    }
                }
            });
        });
        match choice {
            None => true,
            Some(0) => false,
            Some(1) => {
                if let Some(p) = platform::dialogs::pick_open_pdfs(None).into_iter().next() {
                    st.older = Some(p);
                    st.error = None;
                }
                true
            }
            Some(2) => {
                if let Some(p) = platform::dialogs::pick_open_pdfs(None).into_iter().next() {
                    st.newer = Some(p);
                    st.error = None;
                }
                true
            }
            Some(4) => {
                if let Some(j) = &self.compare_job {
                    j.cancel.store(true, Ordering::Relaxed);
                }
                true
            }
            Some(_) => {
                match self.start_compare(st) {
                    Ok(()) => {}
                    Err(e) => st.error = Some(e),
                }
                true
            }
        }
    }

    fn start_compare(&mut self, st: &CompareState) -> Result<(), String> {
        let Some(older) = &st.older else {
            return Err(tr("Choose the older version first.").into());
        };
        let old = read_pdf(older)?;
        let (newer, title) = match &st.newer {
            Some(p) => (read_pdf(p)?, file_name(p)),
            None => {
                let tab = self
                    .tabs
                    .get_mut(self.active)
                    .ok_or_else(|| tr("Open the newer version first.").to_string())?;
                let snap = tab.session.snapshot().map_err(|e| e.to_string())?;
                (snap.bytes.to_vec(), tab.session.title.clone())
            }
        };
        let newer = Arc::new(newer);
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (n, c) = (newer.clone(), cancel.clone());
        std::thread::spawn(move || {
            let r = compare::compare(Arc::new(old), n, &c).map_err(|e| e.to_string());
            let _ = tx.send(r);
        });
        self.compare_job = Some(CompareJob {
            rx,
            cancel,
            newer,
            title: format!("{} ↔ {}", title.trim_end_matches(".pdf"), file_name(older)),
        });
        Ok(())
    }

    /// Collect a finished comparison (called every frame).
    pub fn poll_compare(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.compare_job else { return };
        let result = match job.rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(tr("The comparison stopped unexpectedly.").into())
            }
        };
        let Some(job) = self.compare_job.take() else {
            return;
        };
        if matches!(self.dialog, Some(Dialog::Compare(_))) {
            self.dialog = None;
        }
        let cmp = match result {
            Ok(c) => c,
            Err(e) if job.cancel.load(Ordering::Relaxed) => {
                let _ = e;
                self.notify(tr("Comparison cancelled."));
                return;
            }
            Err(e) => {
                self.notify_error(tf!("Could not compare: {}", e));
                return;
            }
        };
        if cmp.old_words == 0 || cmp.new_words == 0 {
            self.notify_error(tr(
                "One of the versions has no text to compare (a scan?). Run OCR on it first.",
            ));
            return;
        }
        if cmp.changes.is_empty() {
            self.notify(tr("The text of both versions is the same."));
            return;
        }
        let mut session = match editor_core::session::DocumentSession::open_bytes(
            job.newer.as_ref().clone(),
            &format!("{}.pdf", job.title),
        ) {
            Ok(s) => s,
            Err(e) => {
                self.notify_error(tf!("Could not open the result: {}", e));
                return;
            }
        };
        let pages = session.doc().page_ids().unwrap_or_default();
        let texts = [tr("Inserted"), tr("Deleted"), tr("Replaced"), tr("Was:")];
        if let Err(e) = session.execute(tr("Mark changes"), |tx| {
            compare::annotate(tx, &pages, &cmp, texts)
        }) {
            self.notify_error(tf!("Could not mark the changes: {}", e));
            return;
        }
        session.mark_unsaved();
        self.add_tab(session);
        self.right_tab = RightTab::Comments;
        self.prefs.show_right_sidebar = true;
        let (ins, del, rep) = cmp.counts();
        let more = if cmp.changes.len() > compare::MAX_MARKED {
            tf!(" (the first {} are marked)", compare::MAX_MARKED)
        } else {
            String::new()
        };
        self.notify(tf!(
            "{} inserted, {} deleted, {} replaced{}. The comments panel lists them.",
            ins,
            del,
            rep,
            more
        ));
    }
}
