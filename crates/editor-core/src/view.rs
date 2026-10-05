//! View state and page layout. Pure geometry — no UI toolkit types.
//!
//! "Zoom" is relative to 100% = 96 logical pixels per inch (1.333 px per PDF point).

use pdf_engine::geom::{Point, Rect, Rotation, Size, Vec2};

/// Logical pixels per PDF point at 100% zoom.
pub const PX_PER_PT_AT_100: f64 = 96.0 / 72.0;
/// Smallest allowed zoom factor (5%).
pub const MIN_ZOOM: f64 = 0.05;
/// Largest allowed zoom factor (6400%).
pub const MAX_ZOOM: f64 = 64.0;

/// Page arrangement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ViewMode {
    /// One column, scrolling through all pages.
    #[default]
    Continuous,
    /// One page at a time.
    SinglePage,
    /// Two pages side by side, scrolling continuously.
    Facing,
}

/// How zoom is determined.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ZoomMode {
    /// Explicit factor.
    #[default]
    Custom,
    /// Fit the current page entirely in the viewport.
    FitPage,
    /// Fit the current page width to the viewport.
    FitWidth,
}

/// Per-tab view state.
#[derive(Clone, Debug)]
pub struct ViewState {
    /// Page arrangement.
    pub mode: ViewMode,
    /// Zoom determination.
    pub zoom_mode: ZoomMode,
    /// Zoom factor (1.0 = 100%).
    pub zoom: f64,
    /// Temporary display rotation (never saved).
    pub rotation: Rotation,
    /// Current page (zero-based).
    pub current_page: usize,
    /// Scroll offset: content coordinates of the viewport's top-left corner.
    pub scroll: Vec2,
    /// In facing mode, show the first page alone (like a book cover).
    pub cover_page: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            mode: ViewMode::Continuous,
            zoom_mode: ZoomMode::FitWidth,
            zoom: 1.0,
            rotation: Rotation::R0,
            current_page: 0,
            scroll: Vec2::ZERO,
            cover_page: true,
        }
    }
}

/// Spacing used by the layout.
#[derive(Clone, Copy, Debug)]
pub struct LayoutMetrics {
    /// Gap between pages (logical px).
    pub gap: f64,
    /// Margin around the content (logical px).
    pub margin: f64,
}

impl Default for LayoutMetrics {
    fn default() -> Self {
        Self {
            gap: 12.0,
            margin: 16.0,
        }
    }
}

/// Positions of all pages in content space.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    /// One rect per page (zero-sized when not laid out, e.g. hidden in single-page mode).
    pub slots: Vec<Rect>,
    /// Total content size.
    pub content: Size,
}

impl Layout {
    /// Lay out pages whose displayed sizes (points, rotation applied) are `sizes`.
    pub fn compute(
        sizes: &[Size],
        mode: ViewMode,
        cover_page: bool,
        current: usize,
        px_per_pt: f64,
        viewport_w: f64,
        m: LayoutMetrics,
    ) -> Layout {
        let scaled: Vec<Size> = sizes
            .iter()
            .map(|s| Size::new(s.width * px_per_pt, s.height * px_per_pt))
            .collect();
        let mut slots = vec![Rect::ZERO; sizes.len()];
        match mode {
            ViewMode::Continuous => {
                let max_w = scaled.iter().map(|s| s.width).fold(0.0, f64::max);
                let w = (max_w + 2.0 * m.margin).max(viewport_w);
                let mut y = m.margin;
                for (i, s) in scaled.iter().enumerate() {
                    let x = (w - s.width) / 2.0;
                    slots[i] = Rect::new(x, y, x + s.width, y + s.height);
                    y += s.height + m.gap;
                }
                Layout {
                    slots,
                    content: Size::new(w, (y - m.gap + m.margin).max(0.0)),
                }
            }
            ViewMode::SinglePage => {
                let cur = current.min(scaled.len().saturating_sub(1));
                if let Some(s) = scaled.get(cur) {
                    let w = (s.width + 2.0 * m.margin).max(viewport_w);
                    let x = (w - s.width) / 2.0;
                    slots[cur] = Rect::new(x, m.margin, x + s.width, m.margin + s.height);
                    Layout {
                        slots,
                        content: Size::new(w, s.height + 2.0 * m.margin),
                    }
                } else {
                    Layout::default()
                }
            }
            ViewMode::Facing => {
                // Rows of (left, right?) pages.
                let mut rows: Vec<Vec<usize>> = Vec::new();
                let mut i = 0;
                if cover_page && !scaled.is_empty() {
                    rows.push(vec![0]);
                    i = 1;
                }
                while i < scaled.len() {
                    if i + 1 < scaled.len() {
                        rows.push(vec![i, i + 1]);
                        i += 2;
                    } else {
                        rows.push(vec![i]);
                        i += 1;
                    }
                }
                // Column width is the widest page so single pages align with spreads.
                let max_w = scaled.iter().map(|s| s.width).fold(0.0, f64::max);
                let total_w = (2.0 * max_w + m.gap + 2.0 * m.margin).max(viewport_w);
                let centre = total_w / 2.0;
                let mut y = m.margin;
                for row in &rows {
                    let h = row.iter().map(|&p| scaled[p].height).fold(0.0, f64::max);
                    match row.as_slice() {
                        [a, b] => {
                            let xa = centre - m.gap / 2.0 - scaled[*a].width;
                            let xb = centre + m.gap / 2.0;
                            slots[*a] =
                                Rect::new(xa, y, xa + scaled[*a].width, y + scaled[*a].height);
                            slots[*b] =
                                Rect::new(xb, y, xb + scaled[*b].width, y + scaled[*b].height);
                        }
                        [a] => {
                            // Cover/last page sits on the right (cover) or left (last) of centre.
                            let is_cover = cover_page && *a == 0;
                            let x = if is_cover {
                                centre + m.gap / 2.0
                            } else {
                                centre - m.gap / 2.0 - scaled[*a].width
                            };
                            slots[*a] =
                                Rect::new(x, y, x + scaled[*a].width, y + scaled[*a].height);
                        }
                        _ => {}
                    }
                    y += h + m.gap;
                }
                Layout {
                    slots,
                    content: Size::new(total_w, (y - m.gap + m.margin).max(0.0)),
                }
            }
        }
    }

    /// Pages whose slot intersects the vertical band `[top, bottom]` (content coordinates).
    pub fn pages_in_band(&self, top: f64, bottom: f64) -> Vec<usize> {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, r)| r.height() > 0.0 && r.y1 >= top && r.y0 <= bottom)
            .map(|(i, _)| i)
            .collect()
    }

    /// The page that "is current" for a scroll position: the one with the largest visible
    /// area, ties broken toward the top.
    pub fn current_page_for(&self, scroll_y: f64, viewport_h: f64) -> Option<usize> {
        let (top, bottom) = (scroll_y, scroll_y + viewport_h);
        let mut best: Option<(usize, f64)> = None;
        for i in self.pages_in_band(top, bottom) {
            let r = self.slots[i];
            let vis = (r.y1.min(bottom) - r.y0.max(top)).max(0.0) * r.width();
            if best.is_none_or(|(_, b)| vis > b + 1e-6) {
                best = Some((i, vis));
            }
        }
        best.map(|(i, _)| i)
    }

    /// Locate a content point: `(page, fx, fy)` with fractions of the page slot (clamped).
    pub fn locate(&self, p: Point) -> Option<(usize, f64, f64)> {
        let mut best: Option<(usize, f64)> = None;
        for (i, r) in self.slots.iter().enumerate() {
            if r.height() <= 0.0 {
                continue;
            }
            let dx = (r.x0 - p.x).max(0.0).max(p.x - r.x1);
            let dy = (r.y0 - p.y).max(0.0).max(p.y - r.y1);
            let d = dx * dx + dy * dy;
            if best.is_none_or(|(_, b)| d < b) {
                best = Some((i, d));
            }
        }
        let (i, _) = best?;
        let r = self.slots[i];
        Some((
            i,
            ((p.x - r.x0) / r.width()).clamp(0.0, 1.0),
            ((p.y - r.y0) / r.height()).clamp(0.0, 1.0),
        ))
    }

    /// Page under a content point (exact hit only).
    pub fn page_at(&self, p: Point) -> Option<usize> {
        self.slots
            .iter()
            .position(|r| r.height() > 0.0 && r.contains(p))
    }
}

/// Zoom (factor, 1.0 = 100%) that fits a page width into the viewport.
pub fn fit_width_zoom(page: Size, viewport_w: f64, margin: f64) -> f64 {
    (((viewport_w - 2.0 * margin).max(10.0)) / (page.width * PX_PER_PT_AT_100))
        .clamp(MIN_ZOOM, MAX_ZOOM)
}

/// Zoom (factor) that fits a whole page into the viewport.
pub fn fit_page_zoom(page: Size, viewport: Size, margin: f64) -> f64 {
    let zw = ((viewport.width - 2.0 * margin).max(10.0)) / (page.width * PX_PER_PT_AT_100);
    let zh = ((viewport.height - 2.0 * margin).max(10.0)) / (page.height * PX_PER_PT_AT_100);
    zw.min(zh).clamp(MIN_ZOOM, MAX_ZOOM)
}

/// Discrete zoom steps used by Zoom In/Out buttons.
pub const ZOOM_STEPS: &[f64] = &[
    0.05, 0.1, 0.25, 0.33, 0.5, 0.67, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0,
    24.0, 32.0, 48.0, 64.0,
];

/// Next larger step.
pub fn zoom_in_step(z: f64) -> f64 {
    ZOOM_STEPS
        .iter()
        .copied()
        .find(|s| *s > z * 1.001)
        .unwrap_or(MAX_ZOOM)
}

/// Next smaller step.
pub fn zoom_out_step(z: f64) -> f64 {
    ZOOM_STEPS
        .iter()
        .rev()
        .copied()
        .find(|s| *s < z / 1.001)
        .unwrap_or(MIN_ZOOM)
}

/// Compute the new scroll offset after changing zoom so the content under `anchor`
/// (viewport coordinates) stays put. `old`/`new` are layouts before/after.
pub fn scroll_after_zoom(old: &Layout, new: &Layout, old_scroll: Vec2, anchor: Vec2) -> Vec2 {
    let p = Point::new(old_scroll.x + anchor.x, old_scroll.y + anchor.y);
    let Some((page, fx, fy)) = old.locate(p) else {
        return old_scroll;
    };
    let Some(r) = new.slots.get(page).copied().filter(|r| r.height() > 0.0) else {
        return old_scroll;
    };
    Vec2::new(
        r.x0 + fx * r.width() - anchor.x,
        r.y0 + fy * r.height() - anchor.y,
    )
}

/// Clamp scroll to the content bounds for a viewport.
pub fn clamp_scroll(scroll: Vec2, content: Size, viewport: Size) -> Vec2 {
    Vec2::new(
        scroll
            .x
            .clamp(0.0, (content.width - viewport.width).max(0.0)),
        scroll
            .y
            .clamp(0.0, (content.height - viewport.height).max(0.0)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn a4() -> Size {
        Size::new(595.0, 842.0)
    }

    #[test]
    fn continuous_layout_stacks_and_centres() {
        let sizes = vec![a4(), Size::new(300.0, 400.0), a4()];
        let l = Layout::compute(
            &sizes,
            ViewMode::Continuous,
            false,
            0,
            1.0,
            1000.0,
            LayoutMetrics::default(),
        );
        assert_eq!(l.slots.len(), 3);
        assert!(l.slots[0].y1 + 12.0 <= l.slots[1].y0 + 1e-9);
        // Narrow page is centred in a 1000px-wide viewport.
        let mid = (l.slots[1].x0 + l.slots[1].x1) / 2.0;
        assert!((mid - 500.0).abs() < 1e-9);
        assert_eq!(l.pages_in_band(0.0, 100.0), vec![0]);
    }

    #[test]
    fn single_page_hides_others() {
        let l = Layout::compute(
            &[a4(), a4()],
            ViewMode::SinglePage,
            false,
            1,
            1.0,
            800.0,
            LayoutMetrics::default(),
        );
        assert_eq!(l.slots[0], Rect::ZERO);
        assert!(l.slots[1].height() > 0.0);
        assert_eq!(l.page_at(Point::new(400.0, 100.0)), Some(1));
    }

    #[test]
    fn facing_pairs_with_cover() {
        let l = Layout::compute(
            &[a4(), a4(), a4(), a4()],
            ViewMode::Facing,
            true,
            0,
            1.0,
            1500.0,
            LayoutMetrics::default(),
        );
        // Cover alone on the right; pages 1+2 share a row; 3 alone on the left.
        assert!(l.slots[1].y0 > l.slots[0].y0);
        assert_eq!(l.slots[1].y0, l.slots[2].y0);
        assert!(l.slots[1].x1 < l.slots[2].x0);
        assert!(l.slots[3].y0 > l.slots[1].y0);
        let l2 = Layout::compute(
            &[a4(), a4()],
            ViewMode::Facing,
            false,
            0,
            1.0,
            1500.0,
            LayoutMetrics::default(),
        );
        assert_eq!(l2.slots[0].y0, l2.slots[1].y0);
    }

    #[test]
    fn fit_modes_and_steps() {
        let z = fit_width_zoom(a4(), 1000.0, 16.0);
        assert!((595.0 * PX_PER_PT_AT_100 * z - 968.0).abs() < 1e-6);
        let zp = fit_page_zoom(a4(), Size::new(2000.0, 900.0), 16.0);
        assert!((842.0 * PX_PER_PT_AT_100 * zp - 868.0).abs() < 1e-6);
        assert_eq!(zoom_in_step(1.0), 1.25);
        assert_eq!(zoom_out_step(1.0), 0.75);
        assert_eq!(zoom_in_step(64.0), 64.0);
        assert_eq!(zoom_out_step(0.05), 0.05);
    }

    #[test]
    fn current_page_follows_scroll() {
        let sizes = vec![a4(); 5];
        let l = Layout::compute(
            &sizes,
            ViewMode::Continuous,
            false,
            0,
            1.0,
            800.0,
            LayoutMetrics::default(),
        );
        assert_eq!(l.current_page_for(0.0, 600.0), Some(0));
        let y = l.slots[3].y0 - 20.0;
        assert_eq!(l.current_page_for(y, 600.0), Some(3));
    }

    proptest! {
        #[test]
        fn zoom_keeps_anchor_content_fixed(
            z0 in 0.2f64..4.0, z1 in 0.2f64..4.0,
            ax in 0.0f64..800.0, ay in 0.0f64..600.0,
            sy in 0.0f64..3000.0,
        ) {
            let sizes = vec![a4(); 6];
            let m = LayoutMetrics::default();
            let l0 = Layout::compute(&sizes, ViewMode::Continuous, false, 0, z0 * PX_PER_PT_AT_100, 800.0, m);
            let l1 = Layout::compute(&sizes, ViewMode::Continuous, false, 0, z1 * PX_PER_PT_AT_100, 800.0, m);
            let scroll = Vec2::new(0.0, sy.min((l0.content.height - 600.0).max(0.0)));
            let anchor = Vec2::new(ax, ay);
            let before = l0.locate(Point::new(scroll.x + ax, scroll.y + ay)).unwrap();
            let ns = scroll_after_zoom(&l0, &l1, scroll, anchor);
            let after = l1.locate(Point::new(ns.x + ax, ns.y + ay)).unwrap();
            prop_assert_eq!(before.0, after.0);
            prop_assert!((before.1 - after.1).abs() < 1e-6 && (before.2 - after.2).abs() < 1e-6);
        }
    }
}
