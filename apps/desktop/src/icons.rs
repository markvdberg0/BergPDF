//! Original vector icons drawn with egui primitives on a 24×24 grid (no third-party assets).

use egui::{Color32, Painter, Pos2, Rect, Stroke, Vec2, epaint::PathShape};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Snap,
    Sparkle,
    Translate,
    Optimize,
    Lock,
    Bookmark,
    Sliders,
    Print,
    Redact,
    RedactApply,
    Archive,
    Open,
    Save,
    SaveAs,
    Close,
    Undo,
    Redo,
    Find,
    ZoomIn,
    ZoomOut,
    FitPage,
    FitWidth,
    RotateCw,
    RotateCcw,
    Hand,
    Cursor,
    TextCursor,
    Highlight,
    Underline,
    Strike,
    Note,
    TextBox,
    Callout,
    Rect,
    Ellipse,
    Cloud,
    Line,
    Arrow,
    Polygon,
    Polyline,
    Pencil,
    Stamp,
    EditText,
    AddText,
    AddImage,
    Trash,
    Duplicate,
    Extract,
    InsertPage,
    Merge,
    SidebarL,
    SidebarR,
    Settings,
    Palette,
    Page,
    Spread,
    Scroll,
    Single,
    Moon,
    ArrowUp,
    ArrowDown,
    Info,
    Keys,
    FormField,
    Ruler,
    PathLen,
    AreaPoly,
    RectMeasure,
    RadiusMeasure,
    AngleMeasure,
    CountMark,
    Calibrate,
    Export,
    Ocr,
    Signature,
    Certificate,
    Shield,
}

/// `pts` with every corner rounded by radius `r` (a quadratic curve through the corner, never longer
/// than half of the adjoining sides). Open paths keep their end points.
fn rounded(pts: &[(f32, f32)], r: f32, closed: bool) -> Vec<(f32, f32)> {
    let n = pts.len();
    let mut out = Vec::new();
    for i in 0..n {
        let p = pts[i];
        if !closed && (i == 0 || i == n - 1) {
            out.push(p);
            continue;
        }
        let a = pts[(i + n - 1) % n];
        let b = pts[(i + 1) % n];
        let (da, db) = ((a.0 - p.0, a.1 - p.1), (b.0 - p.0, b.1 - p.1));
        let (la, lb) = (da.0.hypot(da.1), db.0.hypot(db.1));
        let d = r.min(la / 2.0).min(lb / 2.0);
        let s = (p.0 + da.0 / la * d, p.1 + da.1 / la * d);
        let e = (p.0 + db.0 / lb * d, p.1 + db.1 / lb * d);
        for k in 0..=6 {
            let t = k as f32 / 6.0;
            let m = 1.0 - t;
            out.push((
                m * m * s.0 + 2.0 * m * t * p.0 + t * t * e.0,
                m * m * s.1 + 2.0 * m * t * p.1 + t * t * e.1,
            ));
        }
    }
    out
}

/// Points of the arc of a circle, angles in degrees (0 = right, 90 = down).
fn arc(c: (f32, f32), r: f32, from: f32, to: f32) -> Vec<(f32, f32)> {
    (0..=14)
        .map(|k| {
            let a = (from + (to - from) * k as f32 / 14.0).to_radians();
            (c.0 + r * a.cos(), c.1 + r * a.sin())
        })
        .collect()
}

/// Paint an icon in `rect` (square) using `color`.
pub fn paint(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height());
    let o = rect.center() - Vec2::splat(s / 2.0);
    let u = s / 24.0;
    let pt = |x: f32, y: f32| Pos2::new(o.x + x * u, o.y + y * u);
    let st = Stroke::new((1.6 * u).max(1.0), color);
    let line = |a: (f32, f32), b: (f32, f32)| {
        p.line_segment([pt(a.0, a.1), pt(b.0, b.1)], st);
    };
    let poly = |pts: &[(f32, f32)], closed: bool| {
        let v: Vec<Pos2> = pts.iter().map(|q| pt(q.0, q.1)).collect();
        p.add(PathShape {
            points: v,
            closed,
            fill: Color32::TRANSPARENT,
            stroke: st.into(),
        });
    };
    let fill_poly = |pts: &[(f32, f32)], c: Color32| {
        let v: Vec<Pos2> = pts.iter().map(|q| pt(q.0, q.1)).collect();
        p.add(PathShape::convex_polygon(v, c, Stroke::NONE));
    };
    let rrect = |x0: f32, y0: f32, x1: f32, y1: f32| {
        p.rect_stroke(
            Rect::from_min_max(pt(x0, y0), pt(x1, y1)),
            2.0 * u,
            st,
            egui::StrokeKind::Middle,
        );
    };
    let dim = color.gamma_multiply(0.35);
    let circle = |c: (f32, f32), r: f32| p.circle_stroke(pt(c.0, c.1), r * u, st);
    match icon {
        Icon::Snap => {
            circle((12.0, 12.0), 7.0);
            line((12.0, 2.0), (12.0, 8.0));
            line((12.0, 16.0), (12.0, 22.0));
            line((2.0, 12.0), (8.0, 12.0));
            line((16.0, 12.0), (22.0, 12.0));
            rrect(10.0, 10.0, 14.0, 14.0);
        }
        Icon::Sparkle => {
            poly(
                &[
                    (11.0, 2.0),
                    (13.0, 9.0),
                    (20.0, 11.0),
                    (13.0, 13.0),
                    (11.0, 20.0),
                    (9.0, 13.0),
                    (2.0, 11.0),
                    (9.0, 9.0),
                ],
                true,
            );
            line((19.0, 16.0), (19.0, 21.0));
            line((16.5, 18.5), (21.5, 18.5));
        }
        Icon::Translate => {
            rrect(2.0, 3.0, 14.0, 15.0);
            line((5.0, 7.0), (11.0, 7.0));
            line((8.0, 5.0), (8.0, 7.0));
            line((6.0, 12.0), (10.0, 8.0));
            rrect(10.0, 11.0, 22.0, 21.0);
            line((13.0, 19.0), (16.0, 13.0));
            line((16.0, 13.0), (19.0, 19.0));
            line((14.0, 17.0), (18.0, 17.0));
        }
        Icon::Optimize => {
            rrect(5.0, 3.0, 19.0, 21.0);
            line((12.0, 7.0), (12.0, 13.0));
            poly(&[(8.0, 11.0), (12.0, 15.0), (16.0, 11.0)], false);
            line((8.0, 18.0), (16.0, 18.0));
        }
        Icon::Bookmark => {
            poly(
                &[
                    (7.0, 3.0),
                    (17.0, 3.0),
                    (17.0, 21.0),
                    (12.0, 16.5),
                    (7.0, 21.0),
                ],
                true,
            );
        }
        Icon::Sliders => {
            line((4.0, 8.0), (20.0, 8.0));
            rrect(7.0, 5.0, 11.0, 11.0);
            line((4.0, 16.0), (20.0, 16.0));
            rrect(13.0, 13.0, 17.0, 19.0);
        }
        Icon::Print => {
            rrect(7.0, 3.0, 17.0, 9.0);
            rrect(3.0, 9.0, 21.0, 17.0);
            rrect(7.0, 14.0, 17.0, 21.0);
            line((9.0, 17.0), (15.0, 17.0));
        }
        Icon::Redact => {
            line((4.0, 6.0), (20.0, 6.0));
            line((4.0, 18.0), (20.0, 18.0));
            rrect(4.0, 9.5, 20.0, 14.5);
            line((7.0, 12.0), (17.0, 12.0));
        }
        Icon::RedactApply => {
            rrect(4.0, 4.0, 20.0, 20.0);
            line((7.0, 9.0), (17.0, 9.0));
            rrect(7.0, 12.0, 17.0, 16.0);
            line((16.0, 20.0), (21.0, 22.0));
        }
        Icon::Lock => {
            rrect(5.0, 11.0, 19.0, 21.0);
            poly(
                &[
                    (8.0, 11.0),
                    (8.0, 7.0),
                    (10.0, 4.0),
                    (14.0, 4.0),
                    (16.0, 7.0),
                    (16.0, 11.0),
                ],
                false,
            );
            line((12.0, 14.0), (12.0, 18.0));
        }
        Icon::Archive => {
            rrect(3.0, 4.0, 21.0, 9.0);
            rrect(4.0, 9.0, 20.0, 21.0);
            line((9.0, 13.0), (15.0, 13.0));
        }
        Icon::Open => {
            // A folder with its tab, and the front flap.
            poly(
                &rounded(
                    &[
                        (3.0, 5.0),
                        (9.0, 5.0),
                        (11.5, 8.0),
                        (20.0, 8.0),
                        (20.0, 19.0),
                        (3.0, 19.0),
                    ],
                    2.0,
                    true,
                ),
                true,
            );
            poly(
                &rounded(&[(3.0, 19.0), (5.5, 11.5), (21.5, 11.5)], 1.5, false),
                false,
            );
        }
        Icon::Save => {
            poly(
                &rounded(
                    &[
                        (4.0, 4.0),
                        (16.0, 4.0),
                        (20.0, 8.0),
                        (20.0, 20.0),
                        (4.0, 20.0),
                    ],
                    2.0,
                    true,
                ),
                true,
            );
            poly(
                &rounded(
                    &[(8.0, 4.0), (8.0, 9.0), (15.0, 9.0), (15.0, 4.0)],
                    1.0,
                    false,
                ),
                false,
            );
            poly(
                &rounded(
                    &[(7.0, 20.0), (7.0, 14.0), (17.0, 14.0), (17.0, 20.0)],
                    1.0,
                    false,
                ),
                false,
            );
        }
        Icon::SaveAs => {
            poly(
                &rounded(
                    &[
                        (3.0, 4.0),
                        (12.0, 4.0),
                        (15.0, 7.0),
                        (15.0, 20.0),
                        (3.0, 20.0),
                    ],
                    2.0,
                    true,
                ),
                true,
            );
            poly(
                &rounded(
                    &[(6.0, 4.0), (6.0, 8.5), (11.0, 8.5), (11.0, 4.0)],
                    1.0,
                    false,
                ),
                false,
            );
            // A small pencil: the copy is written under a new name.
            poly(
                &[
                    (14.5, 19.5),
                    (15.0, 16.5),
                    (20.0, 11.5),
                    (22.0, 13.5),
                    (17.0, 18.5),
                    (14.5, 19.5),
                ],
                true,
            );
        }
        Icon::Close => {
            line((6.0, 6.0), (18.0, 18.0));
            line((18.0, 6.0), (6.0, 18.0));
        }
        Icon::Undo | Icon::Redo => {
            // An arrow that turns back: head on the left (undo) or right (redo), then a loop.
            let m = |x: f32| if icon == Icon::Redo { 24.0 - x } else { x };
            let head = vec![(m(9.0), 4.0), (m(4.0), 9.0), (m(9.0), 14.0)];
            poly(&rounded(&head, 0.5, false), false);
            let mut loop_path = vec![(m(4.0), 9.0), (m(14.5), 9.0)];
            loop_path.extend(
                arc((14.5, 14.5), 5.5, -90.0, 90.0)
                    .into_iter()
                    .skip(1)
                    .map(|(x, y)| (m(x), y)),
            );
            loop_path.push((m(11.0), 20.0));
            poly(&loop_path, false);
        }
        Icon::Find => {
            circle((10.0, 10.0), 6.0);
            line((15.0, 15.0), (21.0, 21.0));
        }
        Icon::ZoomIn => {
            circle((10.0, 10.0), 6.5);
            line((15.0, 15.0), (21.0, 21.0));
            line((7.0, 10.0), (13.0, 10.0));
            line((10.0, 7.0), (10.0, 13.0));
        }
        Icon::ZoomOut => {
            circle((10.0, 10.0), 6.5);
            line((15.0, 15.0), (21.0, 21.0));
            line((7.0, 10.0), (13.0, 10.0));
        }
        Icon::FitPage => {
            rrect(6.0, 3.0, 18.0, 21.0);
            line((3.0, 8.0), (3.0, 3.0));
            line((3.0, 3.0), (8.0, 3.0));
            line((21.0, 16.0), (21.0, 21.0));
            line((21.0, 21.0), (16.0, 21.0));
        }
        Icon::FitWidth => {
            rrect(6.0, 4.0, 18.0, 20.0);
            line((2.0, 12.0), (6.0, 12.0));
            line((18.0, 12.0), (22.0, 12.0));
            poly(&[(4.0, 10.0), (2.0, 12.0), (4.0, 14.0)], false);
            poly(&[(20.0, 10.0), (22.0, 12.0), (20.0, 14.0)], false);
        }
        Icon::RotateCw => {
            poly(
                &[
                    (19.0, 12.0),
                    (18.0, 16.0),
                    (14.0, 19.0),
                    (9.0, 19.0),
                    (5.0, 16.0),
                    (4.0, 12.0),
                    (6.0, 8.0),
                    (10.0, 5.5),
                    (14.0, 5.5),
                ],
                false,
            );
            poly(&[(12.0, 3.0), (15.0, 5.5), (12.0, 8.0)], false);
        }
        Icon::RotateCcw => {
            poly(
                &[
                    (5.0, 12.0),
                    (6.0, 16.0),
                    (10.0, 19.0),
                    (15.0, 19.0),
                    (19.0, 16.0),
                    (20.0, 12.0),
                    (18.0, 8.0),
                    (14.0, 5.5),
                    (10.0, 5.5),
                ],
                false,
            );
            poly(&[(12.0, 3.0), (9.0, 5.5), (12.0, 8.0)], false);
        }
        Icon::Hand => {
            poly(
                &[
                    (8.0, 20.0),
                    (5.0, 14.0),
                    (5.0, 11.0),
                    (7.0, 11.0),
                    (8.0, 13.0),
                    (8.0, 5.0),
                    (10.0, 5.0),
                    (10.0, 11.0),
                    (10.0, 4.0),
                    (12.0, 4.0),
                    (12.0, 11.0),
                    (12.0, 5.0),
                    (14.0, 5.0),
                    (14.0, 12.0),
                    (14.0, 8.0),
                    (16.0, 8.0),
                    (16.0, 15.0),
                    (14.0, 20.0),
                ],
                true,
            );
        }
        Icon::Cursor => {
            fill_poly(
                &[
                    (6.0, 3.0),
                    (6.0, 19.0),
                    (10.0, 15.0),
                    (13.0, 21.0),
                    (15.5, 20.0),
                    (12.5, 14.0),
                    (18.0, 14.0),
                ],
                color,
            );
        }
        Icon::TextCursor => {
            line((12.0, 4.0), (12.0, 20.0));
            line((8.0, 4.0), (16.0, 4.0));
            line((8.0, 20.0), (16.0, 20.0));
        }
        Icon::Highlight => {
            fill_poly(
                &[(3.0, 21.0), (3.0, 17.0), (21.0, 17.0), (21.0, 21.0)],
                color.gamma_multiply(0.5),
            );
            poly(&[(5.0, 15.0), (14.0, 4.0), (19.0, 8.0), (10.0, 17.0)], true);
            line((5.0, 15.0), (4.0, 17.0));
        }
        Icon::Underline => {
            line((8.0, 4.0), (8.0, 12.0));
            line((16.0, 4.0), (16.0, 12.0));
            poly(
                &[
                    (8.0, 12.0),
                    (9.0, 15.0),
                    (12.0, 16.0),
                    (15.0, 15.0),
                    (16.0, 12.0),
                ],
                false,
            );
            line((5.0, 20.0), (19.0, 20.0));
        }
        Icon::Strike => {
            line((4.0, 12.0), (20.0, 12.0));
            poly(
                &[
                    (17.0, 7.0),
                    (14.0, 5.0),
                    (10.0, 5.0),
                    (7.0, 7.5),
                    (8.0, 10.5),
                    (12.0, 12.0),
                ],
                false,
            );
            poly(
                &[
                    (12.0, 12.0),
                    (16.0, 13.5),
                    (17.0, 16.5),
                    (14.0, 19.0),
                    (10.0, 19.0),
                    (7.0, 17.0),
                ],
                false,
            );
        }
        Icon::Note => {
            poly(
                &[
                    (4.0, 4.0),
                    (20.0, 4.0),
                    (20.0, 16.0),
                    (14.0, 16.0),
                    (14.0, 21.0),
                    (10.0, 16.0),
                    (4.0, 16.0),
                ],
                true,
            );
            line((8.0, 8.0), (16.0, 8.0));
            line((8.0, 12.0), (14.0, 12.0));
        }
        Icon::TextBox => {
            rrect(3.0, 4.0, 21.0, 20.0);
            line((8.0, 9.0), (16.0, 9.0));
            line((12.0, 9.0), (12.0, 16.0));
        }
        Icon::Callout => {
            rrect(8.0, 3.0, 21.0, 12.0);
            poly(&[(10.0, 12.0), (4.0, 20.0)], false);
            line((4.0, 20.0), (4.0, 16.5));
            line((4.0, 20.0), (7.5, 19.0));
        }
        Icon::Rect => rrect(4.0, 6.0, 20.0, 18.0),
        Icon::Ellipse => {
            p.add(egui::epaint::EllipseShape::stroke(
                pt(12.0, 12.0),
                Vec2::new(8.5 * u, 6.5 * u),
                st,
            ));
        }
        Icon::Cloud => {
            // A rectangle whose sides bulge outwards in scallops.
            let mut pts: Vec<(f32, f32)> = Vec::new();
            let (x0, y0, x1, y1) = (5.0_f32, 7.0_f32, 19.0_f32, 17.0_f32);
            let corners = [(x0, y1), (x1, y1), (x1, y0), (x0, y0)];
            for s in 0..4 {
                let (a, b) = (corners[s], corners[(s + 1) % 4]);
                let n = if s % 2 == 0 { 3 } else { 2 };
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                let len = dx.hypot(dy);
                let (nx, ny) = (-dy / len, dx / len);
                for i in 0..n {
                    for k in 0..6 {
                        let t = (i as f32 + k as f32 / 6.0) / n as f32;
                        let bulge = (std::f32::consts::PI * (k as f32 / 6.0)).sin() * 2.2;
                        pts.push((a.0 + dx * t + nx * bulge, a.1 + dy * t + ny * bulge));
                    }
                }
            }
            poly(&pts, true);
        }
        Icon::Line => line((4.0, 20.0), (20.0, 4.0)),
        Icon::Arrow => {
            line((4.0, 20.0), (19.0, 5.0));
            line((19.0, 5.0), (12.0, 5.0));
            line((19.0, 5.0), (19.0, 12.0));
        }
        Icon::Polygon => poly(
            &[
                (12.0, 3.0),
                (21.0, 10.0),
                (17.5, 20.0),
                (6.5, 20.0),
                (3.0, 10.0),
            ],
            true,
        ),
        Icon::Polyline => {
            poly(&[(3.0, 18.0), (9.0, 7.0), (14.0, 15.0), (21.0, 5.0)], false);
        }
        Icon::Pencil => {
            poly(
                &[
                    (4.0, 20.0),
                    (5.0, 15.0),
                    (16.0, 4.0),
                    (20.0, 8.0),
                    (9.0, 19.0),
                ],
                true,
            );
            line((14.0, 6.0), (18.0, 10.0));
        }
        Icon::Stamp => {
            rrect(8.0, 3.0, 16.0, 10.0);
            poly(
                &[
                    (10.0, 10.0),
                    (10.0, 14.0),
                    (4.0, 14.0),
                    (4.0, 19.0),
                    (20.0, 19.0),
                    (20.0, 14.0),
                    (14.0, 14.0),
                    (14.0, 10.0),
                ],
                false,
            );
            line((4.0, 22.0), (20.0, 22.0));
        }
        Icon::EditText => {
            line((4.0, 6.0), (4.0, 6.0));
            line((5.0, 5.0), (14.0, 5.0));
            line((9.5, 5.0), (9.5, 17.0));
            line((7.0, 17.0), (12.0, 17.0));
            poly(
                &[
                    (14.0, 21.0),
                    (15.0, 17.0),
                    (21.0, 11.0),
                    (23.0, 13.0),
                    (17.0, 19.0),
                ],
                true,
            );
        }
        Icon::AddText => {
            line((4.0, 6.0), (16.0, 6.0));
            line((10.0, 6.0), (10.0, 18.0));
            line((7.0, 18.0), (13.0, 18.0));
            line((19.0, 13.0), (19.0, 21.0));
            line((15.0, 17.0), (23.0, 17.0));
        }
        Icon::AddImage => {
            rrect(3.0, 5.0, 17.0, 19.0);
            poly(
                &[(4.0, 17.0), (9.0, 11.0), (13.0, 15.0), (16.0, 12.0)],
                false,
            );
            circle((8.0, 9.0), 1.5);
            line((20.0, 3.0), (20.0, 11.0));
            line((16.0, 7.0), (24.0, 7.0));
        }
        Icon::FormField => {
            rrect(3.0, 6.0, 21.0, 18.0);
            line((7.0, 9.0), (7.0, 15.0));
            line((10.0, 12.0), (15.0, 12.0));
        }
        Icon::Ruler => {
            poly(&[(3.0, 17.0), (17.0, 3.0), (21.0, 7.0), (7.0, 21.0)], true);
            for k in 0..4 {
                let t = 4.0 + k as f32 * 3.4;
                line((t, 20.0 - t + 2.0), (t + 1.8, 20.0 - t + 0.2));
            }
        }
        Icon::PathLen => {
            poly(&[(3.0, 18.0), (9.0, 8.0), (14.0, 15.0), (21.0, 5.0)], false);
            circle((3.0, 18.0), 1.6);
            circle((21.0, 5.0), 1.6);
        }
        Icon::AreaPoly => {
            fill_poly(&[(4.0, 18.0), (7.0, 6.0), (19.0, 8.0), (20.0, 19.0)], dim);
            poly(&[(4.0, 18.0), (7.0, 6.0), (19.0, 8.0), (20.0, 19.0)], true);
        }
        Icon::RectMeasure => {
            fill_poly(&[(4.0, 6.0), (20.0, 6.0), (20.0, 18.0), (4.0, 18.0)], dim);
            rrect(4.0, 6.0, 20.0, 18.0);
            line((4.0, 21.0), (20.0, 21.0));
        }
        Icon::RadiusMeasure => {
            circle((12.0, 12.0), 8.0);
            line((12.0, 12.0), (20.0, 12.0));
            p.circle_filled(pt(12.0, 12.0), 1.5 * u, color);
        }
        Icon::AngleMeasure => {
            poly(&[(20.0, 19.0), (4.0, 19.0), (15.0, 5.0)], false);
            poly(&[(11.0, 19.0), (10.0, 15.0), (7.0, 13.0)], false);
        }
        Icon::CountMark => {
            circle((12.0, 12.0), 8.0);
            line((9.0, 12.0), (15.0, 12.0));
            line((12.0, 9.0), (12.0, 15.0));
        }
        Icon::Calibrate => {
            line((3.0, 12.0), (21.0, 12.0));
            line((3.0, 8.0), (3.0, 16.0));
            line((21.0, 8.0), (21.0, 16.0));
            line((9.0, 12.0), (9.0, 15.0));
            line((15.0, 12.0), (15.0, 15.0));
        }
        Icon::Ocr => {
            line((4.0, 8.0), (4.0, 4.0));
            line((4.0, 4.0), (8.0, 4.0));
            line((20.0, 8.0), (20.0, 4.0));
            line((20.0, 4.0), (16.0, 4.0));
            line((4.0, 16.0), (4.0, 20.0));
            line((4.0, 20.0), (8.0, 20.0));
            line((20.0, 16.0), (20.0, 20.0));
            line((20.0, 20.0), (16.0, 20.0));
            line((8.0, 9.0), (16.0, 9.0));
            line((8.0, 12.0), (14.0, 12.0));
            line((8.0, 15.0), (16.0, 15.0));
        }
        Icon::Signature => {
            poly(
                &[
                    (3.0, 17.0),
                    (6.0, 9.0),
                    (8.0, 15.0),
                    (11.0, 8.0),
                    (13.0, 15.0),
                    (16.0, 11.0),
                    (18.0, 13.0),
                ],
                false,
            );
            line((3.0, 20.0), (21.0, 20.0));
        }
        Icon::Certificate => {
            rrect(3.0, 4.0, 21.0, 16.0);
            line((7.0, 8.0), (17.0, 8.0));
            line((7.0, 12.0), (13.0, 12.0));
            circle((17.0, 17.0), 3.0);
            line((15.5, 19.5), (14.5, 23.0));
            line((18.5, 19.5), (19.5, 23.0));
        }
        Icon::Shield => {
            poly(
                &[
                    (12.0, 3.0),
                    (20.0, 6.0),
                    (20.0, 13.0),
                    (12.0, 21.0),
                    (4.0, 13.0),
                    (4.0, 6.0),
                ],
                true,
            );
            poly(&[(8.5, 12.0), (11.0, 14.5), (16.0, 9.0)], false);
        }
        Icon::Export => {
            rrect(4.0, 4.0, 16.0, 20.0);
            line((12.0, 12.0), (22.0, 12.0));
            line((19.0, 9.0), (22.0, 12.0));
            line((19.0, 15.0), (22.0, 12.0));
        }
        Icon::Trash => {
            line((4.0, 7.0), (20.0, 7.0));
            poly(&[(6.0, 7.0), (7.0, 21.0), (17.0, 21.0), (18.0, 7.0)], false);
            poly(&[(9.0, 7.0), (9.0, 4.0), (15.0, 4.0), (15.0, 7.0)], false);
            line((10.0, 11.0), (10.0, 17.0));
            line((14.0, 11.0), (14.0, 17.0));
        }
        Icon::Duplicate => {
            rrect(8.0, 8.0, 20.0, 20.0);
            poly(&[(4.0, 16.0), (4.0, 4.0), (16.0, 4.0)], false);
        }
        Icon::Extract => {
            rrect(4.0, 3.0, 14.0, 17.0);
            line((15.0, 17.0), (21.0, 17.0));
            poly(&[(18.0, 14.0), (21.0, 17.0), (18.0, 20.0)], false);
        }
        Icon::InsertPage => {
            rrect(5.0, 3.0, 17.0, 21.0);
            line((11.0, 9.0), (11.0, 17.0));
            line((7.0, 13.0), (15.0, 13.0));
        }
        Icon::Merge => {
            rrect(3.0, 3.0, 11.0, 11.0);
            rrect(13.0, 13.0, 21.0, 21.0);
            poly(&[(14.0, 8.0), (18.0, 8.0), (18.0, 12.0)], false);
        }
        Icon::SidebarL => {
            rrect(3.0, 4.0, 21.0, 20.0);
            line((9.0, 4.0), (9.0, 20.0));
        }
        Icon::SidebarR => {
            rrect(3.0, 4.0, 21.0, 20.0);
            line((15.0, 4.0), (15.0, 20.0));
        }
        Icon::Settings => {
            circle((12.0, 12.0), 3.0);
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                line(
                    (12.0 + 6.0 * a.cos(), 12.0 + 6.0 * a.sin()),
                    (12.0 + 9.0 * a.cos(), 12.0 + 9.0 * a.sin()),
                );
            }
            circle((12.0, 12.0), 6.0);
        }
        Icon::Palette => {
            rrect(3.0, 6.0, 21.0, 18.0);
            line((6.0, 12.0), (11.0, 12.0));
            poly(&[(14.0, 10.0), (16.5, 14.0), (19.0, 10.0)], false);
        }
        Icon::Page => {
            poly(
                &[
                    (5.0, 3.0),
                    (14.0, 3.0),
                    (19.0, 8.0),
                    (19.0, 21.0),
                    (5.0, 21.0),
                ],
                true,
            );
            poly(&[(14.0, 3.0), (14.0, 8.0), (19.0, 8.0)], false);
        }
        Icon::Spread => {
            rrect(2.0, 5.0, 12.0, 19.0);
            rrect(12.0, 5.0, 22.0, 19.0);
        }
        Icon::Scroll => {
            rrect(6.0, 2.0, 18.0, 10.0);
            rrect(6.0, 12.0, 18.0, 22.0);
        }
        Icon::Single => rrect(6.0, 3.0, 18.0, 21.0),
        Icon::Moon => {
            poly(
                &[
                    (15.0, 4.0),
                    (11.0, 5.5),
                    (8.0, 9.0),
                    (8.0, 14.0),
                    (11.0, 18.0),
                    (16.0, 19.5),
                    (20.0, 17.0),
                    (16.0, 16.0),
                    (13.0, 13.0),
                    (13.0, 8.0),
                ],
                true,
            );
        }
        Icon::ArrowUp => {
            line((12.0, 19.0), (12.0, 5.0));
            poly(&[(6.0, 11.0), (12.0, 5.0), (18.0, 11.0)], false);
        }
        Icon::ArrowDown => {
            line((12.0, 5.0), (12.0, 19.0));
            poly(&[(6.0, 13.0), (12.0, 19.0), (18.0, 13.0)], false);
        }
        Icon::Info => {
            circle((12.0, 12.0), 9.0);
            line((12.0, 11.0), (12.0, 17.0));
            line((12.0, 7.0), (12.0, 7.2));
        }
        Icon::Keys => {
            rrect(2.0, 6.0, 22.0, 18.0);
            for x in [5.0, 9.0, 13.0, 17.0] {
                line((x, 10.0), (x + 1.5, 10.0));
            }
            line((7.0, 14.5), (17.0, 14.5));
        }
    }
    let _ = dim;
}

/// Draw the BergPDF logo (Mont Blanc outline with a copper snow line) inside `rect`.
pub fn paint_logo(p: &Painter, rect: Rect, outline: Color32, accent: Color32) {
    let s = rect.width().min(rect.height());
    let o = rect.center() - Vec2::splat(s / 2.0);
    let u = s / 100.0;
    // Inset the artwork slightly so round joins are not clipped.
    let (scale, off) = (0.94, 3.0);
    let to =
        |q: &(f32, f32)| Pos2::new(o.x + (q.0 * scale + off) * u, o.y + (q.1 * scale + off) * u);
    let width = (brand::STROKE * scale * u).max(1.0);
    for (poly, is_accent) in brand::strokes() {
        let pts: Vec<Pos2> = poly.iter().map(to).collect();
        let st = Stroke::new(width, if is_accent { accent } else { outline });
        p.add(PathShape {
            points: pts.clone(),
            closed: false,
            fill: Color32::TRANSPARENT,
            stroke: st.into(),
        });
        // Round joins/caps for the soft look of the raster icon.
        for q in &pts {
            p.circle_filled(*q, width / 2.0, st.color);
        }
    }
}
