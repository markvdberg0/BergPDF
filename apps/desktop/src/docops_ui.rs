//! Document-level operations with their own small dialogs: document properties, flattening
//! form fields, and exporting a page as a PNG image (rendered on a background thread).

use crate::dialogs::modal;
use crate::i18n::tr;
use crate::state::*;
use crate::tf;
use egui::RichText;
use pdf_engine::meta::{self, InfoEdit};
use pdf_engine::render::{export_scale, with_session};
use std::sync::mpsc;

/// A running page export.
pub struct ExportJob {
    pub rx: mpsc::Receiver<Result<String, String>>,
}

impl App {
    /// Open the Document Properties dialog for the active document.
    pub fn open_properties(&mut self) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let doc = tab.session.doc();
        let info = meta::read_info(doc);
        let st = PropsState {
            title: info.title,
            author: info.author,
            subject: info.subject,
            keywords: info.keywords,
            creator: info.creator,
            producer: info.producer,
            created: meta::format_pdf_date(&info.created),
            modified: meta::format_pdf_date(&info.modified),
            pages: doc.page_count(),
            file_size: doc.original_bytes().len(),
            xmp: meta::has_xmp(doc),
            can_edit: doc.capabilities().can_edit,
            error: None,
        };
        self.dialog = Some(Dialog::Properties(Box::new(st)));
    }

    /// The Document Properties dialog. Returns whether it stays open.
    pub fn dialog_properties(&mut self, ctx: &egui::Context, st: &mut PropsState) -> bool {
        let mut choice: Option<bool> = None;
        modal(ctx, "doc_props", |ui| {
            ui.heading(tr("Document properties"));
            ui.add_space(4.0);
            egui::Grid::new("props_grid")
                .num_columns(2)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    for (label, value) in [
                        (tr("Title"), &mut st.title),
                        (tr("Author"), &mut st.author),
                        (tr("Subject"), &mut st.subject),
                        (tr("Keywords"), &mut st.keywords),
                    ] {
                        ui.label(label);
                        ui.add_enabled(
                            st.can_edit,
                            crate::ui_kit::singleline(value).desired_width(320.0),
                        );
                        ui.end_row();
                    }
                });
            ui.add_space(6.0);
            let dim = self.pal.text_dim;
            let line = |ui: &mut egui::Ui, k: &str, v: String| {
                if !v.is_empty() {
                    ui.label(RichText::new(format!("{k}: {v}")).size(12.0).color(dim));
                }
            };
            line(ui, tr("Pages"), st.pages.to_string());
            line(
                ui,
                tr("File size"),
                format!("{:.1} KB", st.file_size as f64 / 1024.0),
            );
            line(ui, tr("Created by"), st.creator.clone());
            line(ui, tr("PDF producer"), st.producer.clone());
            line(ui, tr("Created"), st.created.clone());
            line(ui, tr("Modified"), st.modified.clone());
            if st.xmp {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(tr("This document also has XMP metadata, which is not updated by these edits; other programs may show the old values."))
                        .size(12.0)
                        .color(self.pal.danger),
                );
            }
            if !st.can_edit {
                ui.label(
                    RichText::new(tr(
                        "This document cannot be edited, so its properties are read-only.",
                    ))
                    .size(12.0)
                    .color(dim),
                );
            }
            if let Some(e) = &st.error {
                ui.colored_label(self.pal.danger, e);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(st.can_edit, egui::Button::new("OK"))
                    .clicked()
                {
                    choice = Some(true);
                }
                if ui
                    .button(if st.can_edit {
                        tr("Cancel")
                    } else {
                        tr("Close")
                    })
                    .clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    choice = Some(false);
                }
            });
        });
        match choice {
            None => true,
            Some(false) => false,
            Some(true) => {
                let edit = InfoEdit {
                    title: st.title.trim().to_string(),
                    author: st.author.trim().to_string(),
                    subject: st.subject.trim().to_string(),
                    keywords: st.keywords.trim().to_string(),
                };
                let current = meta::read_info(self.tabs[self.active].session.doc());
                let unchanged = edit.title == current.title
                    && edit.author == current.author
                    && edit.subject == current.subject
                    && edit.keywords == current.keywords;
                if unchanged {
                    return false;
                }
                match self.tabs[self.active]
                    .session
                    .execute(tr("Edit document properties"), |tx| {
                        meta::set_info(tx, &edit)
                    }) {
                    Ok(()) => false,
                    Err(e) => {
                        st.error = Some(e.to_string());
                        true
                    }
                }
            }
        }
    }

    /// Ask before flattening (the result is undoable, but it is a big change).
    pub fn request_flatten(&mut self) {
        let n = self.form_for().fields.len();
        if n == 0 {
            self.notify(tr("This document has no form fields."));
            return;
        }
        self.dialog = Some(Dialog::ConfirmFlatten { fields: n });
    }

    /// Confirmation dialog for flattening. Returns whether it stays open.
    pub fn dialog_confirm_flatten(&mut self, ctx: &egui::Context, fields: usize) -> bool {
        let mut choice: Option<bool> = None;
        modal(ctx, "confirm_flatten", |ui| {
            ui.heading(tr("Flatten form fields?"));
            ui.label(tf!("All {} form field(s) become plain page content: their current values stay visible but can no longer be edited, and the form is removed from the document.", fields));
            ui.label(
                RichText::new(tr("You can undo this until you close the document."))
                    .size(12.0)
                    .color(self.pal.text_dim),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if crate::ui_kit::primary_button(ui, tr("Flatten")).clicked() {
                    choice = Some(true);
                }
                if ui.button(tr("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    choice = Some(false);
                }
            });
        });
        match choice {
            None => true,
            Some(false) => false,
            Some(true) => {
                match self.tabs[self.active].session.execute(
                    tr("Flatten form fields"),
                    pdf_engine::forms::flatten_document,
                ) {
                    Ok((baked, dropped)) => {
                        let extra = if dropped > 0 {
                            tf!(" ({} empty or hidden field(s) removed)", dropped)
                        } else {
                            String::new()
                        };
                        self.notify(tf!("Flattened {} field(s){}.", baked, extra));
                    }
                    Err(e) => self.notify_error(e.to_string()),
                }
                false
            }
        }
    }

    /// Export the current page as a PNG (150 dpi, reduced for very large pages).
    pub fn export_page_image(&mut self) {
        let ti = self.active;
        let Some(tab) = self.tabs.get_mut(ti) else {
            return;
        };
        let cur = tab.session.view.current_page;
        let rot = tab.session.view.rotation;
        let Ok(pages) = tab.session.pages() else {
            return;
        };
        let Some(page) = pages.get(cur) else { return };
        let geometry = page.geometry;
        let size = geometry.view_size(rot);
        let suggested = format!(
            "{}-page-{}.png",
            tab.session.title.trim_end_matches(".pdf"),
            cur + 1
        );
        let Ok(snap) = tab.session.snapshot() else {
            self.notify_error(tr("Could not prepare the document for export."));
            return;
        };
        let Some(dest) = platform::dialogs::pick_save_png(&suggested) else {
            return;
        };
        let scale = export_scale(size.width, size.height, 150.0);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = with_session(snap.bytes.clone(), |s| {
                s.render_page(cur, geometry, rot, scale)?.to_png()
            })
            .map_err(|e| e.to_string())
            .and_then(|png| {
                let tmp = dest.with_extension("png.partial");
                std::fs::write(&tmp, &png)
                    .and_then(|()| std::fs::rename(&tmp, &dest))
                    .map_err(|e| tf!("Could not write {}: {}", dest.display(), e))?;
                Ok(tf!("Saved {}", dest.display()))
            });
            let _ = tx.send(result);
        });
        self.exports.push(ExportJob { rx });
        self.notify(tr("Exporting page…"));
    }

    /// Collect finished exports (called every frame).
    pub fn poll_exports(&mut self, ctx: &egui::Context) {
        if self.exports.is_empty() {
            return;
        }
        let mut done: Vec<Result<String, String>> = Vec::new();
        self.exports.retain(|j| match j.rx.try_recv() {
            Ok(r) => {
                done.push(r);
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => {
                done.push(Err(tr("The export stopped unexpectedly.").into()));
                false
            }
        });
        for r in done {
            match r {
                Ok(m) => self.notify(m),
                Err(e) => self.notify_error(e),
            }
        }
        if !self.exports.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}
