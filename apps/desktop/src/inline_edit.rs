//! Editing the text of a text box, note or stamp right on the page: double-click it with the Select tool.
//!
//! The field sits exactly over the annotation. Ctrl+Enter or a click anywhere else applies the text as
//! one undoable step; Esc throws the change away.

use crate::canvas::ViewCtx;
use crate::i18n::tr;
use crate::state::*;
use egui::{Color32, FontId, Stroke};
use pdf_engine::annot::{self, AnnotId, AnnotationInfo, AnnotationKind};
use pdf_engine::doc::PageId;

/// A text being edited in place.
pub struct InlineEdit {
    page: PageId,
    id: AnnotId,
    text: String,
    original: String,
    /// The field has had the keyboard focus; losing it after that ends the edit.
    focused: bool,
}

impl InlineEdit {
    /// Begin editing `a`, if its kind has text of its own.
    pub fn start(page: PageId, a: &AnnotationInfo) -> Option<Self> {
        let spec = a.spec.as_ref()?;
        let text = match &spec.kind {
            AnnotationKind::StampText { label, .. } => label.clone(),
            AnnotationKind::FreeText { .. } | AnnotationKind::Note { .. } => spec.contents.clone(),
            _ => return None,
        };
        Some(Self {
            page,
            id: a.id,
            original: text.clone(),
            text,
            focused: false,
        })
    }
}

fn color(c: annot::Rgb) -> Color32 {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(b(c.0), b(c.1), b(c.2))
}

impl App {
    /// The editing field, once per frame while an inline edit is open.
    pub fn inline_edit_ui(&mut self, ctx: &egui::Context, vc: &ViewCtx) {
        let ti = self.active;
        let Some(mut ed) = self.tabs[ti].ui.inline_edit.take() else {
            return;
        };
        let Some(i) = vc.pages.iter().position(|p| p.id == ed.page) else {
            return;
        };
        let annots = self.annots_for(ed.page);
        // The annotation went away (undo, delete): nothing to edit any more.
        let Some(info) = annots.iter().find(|a| a.id == ed.id) else {
            return;
        };
        let Some(spec) = info.spec.as_ref() else {
            return;
        };
        let rect = Self::rect_screen(vc, i, info.rect);
        let (font_px, fill, ink, rows) = match &spec.kind {
            AnnotationKind::FreeText {
                font_size,
                text_color,
                ..
            } => {
                let px = (*font_size as f32 * vc.px_per_pt as f32).clamp(10.0, 60.0);
                let rows = (rect.height() / (px * 1.3)).floor().clamp(2.0, 14.0) as usize;
                (
                    px,
                    spec.fill.map_or(Color32::WHITE, color),
                    color(*text_color),
                    rows,
                )
            }
            _ => (14.0, Color32::WHITE, Color32::BLACK, 3),
        };
        let width = rect.width().max(200.0);
        let accent = self.pal.accent;
        let mut apply = false;
        let cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)) {
            apply = true;
        }
        egui::Area::new(egui::Id::new("inline_text_edit"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(fill)
                    .stroke(Stroke::new(2.0, accent))
                    .corner_radius(egui::CornerRadius::same(4))
                    .shadow(egui::Shadow {
                        offset: [0, 3],
                        blur: 12,
                        spread: 0,
                        color: Color32::from_black_alpha(60),
                    })
                    .show(ui, |ui| {
                        ui.set_width(width);
                        let r = ui.add(
                            crate::ui_kit::multiline(&mut ed.text)
                                .frame(egui::Frame::NONE)
                                .font(FontId::proportional(font_px))
                                .text_color(ink)
                                .desired_width(width)
                                .desired_rows(rows),
                        );
                        if !ed.focused {
                            r.request_focus();
                            if r.has_focus() {
                                ed.focused = true;
                            }
                        } else if !r.has_focus() {
                            apply = true;
                        }
                    });
                ui.add_space(4.0);
                egui::Frame::new()
                    .fill(Color32::from_black_alpha(200))
                    .corner_radius(egui::CornerRadius::same(4))
                    .inner_margin(egui::Margin::symmetric(8, 3))
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(tr(
                                    "Ctrl+Enter or click outside to apply, Esc to cancel",
                                ))
                                .size(11.0)
                                .color(Color32::WHITE),
                            )
                            .extend(),
                        );
                    });
            });
        if cancel {
            return;
        }
        if apply {
            self.apply_inline_edit(&ed);
            return;
        }
        self.tabs[ti].ui.inline_edit = Some(ed);
        ctx.request_repaint();
    }

    fn apply_inline_edit(&mut self, ed: &InlineEdit) {
        let ti = self.active;
        if ed.text == ed.original {
            return;
        }
        let Some(info) = annot::read_annotation(self.tabs[ti].session.doc().lopdf(), ed.id) else {
            return;
        };
        let Some(mut spec) = info.spec else { return };
        spec.contents = ed.text.clone();
        match &mut spec.kind {
            AnnotationKind::StampText { label, .. } => {
                if ed.text.trim().is_empty() {
                    return;
                }
                label.clone_from(&ed.text);
            }
            AnnotationKind::FreeText {
                rect,
                font_size,
                font,
                ..
            } => {
                let (local_w, _) = annot::local_size(*rect, spec.rotation);
                if let Ok(h) = annot::freetext_required_height(&ed.text, *font_size, local_w, *font)
                {
                    // Grow the box so the text is not clipped.
                    *rect = annot::grow_box(*rect, spec.rotation, h);
                }
            }
            _ => {}
        }
        let id = ed.id;
        if let Err(e) = self.tabs[ti].session.execute(tr("Edit text"), |tx| {
            annot::update_annotation(tx, id, &spec)
        }) {
            self.notify_error(e.to_string());
        }
    }
}
