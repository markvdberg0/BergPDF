//! Headers, footers, Bates numbers and watermarks: text (or a picture) stamped onto pages as real
//! page content, so every reader and printer shows it.
//!
//! **What is added.** Each marked page gets two extra content streams: one *before* its own content
//! (an opening `q`, plus a watermark that goes behind the page) and one *after* it (the closing `Q`s,
//! then the header/footer or a watermark on top). Both carry a private `/BergMark` key in their stream
//! dictionary and wrap their drawing in `/Artifact` marked content (`/Type /Pagination`, subtype
//! `/Header`, `/Footer` or `/Watermark`, as Acrobat does), so screen readers skip them and
//! [`remove_marks`] can take exactly them out again later. The page's own streams are never touched.
//!
//! **Where.** Positions are measured on the page *as shown*: the visible crop box with the page's
//! `/Rotate` applied. A header is at the top of a landscape page even when that page is a portrait
//! sheet turned by 90°.
//!
//! **Placeholders** in a header or footer: `{page}` (number counted from `first_number`), `{pages}`
//! (total pages in the document), `{label}` (the page label, see [`crate::pagelabels`]), `{bates}`
//! (the Bates number), `{date}` (as given by the caller) and `{file}` (the file name).

use crate::doc::{PageId, Tx};
use crate::error::{EngineError, Result};
use crate::fontembed::{BundledFace, FontBuilder, FontStyle, hex_string, zlib};
use crate::geom::{Affine, Rotation};
use crate::objutil::{self, fmt_num_prec, reference};
use crate::pagecontent::{PageContent, add_resource, content_stream_ids};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};

/// Key in the stream dictionary that marks a stream this module added.
const MARK_KEY: &[u8] = b"BergMark";
/// Largest number of characters in one header or footer slot.
pub const MAX_SLOT: usize = 200;

/// The kind of mark a stream holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkKind {
    /// Header and footer texts (including Bates numbers).
    HeaderFooter,
    /// A watermark (text or picture).
    Watermark,
}

impl MarkKind {
    fn name(self) -> &'static str {
        match self {
            MarkKind::HeaderFooter => "HeaderFooter",
            MarkKind::Watermark => "Watermark",
        }
    }
}

/// Bates numbering: `prefix` + the number with at least `digits` digits + `suffix`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bates {
    /// Text before the number, e.g. `ACME-`.
    pub prefix: String,
    /// Text after the number.
    pub suffix: String,
    /// Number of the first marked page.
    pub start: u64,
    /// Minimum number of digits (zero-padded), 1–12.
    pub digits: usize,
}

impl Default for Bates {
    fn default() -> Self {
        Self {
            prefix: String::new(),
            suffix: String::new(),
            start: 1,
            digits: 6,
        }
    }
}

impl Bates {
    /// The Bates number of the `n`-th marked page (0-based).
    pub fn format(&self, n: usize) -> String {
        let num = self.start.saturating_add(n as u64);
        format!(
            "{}{:0width$}{}",
            self.prefix,
            num,
            self.suffix,
            width = self.digits.clamp(1, 12)
        )
    }
}

/// Headers and footers: six slots, three at the top and three at the bottom.
#[derive(Clone, Debug, PartialEq)]
pub struct HeaderFooter {
    /// Top-left, top-centre, top-right, bottom-left, bottom-centre, bottom-right.
    pub slots: [String; 6],
    /// Font of all slots.
    pub font: FontStyle,
    /// Font size in points.
    pub size: f64,
    /// Text colour (red, green, blue 0–1).
    pub color: (f32, f32, f32),
    /// Distance of the header's top from the page's top edge, in points.
    pub margin_top: f64,
    /// Distance of the footer's baseline area from the page's bottom edge, in points.
    pub margin_bottom: f64,
    /// Distance from the left and right edges, in points.
    pub margin_side: f64,
    /// Number shown by `{page}` on the first marked page.
    pub first_number: usize,
    /// Bates numbering for `{bates}`.
    pub bates: Option<Bates>,
}

impl Default for HeaderFooter {
    fn default() -> Self {
        Self {
            slots: Default::default(),
            font: FontStyle::default(),
            size: 10.0,
            color: (0.0, 0.0, 0.0),
            margin_top: 28.0,
            margin_bottom: 28.0,
            margin_side: 36.0,
            first_number: 1,
            bates: None,
        }
    }
}

/// What a watermark shows.
#[derive(Clone, Debug, PartialEq)]
pub enum WatermarkContent {
    /// A text, one or more lines.
    Text {
        /// What it says.
        text: String,
        /// Font.
        font: FontStyle,
        /// Size in points; `0` = as large as fits across the page.
        size: f64,
        /// Colour.
        color: (f32, f32, f32),
    },
    /// A PNG or JPEG picture, `width` of the page wide (0–1).
    Image {
        /// The file's bytes.
        data: Vec<u8>,
        /// Width as a fraction of the page width.
        width: f64,
    },
}

/// A watermark.
#[derive(Clone, Debug, PartialEq)]
pub struct Watermark {
    /// Text or picture.
    pub content: WatermarkContent,
    /// Opacity 0–1.
    pub opacity: f64,
    /// Counter-clockwise rotation in degrees, around the page centre.
    pub rotation: f64,
    /// Behind the page content instead of on top of it.
    pub behind: bool,
}

/// What the placeholders stand for, besides the page numbers.
#[derive(Clone, Debug, Default)]
pub struct MarkContext {
    /// `{date}`.
    pub date: String,
    /// `{file}`.
    pub file: String,
    /// Page labels for every page of the document (`{label}`); empty = page numbers.
    pub labels: Vec<String>,
}

/// Replace the placeholders of a header/footer text.
pub fn expand(
    template: &str,
    page_number: usize,
    pages: usize,
    label: &str,
    bates: &str,
    ctx: &MarkContext,
) -> String {
    template
        .replace("{page}", &page_number.to_string())
        .replace("{pages}", &pages.to_string())
        .replace("{label}", label)
        .replace("{bates}", bates)
        .replace("{date}", &ctx.date)
        .replace("{file}", &ctx.file)
}

/// The transform from "shown page" space (origin bottom-left of the page as displayed, y up,
/// points) to the page's user space, and the shown width and height.
fn shown_to_user(tx: &Tx<'_>, page: ObjectId) -> Result<(Affine, f64, f64)> {
    let doc = tx.doc();
    let rect =
        |key: &[u8]| objutil::inherited_obj(doc, page, key).and_then(|o| objutil::rect(doc, &o));
    let rotate = objutil::inherited_obj(doc, page, b"Rotate")
        .and_then(|o| objutil::num(doc, &o))
        .and_then(|d| Rotation::from_degrees(d as i64))
        .unwrap_or(Rotation::R0);
    let uu = doc
        .get_dictionary(page)
        .ok()
        .and_then(|d| objutil::dict_num(doc, d, b"UserUnit"))
        .unwrap_or(1.0);
    let g = crate::geom::PageGeometry::new(rect(b"MediaBox"), rect(b"CropBox"), rotate, uu);
    let size = g.view_size(Rotation::R0);
    let flip = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, size.height]);
    Ok((g.view_to_pdf(Rotation::R0) * flip, size.width, size.height))
}

fn cm(a: Affine) -> String {
    a.as_coeffs()
        .iter()
        .map(|v| fmt_num_prec(*v))
        .collect::<Vec<_>>()
        .join(" ")
}

fn rg(c: (f32, f32, f32)) -> String {
    format!(
        "{} {} {} rg",
        fmt_num_prec(f64::from(c.0)),
        fmt_num_prec(f64::from(c.1)),
        fmt_num_prec(f64::from(c.2))
    )
}

fn tagged_stream(kind: &str, content: &[u8]) -> Stream {
    let mut s = Stream::new(
        dictionary! { "Filter" => "FlateDecode", "BergMark" => Object::Name(kind.as_bytes().to_vec()) },
        zlib(content),
    );
    s.allows_compression = false;
    s
}

fn mark_of(doc: &Document, id: ObjectId) -> Option<Vec<u8>> {
    match doc.objects.get(&id) {
        Some(Object::Stream(s)) => s
            .dict
            .get(MARK_KEY)
            .ok()
            .and_then(|o| o.as_name().ok())
            .map(<[u8]>::to_vec),
        _ => None,
    }
}

/// Put a mark around the page's own content. The streams of a marked page are, in order:
///
/// `[marks behind…] [Wrap: q] [the page's own streams] [Unwrap: Q…] [marks on top…]`
///
/// Every mark stream is balanced (`q … Q`) and starts in the page's initial graphics state, so marks
/// can be added and removed in any order. The page's own streams are never changed.
fn wrap_page(
    tx: &mut Tx<'_>,
    page: ObjectId,
    kind: MarkKind,
    before: &str,
    after: &str,
) -> Result<()> {
    let mut arr: Vec<ObjectId> = content_stream_ids(tx.doc(), page);
    if !arr
        .iter()
        .any(|id| mark_of(tx.doc(), *id).as_deref() == Some(b"Wrap"))
    {
        // How many `q` the page's own content leaves open: closed before drawing on top of it.
        let open = if arr.is_empty() {
            0
        } else {
            PageContent::load(tx.doc(), page)
                .map(|pc| pc.end_state().1)
                .unwrap_or(0)
        };
        let wrap = tx.add(Object::Stream(tagged_stream(
            "Wrap", b"q
",
        )));
        let unwrap = tx.add(Object::Stream(tagged_stream(
            "Unwrap",
            "Q
"
            .repeat(open + 1)
            .as_bytes(),
        )));
        arr.insert(0, wrap);
        arr.push(unwrap);
    }
    if !before.is_empty() {
        let id = tx.add(Object::Stream(tagged_stream(
            kind.name(),
            format!(
                "q
{before}Q
"
            )
            .as_bytes(),
        )));
        arr.insert(0, id);
    }
    if !after.is_empty() {
        let id = tx.add(Object::Stream(tagged_stream(
            kind.name(),
            format!(
                "q
{after}Q
"
            )
            .as_bytes(),
        )));
        arr.push(id);
    }
    tx.dict_mut(page)?.set(
        "Contents",
        Object::Array(arr.into_iter().map(reference).collect()),
    );
    Ok(())
}

/// Stamp headers and footers onto `pages` (in document order). Returns the number of pages marked.
pub fn add_header_footer(
    tx: &mut Tx<'_>,
    pages: &[PageId],
    spec: &HeaderFooter,
    ctx: &MarkContext,
) -> Result<usize> {
    if spec.slots.iter().all(|s| s.trim().is_empty()) {
        return Err(EngineError::InvalidArgument(
            "the header and footer are empty".into(),
        ));
    }
    if !(1.0..=200.0).contains(&spec.size) {
        return Err(EngineError::InvalidArgument(
            "the font size must be between 1 and 200 points".into(),
        ));
    }
    if spec.slots.iter().any(|s| s.chars().count() > MAX_SLOT) {
        return Err(EngineError::InvalidArgument(format!(
            "a header or footer text is longer than {MAX_SLOT} characters"
        )));
    }
    let total = crate::pageops::page_order(tx.doc())?.len();
    let all = crate::pageops::page_order(tx.doc())?;
    let mut fb = FontBuilder::new(BundledFace::Styled(spec.font))?;
    // Every text of every page first: one font subset serves all pages.
    let mut planned: Vec<(ObjectId, Vec<(usize, String)>)> = Vec::new();
    for (n, p) in pages.iter().enumerate() {
        let idx = all
            .iter()
            .position(|x| x == p)
            .ok_or(EngineError::NoSuchPage(p.0.0 as usize))?;
        let label = ctx
            .labels
            .get(idx)
            .cloned()
            .unwrap_or_else(|| (idx + 1).to_string());
        let bates = spec.bates.as_ref().map(|b| b.format(n)).unwrap_or_default();
        let texts: Vec<(usize, String)> = spec
            .slots
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.trim().is_empty())
            .map(|(slot, s)| {
                let t = expand(s, spec.first_number + n, total, &label, &bates, ctx);
                (slot, t.replace(['\r', '\n'], " "))
            })
            .collect();
        for (_, t) in &texts {
            let missing = fb.missing_chars(t);
            if !missing.is_empty() {
                return Err(EngineError::MissingGlyphs {
                    font: spec.font.family.title().into(),
                    chars: missing
                        .iter()
                        .map(char::to_string)
                        .collect::<Vec<_>>()
                        .join(" "),
                });
            }
        }
        planned.push((p.0, texts));
    }
    let mut bodies: Vec<(ObjectId, String)> = Vec::new();
    for (page, texts) in &planned {
        let (m, w, h) = shown_to_user(tx, *page)?;
        let mut body = format!("{} cm\n", cm(m));
        for (slot, text) in texts {
            let codes = fb.encode_str(text)?;
            let tw = fb.text_width(text, spec.size);
            let x = match slot % 3 {
                0 => spec.margin_side,
                1 => (w - tw) / 2.0,
                _ => w - spec.margin_side - tw,
            };
            let header = *slot < 3;
            let y = if header {
                h - spec.margin_top - fb.ascent_em() * spec.size
            } else {
                spec.margin_bottom + fb.descent_em().abs() * spec.size
            };
            body.push_str(&format!(
                "/Artifact <</Type /Pagination /Subtype /{}>> BDC\nBT\n/BergHF {} Tf\n{}\n{} {} Td\n{} Tj\nET\nEMC\n",
                if header { "Header" } else { "Footer" },
                fmt_num_prec(spec.size),
                rg(spec.color),
                fmt_num_prec(x),
                fmt_num_prec(y),
                hex_string(&codes)
            ));
        }
        bodies.push((*page, body));
    }
    let font = fb.finish(tx)?;
    for (page, body) in &bodies {
        add_resource(tx, *page, b"Font", "BergHF", Object::Reference(font))?;
        wrap_page(tx, *page, MarkKind::HeaderFooter, "", body)?;
    }
    Ok(bodies.len())
}

/// Stamp a watermark onto `pages`. Returns the number of pages marked.
pub fn add_watermark(tx: &mut Tx<'_>, pages: &[PageId], spec: &Watermark) -> Result<usize> {
    if !(0.0..=1.0).contains(&spec.opacity) || !spec.rotation.is_finite() {
        return Err(EngineError::InvalidArgument(
            "opacity must be between 0 and 1".into(),
        ));
    }
    let gs = tx.add(Object::Dictionary(dictionary! {
        "Type" => "ExtGState",
        "ca" => Object::Real(spec.opacity as f32),
        "CA" => Object::Real(spec.opacity as f32),
    }));
    enum Prepared {
        Text {
            fb: Box<FontBuilder>,
            lines: Vec<String>,
            size: f64,
            color: (f32, f32, f32),
        },
        Image {
            id: ObjectId,
            aspect: f64,
            width: f64,
        },
    }
    let mut prepared = match &spec.content {
        WatermarkContent::Text {
            text,
            font,
            size,
            color,
        } => {
            let lines: Vec<String> = text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect();
            if lines.is_empty() {
                return Err(EngineError::InvalidArgument(
                    "the watermark has no text".into(),
                ));
            }
            if !(0.0..=500.0).contains(size) {
                return Err(EngineError::InvalidArgument(
                    "the font size must be between 1 and 500 points".into(),
                ));
            }
            let fb = FontBuilder::new(BundledFace::Styled(*font))?;
            let missing: Vec<char> = lines.iter().flat_map(|l| fb.missing_chars(l)).collect();
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
            Prepared::Text {
                fb: Box::new(fb),
                lines,
                size: *size,
                color: *color,
            }
        }
        WatermarkContent::Image { data, width } => {
            if !(0.01..=1.0).contains(width) {
                return Err(EngineError::InvalidArgument(
                    "the picture width must be between 1 % and 100 % of the page".into(),
                ));
            }
            let img = crate::imageembed::add_image_xobject(tx, data)?;
            let (iw, ih) = crate::imageembed::dimensions(data)?;
            Prepared::Image {
                id: img.id,
                aspect: f64::from(ih.max(1)) / f64::from(iw.max(1)),
                width: *width,
            }
        }
    };
    let (sin, cos) = spec.rotation.to_radians().sin_cos();
    let mut bodies: Vec<(ObjectId, String)> = Vec::new();
    for p in pages {
        let (m, w, h) = shown_to_user(tx, p.0)?;
        // Around the centre of the shown page, turned counter-clockwise.
        let around = Affine::new([cos, sin, -sin, cos, w / 2.0, h / 2.0]);
        let mut body = format!(
            "/Artifact <</Type /Pagination /Subtype /Watermark>> BDC\nq\n{} cm\n{} cm\n/BergWmGS gs\n",
            cm(m),
            cm(around)
        );
        match &mut prepared {
            Prepared::Text {
                fb,
                lines,
                size,
                color,
            } => {
                let widest = lines
                    .iter()
                    .map(|l| fb.text_width(l, 1.0))
                    .fold(0.0_f64, f64::max)
                    .max(0.1);
                // Fit the diagonal-ish length that stays on the page whatever the angle.
                let room = (w.min(h) / (sin.abs() + cos.abs()).max(0.5)) * 0.9;
                let fs = if *size > 0.0 {
                    *size
                } else {
                    (room / widest).clamp(6.0, 300.0)
                };
                let lh = fb.line_height_em() * fs;
                let block = lh * lines.len() as f64;
                body.push_str(&format!(
                    "BT\n/BergWm {} Tf\n{}\n",
                    fmt_num_prec(fs),
                    rg(*color)
                ));
                let mut prev = (0.0, 0.0);
                for (i, l) in lines.iter().enumerate() {
                    let x = -fb.text_width(l, fs) / 2.0;
                    let y = block / 2.0 - fb.ascent_em() * fs - lh * i as f64;
                    let codes = fb.encode_str(l)?;
                    body.push_str(&format!(
                        "{} {} Td\n{} Tj\n",
                        fmt_num_prec(x - prev.0),
                        fmt_num_prec(y - prev.1),
                        hex_string(&codes)
                    ));
                    prev = (x, y);
                }
                body.push_str("ET\n");
            }
            Prepared::Image { aspect, width, .. } => {
                let iw = w * *width;
                let ih = iw * *aspect;
                body.push_str(&format!(
                    "{} 0 0 {} {} {} cm\n/BergWmIm Do\n",
                    fmt_num_prec(iw),
                    fmt_num_prec(ih),
                    fmt_num_prec(-iw / 2.0),
                    fmt_num_prec(-ih / 2.0)
                ));
            }
        }
        body.push_str("Q\nEMC\n");
        bodies.push((p.0, body));
    }
    let resource = match prepared {
        Prepared::Text { fb, .. } => (b"Font".as_slice(), "BergWm", fb.finish(tx)?),
        Prepared::Image { id, .. } => (b"XObject".as_slice(), "BergWmIm", id),
    };
    for (page, body) in &bodies {
        add_resource(
            tx,
            *page,
            resource.0,
            resource.1,
            Object::Reference(resource.2),
        )?;
        add_resource(tx, *page, b"ExtGState", "BergWmGS", Object::Reference(gs))?;
        if spec.behind {
            wrap_page(tx, *page, MarkKind::Watermark, &format!("q\n{body}Q\n"), "")?;
        } else {
            wrap_page(tx, *page, MarkKind::Watermark, "", body)?;
        }
    }
    Ok(bodies.len())
}

/// Whether any page carries marks of `kind`.
pub fn has_marks(doc: &Document, kind: MarkKind) -> bool {
    crate::pageops::page_order(doc)
        .unwrap_or_default()
        .iter()
        .any(|p| {
            content_stream_ids(doc, p.0)
                .iter()
                .any(|id| mark_of(doc, *id).as_deref() == Some(kind.name().as_bytes()))
        })
}

/// Take every mark of `kind` off every page again. Returns the number of pages changed.
pub fn remove_marks(tx: &mut Tx<'_>, kind: MarkKind) -> Result<usize> {
    let mut changed = 0;
    for p in crate::pageops::page_order(tx.doc())? {
        let ids = content_stream_ids(tx.doc(), p.0);
        let marks: Vec<Option<Vec<u8>>> = ids.iter().map(|id| mark_of(tx.doc(), *id)).collect();
        if !marks
            .iter()
            .any(|m| m.as_deref() == Some(kind.name().as_bytes()))
        {
            continue;
        }
        let is_ours = |m: &[u8]| m == b"Wrap" || m == b"Unwrap";
        let others_left = marks.iter().any(|m| {
            m.as_deref()
                .is_some_and(|m| !is_ours(m) && m != kind.name().as_bytes())
        });
        let keep: Vec<Object> = ids
            .iter()
            .zip(&marks)
            .filter(|(_, m)| match m.as_deref() {
                Some(m) if is_ours(m) => others_left,
                Some(m) => m != kind.name().as_bytes(),
                None => true,
            })
            .map(|(id, _)| reference(*id))
            .collect();
        tx.dict_mut(p.0)?.set("Contents", Object::Array(keep));
        if !others_left {
            strip_resources(tx, p.0)?;
        }
        changed += 1;
    }
    Ok(changed)
}

/// Drop the resource names this module added (only once no mark is left on the page).
fn strip_resources(tx: &mut Tx<'_>, page: ObjectId) -> Result<()> {
    let res = match objutil::inherited_obj(tx.doc(), page, b"Resources")
        .and_then(|o| objutil::deref(tx.doc(), &o).cloned())
    {
        Some(Object::Dictionary(d)) => d,
        _ => return Ok(()),
    };
    let mut res: Dictionary = res;
    for (cat, keys) in [
        (b"Font".as_slice(), &["BergHF", "BergWm"][..]),
        (b"XObject".as_slice(), &["BergWmIm"][..]),
        (b"ExtGState".as_slice(), &["BergWmGS"][..]),
    ] {
        if let Some(Object::Dictionary(mut d)) = res
            .get(cat)
            .ok()
            .and_then(|o| objutil::deref(tx.doc(), o).cloned())
        {
            for k in keys {
                d.remove(k.as_bytes());
            }
            res.set(cat.to_vec(), Object::Dictionary(d));
        }
    }
    tx.dict_mut(page)?.set("Resources", Object::Dictionary(res));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bates_numbers_are_padded_and_counted_up() {
        let b = Bates {
            prefix: "ACME-".into(),
            suffix: "-X".into(),
            start: 98,
            digits: 4,
        };
        assert_eq!(b.format(0), "ACME-0098-X");
        assert_eq!(b.format(3), "ACME-0101-X");
        let wide = Bates {
            digits: 2,
            start: 12345,
            ..Bates::default()
        };
        assert_eq!(wide.format(0), "12345");
    }

    #[test]
    fn placeholders_are_replaced() {
        let ctx = MarkContext {
            date: "2026-11-03".into(),
            file: "plan.pdf".into(),
            labels: Vec::new(),
        };
        assert_eq!(
            expand(
                "Page {page} of {pages} · {label} · {bates} · {date} · {file}",
                3,
                9,
                "iii",
                "B-01",
                &ctx
            ),
            "Page 3 of 9 · iii · B-01 · 2026-11-03 · plan.pdf"
        );
    }
}
