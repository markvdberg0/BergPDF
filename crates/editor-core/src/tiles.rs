//! Tile planning and a byte-budgeted LRU cache.
//!
//! A page is rendered as one bitmap when it is small enough at the requested scale;
//! otherwise it is cut into fixed-size tiles and only tiles near the viewport are requested,
//! so an A0 drawing at high zoom never allocates an unbounded bitmap.

use crate::session::DocId;
use pdf_engine::doc::PageId;
use pdf_engine::geom::Rotation;
use std::collections::HashMap;
use std::hash::Hash;

/// Tile edge in device pixels.
pub const TILE_PX: u32 = 512;
/// Pages up to this many device pixels are rendered in one piece.
pub const WHOLE_PAGE_MAX_PIXELS: u64 = 2048 * 2048;

/// Identity of a rendered tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileKey {
    /// Document.
    pub doc: DocId,
    /// Document revision the pixels were rendered from.
    pub revision: u64,
    /// Page.
    pub page: PageId,
    /// Temporary view rotation.
    pub rotation: Rotation,
    /// Device pixels per point × 1000 (quantised).
    pub scale_milli: u32,
    /// Tile column (0 for whole-page renders).
    pub tx: u32,
    /// Tile row (0 for whole-page renders).
    pub ty: u32,
    /// Whether this is a whole-page render.
    pub whole: bool,
}

/// One tile to render, in device pixels within the full page bitmap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TilePlan {
    /// Key (sans doc/revision/page, which the caller fills in).
    pub tx: u32,
    /// Tile row.
    pub ty: u32,
    /// Whole-page render?
    pub whole: bool,
    /// Left edge in device px.
    pub x: u32,
    /// Top edge in device px.
    pub y: u32,
    /// Width in device px.
    pub w: u32,
    /// Height in device px.
    pub h: u32,
}

/// Quantise a scale (device px per pt) for use in keys.
pub fn quantize_scale(scale: f64) -> u32 {
    (scale * 1000.0).round().clamp(1.0, f64::from(u32::MAX)) as u32
}

/// The scale value a key stands for.
pub fn dequantize_scale(milli: u32) -> f64 {
    f64::from(milli) / 1000.0
}

/// Plan the tiles needed to cover `visible` (device pixels within the page bitmap) of a page
/// whose full bitmap would be `page_w × page_h` device pixels.
pub fn plan_tiles(
    page_w: u32,
    page_h: u32,
    visible: (f64, f64, f64, f64),
    margin_tiles: u32,
) -> Vec<TilePlan> {
    if page_w == 0 || page_h == 0 {
        return Vec::new();
    }
    if u64::from(page_w) * u64::from(page_h) <= WHOLE_PAGE_MAX_PIXELS {
        return vec![TilePlan {
            tx: 0,
            ty: 0,
            whole: true,
            x: 0,
            y: 0,
            w: page_w,
            h: page_h,
        }];
    }
    let (vx0, vy0, vx1, vy1) = visible;
    let cols = page_w.div_ceil(TILE_PX);
    let rows = page_h.div_ceil(TILE_PX);
    let clamp_i =
        |v: f64, hi: u32| -> u32 { (v.floor().max(0.0) as u32).min(hi.saturating_sub(1)) };
    let c0 = clamp_i(vx0 / f64::from(TILE_PX), cols).saturating_sub(margin_tiles);
    let c1 = (clamp_i(vx1 / f64::from(TILE_PX), cols) + margin_tiles).min(cols - 1);
    let r0 = clamp_i(vy0 / f64::from(TILE_PX), rows).saturating_sub(margin_tiles);
    let r1 = (clamp_i(vy1 / f64::from(TILE_PX), rows) + margin_tiles).min(rows - 1);
    let mut out = Vec::new();
    for ty in r0..=r1 {
        for tx in c0..=c1 {
            let x = tx * TILE_PX;
            let y = ty * TILE_PX;
            out.push(TilePlan {
                tx,
                ty,
                whole: false,
                x,
                y,
                w: TILE_PX.min(page_w - x),
                h: TILE_PX.min(page_h - y),
            });
        }
    }
    out
}

/// LRU cache bounded by an approximate byte budget.
pub struct ByteLru<K, V> {
    map: HashMap<K, (V, usize, u64)>,
    bytes: usize,
    budget: usize,
    tick: u64,
}

impl<K: Eq + Hash + Clone, V> ByteLru<K, V> {
    /// Create a cache with a budget in bytes.
    pub fn new(budget: usize) -> Self {
        Self {
            map: HashMap::new(),
            bytes: 0,
            budget,
            tick: 0,
        }
    }

    /// Current total size.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Change the budget (evicts if needed).
    pub fn set_budget(&mut self, budget: usize) {
        self.budget = budget;
        self.evict();
    }

    /// Fetch and mark as recently used.
    pub fn get(&mut self, k: &K) -> Option<&V> {
        self.tick += 1;
        let t = self.tick;
        self.map.get_mut(k).map(|e| {
            e.2 = t;
            &e.0
        })
    }

    /// Peek without touching recency.
    pub fn peek(&self, k: &K) -> Option<&V> {
        self.map.get(k).map(|e| &e.0)
    }

    /// Insert (replacing any previous value) and evict least-recently-used entries.
    pub fn insert(&mut self, k: K, v: V, bytes: usize) {
        self.tick += 1;
        if let Some((_, old, _)) = self.map.insert(k, (v, bytes, self.tick)) {
            self.bytes -= old;
        }
        self.bytes += bytes;
        self.evict();
    }

    /// Remove entries matching a predicate (e.g. stale revisions).
    pub fn retain(&mut self, mut keep: impl FnMut(&K) -> bool) {
        let mut freed = 0;
        self.map.retain(|k, (_, b, _)| {
            let keep = keep(k);
            if !keep {
                freed += *b;
            }
            keep
        });
        self.bytes -= freed;
    }

    /// Iterate keys.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.map.keys()
    }

    fn evict(&mut self) {
        while self.bytes > self.budget && self.map.len() > 1 {
            let oldest = self
                .map
                .iter()
                .min_by_key(|(_, (_, _, t))| *t)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(k) => {
                    if let Some((_, b, _)) = self.map.remove(&k) {
                        self.bytes -= b;
                    }
                }
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_pages_are_one_tile_large_pages_are_tiled_near_the_viewport() {
        let whole = plan_tiles(1000, 1400, (0.0, 0.0, 100.0, 100.0), 1);
        assert_eq!(whole.len(), 1);
        assert!(whole[0].whole);
        // A0 at high zoom: 40000 x 56000 device px.
        let t = plan_tiles(40_000, 56_000, (20_000.0, 30_000.0, 21_500.0, 31_000.0), 0);
        assert!(!t[0].whole);
        assert!(
            t.len() <= 4 * 3 + 2,
            "only viewport tiles requested, got {}",
            t.len()
        );
        assert!(t.iter().all(|p| p.w <= TILE_PX && p.h <= TILE_PX));
        assert!(t.iter().any(|p| p.x <= 20_000 && p.x + p.w > 20_000));
        // Edge tile is clipped to the page.
        let e = plan_tiles(1030 * 3, 3000, (3000.0, 2900.0, 3089.0, 2999.0), 0);
        assert_eq!(e.iter().map(|p| p.x + p.w).max(), Some(3090));
        assert!(e.iter().all(|p| p.y + p.h <= 3000));
    }

    #[test]
    fn tile_bytes_never_exceed_budget() {
        // Worst case tile memory: 512*512*4 = 1 MiB.
        assert_eq!(TILE_PX * TILE_PX * 4, 1 << 20);
    }

    #[test]
    fn lru_evicts_oldest_within_budget() {
        let mut c: ByteLru<u32, u32> = ByteLru::new(250);
        c.insert(1, 1, 100);
        c.insert(2, 2, 100);
        assert_eq!(c.get(&1), Some(&1)); // 1 is now most recent
        c.insert(3, 3, 100); // exceeds budget -> evict 2
        assert!(c.peek(&2).is_none());
        assert!(c.peek(&1).is_some() && c.peek(&3).is_some());
        assert!(c.bytes() <= 250);
        c.retain(|k| *k != 1);
        assert_eq!(c.len(), 1);
        assert_eq!(c.bytes(), 100);
    }

    #[test]
    fn scale_quantisation_roundtrips() {
        assert_eq!(quantize_scale(1.3333), 1333);
        assert!((dequantize_scale(1333) - 1.333).abs() < 1e-9);
    }
}
