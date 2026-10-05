//! "Convert to PDF/A": show what would change (or why the document cannot be converted), then
//! write a PDF/A-2b copy on a background thread. The original is never modified.

use crate::dialogs::modal;
use crate::optimize_ui::human_size;
use crate::state::*;
use egui::RichText;
use pdf_engine::pdfa::{PdfaReport, analyze, convert_bytes};
use pdf_engine::save::{SaveOptions, write_atomic};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

type Analysis = Result<PdfaReport, String>;

/// State of the PDF/A dialog.
pub struct PdfaDialogState {
    rx: Option<Receiver<Analysis>>,
    pub result: Option<Analysis>,
    pub signed: bool,
    pub error: Option<String>,
}

impl App {
    pub fn open_pdfa_dialog(&mut self) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        if tab.session.doc().capabilities().encrypted {
            self.notify_error("Encrypted documents cannot be converted to PDF/A.");
            return;
        }
        let signed = tab.session.doc().capabilities().has_signatures;
        let Ok(snap) = tab.session.snapshot() else {
            self.notify_error("Could not prepare the document for checking.");
            return;
        };
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(|| analyze(&snap.bytes).map_err(|e| e.to_string()))
                .unwrap_or_else(|_| Err("The check stopped unexpectedly.".into()));
            let _ = tx.send(r);
        });
        self.dialog = Some(Dialog::PdfA(Box::new(PdfaDialogState {
            rx: Some(rx),
            result: None,
            signed,
            error: None,
        })));
    }

    pub fn dialog_pdfa(&mut self, ctx: &egui::Context, st: &mut PdfaDialogState) -> bool {
        if let Some(rx) = &st.rx {
            match rx.try_recv() {
                Ok(r) => {
                    st.result = Some(r);
                    st.rx = None;
                }
                Err(TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(80))
                }
                Err(TryRecvError::Disconnected) => {
                    st.result = Some(Err("The check stopped unexpectedly.".into()));
                    st.rx = None;
                }
            }
        }
        let mut go = false;
        let mut cancel = false;
        let dim = self.pal.text_dim;
        let danger = self.pal.danger;
        modal(ctx, "pdfa", |ui| {
            ui.set_max_width(520.0);
            ui.heading("Convert to PDF/A");
            ui.label(
                "Saves a PDF/A-2b copy for long-term archiving. The open document and its file are not changed.",
            );
            ui.add_space(6.0);
            match &st.result {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Checking the document…");
                    });
                }
                Some(Err(e)) => {
                    ui.colored_label(danger, e);
                }
                Some(Ok(rep)) => {
                    if !rep.blockers.is_empty() {
                        ui.label(
                            RichText::new("This document cannot be converted as it is:").strong(),
                        );
                        for b in &rep.blockers {
                            ui.colored_label(danger, format!("• {b}"));
                        }
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("BergPDF will not change fonts or colours to force a result, because the copy would no longer be the same document.")
                                .size(12.0)
                                .color(dim),
                        );
                    } else {
                        ui.label(RichText::new("BergPDF will:").strong());
                        for f in &rep.fixes {
                            ui.label(format!("• {f}"));
                        }
                        for r in &rep.removed {
                            ui.label(format!("• remove {r}"));
                        }
                        ui.add_space(4.0);
                        for n in &rep.notes {
                            ui.label(RichText::new(n).size(12.0).color(dim));
                        }
                    }
                    if st.signed {
                        ui.add_space(4.0);
                        ui.colored_label(
                            danger,
                            "This document has digital signatures. They will no longer be valid in the converted copy.",
                        );
                    }
                }
            }
            if let Some(e) = &st.error {
                ui.colored_label(danger, e);
            }
            ui.add_space(8.0);
            let can = matches!(&st.result, Some(Ok(r)) if r.can_convert());
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(can, egui::Button::new("Choose file and convert…"))
                    .clicked()
                {
                    go = true;
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
        });
        if cancel {
            return false;
        }
        if go {
            match self.start_pdfa() {
                Ok(started) => return !started,
                Err(e) => st.error = Some(e),
            }
        }
        true
    }

    /// Ask for a destination and start converting. `Ok(false)`: the picker was cancelled.
    fn start_pdfa(&mut self) -> Result<bool, String> {
        let ti = self.active;
        let Some(tab) = self.tabs.get_mut(ti) else {
            return Ok(false);
        };
        let suggested = format!("{}-pdfa.pdf", tab.session.title.trim_end_matches(".pdf"));
        let current = tab.session.path.clone();
        let snap = tab
            .session
            .snapshot()
            .map_err(|e| format!("Could not prepare the document: {e}"))?;
        let pages = tab.session.doc().page_count();
        let start = current.as_deref().and_then(|p| p.parent());
        let Some(dest) = platform::dialogs::pick_save_pdf(&suggested, start) else {
            return Ok(false);
        };
        if current.as_deref() == Some(dest.as_path()) {
            return Err("Choose a different file name: the original is never overwritten.".into());
        }
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(|| {
                let (bytes, _rep) = convert_bytes(&snap.bytes).map_err(|e| e.to_string())?;
                write_atomic(
                    &dest,
                    &bytes,
                    &SaveOptions {
                        expected_pages: Some(pages),
                        ..SaveOptions::default()
                    },
                )
                .map_err(|e| e.to_string())?;
                let name = dest
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                Ok::<String, String>(format!(
                    "Saved {name} ({}) prepared as PDF/A-2b. Check it with a PDF/A validator if conformance matters.",
                    human_size(bytes.len())
                ))
            });
            let _ =
                tx.send(r.unwrap_or_else(|_| Err("The conversion stopped unexpectedly.".into())));
        });
        self.exports.push(crate::docops_ui::ExportJob { rx });
        self.notify("Converting to PDF/A…");
        Ok(true)
    }
}
