//! Right-click menu on the page canvas.
//!
//! What it offers depends on what is under the pointer: selected text (copy, markup, search),
//! an annotation (delete, duplicate, properties) or empty page area (note, zoom, tools).

use crate::canvas::ViewCtx;
use crate::i18n::tr;
use crate::state::*;
use crate::tf;
use editor_core::command::CommandId as C;
use editor_core::selection::TextSelection;
use editor_core::tools::Tool;
use pdf_engine::annot::{AnnotationKind, Rgb};
use pdf_engine::doc::PageId;
use pdf_engine::geom::{Point, Rect as PRect};

/// Where the menu was opened, captured when the secondary button went down.
#[derive(Clone, Copy, Debug)]
pub struct CtxTarget {
    pub page: PageId,
    pub pt: Point,
}

/// Text markup flavours the menu can apply to the selection.
#[derive(Clone, Copy)]
pub enum Markup {
    Highlight,
    Underline,
    StrikeOut,
    Squiggly,
}

impl App {
    pub fn canvas_context_menu(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        vc: &ViewCtx,
    ) {
        if self.dialog.is_some() || self.palette_open {
            return;
        }
        if response.secondary_clicked() {
            self.prepare_ctx_target(ctx, response, vc);
        }
        response.context_menu(|ui| {
            self.context_menu_items(ui, ctx);
        });
    }

    /// Decide what the right-click refers to and select it, like other editors do.
    fn prepare_ctx_target(&mut self, ctx: &egui::Context, response: &egui::Response, vc: &ViewCtx) {
        let ti = self.active;
        self.ctx_target = None;
        let Some(pos) = response
            .interact_pointer_pos()
            .or_else(|| ctx.input(|i| i.pointer.hover_pos()))
        else {
            return;
        };
        let Some(i) = vc.page_at(pos) else { return };
        let page = vc.pages[i].id;
        let pt = vc.screen_to_pdf(i, pos);
        self.ctx_target = Some(CtxTarget { page, pt });
        let tp = self.text_for(vc, i);
        // Keep an existing text selection when the click lands inside it.
        if let (Some(sel), Some(tp)) = (self.tabs[ti].session.selection.text.clone(), &tp)
            && sel.page == page
            && tp
                .selection_quads(sel.glyphs.clone())
                .iter()
                .any(|q| q.bounds().inflate(2.0, 2.0).contains(pt))
        {
            return;
        }
        // An annotation under the pointer becomes the selection.
        let tol = 4.0 / vc.px_per_pt;
        let annots = self.annots_for(page);
        let hit = annots
            .iter()
            .rev()
            .filter(|a| !matches!(a.subtype.as_str(), "Link" | "Widget" | "Popup"))
            .find(|a| a.rect.inflate(tol, tol).contains(pt));
        if let Some(a) = hit {
            let sel = &mut self.tabs[ti].session.selection;
            if !sel.has_annotation(a.id) {
                sel.select_annotation(page, a.id, false);
            }
            return;
        }
        // Otherwise the word under the pointer.
        if let Some(tp) = tp
            && let Some(g) = tp.hit_test(pt, 6.0)
        {
            let w = tp.word_range(g);
            if !w.is_empty() {
                let sel = &mut self.tabs[ti].session.selection;
                sel.annotations.clear();
                sel.text = Some(TextSelection { page, glyphs: w });
                return;
            }
        }
        self.tabs[ti].session.selection.clear_content();
    }

    fn menu_item(ui: &mut egui::Ui, label: &str, hint: &str) -> bool {
        let mut b = egui::Button::new(label);
        if !hint.is_empty() {
            b = b.shortcut_text(hint);
        }
        let clicked = ui.add(b).clicked();
        if clicked {
            ui.close();
        }
        clicked
    }

    fn context_menu_items(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.set_min_width(200.0);
        let ti = self.active;
        let Some(tab) = self.tabs.get(ti) else { return };
        let text_sel = tab.session.selection.text.clone();
        let annot_sel = !tab.session.selection.annotations.is_empty();
        let can_edit = tab.session.doc().capabilities().can_edit;
        let target = self.ctx_target;
        if let Some(sel) = text_sel {
            let doc = self.tabs[ti].session.id;
            let snippet = self
                .text
                .get(&(doc, sel.page))
                .map(|(_, tp)| tp.text_of(sel.glyphs.clone()))
                .unwrap_or_default();
            if self.text_use_allowed() && Self::menu_item(ui, tr("Copy"), "Ctrl+C") {
                self.run_command(ctx, C::EditCopy);
            }
            ui.separator();
            ui.add_enabled_ui(can_edit, |ui| {
                for (label, m) in [
                    (tr("Highlight"), Markup::Highlight),
                    (tr("Underline"), Markup::Underline),
                    (tr("Strikeout"), Markup::StrikeOut),
                    (tr("Squiggly"), Markup::Squiggly),
                ] {
                    if Self::menu_item(ui, label, "") {
                        self.markup_selection(m);
                    }
                }
            });
            ui.separator();
            let short: String = snippet.chars().take(24).collect();
            let label = tf!(
                "Find “{}{}” in document",
                short.trim(),
                if snippet.chars().count() > 24 {
                    "…"
                } else {
                    ""
                }
            );
            if Self::menu_item(ui, &label, "") {
                self.search_for(&snippet);
            }
            self.context_menu_ai_items(ui, ctx, &snippet);
        } else if annot_sel {
            ui.add_enabled_ui(can_edit, |ui| {
                if Self::menu_item(ui, tr("Delete"), "Del") {
                    self.run_command(ctx, C::EditDelete);
                }
                if Self::menu_item(ui, tr("Duplicate"), "") {
                    self.run_command(ctx, C::EditDuplicate);
                }
            });
            ui.separator();
            if Self::menu_item(ui, tr("Properties"), "") {
                self.prefs.show_right_sidebar = true;
                self.right_tab = RightTab::Properties;
            }
        } else {
            if let Some(t) = target {
                ui.add_enabled_ui(can_edit, |ui| {
                    if Self::menu_item(ui, tr("Add note here…"), "") {
                        self.dialog = Some(Dialog::TextEntry {
                            page: t.page,
                            tool: Tool::Note,
                            rect: PRect::new(t.pt.x, t.pt.y, t.pt.x, t.pt.y),
                            text: String::new(),
                            callout: None,
                        });
                    }
                });
                if Self::menu_item(ui, tr("Select all text on this page"), "Ctrl+A") {
                    self.run_command(ctx, C::EditSelectAll);
                }
                ui.separator();
            }
            ui.menu_button(tr("Tool"), |ui| {
                for (label, id) in [
                    (tr("Select"), C::ToolSelect),
                    (tr("Hand"), C::ToolHand),
                    (tr("Select text"), C::ToolTextSelect),
                    (tr("Highlight"), C::ToolHighlight),
                ] {
                    if Self::menu_item(ui, label, "") {
                        self.run_command(ctx, id);
                    }
                }
            });
            ui.menu_button(tr("View"), |ui| {
                for (label, id) in [
                    (tr("Zoom in"), C::ViewZoomIn),
                    (tr("Zoom out"), C::ViewZoomOut),
                    (tr("Fit page"), C::ViewFitPage),
                    (tr("Fit width"), C::ViewFitWidth),
                    (tr("Rotate view clockwise"), C::ViewRotateClockwise),
                ] {
                    if Self::menu_item(ui, label, "") {
                        self.run_command(ctx, id);
                    }
                }
            });
        }
    }

    /// Apply a markup annotation to the current text selection.
    pub fn markup_selection(&mut self, m: Markup) {
        let ti = self.active;
        let Some(sel) = self.tabs[ti].session.selection.text.clone() else {
            return;
        };
        let doc = self.tabs[ti].session.id;
        let Some((_, tp)) = self.text.get(&(doc, sel.page)).cloned() else {
            return;
        };
        let quads = tp.selection_quads(sel.glyphs);
        if quads.is_empty() {
            return;
        }
        let (kind, label) = match m {
            Markup::Highlight => (AnnotationKind::Highlight { quads }, tr("Highlight text")),
            Markup::Underline => (AnnotationKind::Underline { quads }, tr("Underline text")),
            Markup::StrikeOut => (AnnotationKind::StrikeOut { quads }, tr("Strike out text")),
            Markup::Squiggly => (AnnotationKind::Squiggly { quads }, tr("Squiggly underline")),
        };
        let mut spec = self.new_spec(kind);
        if matches!(m, Markup::Squiggly) {
            spec.color = Rgb(0.85, 0.1, 0.1);
        }
        self.add_annotation(sel.page, spec, label);
        self.tabs[ti].session.selection.text = None;
    }

    /// Put `text` in the search box and run the search.
    pub fn search_for(&mut self, text: &str) {
        let q: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if q.is_empty() {
            return;
        }
        self.prefs.show_left_sidebar = true;
        self.left_tab = LeftTab::Search;
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.session.search.query = q;
        }
        self.start_search();
    }
}

impl App {
    /// Copilot entries for selected text.
    fn context_menu_ai_items(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, text: &str) {
        let ti = self.active;
        let page = self.tabs[ti]
            .session
            .selection
            .text
            .as_ref()
            .and_then(|t| self.tabs[ti].session.doc().page_index(t.page))
            .unwrap_or(self.tabs[ti].session.view.current_page);
        ui.separator();
        if !self.text_use_allowed() {
            return;
        }
        let busy = self.ai_busy();
        ui.add_enabled_ui(!busy, |ui| {
            if Self::menu_item(ui, tr("Explain"), "") {
                self.copilot_explain(ctx, text, page);
            }
            if Self::menu_item(ui, tr("Translate"), "") {
                self.copilot_translate_selection(ctx, text);
            }
        });
    }
}
