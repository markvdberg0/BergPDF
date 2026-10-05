//! The document canvas: layout, virtualized page painting, tiles, zoom/scroll.

use crate::state::*;
use editor_core::jobs::{Job, JobKind};
use editor_core::tiles::{TileKey, TilePlan, plan_tiles, quantize_scale};
use editor_core::view::{self, Layout, LayoutMetrics, PX_PER_PT_AT_100, ViewMode, ZoomMode};
use egui::{Color32, Pos2, Rect, Sense, Vec2};
use pdf_engine::doc::PageInfo;
use pdf_engine::geom::{Affine, Point, Rotation, Size};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// Everything needed to map between PDF space and the screen for one frame.
#[derive(Clone)]
pub struct ViewCtx {
    pub viewport: Rect,
    pub scroll: Vec2,
    /// Logical (egui) px per PDF point.
    pub px_per_pt: f64,
    pub ppp: f32,
    pub rotation: Rotation,
    pub layout: Layout,
    pub pages: Arc<Vec<PageInfo>>,
}

impl ViewCtx {
    pub fn slot(&self, i: usize) -> Rect {
        let r = self.layout.slots[i];
        Rect::from_min_max(
            self.viewport.min + Vec2::new(r.x0 as f32, r.y0 as f32) - self.scroll,
            self.viewport.min + Vec2::new(r.x1 as f32, r.y1 as f32) - self.scroll,
        )
    }

    fn to_view(&self, i: usize) -> Affine {
        self.pages[i].geometry.pdf_to_view(self.rotation)
    }

    pub fn pdf_to_screen(&self, i: usize, p: Point) -> Pos2 {
        let v = self.to_view(i) * p;
        let s = self.slot(i).min;
        Pos2::new(
            s.x + (v.x * self.px_per_pt) as f32,
            s.y + (v.y * self.px_per_pt) as f32,
        )
    }

    pub fn screen_to_pdf(&self, i: usize, pos: Pos2) -> Point {
        let s = self.slot(i).min;
        let v = Point::new(
            f64::from(pos.x - s.x) / self.px_per_pt,
            f64::from(pos.y - s.y) / self.px_per_pt,
        );
        self.to_view(i).inverse() * v
    }

    /// Convert a screen-space delta into a PDF-space delta on page `i`.
    pub fn delta_to_pdf(&self, i: usize, d: Vec2) -> (f64, f64) {
        let a = self.screen_to_pdf(i, Pos2::new(100.0, 100.0));
        let b = self.screen_to_pdf(i, Pos2::new(100.0 + d.x, 100.0 + d.y));
        (b.x - a.x, b.y - a.y)
    }

    pub fn page_at(&self, pos: Pos2) -> Option<usize> {
        let c = pos - self.viewport.min + self.scroll;
        self.layout
            .page_at(Point::new(f64::from(c.x), f64::from(c.y)))
    }

    /// Page nearest to a screen position (clamped), for drags that leave the page.
    pub fn nearest_page(&self, pos: Pos2) -> Option<usize> {
        let c = pos - self.viewport.min + self.scroll;
        self.layout
            .locate(Point::new(f64::from(c.x), f64::from(c.y)))
            .map(|(i, _, _)| i)
    }
}

const PREFETCH_PX: f32 = 400.0;

/// Conversions between the core's f64 scroll vector and egui's f32 one.
pub trait ScrollExt {
    fn egui(&self) -> Vec2;
}

impl ScrollExt for pdf_engine::geom::Vec2 {
    fn egui(&self) -> Vec2 {
        Vec2::new(self.x as f32, self.y as f32)
    }
}

pub fn to_core(v: Vec2) -> pdf_engine::geom::Vec2 {
    pdf_engine::geom::Vec2::new(f64::from(v.x), f64::from(v.y))
}

impl App {
    /// Build the per-frame view context for the active tab and apply zoom/scroll requests.
    fn prepare_view(&mut self, ui_rect: Rect, ppp: f32) -> Option<ViewCtx> {
        let ui_scale = self.prefs.ui_scale as f64;
        let ti = self.active;
        let tab = self.tabs.get_mut(ti)?;
        let pages = tab.session.pages().ok()?;
        if pages.is_empty() {
            return None;
        }
        let rot = tab.session.view.rotation;
        let sizes: Vec<Size> = pages.iter().map(|p| p.geometry.view_size(rot)).collect();
        let m = LayoutMetrics::default();
        let vp = Size::new(f64::from(ui_rect.width()), f64::from(ui_rect.height()));
        let v = &mut tab.session.view;
        v.current_page = v.current_page.min(pages.len() - 1);
        let cur_size = sizes[v.current_page];
        // Fit modes derive the zoom.
        match v.zoom_mode {
            ZoomMode::FitWidth => {
                v.zoom = (view::fit_width_zoom(cur_size, vp.width, m.margin) * ui_scale)
                    .clamp(view::MIN_ZOOM, view::MAX_ZOOM)
            }
            ZoomMode::FitPage => {
                v.zoom = (view::fit_page_zoom(cur_size, vp, m.margin) * ui_scale)
                    .clamp(view::MIN_ZOOM, view::MAX_ZOOM)
            }
            ZoomMode::Custom => {}
        }
        // `fit_*` return a zoom relative to the 96-dpi scale with *logical* px; convert for UI scale.
        let scale_of = |z: f64| z * PX_PER_PT_AT_100 / ui_scale;
        let mk = |v: &editor_core::view::ViewState, z: f64| {
            Layout::compute(
                &sizes,
                v.mode,
                v.cover_page,
                v.current_page,
                scale_of(z),
                vp.width,
                m,
            )
        };
        let mut layout = mk(v, v.zoom);
        // Explicit zoom request (buttons, ctrl+wheel) keeps the anchor content fixed.
        if let Some((z, anchor)) = tab.ui.zoom_request.take() {
            let anchor = anchor.unwrap_or(Vec2::new(vp.width as f32 / 2.0, vp.height as f32 / 2.0));
            let old_scroll = v.scroll.egui();
            let new_layout = mk(v, z);
            let ns = view::scroll_after_zoom(
                &layout,
                &new_layout,
                pdf_engine::geom::Vec2::new(f64::from(old_scroll.x), f64::from(old_scroll.y)),
                pdf_engine::geom::Vec2::new(f64::from(anchor.x), f64::from(anchor.y)),
            );
            v.zoom = z;
            v.scroll = ns;
            layout = new_layout;
            tab.ui.last_zoom_change = Some(std::time::Instant::now());
        }
        // Go-to-page.
        if let Some(p) = tab.ui.goto.take() {
            let p = p.min(pages.len() - 1);
            v.current_page = p;
            if v.mode == ViewMode::SinglePage {
                layout = mk(v, v.zoom);
                v.scroll = pdf_engine::geom::Vec2::ZERO;
            } else {
                let r = layout.slots[p];
                v.scroll.y = r.y0 - m.margin;
                if r.x0 < v.scroll.x {
                    v.scroll.x = 0.0;
                }
            }
            tab.ui.thumb_scroll_to_current = true;
        }
        let content = layout.content;
        let ns = view::clamp_scroll(v.scroll, content, vp);
        v.scroll = ns;
        Some(ViewCtx {
            viewport: ui_rect,
            scroll: v.scroll.egui(),
            px_per_pt: scale_of(v.zoom),
            ppp,
            rotation: rot,
            layout,
            pages,
        })
    }

    pub fn canvas(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let rect = ui.available_rect_before_wrap();
        let ppp = ctx.pixels_per_point();
        let response = ui.interact(rect, ui.id().with("canvas"), Sense::click_and_drag());
        let Some(mut vc) = self.prepare_view(rect, ppp) else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "This document has no pages",
                egui::FontId::proportional(16.0),
                self.pal.text_dim,
            );
            return;
        };
        let ti = self.active;

        // ---- zoom / scroll input ----
        let hovered = response.hovered() && self.dialog.is_none() && !self.palette_open;
        if hovered {
            let (zd, scroll, shift, pos) = ctx.input(|i| {
                (
                    i.zoom_delta(),
                    i.smooth_scroll_delta,
                    i.modifiers.shift,
                    i.pointer.hover_pos(),
                )
            });
            if (zd - 1.0).abs() > 1e-4 {
                let anchor = pos.map(|p| p - rect.min);
                let z = self.tabs[ti].session.view.zoom * f64::from(zd);
                self.request_zoom(z, anchor);
            } else if scroll != Vec2::ZERO {
                let mut d = scroll;
                if shift && d.x == 0.0 {
                    d = Vec2::new(d.y, 0.0);
                }
                let tab = &mut self.tabs[ti];
                let content = vc.layout.content;
                let ns = view::clamp_scroll(
                    pdf_engine::geom::Vec2::new(
                        tab.session.view.scroll.x - f64::from(d.x),
                        tab.session.view.scroll.y - f64::from(d.y),
                    ),
                    content,
                    Size::new(f64::from(rect.width()), f64::from(rect.height())),
                );
                tab.session.view.scroll = ns;
                vc.scroll = ns.egui();
            }
        }
        // Keyboard scrolling when no text field has focus.
        if !ctx.egui_wants_keyboard_input() && self.dialog.is_none() && !self.palette_open {
            let (dx, dy) = ctx.input(|i| {
                let mut dx = 0.0;
                let mut dy = 0.0;
                let step = 48.0;
                if i.key_pressed(egui::Key::ArrowDown) {
                    dy += step;
                }
                if i.key_pressed(egui::Key::ArrowUp) {
                    dy -= step;
                }
                if i.key_pressed(egui::Key::ArrowRight) {
                    dx += step;
                }
                if i.key_pressed(egui::Key::ArrowLeft) {
                    dx -= step;
                }
                if i.key_pressed(egui::Key::Space) {
                    dy += if i.modifiers.shift {
                        -rect.height() * 0.9
                    } else {
                        rect.height() * 0.9
                    };
                }
                (dx, dy)
            });
            if dx != 0.0 || dy != 0.0 {
                let tab = &mut self.tabs[ti];
                let ns = view::clamp_scroll(
                    pdf_engine::geom::Vec2::new(
                        tab.session.view.scroll.x + f64::from(dx),
                        tab.session.view.scroll.y + f64::from(dy),
                    ),
                    vc.layout.content,
                    Size::new(f64::from(rect.width()), f64::from(rect.height())),
                );
                tab.session.view.scroll = ns;
                vc.scroll = ns.egui();
            }
        }

        // Tool interaction (may consume the drag for panning, drawing, selecting…).
        self.canvas_interaction(ctx, &response, &mut vc);
        self.canvas_context_menu(ctx, &response, &vc);

        // ---- current page from scroll ----
        {
            let tab = &mut self.tabs[ti];
            if tab.session.view.mode != ViewMode::SinglePage
                && let Some(p) = vc
                    .layout
                    .current_page_for(f64::from(vc.scroll.y), f64::from(rect.height()))
                && p != tab.session.view.current_page
            {
                tab.session.view.current_page = p;
                tab.ui.thumb_scroll_to_current = true;
            }
        }

        // ---- paint ----
        let painter = ui.painter_at(rect);
        self.paint_pages(&painter, ctx, &vc);
        self.paint_overlays(&painter, &vc);
        self.paint_snap_hover(&painter, &vc);
        self.paint_scrollbars(ui, &vc);
        self.canvas_cursor(ctx, &response);
    }

    fn paint_pages(&mut self, painter: &egui::Painter, ctx: &egui::Context, vc: &ViewCtx) {
        let ti = self.active;
        let (doc, revision) = (self.tabs[ti].session.id, self.tabs[ti].session.revision());
        let band_top = f64::from(vc.scroll.y - PREFETCH_PX);
        let band_bottom = f64::from(vc.scroll.y + vc.viewport.height() + PREFETCH_PX);
        let zoom_settling = self.tabs[ti]
            .ui
            .last_zoom_change
            .is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(140));
        if zoom_settling {
            ctx.request_repaint_after(std::time::Duration::from_millis(160));
        }
        let visible_pages = vc.layout.pages_in_band(band_top, band_bottom);
        let expanded = vc.viewport.expand(PREFETCH_PX);
        let snapshot = if zoom_settling {
            None
        } else {
            self.tabs[ti].session.snapshot().ok()
        };
        if let Some(s) = &snapshot {
            self.hub.ensure(s);
        }
        for i in visible_pages {
            let slot = vc.slot(i);
            // Page background + shadow.
            let shadow = Rect::from_min_max(
                slot.min + Vec2::new(2.0, 3.0),
                slot.max + Vec2::new(2.0, 3.0),
            );
            painter.rect_filled(shadow, 1.0, self.pal.page_shadow);
            painter.rect_filled(
                slot,
                0.0,
                if self.prefs.dark_page_filter {
                    Color32::BLACK
                } else {
                    Color32::WHITE
                },
            );
            let page = &vc.pages[i];
            let scale = vc.px_per_pt * f64::from(vc.ppp);
            let size = page.geometry.view_size(vc.rotation);
            let (w_px, h_px) = (
                (size.width * scale).ceil().max(1.0) as u32,
                (size.height * scale).ceil().max(1.0) as u32,
            );
            let vis = slot.intersect(expanded);
            if vis.width() <= 0.0 || vis.height() <= 0.0 {
                continue;
            }
            let rel = (
                f64::from((vis.min.x - slot.min.x) * vc.ppp),
                f64::from((vis.min.y - slot.min.y) * vc.ppp),
                f64::from((vis.max.x - slot.min.x) * vc.ppp),
                f64::from((vis.max.y - slot.min.y) * vc.ppp),
            );
            let in_view = slot.intersects(vc.viewport);
            let plans = plan_tiles(w_px, h_px, rel, 1);
            let scale_milli = quantize_scale(scale);
            let mut drew_any = false;
            for plan in plans {
                let key = TileKey {
                    doc,
                    revision,
                    page: page.id,
                    rotation: vc.rotation,
                    scale_milli,
                    tx: plan.tx,
                    ty: plan.ty,
                    whole: plan.whole,
                };
                let dest = Rect::from_min_size(
                    slot.min + Vec2::new(plan.x as f32 / vc.ppp, plan.y as f32 / vc.ppp),
                    Vec2::new(plan.w as f32 / vc.ppp, plan.h as f32 / vc.ppp),
                );
                if let Some(tex) = self.tiles.get(&key) {
                    painter.image(
                        tex.id(),
                        dest,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    drew_any = true;
                } else {
                    if !drew_any {
                        self.paint_placeholder(painter, vc, i, slot);
                        drew_any = true;
                    }
                    if !zoom_settling {
                        self.request_tile(
                            key,
                            plan,
                            i,
                            page,
                            if in_view { 100 } else { 40 },
                            doc,
                            revision,
                        );
                    }
                }
            }
        }
        // Thumbnails of visible pages keep the placeholder path warm.
    }

    /// Draw the best cached stand-in for a page that is not rendered at the right scale yet.
    fn paint_placeholder(&mut self, painter: &egui::Painter, vc: &ViewCtx, i: usize, slot: Rect) {
        let ti = self.active;
        let page = vc.pages[i].id;
        let doc = self.tabs[ti].session.id;
        // Prefer a whole-page tile of the same page/rotation (any revision/scale): closest scale.
        let target = quantize_scale(vc.px_per_pt * f64::from(vc.ppp)) as i64;
        let best = self
            .tiles
            .keys()
            .filter(|k| k.doc == doc && k.page == page && k.rotation == vc.rotation && k.whole)
            .min_by_key(|k| (i64::from(k.scale_milli) - target).abs())
            .copied();
        if let Some(k) = best
            && let Some(tex) = self.tiles.get(&k)
        {
            painter.image(
                tex.id(),
                slot,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn request_tile(
        &mut self,
        key: TileKey,
        plan: TilePlan,
        page_index: usize,
        page: &PageInfo,
        priority: i32,
        doc: editor_core::session::DocId,
        revision: u64,
    ) {
        if self.in_flight.contains(&key) || self.failed.contains(&key) || self.in_flight.len() > 96
        {
            return;
        }
        // Panels are painted in any order; make sure workers exist for this exact revision.
        if !self.hub.has_pool(doc, revision)
            && let Some(tab) = self.tabs.iter_mut().find(|t| t.session.id == doc)
            && let Ok(snap) = tab.session.snapshot()
        {
            self.hub.ensure(&snap);
        }
        let accepted = self.hub.submit(Job {
            doc,
            revision,
            request: 0,
            priority,
            cancel: Arc::new(AtomicBool::new(false)),
            kind: JobKind::Tile {
                key,
                plan,
                geometry: page.geometry,
                page_index,
            },
        });
        if accepted {
            self.in_flight.insert(key);
        }
    }

    fn paint_scrollbars(&mut self, ui: &mut egui::Ui, vc: &ViewCtx) {
        let ti = self.active;
        let content = vc.layout.content;
        let (cw, ch) = (content.width as f32, content.height as f32);
        let rect = vc.viewport;
        let painter = ui.painter_at(rect);
        let thumb_col = self.pal.text_dim.gamma_multiply(0.55);
        if ch > rect.height() + 1.0 {
            let track = Rect::from_min_max(Pos2::new(rect.max.x - 10.0, rect.min.y), rect.max);
            let frac = rect.height() / ch;
            let th = (track.height() * frac).max(28.0);
            let y =
                track.min.y + (track.height() - th) * (vc.scroll.y / (ch - rect.height()).max(1.0));
            let thumb = Rect::from_min_size(Pos2::new(track.min.x + 2.0, y), Vec2::new(6.0, th));
            let resp = ui.interact(track, ui.id().with("vscroll"), Sense::click_and_drag());
            painter.rect_filled(
                thumb,
                3.0,
                if resp.hovered() || resp.dragged() {
                    self.pal.text_dim
                } else {
                    thumb_col
                },
            );
            if (resp.dragged() || resp.clicked())
                && let Some(p) = resp.interact_pointer_pos()
            {
                let t = ((p.y - track.min.y - th / 2.0) / (track.height() - th).max(1.0))
                    .clamp(0.0, 1.0);
                self.tabs[ti].session.view.scroll.y = f64::from(t * (ch - rect.height()));
            }
        }
        if cw > rect.width() + 1.0 {
            let track = Rect::from_min_max(Pos2::new(rect.min.x, rect.max.y - 10.0), rect.max);
            let frac = rect.width() / cw;
            let tw = (track.width() * frac).max(28.0);
            let x =
                track.min.x + (track.width() - tw) * (vc.scroll.x / (cw - rect.width()).max(1.0));
            let thumb = Rect::from_min_size(Pos2::new(x, track.min.y + 2.0), Vec2::new(tw, 6.0));
            let resp = ui.interact(track, ui.id().with("hscroll"), Sense::click_and_drag());
            painter.rect_filled(
                thumb,
                3.0,
                if resp.hovered() || resp.dragged() {
                    self.pal.text_dim
                } else {
                    thumb_col
                },
            );
            if (resp.dragged() || resp.clicked())
                && let Some(p) = resp.interact_pointer_pos()
            {
                let t = ((p.x - track.min.x - tw / 2.0) / (track.width() - tw).max(1.0))
                    .clamp(0.0, 1.0);
                self.tabs[ti].session.view.scroll.x = f64::from(t * (cw - rect.width()));
            }
        }
    }
}
