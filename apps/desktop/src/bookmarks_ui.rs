//! The Bookmarks panel: navigate, and (when the document can be edited) add, rename, delete, move,
//! indent and re-target bookmarks. Every change is one undoable step that rewrites the outline with
//! `pdf_engine::outline`.

use crate::i18n::tr;
use crate::state::*;
use crate::tf;
use egui::RichText;
use pdf_engine::outline::{self, OutlineItem};

/// A change asked for in the panel.
enum BmAction {
    Select(Vec<usize>, Option<usize>),
    StartRename(Vec<usize>),
    RenameDraft(Vec<usize>, String),
    Rename(Vec<usize>, String),
    CancelRename,
    AddAfter,
    AddChild,
    Delete,
    Move(i64),
    Indent,
    Outdent,
    Retarget,
}

impl App {
    /// Text for a new bookmark: the selected text, or "Page N".
    fn new_bookmark_title(&self, page: usize) -> String {
        if let Some(tab) = self.active_tab()
            && let Some(sel) = &tab.session.selection.text
            && let Some((_, tp)) = self.text.get(&(tab.session.id, sel.page))
        {
            let t: String = tp
                .text_of(sel.glyphs.clone())
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if !t.is_empty() {
                return t.chars().take(120).collect();
            }
        }
        tf!("Page {}", page + 1)
    }

    /// Read the outline, change it with `f`, and write it back as one step when `f` says it changed.
    fn edit_bookmarks(
        &mut self,
        label: &'static str,
        f: impl FnOnce(&mut Vec<OutlineItem>) -> Option<Vec<usize>>,
    ) {
        let ti = self.active;
        let mut items = outline::read_outline(self.tabs[ti].session.doc());
        let Some(sel) = f(&mut items) else { return };
        match self.tabs[ti]
            .session
            .execute(label, |tx| outline::write_outline(tx, &items))
        {
            Ok(()) => self.tabs[ti].ui.bookmark_sel = Some(sel),
            Err(e) => self.notify_error(e.to_string()),
        }
    }

    /// Navigate ▸ Add Bookmark: a bookmark to the current page, after the selected one.
    pub fn add_bookmark_here(&mut self) {
        let Some(tab) = self.active_tab() else { return };
        let page = tab.session.view.current_page;
        let after = tab.ui.bookmark_sel.clone();
        let title = self.new_bookmark_title(page);
        self.edit_bookmarks(tr("Add bookmark"), |items| {
            let item = OutlineItem::new(&title, page, None);
            let valid = after
                .as_deref()
                .filter(|p| outline::get(items, p).is_some());
            Some(outline::insert_after(items, valid, item))
        });
        let ti = self.active;
        let sel = self.tabs[ti].ui.bookmark_sel.clone();
        if let Some(p) = sel {
            self.tabs[ti].ui.bookmark_rename = Some((p, title));
        }
        self.left_tab = LeftTab::Bookmarks;
        self.prefs.show_left_sidebar = true;
    }

    pub(crate) fn bookmarks(&mut self, ui: &mut egui::Ui) {
        let ti = self.active;
        let rev = self.tabs[ti].session.revision();
        let can_edit = self.tabs[ti].session.doc().capabilities().can_edit;
        let need = !matches!(&self.tabs[ti].ui.outline, Some((r, _)) if *r == rev);
        if need {
            let b = std::sync::Arc::new(outline::read_outline(self.tabs[ti].session.doc()));
            self.tabs[ti].ui.outline = Some((rev, b));
        }
        let Some((_, items)) = self.tabs[ti].ui.outline.clone() else {
            return;
        };
        let sel = self.tabs[ti]
            .ui
            .bookmark_sel
            .clone()
            .filter(|p| outline::get(&items, p).is_some());
        let mut act: Option<BmAction> = None;
        if can_edit {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let has = sel.is_some();
                let b = |ui: &mut egui::Ui, on: bool, text: &str, tip: &str| {
                    ui.add_enabled(on, egui::Button::new(text).small())
                        .on_hover_text(tip)
                        .clicked()
                };
                if b(
                    ui,
                    true,
                    "+",
                    tr("Add a bookmark to the current page (Ctrl+B)"),
                ) {
                    act = Some(BmAction::AddAfter);
                }
                if b(ui, has, "+↳", tr("Add a sub-bookmark to the current page")) {
                    act = Some(BmAction::AddChild);
                }
                if b(ui, has, "✎", tr("Rename (or double-click the bookmark)")) {
                    act = sel.clone().map(BmAction::StartRename);
                }
                if b(ui, has, "↑", tr("Move up")) {
                    act = Some(BmAction::Move(-1));
                }
                if b(ui, has, "↓", tr("Move down")) {
                    act = Some(BmAction::Move(1));
                }
                if b(
                    ui,
                    has && sel.as_ref().is_some_and(|p| p.len() > 1),
                    "←",
                    tr("Move out one level"),
                ) {
                    act = Some(BmAction::Outdent);
                }
                if b(
                    ui,
                    has && sel.as_ref().is_some_and(|p| p.last() != Some(&0)),
                    "→",
                    tr("Make it a sub-bookmark of the one above"),
                ) {
                    act = Some(BmAction::Indent);
                }
                if b(ui, has, "⌖", tr("Point the bookmark at the current page")) {
                    act = Some(BmAction::Retarget);
                }
                if b(
                    ui,
                    has,
                    "✕",
                    tr("Delete the bookmark and its sub-bookmarks"),
                ) {
                    act = Some(BmAction::Delete);
                }
            });
            ui.add_space(2.0);
        }
        if items.is_empty() {
            ui.add_space(12.0);
            ui.label(RichText::new(tr("This document has no bookmarks.")).color(self.pal.text_dim));
            if can_edit {
                ui.label(
                    RichText::new(tr("Press + (or Ctrl+B) to add one for the current page."))
                        .size(12.0)
                        .color(self.pal.text_dim),
                );
            }
        }
        let rename = self.tabs[ti].ui.bookmark_rename.clone();
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                struct Ctx<'a> {
                    sel: Option<&'a [usize]>,
                    rename: Option<&'a (Vec<usize>, String)>,
                    act: &'a mut Option<BmAction>,
                    accent: egui::Color32,
                    can_edit: bool,
                }
                fn level(
                    ui: &mut egui::Ui,
                    items: &[OutlineItem],
                    path: &mut Vec<usize>,
                    c: &mut Ctx<'_>,
                ) {
                    for (n, it) in items.iter().enumerate() {
                        path.push(n);
                        let id = ui.make_persistent_id(("bm", path.clone()));
                        let header = |ui: &mut egui::Ui, c: &mut Ctx<'_>, path: &[usize]| {
                            if let Some((rp, text)) = c.rename
                                && rp.as_slice() == path
                            {
                                let mut t = text.clone();
                                let r =
                                    ui.add(crate::ui_kit::singleline(&mut t).desired_width(160.0));
                                if !r.has_focus() && !r.lost_focus() {
                                    r.request_focus();
                                }
                                // Enter or a click elsewhere writes the title; Esc keeps the old one.
                                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                    *c.act = Some(BmAction::CancelRename);
                                } else if r.lost_focus() {
                                    *c.act = Some(BmAction::Rename(path.to_vec(), t));
                                } else if r.changed() {
                                    *c.act = Some(BmAction::RenameDraft(path.to_vec(), t));
                                }
                                return;
                            }
                            let title = if it.title.is_empty() {
                                tr("(untitled)")
                            } else {
                                it.title.as_str()
                            };
                            let mut rt = RichText::new(title);
                            if it.flags & 2 != 0 {
                                rt = rt.strong();
                            }
                            if it.flags & 1 != 0 {
                                rt = rt.italics();
                            }
                            if let Some(col) = it.color
                                && col != [0.0, 0.0, 0.0]
                            {
                                rt = rt.color(egui::Color32::from_rgb(
                                    (col[0] * 255.0) as u8,
                                    (col[1] * 255.0) as u8,
                                    (col[2] * 255.0) as u8,
                                ));
                            }
                            let selected = c.sel == Some(path);
                            let r = ui.selectable_label(selected, rt);
                            if selected {
                                ui.painter().rect_stroke(
                                    r.rect,
                                    2.0,
                                    egui::Stroke::new(1.0, c.accent),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            if r.double_clicked() && c.can_edit {
                                *c.act = Some(BmAction::StartRename(path.to_vec()));
                            } else if r.clicked() {
                                *c.act = Some(BmAction::Select(path.to_vec(), it.page));
                            }
                        };
                        if it.children.is_empty() {
                            ui.horizontal(|ui| {
                                ui.add_space(18.0);
                                header(ui, c, path);
                            });
                        } else {
                            egui::collapsing_header::CollapsingState::load_with_default_open(
                                ui.ctx(),
                                id,
                                it.open,
                            )
                            .show_header(ui, |ui| header(ui, c, path))
                            .body(|ui| level(ui, &it.children, path, c));
                        }
                        path.pop();
                    }
                }
                let mut c = Ctx {
                    sel: sel.as_deref(),
                    rename: rename.as_ref(),
                    act: &mut act,
                    accent: self.pal.accent,
                    can_edit,
                };
                level(ui, &items, &mut Vec::new(), &mut c);
            });
        if can_edit
            && act.is_none()
            && rename.is_none()
            && sel.is_some()
            && ui.ui_contains_pointer()
            && ui.input(|i| i.key_pressed(egui::Key::Delete))
        {
            act = Some(BmAction::Delete);
        }
        self.bookmark_action(act, sel);
    }

    fn bookmark_action(&mut self, act: Option<BmAction>, sel: Option<Vec<usize>>) {
        let ti = self.active;
        let Some(act) = act else { return };
        let page = self.tabs[ti].session.view.current_page;
        match act {
            BmAction::Select(p, target) => {
                self.tabs[ti].ui.bookmark_sel = Some(p);
                if let Some(t) = target {
                    self.go_to(t);
                }
            }
            BmAction::StartRename(p) => {
                let title = self.tabs[ti]
                    .ui
                    .outline
                    .as_ref()
                    .and_then(|(_, items)| outline::get(items, &p).map(|i| i.title.clone()))
                    .unwrap_or_default();
                self.tabs[ti].ui.bookmark_sel = Some(p.clone());
                self.tabs[ti].ui.bookmark_rename = Some((p, title));
            }
            BmAction::RenameDraft(p, t) => {
                self.tabs[ti].ui.bookmark_rename = Some((p, t));
            }
            BmAction::CancelRename => self.tabs[ti].ui.bookmark_rename = None,
            BmAction::Rename(p, t) => {
                self.tabs[ti].ui.bookmark_rename = None;
                let t = t.trim().to_string();
                self.edit_bookmarks(tr("Rename bookmark"), |items| {
                    let it = outline::get_mut(items, &p)?;
                    if it.title == t || t.is_empty() {
                        return None;
                    }
                    it.title = t;
                    Some(p)
                });
            }
            BmAction::AddAfter => self.add_bookmark_here(),
            BmAction::AddChild => {
                let Some(parent) = sel else { return };
                let title = self.new_bookmark_title(page);
                self.edit_bookmarks(tr("Add bookmark"), |items| {
                    outline::insert_child(items, &parent, OutlineItem::new(&title, page, None))
                });
                if let Some(p) = self.tabs[ti].ui.bookmark_sel.clone() {
                    self.tabs[ti].ui.bookmark_rename = Some((p, title));
                }
            }
            BmAction::Delete => {
                let Some(p) = sel else { return };
                self.edit_bookmarks(tr("Delete bookmark"), |items| {
                    outline::remove(items, &p)?;
                    // Select the neighbour that took its place, or the parent.
                    let mut next = p.clone();
                    if outline::get(items, &next).is_none() {
                        if let Some(l) = next.last_mut()
                            && *l > 0
                        {
                            *l -= 1;
                        } else {
                            next.pop();
                        }
                    }
                    Some(next)
                });
            }
            BmAction::Move(d) => {
                let Some(p) = sel else { return };
                self.edit_bookmarks(tr("Move bookmark"), |items| outline::move_by(items, &p, d));
            }
            BmAction::Indent => {
                let Some(p) = sel else { return };
                self.edit_bookmarks(tr("Move bookmark"), |items| outline::indent(items, &p));
            }
            BmAction::Outdent => {
                let Some(p) = sel else { return };
                self.edit_bookmarks(tr("Move bookmark"), |items| outline::outdent(items, &p));
            }
            BmAction::Retarget => {
                let Some(p) = sel else { return };
                self.edit_bookmarks(tr("Change bookmark destination"), |items| {
                    outline::get_mut(items, &p)?.retarget(page, None);
                    Some(p)
                });
                self.notify(tf!("The bookmark now goes to page {}.", page + 1));
            }
        }
    }
}
