//! Pointer/keyboard interaction on the canvas: tools, selection, move/resize, drawing,
//! plus the overlay painting that visualises them.

use crate::canvas::ViewCtx;
use crate::state::*;
use editor_core::selection::TextSelection;
use editor_core::tools::Tool;
use egui::{Color32, CursorIcon, Pos2, Rect, Stroke, Vec2};
use pdf_engine::annot::{
    self, Align, AnnotationInfo, AnnotationKind, AnnotationSpec, LineEnding, Rgb,
};
use pdf_engine::doc::PageId;
use pdf_engine::geom::{Point, Quad, Rect as PRect};
use pdf_engine::text::TextPage;
use std::sync::Arc;

pub(crate) const HANDLE: f32 = 7.0;

fn rgb(c: [f32; 3]) -> Rgb {
    Rgb(c[0], c[1], c[2])
}

fn is_resizable(a: &AnnotationInfo) -> bool {
    matches!(
        a.subtype.as_str(),
        "Square" | "Circle" | "FreeText" | "Stamp"
    )
}

fn is_selectable(a: &AnnotationInfo) -> bool {
    !matches!(a.subtype.as_str(), "Link" | "Widget" | "Popup")
}

impl App {
    /// Annotations of a page for the current revision (cached).
    pub fn annots_for(&mut self, page: PageId) -> Arc<Vec<AnnotationInfo>> {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return Arc::new(Vec::new());
        };
        let rev = tab.session.revision();
        if let Some((r, a)) = tab.ui.annots.get(&page)
            && *r == rev
        {
            return a.clone();
        }
        let list = Arc::new(annot::read_annotations(tab.session.doc(), page));
        tab.ui.annots.insert(page, (rev, list.clone()));
        list
    }

    /// Text for a page if extracted (requests extraction otherwise).
    pub fn text_for(&mut self, vc: &ViewCtx, page_index: usize) -> Option<Arc<TextPage>> {
        let ti = self.active;
        let (doc, rev) = (self.tabs[ti].session.id, self.tabs[ti].session.revision());
        let page = &vc.pages[page_index];
        if let Some((r, t)) = self.text.get(&(doc, page.id))
            && *r == rev
        {
            return Some(t.clone());
        }
        if self.text_pending.insert((doc, page.id, rev))
            && let Ok(snap) = self.tabs[ti].session.snapshot()
        {
            self.hub.ensure(&snap);
            self.hub.submit(editor_core::jobs::Job {
                doc,
                revision: rev,
                request: 0,
                priority: 90,
                cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                kind: editor_core::jobs::JobKind::Text {
                    page: page.id,
                    page_index,
                    geometry: page.geometry,
                },
            });
        }
        None
    }

    pub(crate) fn quad_screen(vc: &ViewCtx, i: usize, q: &Quad) -> Vec<Pos2> {
        q.0.iter().map(|p| vc.pdf_to_screen(i, *p)).collect()
    }

    /// Screen-space bounding rect of a PDF rect on page `i`.
    pub(crate) fn rect_screen(vc: &ViewCtx, i: usize, r: PRect) -> Rect {
        let pts = [
            vc.pdf_to_screen(i, Point::new(r.x0, r.y0)),
            vc.pdf_to_screen(i, Point::new(r.x1, r.y0)),
            vc.pdf_to_screen(i, Point::new(r.x1, r.y1)),
            vc.pdf_to_screen(i, Point::new(r.x0, r.y1)),
        ];
        Rect::from_points(&pts)
    }

    pub(crate) fn screen_rect_to_pdf(vc: &ViewCtx, i: usize, r: Rect) -> PRect {
        let a = vc.screen_to_pdf(i, r.min);
        let b = vc.screen_to_pdf(i, r.max);
        PRect::new(a.x, a.y, b.x, b.y).abs()
    }

    pub(crate) fn handle_positions(r: Rect) -> [Pos2; 8] {
        let c = r.center();
        [
            r.left_top(),
            Pos2::new(c.x, r.min.y),
            r.right_top(),
            Pos2::new(r.max.x, c.y),
            r.right_bottom(),
            Pos2::new(c.x, r.max.y),
            r.left_bottom(),
            Pos2::new(r.min.x, c.y),
        ]
    }

    pub(crate) fn new_spec(&self, kind: AnnotationKind) -> AnnotationSpec {
        let d = &self.prefs.tool_defaults;
        let mut s = AnnotationSpec::new(kind);
        s.author = self.prefs.author.clone();
        s.opacity = d.opacity;
        s.border_width = d.stroke_width;
        match &s.kind {
            AnnotationKind::Highlight { .. } => s.color = rgb(d.highlight),
            AnnotationKind::Note { .. } => s.color = Rgb(1.0, 0.85, 0.2),
            _ => s.color = rgb(d.stroke),
        }
        s
    }

    pub(crate) fn add_annotation(&mut self, page: PageId, spec: AnnotationSpec, label: &str) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        match tab
            .session
            .execute(label, |tx| annot::add_annotation(tx, page, &spec))
        {
            Ok(id) => {
                tab.session.selection.annotations = vec![(page, id)];
                tab.session.selection.text = None;
            }
            Err(e) => {
                let msg = e.to_string();
                self.notify_error(msg);
            }
        }
    }

    /// Finish a free-text/note/stamp dialog.
    pub fn commit_text_entry(&mut self, page: PageId, tool: Tool, rect: PRect, text: String) {
        let d = self.prefs.tool_defaults.clone();
        let kind = match tool {
            Tool::Note => AnnotationKind::Note {
                pos: Point::new(rect.x0, rect.y1),
            },
            Tool::Stamp => AnnotationKind::StampText {
                rect,
                label: text.clone(),
            },
            Tool::Callout => {
                let tip = Point::new(rect.x0 - 40.0, rect.y0 - 30.0);
                let knee = Point::new(rect.x0, (rect.y0 + rect.y1) / 2.0);
                AnnotationKind::FreeText {
                    rect,
                    font_size: d.font_size,
                    text_color: Rgb::BLACK,
                    align: Align::Left,
                    bold: false,
                    callout: Some(vec![tip, knee]),
                }
            }
            _ => AnnotationKind::FreeText {
                rect,
                font_size: d.font_size,
                text_color: Rgb::BLACK,
                align: Align::Left,
                bold: false,
                callout: None,
            },
        };
        let mut spec = self.new_spec(kind);
        spec.contents = text;
        if matches!(tool, Tool::FreeText | Tool::Callout) {
            spec.fill = Some(Rgb(1.0, 1.0, 0.85));
            spec.border_width = 1.0;
            let contents = spec.contents.clone();
            if let AnnotationKind::FreeText {
                rect,
                font_size,
                bold,
                ..
            } = &mut spec.kind
                && let Ok(h) =
                    annot::freetext_required_height(&contents, *font_size, rect.width(), *bold)
                && h > rect.height()
            {
                // Grow the box so the text is not clipped.
                rect.y0 = rect.y1 - h;
            }
        }
        let label = match tool {
            Tool::Note => "Add note",
            Tool::Stamp => "Add stamp",
            Tool::Callout => "Add callout",
            _ => "Add text box",
        };
        self.add_annotation(page, spec, label);
    }

    fn finish_draw(
        &mut self,
        vc: &ViewCtx,
        page_index: usize,
        tool: Tool,
        pts: Vec<Point>,
        shift: bool,
    ) {
        let Some(page) = vc.pages.get(page_index).map(|p| p.id) else {
            return;
        };
        let two = |pts: &[Point]| -> Option<(Point, Point)> {
            if pts.len() >= 2 {
                Some((pts[0], *pts.last()?))
            } else {
                None
            }
        };
        let min_extent = 3.0;
        match tool {
            Tool::Rectangle | Tool::Ellipse | Tool::FreeText | Tool::Stamp => {
                let Some((a, mut b)) = two(&pts) else { return };
                if shift && matches!(tool, Tool::Rectangle | Tool::Ellipse) {
                    let d = ((b.x - a.x).abs()).max((b.y - a.y).abs());
                    b = Point::new(
                        a.x + d * (b.x - a.x).signum(),
                        a.y + d * (b.y - a.y).signum(),
                    );
                }
                let r = PRect::new(a.x, a.y, b.x, b.y).abs();
                if r.width() < min_extent && r.height() < min_extent {
                    return;
                }
                match tool {
                    Tool::Rectangle => self.add_annotation(
                        page,
                        self.new_spec(AnnotationKind::Rectangle { rect: r }),
                        "Add rectangle",
                    ),
                    Tool::Ellipse => self.add_annotation(
                        page,
                        self.new_spec(AnnotationKind::Ellipse { rect: r }),
                        "Add ellipse",
                    ),
                    Tool::FreeText => {
                        let r = if r.width() < 40.0 || r.height() < 16.0 {
                            PRect::new(r.x0, r.y1 - 40.0, r.x0 + 180.0, r.y1)
                        } else {
                            r
                        };
                        self.dialog = Some(Dialog::TextEntry {
                            page,
                            tool,
                            rect: r,
                            text: String::new(),
                        });
                    }
                    _ => {
                        let r = if r.width() < 40.0 || r.height() < 16.0 {
                            PRect::new(r.x0, r.y1 - 36.0, r.x0 + 140.0, r.y1)
                        } else {
                            r
                        };
                        self.dialog = Some(Dialog::TextEntry {
                            page,
                            tool,
                            rect: r,
                            text: "APPROVED".into(),
                        });
                    }
                }
            }
            Tool::Callout => {
                let Some((_, b)) = two(&pts) else { return };
                // The leader points back toward where the drag started; the box sits where it ended.
                let r = PRect::new(b.x, b.y - 24.0, b.x + 170.0, b.y + 24.0);
                self.dialog = Some(Dialog::TextEntry {
                    page,
                    tool,
                    rect: r,
                    text: String::new(),
                });
            }
            Tool::Line | Tool::Arrow => {
                let Some((a, mut b)) = two(&pts) else { return };
                if shift {
                    b = snap_angle(a, b);
                }
                if (a.x - b.x).abs() < min_extent && (a.y - b.y).abs() < min_extent {
                    return;
                }
                let kind = AnnotationKind::Line {
                    start: a,
                    end: b,
                    start_ending: LineEnding::None,
                    end_ending: if tool == Tool::Arrow {
                        LineEnding::ClosedArrow
                    } else {
                        LineEnding::None
                    },
                };
                self.add_annotation(
                    page,
                    self.new_spec(kind),
                    if tool == Tool::Arrow {
                        "Add arrow"
                    } else {
                        "Add line"
                    },
                );
            }
            Tool::Ink => {
                if pts.len() >= 2 {
                    self.add_annotation(
                        page,
                        self.new_spec(AnnotationKind::Ink { strokes: vec![pts] }),
                        "Add drawing",
                    );
                }
            }
            Tool::Polygon if pts.len() >= 3 => {
                self.add_annotation(
                    page,
                    self.new_spec(AnnotationKind::Polygon { points: pts }),
                    "Add polygon",
                );
            }
            Tool::Polyline if pts.len() >= 2 => {
                let kind = AnnotationKind::PolyLine {
                    points: pts,
                    start_ending: LineEnding::None,
                    end_ending: LineEnding::None,
                };
                self.add_annotation(page, self.new_spec(kind), "Add polyline");
            }
            _ => {}
        }
    }

    fn apply_markup(
        &mut self,
        page: PageId,
        tp: &TextPage,
        range: std::ops::Range<usize>,
        tool: Tool,
    ) {
        let quads = tp.selection_quads(range);
        if quads.is_empty() {
            return;
        }
        let (kind, label) = match tool {
            Tool::Highlight => (AnnotationKind::Highlight { quads }, "Highlight text"),
            Tool::Underline => (AnnotationKind::Underline { quads }, "Underline text"),
            _ => (AnnotationKind::StrikeOut { quads }, "Strike out text"),
        };
        let spec = self.new_spec(kind);
        self.add_annotation(page, spec, label);
    }

    /// Apply a markup tool to the current text selection (ribbon buttons).
    pub fn markup_current_selection(&mut self, tool: Tool) {
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
        self.apply_markup(sel.page, &tp, sel.glyphs, tool);
        if let Some(t) = self.tabs.get_mut(ti) {
            t.session.selection.text = None;
        }
    }

    pub fn canvas_interaction(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        vc: &mut ViewCtx,
    ) {
        if self.dialog.is_some() || self.palette_open {
            return;
        }
        let tool = self.tool;
        let (mods, hover) = ctx.input(|i| (i.modifiers, i.pointer.hover_pos()));
        let pos = response.interact_pointer_pos().or(hover);

        // Middle-button drag pans in every tool.
        if response.dragged_by(egui::PointerButton::Middle) {
            self.pan(vc, response.drag_delta());
            return;
        }
        if tool == Tool::Hand {
            if response.dragged_by(egui::PointerButton::Primary) {
                self.pan(vc, response.drag_delta());
            }
            self.form_click(response, vc, pos);
            self.follow_links(response, vc);
            return;
        }

        // Make sure text is (being) extracted for the page under the cursor.
        if matches!(
            tool,
            Tool::TextSelect | Tool::Highlight | Tool::Underline | Tool::StrikeOut | Tool::Select
        ) && let Some(i) = pos.and_then(|p| vc.page_at(p).or_else(|| vc.nearest_page(p)))
        {
            let _ = self.text_for(vc, i);
        }

        match tool {
            Tool::TextSelect | Tool::Highlight | Tool::Underline | Tool::StrikeOut => {
                self.text_tool(response, vc, pos, tool)
            }
            Tool::Select => self.select_tool(response, vc, pos, mods),
            Tool::Note => {
                if response.clicked_by(egui::PointerButton::Primary)
                    && let Some(p) = pos
                    && let Some(i) = vc.page_at(p)
                {
                    let pt = vc.screen_to_pdf(i, p);
                    let page = vc.pages[i].id;
                    self.dialog = Some(Dialog::TextEntry {
                        page,
                        tool: Tool::Note,
                        rect: PRect::new(pt.x, pt.y, pt.x, pt.y),
                        text: String::new(),
                    });
                }
            }
            Tool::Polygon | Tool::Polyline => self.poly_tool(response, vc, pos, tool, mods),
            Tool::Rectangle
            | Tool::Ellipse
            | Tool::Line
            | Tool::Arrow
            | Tool::Ink
            | Tool::FreeText
            | Tool::Callout
            | Tool::Stamp => {
                self.drag_draw_tool(response, vc, pos, tool, mods);
            }
            Tool::EditText | Tool::AddText | Tool::AddImage => {
                self.content_tool(response, vc, pos, tool)
            }
            Tool::FillForm => self.form_click(response, vc, pos),
            Tool::PlaceSignature => self.place_signature_tool(response, vc, pos),
            Tool::SignArea => self.sign_area_tool(response, vc, pos),
            Tool::MeasureDistance
            | Tool::MeasurePerimeter
            | Tool::MeasureArea
            | Tool::MeasureRect
            | Tool::MeasureRadius
            | Tool::MeasureAngle
            | Tool::Count => self.measure_tool(response, vc, pos, tool, mods),
            Tool::Calibrate => self.calibrate_tool(response, vc, pos, mods),
            Tool::Hand => {}
        }
        if matches!(tool, Tool::MeasurePerimeter | Tool::MeasureArea)
            && ctx.input(|i| i.key_pressed(egui::Key::Enter))
        {
            self.finish_measure_poly(vc, tool);
        }
        // Enter finishes polygon/polyline.
        if matches!(tool, Tool::Polygon | Tool::Polyline)
            && ctx.input(|i| i.key_pressed(egui::Key::Enter))
        {
            self.finish_poly(vc, tool, mods.shift);
        }
        if matches!(tool, Tool::Hand | Tool::Select | Tool::TextSelect) {
            self.form_click(response, vc, pos);
        }
        self.follow_links(response, vc);
    }

    fn pan(&mut self, vc: &mut ViewCtx, d: Vec2) {
        let tab = &mut self.tabs[self.active];
        tab.session.view.scroll -= crate::canvas::to_core(d);
        vc.scroll = crate::canvas::ScrollExt::egui(&tab.session.view.scroll);
    }

    fn text_tool(
        &mut self,
        response: &egui::Response,
        vc: &ViewCtx,
        pos: Option<Pos2>,
        tool: Tool,
    ) {
        let ti = self.active;
        let doc = self.tabs[ti].session.id;
        let rev = self.tabs[ti].session.revision();
        let Some(pos) = pos else { return };
        let Some(i) = vc.page_at(pos).or_else(|| vc.nearest_page(pos)) else {
            return;
        };
        let page = vc.pages[i].id;
        let Some((r, tp)) = self.text.get(&(doc, page)).cloned() else {
            return;
        };
        if r != rev {
            return;
        }
        let pt = vc.screen_to_pdf(i, pos);
        if response.drag_started_by(egui::PointerButton::Primary) {
            if let Some(a) = tp.hit_test(pt, 10.0) {
                self.tabs[ti].ui.interaction = Interaction::TextSelect { page, anchor: a };
                self.tabs[ti].session.selection.annotations.clear();
                self.tabs[ti].session.selection.text = Some(TextSelection {
                    page,
                    glyphs: a..a + 1,
                });
            } else {
                self.tabs[ti].session.selection.text = None;
            }
        } else if response.dragged_by(egui::PointerButton::Primary) {
            // Extend within the anchor page (cross-page selection is not supported).
            if let Interaction::TextSelect { page: ap, anchor } =
                self.tabs[ti].ui.interaction.clone()
                && ap == page
                && let Some(c) = tp.hit_test(pt, 400.0)
            {
                let (s, e) = if c >= anchor {
                    (anchor, c + 1)
                } else {
                    (c, anchor + 1)
                };
                self.tabs[ti].session.selection.text = Some(TextSelection {
                    page: ap,
                    glyphs: s..e,
                });
            }
        } else if response.drag_stopped() {
            if let Interaction::TextSelect { page: ap, .. } = self.tabs[ti].ui.interaction.clone() {
                self.tabs[ti].ui.interaction = Interaction::None;
                if matches!(tool, Tool::Highlight | Tool::Underline | Tool::StrikeOut)
                    && let Some(sel) = self.tabs[ti].session.selection.text.clone()
                    && sel.page == ap
                {
                    self.apply_markup(ap, &tp, sel.glyphs, tool);
                    self.tabs[ti].session.selection.text = None;
                }
            }
        } else if response.double_clicked()
            && tool == Tool::TextSelect
            && let Some(g) = tp.hit_test(pt, 8.0)
        {
            let w = tp.word_range(g);
            if !w.is_empty() {
                self.tabs[ti].session.selection.text = Some(TextSelection { page, glyphs: w });
            }
        } else if response.clicked() {
            self.tabs[ti].session.selection.text = None;
        }
    }

    fn select_tool(
        &mut self,
        response: &egui::Response,
        vc: &ViewCtx,
        pos: Option<Pos2>,
        mods: egui::Modifiers,
    ) {
        let ti = self.active;
        let Some(pos) = pos else { return };
        if response.drag_started_by(egui::PointerButton::Primary) {
            let Some(i) = vc.page_at(pos) else { return };
            let page = vc.pages[i].id;
            let annots = self.annots_for(page);
            // Resize handle of the single selected annotation?
            let sel = self.tabs[ti].session.selection.annotations.clone();
            if sel.len() == 1
                && sel[0].0 == page
                && let Some(a) = annots.iter().find(|a| a.id == sel[0].1)
                && is_resizable(a)
            {
                let r = Self::rect_screen(vc, i, a.rect).expand(2.0);
                for (h, hp) in Self::handle_positions(r).iter().enumerate() {
                    if hp.distance(pos) <= HANDLE {
                        self.tabs[ti].ui.interaction = Interaction::Resize {
                            id: a.id,
                            page,
                            handle: h,
                            rect: a.rect,
                        };
                        return;
                    }
                }
            }
            let pt = vc.screen_to_pdf(i, pos);
            let tol = 4.0 / vc.px_per_pt;
            let hit = annots
                .iter()
                .rev()
                .filter(|a| is_selectable(a))
                .find(|a| a.rect.inflate(tol, tol).contains(Point::new(pt.x, pt.y)));
            match hit {
                Some(a) => {
                    let already = self.tabs[ti].session.selection.has_annotation(a.id);
                    if !already || mods.shift {
                        self.tabs[ti]
                            .session
                            .selection
                            .select_annotation(page, a.id, mods.shift);
                    }
                    if self.tabs[ti].session.selection.has_annotation(a.id)
                        && self.tabs[ti].session.doc().capabilities().can_edit
                    {
                        self.tabs[ti].ui.interaction = Interaction::Move { delta: (0.0, 0.0) };
                    }
                }
                None => {
                    if !mods.shift {
                        self.tabs[ti].session.selection.clear_content();
                    }
                    self.tabs[ti].ui.interaction = Interaction::Panning;
                }
            }
        } else if response.dragged_by(egui::PointerButton::Primary) {
            let inter = self.tabs[ti].ui.interaction.clone();
            match inter {
                Interaction::Move { delta } => {
                    let sel = self.tabs[ti]
                        .session
                        .selection
                        .annotations
                        .first()
                        .map(|(p, _)| *p);
                    if let Some(page) = sel
                        && let Some(i) = vc.pages.iter().position(|p| p.id == page)
                    {
                        let (dx, dy) = vc.delta_to_pdf(i, response.drag_delta());
                        self.tabs[ti].ui.interaction = Interaction::Move {
                            delta: (delta.0 + dx, delta.1 + dy),
                        };
                    }
                }
                Interaction::Resize {
                    id,
                    page,
                    handle,
                    rect,
                } => {
                    if let Some(i) = vc.pages.iter().position(|p| p.id == page) {
                        let mut r = Self::rect_screen(vc, i, rect);
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
                        let nr = Self::screen_rect_to_pdf(vc, i, Rect::from_two_pos(r.min, r.max));
                        self.tabs[ti].ui.interaction = Interaction::Resize {
                            id,
                            page,
                            handle,
                            rect: nr,
                        };
                    }
                }
                Interaction::Panning => {
                    self.tabs[ti].session.view.scroll -=
                        crate::canvas::to_core(response.drag_delta());
                }
                _ => {}
            }
        } else if response.drag_stopped() {
            let inter = std::mem::take(&mut self.tabs[ti].ui.interaction);
            match inter {
                Interaction::Move { delta } if delta.0.abs() + delta.1.abs() > 0.01 => {
                    let sel = self.tabs[ti].session.selection.annotations.clone();
                    let label = if sel.len() > 1 {
                        "Move annotations"
                    } else {
                        "Move annotation"
                    };
                    let r = self.tabs[ti].session.execute(label, |tx| {
                        for (_, id) in &sel {
                            annot::move_annotation(tx, *id, delta.0, delta.1)?;
                        }
                        Ok(())
                    });
                    if let Err(e) = r {
                        self.notify_error(e.to_string());
                    }
                }
                Interaction::Resize { id, rect, .. } => {
                    let info = annot::read_annotation(self.tabs[ti].session.doc().lopdf(), id);
                    if let Some(info) = info
                        && let Some(mut spec) = info.spec
                    {
                        match &mut spec.kind {
                            AnnotationKind::Rectangle { rect: r }
                            | AnnotationKind::Ellipse { rect: r }
                            | AnnotationKind::StampText { rect: r, .. }
                            | AnnotationKind::FreeText { rect: r, .. } => *r = rect,
                            _ => {}
                        }
                        let res = self.tabs[ti].session.execute("Resize annotation", |tx| {
                            annot::update_annotation(tx, id, &spec)
                        });
                        if let Err(e) = res {
                            self.notify_error(e.to_string());
                        }
                    }
                }
                _ => {}
            }
        } else if response.double_clicked() {
            // Jump to the comment editor for the selected annotation.
            self.right_tab = RightTab::Properties;
            self.prefs.show_right_sidebar = true;
        }
    }

    fn drag_draw_tool(
        &mut self,
        response: &egui::Response,
        vc: &ViewCtx,
        pos: Option<Pos2>,
        tool: Tool,
        mods: egui::Modifiers,
    ) {
        let ti = self.active;
        let Some(pos) = pos else { return };
        if response.drag_started_by(egui::PointerButton::Primary) {
            let Some(i) = vc.page_at(pos) else { return };
            let p = vc.screen_to_pdf(i, pos);
            self.tabs[ti].session.selection.clear_content();
            self.tabs[ti].ui.interaction = Interaction::Draw {
                page: vc.pages[i].id,
                tool,
                points: vec![p, p],
            };
        } else if response.dragged_by(egui::PointerButton::Primary)
            && let Interaction::Draw {
                page,
                tool: t,
                mut points,
            } = self.tabs[ti].ui.interaction.clone()
            && let Some(i) = vc.pages.iter().position(|p| p.id == page)
        {
            let mut p = vc.screen_to_pdf(i, pos);
            if t == Tool::Ink {
                if points
                    .last()
                    .is_none_or(|l| (l.x - p.x).hypot(l.y - p.y) > 1.0)
                {
                    points.push(p);
                }
            } else {
                if mods.shift && matches!(t, Tool::Line | Tool::Arrow) {
                    p = snap_angle(points[0], p);
                }
                points.truncate(1);
                points.push(p);
            }
            self.tabs[ti].ui.interaction = Interaction::Draw {
                page,
                tool: t,
                points,
            };
        } else if response.drag_stopped()
            && let Interaction::Draw {
                page,
                tool: t,
                points,
            } = std::mem::take(&mut self.tabs[ti].ui.interaction)
            && let Some(i) = vc.pages.iter().position(|p| p.id == page)
        {
            self.finish_draw(vc, i, t, points, mods.shift);
        }
    }

    fn poly_tool(
        &mut self,
        response: &egui::Response,
        vc: &ViewCtx,
        pos: Option<Pos2>,
        tool: Tool,
        mods: egui::Modifiers,
    ) {
        let ti = self.active;
        let Some(pos) = pos else { return };
        if response.double_clicked() {
            self.finish_poly(vc, tool, mods.shift);
        } else if response.clicked_by(egui::PointerButton::Primary) {
            let page_known = if let Interaction::Draw { page, .. } = &self.tabs[ti].ui.interaction {
                vc.pages.iter().position(|p| p.id == *page)
            } else {
                vc.page_at(pos)
            };
            if let Some(i) = page_known {
                let mut p = vc.screen_to_pdf(i, pos);
                if mods.shift
                    && let Some(last) = self.tabs[ti].ui.polygon_points.last()
                {
                    p = snap_angle(*last, p);
                }
                let page = vc.pages[i].id;
                self.tabs[ti].ui.polygon_points.push(p);
                self.tabs[ti].ui.interaction = Interaction::Draw {
                    page,
                    tool,
                    points: self.tabs[ti].ui.polygon_points.clone(),
                };
            }
        }
    }

    fn finish_poly(&mut self, vc: &ViewCtx, tool: Tool, shift: bool) {
        let ti = self.active;
        let mut pts = std::mem::take(&mut self.tabs[ti].ui.polygon_points);
        // A double-click adds the same point twice.
        if pts.len() >= 2 && pts[pts.len() - 1] == pts[pts.len() - 2] {
            pts.pop();
        }
        let inter = std::mem::take(&mut self.tabs[ti].ui.interaction);
        if let Interaction::Draw { page, .. } = inter
            && let Some(i) = vc.pages.iter().position(|p| p.id == page)
        {
            self.finish_draw(vc, i, tool, pts, shift);
        }
    }

    fn follow_links(&mut self, response: &egui::Response, vc: &ViewCtx) {
        if !response.clicked_by(egui::PointerButton::Primary)
            || !matches!(self.tool, Tool::Hand | Tool::Select | Tool::TextSelect)
        {
            return;
        }
        let Some(pos) = response.interact_pointer_pos() else {
            return;
        };
        let Some(i) = vc.page_at(pos) else { return };
        let pt = vc.screen_to_pdf(i, pos);
        let page = vc.pages[i].id;
        let links = pdf_engine::nav::page_links(self.tabs[self.active].session.doc(), page);
        if let Some(l) = links
            .into_iter()
            .find(|l| l.rect.contains(Point::new(pt.x, pt.y)))
        {
            match l.target {
                pdf_engine::nav::LinkTarget::Page(n) => self.go_to(n),
                // External navigation always needs explicit confirmation.
                pdf_engine::nav::LinkTarget::Uri(u) => {
                    self.dialog = Some(Dialog::ConfirmLink { uri: u })
                }
            }
        }
    }

    pub fn canvas_cursor(&self, ctx: &egui::Context, response: &egui::Response) {
        if !response.hovered() {
            return;
        }
        let icon = match self.tool {
            Tool::Hand => {
                if response.dragged() {
                    CursorIcon::Grabbing
                } else {
                    CursorIcon::Grab
                }
            }
            Tool::TextSelect | Tool::Highlight | Tool::Underline | Tool::StrikeOut => {
                CursorIcon::Text
            }
            Tool::Select => match self.tabs.get(self.active).map(|t| &t.ui.interaction) {
                Some(Interaction::Move { .. }) => CursorIcon::Move,
                Some(Interaction::Resize { .. }) => CursorIcon::ResizeNwSe,
                _ => CursorIcon::Default,
            },
            Tool::EditText | Tool::FillForm => CursorIcon::PointingHand,
            Tool::PlaceSignature => CursorIcon::Crosshair,
            Tool::AddText => CursorIcon::Text,
            Tool::AddImage => CursorIcon::Crosshair,
            _ => CursorIcon::Crosshair,
        };
        ctx.set_cursor_icon(icon);
    }

    pub fn paint_overlays(&mut self, painter: &egui::Painter, vc: &ViewCtx) {
        self.paint_content_overlays(painter, vc);
        self.paint_form_overlays(painter, vc);
        self.paint_measure_overlays(painter, vc);
        let ti = self.active;
        let doc = self.tabs[ti].session.id;
        let band = (
            f64::from(vc.scroll.y - 50.0),
            f64::from(vc.scroll.y + vc.viewport.height() + 50.0),
        );
        let pages = vc.layout.pages_in_band(band.0, band.1);
        let accent = self.pal.accent;
        let search_cur = self.tabs[ti].session.search.current;
        let matches = self.tabs[ti].session.search.matches.clone();
        let text_sel = self.tabs[ti].session.selection.text.clone();
        let sel_annots = self.tabs[ti].session.selection.annotations.clone();
        let inter = self.tabs[ti].ui.interaction.clone();
        for i in pages {
            let page = vc.pages[i].id;
            // Search highlights.
            for (n, m) in matches.iter().enumerate().filter(|(_, m)| m.page == page) {
                let cur = Some(n) == search_cur;
                let fill = if cur {
                    Color32::from_rgba_unmultiplied(255, 140, 0, 120)
                } else {
                    Color32::from_rgba_unmultiplied(255, 220, 0, 90)
                };
                for q in &m.hit.quads {
                    painter.add(egui::epaint::PathShape::convex_polygon(
                        Self::quad_screen(vc, i, q),
                        fill,
                        Stroke::NONE,
                    ));
                }
            }
            // Text selection.
            if let Some(sel) = &text_sel
                && sel.page == page
                && let Some((_, tp)) = self.text.get(&(doc, page))
            {
                for q in tp.selection_quads(sel.glyphs.clone()) {
                    painter.add(egui::epaint::PathShape::convex_polygon(
                        Self::quad_screen(vc, i, &q),
                        Color32::from_rgba_unmultiplied(70, 130, 230, 90),
                        Stroke::NONE,
                    ));
                }
            }
            // Annotation selection.
            if sel_annots.iter().any(|(p, _)| *p == page) {
                let annots = self.annots_for(page);
                for (p, id) in &sel_annots {
                    if *p != page {
                        continue;
                    }
                    let Some(a) = annots.iter().find(|a| a.id == *id) else {
                        continue;
                    };
                    let mut rect = a.rect;
                    match &inter {
                        Interaction::Move { delta } => {
                            rect = PRect::new(
                                rect.x0 + delta.0,
                                rect.y0 + delta.1,
                                rect.x1 + delta.0,
                                rect.y1 + delta.1,
                            )
                        }
                        Interaction::Resize {
                            id: rid, rect: nr, ..
                        } if rid == id => rect = *nr,
                        _ => {}
                    }
                    let r = Self::rect_screen(vc, i, rect).expand(2.0);
                    dashed_rect(painter, r, Stroke::new(1.5, accent));
                    if sel_annots.len() == 1
                        && is_resizable(a)
                        && !matches!(inter, Interaction::Move { .. })
                    {
                        for hp in Self::handle_positions(r) {
                            let hr = Rect::from_center_size(hp, Vec2::splat(HANDLE));
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
            // Drawing preview (a preview of a command — nothing is in the document yet).
            if let Interaction::Draw {
                page: dp,
                tool,
                points,
            } = &inter
                && *dp == page
            {
                let st = Stroke::new(1.5, accent);
                let sp: Vec<Pos2> = points.iter().map(|p| vc.pdf_to_screen(i, *p)).collect();
                match tool {
                    Tool::Rectangle
                    | Tool::Ellipse
                    | Tool::FreeText
                    | Tool::Stamp
                    | Tool::SignArea
                        if sp.len() >= 2 =>
                    {
                        let r = Rect::from_two_pos(sp[0], sp[sp.len() - 1]);
                        if *tool == Tool::Ellipse {
                            painter.add(egui::epaint::EllipseShape::stroke(
                                r.center(),
                                r.size() / 2.0,
                                st,
                            ));
                        } else {
                            dashed_rect(painter, r, st);
                        }
                    }
                    Tool::Line | Tool::Arrow | Tool::Callout if sp.len() >= 2 => {
                        painter.line_segment([sp[0], sp[sp.len() - 1]], st);
                    }
                    Tool::Ink | Tool::Polyline | Tool::Polygon => {
                        if sp.len() >= 2 {
                            painter.add(egui::epaint::PathShape::line(sp.clone(), st));
                        }
                        if matches!(tool, Tool::Polygon | Tool::Polyline) {
                            for p in &sp {
                                painter.circle_filled(*p, 3.0, accent);
                            }
                            if let Some(h) = painter.ctx().input(|inp| inp.pointer.hover_pos())
                                && let Some(last) = sp.last()
                            {
                                painter.line_segment(
                                    [*last, h],
                                    Stroke::new(1.0, accent.gamma_multiply(0.6)),
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn dashed_rect(painter: &egui::Painter, r: Rect, stroke: Stroke) {
    let pts = [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
        r.left_top(),
    ];
    painter.add(egui::Shape::dashed_line(&pts, stroke, 5.0, 3.0));
}

/// Constrain the segment `a→b` to the nearest 15° increment.
pub fn snap_angle(a: Point, b: Point) -> Point {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = dx.hypot(dy);
    if len < 1e-9 {
        return b;
    }
    let step = std::f64::consts::PI / 12.0;
    let ang = (dy.atan2(dx) / step).round() * step;
    Point::new(a.x + len * ang.cos(), a.y + len * ang.sin())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angle_snapping_is_in_15_degree_steps() {
        let a = Point::new(0.0, 0.0);
        let p = snap_angle(a, Point::new(10.0, 1.0));
        assert!(p.y.abs() < 1e-9 && (p.x - (101.0f64).sqrt()).abs() < 1e-9);
        let q = snap_angle(a, Point::new(10.0, 9.0));
        let ang = q.y.atan2(q.x).to_degrees();
        assert!((ang - 45.0).abs() < 1e-6, "{ang}");
        assert_eq!(snap_angle(a, a), a);
    }
}
