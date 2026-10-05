//! Standard PDF annotations with appearance streams.
//!
//! Annotations are persisted as ordinary PDF annotation dictionaries with `/AP /N` form
//! XObjects generated here, so other viewers display them without any sidecar data. Fonts
//! used by FreeText/stamp appearances are embedded subsets (see [`crate::fontembed`]).

use crate::doc::{PageId, PdfDocument, Tx};
use crate::error::{EngineError, Result};
use crate::fontembed::{BundledFace, FontBuilder, FontStyle, hex_string};
use crate::geom::{Point, Quad, Rect};
use crate::objutil::{self, fmt_num, name, num_array, reference, text_obj};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};

/// Annotation object id (stable for the life of the annotation, including undo/redo).
pub type AnnotId = ObjectId;

/// RGB colour with components in 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb(pub f32, pub f32, pub f32);

impl Rgb {
    /// Pure black.
    pub const BLACK: Rgb = Rgb(0.0, 0.0, 0.0);
    /// Highlight yellow.
    pub const YELLOW: Rgb = Rgb(1.0, 0.92, 0.0);
    /// Red.
    pub const RED: Rgb = Rgb(0.85, 0.1, 0.1);

    fn ops(self) -> String {
        format!(
            "{} {} {}",
            fmt_num(self.0.into()),
            fmt_num(self.1.into()),
            fmt_num(self.2.into())
        )
    }

    fn array(self) -> Object {
        num_array(&[self.0.into(), self.1.into(), self.2.into()])
    }

    fn from_array(doc: &Document, o: &Object) -> Option<Rgb> {
        let a = objutil::deref(doc, o)?.as_array().ok()?;
        let v: Vec<f64> = a.iter().filter_map(|x| objutil::num(doc, x)).collect();
        match v.len() {
            1 => Some(Rgb(v[0] as f32, v[0] as f32, v[0] as f32)),
            3 => Some(Rgb(v[0] as f32, v[1] as f32, v[2] as f32)),
            4 => {
                let k = 1.0 - v[3];
                Some(Rgb(
                    ((1.0 - v[0]) * k) as f32,
                    ((1.0 - v[1]) * k) as f32,
                    ((1.0 - v[2]) * k) as f32,
                ))
            }
            _ => None,
        }
    }
}

/// Line ending styles for lines/polylines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnding {
    /// No ending.
    None,
    /// Open (two-stroke) arrow.
    OpenArrow,
    /// Filled arrow.
    ClosedArrow,
}

impl LineEnding {
    fn pdf_name(self) -> &'static str {
        match self {
            LineEnding::None => "None",
            LineEnding::OpenArrow => "OpenArrow",
            LineEnding::ClosedArrow => "ClosedArrow",
        }
    }
    fn parse(n: &[u8]) -> LineEnding {
        match n {
            b"OpenArrow" => LineEnding::OpenArrow,
            b"ClosedArrow" => LineEnding::ClosedArrow,
            _ => LineEnding::None,
        }
    }
}

/// Border style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderStyle {
    /// Solid line.
    Solid,
    /// Dashed line.
    Dashed,
}

/// Text alignment inside FreeText.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    /// Left aligned.
    Left,
    /// Centred.
    Center,
    /// Right aligned.
    Right,
}

/// Kind-specific data.
#[derive(Clone, Debug, PartialEq)]
pub enum AnnotationKind {
    /// Text highlight over glyph quads (PDF user space).
    Highlight { quads: Vec<Quad> },
    /// Underline.
    Underline { quads: Vec<Quad> },
    /// Strike-through.
    StrikeOut { quads: Vec<Quad> },
    /// Squiggly underline.
    Squiggly { quads: Vec<Quad> },
    /// Sticky note anchored with its top-left corner at `pos`.
    Note { pos: Point },
    /// Text box, optionally with a callout leader (`callout`: 2 or 3 points, tip first).
    FreeText {
        rect: Rect,
        font_size: f64,
        text_color: Rgb,
        align: Align,
        /// Bundled font family, weight and slant.
        font: FontStyle,
        callout: Option<Vec<Point>>,
    },
    /// Rectangle.
    Rectangle { rect: Rect },
    /// Ellipse.
    Ellipse { rect: Rect },
    /// Line or arrow.
    Line {
        start: Point,
        end: Point,
        start_ending: LineEnding,
        end_ending: LineEnding,
    },
    /// Closed polygon.
    Polygon { points: Vec<Point> },
    /// Open polyline.
    PolyLine {
        points: Vec<Point>,
        start_ending: LineEnding,
        end_ending: LineEnding,
    },
    /// Freehand strokes.
    Ink { strokes: Vec<Vec<Point>> },
    /// Text stamp (e.g. "APPROVED").
    StampText { rect: Rect, label: String },
}

/// A complete, regenerable annotation description.
#[derive(Clone, Debug, PartialEq)]
pub struct AnnotationSpec {
    /// Kind-specific data.
    pub kind: AnnotationKind,
    /// `/Contents` — the comment text (and the visible text of FreeText).
    pub contents: String,
    /// `/T` author.
    pub author: String,
    /// `/Subj` subject.
    pub subject: String,
    /// Stroke / markup colour.
    pub color: Rgb,
    /// Interior / background colour.
    pub fill: Option<Rgb>,
    /// Opacity 0..=1.
    pub opacity: f64,
    /// Border / line width in points.
    pub border_width: f64,
    /// Border style.
    pub border_style: BorderStyle,
    /// PDF date string for `/M` (`None` = now).
    pub modified: Option<String>,
    /// `/NM` unique name (`None` = generated).
    pub name: Option<String>,
    /// Measurement data (`Some` for measurement annotations).
    pub measure: Option<crate::measure::MeasureData>,
    /// Text rotation for FreeText and text stamps: degrees counter-clockwise in page user space, a
    /// multiple of 90. A page shown with `/Rotate 90` needs 90 to read upright; see [`upright_for`].
    pub rotation: i32,
}

impl AnnotationSpec {
    /// Sensible defaults for a kind.
    pub fn new(kind: AnnotationKind) -> Self {
        let color = match &kind {
            AnnotationKind::Highlight { .. } => Rgb::YELLOW,
            AnnotationKind::Note { .. } => Rgb(1.0, 0.85, 0.2),
            _ => Rgb::RED,
        };
        Self {
            kind,
            contents: String::new(),
            author: String::new(),
            subject: String::new(),
            color,
            fill: None,
            opacity: 1.0,
            border_width: 1.5,
            border_style: BorderStyle::Solid,
            modified: None,
            name: None,
            measure: None,
            rotation: 0,
        }
    }

    fn subtype(&self) -> &'static str {
        match &self.kind {
            AnnotationKind::Highlight { .. } => "Highlight",
            AnnotationKind::Underline { .. } => "Underline",
            AnnotationKind::StrikeOut { .. } => "StrikeOut",
            AnnotationKind::Squiggly { .. } => "Squiggly",
            AnnotationKind::Note { .. } => "Text",
            AnnotationKind::FreeText { .. } => "FreeText",
            AnnotationKind::Rectangle { .. } => "Square",
            AnnotationKind::Ellipse { .. } => "Circle",
            AnnotationKind::Line { .. } => "Line",
            AnnotationKind::Polygon { .. } => "Polygon",
            AnnotationKind::PolyLine { .. } => "PolyLine",
            AnnotationKind::Ink { .. } => "Ink",
            AnnotationKind::StampText { .. } => "Stamp",
        }
    }

    /// Annotation `/Rect` in user space (includes the measurement label, if any).
    pub fn rect(&self) -> Rect {
        let shape = self.shape_rect();
        match measure_label_box(self) {
            Some(lb) => shape.union(lb.rect),
            None => shape,
        }
    }

    fn shape_rect(&self) -> Rect {
        let lw = self.border_width.max(0.0);
        match &self.kind {
            AnnotationKind::Highlight { quads }
            | AnnotationKind::Underline { quads }
            | AnnotationKind::StrikeOut { quads }
            | AnnotationKind::Squiggly { quads } => union_all(quads.iter().map(|q| q.bounds()))
                .unwrap_or(Rect::ZERO)
                .inflate(1.0, 1.0),
            AnnotationKind::Note { pos } => Rect::new(pos.x, pos.y - 20.0, pos.x + 20.0, pos.y),
            AnnotationKind::FreeText { rect, callout, .. } => {
                let mut r = rect.abs();
                if let Some(c) = callout {
                    for p in c {
                        r = r.union_pt(*p);
                    }
                }
                r
            }
            AnnotationKind::Rectangle { rect }
            | AnnotationKind::Ellipse { rect }
            | AnnotationKind::StampText { rect, .. } => rect.abs(),
            AnnotationKind::Line {
                start,
                end,
                start_ending,
                end_ending,
            } => {
                let pad = lw / 2.0
                    + if *start_ending != LineEnding::None || *end_ending != LineEnding::None {
                        arrow_len(lw)
                    } else {
                        0.0
                    };
                Rect::from_points(*start, *end)
                    .abs()
                    .inflate(pad + 0.5, pad + 0.5)
            }
            AnnotationKind::Polygon { points } | AnnotationKind::PolyLine { points, .. } => {
                bounds_of(points).inflate(
                    lw / 2.0 + arrow_len(lw) + 0.5,
                    lw / 2.0 + arrow_len(lw) + 0.5,
                )
            }
            AnnotationKind::Ink { strokes } => {
                let all: Vec<Point> = strokes.iter().flatten().copied().collect();
                bounds_of(&all).inflate(lw / 2.0 + 0.5, lw / 2.0 + 0.5)
            }
        }
    }
}

fn arrow_len(lw: f64) -> f64 {
    (lw * 5.0).max(8.0)
}

fn union_all(it: impl Iterator<Item = Rect>) -> Option<Rect> {
    it.reduce(|a, b| a.union(b))
}

fn bounds_of(points: &[Point]) -> Rect {
    let mut it = points.iter();
    let Some(first) = it.next() else {
        return Rect::ZERO;
    };
    let mut r = Rect::new(first.x, first.y, first.x, first.y);
    for p in it {
        r = r.union_pt(*p);
    }
    r
}

/// Read model for the UI.
#[derive(Clone, Debug)]
pub struct AnnotationInfo {
    /// Object id.
    pub id: AnnotId,
    /// Subtype name, e.g. `Highlight`.
    pub subtype: String,
    /// `/Rect`.
    pub rect: Rect,
    /// `/Contents`.
    pub contents: String,
    /// `/T`.
    pub author: String,
    /// `/M`.
    pub modified: String,
    /// Fully parsed, regenerable description when the subtype is supported.
    pub spec: Option<AnnotationSpec>,
}

// ---------------------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------------------

/// Add an annotation to a page and return its id.
pub fn add_annotation(tx: &mut Tx<'_>, page: PageId, spec: &AnnotationSpec) -> Result<AnnotId> {
    let (mut dict, ap) = build(tx, spec)?;
    let ap_id = tx.add(Object::Stream(ap));
    dict.set("AP", dictionary! { "N" => ap_id });
    dict.set("P", reference(page.0));
    let id = tx.add(Object::Dictionary(dict));
    append_annot_ref(tx, page.0, id)?;
    Ok(id)
}

/// Replace an existing annotation's description and regenerate its appearance.
/// The object id (and therefore selection/undo identity) is preserved.
pub fn update_annotation(tx: &mut Tx<'_>, id: AnnotId, spec: &AnnotationSpec) -> Result<()> {
    let old = tx.doc().get_dictionary(id)?.clone();
    let (mut dict, ap) = build(tx, spec)?;
    // Preserve identity / linkage keys we do not regenerate.
    for key in [&b"P"[..], b"NM", b"Popup", b"IRT", b"RT"] {
        if let Ok(v) = old.get(key) {
            dict.set(key.to_vec(), v.clone());
        }
    }
    if spec.name.is_none() {
        // Keep the existing /NM when the caller did not supply one.
        if let Ok(v) = old.get(b"NM") {
            dict.set("NM", v.clone());
        }
    }
    let ap_id = tx.add(Object::Stream(ap));
    dict.set("AP", dictionary! { "N" => ap_id });
    tx.set(id, Object::Dictionary(dict));
    Ok(())
}

/// Translate an annotation by `(dx, dy)` in user space without regenerating its
/// appearance (the appearance maps onto the moved `/Rect`).
pub fn move_annotation(tx: &mut Tx<'_>, id: AnnotId, dx: f64, dy: f64) -> Result<()> {
    let d = tx.dict_mut(id)?;
    shift_array(d, b"Rect", dx, dy, 2);
    shift_array(d, b"QuadPoints", dx, dy, 2);
    shift_array(d, b"Vertices", dx, dy, 2);
    shift_array(d, b"L", dx, dy, 2);
    shift_array(d, b"CL", dx, dy, 2);
    if let Ok(Object::Array(lists)) = d.get_mut(b"InkList") {
        for l in lists.iter_mut() {
            if let Object::Array(a) = l {
                shift_vec(a, dx, dy);
            }
        }
    }
    Ok(())
}

fn shift_array(d: &mut Dictionary, key: &[u8], dx: f64, dy: f64, _stride: usize) {
    if let Ok(Object::Array(a)) = d.get_mut(key) {
        shift_vec(a, dx, dy);
    }
}

fn shift_vec(a: &mut [Object], dx: f64, dy: f64) {
    for (i, o) in a.iter_mut().enumerate() {
        let v = match o {
            Object::Integer(v) => *v as f64,
            Object::Real(v) => f64::from(*v),
            _ => continue,
        };
        *o = objutil::real(v + if i % 2 == 0 { dx } else { dy });
    }
}

/// Remove an annotation from its page (the object itself is dropped from the file on a
/// full rewrite; in incremental saves it simply becomes unreferenced).
pub fn delete_annotation(tx: &mut Tx<'_>, page: PageId, id: AnnotId) -> Result<()> {
    remove_annot_ref(tx, page.0, id)?;
    // Also drop any Popup child.
    let popup = tx
        .doc()
        .get_dictionary(id)
        .ok()
        .and_then(|d| d.get(b"Popup").ok().and_then(|p| p.as_reference().ok()));
    if let Some(p) = popup {
        let _ = remove_annot_ref(tx, page.0, p);
    }
    Ok(())
}

fn annots_array_location(doc: &Document, page: ObjectId) -> Result<AnnotsLoc> {
    let d = doc.get_dictionary(page)?;
    Ok(match d.get(b"Annots") {
        Ok(Object::Reference(r)) => AnnotsLoc::Indirect(*r),
        Ok(Object::Array(_)) => AnnotsLoc::Direct,
        _ => AnnotsLoc::Missing,
    })
}

enum AnnotsLoc {
    Direct,
    Indirect(ObjectId),
    Missing,
}

pub(crate) fn append_annot_ref(tx: &mut Tx<'_>, page: ObjectId, annot: ObjectId) -> Result<()> {
    match annots_array_location(tx.doc(), page)? {
        AnnotsLoc::Missing => {
            tx.dict_mut(page)?.set("Annots", vec![reference(annot)]);
        }
        AnnotsLoc::Direct => {
            if let Ok(Object::Array(a)) = tx.dict_mut(page)?.get_mut(b"Annots") {
                a.push(reference(annot));
            }
        }
        AnnotsLoc::Indirect(r) => {
            if let Object::Array(a) = tx.object_mut(r)? {
                a.push(reference(annot));
            } else {
                return Err(EngineError::Malformed("/Annots is not an array".into()));
            }
        }
    }
    Ok(())
}

fn remove_annot_ref(tx: &mut Tx<'_>, page: ObjectId, annot: ObjectId) -> Result<()> {
    let keep = |o: &Object| o.as_reference().ok() != Some(annot);
    match annots_array_location(tx.doc(), page)? {
        AnnotsLoc::Missing => {}
        AnnotsLoc::Direct => {
            if let Ok(Object::Array(a)) = tx.dict_mut(page)?.get_mut(b"Annots") {
                a.retain(keep);
            }
        }
        AnnotsLoc::Indirect(r) => {
            if let Object::Array(a) = tx.object_mut(r)? {
                a.retain(keep);
            }
        }
    }
    Ok(())
}

/// Insert an annotation reference at a specific index (used by undo of delete via deltas,
/// and by z-order operations).
pub fn annotation_ids(doc: &PdfDocument, page: PageId) -> Vec<AnnotId> {
    let d = doc.lopdf();
    let Ok(pd) = d.get_dictionary(page.0) else {
        return Vec::new();
    };
    let Some(arr) = objutil::dict_array(d, pd, b"Annots") else {
        return Vec::new();
    };
    arr.iter().filter_map(|o| o.as_reference().ok()).collect()
}

/// Read every annotation on a page.
pub fn read_annotations(doc: &PdfDocument, page: PageId) -> Vec<AnnotationInfo> {
    read_annotations_doc(doc.lopdf(), page)
}

/// Read every annotation on a page from a raw document.
pub fn read_annotations_doc(doc: &Document, page: PageId) -> Vec<AnnotationInfo> {
    let Ok(pd) = doc.get_dictionary(page.0) else {
        return Vec::new();
    };
    let Some(arr) = objutil::dict_array(doc, pd, b"Annots") else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|o| o.as_reference().ok())
        .filter_map(|id| read_annotation(doc, id))
        .collect()
}

/// Read one annotation.
pub fn read_annotation(doc: &Document, id: AnnotId) -> Option<AnnotationInfo> {
    let d = doc.get_dictionary(id).ok()?;
    let subtype = String::from_utf8_lossy(objutil::dict_name(doc, d, b"Subtype")?).into_owned();
    let rect = d.get(b"Rect").ok().and_then(|r| objutil::rect(doc, r))?;
    let text = |k: &[u8]| {
        d.get(k)
            .ok()
            .and_then(|o| objutil::deref(doc, o))
            .and_then(|o| o.as_str().ok())
            .map(objutil::decode_text_string)
            .unwrap_or_default()
    };
    Some(AnnotationInfo {
        id,
        subtype,
        rect,
        contents: text(b"Contents"),
        author: text(b"T"),
        modified: text(b"M"),
        spec: parse_spec(doc, d),
    })
}

// ---------------------------------------------------------------------------------------
// Appearance building
// ---------------------------------------------------------------------------------------

struct Ap {
    content: String,
    resources: Dictionary,
}

fn gs_resources(opacity: f64, multiply: bool) -> Dictionary {
    let mut gs = dictionary! {
        "Type" => "ExtGState",
        "CA" => objutil::real(opacity),
        "ca" => objutil::real(opacity),
    };
    gs.set("BM", name(if multiply { "Multiply" } else { "Normal" }));
    dictionary! { "ExtGState" => dictionary! { "GS" => gs } }
}

fn dash_op(style: BorderStyle, lw: f64) -> String {
    match style {
        BorderStyle::Solid => "[] 0 d\n".to_string(),
        BorderStyle::Dashed => format!("[{0} {0}] 0 d\n", fmt_num((lw * 3.0).max(3.0))),
    }
}

fn pt(p: Point) -> String {
    format!("{} {}", fmt_num(p.x), fmt_num(p.y))
}

fn build(tx: &mut Tx<'_>, spec: &AnnotationSpec) -> Result<(Dictionary, Stream)> {
    let rect = spec.rect();
    let lw = spec.border_width.max(0.0);
    let opacity = spec.opacity.clamp(0.0, 1.0);
    let mut dict = Dictionary::new();
    dict.set("Type", name("Annot"));
    dict.set("Subtype", name(spec.subtype()));
    dict.set("Rect", num_array(&[rect.x0, rect.y0, rect.x1, rect.y1]));
    dict.set("F", 4i64);
    dict.set("C", spec.color.array());
    if let Some(f) = spec.fill {
        dict.set("IC", f.array());
    }
    dict.set("CA", objutil::real(opacity));
    dict.set("Contents", text_obj(&spec.contents));
    if !spec.author.is_empty() {
        dict.set("T", text_obj(&spec.author));
    }
    if !spec.subject.is_empty() {
        dict.set("Subj", text_obj(&spec.subject));
    }
    dict.set(
        "M",
        Object::string_literal(spec.modified.clone().unwrap_or_else(now_pdf_date)),
    );
    dict.set(
        "NM",
        text_obj(&spec.name.clone().unwrap_or_else(|| unique_name(tx))),
    );

    let mut bs = dictionary! { "Type" => "Border", "W" => objutil::real(lw) };
    bs.set(
        "S",
        name(if spec.border_style == BorderStyle::Dashed {
            "D"
        } else {
            "S"
        }),
    );
    if spec.border_style == BorderStyle::Dashed {
        let d = (lw * 3.0).max(3.0);
        bs.set("D", num_array(&[d, d]));
    }

    let mut ap = Ap {
        content: String::new(),
        resources: Dictionary::new(),
    };
    let c = spec.color.ops();

    match &spec.kind {
        AnnotationKind::Highlight { quads } => {
            dict.set("QuadPoints", quad_points(quads));
            ap.resources = gs_resources(opacity, true);
            ap.content.push_str(&format!("/GS gs\n{c} rg\n"));
            for q in quads {
                let p = q.0;
                ap.content.push_str(&format!(
                    "{} m {} l {} l {} l h f\n",
                    pt(p[0]),
                    pt(p[1]),
                    pt(p[2]),
                    pt(p[3])
                ));
            }
        }
        AnnotationKind::Underline { quads } | AnnotationKind::StrikeOut { quads } => {
            dict.set("QuadPoints", quad_points(quads));
            ap.resources = gs_resources(opacity, false);
            let strike = matches!(spec.kind, AnnotationKind::StrikeOut { .. });
            ap.content.push_str(&format!("/GS gs\n{c} RG\n"));
            for q in quads {
                let p = q.0;
                let h = dist(p[0], p[3]);
                let w = (h * 0.07).max(0.6);
                let (a, b) = if strike {
                    (mid(p[0], p[3]), mid(p[1], p[2]))
                } else {
                    (lerp(p[0], p[3], 0.12), lerp(p[1], p[2], 0.12))
                };
                ap.content
                    .push_str(&format!("{} w {} m {} l S\n", fmt_num(w), pt(a), pt(b)));
            }
        }
        AnnotationKind::Squiggly { quads } => {
            dict.set("QuadPoints", quad_points(quads));
            ap.resources = gs_resources(opacity, false);
            ap.content.push_str(&format!("/GS gs\n{c} RG\n0.6 w\n"));
            for q in quads {
                let p = q.0;
                let h = dist(p[0], p[3]);
                let amp = (h * 0.06).max(0.7);
                let step = amp * 2.0;
                let len = dist(p[0], p[1]);
                let n = (len / step).floor().max(1.0) as usize;
                let ux = (p[1].x - p[0].x) / len.max(1e-9);
                let uy = (p[1].y - p[0].y) / len.max(1e-9);
                let (nx, ny) = (-uy, ux);
                let base = lerp(p[0], p[3], 0.1);
                let mut s = format!("{} m ", pt(base));
                for i in 1..=n {
                    let t = i as f64 * step;
                    let off = if i % 2 == 1 { amp } else { 0.0 };
                    s.push_str(&format!(
                        "{} {} l ",
                        fmt_num(base.x + ux * t + nx * off),
                        fmt_num(base.y + uy * t + ny * off)
                    ));
                }
                s.push_str("S\n");
                ap.content.push_str(&s);
            }
        }
        AnnotationKind::Note { .. } => {
            dict.set("Name", name("Note"));
            dict.set("F", 28i64);
            dict.set("Open", false);
            ap.resources = gs_resources(opacity, false);
            let (x0, y0, x1, y1) = (rect.x0, rect.y0, rect.x1, rect.y1);
            ap.content.push_str(&format!(
                "/GS gs\n{} rg 0.25 0.2 0.0 RG 1 w\n{} {} {} {} re B\n0.25 0.2 0 RG 1 w\n",
                spec.color.ops(),
                fmt_num(x0 + 1.0),
                fmt_num(y0 + 1.0),
                fmt_num(x1 - x0 - 2.0),
                fmt_num(y1 - y0 - 2.0)
            ));
            for i in 0..3 {
                let y = y1 - 5.5 - f64::from(i) * 4.0;
                ap.content.push_str(&format!(
                    "{} {} m {} {} l S\n",
                    fmt_num(x0 + 4.0),
                    fmt_num(y),
                    fmt_num(x1 - 4.0),
                    fmt_num(y)
                ));
            }
        }
        AnnotationKind::FreeText {
            rect: r,
            font_size,
            text_color,
            align,
            font,
            callout,
        } => {
            build_freetext(
                tx,
                spec,
                *r,
                *font_size,
                *text_color,
                *align,
                *font,
                callout.as_deref(),
                &mut dict,
                &mut ap,
                &bs,
            )?;
        }
        AnnotationKind::Rectangle { rect: r } | AnnotationKind::Ellipse { rect: r } => {
            dict.set("BS", bs);
            if lw == 0.0 {
                dict.set("BS", dictionary! { "W" => 0i64 });
            }
            ap.resources = gs_resources(opacity, false);
            let r = r.abs();
            let h = lw / 2.0;
            let inset = Rect::new(r.x0 + h, r.y0 + h, r.x1 - h, r.y1 - h);
            ap.content.push_str("/GS gs\n");
            ap.content.push_str(&format!("{} w\n{c} RG\n", fmt_num(lw)));
            ap.content.push_str(&dash_op(spec.border_style, lw));
            if let Some(f) = spec.fill {
                ap.content.push_str(&format!("{} rg\n", f.ops()));
            }
            let paint = match (spec.fill.is_some(), lw > 0.0) {
                (true, true) => "B",
                (true, false) => "f",
                (false, true) => "S",
                (false, false) => "n",
            };
            if matches!(spec.kind, AnnotationKind::Rectangle { .. }) {
                ap.content.push_str(&format!(
                    "{} {} {} {} re {paint}\n",
                    fmt_num(inset.x0),
                    fmt_num(inset.y0),
                    fmt_num(inset.width().max(0.0)),
                    fmt_num(inset.height().max(0.0))
                ));
            } else {
                ap.content.push_str(&ellipse_path(inset));
                ap.content.push_str(&format!("{paint}\n"));
            }
        }
        AnnotationKind::Line {
            start,
            end,
            start_ending,
            end_ending,
        } => {
            dict.set("BS", bs);
            dict.set("L", num_array(&[start.x, start.y, end.x, end.y]));
            dict.set(
                "LE",
                vec![name(start_ending.pdf_name()), name(end_ending.pdf_name())],
            );
            ap.resources = gs_resources(opacity, false);
            ap.content.push_str("/GS gs\n");
            ap.content.push_str(&stroke_setup(spec, &c, lw));
            ap.content.push_str(&path_with_endings(
                &[*start, *end],
                false,
                *start_ending,
                *end_ending,
                lw,
                spec.color,
            ));
        }
        AnnotationKind::PolyLine {
            points,
            start_ending,
            end_ending,
        } => {
            dict.set("BS", bs);
            dict.set("Vertices", points_array(points));
            dict.set(
                "LE",
                vec![name(start_ending.pdf_name()), name(end_ending.pdf_name())],
            );
            ap.resources = gs_resources(opacity, false);
            ap.content.push_str("/GS gs\n");
            ap.content.push_str(&stroke_setup(spec, &c, lw));
            ap.content.push_str(&path_with_endings(
                points,
                false,
                *start_ending,
                *end_ending,
                lw,
                spec.color,
            ));
        }
        AnnotationKind::Polygon { points } => {
            dict.set("BS", bs);
            dict.set("Vertices", points_array(points));
            ap.resources = gs_resources(opacity, false);
            ap.content.push_str("/GS gs\n");
            ap.content.push_str(&stroke_setup(spec, &c, lw));
            if let Some(f) = spec.fill {
                ap.content.push_str(&format!("{} rg\n", f.ops()));
            }
            ap.content.push_str(&polyline_path(points));
            ap.content.push_str(match (spec.fill.is_some(), lw > 0.0) {
                (true, true) => "h B\n",
                (true, false) => "h f\n",
                _ => "h S\n",
            });
        }
        AnnotationKind::Ink { strokes } => {
            dict.set("BS", bs);
            dict.set(
                "InkList",
                Object::Array(strokes.iter().map(|s| points_array(s)).collect()),
            );
            ap.resources = gs_resources(opacity, false);
            ap.content.push_str("/GS gs\n1 J 1 j\n");
            ap.content.push_str(&stroke_setup(spec, &c, lw));
            for s in strokes {
                if s.len() == 1 {
                    // A dot: zero-length segment with round caps.
                    ap.content
                        .push_str(&format!("{} m {} l S\n", pt(s[0]), pt(s[0])));
                } else {
                    ap.content.push_str(&polyline_path(s));
                    ap.content.push_str("S\n");
                }
            }
        }
        AnnotationKind::StampText { rect: r, label } => {
            dict.set("Name", name("Draft"));
            dict.set("Subj", text_obj(label));
            let r = r.abs();
            let mut fb = FontBuilder::new(BundledFace::SansBold)?;
            let missing = fb.missing_chars(label);
            if !missing.is_empty() {
                return Err(EngineError::Unsupported(format!(
                    "stamp text contains characters not in the bundled font: {missing:?}"
                )));
            }
            let rot = norm_rot(spec.rotation);
            if rot != 0 {
                dict.set("BergRot", i64::from(rot));
            }
            let (bw_, bh_) = local_size(r, rot);
            let lr = Rect::new(0.0, 0.0, bw_, bh_);
            let pad = 4.0;
            let wem = (fb.text_width(label, 1.0)).max(0.1);
            let fs = ((lr.width() - 2.0 * pad) / wem)
                .min((lr.height() - 2.0 * pad) * 0.8)
                .max(1.0);
            let tw = wem * fs;
            let bw = lw.max(1.0);
            let x = lr.x0 + (lr.width() - tw) / 2.0;
            let y = lr.y0 + (lr.height() - fs * (fb.ascent_em() + fb.descent_em())) / 2.0;
            let codes = fb.encode_str(label)?;
            let font_id = fb.finish(tx)?;
            ap.resources = gs_resources(opacity, false);
            ap.resources.set("Font", dictionary! { "F1" => font_id });
            ap.content.push_str(&format!(
                "/GS gs\nq\n{}{c} RG {c} rg\n{} w\n{} {} {} {} re S\nBT /F1 {} Tf {} {} Td {} Tj ET\nQ\n",
                cm_op(rotated_frame(r, rot)),
                fmt_num(bw),
                fmt_num(lr.x0 + bw / 2.0),
                fmt_num(lr.y0 + bw / 2.0),
                fmt_num(lr.width() - bw),
                fmt_num(lr.height() - bw),
                fmt_num(fs),
                fmt_num(x),
                fmt_num(y),
                hex_string(&codes)
            ));
        }
    }

    if let Some(md) = &spec.measure {
        let (measure, private, it) = crate::measure::to_pdf(md);
        dict.set("Measure", Object::Dictionary(measure));
        dict.set("BergMeasure", Object::Dictionary(private));
        if let Some(it) = it {
            dict.set("IT", name(it));
        }
        if let Some(lb) = measure_label_box(spec) {
            let mut fb = FontBuilder::new(BundledFace::Sans)?;
            let codes = fb.encode_str(&lb.text)?;
            let font_id = fb.finish(tx)?;
            ap.resources.set("Font", dictionary! { "FM" => font_id });
            ap.content.push_str(&format!(
                "q 1 1 1 rg {} {} {} {} re f 0.2 0.2 0.2 RG 0.5 w {} {} {} {} re S BT /FM {} Tf 0 0 0 rg {} {} Td {} Tj ET Q\n",
                fmt_num(lb.rect.x0), fmt_num(lb.rect.y0), fmt_num(lb.rect.width()), fmt_num(lb.rect.height()),
                fmt_num(lb.rect.x0), fmt_num(lb.rect.y0), fmt_num(lb.rect.width()), fmt_num(lb.rect.height()),
                fmt_num(lb.font_size), fmt_num(lb.origin.x), fmt_num(lb.origin.y), hex_string(&codes)
            ));
        }
    }
    let stream = form_xobject(rect, ap);
    Ok((dict, stream))
}

fn stroke_setup(spec: &AnnotationSpec, c: &str, lw: f64) -> String {
    format!(
        "{} w\n{c} RG\n{}",
        fmt_num(lw),
        dash_op(spec.border_style, lw)
    )
}

fn form_xobject(bbox: Rect, ap: Ap) -> Stream {
    let mut d = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Form",
        "FormType" => 1i64,
        "BBox" => num_array(&[bbox.x0, bbox.y0, bbox.x1, bbox.y1]),
    };
    d.set("Resources", Object::Dictionary(ap.resources));
    let mut s = Stream::new(d, crate::fontembed::zlib(ap.content.as_bytes()));
    s.dict.set("Filter", name("FlateDecode"));
    s.allows_compression = false;
    s
}

fn quad_points(quads: &[Quad]) -> Object {
    // De-facto order used by Acrobat and followed by other viewers: TL, TR, BL, BR.
    let mut v = Vec::with_capacity(quads.len() * 8);
    for q in quads {
        let p = q.0;
        for pt in [p[3], p[2], p[0], p[1]] {
            v.push(pt.x);
            v.push(pt.y);
        }
    }
    num_array(&v)
}

fn points_array(p: &[Point]) -> Object {
    let v: Vec<f64> = p.iter().flat_map(|p| [p.x, p.y]).collect();
    num_array(&v)
}

fn polyline_path(points: &[Point]) -> String {
    let mut s = String::new();
    for (i, p) in points.iter().enumerate() {
        s.push_str(&pt(*p));
        s.push_str(if i == 0 { " m " } else { " l " });
    }
    s
}

fn ellipse_path(r: Rect) -> String {
    const K: f64 = 0.552_284_749_8;
    let (cx, cy) = ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0);
    let (rx, ry) = (r.width().max(0.0) / 2.0, r.height().max(0.0) / 2.0);
    let f = fmt_num;
    format!(
        "{} {} m\n{} {} {} {} {} {} c\n{} {} {} {} {} {} c\n{} {} {} {} {} {} c\n{} {} {} {} {} {} c\nh\n",
        f(cx + rx),
        f(cy),
        f(cx + rx),
        f(cy + K * ry),
        f(cx + K * rx),
        f(cy + ry),
        f(cx),
        f(cy + ry),
        f(cx - K * rx),
        f(cy + ry),
        f(cx - rx),
        f(cy + K * ry),
        f(cx - rx),
        f(cy),
        f(cx - rx),
        f(cy - K * ry),
        f(cx - K * rx),
        f(cy - ry),
        f(cx),
        f(cy - ry),
        f(cx + K * rx),
        f(cy - ry),
        f(cx + rx),
        f(cy - K * ry),
        f(cx + rx),
        f(cy),
    )
}

/// Build the stroke path with arrowheads; shortens the line under closed arrowheads.
fn path_with_endings(
    points: &[Point],
    close: bool,
    start: LineEnding,
    end: LineEnding,
    lw: f64,
    color: Rgb,
) -> String {
    let mut pts = points.to_vec();
    let len = arrow_len(lw);
    let mut heads = String::new();
    if pts.len() >= 2 {
        if end != LineEnding::None {
            let n = pts.len();
            let (tip, from) = (pts[n - 1], pts[n - 2]);
            let (h, base) = arrowhead(tip, from, len, end, color);
            heads.push_str(&h);
            if end == LineEnding::ClosedArrow {
                pts[n - 1] = base;
            }
        }
        if start != LineEnding::None {
            let (tip, from) = (pts[0], pts[1]);
            let (h, base) = arrowhead(tip, from, len, start, color);
            heads.push_str(&h);
            if start == LineEnding::ClosedArrow {
                pts[0] = base;
            }
        }
    }
    let mut s = polyline_path(&pts);
    s.push_str(if close { "h S\n" } else { "S\n" });
    s.push_str(&heads);
    s
}

fn arrowhead(tip: Point, from: Point, len: f64, kind: LineEnding, color: Rgb) -> (String, Point) {
    let d = dist(tip, from).max(1e-9);
    let (ux, uy) = ((tip.x - from.x) / d, (tip.y - from.y) / d);
    let base = Point::new(tip.x - ux * len, tip.y - uy * len);
    let half = len * 0.38;
    let a = Point::new(base.x - uy * half, base.y + ux * half);
    let b = Point::new(base.x + uy * half, base.y - ux * half);
    let s = match kind {
        LineEnding::ClosedArrow => format!(
            "{} rg {} m {} l {} l h f\n",
            color.ops(),
            pt(tip),
            pt(a),
            pt(b)
        ),
        _ => format!("{} m {} l {} m {} l S\n", pt(a), pt(tip), pt(tip), pt(b)),
    };
    (s, base)
}

fn dist(a: Point, b: Point) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}
fn mid(a: Point, b: Point) -> Point {
    Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)
}
fn lerp(a: Point, b: Point, t: f64) -> Point {
    Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

/// Rotation (counter-clockwise degrees in user space) that makes text read upright for a page
/// that is displayed rotated clockwise by `shown_cw` degrees.
pub fn upright_for(shown_cw: i64) -> i32 {
    shown_cw.rem_euclid(360) as i32
}

fn norm_rot(rot: i32) -> i32 {
    let r = rot.rem_euclid(360);
    (r / 90) * 90
}

/// Size of a text block in its own upright frame (width and height swap for 90/270).
pub fn local_size(r: Rect, rot: i32) -> (f64, f64) {
    let r = r.abs();
    if norm_rot(rot) % 180 == 0 {
        (r.width(), r.height())
    } else {
        (r.height(), r.width())
    }
}

/// Matrix `[a b c d e f]` taking a block's own upright frame (origin bottom-left, x along the
/// text) to user space inside `r`, for text rotated `rot` degrees counter-clockwise.
pub fn rotated_frame(r: Rect, rot: i32) -> [f64; 6] {
    let r = r.abs();
    match norm_rot(rot) {
        90 => [0.0, 1.0, -1.0, 0.0, r.x1, r.y0],
        180 => [-1.0, 0.0, 0.0, -1.0, r.x1, r.y1],
        270 => [0.0, -1.0, 1.0, 0.0, r.x0, r.y1],
        _ => [1.0, 0.0, 0.0, 1.0, r.x0, r.y0],
    }
}

pub(crate) fn cm_op(m: [f64; 6]) -> String {
    format!(
        "{} {} {} {} {} {} cm\n",
        fmt_num(m[0]),
        fmt_num(m[1]),
        fmt_num(m[2]),
        fmt_num(m[3]),
        fmt_num(m[4]),
        fmt_num(m[5])
    )
}

/// `r` made tall enough (in the block's own frame) for `needed` points, keeping the top edge of
/// the text where it is.
pub fn grow_box(r: Rect, rot: i32, needed: f64) -> Rect {
    let mut r = r.abs();
    let (_, h) = local_size(r, rot);
    if needed <= h {
        return r;
    }
    match norm_rot(rot) {
        90 => r.x1 = r.x0 + needed,
        180 => r.y1 = r.y0 + needed,
        270 => r.x0 = r.x1 - needed,
        _ => r.y0 = r.y1 - needed,
    }
    r
}

/// Required height for FreeText content at a given width (for auto-sizing the box).
pub fn freetext_required_height(
    text: &str,
    font_size: f64,
    width: f64,
    font: FontStyle,
) -> Result<f64> {
    let fb = FontBuilder::new(BundledFace::Styled(font))?;
    let lines = wrap_lines(&fb, text, font_size, (width - 2.0 * FT_PAD).max(1.0));
    Ok(lines.len() as f64 * fb.line_height_em() * font_size + 2.0 * FT_PAD)
}

const FT_PAD: f64 = 3.0;

pub(crate) fn wrap_lines(fb: &FontBuilder, text: &str, fs: f64, max_w: f64) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut cur = String::new();
        for word in para.split(' ') {
            let candidate = if cur.is_empty() {
                word.to_string()
            } else {
                format!("{cur} {word}")
            };
            if fb.text_width(&candidate, fs) <= max_w || cur.is_empty() {
                cur = candidate;
                // Break overlong single words by characters.
                while fb.text_width(&cur, fs) > max_w && cur.chars().count() > 1 {
                    let mut cut = String::new();
                    let mut rest = cur.clone();
                    while let Some(c) = rest.chars().next() {
                        let next = format!("{cut}{c}");
                        if fb.text_width(&next, fs) > max_w && !cut.is_empty() {
                            break;
                        }
                        cut = next;
                        rest = rest.chars().skip(1).collect();
                    }
                    lines.push(cut);
                    cur = rest;
                    if cur.is_empty() {
                        break;
                    }
                }
            } else {
                lines.push(std::mem::take(&mut cur));
                cur = word.to_string();
            }
        }
        lines.push(cur);
    }
    lines
}

#[allow(clippy::too_many_arguments)]
fn build_freetext(
    tx: &mut Tx<'_>,
    spec: &AnnotationSpec,
    rect: Rect,
    font_size: f64,
    text_color: Rgb,
    align: Align,
    font: FontStyle,
    callout: Option<&[Point]>,
    dict: &mut Dictionary,
    ap: &mut Ap,
    bs: &Dictionary,
) -> Result<()> {
    let r = rect.abs();
    let lw = spec.border_width.max(0.0);
    let mut fb = FontBuilder::new(BundledFace::Styled(font))?;
    let missing = fb.missing_chars(&spec.contents);
    if !missing.is_empty() {
        return Err(EngineError::Unsupported(format!(
            "text contains characters that are not in {}: {missing:?}",
            font.family.title()
        )));
    }
    // Remember the choice so it survives saving and re-opening (the content stream alone
    // cannot tell us which bundled font was used).
    dict.set("BergFont", Object::string_literal(font.base_name()));
    dict.set(
        "DA",
        Object::string_literal(format!(
            "/Helv {} Tf {} rg",
            fmt_num(font_size),
            text_color.ops()
        )),
    );
    dict.set(
        "Q",
        match align {
            Align::Left => 0i64,
            Align::Center => 1,
            Align::Right => 2,
        },
    );
    dict.set("BS", bs.clone());
    dict.set("C", spec.fill.unwrap_or(Rgb(1.0, 1.0, 1.0)).array());
    if spec.fill.is_none() {
        dict.remove(b"IC");
        dict.remove(b"C");
    }
    dict.set(
        "IT",
        name(if callout.is_some() {
            "FreeTextCallout"
        } else {
            "FreeText"
        }),
    );
    if let Some(c) = callout {
        let v: Vec<f64> = c.iter().flat_map(|p| [p.x, p.y]).collect();
        dict.set("CL", num_array(&v));
        dict.set("LE", name("OpenArrow"));
    }
    // The box is laid out in its own upright frame (`lr`, origin bottom-left) and placed into user
    // space with a rotation matrix, so text reads upright on rotated pages.
    let rot = norm_rot(spec.rotation);
    if rot != 0 {
        dict.set("BergRot", i64::from(rot));
    }
    if callout.is_some() {
        // /Rect also covers the leader line; keep the text box itself for re-editing.
        dict.set("BergBox", num_array(&[r.x0, r.y0, r.x1, r.y1]));
    }
    let (bw, bh) = local_size(r, rot);
    let lr = Rect::new(0.0, 0.0, bw, bh);
    let frame = cm_op(rotated_frame(r, rot));
    let lines = wrap_lines(
        &fb,
        &spec.contents,
        font_size,
        (lr.width() - 2.0 * FT_PAD - lw).max(1.0),
    );
    let lh = fb.line_height_em() * font_size;
    let mut content = String::from("/GS gs\nq\n");
    content.push_str(&frame);
    if let Some(f) = spec.fill {
        content.push_str(&format!(
            "{} rg\n{} {} {} {} re f\n",
            f.ops(),
            fmt_num(lr.x0),
            fmt_num(lr.y0),
            fmt_num(lr.width()),
            fmt_num(lr.height())
        ));
    }
    if lw > 0.0 {
        content.push_str(&format!(
            "{} RG {} w\n{}{} {} {} {} re S\n",
            spec.color.ops(),
            fmt_num(lw),
            dash_op(spec.border_style, lw),
            fmt_num(lr.x0 + lw / 2.0),
            fmt_num(lr.y0 + lw / 2.0),
            fmt_num(lr.width() - lw),
            fmt_num(lr.height() - lw)
        ));
    }
    content.push_str("Q\n");
    if let Some(c) = callout.filter(|c| c.len() >= 2) {
        content.push_str(&format!(
            "{} RG {} w\n",
            spec.color.ops(),
            fmt_num(lw.max(0.75))
        ));
        let mut pts: Vec<Point> = c.to_vec();
        let (head, base) = arrowhead(
            pts[0],
            pts[1],
            arrow_len(lw.max(0.75)) * 0.8,
            LineEnding::OpenArrow,
            spec.color,
        );
        let _ = base;
        content.push_str(&polyline_path(&pts));
        content.push_str("S\n");
        content.push_str(&head);
        pts.clear();
    }
    content.push_str("q\n");
    content.push_str(&frame);
    content.push_str(&format!(
        "{} {} {} {} re W n\nBT\n/F1 {} Tf {} rg\n",
        fmt_num(lr.x0),
        fmt_num(lr.y0),
        fmt_num(lr.width()),
        fmt_num(lr.height()),
        fmt_num(font_size),
        text_color.ops()
    ));
    let mut cursor_x = 0.0;
    let mut first = true;
    let mut y = lr.y1 - FT_PAD - lw - fb.ascent_em() * font_size;
    for line in &lines {
        let w = fb.text_width(line, font_size);
        let x = match align {
            Align::Left => lr.x0 + FT_PAD + lw,
            Align::Center => lr.x0 + (lr.width() - w) / 2.0,
            Align::Right => lr.x1 - FT_PAD - lw - w,
        };
        let codes = fb.encode_str(line)?;
        // Td is relative to the start of the previous line.
        content.push_str(&format!(
            "{} {} Td\n",
            fmt_num(x - cursor_x),
            fmt_num(if first { y } else { -lh })
        ));
        if !codes.is_empty() {
            content.push_str(&format!("{} Tj\n", hex_string(&codes)));
        }
        cursor_x = x;
        first = false;
        y -= lh;
    }
    content.push_str("ET\nQ\n");
    let font_id = fb.finish(tx)?;
    ap.resources = gs_resources(spec.opacity.clamp(0.0, 1.0), false);
    ap.resources.set("Font", dictionary! { "F1" => font_id });
    ap.content = content;
    Ok(())
}

// ---------------------------------------------------------------------------------------
// Reading back into a spec
// ---------------------------------------------------------------------------------------

fn parse_spec(doc: &Document, d: &Dictionary) -> Option<AnnotationSpec> {
    let subtype = objutil::dict_name(doc, d, b"Subtype")?;
    let rect = d.get(b"Rect").ok().and_then(|r| objutil::rect(doc, r))?;
    let nums = |key: &[u8]| -> Option<Vec<f64>> {
        let a = objutil::dict_array(doc, d, key)?;
        Some(a.iter().filter_map(|o| objutil::num(doc, o)).collect())
    };
    let pts =
        |v: &[f64]| -> Vec<Point> { v.chunks_exact(2).map(|c| Point::new(c[0], c[1])).collect() };
    let quads = || -> Option<Vec<Quad>> {
        let v = nums(b"QuadPoints")?;
        Some(
            v.chunks_exact(8)
                .map(|c| {
                    // file order TL, TR, BL, BR -> internal BL, BR, TR, TL
                    Quad([
                        Point::new(c[4], c[5]),
                        Point::new(c[6], c[7]),
                        Point::new(c[2], c[3]),
                        Point::new(c[0], c[1]),
                    ])
                })
                .collect(),
        )
    };
    let endings = || -> (LineEnding, LineEnding) {
        match objutil::dict_array(doc, d, b"LE") {
            Some(a) if a.len() >= 2 => (
                objutil::deref(doc, &a[0])
                    .and_then(|o| o.as_name().ok())
                    .map_or(LineEnding::None, LineEnding::parse),
                objutil::deref(doc, &a[1])
                    .and_then(|o| o.as_name().ok())
                    .map_or(LineEnding::None, LineEnding::parse),
            ),
            _ => (LineEnding::None, LineEnding::None),
        }
    };
    let kind = match subtype {
        b"Highlight" => AnnotationKind::Highlight { quads: quads()? },
        b"Underline" => AnnotationKind::Underline { quads: quads()? },
        b"StrikeOut" => AnnotationKind::StrikeOut { quads: quads()? },
        b"Squiggly" => AnnotationKind::Squiggly { quads: quads()? },
        b"Text" => AnnotationKind::Note {
            pos: Point::new(rect.x0, rect.y1),
        },
        b"Square" => AnnotationKind::Rectangle { rect },
        b"Circle" => AnnotationKind::Ellipse { rect },
        b"Line" => {
            let l = nums(b"L")?;
            if l.len() < 4 {
                return None;
            }
            let (s, e) = endings();
            AnnotationKind::Line {
                start: Point::new(l[0], l[1]),
                end: Point::new(l[2], l[3]),
                start_ending: s,
                end_ending: e,
            }
        }
        b"Polygon" => AnnotationKind::Polygon {
            points: pts(&nums(b"Vertices")?),
        },
        b"PolyLine" => {
            let (s, e) = endings();
            AnnotationKind::PolyLine {
                points: pts(&nums(b"Vertices")?),
                start_ending: s,
                end_ending: e,
            }
        }
        b"Ink" => {
            let lists = objutil::dict_array(doc, d, b"InkList")?;
            let strokes = lists
                .iter()
                .filter_map(|l| objutil::deref(doc, l)?.as_array().ok())
                .map(|a| {
                    let v: Vec<f64> = a.iter().filter_map(|o| objutil::num(doc, o)).collect();
                    pts(&v)
                })
                .collect();
            AnnotationKind::Ink { strokes }
        }
        b"FreeText" => {
            let da = d
                .get(b"DA")
                .ok()
                .and_then(|o| objutil::deref(doc, o))
                .and_then(|o| o.as_str().ok())
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_default();
            let (fs, tc) = parse_da(&da);
            let align = match d.get(b"Q").ok().and_then(|o| o.as_i64().ok()) {
                Some(1) => Align::Center,
                Some(2) => Align::Right,
                _ => Align::Left,
            };
            let callout = nums(b"CL").map(|v| pts(&v)).filter(|c| c.len() >= 2);
            // With a leader, /Rect also covers the leader; the text box is kept separately.
            let rect = if callout.is_some() {
                nums(b"BergBox")
                    .filter(|b| b.len() == 4)
                    .map_or(rect, |b| Rect::new(b[0], b[1], b[2], b[3]))
            } else {
                rect
            };
            let font = d
                .get(b"BergFont")
                .ok()
                .and_then(|o| objutil::deref(doc, o))
                .and_then(|o| o.as_str().ok())
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .and_then(|n| FontStyle::from_base_name(&n))
                .unwrap_or_default();
            AnnotationKind::FreeText {
                rect,
                font_size: fs,
                text_color: tc,
                align,
                font,
                callout,
            }
        }
        b"Stamp" => {
            let subj = d
                .get(b"Subj")
                .ok()
                .and_then(|o| objutil::deref(doc, o))
                .and_then(|o| o.as_str().ok())
                .map(objutil::decode_text_string)?;
            AnnotationKind::StampText { rect, label: subj }
        }
        _ => return None,
    };
    let text = |k: &[u8]| {
        d.get(k)
            .ok()
            .and_then(|o| objutil::deref(doc, o))
            .and_then(|o| o.as_str().ok())
            .map(objutil::decode_text_string)
            .unwrap_or_default()
    };
    let color = d
        .get(b"C")
        .ok()
        .and_then(|o| Rgb::from_array(doc, o))
        .unwrap_or(Rgb::BLACK);
    let bs = objutil::dict_dict(doc, d, b"BS");
    let border_width = bs
        .and_then(|b| objutil::dict_num(doc, b, b"W"))
        .unwrap_or(1.0);
    let dashed = bs.and_then(|b| objutil::dict_name(doc, b, b"S")) == Some(b"D");
    let mut spec = AnnotationSpec::new(kind);
    spec.contents = text(b"Contents");
    spec.author = text(b"T");
    spec.subject = text(b"Subj");
    spec.color = color;
    spec.fill = d.get(b"IC").ok().and_then(|o| Rgb::from_array(doc, o));
    if matches!(spec.kind, AnnotationKind::FreeText { .. }) {
        spec.fill = d.get(b"C").ok().and_then(|o| Rgb::from_array(doc, o));
        spec.color = Rgb::BLACK;
        if let AnnotationKind::FreeText { text_color, .. } = &spec.kind {
            spec.color = *text_color;
        }
    }
    spec.opacity = objutil::dict_num(doc, d, b"CA").unwrap_or(1.0);
    spec.border_width = border_width;
    spec.border_style = if dashed {
        BorderStyle::Dashed
    } else {
        BorderStyle::Solid
    };
    spec.modified = Some(text(b"M")).filter(|s| !s.is_empty());
    spec.name = Some(text(b"NM")).filter(|s| !s.is_empty());
    spec.measure = crate::measure::from_pdf(doc, d)
        .filter(|m| crate::measure::points_of(m.kind, &spec.kind).is_some());
    spec.rotation = objutil::dict_num(doc, d, b"BergRot").map_or(0, |v| norm_rot(v as i32));
    Some(spec)
}

fn parse_da(da: &str) -> (f64, Rgb) {
    let toks: Vec<&str> = da.split_whitespace().collect();
    let mut fs = 12.0;
    let mut col = Rgb::BLACK;
    for (i, t) in toks.iter().enumerate() {
        if *t == "Tf" && i >= 1 {
            fs = toks[i - 1].parse().unwrap_or(12.0);
        }
        let n = |j: usize| {
            toks.get(i.wrapping_sub(j))
                .and_then(|s| s.parse::<f32>().ok())
        };
        match *t {
            "g" if i >= 1 => {
                if let Some(v) = n(1) {
                    col = Rgb(v, v, v);
                }
            }
            "rg" if i >= 3 => {
                if let (Some(r), Some(g), Some(b)) = (n(3), n(2), n(1)) {
                    col = Rgb(r, g, b);
                }
            }
            _ => {}
        }
    }
    (fs, col)
}

// ---------------------------------------------------------------------------------------
// Misc
// ---------------------------------------------------------------------------------------

/// Current UTC time as a PDF date string.
pub fn now_pdf_date() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    pdf_date_from_unix(secs)
}

/// Convert unix seconds to `D:YYYYMMDDHHmmSSZ`.
pub fn pdf_date_from_unix(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "D:{y:04}{m:02}{d:02}{:02}{:02}{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn unique_name(tx: &Tx<'_>) -> String {
    format!("berg-{}", tx.doc().max_id + 1)
}

impl AnnotationSpec {
    /// A copy moved by `(dx, dy)` in user space (used by duplicate/paste).
    pub fn translated(&self, dx: f64, dy: f64) -> AnnotationSpec {
        let mv = |p: &Point| Point::new(p.x + dx, p.y + dy);
        let mq = |q: &Quad| Quad([mv(&q.0[0]), mv(&q.0[1]), mv(&q.0[2]), mv(&q.0[3])]);
        let mr = |r: &Rect| Rect::new(r.x0 + dx, r.y0 + dy, r.x1 + dx, r.y1 + dy);
        let mut s = self.clone();
        s.name = None;
        s.modified = None;
        s.kind = match &self.kind {
            AnnotationKind::Highlight { quads } => AnnotationKind::Highlight {
                quads: quads.iter().map(mq).collect(),
            },
            AnnotationKind::Underline { quads } => AnnotationKind::Underline {
                quads: quads.iter().map(mq).collect(),
            },
            AnnotationKind::StrikeOut { quads } => AnnotationKind::StrikeOut {
                quads: quads.iter().map(mq).collect(),
            },
            AnnotationKind::Squiggly { quads } => AnnotationKind::Squiggly {
                quads: quads.iter().map(mq).collect(),
            },
            AnnotationKind::Note { pos } => AnnotationKind::Note { pos: mv(pos) },
            AnnotationKind::FreeText {
                rect,
                font_size,
                text_color,
                align,
                font,
                callout,
            } => AnnotationKind::FreeText {
                rect: mr(rect),
                font_size: *font_size,
                text_color: *text_color,
                align: *align,
                font: *font,
                callout: callout.as_ref().map(|c| c.iter().map(mv).collect()),
            },
            AnnotationKind::Rectangle { rect } => AnnotationKind::Rectangle { rect: mr(rect) },
            AnnotationKind::Ellipse { rect } => AnnotationKind::Ellipse { rect: mr(rect) },
            AnnotationKind::Line {
                start,
                end,
                start_ending,
                end_ending,
            } => AnnotationKind::Line {
                start: mv(start),
                end: mv(end),
                start_ending: *start_ending,
                end_ending: *end_ending,
            },
            AnnotationKind::Polygon { points } => AnnotationKind::Polygon {
                points: points.iter().map(mv).collect(),
            },
            AnnotationKind::PolyLine {
                points,
                start_ending,
                end_ending,
            } => AnnotationKind::PolyLine {
                points: points.iter().map(mv).collect(),
                start_ending: *start_ending,
                end_ending: *end_ending,
            },
            AnnotationKind::Ink { strokes } => AnnotationKind::Ink {
                strokes: strokes.iter().map(|s| s.iter().map(mv).collect()).collect(),
            },
            AnnotationKind::StampText { rect, label } => AnnotationKind::StampText {
                rect: mr(rect),
                label: label.clone(),
            },
        };
        s
    }
}

/// Word-wrap helper shared with form appearance generation.
pub(crate) fn wrap_lines_pub(fb: &FontBuilder, text: &str, fs: f64, max_w: f64) -> Vec<String> {
    wrap_lines(fb, text, fs, max_w)
}

/// Placement of a measurement's text label.
struct LabelBox {
    rect: Rect,
    font_size: f64,
    origin: Point,
    text: String,
}

/// Compute where the label of a measurement annotation goes (None for counts / no label).
fn measure_label_box(spec: &AnnotationSpec) -> Option<LabelBox> {
    let md = spec.measure.as_ref()?;
    if md.kind == crate::measure::MeasureKind::Count || md.label.is_empty() {
        return None;
    }
    let pts = crate::measure::points_of(md.kind, &spec.kind)?;
    let anchor = crate::measure::label_anchor(md.kind, &pts);
    let fb = FontBuilder::new(BundledFace::Sans).ok()?;
    if !fb.missing_chars(&md.label).is_empty() {
        return None;
    }
    let fs = 9.0;
    let w = fb.text_width(&md.label, fs) + 6.0;
    let h = fs * (fb.ascent_em() - fb.descent_em()) + 4.0;
    let rect = Rect::new(
        anchor.x - w / 2.0,
        anchor.y - h / 2.0,
        anchor.x + w / 2.0,
        anchor.y + h / 2.0,
    );
    let baseline = rect.y0 + 2.0 - fs * fb.descent_em();
    Some(LabelBox {
        rect,
        font_size: fs,
        origin: Point::new(rect.x0 + 3.0, baseline),
        text: md.label.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_dates() {
        assert_eq!(pdf_date_from_unix(0), "D:19700101000000Z");
        assert_eq!(pdf_date_from_unix(1_700_000_000), "D:20231114221320Z");
    }

    #[test]
    fn da_parsing() {
        let (fs, c) = parse_da("/Helv 14 Tf 1 0 0 rg");
        assert_eq!(fs, 14.0);
        assert_eq!(c, Rgb(1.0, 0.0, 0.0));
    }

    #[test]
    fn quad_order_roundtrip() {
        let q = Quad::from_rect(Rect::new(0.0, 0.0, 10.0, 5.0));
        let arr = quad_points(&[q]);
        let Object::Array(a) = arr else { panic!() };
        let v: Vec<f64> = a
            .iter()
            .map(|o| objutil::num(&Document::new(), o).unwrap())
            .collect();
        // TL, TR, BL, BR
        assert_eq!(v, vec![0.0, 5.0, 10.0, 5.0, 0.0, 0.0, 10.0, 0.0]);
    }
}
