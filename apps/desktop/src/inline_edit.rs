//! Editing text right on the page: double-click a text box, note or stamp (Select tool) or a line of the
//! page's own text (Edit Text tool).
//!
//! The field sits exactly over the text. A click anywhere else (or Ctrl+Enter, or Enter for a line of
//! page text) applies it as one undoable step; Esc throws the change away.

use crate::canvas::ViewCtx;
use crate::i18n::tr;
use crate::state::*;
use egui::{Color32, FontId, Stroke};
use pdf_engine::annot::{self, AnnotId, AnnotationInfo, AnnotationKind};
use pdf_engine::doc::PageId;
use pdf_engine::pagecontent::{ObjRef, TextRunInfo};

/// What is being edited.
#[derive(Clone, Copy)]
enum Target {
    Annotation(AnnotId),
    /// A text run of the page content.
    Run(ObjRef),
}

/// A text being edited in place.
pub struct InlineEdit {
    page: PageId,
    target: Target,
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
            target: Target::Annotation(a.id),
            original: text.clone(),
            text,
            focused: false,
        })
    }

    /// Begin editing a line of the page's own text.
    pub fn start_run(page: PageId, run: &TextRunInfo) -> Self {
        Self {
            page,
            target: Target::Run(run.id),
            original: run.text.clone(),
            text: run.text.clone(),
            focused: false,
        }
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
        // Where the text is and how it looks. Gone (undo, delete, a changed page): nothing to edit any more.
        let (rect, font_px, fill, ink, rows, single) = match ed.target {
            Target::Annotation(id) => {
                let annots = self.annots_for(ed.page);
                let Some(info) = annots.iter().find(|a| a.id == id) else {
                    return;
                };
                let Some(spec) = info.spec.as_ref() else {
                    return;
                };
                let rect = Self::rect_screen(vc, i, info.rect);
                match &spec.kind {
                    AnnotationKind::FreeText {
                        font_size,
                        text_color,
                        ..
                    } => {
                        let px = (*font_size as f32 * vc.px_per_pt as f32).clamp(10.0, 60.0);
                        let rows = (rect.height() / (px * 1.3)).floor().clamp(2.0, 14.0) as usize;
                        (
                            rect,
                            px,
                            spec.fill.map_or(Color32::WHITE, color),
                            color(*text_color),
                            rows,
                            false,
                        )
                    }
                    _ => (rect, 14.0, Color32::WHITE, Color32::BLACK, 3, false),
                }
            }
            Target::Run(rid) => {
                let objs = self.objects_for(ed.page);
                let Some(run) = objs.runs.iter().find(|r| r.id == rid) else {
                    return;
                };
                let rect = Self::rect_screen(vc, i, run.quad.bounds());
                let px = (run.size_pt as f32 * vc.px_per_pt as f32).clamp(9.0, 80.0);
                (rect, px, Color32::WHITE, Color32::BLACK, 1, true)
            }
        };
        let width = if single {
            rect.width().max(120.0)
        } else {
            rect.width().max(200.0)
        };
        let accent = self.pal.accent;
        let mut apply = false;
        let cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)) {
            apply = true;
        }
        egui::Area::new(egui::Id::new("inline_text_edit"))
            .order(egui::Order::Foreground)
            .fixed_pos(if single {
                // The text of the field starts where the page's text starts.
                rect.min + egui::vec2(-4.0, -7.0)
            } else {
                rect.min
            })
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
                        let field = if single {
                            crate::ui_kit::singleline(&mut ed.text)
                        } else {
                            crate::ui_kit::multiline(&mut ed.text).desired_rows(rows)
                        };
                        let r = ui.add(
                            field
                                .frame(egui::Frame::NONE)
                                .font(FontId::proportional(font_px))
                                .text_color(ink)
                                .desired_width(width),
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
                                egui::RichText::new(tr(if single {
                                    "Enter or click outside to apply, Esc to cancel"
                                } else {
                                    "Ctrl+Enter or click outside to apply, Esc to cancel"
                                }))
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
            match ed.target {
                Target::Annotation(_) => self.apply_inline_edit(&ed),
                Target::Run(run) => self.apply_inline_run(ed.page, run, &ed.text, &ed.original),
            }
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
        let Target::Annotation(id) = ed.target else {
            return;
        };
        let Some(info) = annot::read_annotation(self.tabs[ti].session.doc().lopdf(), id) else {
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
        if let Err(e) = self.tabs[ti].session.execute(tr("Edit text"), |tx| {
            annot::update_annotation(tx, id, &spec)
        }) {
            self.notify_error(e.to_string());
        }
    }
}
