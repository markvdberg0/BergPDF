//! "Save As Optimized": choose how aggressively to shrink the document, then write a new,
//! smaller copy on a background thread. The open document and its file are never changed.

use crate::dialogs::modal;
use crate::i18n::tr;
use crate::state::*;
use crate::tf;
use egui::RichText;
use pdf_engine::optimize::{OptimizeOptions, optimize_bytes};
use pdf_engine::save::{SaveOptions, write_atomic};

/// The three presets offered.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Preset {
    Lossless,
    Balanced,
    Smallest,
}

impl Preset {
    fn options(self) -> OptimizeOptions {
        match self {
            Preset::Lossless => OptimizeOptions::lossless(),
            Preset::Balanced => OptimizeOptions::balanced(),
            Preset::Smallest => OptimizeOptions::smallest(),
        }
    }
}

/// State of the optimize dialog.
pub struct OptimizeDialogState {
    pub preset: Preset,
    pub size: usize,
    pub signed: bool,
    pub error: Option<String>,
}

pub fn human_size(n: usize) -> String {
    let n = n as f64;
    if n >= 1024.0 * 1024.0 {
        format!("{:.1} MB", n / (1024.0 * 1024.0))
    } else {
        format!("{:.0} KB", (n / 1024.0).max(1.0))
    }
}

impl App {
    pub fn open_optimize_dialog(&mut self) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let doc = tab.session.doc();
        if doc.capabilities().encrypted {
            self.notify_error(tr(
                "A password-protected document cannot be optimized. Remove its protection first (use Protect in the Document group).",
            ));
            return;
        }
        self.dialog = Some(Dialog::Optimize(Box::new(OptimizeDialogState {
            preset: Preset::Balanced,
            size: doc.original_bytes().len(),
            signed: doc.capabilities().has_signatures,
            error: None,
        })));
    }

    pub fn dialog_optimize(&mut self, ctx: &egui::Context, st: &mut OptimizeDialogState) -> bool {
        let mut go = false;
        let mut cancel = false;
        modal(ctx, "optimize", |ui| {
            ui.heading(tr("Save As Optimized"));
            ui.label(tf!("Writes a smaller copy of this document (now about {}). The open document and its file are not changed.", human_size(st.size)));
            ui.add_space(6.0);
            for (p, title, text) in [
                (
                    Preset::Lossless,
                    tr("Lossless"),
                    tr(
                        "Removes unused data, merges duplicates and compresses better. Pictures stay exactly as they are.",
                    ),
                ),
                (
                    Preset::Balanced,
                    tr("Balanced (recommended)"),
                    tr(
                        "Also reduces pictures above 150 dpi (photographs become JPEG, quality 75). Fine for screen and office printing.",
                    ),
                ),
                (
                    Preset::Smallest,
                    tr("Smallest"),
                    tr(
                        "Reduces pictures above 96 dpi (JPEG quality 60). For email and screen only; text and drawings stay sharp.",
                    ),
                ),
            ] {
                ui.radio_value(&mut st.preset, p, RichText::new(title).strong());
                ui.label(RichText::new(text).size(12.0).color(self.pal.text_dim));
                ui.add_space(2.0);
            }
            if st.preset != Preset::Lossless {
                ui.label(
                    RichText::new(tr("Reducing pictures is permanent in the new copy; keep the original if you need full quality."))
                        .size(12.0)
                        .color(self.pal.text_dim),
                );
            }
            if st.signed {
                ui.add_space(4.0);
                ui.colored_label(
                    self.pal.danger,
                    tr("This document has digital signatures. The optimized copy is rewritten, so those signatures will no longer be valid."),
                );
            }
            if let Some(e) = &st.error {
                ui.colored_label(self.pal.danger, e);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tr("Choose file and save…")).clicked() {
                    go = true;
                }
                if ui.button(tr("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    cancel = true;
                }
            });
        });
        if cancel {
            return false;
        }
        if go {
            match self.start_optimize(st.preset) {
                // Started: close the dialog. Picker cancelled: stay so the choice can be changed.
                Ok(started) => return !started,
                Err(e) => st.error = Some(e),
            }
        }
        true
    }

    /// Ask for a destination and start the job. `Ok(false)` means the user cancelled the picker.
    fn start_optimize(&mut self, preset: Preset) -> Result<bool, String> {
        let ti = self.active;
        let Some(tab) = self.tabs.get_mut(ti) else {
            return Ok(false);
        };
        let suggested = format!(
            "{}-optimized.pdf",
            tab.session.title.trim_end_matches(".pdf")
        );
        let current = tab.session.path.clone();
        let snap = tab
            .session
            .snapshot()
            .map_err(|e| tf!("Could not prepare the document: {}", e))?;
        let pages = tab.session.doc().page_count();
        let start = current.as_deref().and_then(|p| p.parent());
        let Some(dest) = platform::dialogs::pick_save_pdf(&suggested, start) else {
            return Ok(false);
        };
        if current.as_deref() == Some(dest.as_path()) {
            return Err(
                tr("Choose a different file name: the original is never overwritten.").into(),
            );
        }
        let opts = preset.options();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(|| {
                let (bytes, rep) = optimize_bytes(&snap.bytes, &opts).map_err(|e| e.to_string())?;
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
                Ok::<String, String>(if rep.kept_original {
                    tf!(
                        "Saved {}: no further reduction was possible, so it is a plain copy ({}).",
                        name,
                        human_size(rep.after)
                    )
                } else {
                    let pct = 100.0 - rep.after as f64 * 100.0 / rep.before.max(1) as f64;
                    let mut extra = Vec::new();
                    if rep.images_downsampled > 0 {
                        extra.push(tf!("{} picture(s) reduced", rep.images_downsampled));
                    }
                    if rep.duplicates_merged > 0 {
                        extra.push(tf!("{} duplicate(s) merged", rep.duplicates_merged));
                    }
                    tf!(
                        "Saved {}: {} → {} (−{} %){}",
                        name,
                        human_size(rep.before),
                        human_size(rep.after),
                        format!("{:.0}", pct),
                        if extra.is_empty() {
                            String::new()
                        } else {
                            format!(" — {}", extra.join(", "))
                        }
                    )
                })
            });
            let _ = tx
                .send(r.unwrap_or_else(|_| Err(tr("The optimizer stopped unexpectedly.").into())));
        });
        self.exports.push(crate::docops_ui::ExportJob { rx });
        self.notify(tr("Optimizing…"));
        Ok(true)
    }
}
