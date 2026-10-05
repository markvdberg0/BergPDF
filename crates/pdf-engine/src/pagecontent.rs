//! Genuine page-content editing.
//!
//! Every edit is a *byte-span replacement inside the decoded content stream* located by the
//! scanner in [`crate::content`]; bytes we do not touch are preserved verbatim. Nothing is
//! covered with rectangles and nothing is rasterised.
//!
//! **State preservation.** A text edit replaces the whole span of a [`TextGroup`]. The
//! replacement starts with an absolute `Tm` (so the new text begins where the old text did,
//! optionally shifted) and ends by re-establishing the exact text/line matrices the original
//! left behind, so every later operator behaves as before. Surrounding text therefore never
//! moves; if the new text is longer it may overlap its neighbour, which is reported.
//!
//! **Copy-on-write.** A content stream referenced from more than one place is cloned before it
//! is changed; resource dictionaries are materialised per page before entries are added, so
//! editing one page never alters another page that shared the stream or resources.

use crate::content::{self, FontMetrics, Mat, Op, Operand, RunItem, TextGroup, Walk};
use crate::doc::{PageId, Tx};
use crate::error::{EngineError, Result};
use crate::fontembed::{BundledFace, FontBuilder, FontStyle, zlib};
use crate::geom::{Point, Quad, Rect};
use crate::objutil::{self, fmt_num_prec, name};
use crate::textfont::{Enc, FontInfo};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

/// Maximum decoded size of one page's content (all streams).
pub const MAX_CONTENT_BYTES: usize = 128 * 1024 * 1024;

/// Reference to a text run or image on a given revision of a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObjRef {
    /// Byte offset of the span in the concatenated page content.
    pub start: usize,
    /// FNV-1a hash of the span bytes (detects stale references).
    pub fingerprint: u64,
}

/// A text run offered for editing.
#[derive(Clone, Debug)]
pub struct TextRunInfo {
    /// Stable reference for this page revision.
    pub id: ObjRef,
    /// Decoded text (spaces inferred from gaps).
    pub text: String,
    /// Box in PDF user space.
    pub quad: Quad,
    /// Font resource name.
    pub font_resource: String,
    /// Base font name (subset tag removed).
    pub base_font: String,
    /// The font program is embedded.
    pub embedded: bool,
    /// The embedded font is a subset.
    pub subset: bool,
    /// Effective font size in points (text size × matrix scale).
    pub size_pt: f64,
    /// `Ok` when the run can be edited; otherwise the reason it cannot.
    pub editable: std::result::Result<(), String>,
    /// Width of the run in points along its baseline.
    pub width_pt: f64,
}

/// An image placed on the page.
#[derive(Clone, Debug)]
pub struct ImageInfo {
    /// Stable reference for this page revision.
    pub id: ObjRef,
    /// XObject resource name.
    pub name: String,
    /// Placement quad in user space.
    pub quad: Quad,
    /// Pixel width.
    pub width_px: i64,
    /// Pixel height.
    pub height_px: i64,
}

/// Parameters of a text edit.
#[derive(Clone, Debug, Default)]
pub struct TextEdit {
    /// Replacement text (`None` keeps the existing text and its exact layout).
    pub text: Option<String>,
    /// New font size in points (`None` keeps the size).
    pub size_pt: Option<f64>,
    /// Move by this vector in user space.
    pub shift: Option<(f64, f64)>,
    /// Distribute the difference in width over character spacing so the run keeps its width.
    pub fit_width: bool,
    /// Replace this run's font with a bundled one — an explicit, visible font change.
    pub substitute_font: Option<FontStyle>,
}

/// What an edit did.
#[derive(Clone, Debug, Default)]
pub struct EditReport {
    /// Run width before, in points.
    pub width_before_pt: f64,
    /// Run width after, in points.
    pub width_after_pt: f64,
    /// The new text now overlaps text that follows it on the same line.
    pub overlaps_following: bool,
    /// Spaces approximated by gaps because the font has no space glyph.
    pub gap_spaces: usize,
    /// Human-readable notes (font change, width change, clipping risk…).
    pub warnings: Vec<String>,
}

fn fnv(b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for x in b {
        h ^= u64::from(*x);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// Analysed content of one page.
pub struct PageContent {
    page: ObjectId,
    streams: Vec<(ObjectId, Range<usize>)>,
    buf: Vec<u8>,
    ops: Vec<Op>,
    walk: Walk,
    groups: Vec<TextGroup>,
    fonts: HashMap<Vec<u8>, Arc<FontInfo>>,
    xobjects: HashMap<Vec<u8>, ObjectId>,
}

struct Adapter(Arc<FontInfo>);

impl FontMetrics for Adapter {
    fn codes(&self, s: &[u8]) -> Vec<u32> {
        self.0.codes(s)
    }
    fn width1000(&self, c: u32) -> f64 {
        self.0.width1000(c)
    }
    fn is_word_space(&self, c: u32) -> bool {
        self.0.is_word_space(c)
    }
    fn ascent1000(&self) -> f64 {
        self.0.ascent1000()
    }
    fn descent1000(&self) -> f64 {
        self.0.descent1000()
    }
    fn text_of(&self, c: u32) -> Option<String> {
        self.0.text_of(c)
    }
}

/// Content stream object ids of a page (`/Contents` may be one stream or an array).
pub fn content_stream_ids(doc: &Document, page: ObjectId) -> Vec<ObjectId> {
    let Ok(d) = doc.get_dictionary(page) else {
        return Vec::new();
    };
    match d.get(b"Contents") {
        Ok(Object::Reference(r)) => match doc.objects.get(r) {
            Some(Object::Stream(_)) => vec![*r],
            Some(Object::Array(a)) => a.iter().filter_map(|o| o.as_reference().ok()).collect(),
            _ => Vec::new(),
        },
        Ok(Object::Array(a)) => a.iter().filter_map(|o| o.as_reference().ok()).collect(),
        _ => Vec::new(),
    }
}

impl PageContent {
    /// Decode and analyse a page's content.
    pub fn load(doc: &Document, page: ObjectId) -> Result<PageContent> {
        let ids = content_stream_ids(doc, page);
        let mut buf = Vec::new();
        let mut streams = Vec::new();
        for id in ids {
            let Some(Object::Stream(s)) = doc.objects.get(&id) else {
                continue;
            };
            let data = s
                .decompressed_content_with_limit(MAX_CONTENT_BYTES)
                .map_err(|e| {
                    EngineError::Unsupported(format!("content stream cannot be decoded: {e}"))
                })?;
            let start = buf.len();
            buf.extend_from_slice(&data);
            streams.push((id, start..buf.len()));
            buf.push(b'\n');
            if buf.len() > MAX_CONTENT_BYTES {
                return Err(EngineError::LimitExceeded(
                    "page content is too large to edit".into(),
                ));
            }
        }
        let ops = content::scan(&buf)?;
        // Resources.
        let res = objutil::inherited_obj(doc, page, b"Resources")
            .and_then(|o| objutil::deref(doc, &o).cloned());
        let res_dict = match res {
            Some(Object::Dictionary(d)) => Some(d),
            _ => None,
        };
        let mut fonts: HashMap<Vec<u8>, Arc<FontInfo>> = HashMap::new();
        let mut xobjects: HashMap<Vec<u8>, ObjectId> = HashMap::new();
        if let Some(rd) = &res_dict {
            if let Some(fd) = objutil::dict_dict(doc, rd, b"Font") {
                for (k, v) in fd.iter() {
                    if let Some(Object::Dictionary(f)) = objutil::deref(doc, v) {
                        fonts.insert(k.clone(), Arc::new(FontInfo::load(doc, f)));
                    }
                }
            }
            if let Some(xd) = objutil::dict_dict(doc, rd, b"XObject") {
                for (k, v) in xd.iter() {
                    if let Ok(id) = v.as_reference() {
                        xobjects.insert(k.clone(), id);
                    }
                }
            }
        }
        let fonts_ref = &fonts;
        let walk = content::walk(&ops, &mut |n: &[u8]| {
            fonts_ref
                .get(n)
                .map(|f| Arc::new(Adapter(f.clone())) as Arc<dyn FontMetrics>)
        });
        let groups = content::group_runs(&walk, &ops);
        Ok(PageContent {
            page,
            streams,
            buf,
            ops,
            walk,
            groups,
            fonts,
            xobjects,
        })
    }

    fn group_by_ref(&self, r: ObjRef) -> Result<usize> {
        self.groups
            .iter()
            .position(|g| {
                g.span.start == r.start && fnv(&self.buf[g.span.clone()]) == r.fingerprint
            })
            .ok_or(EngineError::StaleReference)
    }

    fn group_font(&self, g: &TextGroup) -> Option<&Arc<FontInfo>> {
        self.fonts.get(&self.walk.runs[g.runs.start].params.font)
    }

    fn scale_of(m: &Mat) -> f64 {
        m.det().abs().sqrt()
    }

    /// Text of a group (spaces inferred from visible gaps).
    fn group_text(&self, g: &TextGroup) -> String {
        let mut out = String::new();
        let font = self.group_font(g);
        let mut prev_adv_end: Option<(f64, f64)> = None; // (end x local, size)
        let first = &self.walk.runs[g.runs.start];
        let m0 = first.tm.then(&first.ctm);
        let inv = m0.inverse();
        for k in g.runs.clone() {
            let r = &self.walk.runs[k];
            let size = r.params.size.abs().max(1e-6);
            if let (Some((end, _)), Some(inv)) = (prev_adv_end, inv) {
                let (ox, oy) = r.tm.then(&r.ctm).apply(0.0, 0.0);
                let (lx, _) = inv.apply(ox, oy);
                if lx - end > 0.15 * size && !out.ends_with(' ') {
                    out.push(' ');
                }
            }
            for it in &r.items {
                match it {
                    RunItem::Codes(s) => {
                        if let Some(f) = font {
                            for c in f.codes(s) {
                                out.push_str(&f.text_of(c).unwrap_or_else(|| "\u{FFFD}".into()));
                            }
                        }
                    }
                    RunItem::Adjust(n) => {
                        if -n / 1000.0 * size * r.params.th > 0.15 * size && !out.ends_with(' ') {
                            out.push(' ');
                        }
                    }
                }
            }
            if let Some(inv) = inv {
                let (ox, oy) = r.tm.then(&r.ctm).apply(0.0, 0.0);
                let (lx, _) = inv.apply(ox, oy);
                prev_adv_end = Some((lx + r.advance, size));
            }
        }
        out.trim_end().to_string()
    }

    fn group_width_text_units(&self, g: &TextGroup) -> f64 {
        let first = &self.walk.runs[g.runs.start];
        let last = &self.walk.runs[g.runs.end - 1];
        let Some(inv) = first.tm.then(&first.ctm).inverse() else {
            return first.advance;
        };
        let (ox, oy) = last.tm.then(&last.ctm).apply(0.0, 0.0);
        let (lx, _) = inv.apply(ox, oy);
        lx + last.advance
    }

    /// Whether the page shows or hides any text at all (invisible OCR text counts).
    pub fn has_text(&self) -> bool {
        self.groups
            .iter()
            .any(|g| !self.group_text(g).trim().is_empty())
    }

    /// Editable text runs, in content order.
    pub fn text_runs(&self) -> Vec<TextRunInfo> {
        let mut out = Vec::new();
        for g in &self.groups {
            let first = &self.walk.runs[g.runs.start];
            if !first.visible() {
                continue;
            }
            let font = self.group_font(g);
            let text = self.group_text(g);
            // Advance-only or whitespace-only groups (e.g. the state-restoring `[n] TJ` we emit) are
            // not user-visible runs.
            if text.trim().is_empty() {
                continue;
            }
            let w_text = self.group_width_text_units(g);
            let m = first.tm.then(&first.ctm);
            let asc = font.map_or(800.0, |f| f.ascent1000());
            let desc = font.map_or(-200.0, |f| f.descent1000());
            let (y0, y1) = (
                first.params.rise + desc / 1000.0 * first.params.size,
                first.params.rise + asc / 1000.0 * first.params.size,
            );
            let pt = |x: f64, y: f64| {
                let (a, b) = m.apply(x, y);
                Point::new(a, b)
            };
            let quad = Quad([pt(0.0, y0), pt(w_text, y0), pt(w_text, y1), pt(0.0, y1)]);
            let scale = Self::scale_of(&m);
            let editable = match (font, first.operator.as_str()) {
                (None, _) => Err(format!(
                    "font /{} could not be found",
                    String::from_utf8_lossy(&first.params.font)
                )),
                (Some(f), _) if f.can_edit().is_err() => f.can_edit(),
                (Some(_), "\"") => {
                    Err("this text uses the \" operator, which changes spacing state".into())
                }
                _ if self.walk.runs[g.runs.clone()]
                    .iter()
                    .any(|r| !r.in_text_object) =>
                {
                    Err("text outside a text object".into())
                }
                _ if m.det().abs() < 1e-12 => Err("degenerate text matrix".into()),
                _ => Ok(()),
            };
            out.push(TextRunInfo {
                id: ObjRef {
                    start: g.span.start,
                    fingerprint: fnv(&self.buf[g.span.clone()]),
                },
                text,
                quad,
                font_resource: String::from_utf8_lossy(&first.params.font).into_owned(),
                base_font: font.map(|f| f.base_font.clone()).unwrap_or_default(),
                embedded: font.is_some_and(|f| f.embedded),
                subset: font.is_some_and(|f| f.subset),
                size_pt: first.params.size.abs() * scale,
                editable,
                width_pt: w_text * scale,
            });
        }
        out
    }

    /// Images (Image XObjects painted with `Do`).
    pub fn images(&self, doc: &Document) -> Vec<ImageInfo> {
        let mut out = Vec::new();
        for d in &self.walk.dos {
            let Some(xid) = self.xobjects.get(&d.name) else {
                continue;
            };
            let Some(Object::Stream(s)) = doc.objects.get(xid) else {
                continue;
            };
            if objutil::dict_name(doc, &s.dict, b"Subtype") != Some(b"Image") {
                continue;
            }
            let q = d.quad();
            out.push(ImageInfo {
                id: ObjRef {
                    start: d.span.start,
                    fingerprint: fnv(&self.buf[d.span.clone()]),
                },
                name: String::from_utf8_lossy(&d.name).into_owned(),
                quad: Quad([
                    Point::new(q[0].0, q[0].1),
                    Point::new(q[1].0, q[1].1),
                    Point::new(q[2].0, q[2].1),
                    Point::new(q[3].0, q[3].1),
                ]),
                width_px: objutil::dict_num(doc, &s.dict, b"Width").unwrap_or(0.0) as i64,
                height_px: objutil::dict_num(doc, &s.dict, b"Height").unwrap_or(0.0) as i64,
            });
        }
        out
    }

    /// Codes already shown with each font name.
    fn used_codes(&self, font: &[u8]) -> HashSet<u32> {
        self.walk
            .runs
            .iter()
            .filter(|r| r.params.font == font)
            .flat_map(|r| r.glyphs.iter().map(|g| g.0))
            .collect()
    }

    fn stream_for_span(&self, span: &Range<usize>) -> Result<(usize, Range<usize>)> {
        for (i, (_, r)) in self.streams.iter().enumerate() {
            if span.start >= r.start && span.end <= r.end {
                return Ok((i, (span.start - r.start)..(span.end - r.start)));
            }
        }
        Err(EngineError::Unsupported(
            "this object spans two content streams".into(),
        ))
    }

    // ---- edits ---------------------------------------------------------------------------

    /// Original layout of a group as TJ items (codes + adjustments), including inter-run gaps.
    fn original_items(&self, g: &TextGroup) -> Vec<RunItem> {
        let first = &self.walk.runs[g.runs.start];
        let inv = first.tm.then(&first.ctm).inverse();
        let mut out = Vec::new();
        let mut prev_end: Option<f64> = None;
        for k in g.runs.clone() {
            let r = &self.walk.runs[k];
            let size = r.params.size;
            if let (Some(end), Some(inv)) = (prev_end, inv) {
                let (ox, oy) = r.tm.then(&r.ctm).apply(0.0, 0.0);
                let (lx, _) = inv.apply(ox, oy);
                let gap = lx - end;
                if gap.abs() > 1e-9 && (size * r.params.th).abs() > 1e-12 {
                    out.push(RunItem::Adjust(-gap / (size * r.params.th) * 1000.0));
                }
            }
            out.extend(r.items.iter().cloned());
            if let Some(inv) = inv {
                let (ox, oy) = r.tm.then(&r.ctm).apply(0.0, 0.0);
                let (lx, _) = inv.apply(ox, oy);
                prev_end = Some(lx + r.advance);
            }
        }
        out
    }

    /// Edit a text run. Returns a report describing width/overlap/font effects.
    pub fn edit_text(&self, tx: &mut Tx<'_>, run: ObjRef, edit: &TextEdit) -> Result<EditReport> {
        let gi = self.group_by_ref(run)?;
        let g = &self.groups[gi];
        let first = &self.walk.runs[g.runs.start];
        let last = &self.walk.runs[g.runs.end - 1];
        let info = self
            .text_runs()
            .into_iter()
            .find(|t| t.id == run)
            .ok_or(EngineError::StaleReference)?;
        info.editable.clone().map_err(EngineError::Unsupported)?;
        let font = self
            .group_font(g)
            .ok_or_else(|| EngineError::Unsupported("font missing".into()))?
            .clone();
        let m0 = first.tm.then(&first.ctm);
        let scale = Self::scale_of(&m0);
        let old_size = first.params.size;
        let new_size = edit
            .size_pt
            .map_or(old_size, |pt| old_size.signum() * (pt / scale));
        let th = first.params.th;
        let width_before_text = self.group_width_text_units(g);

        // Decide font + items.
        let mut report = EditReport {
            width_before_pt: width_before_text * scale,
            ..Default::default()
        };
        let mut font_res: Vec<u8> = first.params.font.clone();
        let mut new_font_obj: Option<ObjectId> = None;
        let items: Vec<RunItem>;
        let new_text_width: f64; // text-space units at new size (before fit)
        let mut code_len = font.code_len;
        match (&edit.text, edit.substitute_font) {
            (Some(text), Some(style)) => {
                let mut fb = FontBuilder::new(BundledFace::Styled(style))?;
                let missing = fb.missing_chars(text);
                if !missing.is_empty() {
                    return Err(EngineError::MissingGlyphs {
                        font: style.family.title().into(),
                        chars: missing
                            .iter()
                            .map(char::to_string)
                            .collect::<Vec<_>>()
                            .join(" "),
                    });
                }
                let codes = fb.encode_str(text)?;
                let w = fb.text_width(text, new_size.abs());
                let fid = fb.finish(tx)?;
                let nm = format!("BergF{}", fid.0);
                font_res = nm.clone().into_bytes();
                new_font_obj = Some(fid);
                items = vec![RunItem::Codes(codes)];
                new_text_width = w * th * new_size.signum();
                code_len = 2;
                report.warnings.push(format!(
                    "Font changed to {} for this text (was {}).",
                    style.family.title(),
                    if font.base_font.is_empty() {
                        "an unnamed font"
                    } else {
                        &font.base_font
                    }
                ));
            }
            (Some(text), None) => {
                let used = self.used_codes(&first.params.font);
                let enc = font.encode(text, &used);
                if !enc.missing.is_empty() {
                    return Err(EngineError::MissingGlyphs {
                        font: font.base_font.clone(),
                        chars: enc
                            .missing
                            .iter()
                            .map(char::to_string)
                            .collect::<Vec<_>>()
                            .join(" "),
                    });
                }
                report.gap_spaces = enc.gap_spaces;
                if enc.gap_spaces > 0 {
                    report.warnings.push(format!(
                        "{} space(s) are drawn as gaps because this font has no space glyph.",
                        enc.gap_spaces
                    ));
                }
                let w1000 = font.text_width1000(&enc);
                let mut its = Vec::new();
                for e in enc.items {
                    match e {
                        Enc::Codes(b) => its.push(RunItem::Codes(b)),
                        Enc::Gap(g1000) => its.push(RunItem::Adjust(-g1000)),
                    }
                }
                items = its;
                // Natural advance including Tc/Tw.
                let nchars: usize = items
                    .iter()
                    .map(|i| {
                        if let RunItem::Codes(b) = i {
                            font.codes(b).len()
                        } else {
                            0
                        }
                    })
                    .sum();
                new_text_width = (w1000 / 1000.0 * new_size + first.params.tc * nchars as f64) * th;
            }
            (None, _) => {
                items = self.original_items(g);
                new_text_width = width_before_text * (new_size / old_size);
            }
        }
        let font_changed = new_font_obj.is_some();

        // Fit width: adjust character spacing so the run keeps its original width.
        let mut tc_new: Option<f64> = None;
        if edit.fit_width && edit.text.is_some() {
            let n: usize = items
                .iter()
                .map(|i| match i {
                    RunItem::Codes(b) => {
                        if code_len == 2 {
                            b.len() / 2
                        } else {
                            b.len()
                        }
                    }
                    RunItem::Adjust(_) => 0,
                })
                .sum();
            if n > 0 && th.abs() > 1e-12 {
                tc_new =
                    Some(first.params.tc + (width_before_text - new_text_width) / (n as f64 * th));
            }
        }
        let width_after_text = match tc_new {
            Some(_) => width_before_text,
            None => new_text_width,
        };
        report.width_after_pt = width_after_text * scale;

        // Overlap with following text on the same baseline.
        if let Some(inv) = m0.inverse() {
            for (k, other) in self.groups.iter().enumerate() {
                if k == gi {
                    continue;
                }
                let o = &self.walk.runs[other.runs.start];
                let (ox, oy) = o.tm.then(&o.ctm).apply(0.0, 0.0);
                let (lx, ly) = inv.apply(ox, oy);
                if ly.abs() < 0.3 * old_size.abs()
                    && lx >= width_before_text - 1e-6
                    && lx < width_after_text - 0.02 * old_size.abs()
                {
                    report.overlaps_following = true;
                }
            }
        }
        if report.overlaps_following {
            report.warnings.push("The new text is wider than the original and now overlaps the text that follows. Surrounding text was not moved.".into());
        } else if (report.width_after_pt - report.width_before_pt).abs() > 0.5 {
            report.warnings.push(format!(
                "The text is {:.1} pt {} than before; surrounding text was not moved.",
                (report.width_after_pt - report.width_before_pt).abs(),
                if report.width_after_pt > report.width_before_pt {
                    "wider"
                } else {
                    "narrower"
                }
            ));
        }

        // Start matrix (optionally moved in user space).
        let mut tm_start = first.tm;
        if let Some((dx, dy)) = edit.shift
            && let Some(inv_ctm) = first.ctm.inverse()
        {
            tm_start = first
                .tm
                .then(&first.ctm)
                .then(&Mat::translate(dx, dy))
                .then(&inv_ctm);
        }

        // Emit.
        let mut out = String::new();
        let tf_changed = font_changed || (new_size - old_size).abs() > 1e-9;
        if tf_changed {
            out.push_str(&format!(
                "/{} {} Tf\n",
                pdf_name(&font_res),
                fmt_num_prec(new_size)
            ));
        }
        if let Some(tc) = tc_new {
            out.push_str(&format!("{} Tc\n", fmt_num_prec(tc)));
        }
        out.push_str(&format!("{} Tm\n", tm_start.operands()));
        out.push_str(&tj_array(&items, code_len));
        if tc_new.is_some() {
            out.push_str(&format!("{} Tc\n", fmt_num_prec(first.params.tc)));
        }
        if tf_changed {
            out.push_str(&format!(
                "/{} {} Tf\n",
                pdf_name(&first.params.font),
                fmt_num_prec(old_size)
            ));
        }
        out.push_str(&self.restore_state(g, last));

        let mut bytes = out.into_bytes();
        // Keep a separator so the replacement cannot fuse with following tokens.
        bytes.push(b' ');
        self.replace_span(tx, g.span.clone(), &bytes)?;
        if let Some(fid) = new_font_obj {
            add_resource(
                tx,
                self.page,
                b"Font",
                &String::from_utf8_lossy(&font_res),
                Object::Reference(fid),
            )?;
        }
        Ok(report)
    }

    /// Operators that re-establish the text/line matrices the original group left behind.
    fn restore_state(&self, g: &TextGroup, last: &content::Run) -> String {
        // Nothing needed when the next operator sets the position absolutely or ends the object.
        let next = self.ops.get(g.ops.end);
        if next.is_none_or(|o| matches!(o.name.as_slice(), b"ET" | b"Tm" | b"BT")) {
            return String::new();
        }
        let mut s = format!("{} Tm\n", last.tlm.operands());
        let (size, th) = (last.params.size, last.params.th);
        if last.advance.abs() > 1e-9 && (size * th).abs() > 1e-12 {
            // Advance Tm to where the original show left it: TJ number n moves by -n/1000·size·th.
            s.push_str(&format!(
                "[{}] TJ\n",
                fmt_num_prec(-last.advance / (size * th) * 1000.0)
            ));
        }
        s
    }

    /// Delete a text run (nothing is drawn; following text is unaffected).
    pub fn delete_text(&self, tx: &mut Tx<'_>, run: ObjRef) -> Result<()> {
        let gi = self.group_by_ref(run)?;
        let g = &self.groups[gi];
        let last = &self.walk.runs[g.runs.end - 1];
        let mut s = self.restore_state(g, last);
        s.push(' ');
        self.replace_span(tx, g.span.clone(), s.as_bytes())
    }

    /// Delete several text runs at once (stale references are skipped). Returns how many were
    /// removed. All edits to one content stream are applied together, back to front.
    pub fn delete_texts(&self, tx: &mut Tx<'_>, runs: &[ObjRef]) -> Result<usize> {
        let mut per_stream: std::collections::BTreeMap<usize, Vec<(Range<usize>, String)>> =
            std::collections::BTreeMap::new();
        let mut n = 0;
        for r in runs {
            let Ok(gi) = self.group_by_ref(*r) else {
                continue;
            };
            let g = &self.groups[gi];
            let last = &self.walk.runs[g.runs.end - 1];
            let mut s = self.restore_state(g, last);
            s.push(' ');
            let Ok((si, local)) = self.stream_for_span(&g.span) else {
                continue;
            };
            per_stream.entry(si).or_default().push((local, s));
            n += 1;
        }
        for (si, mut edits) in per_stream {
            let (sid, srange) = &self.streams[si];
            let mut bytes = self.buf[srange.clone()].to_vec();
            edits.sort_by_key(|e| std::cmp::Reverse(e.0.start));
            for (local, rep) in edits {
                bytes.splice(local, rep.bytes());
            }
            commit_stream(tx, self.page, *sid, bytes)?;
        }
        Ok(n)
    }

    /// Delete an image placement (`/Name Do`).
    pub fn delete_image(&self, tx: &mut Tx<'_>, img: ObjRef) -> Result<()> {
        let d = self.do_by_ref(img)?;
        self.replace_span(tx, d.span.clone(), b" ")
    }

    fn do_by_ref(&self, r: ObjRef) -> Result<&content::DoUse> {
        self.walk
            .dos
            .iter()
            .find(|d| d.span.start == r.start && fnv(&self.buf[d.span.clone()]) == r.fingerprint)
            .ok_or(EngineError::StaleReference)
    }

    /// Move/resize an image so its bounding box becomes `new_box` (user space).
    pub fn place_image(&self, tx: &mut Tx<'_>, img: ObjRef, new_box: Rect) -> Result<()> {
        let d = self.do_by_ref(img)?;
        let q = d.quad();
        let xs = [q[0].0, q[1].0, q[2].0, q[3].0];
        let ys = [q[0].1, q[1].1, q[2].1, q[3].1];
        let old = Rect::new(
            xs.iter().copied().fold(f64::MAX, f64::min),
            ys.iter().copied().fold(f64::MAX, f64::min),
            xs.iter().copied().fold(f64::MIN, f64::max),
            ys.iter().copied().fold(f64::MIN, f64::max),
        );
        let new_box = new_box.abs();
        if old.width() < 1e-9
            || old.height() < 1e-9
            || new_box.width() < 1.0
            || new_box.height() < 1.0
        {
            return Err(EngineError::InvalidArgument(
                "image size out of range".into(),
            ));
        }
        let t = Mat::translate(-old.x0, -old.y0)
            .then(&Mat::scale(
                new_box.width() / old.width(),
                new_box.height() / old.height(),
            ))
            .then(&Mat::translate(new_box.x0, new_box.y0));
        let a = d
            .ctm
            .conjugate(&t)
            .ok_or_else(|| EngineError::Unsupported("degenerate image matrix".into()))?;
        let s = format!("q {} cm /{} Do Q ", a.operands(), pdf_name(&d.name));
        self.replace_span(tx, d.span.clone(), s.as_bytes())
    }

    /// Replace an image's pixels, keeping its box (the new image is fitted inside it,
    /// preserving aspect ratio, unless `stretch`).
    pub fn replace_image(
        &self,
        tx: &mut Tx<'_>,
        img: ObjRef,
        data: &[u8],
        stretch: bool,
    ) -> Result<()> {
        let d = self.do_by_ref(img)?;
        let new = crate::imageembed::add_image_xobject(tx, data)?;
        let nm = format!("BergIm{}", new.id.0);
        add_resource(tx, self.page, b"XObject", &nm, Object::Reference(new.id))?;
        let mut wrap = String::new();
        if !stretch {
            // Fit inside the old unit square (aspect-preserving) via a matrix in the image's own space.
            let ar = f64::from(new.width) / f64::from(new.height);
            let q = d.quad();
            let w = ((q[1].0 - q[0].0).hypot(q[1].1 - q[0].1)).max(1e-9);
            let h = ((q[3].0 - q[0].0).hypot(q[3].1 - q[0].1)).max(1e-9);
            let (sx, sy) = if ar > w / h {
                (1.0, (w / ar) / h)
            } else {
                ((h * ar) / w, 1.0)
            };
            let fit = Mat::scale(sx, sy).then(&Mat::translate((1.0 - sx) / 2.0, (1.0 - sy) / 2.0));
            wrap = format!("{} cm ", fit.operands());
        }
        let s = format!("q {wrap}/{nm} Do Q ");
        self.replace_span(tx, d.span.clone(), s.as_bytes())
    }

    fn replace_span(&self, tx: &mut Tx<'_>, span: Range<usize>, replacement: &[u8]) -> Result<()> {
        let (si, local) = self.stream_for_span(&span)?;
        let (sid, srange) = &self.streams[si];
        let mut bytes = self.buf[srange.clone()].to_vec();
        bytes.splice(local, replacement.iter().copied());
        commit_stream(tx, self.page, *sid, bytes)
    }

    /// CTM and `q` depth at the end of the page content (for appending new content).
    pub fn end_state(&self) -> (Mat, usize) {
        (self.walk.end_ctm, self.walk.open_q)
    }
}

fn pdf_name(n: &[u8]) -> String {
    let mut s = String::new();
    for &b in n {
        if b > 0x20
            && b < 0x7f
            && !matches!(
                b,
                b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%' | b'#'
            )
        {
            s.push(b as char);
        } else {
            s.push_str(&format!("#{b:02X}"));
        }
    }
    s
}

fn pdf_string(bytes: &[u8], code_len: usize) -> String {
    if code_len == 2 {
        let mut s = String::from("<");
        for b in bytes {
            s.push_str(&format!("{b:02X}"));
        }
        s.push('>');
        s
    } else {
        let mut s = String::from("(");
        for &b in bytes {
            match b {
                b'(' | b')' | b'\\' => {
                    s.push('\\');
                    s.push(b as char);
                }
                0x20..=0x7e => s.push(b as char),
                _ => s.push_str(&format!("\\{b:03o}")),
            }
        }
        s.push(')');
        s
    }
}

fn tj_array(items: &[RunItem], code_len: usize) -> String {
    let mut s = String::from("[");
    for (i, it) in items.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        match it {
            RunItem::Codes(b) => s.push_str(&pdf_string(b, code_len)),
            RunItem::Adjust(n) => s.push_str(&fmt_num_prec(*n)),
        }
    }
    s.push_str("] TJ\n");
    s
}

/// Write new decoded bytes for content stream `sid`, copying the stream first when it is
/// referenced from anywhere else (so other pages sharing it are unaffected).
fn commit_stream(tx: &mut Tx<'_>, page: ObjectId, sid: ObjectId, bytes: Vec<u8>) -> Result<()> {
    let mut dict = match tx.doc().objects.get(&sid) {
        Some(Object::Stream(s)) => s.dict.clone(),
        _ => return Err(EngineError::NoSuchObject(sid.0, sid.1)),
    };
    for k in [
        &b"Filter"[..],
        b"DecodeParms",
        b"Length",
        b"F",
        b"FFilter",
        b"FDecodeParms",
        b"DL",
    ] {
        dict.remove(k);
    }
    dict.set("Filter", name("FlateDecode"));
    let mut stream = Stream::new(dict, zlib(&bytes));
    stream.allows_compression = false;
    if objutil::reference_count(tx.doc(), sid) <= 1 {
        tx.set(sid, Object::Stream(stream));
        return Ok(());
    }
    let new_id = tx.add(Object::Stream(stream));
    // Re-point this page's /Contents entry at the copy.
    let contents = tx
        .doc()
        .get_dictionary(page)?
        .get(b"Contents")
        .ok()
        .cloned();
    let replaced = match contents {
        Some(Object::Reference(r)) if r == sid => Object::Reference(new_id),
        Some(Object::Reference(r)) => match tx.doc().objects.get(&r) {
            Some(Object::Array(a)) => Object::Array(
                a.iter()
                    .map(|o| {
                        if o.as_reference().ok() == Some(sid) {
                            Object::Reference(new_id)
                        } else {
                            o.clone()
                        }
                    })
                    .collect(),
            ),
            _ => return Err(EngineError::Malformed("unexpected /Contents".into())),
        },
        Some(Object::Array(a)) => Object::Array(
            a.iter()
                .map(|o| {
                    if o.as_reference().ok() == Some(sid) {
                        Object::Reference(new_id)
                    } else {
                        o.clone()
                    }
                })
                .collect(),
        ),
        _ => return Err(EngineError::Malformed("page has no /Contents".into())),
    };
    tx.dict_mut(page)?.set("Contents", replaced);
    Ok(())
}

/// Add an entry to the page's `/Resources /<category>` dictionary without touching any other
/// page: the resources are materialised as a direct dictionary on the page.
pub fn add_resource(
    tx: &mut Tx<'_>,
    page: ObjectId,
    category: &[u8],
    key: &str,
    value: Object,
) -> Result<()> {
    let mut res = match objutil::inherited_obj(tx.doc(), page, b"Resources")
        .and_then(|o| objutil::deref(tx.doc(), &o).cloned())
    {
        Some(Object::Dictionary(d)) => d,
        _ => Dictionary::new(),
    };
    let mut sub = match res
        .get(category)
        .ok()
        .and_then(|o| objutil::deref(tx.doc(), o).cloned())
    {
        Some(Object::Dictionary(d)) => d,
        _ => Dictionary::new(),
    };
    sub.set(key, value);
    res.set(category.to_vec(), Object::Dictionary(sub));
    tx.dict_mut(page)?.set("Resources", Object::Dictionary(res));
    Ok(())
}

/// Append a new content stream to a page (copy-on-write for the `/Contents` array).
pub(crate) fn append_content(tx: &mut Tx<'_>, page: ObjectId, bytes: &[u8]) -> Result<()> {
    let mut stream = Stream::new(dictionary! { "Filter" => "FlateDecode" }, zlib(bytes));
    stream.allows_compression = false;
    let new_id = tx.add(Object::Stream(stream));
    let existing = content_stream_ids(tx.doc(), page);
    let mut arr: Vec<Object> = existing.into_iter().map(Object::Reference).collect();
    arr.push(Object::Reference(new_id));
    tx.dict_mut(page)?.set("Contents", Object::Array(arr));
    Ok(())
}

/// Add new text to a page as real page content (embedded subset font, extractable text).
pub fn add_text(
    tx: &mut Tx<'_>,
    page: PageId,
    at: Point,
    text: &str,
    size_pt: f64,
    color: (f32, f32, f32),
    font: FontStyle,
) -> Result<()> {
    add_text_rotated(tx, page, at, text, size_pt, color, font, 0)
}

/// Like [`add_text`], with the text rotated `rotation` degrees counter-clockwise (a multiple of 90)
/// around `at`; use `crate::annot::upright_for(shown_cw)` to read upright on a rotated page.
#[allow(clippy::too_many_arguments)]
pub fn add_text_rotated(
    tx: &mut Tx<'_>,
    page: PageId,
    at: Point,
    text: &str,
    size_pt: f64,
    color: (f32, f32, f32),
    font: FontStyle,
    rotation: i32,
) -> Result<()> {
    if !(1.0..=500.0).contains(&size_pt) || text.trim().is_empty() {
        return Err(EngineError::InvalidArgument(
            "text is empty or the size is out of range".into(),
        ));
    }
    let pc = PageContent::load(tx.doc(), page.0)?;
    let (ctm, _) = pc.end_state();
    let inv = ctm.inverse().ok_or_else(|| {
        EngineError::Unsupported("the page ends with a degenerate transform".into())
    })?;
    let mut fb = FontBuilder::new(BundledFace::Styled(font))?;
    let missing = fb.missing_chars(text);
    if !missing.is_empty() {
        return Err(EngineError::MissingGlyphs {
            font: font.family.title().into(),
            chars: missing
                .iter()
                .map(char::to_string)
                .collect::<Vec<_>>()
                .join(" "),
        });
    }
    let lh = fb.line_height_em() * size_pt;
    let mut body = String::new();
    for (i, line) in text.split('\n').enumerate() {
        let codes = fb.encode_str(line)?;
        if i == 0 && rotation.rem_euclid(360) != 0 {
            let m = match rotation.rem_euclid(360) {
                90 => [0.0, 1.0, -1.0, 0.0],
                180 => [-1.0, 0.0, 0.0, -1.0],
                _ => [0.0, -1.0, 1.0, 0.0],
            };
            body.push_str(&format!(
                "{} {} {} {} {} {} Tm\n",
                m[0],
                m[1],
                m[2],
                m[3],
                fmt_num_prec(at.x),
                fmt_num_prec(at.y)
            ));
        } else {
            let (dx, dy) = if i == 0 { (at.x, at.y) } else { (0.0, -lh) };
            body.push_str(&format!("{} {} Td\n", fmt_num_prec(dx), fmt_num_prec(dy)));
        }
        if !codes.is_empty() {
            body.push_str(&format!("{} Tj\n", pdf_string(&codes, 2)));
        }
    }
    let fid = fb.finish(tx)?;
    let nm = format!("BergT{}", fid.0);
    let content = format!(
        "q\n{} cm\nBT\n/{} {} Tf\n{} {} {} rg\n{}ET\nQ\n",
        inv.operands(),
        nm,
        fmt_num_prec(size_pt),
        fmt_num_prec(f64::from(color.0)),
        fmt_num_prec(f64::from(color.1)),
        fmt_num_prec(f64::from(color.2)),
        body
    );
    add_resource(tx, page.0, b"Font", &nm, Object::Reference(fid))?;
    append_content(tx, page.0, content.as_bytes())
}

/// Add an image (PNG or JPEG bytes) to a page, filling `rect` (user space).
pub fn add_image(tx: &mut Tx<'_>, page: PageId, rect: Rect, data: &[u8]) -> Result<()> {
    let rect = rect.abs();
    if rect.width() < 1.0 || rect.height() < 1.0 {
        return Err(EngineError::InvalidArgument(
            "image box is too small".into(),
        ));
    }
    let pc = PageContent::load(tx.doc(), page.0)?;
    let (ctm, _) = pc.end_state();
    let inv = ctm.inverse().ok_or_else(|| {
        EngineError::Unsupported("the page ends with a degenerate transform".into())
    })?;
    let new = crate::imageembed::add_image_xobject(tx, data)?;
    let nm = format!("BergIm{}", new.id.0);
    add_resource(tx, page.0, b"XObject", &nm, Object::Reference(new.id))?;
    let content = format!(
        "q\n{} cm\n{} 0 0 {} {} {} cm\n/{} Do\nQ\n",
        inv.operands(),
        fmt_num_prec(rect.width()),
        fmt_num_prec(rect.height()),
        fmt_num_prec(rect.x0),
        fmt_num_prec(rect.y0),
        nm
    );
    append_content(tx, page.0, content.as_bytes())
}

/// Operand helper used by tests.
#[doc(hidden)]
pub fn operand_numbers(ops: &[Op], name: &str) -> Vec<Vec<f64>> {
    ops.iter()
        .filter(|o| o.name == name.as_bytes())
        .map(|o| o.operands.iter().filter_map(Operand::num).collect())
        .collect()
}

/// A recognised word to be added as invisible text: its box in default user space (y up).
#[derive(Clone, Debug, PartialEq)]
pub struct OcrWord {
    /// The text.
    pub text: String,
    /// Bounding box in the page's default user space.
    pub rect: Rect,
}

/// Whether a page already has text that can be selected/searched (so OCR would duplicate it).
pub fn page_has_text(doc: &Document, page: PageId) -> bool {
    PageContent::load(doc, page.0)
        .map(|pc| pc.has_text())
        .unwrap_or(false)
}

/// Add recognised words to pages as **invisible text** (render mode 3) so a scanned page becomes
/// searchable and its text selectable, without changing how it looks. One subset font is embedded
/// for the whole call. Characters the bundled font lacks are dropped from a word (the word is
/// skipped when nothing is left). Returns the number of words written.
pub fn add_ocr_text_layers(tx: &mut Tx<'_>, pages: &[(PageId, Vec<OcrWord>)]) -> Result<usize> {
    if pages.iter().all(|(_, w)| w.is_empty()) {
        return Ok(0);
    }
    let mut fb = FontBuilder::new(BundledFace::Sans)?;
    let em = (fb.ascent_em() - fb.descent_em()).max(0.1);
    let mut per_page: Vec<(PageId, crate::content::Mat, String)> = Vec::new();
    let mut total = 0usize;
    for (page, words) in pages {
        if words.is_empty() {
            continue;
        }
        let pc = PageContent::load(tx.doc(), page.0)?;
        let (ctm, _) = pc.end_state();
        let inv = ctm.inverse().ok_or_else(|| {
            EngineError::Unsupported("the page ends with a degenerate transform".into())
        })?;
        let mut body = String::new();
        for w in words {
            let text: String = w
                .text
                .chars()
                .filter(|c| !c.is_control() && fb.glyph_for(*c).is_some())
                .collect();
            let r = w.rect.abs();
            if text.is_empty() || r.width() < 0.5 || r.height() < 0.5 {
                continue;
            }
            let fs = (r.height() / em).clamp(1.0, 300.0);
            let natural = fb.text_width(&text, fs).max(0.01);
            let tz = (r.width() / natural * 100.0).clamp(5.0, 2000.0);
            let baseline = r.y0 - fb.descent_em() * fs; // descent is negative
            let codes = fb.encode_str(&text)?;
            body.push_str(&format!(
                "/BergOcr {} Tf {} Tz 1 0 0 1 {} {} Tm {} Tj\n",
                fmt_num_prec(fs),
                fmt_num_prec(tz),
                fmt_num_prec(r.x0),
                fmt_num_prec(baseline),
                pdf_string(&codes, 2)
            ));
            total += 1;
        }
        if !body.is_empty() {
            per_page.push((*page, inv, body));
        }
    }
    if per_page.is_empty() {
        return Ok(0);
    }
    let fid = fb.finish(tx)?;
    let nm = format!("BergOcr{}", fid.0);
    for (page, inv, body) in per_page {
        add_resource(tx, page.0, b"Font", &nm, Object::Reference(fid))?;
        let content = format!(
            "q\n{} cm\nBT\n3 Tr\n{}ET\nQ\n",
            inv.operands(),
            body.replace("/BergOcr ", &format!("/{nm} "))
        );
        append_content(tx, page.0, content.as_bytes())?;
    }
    Ok(total)
}
