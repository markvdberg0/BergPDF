//! Left (thumbnails / bookmarks / search) and right (properties / comments) sidebars.

use crate::chrome::command_icon;
use crate::i18n::tr;
use crate::icons::Icon;
use crate::state::*;
use crate::tf;
use editor_core::command::CommandId as C;
use editor_core::tiles::{TileKey, TilePlan, quantize_scale};
use editor_core::tools::Tool;
use egui::{Color32, Pos2, Rect, RichText, Sense, Vec2};
use pdf_engine::annot::{self, AnnotationKind, AnnotationSpec, BorderStyle, Rgb};
use pdf_engine::geom::Rotation;
use std::sync::Arc;

const THUMB_BUCKETS: [f32; 6] = [96.0, 128.0, 160.0, 200.0, 260.0, 340.0];

fn thumb_bucket(w: f32) -> f32 {
    THUMB_BUCKETS
        .iter()
        .copied()
        .find(|b| *b >= w)
        .unwrap_or(340.0)
}

impl App {
    /// Frame shared by both side panels: fill plus breathing room so nothing touches the edges.
    pub fn side_frame(&self) -> egui::Frame {
        egui::Frame::new()
            .fill(self.pal.panel)
            .inner_margin(egui::Margin::symmetric(12, 8))
    }

    /// The button that hides a side panel: a clearly visible square with a chevron.
    fn collapse_button(ui: &mut egui::Ui, pointing_left: bool, tip: &str) -> bool {
        crate::ui_kit::chevron_button(ui, pointing_left, tip).clicked()
    }

    /// The row of tabs at the top of a side panel. It scrolls sideways when the panel is narrow, so the
    /// panel never grows to fit its tabs.
    fn tab_strip<T: Copy + PartialEq>(
        ui: &mut egui::Ui,
        id: &str,
        current: &mut T,
        tabs: &[(T, crate::icons::Icon, &str)],
    ) {
        // The name of the selected tab is shown only when it fits next to the other icons.
        let font = egui::FontId::proportional(13.5);
        let label_w = tabs
            .iter()
            .find(|(t, _, _)| *current == *t)
            .map_or(0.0, |(_, _, l)| {
                ui.painter()
                    .layout_no_wrap((*l).to_string(), font.clone(), egui::Color32::WHITE)
                    .size()
                    .x
                    + 7.0
            });
        let icons_w = tabs.len() as f32 * (18.0 + 24.0 + 2.0);
        let show_label = icons_w + label_w <= ui.available_width() - 4.0;
        egui::ScrollArea::horizontal()
            .id_salt(id)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (t, icon, label) in tabs {
                        if crate::ui_kit::icon_tab(ui, *icon, label, *current == *t, show_label)
                            .clicked()
                        {
                            *current = *t;
                        }
                    }
                });
            });
    }

    /// Thin strip shown instead of a hidden side panel; one click brings the panel back.
    pub fn collapsed_rail(&mut self, ui: &mut egui::Ui, left: bool) {
        let panel = if left {
            egui::Panel::left("left_rail")
        } else {
            egui::Panel::right("right_rail")
        };
        panel
            .resizable(false)
            .exact_size(44.0)
            .frame(
                egui::Frame::new()
                    .fill(self.pal.panel)
                    .inner_margin(egui::Margin::symmetric(8, 12)),
            )
            .show(ui, |ui| {
                let tip = if left {
                    tr("Show the page panel")
                } else {
                    tr("Show the properties panel")
                };
                // Pointing into the window: the panel opens that way.
                if crate::ui_kit::chevron_button(ui, !left, tip).clicked() {
                    if left {
                        self.prefs.show_left_sidebar = true;
                    } else {
                        self.prefs.show_right_sidebar = true;
                    }
                    self.prefs_dirty = true;
                }
            });
    }

    pub fn left_sidebar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut hide = false;
        egui::Sides::new().shrink_left().show(
            ui,
            |ui| {
                let mut tab = self.left_tab;
                Self::tab_strip(
                    ui,
                    "left_tabs",
                    &mut tab,
                    &[
                        (LeftTab::Thumbnails, Icon::Page, tr("Pages")),
                        (LeftTab::Bookmarks, Icon::Bookmark, tr("Bookmarks")),
                        (LeftTab::Search, Icon::Find, tr("Search")),
                    ],
                );
                self.left_tab = tab;
            },
            |ui| {
                hide =
                    Self::collapse_button(ui, true, tr("Hide the page panel (View ▸ Left panel)"));
            },
        );
        if hide {
            self.prefs.show_left_sidebar = false;
            self.prefs_dirty = true;
        }
        ui.separator();
        match self.left_tab {
            LeftTab::Thumbnails => self.thumbnails(ui, ctx),
            LeftTab::Bookmarks => self.bookmarks(ui),
            LeftTab::Search => self.search_panel(ui, ctx),
        }
    }

    fn thumbnails(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let ti = self.active;
        let ppp = ctx.pixels_per_point();
        let Ok(pages) = self.tabs[ti].session.pages() else {
            return;
        };
        let (doc, revision) = (self.tabs[ti].session.id, self.tabs[ti].session.revision());
        let avail_w = ui.available_width();
        let thumb_w = (avail_w - 28.0).max(60.0);
        let label_h = 18.0;
        let heights: Vec<f32> = pages
            .iter()
            .map(|p| {
                let s = p.geometry.view_size(Rotation::R0);
                (thumb_w * (s.height / s.width) as f32).min(thumb_w * 2.0) + label_h + 10.0
            })
            .collect();
        let mut offsets = Vec::with_capacity(heights.len());
        let mut acc = 0.0;
        for h in &heights {
            offsets.push(acc);
            acc += h;
        }
        let total = acc;
        let current = self.tabs[ti].session.view.current_page;
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt("thumbs")
            .auto_shrink([false, false]);
        if std::mem::take(&mut self.tabs[ti].ui.thumb_scroll_to_current)
            && let Some(o) = offsets.get(current)
        {
            scroll = scroll.vertical_scroll_offset((*o - 20.0).max(0.0));
        }
        let selection = self.tabs[ti].session.selection.pages.clone();
        let mut clicked: Option<(usize, egui::Modifiers)> = None;
        let mut drop_target: Option<usize> = None;
        let drag_active = self.tabs[ti].ui.thumb_drag.is_some();
        let mut drag_started: Option<usize> = None;
        let mut drag_stopped = false;
        scroll.show_viewport(ui, |ui, viewport| {
            let (rect, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), total), Sense::hover());
            let painter = ui.painter_at(rect);
            let ptr = ctx.input(|i| i.pointer.hover_pos());
            for (i, page) in pages.iter().enumerate() {
                let top = offsets[i];
                if top + heights[i] < viewport.min.y - 200.0 || top > viewport.max.y + 200.0 {
                    continue;
                }
                let row = Rect::from_min_size(
                    rect.min + Vec2::new(0.0, top),
                    Vec2::new(rect.width(), heights[i]),
                );
                let size = page.geometry.view_size(Rotation::R0);
                let th = (thumb_w * (size.height / size.width) as f32).min(thumb_w * 2.0);
                let tw = th * (size.width / size.height) as f32;
                let img = Rect::from_min_size(
                    Pos2::new(row.center().x - tw / 2.0, row.min.y + 5.0),
                    Vec2::new(tw, th),
                );
                let resp = ui.interact(row, ui.id().with(("thumb", i)), Sense::click_and_drag());
                let is_sel = selection.contains(&page.id);
                if is_sel {
                    painter.rect_filled(
                        row.shrink2(Vec2::new(2.0, 1.0)),
                        5.0,
                        self.pal.accent_soft,
                    );
                } else if resp.hovered() {
                    painter.rect_filled(row.shrink2(Vec2::new(2.0, 1.0)), 5.0, self.pal.hover);
                }
                painter.rect_filled(
                    img.translate(Vec2::new(1.0, 2.0)),
                    1.0,
                    self.pal.page_shadow,
                );
                painter.rect_filled(img, 0.0, Color32::WHITE);
                // Thumbnail bitmap (whole-page tile at a bucketed scale).
                let bw = thumb_bucket(tw);
                let scale = f64::from(bw * ppp) / size.width;
                let key = TileKey {
                    doc,
                    revision,
                    page: page.id,
                    rotation: Rotation::R0,
                    scale_milli: quantize_scale(scale),
                    tx: 0,
                    ty: 0,
                    whole: true,
                };
                if let Some(tex) = self.tiles.get(&key) {
                    painter.image(
                        tex.id(),
                        img,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                } else {
                    // Any cached whole-page render of this page is a fine stand-in.
                    let any = self
                        .tiles
                        .keys()
                        .find(|k| {
                            k.doc == doc
                                && k.page == page.id
                                && k.whole
                                && k.rotation == Rotation::R0
                        })
                        .copied();
                    if let Some(k) = any
                        && let Some(tex) = self.tiles.get(&k)
                    {
                        painter.image(
                            tex.id(),
                            img,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    let (w_px, h_px) = (
                        (size.width * scale).ceil() as u32,
                        (size.height * scale).ceil() as u32,
                    );
                    let plan = TilePlan {
                        tx: 0,
                        ty: 0,
                        whole: true,
                        x: 0,
                        y: 0,
                        w: w_px.max(1),
                        h: h_px.max(1),
                    };
                    self.request_tile(key, plan, i, page, 10, doc, revision);
                }
                if i == current {
                    painter.rect_stroke(
                        img.expand(2.0),
                        1.0,
                        egui::Stroke::new(2.0, self.pal.accent),
                        egui::StrokeKind::Middle,
                    );
                }
                painter.text(
                    Pos2::new(row.center().x, row.max.y - label_h / 2.0 - 2.0),
                    egui::Align2::CENTER_CENTER,
                    format!("{}", i + 1),
                    egui::FontId::proportional(12.0),
                    self.pal.text,
                );
                resp.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tf!("Page {}", i + 1))
                });
                if resp.clicked() {
                    clicked = Some((i, ctx.input(|inp| inp.modifiers)));
                }
                if resp.drag_started() {
                    drag_started = Some(i);
                }
                if drag_active
                    && let Some(p) = ptr
                    && p.y >= row.min.y
                    && p.y < row.max.y
                {
                    let before = p.y < row.center().y;
                    drop_target = Some(if before { i } else { i + 1 });
                    let y = if before { row.min.y } else { row.max.y };
                    painter.line_segment(
                        [Pos2::new(row.min.x + 6.0, y), Pos2::new(row.max.x - 6.0, y)],
                        egui::Stroke::new(3.0, self.pal.accent),
                    );
                }
                resp.context_menu(|ui| {
                    if !self.tabs[ti].session.selection.pages.contains(&page.id) {
                        self.tabs[ti].session.selection.pages = vec![page.id];
                    }
                    for id in [
                        C::PageRotateClockwise,
                        C::PageRotateCounterClockwise,
                        C::PageDuplicate,
                        C::PageInsertBlank,
                        C::PageExtract,
                        C::PageDelete,
                    ] {
                        let en = self.command_enabled(id);
                        if ui
                            .add_enabled(
                                en,
                                egui::Button::new(tr(editor_core::command::info(id).title)),
                            )
                            .clicked()
                        {
                            self.run_command(ctx, id);
                            ui.close();
                        }
                    }
                });
            }
            drag_stopped = ctx.input(|i| i.pointer.any_released());
        });
        if let Some(i) = drag_started {
            let sel = &mut self.tabs[ti].session.selection.pages;
            if !sel.contains(&pages[i].id) {
                *sel = vec![pages[i].id];
            }
            self.tabs[ti].ui.thumb_drag = Some(self.tabs[ti].session.selection.pages.clone());
        }
        if drag_active {
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            if drag_stopped {
                let moving = self.tabs[ti].ui.thumb_drag.take().unwrap_or_default();
                if let Some(target) = drop_target {
                    // `target` indexes the full list; convert to an index among the remaining pages.
                    let removed_before = pages
                        .iter()
                        .take(target)
                        .filter(|p| moving.contains(&p.id))
                        .count();
                    let to = target - removed_before;
                    let r = self.tabs[ti].session.execute(tr("Reorder pages"), |tx| {
                        pdf_engine::pageops::move_pages(tx, &moving, to)
                    });
                    if let Err(e) = r {
                        self.notify_error(e.to_string());
                    }
                }
            }
        }
        if let Some((i, mods)) = clicked {
            let id = pages[i].id;
            let anchor = self.tabs[ti].ui.thumb_anchor;
            let sel = &mut self.tabs[ti].session.selection.pages;
            if mods.shift
                && let Some(a) = anchor
            {
                let (lo, hi) = (a.min(i), a.max(i));
                *sel = pages[lo..=hi].iter().map(|p| p.id).collect();
            } else if mods.command {
                if let Some(pos) = sel.iter().position(|p| *p == id) {
                    sel.remove(pos);
                } else {
                    sel.push(id);
                }
                self.tabs[ti].ui.thumb_anchor = Some(i);
            } else {
                *sel = vec![id];
                self.tabs[ti].ui.thumb_anchor = Some(i);
            }
            self.go_to(i);
        }
    }

    fn bookmarks(&mut self, ui: &mut egui::Ui) {
        let ti = self.active;
        let rev = self.tabs[ti].session.revision();
        let need = !matches!(&self.tabs[ti].ui.bookmarks, Some((r, _)) if *r == rev);
        if need {
            let b = Arc::new(pdf_engine::nav::outlines(self.tabs[ti].session.doc()));
            self.tabs[ti].ui.bookmarks = Some((rev, b));
        }
        let Some((_, marks)) = self.tabs[ti].ui.bookmarks.clone() else {
            return;
        };
        if marks.is_empty() {
            ui.add_space(12.0);
            ui.label(RichText::new(tr("This document has no bookmarks.")).color(self.pal.text_dim));
            return;
        }
        let mut goto: Option<usize> = None;
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                fn node(
                    ui: &mut egui::Ui,
                    b: &pdf_engine::nav::Bookmark,
                    goto: &mut Option<usize>,
                    id: egui::Id,
                ) {
                    let title = if b.title.is_empty() {
                        tr("(untitled)")
                    } else {
                        b.title.as_str()
                    };
                    if b.children.is_empty() {
                        if ui.selectable_label(false, title).clicked() {
                            *goto = b.page;
                        }
                    } else {
                        let h = egui::CollapsingHeader::new(title)
                            .id_salt(id)
                            .default_open(b.open)
                            .show(ui, |ui| {
                                for (n, c) in b.children.iter().enumerate() {
                                    node(ui, c, goto, id.with(n));
                                }
                            });
                        if h.header_response.clicked() {
                            *goto = b.page;
                        }
                    }
                }
                for (n, b) in marks.iter().enumerate() {
                    node(ui, b, &mut goto, egui::Id::new(("bm", n)));
                }
            });
        if let Some(p) = goto {
            self.go_to(p);
        }
    }

    fn search_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let ti = self.active;
        let mut run = false;
        // The button first, from the right; the field takes what is left, so the row never outgrows the panel.
        let row = Vec2::new(ui.available_width(), crate::ui_kit::CONTROL_H);
        ui.allocate_ui_with_layout(
            row,
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                if crate::ui_kit::primary_button(ui, tr("Go")).clicked() {
                    run = true;
                }
                let te = crate::ui_kit::singleline(&mut self.tabs[ti].session.search.query)
                    .hint_text(tr("Find in document"))
                    .desired_width(f32::INFINITY);
                let resp = ui.add_sized([ui.available_width(), crate::ui_kit::CONTROL_H], te);
                if self.search_focus {
                    resp.request_focus();
                    self.search_focus = false;
                }
                if resp.lost_focus() && ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                    run = true;
                    resp.request_focus();
                }
            },
        );
        if ui
            .checkbox(
                &mut self.tabs[ti].session.search.case_sensitive,
                tr("Match case"),
            )
            .changed()
        {
            run = true;
        }
        if run {
            self.start_search();
        }
        let s = self.tabs[ti].session.search.clone();
        ui.add_space(4.0);
        if s.running {
            ui.label(
                RichText::new(tf!("Searching… {}/{} pages", s.progress.0, s.progress.1))
                    .color(self.pal.text_dim),
            );
        } else if !s.query.trim().is_empty() && s.progress.1 > 0 {
            ui.label(
                RichText::new(if s.matches.len() == 1 {
                    tr("1 result").to_string()
                } else {
                    tf!("{} results", s.matches.len())
                })
                .color(self.pal.text_dim),
            );
        }
        // Two equal columns: the buttons share the panel width instead of asking for their own.
        ui.columns(2, |cols| {
            for c in cols.iter_mut() {
                c.spacing_mut().button_padding.x = 6.0;
            }
            let w = cols[0].available_width();
            let size = Vec2::new(w, crate::ui_kit::CONTROL_H);
            if cols[0]
                .add_enabled(
                    !s.matches.is_empty(),
                    egui::Button::new(tr("◀ Prev")).min_size(size),
                )
                .clicked()
            {
                self.run_command(ctx, C::FindPrevious);
            }
            if cols[1]
                .add_enabled(
                    !s.matches.is_empty(),
                    egui::Button::new(tr("Next ▶")).min_size(size),
                )
                .clicked()
            {
                self.run_command(ctx, C::FindNext);
            }
        });
        ui.separator();
        let mut pick: Option<usize> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (n, m) in s.matches.iter().enumerate() {
                    let sel = s.current == Some(n);
                    let text = format!("p.{}  {}", m.page_index + 1, m.snippet.trim());
                    let r = ui.add(
                        egui::Button::selectable(sel, RichText::new(text).size(12.0))
                            .wrap()
                            .min_size(Vec2::new(ui.available_width(), 0.0)),
                    );
                    if r.clicked() {
                        pick = Some(n);
                    }
                }
            });
        if let Some(n) = pick {
            let t = &mut self.tabs[ti];
            t.session.search.current = Some(n);
            t.ui.goto = Some(t.session.search.matches[n].page_index);
        }
    }

    // ---- right sidebar ---------------------------------------------------------------

    pub fn right_sidebar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut hide = false;
        egui::Sides::new().shrink_left().show(
            ui,
            |ui| {
                let mut tab = self.right_tab;
                Self::tab_strip(
                    ui,
                    "right_tabs",
                    &mut tab,
                    &[
                        (RightTab::Properties, Icon::Sliders, tr("Properties")),
                        (RightTab::Comments, Icon::Note, tr("Comments")),
                        (RightTab::Measure, Icon::Ruler, tr("Measure")),
                        (RightTab::Copilot, Icon::Sparkle, tr("Copilot")),
                    ],
                );
                self.right_tab = tab;
            },
            |ui| {
                hide = Self::collapse_button(
                    ui,
                    false,
                    tr("Hide the properties panel (View ▸ Right panel)"),
                );
            },
        );
        if hide {
            self.prefs.show_right_sidebar = false;
            self.prefs_dirty = true;
        }
        ui.separator();
        if self.right_tab == RightTab::Copilot {
            // The chat has its own scroll area and a fixed input row.
            self.copilot_panel(ui, ctx);
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| match self.right_tab {
                RightTab::Properties => self.properties(ui, ctx),
                RightTab::Comments => self.comments(ui),
                RightTab::Measure => self.measure_panel(ui, ctx),
                RightTab::Copilot => {}
            });
    }

    fn properties(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let ti = self.active;
        let sel = self.tabs[ti].session.selection.clone();
        if sel.content.is_some() && self.content_properties(ui, ctx) {
            return;
        }
        if let Some(ts) = &sel.text {
            ui.label(RichText::new(tr("Selected text")).strong());
            let n = ts.glyphs.len();
            ui.label(RichText::new(tf!("{} characters", n)).color(self.pal.text_dim));
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for (t, label) in [
                    (Tool::Highlight, tr("Highlight")),
                    (Tool::Underline, tr("Underline")),
                    (Tool::StrikeOut, tr("Strikeout")),
                ] {
                    if ui
                        .add_enabled(self.command_enabled(t.command()), egui::Button::new(label))
                        .clicked()
                    {
                        self.markup_current_selection(t);
                    }
                }
                if ui.button(tr("Copy")).clicked() {
                    self.run_command(ctx, C::EditCopy);
                }
            });
            self.related_tools(ui, ctx, &[C::Find, C::ToolHighlight, C::ToolNote]);
            return;
        }
        if sel.annotations.is_empty() {
            self.tool_options(ui, ctx);
            return;
        }
        let ids: Vec<_> = sel.annotations.iter().map(|(_, id)| *id).collect();
        let infos: Vec<_> = ids
            .iter()
            .filter_map(|id| annot::read_annotation(self.tabs[ti].session.doc().lopdf(), *id))
            .collect();
        let Some(first) = infos.first().cloned() else {
            return;
        };
        let can_edit = self.tabs[ti].session.doc().capabilities().can_edit;
        if infos.len() > 1 {
            ui.label(RichText::new(tf!("{} annotations selected", infos.len())).strong());
        } else {
            ui.label(RichText::new(pretty_subtype(&first.subtype)).strong());
            if !first.author.is_empty() {
                ui.label(
                    RichText::new(format!("by {}", first.author))
                        .color(self.pal.text_dim)
                        .size(12.0),
                );
            }
        }
        ui.add_space(6.0);
        let Some(base_spec) = first.spec.clone() else {
            ui.label(RichText::new(tr("This annotation type was created by another program. It is displayed and can be moved or deleted, but its appearance cannot be edited here.")).color(self.pal.text_dim).size(12.0));
            return;
        };
        // While a slider or colour is being dragged the edit lives in a draft; the document (and so
        // the page render) only changes once the mouse button / key is released.
        let held = ui.input(|i| i.pointer.any_down() || !i.keys_down.is_empty());
        let draft = self.tabs[ti]
            .ui
            .props_draft
            .as_ref()
            .filter(|(d, _)| *d == ids)
            .map(|(_, s)| s.clone());
        let mut spec = draft.unwrap_or_else(|| base_spec.clone());
        let before = spec.clone();
        let upright = sel
            .annotations
            .first()
            .map_or(0, |(p, _)| self.upright_rotation(*p));
        ui.add_enabled_ui(can_edit, |ui| {
            crate::ui_kit::card(ui, |ui| {
                use crate::ui_kit::{form_row, slider64};
                let color_label = match spec.kind {
                    AnnotationKind::Highlight { .. } => tr("Color"),
                    AnnotationKind::FreeText { .. } => tr("Border"),
                    _ => tr("Line color"),
                };
                form_row(ui, color_label, |ui| {
                    let mut c = [spec.color.0, spec.color.1, spec.color.2];
                    if ui.color_edit_button_rgb(&mut c).changed() {
                        spec.color = Rgb(c[0], c[1], c[2]);
                    }
                });
                if matches!(
                    spec.kind,
                    AnnotationKind::Rectangle { .. }
                        | AnnotationKind::Ellipse { .. }
                        | AnnotationKind::Cloud { .. }
                        | AnnotationKind::Polygon { .. }
                        | AnnotationKind::FreeText { .. }
                ) {
                    form_row(ui, tr("Fill"), |ui| {
                        let mut has = spec.fill.is_some();
                        if ui.checkbox(&mut has, "").changed() {
                            spec.fill = if has { Some(Rgb(1.0, 1.0, 0.85)) } else { None };
                        }
                        if let Some(f) = spec.fill {
                            let mut fc = [f.0, f.1, f.2];
                            if ui.color_edit_button_rgb(&mut fc).changed() {
                                spec.fill = Some(Rgb(fc[0], fc[1], fc[2]));
                            }
                        }
                    });
                }
                if matches!(
                    spec.kind,
                    AnnotationKind::FreeText { .. } | AnnotationKind::StampText { .. }
                ) {
                    form_row(ui, tr("Rotation"), |ui| {
                        if ui
                            .button("⟲")
                            .on_hover_text(tr("Rotate 90° counter-clockwise"))
                            .clicked()
                        {
                            turn_text(&mut spec, 90);
                        }
                        if ui
                            .button("⟳")
                            .on_hover_text(tr("Rotate 90° clockwise"))
                            .clicked()
                        {
                            turn_text(&mut spec, -90);
                        }
                        if ui
                            .add_enabled(spec.rotation != upright, egui::Button::new(tr("Upright")))
                            .on_hover_text(tr("Make the text read horizontally on screen"))
                            .clicked()
                        {
                            let delta = upright - spec.rotation;
                            turn_text(&mut spec, delta);
                        }
                    });
                }
                form_row(ui, tr("Opacity"), |ui| {
                    let mut op = (spec.opacity * 100.0) as f32;
                    if crate::ui_kit::slider(ui, &mut op, 5.0..=100.0, "%") {
                        spec.opacity = f64::from(op) / 100.0;
                    }
                });
                if !matches!(
                    spec.kind,
                    AnnotationKind::Highlight { .. }
                        | AnnotationKind::Underline { .. }
                        | AnnotationKind::StrikeOut { .. }
                        | AnnotationKind::Note { .. }
                        | AnnotationKind::Squiggly { .. }
                        | AnnotationKind::StampText { .. }
                ) || matches!(spec.kind, AnnotationKind::StampText { .. })
                {
                    form_row(ui, tr("Line width"), |ui| {
                        slider64(ui, &mut spec.border_width, 0.0..=20.0, " pt");
                    });
                    form_row(ui, tr("Style"), |ui| {
                        ui.selectable_value(
                            &mut spec.border_style,
                            BorderStyle::Solid,
                            tr("Solid"),
                        );
                        ui.selectable_value(
                            &mut spec.border_style,
                            BorderStyle::Dashed,
                            tr("Dashed"),
                        );
                    });
                }
                if let AnnotationKind::FreeText {
                    font_size,
                    text_color,
                    font,
                    ..
                } = &mut spec.kind
                {
                    form_row(ui, tr("Font size"), |ui| {
                        slider64(ui, font_size, 6.0..=72.0, " pt");
                    });
                    form_row(ui, tr("Text color"), |ui| {
                        let mut tc = [text_color.0, text_color.1, text_color.2];
                        if ui.color_edit_button_rgb(&mut tc).changed() {
                            *text_color = Rgb(tc[0], tc[1], tc[2]);
                        }
                    });
                    form_row(ui, tr("Font"), |ui| {
                        crate::fontpick::font_picker(ui, "freetext_font", font);
                    });
                }
            });
            ui.add_space(6.0);
            ui.label(
                if matches!(
                    spec.kind,
                    AnnotationKind::FreeText { .. } | AnnotationKind::StampText { .. }
                ) {
                    tr("Text")
                } else {
                    tr("Comment")
                },
            );
            ui.add(
                crate::ui_kit::multiline(&mut spec.contents)
                    .desired_rows(4)
                    .desired_width(f32::INFINITY)
                    .hint_text(tr("Add a comment…")),
            );
        });
        if spec != before && can_edit {
            self.tabs[ti].ui.props_draft = Some((ids.clone(), spec.clone()));
        }
        let pending = self.tabs[ti]
            .ui
            .props_draft
            .as_ref()
            .is_some_and(|(d, _)| *d == ids);
        if pending && held {
            ui.ctx().request_repaint();
        }
        if pending && !held {
            self.tabs[ti].ui.props_draft = None;
        }
        if pending && !held && spec != base_spec && can_edit {
            // Edits from the panel apply to every selected annotation of the same kind.
            let targets: Vec<(annot::AnnotId, AnnotationSpec)> = infos
                .iter()
                .filter_map(|i| {
                    let s = i.spec.clone()?;
                    if infos.len() == 1 {
                        return Some((i.id, spec.clone()));
                    }
                    let mut s2 = s;
                    s2.color = spec.color;
                    s2.opacity = spec.opacity;
                    s2.border_width = spec.border_width;
                    s2.border_style = spec.border_style;
                    Some((i.id, s2))
                })
                .collect();
            let key = format!("props:{:?}", ids);
            let r = self.tabs[ti].session.execute_coalesced(
                &key,
                tr("Change annotation properties"),
                |tx| {
                    for (id, s) in &targets {
                        annot::update_annotation(tx, *id, s)?;
                    }
                    Ok(())
                },
            );
            if let Err(e) = r {
                self.notify_error(e.to_string());
            }
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(can_edit, egui::Button::new(tr("Duplicate")))
                .clicked()
            {
                self.run_command(ctx, C::EditDuplicate);
            }
            if ui
                .add_enabled(
                    can_edit,
                    egui::Button::new(RichText::new(tr("Delete")).color(self.pal.danger)),
                )
                .clicked()
            {
                self.run_command(ctx, C::EditDelete);
            }
        });
        self.related_tools(ui, ctx, &[C::ToolSelect, C::ToolFreeText, C::ToolArrow]);
    }

    fn tool_options(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let t = self.tool;
        ui.label(RichText::new(tr(editor_core::command::info(t.command()).title)).strong());
        ui.label(
            RichText::new(tr(t.hint()))
                .color(self.pal.text_dim)
                .size(12.0),
        );
        ui.add_space(8.0);
        use editor_core::tools::ToolFamily;
        if t.family() == ToolFamily::Annotation {
            ui.label(
                RichText::new(tr("Defaults for new markup"))
                    .size(12.0)
                    .color(self.pal.text_dim),
            );
            let d = &mut self.prefs.tool_defaults;
            let mut changed = false;
            crate::ui_kit::card(ui, |ui| {
                use crate::ui_kit::{form_row, slider64};
                form_row(ui, tr("Highlight"), |ui| {
                    changed |= ui.color_edit_button_rgb(&mut d.highlight).changed();
                });
                form_row(ui, tr("Line color"), |ui| {
                    changed |= ui.color_edit_button_rgb(&mut d.stroke).changed();
                });
                form_row(ui, tr("Line width"), |ui| {
                    changed |= slider64(ui, &mut d.stroke_width, 0.5..=12.0, " pt");
                });
                form_row(ui, tr("Opacity"), |ui| {
                    let mut op = (d.opacity * 100.0) as f32;
                    if crate::ui_kit::slider(ui, &mut op, 5.0..=100.0, "%") {
                        d.opacity = f64::from(op) / 100.0;
                        changed = true;
                    }
                });
                form_row(ui, tr("Font size"), |ui| {
                    changed |= slider64(ui, &mut d.font_size, 6.0..=48.0, " pt");
                });
                form_row(ui, tr("Font"), |ui| {
                    let mut st = d.font_style();
                    if crate::fontpick::font_picker(ui, "defaults_font", &mut st) {
                        d.set_font_style(st);
                        changed = true;
                    }
                });
            });
            if changed {
                self.prefs_dirty = true;
            }
            ui.add_space(6.0);
            ui.label(tr("Author"));
            if ui
                .add(
                    crate::ui_kit::singleline(&mut self.prefs.author)
                        .hint_text(tr("Your name"))
                        .desired_width(f32::INFINITY),
                )
                .changed()
            {
                self.prefs_dirty = true;
            }
        } else {
            ui.label(
                RichText::new(tr(
                    "Select an annotation or some text to see its properties.",
                ))
                .color(self.pal.text_dim),
            );
        }
        // Document / page info.
        if let Some(tab) = self.tabs.get_mut(self.active)
            && let Ok(pages) = tab.session.pages()
            && let Some(p) = pages.get(tab.session.view.current_page)
        {
            ui.add_space(14.0);
            ui.label(RichText::new(tr("Page")).strong());
            let s = p.geometry.view_size(Rotation::R0);
            ui.label(
                RichText::new(tf!(
                    "Size: {}",
                    crate::chrome::format_size(s.width, s.height)
                ))
                .size(12.0),
            );
            ui.label(RichText::new(tf!("Rotation: {}°", p.geometry.rotate.degrees())).size(12.0));
            if (p.geometry.user_unit - 1.0).abs() > 1e-9 {
                ui.label(RichText::new(tf!("UserUnit: {}", p.geometry.user_unit)).size(12.0));
            }
        }
        self.related_tools(
            ui,
            ctx,
            &[C::ToolHighlight, C::ToolFreeText, C::ToolRectangle],
        );
    }

    /// Non-intrusive suggestions relevant to the current context.
    fn related_tools(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, ids: &[C]) {
        ui.add_space(14.0);
        ui.label(
            RichText::new(tr("Related tools"))
                .size(11.0)
                .color(self.pal.text_dim),
        );
        ui.horizontal_wrapped(|ui| {
            for &id in ids {
                if self.cmd_button(ui, id, false, None, true) {
                    self.run_command(ctx, id);
                }
            }
            let recent: Vec<C> = self
                .prefs
                .recent_tools
                .iter()
                .copied()
                .filter(|c| !ids.contains(c))
                .take(3)
                .collect();
            for id in recent {
                if self.cmd_button(ui, id, false, None, true) {
                    self.run_command(ctx, id);
                }
            }
        });
        let _ = command_icon;
    }

    fn comments(&mut self, ui: &mut egui::Ui) {
        let ti = self.active;
        let rev = self.tabs[ti].session.revision();
        let need = !matches!(&self.tabs[ti].ui.comments, Some((r, _)) if *r == rev);
        if need {
            let mut all = Vec::new();
            if let Ok(ids) = self.tabs[ti].session.doc().page_ids() {
                for (i, p) in ids.iter().enumerate() {
                    for a in annot::read_annotations(self.tabs[ti].session.doc(), *p) {
                        if !matches!(a.subtype.as_str(), "Link" | "Widget" | "Popup") {
                            all.push((i, *p, a));
                        }
                    }
                    if all.len() > 5000 {
                        break;
                    }
                }
            }
            self.tabs[ti].ui.comments = Some((rev, Arc::new(all)));
        }
        let Some((_, list)) = self.tabs[ti].ui.comments.clone() else {
            return;
        };
        ui.add(
            crate::ui_kit::singleline(&mut self.tabs[ti].ui.comments_filter)
                .hint_text(tr("Filter comments"))
                .desired_width(f32::INFINITY),
        );
        ui.add_space(4.0);
        if list.is_empty() {
            ui.label(RichText::new(tr("No comments or markup yet.")).color(self.pal.text_dim));
            return;
        }
        let filter = self.tabs[ti].ui.comments_filter.to_lowercase();
        let selected: Vec<_> = self.tabs[ti]
            .session
            .selection
            .annotations
            .iter()
            .map(|(_, id)| *id)
            .collect();
        let mut pick: Option<(usize, pdf_engine::doc::PageId, annot::AnnotId)> = None;
        for (pi, page, a) in list.iter() {
            let hay = format!("{} {} {}", a.subtype, a.contents, a.author).to_lowercase();
            if !filter.is_empty() && !hay.contains(&filter) {
                continue;
            }
            let sel = selected.contains(&a.id);
            let frame = egui::Frame::new()
                .fill(if sel {
                    self.pal.accent_soft
                } else {
                    Color32::TRANSPARENT
                })
                .corner_radius(5)
                .inner_margin(6);
            let r = frame
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(pretty_subtype(&a.subtype))
                                .strong()
                                .size(12.0),
                        );
                        ui.label(
                            RichText::new(format!("p.{}", pi + 1))
                                .color(self.pal.text_dim)
                                .size(11.0),
                        );
                        if !a.author.is_empty() {
                            ui.label(RichText::new(&a.author).color(self.pal.text_dim).size(11.0));
                        }
                    });
                    if !a.contents.is_empty() {
                        ui.label(RichText::new(&a.contents).size(12.0));
                    }
                })
                .response
                .interact(Sense::click());
            if r.clicked() {
                pick = Some((*pi, *page, a.id));
            }
        }
        if let Some((pi, page, id)) = pick {
            let t = &mut self.tabs[ti];
            t.session.selection.select_annotation(page, id, false);
            t.ui.goto = Some(pi);
            self.tool = Tool::Select;
        }
    }
}

pub fn pretty_subtype(s: &str) -> String {
    match s {
        "Square" => tr("Rectangle").into(),
        "Circle" => tr("Ellipse").into(),
        "Text" => tr("Note").into(),
        "FreeText" => tr("Text box").into(),
        "StrikeOut" => tr("Strikethrough").into(),
        "PolyLine" => tr("Polyline").into(),
        "Ink" => tr("Drawing").into(),
        other => other.to_string(),
    }
}

/// Turn a text box or stamp by `delta` degrees counter-clockwise (multiples of 90). A quarter turn
/// also swaps the box's width and height around its centre, so the box turns with its text.
fn turn_text(spec: &mut AnnotationSpec, delta: i32) {
    let delta = delta.rem_euclid(360);
    if delta == 0 {
        return;
    }
    if delta % 180 != 0 {
        let swap = |r: &mut pdf_engine::geom::Rect| {
            let c = pdf_engine::geom::Point::new((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0);
            let (hw, hh) = (r.width().abs() / 2.0, r.height().abs() / 2.0);
            *r = pdf_engine::geom::Rect::new(c.x - hh, c.y - hw, c.x + hh, c.y + hw);
        };
        match &mut spec.kind {
            AnnotationKind::FreeText { rect, .. } | AnnotationKind::StampText { rect, .. } => {
                swap(rect);
            }
            _ => {}
        }
    }
    spec.rotation = (spec.rotation + delta).rem_euclid(360);
}
