//! "Save As Optimized": produce a smaller copy of a document.
//!
//! Works on a private re-parse of the document bytes, so the open document is never touched.
//! Steps, each of which can only make the file smaller or leave it alone:
//!
//! 1. drop page thumbnails (viewers regenerate them),
//! 2. optionally downsample over-sampled raster images (JPEG and plain 8-bit Flate RGB/Gray),
//!    judged by their *effective* resolution where they are placed on the pages,
//! 3. merge byte-identical streams (repeated logos, fonts, blank pages),
//! 4. re-compress Flate streams at the best level when that is smaller,
//! 5. drop objects nothing refers to (old revisions, replaced images…),
//! 6. write with object streams and a cross-reference stream.
//!
//! The result is validated by re-opening it; if it is not smaller than the input, the input is
//! returned unchanged and the report says so. Digital signatures do not survive: the file is
//! rewritten, so callers must warn about that.

use crate::content::{self, Mat, Operand};
use crate::error::{EngineError, Result};
use crate::objutil;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use std::collections::HashMap;
use std::io::{Read, Write};

const MAX_DEPTH: usize = 6;
const MAX_STREAM_BYTES: usize = 256 * 1024 * 1024;

/// What to do with raster images.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImagePolicy {
    /// Leave every image as it is (lossless optimisation).
    Keep,
    /// Reduce images whose effective resolution exceeds `dpi`; photographic images are
    /// re-encoded as JPEG at `jpeg_quality` (1–100).
    Downsample {
        /// Target resolution in dots per inch.
        dpi: u32,
        /// JPEG quality for photographic images.
        jpeg_quality: u8,
    },
}

/// Options for [`optimize_bytes`].
#[derive(Clone, Debug)]
pub struct OptimizeOptions {
    /// Image handling.
    pub images: ImagePolicy,
    /// Merge identical streams.
    pub merge_duplicates: bool,
    /// Re-compress Flate streams at the best level.
    pub recompress: bool,
    /// Remove page thumbnails.
    pub remove_thumbnails: bool,
    /// Write object streams and a cross-reference stream (PDF 1.5).
    pub object_streams: bool,
}

impl OptimizeOptions {
    /// Nothing is lost: only structure and compression change.
    pub fn lossless() -> Self {
        Self {
            images: ImagePolicy::Keep,
            merge_duplicates: true,
            recompress: true,
            remove_thumbnails: true,
            object_streams: true,
        }
    }

    /// Images above 150 dpi are reduced (JPEG quality 75).
    pub fn balanced() -> Self {
        Self {
            images: ImagePolicy::Downsample {
                dpi: 150,
                jpeg_quality: 75,
            },
            ..Self::lossless()
        }
    }

    /// Images above 96 dpi are reduced (JPEG quality 60).
    pub fn smallest() -> Self {
        Self {
            images: ImagePolicy::Downsample {
                dpi: 96,
                jpeg_quality: 60,
            },
            ..Self::lossless()
        }
    }
}

/// What the optimiser did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OptimizeReport {
    /// Input size in bytes.
    pub before: usize,
    /// Output size in bytes.
    pub after: usize,
    /// Objects removed because nothing referred to them.
    pub unused_objects: usize,
    /// Duplicate streams merged away.
    pub duplicates_merged: usize,
    /// Streams whose Flate data was rewritten smaller.
    pub streams_recompressed: usize,
    /// Images replaced by a smaller version.
    pub images_downsampled: usize,
    /// Thumbnails removed.
    pub thumbnails_removed: usize,
    /// The optimised file was not smaller, so the input is returned unchanged.
    pub kept_original: bool,
}

/// Replace a stream's bytes and keep `/Length` truthful (the writer trusts the dictionary).
fn set_content(s: &mut Stream, data: Vec<u8>) {
    s.dict.set("Length", Object::Integer(data.len() as i64));
    s.content = data;
}

fn err(m: impl Into<String>) -> EngineError {
    EngineError::Save(m.into())
}

fn name_of(o: Option<&Object>) -> Option<&[u8]> {
    o.and_then(|o| o.as_name().ok())
}

/// Optimise `input` and return the new bytes plus a report.
pub fn optimize_bytes(input: &[u8], opts: &OptimizeOptions) -> Result<(Vec<u8>, OptimizeReport)> {
    let mut doc = Document::load_mem(input)
        .map_err(|e| err(format!("cannot read the document for optimisation: {e}")))?;
    let pages_before = doc.get_pages().len();
    let mut rep = OptimizeReport {
        before: input.len(),
        ..OptimizeReport::default()
    };
    if opts.remove_thumbnails {
        rep.thumbnails_removed = remove_thumbnails(&mut doc);
    }
    if let ImagePolicy::Downsample { dpi, jpeg_quality } = opts.images {
        rep.images_downsampled = downsample_images(&mut doc, dpi, jpeg_quality);
    }
    if opts.merge_duplicates {
        rep.duplicates_merged = merge_duplicates(&mut doc);
    }
    if opts.recompress {
        rep.streams_recompressed = recompress(&mut doc);
    }
    // Streams that were stored uncompressed (content streams, XMP is excluded by its filter rule).
    doc.compress();
    rep.unused_objects = doc.prune_objects().len();
    let mut out = Vec::new();
    if opts.object_streams {
        doc.save_modern(&mut out)
            .map_err(|e| err(format!("writing the optimised file failed: {e}")))?;
    } else {
        doc.save_to(&mut out)
            .map_err(|e| err(format!("writing the optimised file failed: {e}")))?;
    }
    // Validate by reading it back.
    let check = Document::load_mem(&out)
        .map_err(|e| err(format!("the optimised file failed validation: {e}")))?;
    if check.get_pages().len() != pages_before {
        return Err(err(format!(
            "the optimised file has {} pages, expected {pages_before}",
            check.get_pages().len()
        )));
    }
    if out.len() >= input.len() {
        rep.kept_original = true;
        rep.after = input.len();
        return Ok((input.to_vec(), rep));
    }
    rep.after = out.len();
    Ok((out, rep))
}

fn remove_thumbnails(doc: &mut Document) -> usize {
    let ids: Vec<ObjectId> = doc.get_pages().values().copied().collect();
    let mut n = 0;
    for id in ids {
        if let Some(Object::Dictionary(d)) = doc.objects.get_mut(&id)
            && d.remove(b"Thumb").is_some()
        {
            n += 1;
        }
    }
    n
}

// ---- duplicates ---------------------------------------------------------------------------

fn stream_key(s: &Stream) -> (Vec<u8>, u64) {
    let mut d = s.dict.clone();
    d.remove(b"Length");
    let mut bytes = Vec::new();
    crate::serialize::write_dict(&mut bytes, &d);
    // FNV-1a over the content; the dictionary is compared exactly.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in &s.content {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    (bytes, h)
}

fn merge_duplicates(doc: &mut Document) -> usize {
    let mut groups: HashMap<(Vec<u8>, u64, usize), Vec<ObjectId>> = HashMap::new();
    for (id, obj) in &doc.objects {
        if let Object::Stream(s) = obj {
            // Object/xref streams are rebuilt by the writer; never merge them.
            if matches!(name_of(s.dict.get(b"Type").ok()), Some(b"ObjStm" | b"XRef")) {
                continue;
            }
            let (d, h) = stream_key(s);
            groups.entry((d, h, s.content.len())).or_default().push(*id);
        }
    }
    let mut remap: HashMap<ObjectId, ObjectId> = HashMap::new();
    for ids in groups.values() {
        if ids.len() < 2 {
            continue;
        }
        let keep = *ids.iter().min().unwrap_or(&ids[0]);
        for id in ids {
            if *id == keep {
                continue;
            }
            // Confirm equality byte for byte (hash collisions are possible).
            let same = match (doc.objects.get(&keep), doc.objects.get(id)) {
                (Some(Object::Stream(a)), Some(Object::Stream(b))) => a.content == b.content,
                _ => false,
            };
            if same {
                remap.insert(*id, keep);
            }
        }
    }
    if remap.is_empty() {
        return 0;
    }
    // Rewrite references everywhere (objects and trailer).
    fn fix(o: &mut Object, remap: &HashMap<ObjectId, ObjectId>) {
        match o {
            Object::Reference(r) => {
                if let Some(k) = remap.get(r) {
                    *r = *k;
                }
            }
            Object::Array(a) => a.iter_mut().for_each(|x| fix(x, remap)),
            Object::Dictionary(d) => d.iter_mut().for_each(|(_, x)| fix(x, remap)),
            Object::Stream(s) => s.dict.iter_mut().for_each(|(_, x)| fix(x, remap)),
            _ => {}
        }
    }
    for obj in doc.objects.values_mut() {
        fix(obj, &remap);
    }
    for (_, v) in doc.trailer.iter_mut() {
        fix(v, &remap);
    }
    for id in remap.keys() {
        doc.objects.remove(id);
    }
    remap.len()
}

// ---- recompression ---------------------------------------------------------------------------

fn only_flate(s: &Stream) -> bool {
    match s.dict.get(b"Filter") {
        Ok(Object::Name(n)) => n == b"FlateDecode",
        Ok(Object::Array(a)) => a.len() == 1 && name_of(a.first()) == Some(b"FlateDecode"),
        _ => false,
    }
}

fn zlib_best(data: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    let _ = enc.write_all(data);
    enc.finish().unwrap_or_default()
}

fn recompress(doc: &mut Document) -> usize {
    let mut n = 0;
    for obj in doc.objects.values_mut() {
        let Object::Stream(s) = obj else { continue };
        if !only_flate(s)
            || matches!(name_of(s.dict.get(b"Type").ok()), Some(b"ObjStm" | b"XRef"))
            || s.content.len() < 64
        {
            continue;
        }
        let mut raw = Vec::new();
        let ok = flate2::read::ZlibDecoder::new(s.content.as_slice())
            .take(MAX_STREAM_BYTES as u64 + 1)
            .read_to_end(&mut raw)
            .is_ok();
        if !ok || raw.len() > MAX_STREAM_BYTES {
            continue;
        }
        let better = zlib_best(&raw);
        if !better.is_empty() && better.len() < s.content.len() {
            set_content(s, better);
            n += 1;
        }
    }
    n
}

// ---- images --------------------------------------------------------------------------------

/// Largest placed size (width, height in points) of every image XObject, by object id.
fn placements(doc: &Document) -> HashMap<ObjectId, (f64, f64)> {
    let mut out = HashMap::new();
    let pages: Vec<ObjectId> = doc.get_pages().values().copied().collect();
    for page in pages {
        let mut data = Vec::new();
        for id in crate::pagecontent::content_stream_ids(doc, page) {
            if let Some(Object::Stream(s)) = doc.objects.get(&id)
                && let Ok(d) = s.decompressed_content_with_limit(MAX_STREAM_BYTES)
            {
                data.extend_from_slice(&d);
                data.push(b'\n');
            }
        }
        let res = objutil::inherited_obj(doc, page, b"Resources");
        let res = res
            .as_ref()
            .and_then(|o| objutil::deref(doc, o))
            .and_then(|o| o.as_dict().ok());
        place_walk(doc, &data, res, Mat::IDENTITY, 0, &mut out);
    }
    out
}

fn place_walk(
    doc: &Document,
    data: &[u8],
    res: Option<&Dictionary>,
    base: Mat,
    depth: usize,
    out: &mut HashMap<ObjectId, (f64, f64)>,
) {
    let Ok(ops) = content::scan(data) else { return };
    let mut ctm = base;
    let mut stack: Vec<Mat> = Vec::new();
    for op in &ops {
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
                let n: Vec<f64> = op.operands.iter().filter_map(Operand::num).collect();
                if n.len() == 6 {
                    ctm = Mat([n[0], n[1], n[2], n[3], n[4], n[5]]).then(&ctm);
                }
            }
            b"Do" if depth < MAX_DEPTH => {
                let (Some(Operand::Name(name)), Some(res)) = (op.operands.first(), res) else {
                    continue;
                };
                let Some(xo) = res
                    .get(b"XObject")
                    .ok()
                    .and_then(|o| objutil::deref(doc, o))
                    .and_then(|o| o.as_dict().ok())
                else {
                    continue;
                };
                let Ok(entry) = xo.get(name) else { continue };
                let Some(Object::Stream(s)) = objutil::deref(doc, entry) else {
                    continue;
                };
                match name_of(s.dict.get(b"Subtype").ok()) {
                    Some(b"Image") => {
                        if let Object::Reference(id) = entry {
                            let [a, b, c, d, ..] = ctm.0;
                            let (w, h) = (a.hypot(b), c.hypot(d));
                            let e = out.entry(*id).or_insert((0.0, 0.0));
                            e.0 = e.0.max(w);
                            e.1 = e.1.max(h);
                        }
                    }
                    Some(b"Form") => {
                        let m = s
                            .dict
                            .get(b"Matrix")
                            .ok()
                            .and_then(|o| objutil::deref(doc, o))
                            .and_then(|o| o.as_array().ok())
                            .and_then(|a| {
                                let v: Vec<f64> =
                                    a.iter().filter_map(|o| objutil::num(doc, o)).collect();
                                (v.len() == 6).then(|| Mat([v[0], v[1], v[2], v[3], v[4], v[5]]))
                            })
                            .unwrap_or(Mat::IDENTITY);
                        let Ok(d) = s.decompressed_content_with_limit(MAX_STREAM_BYTES) else {
                            continue;
                        };
                        let own = s
                            .dict
                            .get(b"Resources")
                            .ok()
                            .and_then(|o| objutil::deref(doc, o))
                            .and_then(|o| o.as_dict().ok())
                            .or(Some(res));
                        place_walk(doc, &d, own, m.then(&ctm), depth + 1, out);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

struct ImageParts {
    width: u32,
    height: u32,
    comps: usize,
    jpeg: bool,
}

fn int(doc: &Document, d: &Dictionary, key: &[u8]) -> Option<i64> {
    objutil::dict_num(doc, d, key).map(|v| v as i64)
}

/// Whether `s` is an image we know how to resample, and its shape.
fn classify(doc: &Document, s: &Stream) -> Option<ImageParts> {
    let d = &s.dict;
    if name_of(d.get(b"Subtype").ok()) != Some(b"Image") {
        return None;
    }
    let truthy = |k: &[u8]| matches!(d.get(k), Ok(Object::Boolean(true)));
    if truthy(b"ImageMask") || d.has(b"Mask") || d.has(b"Decode") {
        return None;
    }
    if int(doc, d, b"BitsPerComponent")? != 8 {
        return None;
    }
    let comps = match d
        .get(b"ColorSpace")
        .ok()
        .and_then(|o| objutil::deref(doc, o))
    {
        Some(Object::Name(n)) if n == b"DeviceRGB" => 3,
        Some(Object::Name(n)) if n == b"DeviceGray" => 1,
        _ => return None,
    };
    let (w, h) = (int(doc, d, b"Width")?, int(doc, d, b"Height")?);
    if w < 1 || h < 1 || w > 16_384 || h > 16_384 || w * h > 120_000_000 {
        return None;
    }
    let filter = match d.get(b"Filter") {
        Ok(Object::Name(n)) => Some(n.as_slice()),
        Ok(Object::Array(a)) if a.len() == 1 => name_of(a.first()),
        _ => None,
    }?;
    let jpeg = match filter {
        b"DCTDecode" => true,
        b"FlateDecode" => {
            // A predictor means the bytes are not plain pixels; leave those alone.
            let pred = d
                .get(b"DecodeParms")
                .ok()
                .and_then(|o| objutil::deref(doc, o))
                .and_then(|o| o.as_dict().ok())
                .and_then(|p| int(doc, p, b"Predictor"))
                .unwrap_or(1);
            if pred > 1 {
                return None;
            }
            false
        }
        _ => return None,
    };
    Some(ImageParts {
        width: w as u32,
        height: h as u32,
        comps,
        jpeg,
    })
}

fn decode_pixels(s: &Stream, p: &ImageParts) -> Option<Vec<u8>> {
    let expect = p.width as usize * p.height as usize * p.comps;
    if p.jpeg {
        let img = image::load_from_memory_with_format(&s.content, image::ImageFormat::Jpeg).ok()?;
        let (w, h) = (img.width(), img.height());
        if (w, h) != (p.width, p.height) {
            return None;
        }
        Some(if p.comps == 1 {
            img.to_luma8().into_raw()
        } else {
            img.to_rgb8().into_raw()
        })
    } else {
        let mut raw = Vec::with_capacity(expect);
        flate2::read::ZlibDecoder::new(s.content.as_slice())
            .take(expect as u64 + 1)
            .read_to_end(&mut raw)
            .ok()?;
        // Streams may carry trailing padding; too short means damaged.
        (raw.len() >= expect).then(|| {
            raw.truncate(expect);
            raw
        })
    }
}

fn resize(px: Vec<u8>, p: &ImageParts, nw: u32, nh: u32) -> Option<Vec<u8>> {
    use image::imageops::{FilterType, resize};
    if p.comps == 1 {
        let img = image::GrayImage::from_raw(p.width, p.height, px)?;
        Some(resize(&img, nw, nh, FilterType::Lanczos3).into_raw())
    } else {
        let img = image::RgbImage::from_raw(p.width, p.height, px)?;
        Some(resize(&img, nw, nh, FilterType::Lanczos3).into_raw())
    }
}

/// Heuristic: many distinct colours means a photograph (JPEG-friendly); few means line art.
fn photographic(px: &[u8], comps: usize) -> bool {
    let mut seen = std::collections::HashSet::new();
    let step = (px.len() / comps / 4096).max(1);
    for i in (0..px.len() / comps).step_by(step) {
        let k = px[i * comps..i * comps + comps]
            .iter()
            .fold(0u32, |a, b| a << 8 | u32::from(*b));
        seen.insert(k);
        if seen.len() > 400 {
            return true;
        }
    }
    false
}

fn jpeg_encode(px: &[u8], w: u32, h: u32, comps: usize, q: u8) -> Option<Vec<u8>> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, q.clamp(1, 100))
        .write_image(
            px,
            w,
            h,
            if comps == 1 {
                image::ExtendedColorType::L8
            } else {
                image::ExtendedColorType::Rgb8
            },
        )
        .ok()?;
    Some(out)
}

fn downsample_images(doc: &mut Document, target_dpi: u32, quality: u8) -> usize {
    let placed = placements(doc);
    let mut count = 0;
    let mut ids: Vec<_> = placed.keys().copied().collect();
    ids.sort();
    for id in ids {
        let (wpt, hpt) = placed[&id];
        if wpt < 1.0 || hpt < 1.0 {
            continue;
        }
        let Some(Object::Stream(s)) = doc.objects.get(&id).cloned() else {
            continue;
        };
        let Some(p) = classify(doc, &s) else { continue };
        let dpi = (f64::from(p.width) * 72.0 / wpt).max(f64::from(p.height) * 72.0 / hpt);
        // Only act when clearly over-sampled; a 20 % margin avoids pointless re-sampling.
        if dpi <= f64::from(target_dpi) * 1.2 {
            continue;
        }
        let k = f64::from(target_dpi) / dpi;
        let nw = ((f64::from(p.width) * k).round() as u32).max(1);
        let nh = ((f64::from(p.height) * k).round() as u32).max(1);
        let Some(px) = decode_pixels(&s, &p) else {
            continue;
        };
        let photo = photographic(&px, p.comps);
        let Some(small) = resize(px, &p, nw, nh) else {
            continue;
        };
        let (content, filter) = if photo {
            match jpeg_encode(&small, nw, nh, p.comps, quality) {
                Some(j) => (j, "DCTDecode"),
                None => continue,
            }
        } else {
            (crate::fontembed::zlib(&small), "FlateDecode")
        };
        if content.len() >= s.content.len() {
            continue;
        }
        // Soft mask: keep it at the same pixel size as the picture.
        let smask = s
            .dict
            .get(b"SMask")
            .ok()
            .and_then(|o| o.as_reference().ok());
        let mut ns = s.clone();
        set_content(&mut ns, content);
        ns.dict.set("Width", i64::from(nw));
        ns.dict.set("Height", i64::from(nh));
        ns.dict
            .set("Filter", Object::Name(filter.as_bytes().to_vec()));
        ns.dict.remove(b"DecodeParms");
        ns.allows_compression = false;
        if let Some(mid) = smask {
            match resample_mask(doc, mid, nw, nh) {
                Some(m) => {
                    doc.objects.insert(mid, Object::Stream(m));
                }
                None => continue, // cannot keep picture and mask consistent: leave both
            }
        }
        doc.objects.insert(id, Object::Stream(ns));
        count += 1;
    }
    count
}

fn resample_mask(doc: &Document, id: ObjectId, nw: u32, nh: u32) -> Option<Stream> {
    let Object::Stream(s) = doc.objects.get(&id)? else {
        return None;
    };
    let p = classify(doc, s)?;
    if p.comps != 1 || p.jpeg {
        return None;
    }
    let px = decode_pixels(s, &p)?;
    let small = resize(px, &p, nw, nh)?;
    let mut ns = s.clone();
    set_content(&mut ns, crate::fontembed::zlib(&small));
    ns.dict.set("Width", i64::from(nw));
    ns.dict.set("Height", i64::from(nh));
    ns.dict.remove(b"DecodeParms");
    ns.allows_compression = false;
    Some(ns)
}

/// Map of the pages of `doc` for tests and callers that only need the count.
pub fn page_count(bytes: &[u8]) -> Option<usize> {
    Document::load_mem(bytes).ok().map(|d| d.get_pages().len())
}
