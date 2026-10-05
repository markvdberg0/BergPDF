//! Measurement model: units, calibrated scales (document / page / region), geometry
//! computations, PDF persistence and CSV export.
//!
//! Measurements are ordinary PDF annotations (`Line`, `PolyLine`, `Polygon`, `Square`,
//! `Circle`) carrying the standard `/Measure` dictionary (ISO 32000-1 §12.9, `RL` subtype) so
//! other viewers can interpret them, plus a private `/BergMeasure` dictionary that records
//! the exact kind, category and scale this application used. The scale *registry* (which scale
//! applies where) is stored in a private `/BergScales` catalog entry that other readers
//! ignore. Computations are done in PDF user-space points; `/UserUnit` is not applied.

use crate::annot::{self, AnnotId, AnnotationKind, AnnotationSpec};
use crate::doc::{PageId, PdfDocument, Tx};
use crate::error::{EngineError, Result};
use crate::geom::{Point, Rect};
use crate::objutil::{self, num_array, real, reference, text_obj};
use lopdf::{Dictionary, Document, Object, ObjectId, dictionary};
use std::collections::BTreeMap;

/// Hard cap on vertices accepted for one measurement.
pub const MAX_POINTS: usize = 10_000;

/// Linear unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unit {
    /// PDF point (1/72 in) — used when nothing is calibrated.
    Pt,
    /// Millimetre.
    Mm,
    /// Centimetre.
    Cm,
    /// Metre.
    M,
    /// Kilometre.
    Km,
    /// Inch.
    In,
    /// Foot.
    Ft,
    /// Yard.
    Yd,
    /// Mile.
    Mi,
}

impl Unit {
    /// Units offered in the UI.
    pub const ALL: [Unit; 9] = [
        Unit::Mm,
        Unit::Cm,
        Unit::M,
        Unit::Km,
        Unit::In,
        Unit::Ft,
        Unit::Yd,
        Unit::Mi,
        Unit::Pt,
    ];

    /// Short symbol.
    pub fn abbr(self) -> &'static str {
        match self {
            Unit::Pt => "pt",
            Unit::Mm => "mm",
            Unit::Cm => "cm",
            Unit::M => "m",
            Unit::Km => "km",
            Unit::In => "in",
            Unit::Ft => "ft",
            Unit::Yd => "yd",
            Unit::Mi => "mi",
        }
    }

    /// Parse a symbol (case-insensitive, trims, accepts a few long names).
    pub fn parse(s: &str) -> Option<Unit> {
        Some(match s.trim().to_lowercase().as_str() {
            "pt" | "point" | "points" => Unit::Pt,
            "mm" | "millimetre" | "millimeter" => Unit::Mm,
            "cm" | "centimetre" | "centimeter" => Unit::Cm,
            "m" | "metre" | "meter" | "metres" | "meters" => Unit::M,
            "km" | "kilometre" | "kilometer" => Unit::Km,
            "in" | "inch" | "inches" | "\"" => Unit::In,
            "ft" | "foot" | "feet" | "'" => Unit::Ft,
            "yd" | "yard" | "yards" => Unit::Yd,
            "mi" | "mile" | "miles" => Unit::Mi,
            _ => return None,
        })
    }

    /// Length of one unit in metres.
    pub fn metres(self) -> f64 {
        match self {
            Unit::Pt => 0.0254 / 72.0,
            Unit::Mm => 0.001,
            Unit::Cm => 0.01,
            Unit::M => 1.0,
            Unit::Km => 1000.0,
            Unit::In => 0.0254,
            Unit::Ft => 0.3048,
            Unit::Yd => 0.9144,
            Unit::Mi => 1609.344,
        }
    }

    /// PDF points per unit (for paper-side lengths).
    pub fn points(self) -> f64 {
        self.metres() / Unit::Pt.metres()
    }
}

/// A calibrated (or default) drawing scale.
#[derive(Clone, Debug, PartialEq)]
pub struct Scale {
    /// Real-world unit that results are expressed in.
    pub unit: Unit,
    /// Real-world units per PDF point.
    pub units_per_pt: f64,
    /// Human description, e.g. `1 in = 10 ft` or `Calibrated: 12 ft`.
    pub text: String,
    /// `false` for the placeholder scale (results are in PDF points).
    pub calibrated: bool,
    /// Decimal places shown.
    pub precision: u8,
}

impl Scale {
    /// The placeholder scale: results in PDF points, flagged uncalibrated.
    pub fn uncalibrated() -> Scale {
        Scale {
            unit: Unit::Pt,
            units_per_pt: 1.0,
            text: "Not calibrated (PDF points)".into(),
            calibrated: false,
            precision: 1,
        }
    }

    /// `paper_len paper_unit = real_len real_unit` (e.g. `1 in = 10 ft`).
    pub fn from_ratio(
        paper_len: f64,
        paper_unit: Unit,
        real_len: f64,
        real_unit: Unit,
    ) -> Result<Scale> {
        if !(paper_len.is_finite() && real_len.is_finite()) || paper_len <= 0.0 || real_len <= 0.0 {
            return Err(EngineError::InvalidArgument(
                "scale lengths must be positive numbers".into(),
            ));
        }
        let upp = real_len / (paper_len * paper_unit.points());
        Ok(Scale {
            unit: real_unit,
            units_per_pt: upp,
            text: format!(
                "{} {} = {} {}",
                trim_num(paper_len),
                paper_unit.abbr(),
                trim_num(real_len),
                real_unit.abbr()
            ),
            calibrated: true,
            precision: 2,
        })
    }

    /// Architectural/engineering ratio `1:n` (paper and real lengths are in the same unit)
    /// with results shown in `display`.
    pub fn from_one_to(n: f64, display: Unit) -> Result<Scale> {
        if !n.is_finite() || n <= 0.0 {
            return Err(EngineError::InvalidArgument(
                "scale ratio must be positive".into(),
            ));
        }
        let upp = n * Unit::Pt.metres() / display.metres();
        Ok(Scale {
            unit: display,
            units_per_pt: upp,
            text: format!("1:{}", trim_num(n)),
            calibrated: true,
            precision: 2,
        })
    }

    /// Calibrate from a segment of `measured_pts` points known to be `real_len` `unit` long.
    pub fn from_calibration(measured_pts: f64, real_len: f64, unit: Unit) -> Result<Scale> {
        if !(measured_pts.is_finite() && real_len.is_finite())
            || measured_pts <= 1e-6
            || real_len <= 0.0
        {
            return Err(EngineError::InvalidArgument(
                "calibration needs two distinct points and a positive length".into(),
            ));
        }
        Ok(Scale {
            unit,
            units_per_pt: real_len / measured_pts,
            text: format!("measured {} {}", trim_num(real_len), unit.abbr()),
            calibrated: true,
            precision: 2,
        })
    }

    /// Format a length given in points.
    pub fn length_text(&self, pts: f64) -> String {
        format!(
            "{:.p$} {}",
            pts * self.units_per_pt,
            self.unit.abbr(),
            p = usize::from(self.precision)
        )
    }

    /// Format an area given in square points.
    pub fn area_text(&self, pts2: f64) -> String {
        format!(
            "{:.p$} {}²",
            pts2 * self.units_per_pt * self.units_per_pt,
            self.unit.abbr(),
            p = usize::from(self.precision)
        )
    }
}

fn trim_num(x: f64) -> String {
    let s = format!("{x:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

/// What a measurement annotation measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MeasureKind {
    /// Straight distance (2 points).
    Distance,
    /// Path length (≥ 2 points).
    Perimeter,
    /// Polygon area (≥ 3 points).
    Area,
    /// Rectangle (2 opposite corners): area + perimeter.
    RectArea,
    /// Circle (centre, edge point).
    Radius,
    /// Angle (arm, vertex, arm).
    Angle,
    /// Count marker (1 point).
    Count,
}

impl MeasureKind {
    /// Stable name stored in the file.
    pub fn name(self) -> &'static str {
        match self {
            MeasureKind::Distance => "Distance",
            MeasureKind::Perimeter => "Perimeter",
            MeasureKind::Area => "Area",
            MeasureKind::RectArea => "RectArea",
            MeasureKind::Radius => "Radius",
            MeasureKind::Angle => "Angle",
            MeasureKind::Count => "Count",
        }
    }

    /// Parse the stored name.
    pub fn parse(s: &str) -> Option<MeasureKind> {
        Some(match s {
            "Distance" => MeasureKind::Distance,
            "Perimeter" => MeasureKind::Perimeter,
            "Area" => MeasureKind::Area,
            "RectArea" => MeasureKind::RectArea,
            "Radius" => MeasureKind::Radius,
            "Angle" => MeasureKind::Angle,
            "Count" => MeasureKind::Count,
            _ => return None,
        })
    }

    /// Minimum number of points.
    pub fn min_points(self) -> usize {
        match self {
            MeasureKind::Count => 1,
            MeasureKind::Distance
            | MeasureKind::Perimeter
            | MeasureKind::RectArea
            | MeasureKind::Radius => 2,
            MeasureKind::Area | MeasureKind::Angle => 3,
        }
    }

    /// Display title.
    pub fn title(self) -> &'static str {
        match self {
            MeasureKind::Distance => "Distance",
            MeasureKind::Perimeter => "Length",
            MeasureKind::Area => "Area",
            MeasureKind::RectArea => "Rectangle",
            MeasureKind::Radius => "Radius",
            MeasureKind::Angle => "Angle",
            MeasureKind::Count => "Count",
        }
    }
}

/// A computed measurement.
#[derive(Clone, Debug, PartialEq)]
pub struct Measurement {
    /// Main numeric value in the scale's unit (length, area, degrees, or 1 for a count).
    pub value: f64,
    /// Unit text of `value` (`ft`, `ft²`, `°`, `count`).
    pub unit: String,
    /// Extra named values, e.g. `("Perimeter", 44.0, "ft")`.
    pub extra: Vec<(String, f64, String)>,
    /// Text drawn next to the measurement.
    pub label: String,
}

fn dist(a: Point, b: Point) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// Polygon area (shoelace), absolute value, in square points.
pub fn polygon_area(pts: &[Point]) -> f64 {
    if pts.len() < 3 {
        return 0.0;
    }
    let mut s = 0.0;
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        s += a.x * b.y - b.x * a.y;
    }
    s.abs() / 2.0
}

/// Path length in points; `closed` adds the closing segment.
pub fn path_length(pts: &[Point], closed: bool) -> f64 {
    let mut l: f64 = pts.windows(2).map(|w| dist(w[0], w[1])).sum();
    if closed && pts.len() > 2 {
        l += dist(pts[pts.len() - 1], pts[0]);
    }
    l
}

/// Angle at `v` between arms to `a` and `b`, in degrees (0..=180).
pub fn angle_degrees(a: Point, v: Point, b: Point) -> f64 {
    let (ux, uy, wx, wy) = (a.x - v.x, a.y - v.y, b.x - v.x, b.y - v.y);
    let (lu, lw) = (ux.hypot(uy), wx.hypot(wy));
    if lu < 1e-12 || lw < 1e-12 {
        return 0.0;
    }
    ((ux * wx + uy * wy) / (lu * lw))
        .clamp(-1.0, 1.0)
        .acos()
        .to_degrees()
}

/// Compute a measurement of `kind` over `points` (PDF user space) at `scale`.
pub fn measure(kind: MeasureKind, points: &[Point], scale: &Scale) -> Result<Measurement> {
    if points.len() > MAX_POINTS {
        return Err(EngineError::LimitExceeded(
            "too many points in one measurement".into(),
        ));
    }
    if points.len() < kind.min_points() {
        return Err(EngineError::InvalidArgument(format!(
            "{} needs at least {} points",
            kind.title(),
            kind.min_points()
        )));
    }
    if points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return Err(EngineError::InvalidArgument("non-finite coordinate".into()));
    }
    let u = scale.unit.abbr();
    let upp = scale.units_per_pt;
    let len = |pts: f64| pts * upp;
    let area = |pts2: f64| pts2 * upp * upp;
    Ok(match kind {
        MeasureKind::Distance => {
            let d = dist(points[0], points[1]);
            Measurement {
                value: len(d),
                unit: u.into(),
                extra: vec![
                    (
                        "ΔX".into(),
                        len((points[1].x - points[0].x).abs()),
                        u.into(),
                    ),
                    (
                        "ΔY".into(),
                        len((points[1].y - points[0].y).abs()),
                        u.into(),
                    ),
                ],
                label: scale.length_text(d),
            }
        }
        MeasureKind::Perimeter => {
            let l = path_length(points, false);
            Measurement {
                value: len(l),
                unit: u.into(),
                extra: vec![("Segments".into(), (points.len() - 1) as f64, String::new())],
                label: scale.length_text(l),
            }
        }
        MeasureKind::Area => {
            let a = polygon_area(points);
            let p = path_length(points, true);
            Measurement {
                value: area(a),
                unit: format!("{u}²"),
                extra: vec![("Perimeter".into(), len(p), u.into())],
                label: scale.area_text(a),
            }
        }
        MeasureKind::RectArea => {
            let (w, h) = (
                (points[1].x - points[0].x).abs(),
                (points[1].y - points[0].y).abs(),
            );
            Measurement {
                value: area(w * h),
                unit: format!("{u}²"),
                extra: vec![
                    ("Width".into(), len(w), u.into()),
                    ("Height".into(), len(h), u.into()),
                    ("Perimeter".into(), len(2.0 * (w + h)), u.into()),
                ],
                label: scale.area_text(w * h),
            }
        }
        MeasureKind::Radius => {
            let r = dist(points[0], points[1]);
            Measurement {
                value: len(r),
                unit: u.into(),
                extra: vec![
                    ("Diameter".into(), len(2.0 * r), u.into()),
                    (
                        "Circumference".into(),
                        len(2.0 * std::f64::consts::PI * r),
                        u.into(),
                    ),
                    (
                        "Area".into(),
                        area(std::f64::consts::PI * r * r),
                        format!("{u}²"),
                    ),
                ],
                label: format!("r = {}", scale.length_text(r)),
            }
        }
        MeasureKind::Angle => {
            let a = angle_degrees(points[0], points[1], points[2]);
            Measurement {
                value: a,
                unit: "°".into(),
                extra: Vec::new(),
                label: format!("{a:.1}°"),
            }
        }
        MeasureKind::Count => Measurement {
            value: 1.0,
            unit: "count".into(),
            extra: Vec::new(),
            label: String::new(),
        },
    })
}

/// Measurement data attached to an annotation.
#[derive(Clone, Debug, PartialEq)]
pub struct MeasureData {
    /// What is measured.
    pub kind: MeasureKind,
    /// Scale used when the measurement was made.
    pub scale: Scale,
    /// Count category (empty otherwise).
    pub category: String,
    /// Text shown beside the shape.
    pub label: String,
}

/// Pick the vertices a measurement of `kind` is computed from, given the annotation shape.
pub fn points_of(kind: MeasureKind, shape: &AnnotationKind) -> Option<Vec<Point>> {
    Some(match (kind, shape) {
        (MeasureKind::Distance, AnnotationKind::Line { start, end, .. }) => vec![*start, *end],
        (MeasureKind::Perimeter | MeasureKind::Angle, AnnotationKind::PolyLine { points, .. }) => {
            points.clone()
        }
        (MeasureKind::Area, AnnotationKind::Polygon { points }) => points.clone(),
        (MeasureKind::RectArea, AnnotationKind::Rectangle { rect }) => {
            vec![Point::new(rect.x0, rect.y0), Point::new(rect.x1, rect.y1)]
        }
        (MeasureKind::Radius, AnnotationKind::Ellipse { rect }) => {
            let r = rect.abs();
            let c = Point::new((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0);
            vec![c, Point::new(r.x1, c.y)]
        }
        (MeasureKind::Count, AnnotationKind::Ellipse { rect }) => {
            let r = rect.abs();
            vec![Point::new((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0)]
        }
        _ => return None,
    })
}

/// Anchor (user space) for the label of a measurement.
pub fn label_anchor(kind: MeasureKind, pts: &[Point]) -> Point {
    match kind {
        MeasureKind::Distance | MeasureKind::RectArea | MeasureKind::Radius => {
            let (a, b) = (pts[0], pts[1]);
            Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)
        }
        MeasureKind::Perimeter => {
            // Midpoint of the longest segment.
            let mut best = (0.0, pts[0], pts[pts.len().saturating_sub(1)]);
            for w in pts.windows(2) {
                let d = dist(w[0], w[1]);
                if d >= best.0 {
                    best = (d, w[0], w[1]);
                }
            }
            Point::new((best.1.x + best.2.x) / 2.0, (best.1.y + best.2.y) / 2.0)
        }
        MeasureKind::Area => {
            let n = pts.len() as f64;
            Point::new(
                pts.iter().map(|p| p.x).sum::<f64>() / n,
                pts.iter().map(|p| p.y).sum::<f64>() / n,
            )
        }
        MeasureKind::Angle => {
            let v = pts[1];
            Point::new(v.x + 14.0, v.y + 10.0)
        }
        MeasureKind::Count => pts[0],
    }
}

/// Build the annotation spec for a measurement (shape + data). `pts` are user-space vertices.
pub fn spec_for(
    kind: MeasureKind,
    pts: &[Point],
    scale: &Scale,
    category: &str,
) -> Result<AnnotationSpec> {
    let m = measure(kind, pts, scale)?;
    let shape = match kind {
        MeasureKind::Distance => AnnotationKind::Line {
            start: pts[0],
            end: pts[1],
            start_ending: annot::LineEnding::OpenArrow,
            end_ending: annot::LineEnding::OpenArrow,
        },
        MeasureKind::Perimeter | MeasureKind::Angle => AnnotationKind::PolyLine {
            points: pts.to_vec(),
            start_ending: annot::LineEnding::None,
            end_ending: annot::LineEnding::None,
        },
        MeasureKind::Area => AnnotationKind::Polygon {
            points: pts.to_vec(),
        },
        MeasureKind::RectArea => AnnotationKind::Rectangle {
            rect: Rect::new(pts[0].x, pts[0].y, pts[1].x, pts[1].y).abs(),
        },
        MeasureKind::Radius => {
            let r = dist(pts[0], pts[1]);
            AnnotationKind::Ellipse {
                rect: Rect::new(pts[0].x - r, pts[0].y - r, pts[0].x + r, pts[0].y + r),
            }
        }
        MeasureKind::Count => AnnotationKind::Ellipse {
            rect: Rect::new(
                pts[0].x - 6.0,
                pts[0].y - 6.0,
                pts[0].x + 6.0,
                pts[0].y + 6.0,
            ),
        },
    };
    let mut spec = AnnotationSpec::new(shape);
    spec.border_width = 1.5;
    spec.color = if kind == MeasureKind::Count {
        annot::Rgb(0.85, 0.15, 0.1)
    } else {
        annot::Rgb(0.0, 0.45, 0.85)
    };
    if kind == MeasureKind::Count {
        spec.fill = Some(annot::Rgb(0.85, 0.15, 0.1));
    } else if matches!(
        kind,
        MeasureKind::Area | MeasureKind::RectArea | MeasureKind::Radius
    ) {
        spec.fill = Some(annot::Rgb(0.6, 0.8, 1.0));
        spec.opacity = 0.8;
    }
    spec.subject = format!("Measurement: {}", kind.title());
    spec.contents = match kind {
        MeasureKind::Count => category.to_string(),
        _ => m.label.clone(),
    };
    spec.measure = Some(MeasureData {
        kind,
        scale: scale.clone(),
        category: category.to_string(),
        label: m.label,
    });
    Ok(spec)
}

/// Recompute the label (and stored scale) of an existing measurement spec at a new scale.
pub fn rescaled(spec: &AnnotationSpec, scale: &Scale) -> Option<AnnotationSpec> {
    let md = spec.measure.as_ref()?;
    if md.kind == MeasureKind::Count || md.kind == MeasureKind::Angle {
        return None;
    }
    let pts = points_of(md.kind, &spec.kind)?;
    let m = measure(md.kind, &pts, scale).ok()?;
    let mut s = spec.clone();
    if s.contents == md.label {
        s.contents = m.label.clone();
    }
    s.measure = Some(MeasureData {
        kind: md.kind,
        scale: scale.clone(),
        category: md.category.clone(),
        label: m.label,
    });
    Some(s)
}

// -------------------------------------------------------------------------------------------
// PDF persistence of per-annotation measurement data
// -------------------------------------------------------------------------------------------

fn number_format(unit: &str, c: f64, precision: u8) -> Dictionary {
    dictionary! {
        "Type" => "NumberFormat",
        "U" => text_obj(unit),
        "C" => real(c),
        "F" => "D",
        "D" => 10i64.pow(u32::from(precision.min(6))),
    }
}

/// `(standard /Measure dictionary, private /BergMeasure dictionary, /IT name)`.
pub(crate) fn to_pdf(m: &MeasureData) -> (Dictionary, Dictionary, Option<&'static str>) {
    let s = &m.scale;
    let u = s.unit.abbr();
    let mut measure = dictionary! {
        "Type" => "Measure",
        "Subtype" => "RL",
        "R" => text_obj(&s.text),
    };
    let x = vec![Object::Dictionary(number_format(
        u,
        s.units_per_pt,
        s.precision,
    ))];
    measure.set("X", Object::Array(x.clone()));
    measure.set("D", Object::Array(x));
    measure.set(
        "A",
        Object::Array(vec![Object::Dictionary(number_format(
            &format!("sq {u}"),
            s.units_per_pt * s.units_per_pt,
            s.precision,
        ))]),
    );
    let private = dictionary! {
        "K" => text_obj(m.kind.name()),
        "Cat" => text_obj(&m.category),
        "Label" => text_obj(&m.label),
        "Unit" => text_obj(u),
        "UPP" => text_obj(&format!("{}", s.units_per_pt)),
        "Text" => text_obj(&s.text),
        "Cal" => s.calibrated,
        "Prec" => i64::from(s.precision),
    };
    let it = match m.kind {
        MeasureKind::Distance => Some("LineDimension"),
        MeasureKind::Perimeter => Some("PolyLineDimension"),
        MeasureKind::Area => Some("PolygonDimension"),
        _ => None,
    };
    (measure, private, it)
}

/// Scale factors are stored as decimal text so they round-trip exactly (PDF reals are f32).
fn read_upp(doc: &Document, d: &Dictionary) -> Option<f64> {
    let t = text_at(doc, d, b"UPP");
    t.parse::<f64>()
        .ok()
        .or_else(|| objutil::dict_num(doc, d, b"UPP"))
}

fn text_at(doc: &Document, d: &Dictionary, k: &[u8]) -> String {
    d.get(k)
        .ok()
        .and_then(|o| objutil::deref(doc, o))
        .and_then(|o| o.as_str().ok())
        .map(objutil::decode_text_string)
        .unwrap_or_default()
}

/// Read the private measurement dictionary of an annotation, if present and valid.
pub(crate) fn from_pdf(doc: &Document, annot: &Dictionary) -> Option<MeasureData> {
    let d = objutil::dict_dict(doc, annot, b"BergMeasure")?;
    let kind = MeasureKind::parse(&text_at(doc, d, b"K"))?;
    let unit = Unit::parse(&text_at(doc, d, b"Unit"))?;
    let upp = read_upp(doc, d)?;
    if !upp.is_finite() || upp <= 0.0 {
        return None;
    }
    let calibrated = d
        .get(b"Cal")
        .ok()
        .and_then(|o| o.as_bool().ok())
        .unwrap_or(true);
    let precision = objutil::dict_num(doc, d, b"Prec").map_or(2, |p| p.clamp(0.0, 6.0) as u8);
    Some(MeasureData {
        kind,
        scale: Scale {
            unit,
            units_per_pt: upp,
            text: text_at(doc, d, b"Text"),
            calibrated,
            precision,
        },
        category: text_at(doc, d, b"Cat"),
        label: text_at(doc, d, b"Label"),
    })
}

// -------------------------------------------------------------------------------------------
// Scale registry (document / page / region)
// -------------------------------------------------------------------------------------------

/// A region of a page with its own scale (e.g. a detail drawn at a different scale).
#[derive(Clone, Debug, PartialEq)]
pub struct ScaleRegion {
    /// Page.
    pub page: PageId,
    /// Region in user space.
    pub rect: Rect,
    /// Scale inside the region.
    pub scale: Scale,
}

/// Where a resolved scale came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleSource {
    /// A region on the page.
    Region,
    /// A page-wide scale.
    Page,
    /// The document default.
    Document,
    /// Nothing calibrated.
    None,
}

/// All scales set in a document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScaleSet {
    /// Document default.
    pub document: Option<Scale>,
    /// Per-page scales.
    pub pages: BTreeMap<PageId, Scale>,
    /// Regions (smallest containing region wins).
    pub regions: Vec<ScaleRegion>,
}

impl ScaleSet {
    /// The scale that applies to `page` at `at` (or page-wide when `None`).
    pub fn resolve(&self, page: PageId, at: Option<Point>) -> (Scale, ScaleSource) {
        if let Some(p) = at {
            let best = self
                .regions
                .iter()
                .filter(|r| r.page == page && r.rect.abs().contains(p))
                .min_by(|a, b| {
                    let (aa, ab) = (a.rect.abs().area(), b.rect.abs().area());
                    aa.partial_cmp(&ab).unwrap_or(std::cmp::Ordering::Equal)
                });
            if let Some(r) = best {
                return (r.scale.clone(), ScaleSource::Region);
            }
        }
        if let Some(s) = self.pages.get(&page) {
            return (s.clone(), ScaleSource::Page);
        }
        if let Some(s) = &self.document {
            return (s.clone(), ScaleSource::Document);
        }
        (Scale::uncalibrated(), ScaleSource::None)
    }
}

fn scale_to_dict(s: &Scale) -> Dictionary {
    dictionary! {
        "Unit" => text_obj(s.unit.abbr()),
        "UPP" => text_obj(&format!("{}", s.units_per_pt)),
        "Text" => text_obj(&s.text),
        "Cal" => s.calibrated,
        "Prec" => i64::from(s.precision),
    }
}

fn scale_from_dict(doc: &Document, d: &Dictionary) -> Option<Scale> {
    let unit = Unit::parse(&text_at(doc, d, b"Unit"))?;
    let upp = read_upp(doc, d)?;
    if !upp.is_finite() || upp <= 0.0 {
        return None;
    }
    Some(Scale {
        unit,
        units_per_pt: upp,
        text: text_at(doc, d, b"Text"),
        calibrated: d
            .get(b"Cal")
            .ok()
            .and_then(|o| o.as_bool().ok())
            .unwrap_or(true),
        precision: objutil::dict_num(doc, d, b"Prec").map_or(2, |p| p.clamp(0.0, 6.0) as u8),
    })
}

fn catalog_id(doc: &Document) -> Result<ObjectId> {
    Ok(doc.trailer.get(b"Root").and_then(Object::as_reference)?)
}

/// Read the scale registry stored in the catalog.
pub fn read_scales(doc: &PdfDocument) -> ScaleSet {
    let d = doc.lopdf();
    let mut set = ScaleSet::default();
    let Some(cat) = catalog_id(d).ok().and_then(|c| d.get_dictionary(c).ok()) else {
        return set;
    };
    let Some(reg) = objutil::dict_dict(d, cat, b"BergScales") else {
        return set;
    };
    set.document = objutil::dict_dict(d, reg, b"Doc").and_then(|s| scale_from_dict(d, s));
    let page_ids: Vec<PageId> = doc.page_ids().unwrap_or_default();
    let known = |o: &Object| -> Option<PageId> {
        let r = o.as_reference().ok()?;
        page_ids.iter().copied().find(|p| p.0 == r)
    };
    if let Some(a) = objutil::dict_array(d, reg, b"Pages") {
        for e in a {
            let Some(ed) = objutil::deref(d, e).and_then(|o| o.as_dict().ok()) else {
                continue;
            };
            if let (Some(p), Some(s)) = (
                ed.get(b"P").ok().and_then(known),
                objutil::dict_dict(d, ed, b"S").and_then(|s| scale_from_dict(d, s)),
            ) {
                set.pages.insert(p, s);
            }
        }
    }
    if let Some(a) = objutil::dict_array(d, reg, b"Regions") {
        for e in a {
            let Some(ed) = objutil::deref(d, e).and_then(|o| o.as_dict().ok()) else {
                continue;
            };
            if let (Some(p), Some(r), Some(s)) = (
                ed.get(b"P").ok().and_then(known),
                ed.get(b"R").ok().and_then(|r| objutil::rect(d, r)),
                objutil::dict_dict(d, ed, b"S").and_then(|s| scale_from_dict(d, s)),
            ) {
                set.regions.push(ScaleRegion {
                    page: p,
                    rect: r,
                    scale: s,
                });
            }
        }
    }
    set
}

/// Store the scale registry in the catalog (replacing any previous one).
pub fn write_scales(tx: &mut Tx<'_>, set: &ScaleSet) -> Result<()> {
    let cat = catalog_id(tx.doc())?;
    let mut reg = Dictionary::new();
    if let Some(s) = &set.document {
        reg.set("Doc", Object::Dictionary(scale_to_dict(s)));
    }
    let pages: Vec<Object> = set
        .pages
        .iter()
        .map(|(p, s)| {
            Object::Dictionary(dictionary! { "P" => reference(p.0), "S" => scale_to_dict(s) })
        })
        .collect();
    if !pages.is_empty() {
        reg.set("Pages", Object::Array(pages));
    }
    let regions: Vec<Object> = set
        .regions
        .iter()
        .map(|r| {
            let rc = r.rect.abs();
            Object::Dictionary(dictionary! {
                "P" => reference(r.page.0),
                "R" => num_array(&[rc.x0, rc.y0, rc.x1, rc.y1]),
                "S" => scale_to_dict(&r.scale),
            })
        })
        .collect();
    if !regions.is_empty() {
        reg.set("Regions", Object::Array(regions));
    }
    let cd = tx.dict_mut(cat)?;
    if reg.is_empty() {
        cd.remove(b"BergScales");
    } else {
        cd.set("BergScales", Object::Dictionary(reg));
    }
    Ok(())
}

/// Apply `scale` to every existing measurement annotation on `page` (distances, lengths, areas,
/// rectangles and radii; counts and angles are scale-independent). Returns how many changed.
pub fn rescale_page(tx: &mut Tx<'_>, page: PageId, scale: &Scale) -> Result<usize> {
    let infos = annot::read_annotations_doc(tx.doc(), page);
    let mut n = 0;
    for a in infos {
        let Some(spec) = a.spec else { continue };
        if let Some(new) = rescaled(&spec, scale) {
            annot::update_annotation(tx, a.id, &new)?;
            n += 1;
        }
    }
    Ok(n)
}

// -------------------------------------------------------------------------------------------
// Reporting
// -------------------------------------------------------------------------------------------

/// One measurement in the report.
#[derive(Clone, Debug, PartialEq)]
pub struct MeasureRow {
    /// Page (1-based).
    pub page_number: usize,
    /// Page id.
    pub page: PageId,
    /// Annotation id.
    pub id: AnnotId,
    /// Kind.
    pub kind: MeasureKind,
    /// Count category (counts only).
    pub category: String,
    /// Main value in `unit`.
    pub value: f64,
    /// Unit text.
    pub unit: String,
    /// Extra values `name = value unit`.
    pub extra: Vec<(String, f64, String)>,
    /// Scale description.
    pub scale: String,
    /// Whether the scale was calibrated.
    pub calibrated: bool,
    /// Comment text.
    pub contents: String,
    /// Author.
    pub author: String,
}

/// Collect every Berg measurement in the document, recomputed from geometry so reports
/// always match what is drawn.
pub fn collect_measurements(doc: &PdfDocument) -> Vec<MeasureRow> {
    let mut rows = Vec::new();
    let Ok(pages) = doc.page_ids() else {
        return rows;
    };
    for (i, page) in pages.iter().enumerate() {
        for a in annot::read_annotations(doc, *page) {
            let Some(spec) = &a.spec else { continue };
            let Some(md) = &spec.measure else { continue };
            let Some(pts) = points_of(md.kind, &spec.kind) else {
                continue;
            };
            let Ok(m) = measure(md.kind, &pts, &md.scale) else {
                continue;
            };
            rows.push(MeasureRow {
                page_number: i + 1,
                page: *page,
                id: a.id,
                kind: md.kind,
                category: md.category.clone(),
                value: m.value,
                unit: m.unit,
                extra: m.extra,
                scale: md.scale.text.clone(),
                calibrated: md.scale.calibrated,
                contents: a.contents.clone(),
                author: a.author.clone(),
            });
        }
    }
    rows
}

/// Per-category count totals.
#[derive(Clone, Debug, PartialEq)]
pub struct CountRow {
    /// Category name.
    pub category: String,
    /// Counts per page `(page number, n)`.
    pub per_page: Vec<(usize, u32)>,
    /// Total.
    pub total: u32,
}

/// Summarise count markers by category.
pub fn count_summary(rows: &[MeasureRow]) -> Vec<CountRow> {
    let mut map: BTreeMap<String, BTreeMap<usize, u32>> = BTreeMap::new();
    for r in rows.iter().filter(|r| r.kind == MeasureKind::Count) {
        *map.entry(r.category.clone())
            .or_default()
            .entry(r.page_number)
            .or_insert(0) += 1;
    }
    map.into_iter()
        .map(|(category, pages)| {
            let total = pages.values().sum();
            CountRow {
                category,
                per_page: pages.into_iter().collect(),
                total,
            }
        })
        .collect()
}

/// Quote a CSV field; neutralises spreadsheet formula injection (`=`, `+`, `-`, `@`, tab, CR)
/// by prefixing an apostrophe, as recommended for untrusted text opened in spreadsheets.
pub fn csv_field(s: &str) -> String {
    let mut t = s.to_string();
    if t.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        t.insert(0, '\'');
    }
    if t.contains([',', '"', '\n', '\r']) || t != s {
        format!("\"{}\"", t.replace('"', "\"\""))
    } else {
        t
    }
}

/// Render the report as CSV: one row per measurement, then a count summary block.
pub fn to_csv(rows: &[MeasureRow]) -> String {
    let mut out =
        String::from("Page,Type,Category,Value,Unit,Details,Scale,Calibrated,Comment,Author\n");
    for r in rows {
        let details = r
            .extra
            .iter()
            .map(|(n, v, u)| {
                if u.is_empty() {
                    format!("{n}={v:.3}")
                } else {
                    format!("{n}={v:.3} {u}")
                }
            })
            .collect::<Vec<_>>()
            .join("; ");
        out.push_str(&format!(
            "{},{},{},{:.4},{},{},{},{},{},{}\n",
            r.page_number,
            r.kind.title(),
            csv_field(&r.category),
            r.value,
            csv_field(&r.unit),
            csv_field(&details),
            csv_field(&r.scale),
            if r.calibrated { "yes" } else { "no" },
            csv_field(&r.contents),
            csv_field(&r.author),
        ));
    }
    let counts = count_summary(rows);
    if !counts.is_empty() {
        out.push_str("\nCount summary\nCategory,Total,Per page\n");
        for c in counts {
            let per = c
                .per_page
                .iter()
                .map(|(p, n)| format!("p{p}:{n}"))
                .collect::<Vec<_>>()
                .join("; ");
            out.push_str(&format!(
                "{},{},{}\n",
                csv_field(&c.category),
                c.total,
                csv_field(&per)
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn architectural_scale_one_inch_is_ten_feet() {
        let s = Scale::from_ratio(1.0, Unit::In, 10.0, Unit::Ft).unwrap();
        // 72 pt on paper = 10 ft.
        let m = measure(MeasureKind::Distance, &[p(0.0, 0.0), p(72.0, 0.0)], &s).unwrap();
        assert!((m.value - 10.0).abs() < 1e-9, "{m:?}");
        assert_eq!(m.label, "10.00 ft");
    }

    #[test]
    fn one_to_hundred_in_metres() {
        // 1:100 — 1 mm on paper is 100 mm = 0.1 m; 100 mm of paper = 10 m.
        let s = Scale::from_one_to(100.0, Unit::M).unwrap();
        let paper_100mm = 100.0 * Unit::Mm.points();
        let m = measure(
            MeasureKind::Distance,
            &[p(0.0, 0.0), p(paper_100mm, 0.0)],
            &s,
        )
        .unwrap();
        assert!((m.value - 10.0).abs() < 1e-9, "{m:?}");
    }

    #[test]
    fn calibration_from_known_length() {
        let s = Scale::from_calibration(200.0, 5.0, Unit::M).unwrap();
        let m = measure(MeasureKind::Distance, &[p(10.0, 10.0), p(10.0, 110.0)], &s).unwrap();
        assert!((m.value - 2.5).abs() < 1e-12);
        assert!(Scale::from_calibration(0.0, 5.0, Unit::M).is_err());
        assert!(Scale::from_calibration(10.0, -1.0, Unit::M).is_err());
        assert!(Scale::from_calibration(f64::NAN, 1.0, Unit::M).is_err());
    }

    #[test]
    fn area_and_perimeter_of_a_rectangle_and_l_shape() {
        let s = Scale::from_calibration(100.0, 10.0, Unit::M).unwrap(); // 0.1 m per pt
        let r = measure(MeasureKind::RectArea, &[p(0.0, 0.0), p(200.0, 100.0)], &s).unwrap();
        assert!(
            (r.value - 200.0).abs() < 1e-9,
            "20 m x 10 m = 200 m²: {r:?}"
        );
        let per = r.extra.iter().find(|e| e.0 == "Perimeter").unwrap();
        assert!((per.1 - 60.0).abs() < 1e-9);
        // L-shape: 20x10 minus 10x5 notch = 150 m².
        let l = [
            p(0.0, 0.0),
            p(200.0, 0.0),
            p(200.0, 50.0),
            p(100.0, 50.0),
            p(100.0, 100.0),
            p(0.0, 100.0),
        ];
        let a = measure(MeasureKind::Area, &l, &s).unwrap();
        assert!((a.value - 150.0).abs() < 1e-9, "{a:?}");
        assert_eq!(a.unit, "m²");
        // Orientation does not matter.
        let mut rev = l.to_vec();
        rev.reverse();
        assert!((polygon_area(&rev) - polygon_area(&l)).abs() < 1e-9);
    }

    #[test]
    fn polyline_length_radius_and_angle() {
        let s = Scale::from_calibration(10.0, 10.0, Unit::M).unwrap(); // 1 m per pt
        let pl = measure(
            MeasureKind::Perimeter,
            &[p(0.0, 0.0), p(3.0, 4.0), p(3.0, 14.0)],
            &s,
        )
        .unwrap();
        assert!((pl.value - 15.0).abs() < 1e-9);
        let c = measure(MeasureKind::Radius, &[p(5.0, 5.0), p(5.0, 7.0)], &s).unwrap();
        assert!((c.value - 2.0).abs() < 1e-9);
        let circ = c.extra.iter().find(|e| e.0 == "Circumference").unwrap().1;
        assert!((circ - 4.0 * std::f64::consts::PI).abs() < 1e-9);
        let ang = measure(
            MeasureKind::Angle,
            &[p(1.0, 0.0), p(0.0, 0.0), p(0.0, 1.0)],
            &s,
        )
        .unwrap();
        assert!((ang.value - 90.0).abs() < 1e-9);
        assert_eq!(ang.label, "90.0°");
        let straight = angle_degrees(p(-1.0, 0.0), p(0.0, 0.0), p(1.0, 0.0));
        assert!((straight - 180.0).abs() < 1e-9);
        assert_eq!(angle_degrees(p(0.0, 0.0), p(0.0, 0.0), p(1.0, 0.0)), 0.0);
    }

    #[test]
    fn too_few_or_invalid_points_are_rejected() {
        let s = Scale::uncalibrated();
        assert!(measure(MeasureKind::Area, &[p(0.0, 0.0), p(1.0, 1.0)], &s).is_err());
        assert!(measure(MeasureKind::Distance, &[p(0.0, f64::NAN), p(1.0, 1.0)], &s).is_err());
        let many = vec![p(0.0, 0.0); MAX_POINTS + 1];
        assert!(matches!(
            measure(MeasureKind::Perimeter, &many, &s),
            Err(EngineError::LimitExceeded(_))
        ));
    }

    #[test]
    fn region_beats_page_beats_document() {
        let page = PageId((3, 0));
        let other = PageId((4, 0));
        let doc_s = Scale::from_one_to(100.0, Unit::M).unwrap();
        let page_s = Scale::from_one_to(50.0, Unit::M).unwrap();
        let reg_s = Scale::from_one_to(10.0, Unit::M).unwrap();
        let mut set = ScaleSet {
            document: Some(doc_s.clone()),
            ..Default::default()
        };
        assert_eq!(set.resolve(page, None).1, ScaleSource::Document);
        set.pages.insert(page, page_s.clone());
        set.regions.push(ScaleRegion {
            page,
            rect: Rect::new(0.0, 0.0, 100.0, 100.0),
            scale: reg_s.clone(),
        });
        assert_eq!(
            set.resolve(page, Some(p(10.0, 10.0))),
            (reg_s, ScaleSource::Region)
        );
        assert_eq!(
            set.resolve(page, Some(p(500.0, 10.0))),
            (page_s, ScaleSource::Page)
        );
        assert_eq!(set.resolve(other, None), (doc_s, ScaleSource::Document));
        assert_eq!(ScaleSet::default().resolve(page, None).1, ScaleSource::None);
        assert!(!Scale::uncalibrated().calibrated);
    }

    #[test]
    fn csv_neutralises_formula_injection_and_quotes() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("=HYPERLINK(\"x\")"), "\"'=HYPERLINK(\"\"x\"\")\"");
        assert_eq!(csv_field("-5"), "\"'-5\"");
        assert_eq!(csv_field("@SUM(1)"), "\"'@SUM(1)\"");
    }

    #[test]
    fn units_roundtrip() {
        for u in Unit::ALL {
            assert_eq!(Unit::parse(u.abbr()), Some(u));
        }
        assert!((Unit::In.points() - 72.0).abs() < 1e-9);
        assert_eq!(Unit::parse("furlong"), None);
    }
}
