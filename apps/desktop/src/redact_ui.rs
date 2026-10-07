//! Redaction: marking areas and text, and the dialog that applies the marks for good
//! (`pdf_engine::redact`, docs/DECISIONS.md D-036).

use crate::dialogs::modal;
use crate::i18n::tr;
use crate::state::*;
use crate::tf;
use egui::RichText;
use pdf_engine::geom::Rect;
use pdf_engine::redact::{ImageMode, RedactOptions, RedactionReport};

/// The dialog that applies the marks.
pub struct RedactDialog {
    marks: usize,
    pages: usize,
    small: bool,
    jpeg: bool,
    remove_metadata: bool,
    remove_attachments: bool,
    error: Option<String>,
    /// What happened, once applied.
    done: Option<RedactionReport>,
}

impl App {
    /// Mark areas of a page for redaction (one undo step).
    pub fn mark_redaction(&mut self, page: pdf_engine::doc::PageId, rects: Vec<Rect>) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        match tab.session.mark_for_redaction(page, &rects) {
            Ok(()) => self.notify(tr(
                "Marked for redaction. Nothing is removed until you apply the redactions.",
            )),
            Err(e) => self.notify_error(tf!("Could not mark the area: {}", e)),
        }
    }

    /// Mark the selected text for redaction.
    pub fn redact_selection(&mut self) {
        let ti = self.active;
        let Some(sel) = self
            .tabs
            .get(ti)
            .and_then(|t| t.session.selection.text.clone())
        else {
            return;
        };
        let doc = self.tabs[ti].session.id;
        let Some((_, tp)) = self.text.get(&(doc, sel.page)).cloned() else {
            return;
        };
        let rects: Vec<Rect> = tp
            .selection_quads(sel.glyphs)
            .iter()
            .map(|q| q.bounds().abs().inflate(1.0, 1.0))
            .collect();
        if rects.is_empty() {
            return;
        }
        self.mark_redaction(sel.page, rects);
        if let Some(t) = self.tabs.get_mut(ti) {
            t.session.selection.text = None;
        }
    }

    /// Redact ▸ Apply Redactions…
    pub fn open_redact_dialog(&mut self) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let marks = tab.session.redaction_marks();
        if marks.is_empty() {
            self.notify(tr("Nothing is marked for redaction."));
            return;
        }
        let mut pages: Vec<_> = marks.iter().map(|m| m.page).collect();
        pages.sort();
        pages.dedup();
        self.dialog = Some(Dialog::Redact(Box::new(RedactDialog {
            marks: marks.len(),
            pages: pages.len(),
            small: false,
            jpeg: false,
            remove_metadata: false,
            remove_attachments: false,
            error: None,
            done: None,
        })));
    }

    /// The apply dialog. Returns whether it stays open.
    pub fn dialog_redact(&mut self, ctx: &egui::Context, st: &mut RedactDialog) -> bool {
        enum Act {
            Apply,
            SaveAs,
            Close,
        }
        let mut act: Option<Act> = None;
        let danger = self.pal.danger;
        let dim = self.pal.text_dim;
        modal(ctx, "redact_dialog", |ui| {
            ui.set_max_width(520.0);
            match &st.done {
                None => {
                    ui.heading(tr("Apply redactions"));
                    ui.label(tf!(
                        "{} area(s) on {} page(s) will be removed permanently.",
                        st.marks,
                        st.pages
                    ));
                    ui.add_space(4.0);
                    ui.label(tr("Each of those pages is replaced by a picture of itself with the marked areas painted black, so nothing under a mark stays in the file: no text, no picture, no hidden text. The other words on the page stay searchable. Comments and form fields under a mark are deleted."));
                    ui.add_space(4.0);
                    ui.colored_label(
                        danger,
                        tr("This cannot be undone. Use Save As to keep your original file."),
                    );
                    ui.add_space(6.0);
                    ui.checkbox(&mut st.small, tr("Smaller file (150 dpi instead of 300 dpi)"));
                    ui.checkbox(
                        &mut st.jpeg,
                        tr("Store the pictures as JPEG (smaller; for pages with photographs)"),
                    );
                    ui.checkbox(
                        &mut st.remove_metadata,
                        tr("Also remove the document properties (title, author, …)"),
                    );
                    ui.checkbox(
                        &mut st.remove_attachments,
                        tr("Also remove attached files"),
                    );
                    if let Some(e) = &st.error {
                        ui.colored_label(danger, e);
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button(tr("Apply redactions")).clicked() {
                            act = Some(Act::Apply);
                        }
                        if ui.button(tr("Cancel")).clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Escape))
                        {
                            act = Some(Act::Close);
                        }
                    });
                }
                Some(r) => {
                    ui.heading(tr("Redactions applied"));
                    ui.label(tf!(
                        "{} page(s) replaced, {} word(s) removed, {} comment(s) or field(s) deleted.",
                        r.pages,
                        r.words_removed,
                        r.annotations_removed
                    ));
                    if r.lowest_dpi < if st.small { 149.0 } else { 299.0 } {
                        ui.label(tf!(
                            "A very large page was drawn at {} dpi to keep the file workable.",
                            r.lowest_dpi.round() as i64
                        ));
                    }
                    ui.label(
                        RichText::new(tr("The document has not been saved yet. Use Save As to keep your original."))
                            .color(dim),
                    );
                    if !r.leaks.is_empty() {
                        ui.add_space(6.0);
                        ui.colored_label(
                            danger,
                            tr("These places still contain a word that was removed. Check them:"),
                        );
                        egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                            for l in r.leaks.iter().take(40) {
                                ui.label(format!("• {l}"));
                            }
                            if r.leaks.len() > 40 {
                                ui.label(tf!("… and {} more", r.leaks.len() - 40));
                            }
                        });
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button(tr("Save As…")).clicked() {
                            act = Some(Act::SaveAs);
                        }
                        if ui.button(tr("Close")).clicked() {
                            act = Some(Act::Close);
                        }
                    });
                }
            }
        });
        match act {
            None => true,
            Some(Act::Close) => false,
            Some(Act::SaveAs) => {
                self.save_active_as();
                false
            }
            Some(Act::Apply) => {
                let opts = RedactOptions {
                    dpi: if st.small { 150.0 } else { 300.0 },
                    image: if st.jpeg {
                        ImageMode::Jpeg(80)
                    } else {
                        ImageMode::Lossless
                    },
                    remove_metadata: st.remove_metadata,
                    remove_attachments: st.remove_attachments,
                };
                let Some(tab) = self.tabs.get_mut(self.active) else {
                    return false;
                };
                match tab.session.apply_redactions(&opts) {
                    Ok(report) => {
                        st.done = Some(report);
                        st.error = None;
                        // The pages look different now: drop what was cached for them.
                        self.text.clear();
                        true
                    }
                    Err(e) => {
                        st.error = Some(e.to_string());
                        true
                    }
                }
            }
        }
    }
}
