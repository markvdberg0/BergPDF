//! Snap points taken from the vector geometry of a page (technical drawings).
//!
//! The page's content streams (and the Form XObjects they call) are walked once; every stroked
//! or filled path contributes straight segments and the anchor points of its sub-paths. A
//! spatial grid then answers "what is the best snap target within `radius` of this point?" in
//! microseconds, so it can run on every pointer move.
//!
//! Targets, in order of preference: path end points and rectangle corners, intersections of
//! straight segments, midpoints of straight segments, and finally the nearest point on any line
//! or curve. Curves are flattened for the last case only; their interior is never offered as
//! an end point.
//!
//! All coordinates are default user space of the page (the space annotations use). Clip-only
//! paths (`n`) are ignored. Text and raster images contribute nothing.

use crate::content::{self, Mat, Op, Operand};
use crate::geom::Point;
use crate::objutil;
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::HashMap;

/// Upper bound on stored segments (guards against hostile or enormous drawings).
pub const MAX_SEGMENTS: usize = 600_000;
const MAX_DEPTH: usize = 6;
const MAX_STREAM_BYTES: usize = 64 * 1024 * 1024;
const CELL: f64 = 12.0;
const CURVE_STEPS: usize = 8;
const NEAR_LIMIT: usize = 48;

/// What kind of feature a snap hit is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapKind {
    /// End point of a line/curve or corner of a rectangle or polygon.
    Endpoint,
    /// Crossing of two straight segments.
    Intersection,
    /// Middle of a straight segment.
    Midpoint,
    /// Nearest point on a line or curve.
    Edge,
}

impl SnapKind {
    /// Short label for the UI.
    pub fn label(self) -> &'static str {
        match self {
            SnapKind::Endpoint => "Endpoint",
            SnapKind::Intersection => "Intersection",
            SnapKind::Midpoint => "Midpoint",
            SnapKind::Edge => "On line",
        }
    }
}

/// A snap result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapHit {
    /// Point in page user space.
    pub point: Point,
    /// Kind of feature.
    pub kind: SnapKind,
}

#[derive(Clone, Copy, Debug)]
struct Seg {
    a: Point,
    b: Point,
    curve: bool,
}

type Cell = (i32, i32);

/// Searchable geometry of one page.
#[derive(Default)]
pub struct SnapIndex {
    segs: Vec<Seg>,
    nodes: Vec<Point>,
    seg_grid: HashMap<Cell, Vec<u32>>,
    node_grid: HashMap<Cell, Vec<u32>>,
    truncated: bool,
}

fn cell_of(p: Point) -> Cell {
    (
        (p.x / CELL).floor().clamp(-1e8, 1e8) as i32,
        (p.y / CELL).floor().clamp(-1e8, 1e8) as i32,
    )
}

fn dist(a: Point, b: Point) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

fn point_on_segment(p: Point, s: &Seg) -> (Point, f64) {
    let (dx, dy) = (s.b.x - s.a.x, s.b.y - s.a.y);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= f64::EPSILON {
        0.0
    } else {
        (((p.x - s.a.x) * dx + (p.y - s.a.y) * dy) / len2).clamp(0.0, 1.0)
    };
    let q = Point::new(s.a.x + t * dx, s.a.y + t * dy);
    (q, dist(p, q))
}

fn intersect(a: &Seg, b: &Seg) -> Option<Point> {
    let (r_x, r_y) = (a.b.x - a.a.x, a.b.y - a.a.y);
    let (s_x, s_y) = (b.b.x - b.a.x, b.b.y - b.a.y);
    let denom = r_x * s_y - r_y * s_x;
    if denom.abs() < 1e-12 {
        return None;
    }
    let (qp_x, qp_y) = (b.a.x - a.a.x, b.a.y - a.a.y);
    let t = (qp_x * s_y - qp_y * s_x) / denom;
    let u = (qp_x * r_y - qp_y * r_x) / denom;
    let eps = 1e-9;
    if (-eps..=1.0 + eps).contains(&t) && (-eps..=1.0 + eps).contains(&u) {
        Some(Point::new(a.a.x + t * r_x, a.a.y + t * r_y))
    } else {
        None
    }
}

struct Builder<'a> {
    doc: Option<&'a Document>,
    segs: Vec<Seg>,
    nodes: Vec<Point>,
    truncated: bool,
}

#[derive(Default)]
struct PathState {
    segs: Vec<Seg>,
    nodes: Vec<Point>,
    cur: Option<Point>,
    start: Option<Point>,
}

impl PathState {
    fn clear(&mut self) {
        self.segs.clear();
        self.nodes.clear();
        self.cur = None;
        self.start = None;
    }
}

fn num(op: &Op, i: usize) -> Option<f64> {
    op.operands
        .get(i)
        .and_then(Operand::num)
        .filter(|v| v.is_finite())
}

fn bezier(p0: Point, p1: Point, p2: Point, p3: Point, t: f64) -> Point {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    Point::new(
        a * p0.x + b * p1.x + c * p2.x + d * p3.x,
        a * p0.y + b * p1.y + c * p2.y + d * p3.y,
    )
}

impl<'a> Builder<'a> {
    fn commit(&mut self, path: &mut PathState) {
        if self.segs.len() + path.segs.len() > MAX_SEGMENTS {
            self.truncated = true;
        } else {
            self.segs.append(&mut path.segs);
            self.nodes.append(&mut path.nodes);
        }
        path.clear();
    }

    fn walk(&mut self, data: &[u8], res: Option<&'a Dictionary>, base: Mat, depth: usize) {
        let Ok(ops) = content::scan(data) else { return };
        let mut ctm = base;
        let mut stack: Vec<Mat> = Vec::new();
        let mut path = PathState::default();
        let pt = |ctm: &Mat, x: f64, y: f64| {
            let (px, py) = ctm.apply(x, y);
            Point::new(px, py)
        };
        for op in &ops {
            if self.truncated {
                return;
            }
            match op.name.as_slice() {
                b"q" => {
                    if stack.len() < 256 {
                        stack.push(ctm);
                    }
                }
                b"Q" => {
                    if let Some(m) = stack.pop() {
                        ctm = m;
                    }
                }
                b"cm" => {
                    if let [Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)] = [
                        num(op, 0),
                        num(op, 1),
                        num(op, 2),
                        num(op, 3),
                        num(op, 4),
                        num(op, 5),
                    ] {
                        ctm = Mat([a, b, c, d, e, f]).then(&ctm);
                    }
                }
                b"m" => {
                    if let (Some(x), Some(y)) = (num(op, 0), num(op, 1)) {
                        let p = pt(&ctm, x, y);
                        path.cur = Some(p);
                        path.start = Some(p);
                        path.nodes.push(p);
                    }
                }
                b"l" => {
                    if let (Some(x), Some(y)) = (num(op, 0), num(op, 1)) {
                        let p = pt(&ctm, x, y);
                        if let Some(c) = path.cur {
                            path.segs.push(Seg {
                                a: c,
                                b: p,
                                curve: false,
                            });
                        } else {
                            path.start = Some(p);
                        }
                        path.nodes.push(p);
                        path.cur = Some(p);
                    }
                }
                b"c" | b"v" | b"y" => {
                    let n: Vec<Option<f64>> = (0..op.operands.len()).map(|i| num(op, i)).collect();
                    let Some(p0) = path.cur else { continue };
                    let g = |i: usize| n.get(i).copied().flatten();
                    let (c1, c2, e) = match op.name.as_slice() {
                        b"c" => match (g(0), g(1), g(2), g(3), g(4), g(5)) {
                            (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)) => {
                                (pt(&ctm, a, b), pt(&ctm, c, d), pt(&ctm, e, f))
                            }
                            _ => continue,
                        },
                        b"v" => match (g(0), g(1), g(2), g(3)) {
                            (Some(c), Some(d), Some(e), Some(f)) => {
                                (p0, pt(&ctm, c, d), pt(&ctm, e, f))
                            }
                            _ => continue,
                        },
                        _ => match (g(0), g(1), g(2), g(3)) {
                            (Some(a), Some(b), Some(e), Some(f)) => {
                                let end = pt(&ctm, e, f);
                                (pt(&ctm, a, b), end, end)
                            }
                            _ => continue,
                        },
                    };
                    let mut prev = p0;
                    for k in 1..=CURVE_STEPS {
                        let q = bezier(p0, c1, c2, e, k as f64 / CURVE_STEPS as f64);
                        path.segs.push(Seg {
                            a: prev,
                            b: q,
                            curve: true,
                        });
                        prev = q;
                    }
                    path.nodes.push(e);
                    path.cur = Some(e);
                }
                b"re" => {
                    if let (Some(x), Some(y), Some(w), Some(h)) =
                        (num(op, 0), num(op, 1), num(op, 2), num(op, 3))
                    {
                        let c = [
                            pt(&ctm, x, y),
                            pt(&ctm, x + w, y),
                            pt(&ctm, x + w, y + h),
                            pt(&ctm, x, y + h),
                        ];
                        for k in 0..4 {
                            path.segs.push(Seg {
                                a: c[k],
                                b: c[(k + 1) % 4],
                                curve: false,
                            });
                        }
                        path.nodes.extend_from_slice(&c);
                        path.cur = Some(c[0]);
                        path.start = Some(c[0]);
                    }
                }
                b"h" => close(&mut path),
                b"S" | b"f" | b"F" | b"f*" | b"B" | b"B*" => self.commit(&mut path),
                b"s" | b"b" | b"b*" => {
                    close(&mut path);
                    self.commit(&mut path);
                }
                b"n" => path.clear(),
                b"Do" => {
                    if depth < MAX_DEPTH
                        && let Some(Operand::Name(name)) = op.operands.first()
                    {
                        self.form(name, res, &ctm, depth);
                    }
                }
                _ => {}
            }
        }
    }

    fn form(&mut self, name: &[u8], res: Option<&'a Dictionary>, ctm: &Mat, depth: usize) {
        let Some(doc) = self.doc else { return };
        let Some(res) = res else { return };
        let Some(xo) = res
            .get(b"XObject")
            .ok()
            .and_then(|o| objutil::deref(doc, o))
            .and_then(|o| o.as_dict().ok())
        else {
            return;
        };
        let Some(Object::Stream(s)) = xo.get(name).ok().and_then(|o| objutil::deref(doc, o)) else {
            return;
        };
        let is_form = s
            .dict
            .get(b"Subtype")
            .ok()
            .and_then(|o| o.as_name().ok())
            .is_some_and(|n| n == b"Form");
        if !is_form {
            return;
        }
        let m = s
            .dict
            .get(b"Matrix")
            .ok()
            .and_then(|o| objutil::deref(doc, o))
            .and_then(|o| o.as_array().ok())
            .and_then(|a| {
                let v: Vec<f64> = a.iter().filter_map(|o| objutil::num(doc, o)).collect();
                (v.len() == 6).then(|| Mat([v[0], v[1], v[2], v[3], v[4], v[5]]))
            })
            .unwrap_or(Mat::IDENTITY);
        let Ok(data) = s.decompressed_content_with_limit(MAX_STREAM_BYTES) else {
            return;
        };
        let own_res = s
            .dict
            .get(b"Resources")
            .ok()
            .and_then(|o| objutil::deref(doc, o))
            .and_then(|o| o.as_dict().ok())
            .or(Some(res));
        self.walk(&data, own_res, m.then(ctm), depth + 1);
    }
}

fn close(path: &mut PathState) {
    if let (Some(c), Some(s)) = (path.cur, path.start)
        && dist(c, s) > 1e-9
    {
        path.segs.push(Seg {
            a: c,
            b: s,
            curve: false,
        });
        path.cur = Some(s);
    }
}

impl SnapIndex {
    /// Build the index of a page.
    pub fn build(doc: &Document, page: ObjectId) -> SnapIndex {
        let mut data = Vec::new();
        for id in crate::pagecontent::content_stream_ids(doc, page) {
            if let Some(Object::Stream(s)) = doc.objects.get(&id)
                && let Ok(d) = s.decompressed_content_with_limit(MAX_STREAM_BYTES)
            {
                data.extend_from_slice(&d);
                data.push(b'\n');
            }
            if data.len() > MAX_STREAM_BYTES {
                break;
            }
        }
        let res = objutil::inherited_obj(doc, page, b"Resources");
        let res_dict = res
            .as_ref()
            .and_then(|o| objutil::deref(doc, o))
            .and_then(|o| o.as_dict().ok());
        let mut b = Builder {
            doc: Some(doc),
            segs: Vec::new(),
            nodes: Vec::new(),
            truncated: false,
        };
        b.walk(&data, res_dict, Mat::IDENTITY, 0);
        Self::finish(b)
    }

    /// Build from a bare content stream (no resources, so `Do` is ignored). Used by tests.
    pub fn from_content(data: &[u8]) -> SnapIndex {
        let mut b = Builder {
            doc: None,
            segs: Vec::new(),
            nodes: Vec::new(),
            truncated: false,
        };
        b.walk(data, None, Mat::IDENTITY, 0);
        Self::finish(b)
    }

    fn finish(b: Builder<'_>) -> SnapIndex {
        let mut ix = SnapIndex {
            truncated: b.truncated,
            ..SnapIndex::default()
        };
        for s in b.segs {
            if !(s.a.x.is_finite() && s.a.y.is_finite() && s.b.x.is_finite() && s.b.y.is_finite()) {
                continue;
            }
            let id = ix.segs.len() as u32;
            ix.segs.push(s);
            // Walk along the segment so long lines are found from every cell they cross.
            let len = dist(s.a, s.b);
            let steps = ((len / (CELL / 2.0)).ceil() as usize).clamp(1, 20_000);
            let mut last: Option<Cell> = None;
            for k in 0..=steps {
                let t = k as f64 / steps as f64;
                let c = cell_of(Point::new(
                    s.a.x + t * (s.b.x - s.a.x),
                    s.a.y + t * (s.b.y - s.a.y),
                ));
                if last != Some(c) {
                    ix.seg_grid.entry(c).or_default().push(id);
                    last = Some(c);
                }
            }
        }
        let mut seen: std::collections::HashSet<(i64, i64)> = std::collections::HashSet::new();
        for n in b.nodes {
            if !(n.x.is_finite() && n.y.is_finite()) {
                continue;
            }
            // De-duplicate at 1/1000 pt.
            if !seen.insert(((n.x * 1000.0).round() as i64, (n.y * 1000.0).round() as i64)) {
                continue;
            }
            let id = ix.nodes.len() as u32;
            ix.nodes.push(n);
            ix.node_grid.entry(cell_of(n)).or_default().push(id);
        }
        ix
    }

    /// Number of stored straight/flattened segments.
    pub fn segment_count(&self) -> usize {
        self.segs.len()
    }

    /// Whether the page had more geometry than [`MAX_SEGMENTS`].
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// Whether there is nothing to snap to.
    pub fn is_empty(&self) -> bool {
        self.segs.is_empty() && self.nodes.is_empty()
    }

    fn cells_around(p: Point, radius: f64) -> impl Iterator<Item = Cell> {
        let (x0, y0) = cell_of(Point::new(p.x - radius, p.y - radius));
        let (x1, y1) = cell_of(Point::new(p.x + radius, p.y + radius));
        (x0..=x1).flat_map(move |x| (y0..=y1).map(move |y| (x, y)))
    }

    /// Best snap target within `radius` (points) of `p`, if any.
    pub fn query(&self, p: Point, radius: f64) -> Option<SnapHit> {
        if self.is_empty() || radius <= 0.0 || radius.is_nan() {
            return None;
        }
        // Lower score wins; the penalties make end points beat intersections beat midpoints
        // beat "somewhere on the line" while still honouring distance.
        let pen = |k: SnapKind| match k {
            SnapKind::Endpoint => 0.0,
            SnapKind::Intersection => 0.15,
            SnapKind::Midpoint => 0.3,
            SnapKind::Edge => 1.0,
        } * radius;
        let mut best: Option<(f64, SnapHit)> = None;
        let mut offer = |point: Point, kind: SnapKind| {
            let d = dist(p, point);
            if d <= radius {
                let score = d + pen(kind);
                if best.as_ref().is_none_or(|(s, _)| score < *s) {
                    best = Some((score, SnapHit { point, kind }));
                }
            }
        };
        let mut near_segs: Vec<(f64, u32)> = Vec::new();
        for c in Self::cells_around(p, radius) {
            if let Some(ids) = self.node_grid.get(&c) {
                for &i in ids {
                    offer(self.nodes[i as usize], SnapKind::Endpoint);
                }
            }
            if let Some(ids) = self.seg_grid.get(&c) {
                for &i in ids {
                    near_segs.push((0.0, i));
                }
            }
        }
        near_segs.sort_unstable_by_key(|(_, i)| *i);
        near_segs.dedup_by_key(|(_, i)| *i);
        for e in near_segs.iter_mut() {
            e.0 = point_on_segment(p, &self.segs[e.1 as usize]).1;
        }
        near_segs.sort_by(|a, b| a.0.total_cmp(&b.0));
        near_segs.truncate(NEAR_LIMIT);
        for &(d, i) in &near_segs {
            let s = &self.segs[i as usize];
            if d <= radius {
                let (q, _) = point_on_segment(p, s);
                offer(q, SnapKind::Edge);
            }
            if !s.curve {
                offer(
                    Point::new((s.a.x + s.b.x) / 2.0, (s.a.y + s.b.y) / 2.0),
                    SnapKind::Midpoint,
                );
            }
        }
        for (k, &(_, i)) in near_segs.iter().enumerate() {
            let a = &self.segs[i as usize];
            if a.curve {
                continue;
            }
            for &(_, j) in &near_segs[k + 1..] {
                let b = &self.segs[j as usize];
                if b.curve {
                    continue;
                }
                if let Some(x) = intersect(a, b) {
                    offer(x, SnapKind::Intersection);
                }
            }
        }
        best.map(|(_, h)| h)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn pt(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn rectangle_corners_are_endpoints() {
        let ix = SnapIndex::from_content(b"10 20 100 50 re S");
        let h = ix.query(pt(111.0, 69.0), 5.0).unwrap();
        assert_eq!(h.kind, SnapKind::Endpoint);
        assert!(dist(h.point, pt(110.0, 70.0)) < 1e-9);
    }

    #[test]
    fn crossing_lines_give_an_intersection() {
        let ix = SnapIndex::from_content(b"0 0 m 100 100 l S 0 100 m 100 0 l S");
        let h = ix.query(pt(51.0, 49.0), 4.0).unwrap();
        assert_eq!(h.kind, SnapKind::Intersection);
        assert!(dist(h.point, pt(50.0, 50.0)) < 1e-9);
    }

    #[test]
    fn midpoint_and_edge_follow_priority() {
        let ix = SnapIndex::from_content(b"0 0 m 100 0 l S");
        let mid = ix.query(pt(51.0, 2.0), 6.0).unwrap();
        assert_eq!(mid.kind, SnapKind::Midpoint);
        assert!(dist(mid.point, pt(50.0, 0.0)) < 1e-9);
        let edge = ix.query(pt(25.0, 3.0), 6.0).unwrap();
        assert_eq!(edge.kind, SnapKind::Edge);
        assert!(dist(edge.point, pt(25.0, 0.0)) < 1e-9);
        assert!(ix.query(pt(25.0, 30.0), 6.0).is_none());
    }

    #[test]
    fn endpoint_beats_a_nearer_edge_point() {
        let ix = SnapIndex::from_content(b"0 0 m 100 0 l S");
        let h = ix.query(pt(97.0, 0.5), 6.0).unwrap();
        assert_eq!(h.kind, SnapKind::Endpoint);
        assert!(dist(h.point, pt(100.0, 0.0)) < 1e-9);
    }

    #[test]
    fn transforms_and_graphics_state_are_applied() {
        // Scale by 2 and shift by (10, 10) inside q/Q; the second line is outside.
        let ix = SnapIndex::from_content(b"q 2 0 0 2 10 10 cm 0 0 m 5 0 l S Q 0 0 m 1 0 l S");
        let h = ix.query(pt(20.0, 10.0), 2.0).unwrap();
        assert_eq!(h.kind, SnapKind::Endpoint);
        assert!(dist(h.point, pt(20.0, 10.0)) < 1e-9);
        let h2 = ix.query(pt(1.0, 0.0), 1.0).unwrap();
        assert!(dist(h2.point, pt(1.0, 0.0)) < 1e-9);
    }

    #[test]
    fn clip_paths_and_curve_interiors_are_not_targets() {
        let ix = SnapIndex::from_content(b"0 0 50 50 re W n 0 0 m 0 100 100 100 100 0 c S");
        // The clip rectangle's corner (50, 50) is not a target.
        assert!(
            ix.query(pt(50.0, 50.0), 3.0)
                .is_none_or(|h| dist(h.point, pt(50.0, 50.0)) > 1e-6)
        );
        // The curve's end point is.
        let h = ix.query(pt(99.0, 1.0), 4.0).unwrap();
        assert_eq!(h.kind, SnapKind::Endpoint);
        // No midpoint is offered for flattened curve pieces.
        let near_mid = ix.query(pt(50.0, 75.0), 3.0);
        assert!(near_mid.is_none_or(|h| h.kind == SnapKind::Edge));
    }

    #[test]
    fn closing_a_subpath_adds_the_last_edge() {
        let ix = SnapIndex::from_content(b"0 0 m 100 0 l 100 100 l h S");
        // Midpoint of the closing edge (100,100)->(0,0).
        let h = ix.query(pt(50.5, 50.5), 3.0).unwrap();
        assert_eq!(h.kind, SnapKind::Midpoint);
    }

    #[test]
    fn huge_input_is_bounded() {
        let mut s = Vec::new();
        for i in 0..2000 {
            s.extend_from_slice(format!("{i} 0 m {i} 10 l S\n").as_bytes());
        }
        let ix = SnapIndex::from_content(&s);
        assert_eq!(ix.segment_count(), 2000);
        assert!(!ix.truncated());
    }
}
