//! Page-content editing tools (Edit Text / Add Text / Add Image).
//!
//! These change the document's real content stream, unlike annotations. The UI keeps that
//! distinction visible (separate ribbon tab and panel wording) and is explicit about limits:
//! unsupported text is shown with the reason, missing glyphs are listed, font substitution is
//! a deliberate button, and width/overlap effects are reported after each edit.

use crate::canvas::ViewCtx;
use crate::state::*;
use editor_core::selection::ContentRef;
use editor_core::tools::Tool;
use egui::{Color32, Pos2, Rect, RichText, Stroke, Vec2};
use pdf_engine::doc::PageId;
use pdf_engine::error::EngineError;
use pdf_engine::geom::{Point, Rect as PRect};
use pdf_engine::pagecontent::{self, ImageInfo, PageContent, TextEdit, TextRunInfo};
use std::sync::Arc;

impl App {
    /// Editable objects of a page for the current revision (cached; computed on demand).
    pub fn objects_for(&mut self, page: PageId) -> Arc<PageObjects> {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return Arc::new(PageObjects {
                runs: vec![],
                images: vec![],
                error: None,
            });
        };
        let rev = tab.session.revision();
        if let Some((r, o)) = tab.ui.objects.get(&page)
            && *r == rev
        {
            return o.clone();
        }
        let doc = tab.session.doc();
        let objs = match PageContent::load(doc.lopdf(), page.0) {
            Ok(pc) => PageObjects {
                runs: pc.text_runs(),
                images: pc.images(doc.lopdf()),
                error: None,
            },
            Err(e) => PageObjects {
                runs: vec![],
                images: vec![],
                error: Some(e.to_string()),
            },
        };
        let objs = Arc::new(objs);
        tab.ui.objects.insert(page, (rev, objs.clone()));
        objs
    }

    fn run_hit(objs: &PageObjects, p: Point, tol: f64) -> Option<&TextRunInfo> {
        objs.runs.iter().rev().find(|r| {
            r.quad.contains(p)
                || r.quad.bounds().inflate(tol, tol).contains(p)
                    && r.quad.bounds().inflate(tol, tol).area()
                        < r.quad.bounds().area() * 1.6 + 40.0
        })
    }

    fn image_hit(objs: &PageObjects, p: Point) -> Option<&ImageInfo> {
        objs.images
            .iter()
            .rev()
            .find(|i| i.quad.bounds().contains(p))
    }

    /// Dispatch for the three page-content tools.
    pub fn content_tool(
        &mut self,
        response: &egui::Response,
        vc: &ViewCtx,
        pos: Option<Pos2>,
        tool: Tool,
    ) {
        let Some(pos) = pos else { return };
        match tool {
            Tool::EditText => self.edit_tool(response, vc, pos),
            Tool::AddText => {
                if response.clicked_by(egui::PointerButton::Primary)
                    && let Some(i) = vc.page_at(pos)
                {
                    let pt = vc.screen_to_pdf(i, pos);
                    self.dialog = Some(Dialog::AddText {
                        page: vc.pages[i].id,
                        at: pt,
                        text: String::new(),
                        size: self.prefs.tool_defaults.font_size.max(8.0),
                        font: self.prefs.tool_defaults.font_style(),
                    });
                }
            }
            Tool::AddImage => self.add_image_tool(response, vc, pos),
            _ => {}
        }
    }

    fn edit_tool(&mut self, response: &egui::Response, vc: &ViewCtx, pos: Pos2) {
        let ti = self.active;
        let can_edit = self.tabs[ti].session.doc().capabilities().can_edit;
        let Some(i) = vc.page_at(pos) else {
            if response.clicked() {
                self.tabs[ti].session.selection.content = None;
                self.tabs[ti].ui.edit_draft = None;
            }
            return;
        };
        let page = vc.pages[i].id;
        let objs = self.objects_for(page);
        let pt = vc.screen_to_pdf(i, pos);
        let tol = 2.0 / vc.px_per_pt;
        if response.drag_started_by(egui::PointerButton::Primary) {
            // Resize handle of the selected image?
            if let Some((sp, ContentRef::Image(id))) = self.tabs[ti].session.selection.content
                && sp == page
                && let Some(img) = objs.images.iter().find(|x| x.id == id)
            {
                let r = Self::rect_screen(vc, i, img.quad.bounds()).expand(2.0);
                for (h, hp) in Self::handle_positions(r).iter().enumerate() {
                    if hp.distance(pos) <= crate::interaction::HANDLE {
                        self.tabs[ti].ui.interaction = Interaction::ContentResize {
                            page,
                            handle: h,
                            rect: img.quad.bounds(),
                        };
                        return;
                    }
                }
            }
            let hit_run = Self::run_hit(&objs, pt, tol).map(|r| r.id);
            let hit_img = if hit_run.is_none() {
                Self::image_hit(&objs, pt).map(|x| x.id)
            } else {
                None
            };
            let selected = self.tabs[ti].session.selection.content;
            let sel_now = hit_run
                .map(ContentRef::Text)
                .or(hit_img.map(ContentRef::Image));
            if let Some(c) = sel_now {
                if selected != Some((page, c)) {
                    self.select_content(page, c, &objs);
                }
                if can_edit {
                    // Only editable text can be dragged.
                    let draggable = match c {
                        ContentRef::Text(id) => objs
                            .runs
                            .iter()
                            .find(|r| r.id == id)
                            .is_some_and(|r| r.editable.is_ok()),
                        ContentRef::Image(_) => true,
                    };
                    if draggable {
                        self.tabs[ti].ui.interaction =
                            Interaction::ContentMove { delta: (0.0, 0.0) };
                    }
                }
            } else {
                self.tabs[ti].session.selection.content = None;
                self.tabs[ti].ui.edit_draft = None;
                self.tabs[ti].ui.interaction = Interaction::Panning;
            }
        } else if response.clicked_by(egui::PointerButton::Primary) {
            let hit_run = Self::run_hit(&objs, pt, tol).map(|r| r.id);
            let hit_img = if hit_run.is_none() {
                Self::image_hit(&objs, pt).map(|x| x.id)
            } else {
                None
            };
            match hit_run
                .map(ContentRef::Text)
                .or(hit_img.map(ContentRef::Image))
            {
                Some(c) => self.select_content(page, c, &objs),
                None => {
                    self.tabs[ti].session.selection.content = None;
                    self.tabs[ti].ui.edit_draft = None;
                }
            }
        } else if response.dragged_by(egui::PointerButton::Primary) {
            match self.tabs[ti].ui.interaction.clone() {
                Interaction::ContentMove { delta } => {
                    let (dx, dy) = vc.delta_to_pdf(i, response.drag_delta());
                    self.tabs[ti].ui.interaction = Interaction::ContentMove {
                        delta: (delta.0 + dx, delta.1 + dy),
                    };
                }
                Interaction::ContentResize {
                    page: p,
                    handle,
                    rect,
                } => {
                    if let Some(pi) = vc.pages.iter().position(|x| x.id == p) {
                        let mut r = Self::rect_screen(vc, pi, rect);
                        let d = response.drag_delta();
                        let (l, t, rr, b) = match handle {
                            0 => (true, true, false, false),
                            1 => (false, true, false, false),
                            2 => (false, true, true, false),
                            3 => (false, false, true, false),
                            4 => (false, false, true, true),
                            5 => (false, false, false, true),
                            6 => (true, false, false, true),
                            _ => (true, false, false, false),
                        };
                        if l {
                            r.min.x += d.x;
                        }
                        if rr {
                            r.max.x += d.x;
                        }
                        if t {
                            r.min.y += d.y;
                        }
                        if b {
                            r.max.y += d.y;
                        }
                        let nr = Self::screen_rect_to_pdf(vc, pi, Rect::from_two_pos(r.min, r.max));
                        self.tabs[ti].ui.interaction = Interaction::ContentResize {
                            page: p,
                            handle,
                            rect: nr,
                        };
                    }
                }
                Interaction::Panning => {
                    self.tabs[ti].session.view.scroll -=
                        crate::canvas::to_core(response.drag_delta())
                }
                _ => {}
            }
        } else if response.drag_stopped() {
            match std::mem::take(&mut self.tabs[ti].ui.interaction) {
                Interaction::ContentMove { delta } if delta.0.abs() + delta.1.abs() > 0.05 => {
                    self.commit_content_move(delta)
                }
                Interaction::ContentResize { page, rect, .. } => self.commit_image_box(page, rect),
                _ => {}
            }
        }
    }

    fn select_content(&mut self, page: PageId, c: ContentRef, objs: &PageObjects) {
        let ti = self.active;
        self.tabs[ti].session.selection.annotations.clear();
        self.tabs[ti].session.selection.text = None;
        self.tabs[ti].session.selection.content = Some((page, c));
        self.tabs[ti].ui.edit_draft = None;
        if let ContentRef::Text(id) = c
            && let Some(r) = objs.runs.iter().find(|r| r.id == id)
        {
            self.tabs[ti].ui.edit_draft = Some(EditDraft {
                page,
                run: id,
                original: r.text.clone(),
                text: r.text.clone(),
                size: (r.size_pt * 100.0).round() / 100.0,
                original_size: (r.size_pt * 100.0).round() / 100.0,
                fit_width: false,
                original_font: r.base_font.clone(),
                font: None,
                message: r
                    .editable
                    .clone()
                    .err()
                    .map(|e| (true, format!("This text can't be edited: {e}."))),
                missing: None,
                report: None,
            });
        }
    }

    fn commit_content_move(&mut self, delta: (f64, f64)) {
        let ti = self.active;
        let Some((page, c)) = self.tabs[ti].session.selection.content else {
            return;
        };
        let objs = self.objects_for(page);
        let center = match c {
            ContentRef::Text(id) => objs
                .runs
                .iter()
                .find(|r| r.id == id)
                .map(|r| r.quad.bounds().center()),
            ContentRef::Image(id) => objs
                .images
                .iter()
                .find(|r| r.id == id)
                .map(|r| r.quad.bounds().center()),
        };
        let r = match c {
            ContentRef::Text(id) => self.tabs[ti].session.execute("Move text", |tx| {
                PageContent::load(tx.doc(), page.0)?
                    .edit_text(
                        tx,
                        id,
                        &TextEdit {
                            shift: Some(delta),
                            ..Default::default()
                        },
                    )
                    .map(|_| ())
            }),
            ContentRef::Image(id) => {
                let Some(img) = objs.images.iter().find(|x| x.id == id) else {
                    return;
                };
                let b = img.quad.bounds();
                let nb = PRect::new(
                    b.x0 + delta.0,
                    b.y0 + delta.1,
                    b.x1 + delta.0,
                    b.y1 + delta.1,
                );
                self.tabs[ti].session.execute("Move image", |tx| {
                    PageContent::load(tx.doc(), page.0)?.place_image(tx, id, nb)
                })
            }
        };
        match r {
            Ok(()) => {
                if let Some(c0) = center {
                    self.reselect_near(
                        page,
                        Point::new(c0.x + delta.0, c0.y + delta.1),
                        matches!(c, ContentRef::Text(_)),
                    );
                }
            }
            Err(e) => self.notify_error(format!("Could not move: {e}")),
        }
    }

    fn commit_image_box(&mut self, page: PageId, rect: PRect) {
        let ti = self.active;
        let Some((p, ContentRef::Image(id))) = self.tabs[ti].session.selection.content else {
            return;
        };
        if p != page || rect.width() < 2.0 || rect.height() < 2.0 {
            return;
        }
        let r = self.tabs[ti].session.execute("Resize image", |tx| {
            PageContent::load(tx.doc(), page.0)?.place_image(tx, id, rect)
        });
        match r {
            Ok(()) => self.reselect_near(page, rect.center(), false),
            Err(e) => self.notify_error(format!("Could not resize: {e}")),
        }
    }

    /// After an edit the object references change; select the object nearest to `center`.
    fn reselect_near(&mut self, page: PageId, center: Point, text: bool) {
        let ti = self.active;
        let objs = self.objects_for(page);
        let dist = |p: Point| (p.x - center.x).hypot(p.y - center.y);
        let pick = if text {
            objs.runs
                .iter()
                .min_by(|a, b| {
                    dist(a.quad.bounds().center()).total_cmp(&dist(b.quad.bounds().center()))
                })
                .map(|r| ContentRef::Text(r.id))
        } else {
            objs.images
                .iter()
                .min_by(|a, b| {
                    dist(a.quad.bounds().center()).total_cmp(&dist(b.quad.bounds().center()))
                })
                .map(|r| ContentRef::Image(r.id))
        };
        if let Some(c) = pick {
            let keep = self.tabs[ti].ui.edit_draft.clone();
            self.select_content(page, c, &objs);
            if let (Some(old), Some(new)) = (keep, self.tabs[ti].ui.edit_draft.as_mut()) {
                new.report = old.report;
                new.message = old.message.filter(|(err, _)| !*err);
                new.fit_width = old.fit_width;
            }
        } else {
            self.tabs[ti].session.selection.content = None;
            self.tabs[ti].ui.edit_draft = None;
        }
    }

    fn add_image_tool(&mut self, response: &egui::Response, vc: &ViewCtx, pos: Pos2) {
        let ti = self.active;
        if response.drag_started_by(egui::PointerButton::Primary) {
            let Some(i) = vc.page_at(pos) else { return };
            let p = vc.screen_to_pdf(i, pos);
            self.tabs[ti].ui.interaction = Interaction::Draw {
                page: vc.pages[i].id,
                tool: Tool::AddImage,
                points: vec![p, p],
            };
        } else if response.dragged_by(egui::PointerButton::Primary)
            && let Interaction::Draw {
                page,
                tool,
                mut points,
            } = self.tabs[ti].ui.interaction.clone()
            && let Some(i) = vc.pages.iter().position(|p| p.id == page)
        {
            points.truncate(1);
            points.push(vc.screen_to_pdf(i, pos));
            self.tabs[ti].ui.interaction = Interaction::Draw { page, tool, points };
        } else if response.drag_stopped()
            && let Interaction::Draw { page, points, .. } =
                std::mem::take(&mut self.tabs[ti].ui.interaction)
            && points.len() >= 2
        {
            let r = PRect::new(points[0].x, points[0].y, points[1].x, points[1].y).abs();
            if r.width() < 8.0 || r.height() < 8.0 {
                self.notify("Drag a larger box to place the image.");
                return;
            }
            let Some(path) = platform::dialogs::pick_image() else {
                return;
            };
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let res = self.tabs[ti].session.execute("Add image", |tx| {
                        pagecontent::add_image(tx, page, r, &bytes)
                    });
                    match res {
                        Ok(()) => {
                            self.reselect_near(page, r.center(), false);
                            self.set_tool(Tool::EditText);
                        }
                        Err(e) => {
                            self.dialog = Some(Dialog::Error {
                                title: "Could not add image".into(),
                                detail: e.to_string(),
                            })
                        }
                    }
                }
                Err(e) => {
                    self.dialog = Some(Dialog::Error {
                        title: "Could not read the image".into(),
                        detail: e.to_string(),
                    })
                }
            }
        }
    }

    pub fn commit_add_text(
        &mut self,
        page: PageId,
        at: Point,
        text: &str,
        size: f64,
        font: pdf_engine::fontembed::FontStyle,
    ) {
        let ti = self.active;
        let col = (0.0f32, 0.0f32, 0.0f32);
        let r = self.tabs[ti].session.execute("Add text", |tx| {
            pagecontent::add_text(tx, page, at, text, size, col, font)
        });
        match r {
            Ok(()) => {
                self.reselect_near(page, Point::new(at.x + 20.0, at.y + size / 3.0), true);
                self.set_tool(Tool::EditText);
            }
            Err(EngineError::MissingGlyphs { chars, .. }) => {
                self.dialog = Some(Dialog::Error {
                    title: "Some characters are not available".into(),
                    detail: format!(
                        "The chosen font cannot show: {chars}\nThe text was not added. Remove those characters or pick a font that has them."
                    ),
                });
            }
            Err(e) => {
                self.dialog = Some(Dialog::Error {
                    title: "Could not add text".into(),
                    detail: e.to_string(),
                })
            }
        }
    }

    pub fn apply_text_draft(&mut self, force_font: Option<pdf_engine::fontembed::FontStyle>) {
        let ti = self.active;
        let Some(mut d) = self.tabs[ti].ui.edit_draft.clone() else {
            return;
        };
        let font = force_font.or(d.font);
        let text_changed = d.text != d.original;
        let size_changed = (d.size - d.original_size).abs() > 0.005;
        if !text_changed && !size_changed && font.is_none() {
            d.message = Some((false, "No changes to apply.".into()));
            self.tabs[ti].ui.edit_draft = Some(d);
            return;
        }
        let edit = TextEdit {
            // A font change re-encodes the text, so the text is always passed along with it.
            text: (text_changed || font.is_some()).then(|| d.text.clone()),
            size_pt: size_changed.then_some(d.size),
            shift: None,
            fit_width: d.fit_width,
            substitute_font: font,
        };
        let (page, run) = (d.page, d.run);
        let center = self
            .objects_for(page)
            .runs
            .iter()
            .find(|r| r.id == run)
            .map(|r| r.quad.bounds().center());
        let r = self.tabs[ti].session.execute("Edit text", |tx| {
            PageContent::load(tx.doc(), page.0)?.edit_text(tx, run, &edit)
        });
        match r {
            Ok(report) => {
                let mut msg = String::from("Text updated.");
                if !report.warnings.is_empty() {
                    msg = report.warnings.join(" ");
                }
                if let Some(c) = center {
                    self.reselect_near(page, c, true);
                }
                if let Some(nd) = self.tabs[ti].ui.edit_draft.as_mut() {
                    nd.report = Some(report.clone());
                    nd.message = Some((false, msg));
                }
            }
            Err(EngineError::MissingGlyphs { font, chars }) => {
                d.missing = Some((font, chars));
                d.message = Some((
                    true,
                    "The font cannot show some of these characters.".into(),
                ));
                self.tabs[ti].ui.edit_draft = Some(d);
            }
            Err(e) => {
                d.missing = None;
                d.message = Some((true, e.to_string()));
                self.tabs[ti].ui.edit_draft = Some(d);
            }
        }
    }

    pub fn delete_selected_content(&mut self) {
        let ti = self.active;
        let Some((page, c)) = self.tabs[ti].session.selection.content else {
            return;
        };
        let r = match c {
            ContentRef::Text(id) => self.tabs[ti].session.execute("Delete text", |tx| {
                PageContent::load(tx.doc(), page.0)?.delete_text(tx, id)
            }),
            ContentRef::Image(id) => self.tabs[ti].session.execute("Delete image", |tx| {
                PageContent::load(tx.doc(), page.0)?.delete_image(tx, id)
            }),
        };
        self.tabs[ti].session.selection.content = None;
        self.tabs[ti].ui.edit_draft = None;
        if let Err(e) = r {
            self.notify_error(format!("Could not delete: {e}"));
        }
    }

    pub fn replace_selected_image(&mut self) {
        let ti = self.active;
        let Some((page, ContentRef::Image(id))) = self.tabs[ti].session.selection.content else {
            return;
        };
        let Some(path) = platform::dialogs::pick_image() else {
            return;
        };
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                self.dialog = Some(Dialog::Error {
                    title: "Could not read the image".into(),
                    detail: e.to_string(),
                });
                return;
            }
        };
        let center = self
            .objects_for(page)
            .images
            .iter()
            .find(|i| i.id == id)
            .map(|i| i.quad.bounds().center());
        let r = self.tabs[ti].session.execute("Replace image", |tx| {
            PageContent::load(tx.doc(), page.0)?.replace_image(tx, id, &bytes, false)
        });
        match r {
            Ok(()) => {
                if let Some(c) = center {
                    self.reselect_near(page, c, false);
                }
            }
            Err(e) => {
                self.dialog = Some(Dialog::Error {
                    title: "Could not replace the image".into(),
                    detail: e.to_string(),
                })
            }
        }
    }

    /// Overlays for the page-content tools: editable runs, hover, selection, handles.
    pub fn paint_content_overlays(&mut self, painter: &egui::Painter, vc: &ViewCtx) {
        if !matches!(self.tool, Tool::EditText | Tool::AddText | Tool::AddImage) {
            return;
        }
        let ti = self.active;
        let band = (
            f64::from(vc.scroll.y - 20.0),
            f64::from(vc.scroll.y + vc.viewport.height() + 20.0),
        );
        let hover = painter.ctx().input(|i| i.pointer.hover_pos());
        let sel = self.tabs[ti].session.selection.content;
        let inter = self.tabs[ti].ui.interaction.clone();
        let accent = self.pal.accent;
        for i in vc.layout.pages_in_band(band.0, band.1) {
            let page = vc.pages[i].id;
            if self.tool != Tool::EditText {
                continue;
            }
            let objs = self.objects_for(page);
            if let Some(e) = &objs.error {
                painter.text(
                    vc.slot(i).center_top() + Vec2::new(0.0, 10.0),
                    egui::Align2::CENTER_TOP,
                    format!("Editing unavailable: {e}"),
                    egui::FontId::proportional(12.0),
                    self.pal.danger,
                );
                continue;
            }
            let hover_pt = hover
                .filter(|p| vc.slot(i).contains(*p))
                .map(|p| vc.screen_to_pdf(i, p));
            for r in &objs.runs {
                let pts: Vec<Pos2> = r.quad.0.iter().map(|p| vc.pdf_to_screen(i, *p)).collect();
                let is_sel = sel == Some((page, ContentRef::Text(r.id)));
                let hov = hover_pt.is_some_and(|p| r.quad.contains(p));
                let editable = r.editable.is_ok();
                let (fill, stroke) = if is_sel {
                    (
                        Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 40),
                        Stroke::new(1.5, accent),
                    )
                } else if hov {
                    (
                        Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 28),
                        Stroke::new(1.0, accent),
                    )
                } else if editable {
                    (
                        Color32::TRANSPARENT,
                        Stroke::new(0.75, accent.gamma_multiply(0.35)),
                    )
                } else {
                    (
                        Color32::TRANSPARENT,
                        Stroke::new(0.75, self.pal.text_dim.gamma_multiply(0.5)),
                    )
                };
                let mut pts_moved = pts.clone();
                if is_sel && let Interaction::ContentMove { delta } = &inter {
                    let off = vc.pdf_to_screen(i, Point::new(delta.0, delta.1))
                        - vc.pdf_to_screen(i, Point::new(0.0, 0.0));
                    pts_moved = pts.iter().map(|p| *p + off).collect();
                }
                painter.add(egui::epaint::PathShape::convex_polygon(
                    pts_moved, fill, stroke,
                ));
                if hov && let Err(reason) = &r.editable {
                    painter.text(
                        pts[0] + Vec2::new(0.0, -4.0),
                        egui::Align2::LEFT_BOTTOM,
                        format!("Not editable: {reason}"),
                        egui::FontId::proportional(11.0),
                        self.pal.danger,
                    );
                }
            }
            for im in &objs.images {
                let is_sel = sel == Some((page, ContentRef::Image(im.id)));
                let mut rect = im.quad.bounds();
                match (&inter, is_sel) {
                    (Interaction::ContentMove { delta }, true) => {
                        rect = PRect::new(
                            rect.x0 + delta.0,
                            rect.y0 + delta.1,
                            rect.x1 + delta.0,
                            rect.y1 + delta.1,
                        )
                    }
                    (Interaction::ContentResize { rect: nr, .. }, true) => rect = *nr,
                    _ => {}
                }
                let r = Self::rect_screen(vc, i, rect);
                let hov = hover_pt.is_some_and(|p| im.quad.bounds().contains(p));
                let stroke = if is_sel {
                    Stroke::new(1.5, accent)
                } else if hov {
                    Stroke::new(1.0, accent)
                } else {
                    Stroke::new(0.75, accent.gamma_multiply(0.3))
                };
                painter.rect_stroke(r, 0.0, stroke, egui::StrokeKind::Outside);
                if is_sel && !matches!(inter, Interaction::ContentMove { .. }) {
                    for hp in Self::handle_positions(r.expand(2.0)) {
                        let hr =
                            Rect::from_center_size(hp, Vec2::splat(crate::interaction::HANDLE));
                        painter.rect_filled(hr, 1.5, Color32::WHITE);
                        painter.rect_stroke(
                            hr,
                            1.5,
                            Stroke::new(1.5, accent),
                            egui::StrokeKind::Middle,
                        );
                    }
                }
            }
        }
        // Draw preview for Add Image.
        if let Interaction::Draw {
            page,
            tool: Tool::AddImage,
            points,
        } = &inter
            && let Some(i) = vc.pages.iter().position(|p| p.id == *page)
            && points.len() >= 2
        {
            let r = Rect::from_two_pos(
                vc.pdf_to_screen(i, points[0]),
                vc.pdf_to_screen(i, points[1]),
            );
            painter.rect_stroke(r, 0.0, Stroke::new(1.5, accent), egui::StrokeKind::Middle);
        }
    }

    /// Properties panel for a selected page-content object.
    pub fn content_properties(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) -> bool {
        let ti = self.active;
        let Some((page, c)) = self.tabs[ti].session.selection.content else {
            return false;
        };
        let objs = self.objects_for(page);
        let can_edit = self.tabs[ti].session.doc().capabilities().can_edit;
        ui.label(
            RichText::new("Page content")
                .size(11.0)
                .color(self.pal.text_dim),
        );
        match c {
            ContentRef::Text(id) => {
                let Some(run) = objs.runs.iter().find(|r| r.id == id).cloned() else {
                    return false;
                };
                ui.label(RichText::new("Edit text").strong());
                ui.label(
                    RichText::new("Changes the document's own text — not a comment.")
                        .size(11.0)
                        .color(self.pal.text_dim),
                );
                ui.add_space(4.0);
                let editable = run.editable.is_ok() && can_edit;
                ui.label(
                    RichText::new(format!(
                        "Font: {}{}{}",
                        if run.base_font.is_empty() {
                            "(unnamed)"
                        } else {
                            &run.base_font
                        },
                        if run.embedded {
                            " · embedded"
                        } else {
                            " · not embedded"
                        },
                        if run.subset { " subset" } else { "" }
                    ))
                    .size(12.0),
                );
                if let Err(reason) = &run.editable {
                    ui.colored_label(
                        self.pal.danger,
                        format!("This text can't be edited: {reason}."),
                    );
                }
                let mut apply = false;
                let mut apply_sub = false;
                let mut revert = false;
                let mut del = false;
                if let Some(d) = self.tabs[ti].ui.edit_draft.as_mut() {
                    ui.add_enabled_ui(editable, |ui| {
                        ui.add(egui::TextEdit::multiline(&mut d.text).desired_rows(3).desired_width(f32::INFINITY));
                        ui.horizontal(|ui| {
                            ui.label("Size");
                            ui.add(egui::DragValue::new(&mut d.size).range(4.0..=300.0).speed(0.2).suffix(" pt"));
                        });
                        ui.horizontal(|ui| {
                            ui.label("Font");
                            let orig = d.original_font.to_lowercase();
                            let label = match d.font {
                                None => format!("Keep ({})", short_font_name(&d.original_font)),
                                Some(s) => s.family.title().to_string(),
                            };
                            egui::ComboBox::from_id_salt("edit_font").selected_text(label).width(190.0).show_ui(ui, |ui| {
                                if ui.selectable_label(d.font.is_none(), "Keep the original font").clicked() {
                                    d.font = None;
                                }
                                for f in pdf_engine::fontembed::FontFamily::ALL {
                                    let sel = d.font.is_some_and(|s| s.family == f);
                                    let t = egui::RichText::new(f.title()).family(crate::fontpick::egui_family(pdf_engine::fontembed::FontStyle::new(f, false, false)));
                                    if ui.selectable_label(sel, t).clicked() && !sel {
                                        d.font = Some(pdf_engine::fontembed::FontStyle::new(f, orig.contains("bold"), orig.contains("italic") || orig.contains("oblique")));
                                    }
                                }
                            });
                        });
                        if let Some(s) = &mut d.font {
                            ui.horizontal(|ui| {
                                crate::fontpick::font_picker_toggles(ui, s);
                            });
                            ui.label(RichText::new("This replaces the font of this text only; other text keeps its font.").size(11.0).color(self.pal.text_dim));
                        }
                        ui.checkbox(&mut d.fit_width, "Keep original width (adjust spacing)").on_hover_text("Tightens or loosens character spacing so surrounding text keeps its place and nothing overlaps.");
                        ui.horizontal(|ui| {
                            if ui.button("Apply").clicked() {
                                apply = true;
                            }
                            if ui.button("Revert").clicked() {
                                revert = true;
                            }
                        });
                    });
                    if let Some((font, chars)) = d.missing.clone() {
                        ui.add_space(4.0);
                        egui::Frame::new().fill(self.pal.accent_soft).corner_radius(6).inner_margin(8).show(ui, |ui| {
                            ui.label(RichText::new("Characters not in this font").strong());
                            ui.label(format!("“{font}” has no glyph for: {chars}"));
                            ui.label(RichText::new("The font stays unchanged unless you choose to replace it for this text. Pick a font above, or use the default below.").size(11.0));
                            let which = d.font.map_or("DejaVu Sans".to_string(), |s| s.family.title().to_string());
                            if ui.button(format!("Use {which} for this text")).clicked() {
                                apply_sub = true;
                            }
                        });
                    }
                    if let Some((err, msg)) = &d.message {
                        ui.add_space(4.0);
                        ui.colored_label(
                            if *err {
                                self.pal.danger
                            } else {
                                self.pal.text_dim
                            },
                            msg.as_str(),
                        );
                    }
                    if let Some(r) = &d.report
                        && r.overlaps_following
                    {
                        ui.colored_label(self.pal.danger, "The new text overlaps the text after it. Enable “Keep original width” or shorten the text.");
                    }
                }
                ui.add_space(6.0);
                if ui
                    .add_enabled(
                        editable,
                        egui::Button::new(RichText::new("Delete this text").color(self.pal.danger)),
                    )
                    .clicked()
                {
                    del = true;
                }
                if revert && let Some(d) = self.tabs[ti].ui.edit_draft.as_mut() {
                    d.text = d.original.clone();
                    d.size = d.original_size;
                    d.font = None;
                    d.missing = None;
                    d.message = None;
                }
                if apply {
                    self.apply_text_draft(None);
                }
                if apply_sub {
                    let chosen = self.tabs[ti].ui.edit_draft.as_ref().and_then(|d| d.font);
                    let orig = self.tabs[ti]
                        .ui
                        .edit_draft
                        .as_ref()
                        .map(|d| d.original_font.to_lowercase())
                        .unwrap_or_default();
                    self.apply_text_draft(Some(chosen.unwrap_or_else(|| {
                        pdf_engine::fontembed::FontStyle::new(
                            pdf_engine::fontembed::FontFamily::DejaVuSans,
                            orig.contains("bold"),
                            false,
                        )
                    })));
                }
                if del {
                    self.delete_selected_content();
                }
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Only this text run is edited — there is no paragraph reflow.")
                        .size(11.0)
                        .color(self.pal.text_dim),
                );
            }
            ContentRef::Image(id) => {
                let Some(im) = objs.images.iter().find(|r| r.id == id).cloned() else {
                    return false;
                };
                ui.label(RichText::new("Image").strong());
                let b = im.quad.bounds();
                ui.label(
                    RichText::new(format!(
                        "{}×{} px, placed at {:.0}×{:.0} pt",
                        im.width_px,
                        im.height_px,
                        b.width(),
                        b.height()
                    ))
                    .size(12.0),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(can_edit, egui::Button::new("Replace image…"))
                        .clicked()
                    {
                        self.replace_selected_image();
                    }
                    if ui
                        .add_enabled(
                            can_edit,
                            egui::Button::new(RichText::new("Delete").color(self.pal.danger)),
                        )
                        .clicked()
                    {
                        self.delete_selected_content();
                    }
                });
                ui.label(
                    RichText::new("Drag to move; drag a handle to resize.")
                        .size(11.0)
                        .color(self.pal.text_dim),
                );
            }
        }
        let _ = ctx;
        true
    }
}

/// A readable font name without the subset tag ("ABCDEF+Arial-BoldMT" → "Arial-BoldMT").
fn short_font_name(n: &str) -> String {
    let n = match n.split_once('+') {
        Some((p, rest)) if p.len() == 6 && p.chars().all(|c| c.is_ascii_uppercase()) => rest,
        _ => n,
    };
    if n.is_empty() {
        "unnamed font".to_string()
    } else {
        n.to_string()
    }
}
