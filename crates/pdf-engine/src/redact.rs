//! Secure redaction.
//!
//! Redaction is two steps. **Marking** adds standard `/Redact` annotations (a red dashed frame, nothing is
//! removed yet; they can be moved or deleted like any annotation). **Applying** permanently removes what lies
//! under the marks.
//!
//! How a marked page is made safe: the page is *replaced*. Its content (text, pictures, vector drawings, forms,
//! hidden layers, everything that was drawn or only present in the page's resources) is dropped and the page
//! becomes one picture of what it looked like, with the marked areas painted black **in the pixels**. Nothing
//! that was under a mark can survive in the page because the page no longer contains anything but the picture.
//! To keep the page searchable and selectable, the words that are *not* touched by a mark are written back as
//! an invisible text layer (the same mechanism as OCR). Annotations that touch a mark are deleted (comments
//! and form fields under a mark can reveal the same information); the others stay live.
//!
//! The rest is housekeeping so the old content cannot come back from somewhere else: the replaced objects are
//! discarded, the next save is a complete rewrite (an incremental save would keep the old revision inside
//! the file), the tagged-PDF structure tree (which can repeat the text) is dropped, and the apply step lists
//! other places that still contain a removed word (document properties, bookmarks, comments on other
//! pages, form values) so a person can deal with them. See docs/DECISIONS.md D-036.

use crate::annot::{self, AnnotId};
use crate::doc::{PageId, PdfDocument, Tx};
use crate::error::{EngineError, Result};
use crate::fontembed::zlib;
use crate::geom::{Affine, PageGeometry, Point, Rect, Rotation};
use crate::objutil::{self, name, num_array, reference};
use crate::pagecontent::{OcrWord, add_ocr_text_layers};
use crate::render;
use crate::text::TextPage;
use lopdf::{Dictionary, Object, ObjectId, Stream, dictionary};
use std::collections::BTreeSet;
use std::sync::Arc;

/// Name of the picture in a redacted page's resources.
const IMAGE_NAME: &str = "BergRedacted";
/// Most pixels of one redacted page picture (a larger page is drawn at a lower resolution).
pub const MAX_PAGE_PIXELS: f64 = 64_000_000.0;

/// A redaction mark found in a document.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    /// The annotation.
    pub id: AnnotId,
    /// The page it is on.
    pub page: PageId,
    /// The area, in user space.
    pub rect: Rect,
}

/// How the picture of a redacted page is stored.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImageMode {
    /// Flate compression, nothing lost.
    Lossless,
    /// JPEG with this quality (1..=100): smaller, for pages that are mostly photographs.
    Jpeg(u8),
}

/// What to apply and how.
#[derive(Clone, Debug)]
pub struct RedactOptions {
    /// Resolution of the page picture in dots per inch (lower for very large pages).
    pub dpi: f64,
    /// Picture storage.
    pub image: ImageMode,
    /// Also empty the document properties (title, author, …) and the XMP metadata.
    pub remove_metadata: bool,
    /// Also remove embedded files and file attachment annotations.
    pub remove_attachments: bool,
}

impl Default for RedactOptions {
    fn default() -> Self {
        Self {
            dpi: 300.0,
            image: ImageMode::Lossless,
            remove_metadata: false,
            remove_attachments: false,
        }
    }
}

/// What applying redactions did.
#[derive(Clone, Debug, Default)]
pub struct RedactionReport {
    /// Pages that were replaced.
    pub pages: usize,
    /// Marks applied.
    pub marks: usize,
    /// Words removed (every word a mark touched).
    pub words_removed: usize,
    /// Words kept as searchable text.
    pub words_kept: usize,
    /// Annotations (comments, form fields, links…) removed because they lay under a mark.
    pub annotations_removed: usize,
    /// The lowest resolution used, in dpi (below the wanted one for very large pages).
    pub lowest_dpi: f64,
    /// Places outside the redacted pages that still contain a removed word.
    pub leaks: Vec<String>,
}

fn overlaps(a: Rect, b: Rect) -> bool {
    let (a, b) = (a.abs(), b.abs());
    a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1
}

// ---- marking ----------------------------------------------------------------------------------

/// Mark `rect` (user space) on `page` for redaction. Nothing is removed until [`apply`].
pub fn add_mark(tx: &mut Tx<'_>, page: PageId, rect: Rect) -> Result<AnnotId> {
    let r = rect.abs();
    if r.width() < 1.0 || r.height() < 1.0 {
        return Err(EngineError::InvalidArgument(
            "the redaction area is too small".into(),
        ));
    }
    let (w, h) = (r.width(), r.height());
    // Appearance: a see-through red wash and a dashed red frame, so the text underneath can still be read
    // while the marks are reviewed.
    let content = format!(
        "/GS gs\n1 0.2 0.2 rg\n0 0 {w} {h} re f\n/GS2 gs\n0.85 0.05 0.05 RG\n1.5 w [4 3] 0 d\n0.75 0.75 {} {} re S\n",
        objutil::fmt_num(w - 1.5),
        objutil::fmt_num(h - 1.5),
        w = objutil::fmt_num(w),
        h = objutil::fmt_num(h),
    );
    let mut ap = Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => num_array(&[0.0, 0.0, w, h]),
            "Resources" => dictionary! {
                "ExtGState" => dictionary! {
                    "GS" => dictionary! { "ca" => objutil::real(0.25), "CA" => objutil::real(0.25) },
                    "GS2" => dictionary! { "ca" => objutil::real(1.0), "CA" => objutil::real(1.0) },
                },
            },
        },
        content.into_bytes(),
    );
    // The form's own origin is the lower left of its bounding box; /Rect places it.
    ap.dict
        .set("Matrix", num_array(&[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]));
    let ap_id = tx.add(Object::Stream(ap));
    let mut dict = Dictionary::new();
    dict.set("Type", name("Annot"));
    dict.set("Subtype", name("Redact"));
    dict.set("Rect", num_array(&[r.x0, r.y0, r.x1, r.y1]));
    dict.set(
        "QuadPoints",
        num_array(&[r.x0, r.y1, r.x1, r.y1, r.x0, r.y0, r.x1, r.y0]),
    );
    dict.set("C", num_array(&[0.85, 0.05, 0.05]));
    dict.set("IC", num_array(&[0.0, 0.0, 0.0]));
    dict.set("F", 4i64);
    dict.set("Contents", objutil::text_obj(""));
    dict.set("M", Object::string_literal(annot::now_pdf_date()));
    dict.set("P", reference(page.0));
    dict.set("AP", dictionary! { "N" => ap_id });
    let id = tx.add(Object::Dictionary(dict));
    annot::append_annot_ref(tx, page.0, id)?;
    Ok(id)
}

/// Every redaction mark of the document, in page order.
pub fn marks(doc: &PdfDocument) -> Vec<Mark> {
    let mut out = Vec::new();
    for page in doc.page_ids().unwrap_or_default() {
        for id in annot::annotation_ids(doc, page) {
            let d = doc.lopdf();
            let Ok(a) = d.get_dictionary(id) else {
                continue;
            };
            if objutil::dict_name(d, a, b"Subtype") != Some(b"Redact") {
                continue;
            }
            if let Some(rect) = a.get(b"Rect").ok().and_then(|r| objutil::rect(d, r)) {
                out.push(Mark { id, page, rect });
            }
        }
    }
    out
}

// ---- applying ---------------------------------------------------------------------------------

/// Everything worked out for one page before the document is changed.
struct PageJob {
    page: PageId,
    rects: Vec<Rect>,
    image: Stream,
    /// `cm` placing the unit square of the picture on the page.
    placement: [f64; 6],
    words: Vec<OcrWord>,
    removed_words: Vec<String>,
    dpi: f64,
}

/// Permanently remove what lies under every redaction mark of the document. See the module documentation.
///
/// Not undoable and not recorded in a history entry: the caller drops the returned delta's before-images (the
/// session clears its history). Fails without changing the document when something goes wrong.
pub fn apply(doc: &mut PdfDocument, opts: &RedactOptions) -> Result<RedactionReport> {
    let all = marks(doc);
    if all.is_empty() {
        return Err(EngineError::InvalidArgument(
            "there is nothing marked for redaction".into(),
        ));
    }
    let page_ids = doc.page_ids()?;
    let mut by_page: Vec<(usize, PageId, Vec<Rect>)> = Vec::new();
    for (idx, id) in page_ids.iter().enumerate() {
        let rects: Vec<Rect> = all
            .iter()
            .filter(|m| m.page == *id)
            .map(|m| m.rect)
            .collect();
        if !rects.is_empty() {
            by_page.push((idx, *id, rects));
        }
    }

    // 1. Look at the pages as they are now (a picture of each, the words on each).
    let plain = Arc::new(doc.snapshot_bytes()?);
    let mut jobs = Vec::new();
    {
        let geoms: Vec<(usize, PageId, Vec<Rect>, PageGeometry)> = by_page
            .iter()
            .map(|(i, p, r)| Ok((*i, *p, r.clone(), doc.page_geometry(*p)?)))
            .collect::<Result<_>>()?;
        render::with_session(plain, |s| {
            for (idx, page, rects, geom) in &geoms {
                jobs.push(prepare_page(s, *idx, *page, rects, geom, opts)?);
            }
            Ok(())
        })?;
    }

    // 2. Change the document, all or nothing.
    let (removed_annots, _delta) = doc.transact(|tx| {
        let mut removed = 0usize;
        let mut gone: BTreeSet<ObjectId> = BTreeSet::new();
        for job in &jobs {
            replace_page(tx, job)?;
            removed += drop_covered_annotations(tx, job.page, &job.rects, &mut gone)?;
            let words: Vec<OcrWord> = job.words.clone();
            add_ocr_text_layers(tx, &[(job.page, words)])?;
        }
        purge_references(tx, &gone);
        for id in &gone {
            tx.remove(*id);
        }
        sanitize_catalog(tx, opts)?;
        Ok(removed)
    })?;

    // 3. Drop what nothing refers to any more and make the next save a complete rewrite.
    doc.discard_unreferenced_and_rewrite_on_save();

    let removed_text: Vec<String> = jobs
        .iter()
        .flat_map(|j| j.removed_words.iter().cloned())
        .collect();
    let redacted_pages: BTreeSet<PageId> = jobs.iter().map(|j| j.page).collect();
    let mut leaks = find_leaks(doc, &removed_text);
    leaks.extend(find_text_elsewhere(doc, &removed_text, &redacted_pages));
    Ok(RedactionReport {
        pages: jobs.len(),
        marks: all.len(),
        words_removed: removed_text.len(),
        words_kept: jobs.iter().map(|j| j.words.len()).sum(),
        annotations_removed: removed_annots,
        lowest_dpi: jobs.iter().map(|j| j.dpi).fold(f64::MAX, f64::min),
        leaks,
    })
}

fn prepare_page(
    s: &render::Session<'_>,
    index: usize,
    page: PageId,
    rects: &[Rect],
    geom: &PageGeometry,
    opts: &RedactOptions,
) -> Result<PageJob> {
    // The words, and which of them a mark touches.
    let text = s.extract_text(index, geom)?;
    let (kept, removed) = split_words(&text, rects);

    // The picture of the page: its content only, annotations stay live.
    let view = geom.view_size(Rotation::R0);
    let wanted = opts.dpi.clamp(36.0, 600.0) / 72.0;
    let capped = (MAX_PAGE_PIXELS / (view.width * view.height).max(1.0)).sqrt();
    let scale = wanted.min(capped);
    let (w, h) = (
        (view.width * scale).ceil().max(1.0) as u32,
        (view.height * scale).ceil().max(1.0) as u32,
    );
    let mut rgb = render_rgb(s, index, geom, scale, w, h)?;
    // Burn the marks into the pixels, a pixel wider than the mark on every side.
    let to_view = geom.pdf_to_view(Rotation::R0);
    for r in rects {
        let corners = [
            Point::new(r.x0, r.y0),
            Point::new(r.x1, r.y0),
            Point::new(r.x1, r.y1),
            Point::new(r.x0, r.y1),
        ]
        .map(|p| to_view * p);
        let min_x = corners.iter().map(|p| p.x).fold(f64::MAX, f64::min) * scale;
        let max_x = corners.iter().map(|p| p.x).fold(f64::MIN, f64::max) * scale;
        let min_y = corners.iter().map(|p| p.y).fold(f64::MAX, f64::min) * scale;
        let max_y = corners.iter().map(|p| p.y).fold(f64::MIN, f64::max) * scale;
        let x0 = (min_x.floor() - 1.0).max(0.0) as u32;
        let y0 = (min_y.floor() - 1.0).max(0.0) as u32;
        let x1 = ((max_x.ceil() + 1.0).max(0.0) as u32).min(w);
        let y1 = ((max_y.ceil() + 1.0).max(0.0) as u32).min(h);
        for y in y0..y1 {
            let row = (y as usize * w as usize + x0 as usize) * 3;
            let len = (x1.saturating_sub(x0)) as usize * 3;
            rgb[row..row + len].fill(0);
        }
    }
    let image = encode_image(w, h, &rgb, opts.image)?;
    Ok(PageJob {
        page,
        rects: rects.to_vec(),
        image,
        placement: placement(geom, view.width, view.height),
        words: kept,
        removed_words: removed,
        dpi: scale * 72.0,
    })
}

/// The matrix that puts the unit square of the picture (origin lower left, top row up) over the page as it is
/// displayed, in the page's own, unrotated user space (the page keeps its `/Rotate`).
fn placement(geom: &PageGeometry, view_w: f64, view_h: f64) -> [f64; 6] {
    let back: Affine = geom.view_to_pdf(Rotation::R0);
    let origin = back * Point::new(0.0, view_h);
    let x_end = back * Point::new(view_w, view_h);
    let y_end = back * Point::new(0.0, 0.0);
    [
        x_end.x - origin.x,
        x_end.y - origin.y,
        y_end.x - origin.x,
        y_end.y - origin.y,
        origin.x,
        origin.y,
    ]
}

/// RGB8 pixels of a whole page (rendered in tiles), without annotations.
fn render_rgb(
    s: &render::Session<'_>,
    index: usize,
    geom: &PageGeometry,
    scale: f64,
    w: u32,
    h: u32,
) -> Result<Vec<u8>> {
    let mut out = vec![255u8; w as usize * h as usize * 3];
    s.render_tiled(
        index,
        *geom,
        Rotation::R0,
        scale,
        false,
        &mut |x, y, bmp| {
            for row in 0..bmp.height {
                let src =
                    &bmp.rgba[(row * bmp.width) as usize * 4..((row + 1) * bmp.width) as usize * 4];
                let dst = ((y + row) as usize * w as usize + x as usize) * 3;
                for (i, px) in src.chunks_exact(4).enumerate() {
                    out[dst + i * 3..dst + i * 3 + 3].copy_from_slice(&px[..3]);
                }
            }
            Ok(())
        },
    )?;
    Ok(out)
}

fn encode_image(w: u32, h: u32, rgb: &[u8], mode: ImageMode) -> Result<Stream> {
    let gray = rgb.chunks_exact(3).all(|p| p[0] == p[1] && p[1] == p[2]);
    let (cs, samples): (&str, Vec<u8>) = if gray {
        ("DeviceGray", rgb.chunks_exact(3).map(|p| p[0]).collect())
    } else {
        ("DeviceRGB", rgb.to_vec())
    };
    let mut dict = dictionary! {
        "Type" => "XObject", "Subtype" => "Image",
        "Width" => i64::from(w), "Height" => i64::from(h),
        "ColorSpace" => cs, "BitsPerComponent" => 8i64,
    };
    let data = match mode {
        ImageMode::Lossless => {
            dict.set("Filter", "FlateDecode");
            zlib(&samples)
        }
        ImageMode::Jpeg(q) => {
            use image::{ExtendedColorType, ImageEncoder};
            let mut out = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, q.clamp(1, 100))
                .write_image(
                    &samples,
                    w,
                    h,
                    if gray {
                        ExtendedColorType::L8
                    } else {
                        ExtendedColorType::Rgb8
                    },
                )
                .map_err(|e| EngineError::Unsupported(format!("JPEG encoding failed: {e}")))?;
            dict.set("Filter", "DCTDecode");
            out
        }
    };
    let mut s = Stream::new(dict, data);
    s.allows_compression = false;
    Ok(s)
}

/// Split the words of a page into those a mark does not touch (kept, as text with a box) and the text of those
/// it does (removed).
fn split_words(tp: &TextPage, rects: &[Rect]) -> (Vec<OcrWord>, Vec<String>) {
    let mut kept = Vec::new();
    let mut removed = Vec::new();
    let mut text = String::new();
    let mut bounds: Option<Rect> = None;
    let mut hit = false;
    let mut flush = |text: &mut String, bounds: &mut Option<Rect>, hit: &mut bool| {
        if let Some(b) = bounds.take()
            && !text.is_empty()
        {
            if *hit {
                removed.push(std::mem::take(text));
            } else {
                kept.push(OcrWord {
                    text: std::mem::take(text),
                    rect: b,
                });
            }
        }
        text.clear();
        *hit = false;
    };
    for g in &tp.glyphs {
        if g.synthetic || g.text.trim().is_empty() {
            flush(&mut text, &mut bounds, &mut hit);
            continue;
        }
        let b = g.quad.bounds().abs();
        if !(b.is_finite() && b.width() > 0.0 && b.height() > 0.0) {
            continue;
        }
        text.push_str(&g.text);
        bounds = Some(bounds.map_or(b, |a| a.union(b)));
        // A glyph a mark touches takes its whole word with it: half a word would still tell.
        hit |= rects.iter().any(|r| overlaps(*r, b));
    }
    flush(&mut text, &mut bounds, &mut hit);
    (kept, removed)
}

/// Make the page the picture, nothing else.
fn replace_page(tx: &mut Tx<'_>, job: &PageJob) -> Result<()> {
    let image = tx.add(Object::Stream(job.image.clone()));
    let m = job.placement;
    let content = format!(
        "q\n{} cm\n/{IMAGE_NAME} Do\nQ\n",
        crate::content::Mat(m).operands()
    );
    let contents = tx.add(Object::Stream(Stream::new(
        Dictionary::new(),
        content.into_bytes(),
    )));
    let page = tx.dict_mut(job.page.0)?;
    page.set("Contents", reference(contents));
    page.set(
        "Resources",
        dictionary! { "XObject" => dictionary! { IMAGE_NAME => image } },
    );
    // Things a page can carry besides its content that may show or repeat it.
    for key in [
        &b"Thumb"[..],
        b"PieceInfo",
        b"Metadata",
        b"AA",
        b"StructParents",
        b"LastModified",
        b"B",
    ] {
        page.remove(key);
    }
    Ok(())
}

/// Delete the marks and every other annotation that touches a mark (and their pop-ups).
fn drop_covered_annotations(
    tx: &mut Tx<'_>,
    page: PageId,
    rects: &[Rect],
    gone: &mut BTreeSet<ObjectId>,
) -> Result<usize> {
    let ids: Vec<ObjectId> = {
        let d = tx.doc();
        let Ok(pd) = d.get_dictionary(page.0) else {
            return Ok(0);
        };
        objutil::dict_array(d, pd, b"Annots")
            .map(|a| a.iter().filter_map(|o| o.as_reference().ok()).collect())
            .unwrap_or_default()
    };
    let mut n = 0;
    for id in ids {
        let (is_mark, covered, popup) = {
            let d = tx.doc();
            let Ok(a) = d.get_dictionary(id) else {
                continue;
            };
            let mark = objutil::dict_name(d, a, b"Subtype") == Some(b"Redact");
            let r = a.get(b"Rect").ok().and_then(|r| objutil::rect(d, r));
            let covered = r.is_some_and(|r| rects.iter().any(|m| overlaps(*m, r)));
            let popup = a.get(b"Popup").ok().and_then(|p| p.as_reference().ok());
            (mark, covered, popup)
        };
        if is_mark || covered {
            if !is_mark {
                n += 1;
            }
            annot::delete_annotation(tx, page, id)?;
            gone.insert(id);
            if let Some(p) = popup {
                gone.insert(p);
            }
        }
    }
    Ok(n)
}

/// Remove references to deleted objects from every array and dictionary that holds one (a form field's kids,
/// another annotation's `/Popup`…).
fn purge_references(tx: &mut Tx<'_>, gone: &BTreeSet<ObjectId>) {
    if gone.is_empty() {
        return;
    }
    let ids: Vec<ObjectId> = tx.doc().objects.keys().copied().collect();
    for id in ids {
        if gone.contains(&id) {
            continue;
        }
        let touches = match tx.doc().objects.get(&id) {
            Some(Object::Dictionary(d)) => dict_refs(d, gone),
            Some(Object::Array(a)) => array_refs(a, gone),
            Some(Object::Stream(s)) => dict_refs(&s.dict, gone),
            _ => false,
        };
        if !touches {
            continue;
        }
        if let Ok(obj) = tx.object_mut(id) {
            match obj {
                Object::Dictionary(d) => strip_dict(d, gone),
                Object::Array(a) => a.retain(|o| !is_gone(o, gone)),
                Object::Stream(s) => strip_dict(&mut s.dict, gone),
                _ => {}
            }
        }
    }
}

fn is_gone(o: &Object, gone: &BTreeSet<ObjectId>) -> bool {
    matches!(o, Object::Reference(r) if gone.contains(r))
}

fn array_refs(a: &[Object], gone: &BTreeSet<ObjectId>) -> bool {
    a.iter().any(|o| is_gone(o, gone))
}

fn dict_refs(d: &Dictionary, gone: &BTreeSet<ObjectId>) -> bool {
    d.iter().any(|(_, v)| match v {
        Object::Reference(_) => is_gone(v, gone),
        Object::Array(a) => array_refs(a, gone),
        _ => false,
    })
}

fn strip_dict(d: &mut Dictionary, gone: &BTreeSet<ObjectId>) {
    let keys: Vec<Vec<u8>> = d
        .iter()
        .filter(|(_, v)| is_gone(v, gone))
        .map(|(k, _)| k.clone())
        .collect();
    for k in keys {
        d.remove(&k);
    }
    for (_, v) in d.iter_mut() {
        if let Object::Array(a) = v {
            a.retain(|o| !is_gone(o, gone));
        }
    }
}

/// Document-level clean-up after pages were replaced.
fn sanitize_catalog(tx: &mut Tx<'_>, opts: &RedactOptions) -> Result<()> {
    let root = tx
        .doc()
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .map_err(|_| EngineError::Malformed("missing /Root".into()))?;
    // The structure tree of a tagged PDF can repeat the text (alternative text, actual text); it also points
    // at content that no longer exists.
    {
        let cat = tx.dict_mut(root)?;
        cat.remove(b"StructTreeRoot");
        cat.remove(b"MarkInfo");
    }
    if opts.remove_attachments {
        let names = tx
            .doc()
            .get_dictionary(root)
            .ok()
            .and_then(|c| c.get(b"Names").ok())
            .and_then(|n| match n {
                Object::Reference(r) => Some(*r),
                _ => None,
            });
        if let Ok(Object::Dictionary(n)) = tx.dict_mut(root)?.get_mut(b"Names") {
            n.remove(b"EmbeddedFiles");
        }
        if let Some(r) = names {
            tx.dict_mut(r)?.remove(b"EmbeddedFiles");
        }
        tx.dict_mut(root)?.remove(b"AF");
        // File attachment annotations on every page.
        for page in tx_page_ids(tx) {
            let ids: Vec<ObjectId> = {
                let d = tx.doc();
                let Ok(pd) = d.get_dictionary(page.0) else {
                    continue;
                };
                objutil::dict_array(d, pd, b"Annots")
                    .map(|a| a.iter().filter_map(|o| o.as_reference().ok()).collect())
                    .unwrap_or_default()
            };
            for id in ids {
                let is_file = tx.doc().get_dictionary(id).is_ok_and(|a| {
                    objutil::dict_name(tx.doc(), a, b"Subtype") == Some(b"FileAttachment")
                });
                if is_file {
                    annot::delete_annotation(tx, page, id)?;
                    tx.remove(id);
                }
            }
        }
    }
    if opts.remove_metadata {
        tx.dict_mut(root)?.remove(b"Metadata");
        let info = tx
            .doc()
            .trailer
            .get(b"Info")
            .ok()
            .and_then(|i| i.as_reference().ok());
        if let Some(info) = info
            && let Ok(d) = tx.dict_mut(info)
        {
            let keys: Vec<Vec<u8>> = d.iter().map(|(k, _)| k.clone()).collect();
            for k in keys {
                d.remove(&k);
            }
        }
    }
    Ok(())
}

fn tx_page_ids(tx: &Tx<'_>) -> Vec<PageId> {
    tx.doc().page_iter().map(PageId).collect()
}

// ---- what may still hold the removed text -----------------------------------------------------

/// Places that still contain one of `words` (case-insensitively, words of three or more characters): the
/// document properties and XMP metadata, bookmarks, comments and form values.
fn find_leaks(doc: &PdfDocument, words: &[String]) -> Vec<String> {
    let needles = needles_of(words);
    if needles.is_empty() {
        return Vec::new();
    }
    let d = doc.lopdf();
    let hit = |s: &str| {
        let s = s.to_lowercase();
        needles.iter().find(|n| s.contains(n.as_str())).cloned()
    };
    let mut out = Vec::new();
    let note = |what: &str, text: &str, out: &mut Vec<String>| {
        if let Some(n) = hit(text) {
            let line = format!("{what}: “{}” contains “{n}”", shorten(text));
            if !out.contains(&line) {
                out.push(line);
            }
        }
    };
    // Document properties.
    if let Some(info) = d
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|i| objutil::deref(d, i))
        .and_then(|o| o.as_dict().ok())
    {
        for (k, v) in info.iter() {
            if let Some(s) = string_of(d, v) {
                note(
                    &format!("Document property {}", String::from_utf8_lossy(k)),
                    &s,
                    &mut out,
                );
            }
        }
    }
    // XMP metadata.
    if let Some(Object::Stream(s)) = d
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Metadata").ok())
        .and_then(|m| objutil::deref(d, m))
    {
        let data = s
            .decompressed_content()
            .unwrap_or_else(|_| s.content.clone());
        note("XMP metadata", &String::from_utf8_lossy(&data), &mut out);
    }
    // Bookmarks, comments, form values, alternative texts: the strings under the keys that carry text.
    for (id, obj) in &d.objects {
        let Object::Dictionary(dict) = obj else {
            continue;
        };
        for (key, what) in [
            (&b"Title"[..], "Bookmark or title"),
            (b"Contents", "Comment"),
            (b"V", "Form field value"),
            (b"Alt", "Alternative text"),
            (b"ActualText", "Replacement text"),
            (b"Subj", "Comment subject"),
            (b"TU", "Form field tooltip"),
        ] {
            if let Some(s) = dict.get(key).ok().and_then(|v| string_of(d, v)) {
                note(&format!("{what} (object {})", id.0), &s, &mut out);
            }
        }
    }
    out
}

/// Most pages searched for the removed words on the pages that were not redacted.
const MAX_PAGES_CHECKED: usize = 600;

/// The pages that were not redacted but still show a removed word (the same name often appears on many pages).
fn find_text_elsewhere(
    doc: &mut PdfDocument,
    words: &[String],
    redacted: &BTreeSet<PageId>,
) -> Vec<String> {
    let needles = needles_of(words);
    let Ok(ids) = doc.page_ids() else {
        return Vec::new();
    };
    if needles.is_empty() || ids.len() == redacted.len() {
        return Vec::new();
    }
    let Ok(bytes) = doc.snapshot_bytes() else {
        return Vec::new();
    };
    let mut geoms = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        if !redacted.contains(id)
            && geoms.len() < MAX_PAGES_CHECKED
            && let Ok(g) = doc.page_geometry(*id)
        {
            geoms.push((i, g));
        }
    }
    let checked_all = ids.len() - redacted.len() <= MAX_PAGES_CHECKED;
    let mut out = render::with_session(Arc::new(bytes), |s| {
        let mut out = Vec::new();
        for (i, g) in &geoms {
            let Ok(tp) = s.extract_text(*i, g) else {
                continue;
            };
            let text = tp.plain_text().to_lowercase();
            if let Some(n) = needles.iter().find(|n| text.contains(n.as_str())) {
                out.push(format!(
                    "Page {}: the page text still contains “{n}”",
                    i + 1
                ));
            }
        }
        Ok(out)
    })
    .unwrap_or_default();
    if !checked_all {
        out.push(format!(
            "Only the first {MAX_PAGES_CHECKED} pages without redactions were searched for the removed words"
        ));
    }
    out
}

fn needles_of(words: &[String]) -> BTreeSet<String> {
    words
        .iter()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| w.chars().count() >= 3)
        .collect()
}

fn string_of(doc: &lopdf::Document, o: &Object) -> Option<String> {
    match objutil::deref(doc, o)? {
        Object::String(b, _) => Some(objutil::decode_text_string(b)),
        _ => None,
    }
}

fn shorten(s: &str) -> String {
    let t: String = s.chars().filter(|c| !c.is_control()).take(60).collect();
    if s.chars().count() > 60 {
        format!("{t}…")
    } else {
        t
    }
}
