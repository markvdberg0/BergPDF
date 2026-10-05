//! Inline translation: find the paragraphs of a page, and put translated text where the original
//! was.
//!
//! The original text is removed from the page content where the engine can edit it, and every
//! block is covered with the sampled background colour regardless (some text lives in places the
//! editor does not reach, such as form XObjects, or in fonts it cannot edit). The translation is
//! then drawn in the same box with a bundled or installed font, shrunk if it needs more room.

use crate::annot::{cm_op, local_size, rotated_frame, wrap_lines};
use crate::doc::{PageId, Tx};
use crate::error::Result;
use crate::fontembed::{BundledFace, FontBuilder, FontFamily, FontStyle};
use crate::geom::{Affine, PageGeometry, Point, Rect, Rotation};
use crate::objutil::fmt_num_prec;
use crate::pagecontent::{self, ObjRef, PageContent};
use crate::render::Bitmap;
use crate::text::TextPage;
use lopdf::Object;
use std::collections::{BTreeSet, HashMap};

/// A paragraph-like piece of text with its box.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    /// Box in user space.
    pub rect: Rect,
    /// Direction the text runs, degrees counter-clockwise in user space (0, 90, 180, 270).
    pub rotation: i32,
    /// Typical font size in points.
    pub font_size: f64,
    /// The text, lines joined into running text.
    pub text: String,
}

/// A translated block ready to be placed.
#[derive(Clone, Debug)]
pub struct Item {
    /// The original block.
    pub block: Block,
    /// Its translation.
    pub translation: String,
    /// Background colour behind the original (RGB 0..1), see [`sample_background`].
    pub background: (f32, f32, f32),
}

/// What [`apply_translation`] did.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Blocks placed.
    pub blocks: usize,
    /// Original text runs removed from the page content.
    pub runs_removed: usize,
    /// Blocks whose translation needed a smaller font to fit.
    pub shrunk: usize,
    /// Blocks that still overflow their box at the smallest size.
    pub overflowing: usize,
    /// Characters no available font has (shown as "?").
    pub missing_chars: BTreeSet<char>,
    /// Names of installed fonts that had to be used for some blocks.
    pub system_fonts: BTreeSet<String>,
}

struct Seg {
    rect: Rect,
    text: String,
    fs: f64,
    rot: i32,
    s0: f64,
    s1: f64,
    u0: f64,
    u1: f64,
}

/// Coordinates of `p` along the text direction and "up" across it, for text rotated `rot`.
fn frame_coords(rot: i32, p: Point) -> (f64, f64) {
    match rot {
        90 => (p.y, -p.x),
        180 => (-p.x, -p.y),
        270 => (-p.y, p.x),
        _ => (p.x, p.y),
    }
}

fn rot_of(angle: f64) -> Option<i32> {
    let q = (angle / std::f64::consts::FRAC_PI_2).round();
    ((angle - q * std::f64::consts::FRAC_PI_2).abs() < 0.2).then(|| ((q as i32).rem_euclid(4)) * 90)
}

fn is_marker(text: &str) -> bool {
    let t = text.trim_start();
    let mut chars = t.chars();
    match chars.next() {
        Some('•' | '·' | '▪' | '–' | '—' | '*' | '-') => {
            chars.next().is_none_or(char::is_whitespace)
        }
        Some(c) if c.is_ascii_digit() => {
            let rest: String = t
                .chars()
                .skip_while(|c| c.is_ascii_digit())
                .take(2)
                .collect();
            rest.starts_with(". ") || rest.starts_with(") ")
        }
        _ => false,
    }
}

fn segments(tp: &TextPage) -> Vec<Seg> {
    let mut out = Vec::new();
    for line in &tp.lines {
        let Some(first) = tp.glyphs[line.glyphs.clone()]
            .iter()
            .find(|g| !g.synthetic && !g.invisible)
        else {
            continue;
        };
        let Some(rot) = rot_of(first.angle) else {
            continue;
        };
        let mut cur: Option<(Seg, Vec<f64>)> = None;
        let flush = |cur: &mut Option<(Seg, Vec<f64>)>, out: &mut Vec<Seg>| {
            if let Some((mut s, sizes)) = cur.take() {
                s.text = s.text.trim().to_string();
                if !s.text.is_empty() && !sizes.is_empty() {
                    s.fs = sizes.iter().sum::<f64>() / sizes.len() as f64;
                    out.push(s);
                }
            }
        };
        for g in &tp.glyphs[line.glyphs.clone()] {
            if g.invisible {
                continue;
            }
            if g.synthetic {
                if let Some((s, _)) = cur.as_mut() {
                    s.text.push(' ');
                }
                continue;
            }
            let b = g.quad.bounds();
            let cs: Vec<(f64, f64)> = g.quad.0.iter().map(|p| frame_coords(rot, *p)).collect();
            let g0 = cs.iter().map(|c| c.0).fold(f64::MAX, f64::min);
            let g1 = cs.iter().map(|c| c.0).fold(f64::MIN, f64::max);
            let h0 = cs.iter().map(|c| c.1).fold(f64::MAX, f64::min);
            let h1 = cs.iter().map(|c| c.1).fold(f64::MIN, f64::max);
            let fs = g.font_size.max(1.0);
            if let Some((s, _)) = &cur
                && g0 - s.s1 > 1.5 * fs
            {
                flush(&mut cur, &mut out);
            }
            match cur.as_mut() {
                Some((s, sizes)) => {
                    s.rect = s.rect.union(b);
                    s.s0 = s.s0.min(g0);
                    s.s1 = s.s1.max(g1);
                    s.u0 = s.u0.min(h0);
                    s.u1 = s.u1.max(h1);
                    s.text.push_str(&g.text);
                    sizes.push(fs);
                }
                None => {
                    cur = Some((
                        Seg {
                            rect: b,
                            text: g.text.clone(),
                            fs,
                            rot,
                            s0: g0,
                            s1: g1,
                            u0: h0,
                            u1: h1,
                        },
                        vec![fs],
                    ));
                }
            }
        }
        flush(&mut cur, &mut out);
    }
    out
}

struct Para {
    segs: Vec<Seg>,
    closed: bool,
}

fn compatible(para: &Para, n: &Seg) -> Option<f64> {
    if para.closed {
        return None;
    }
    let p = para.segs.last()?;
    if p.rot != n.rot || is_marker(&n.text) {
        return None;
    }
    let ratio = n.fs / p.fs;
    if !(0.85..=1.18).contains(&ratio) {
        return None;
    }
    let fs = p.fs;
    let gap = p.u0 - n.u1;
    if !(-0.5 * fs..=0.5 * fs).contains(&gap) {
        return None;
    }
    if n.s0 >= p.s1 || n.s1 <= p.s0 {
        return None;
    }
    let aligned = (p.s0 - n.s0).abs() <= 2.5 * fs
        || ((p.s0 + p.s1) / 2.0 - (n.s0 + n.s1) / 2.0).abs() <= 1.5 * fs
        || (p.s1 - n.s1).abs() <= 1.5 * fs;
    // A paragraph whose previous line stops well short of its widest line has ended.
    let widest = para.segs.iter().map(|s| s.s1).fold(f64::MIN, f64::max);
    let ended = para.segs.len() >= 2 && p.s1 < widest - 4.0 * fs;
    (aligned && !ended).then_some(gap.abs())
}

fn join_lines(lines: &[&str]) -> String {
    let mut out = String::new();
    for l in lines {
        let l = l.trim();
        if l.is_empty() {
            continue;
        }
        if out.is_empty() {
            out.push_str(l);
            continue;
        }
        let hyphenated = out.ends_with('-')
            && out.chars().rev().nth(1).is_some_and(char::is_alphabetic)
            && l.chars().next().is_some_and(char::is_lowercase);
        if hyphenated {
            out.pop();
        } else {
            out.push(' ');
        }
        out.push_str(l);
    }
    out
}

/// The paragraphs of a page, in reading order. Page numbers, bare numbers and symbols are left
/// out (nothing to translate).
pub fn blocks_of(tp: &TextPage) -> Vec<Block> {
    let mut paras: Vec<Para> = Vec::new();
    for seg in segments(tp) {
        let start = paras.len().saturating_sub(40);
        let best = paras[start..]
            .iter()
            .enumerate()
            .filter_map(|(i, p)| compatible(p, &seg).map(|g| (start + i, g)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i);
        match best {
            Some(i) => paras[i].segs.push(seg),
            None => paras.push(Para {
                segs: vec![seg],
                closed: false,
            }),
        }
    }
    paras
        .into_iter()
        .filter_map(|p| {
            let lines: Vec<&str> = p.segs.iter().map(|s| s.text.as_str()).collect();
            let text = join_lines(&lines);
            if text.chars().filter(|c| c.is_alphabetic()).count() < 2 {
                return None;
            }
            let rect = p
                .segs
                .iter()
                .skip(1)
                .fold(p.segs[0].rect, |r, s| r.union(s.rect));
            let mut sizes: Vec<f64> = p.segs.iter().map(|s| s.fs).collect();
            sizes.sort_by(f64::total_cmp);
            Some(Block {
                rect,
                rotation: p.segs[0].rot,
                font_size: sizes[sizes.len() / 2],
                text,
            })
        })
        .collect()
}

/// The colour behind `rect`: the most common colour on a thin ring just outside it in a render of
/// the page at `scale` pixels per point.
pub fn sample_background(
    bmp: &Bitmap,
    geometry: &PageGeometry,
    scale: f64,
    rect: Rect,
) -> (f32, f32, f32) {
    let m: Affine = geometry.pdf_to_view(Rotation::R0);
    let pad = 2.0 / scale;
    let r = rect.abs().inflate(pad, pad);
    let mut pts = Vec::new();
    for i in 0..=24 {
        let t = f64::from(i) / 24.0;
        let (x, y) = (r.x0 + t * r.width(), r.y0 + t * r.height());
        pts.extend([
            Point::new(x, r.y0),
            Point::new(x, r.y1),
            Point::new(r.x0, y),
            Point::new(r.x1, y),
        ]);
    }
    let mut buckets: HashMap<(u8, u8, u8), (u32, [u32; 3])> = HashMap::new();
    for p in pts {
        let v = m * p;
        let (px, py) = ((v.x * scale).round(), (v.y * scale).round());
        if px < 0.0 || py < 0.0 || px >= f64::from(bmp.width) || py >= f64::from(bmp.height) {
            continue;
        }
        let i = ((py as usize) * bmp.width as usize + px as usize) * 4;
        let Some(px) = bmp.rgba.get(i..i + 3) else {
            continue;
        };
        let e = buckets
            .entry((px[0] >> 3, px[1] >> 3, px[2] >> 3))
            .or_insert((0, [0; 3]));
        e.0 += 1;
        for (acc, v) in e.1.iter_mut().zip(px.iter()) {
            *acc += u32::from(*v);
        }
    }
    buckets
        .values()
        .max_by_key(|(n, _)| *n)
        .map_or((1.0, 1.0, 1.0), |(n, s)| {
            let f = |k: usize| (s[k] as f32 / *n as f32) / 255.0;
            (f(0), f(1), f(2))
        })
}

fn luminance(c: (f32, f32, f32)) -> f32 {
    0.2126 * c.0 + 0.7152 * c.1 + 0.0722 * c.2
}

/// A font for `text`: Liberation Sans, else DejaVu Sans, else an installed font that covers it.
/// Characters nothing covers are replaced by "?" (and reported).
fn choose_font(text: &str, bold: bool, italic: bool, report: &mut Report) -> (FontStyle, String) {
    let sans = FontStyle::new(FontFamily::LiberationSans, bold, italic);
    let dejavu = FontStyle::new(FontFamily::DejaVuSans, bold, false);
    for style in [sans, dejavu] {
        if let Ok(fb) = FontBuilder::new(BundledFace::Styled(style))
            && fb.missing_chars(text).is_empty()
        {
            return (style, text.to_string());
        }
    }
    let missing: Vec<char> = FontBuilder::new(BundledFace::Styled(dejavu))
        .map(|fb| fb.missing_chars(text))
        .unwrap_or_default();
    if let Some(id) = crate::sysfonts::family_covering(&missing) {
        let fam = FontFamily::System(id);
        let style = FontStyle::new(fam, bold && fam.has_bold(), italic);
        report.system_fonts.insert(style.family.title().to_string());
        return (style, text.to_string());
    }
    let cleaned: String = text
        .chars()
        .map(|c| {
            if missing.contains(&c) {
                report.missing_chars.insert(c);
                '?'
            } else {
                c
            }
        })
        .collect();
    (dejavu, cleaned)
}

/// Remove the original text of each block and draw its translation in the same box.
pub fn apply_translation(tx: &mut Tx<'_>, page: PageId, items: &[Item]) -> Result<Report> {
    let mut report = Report::default();
    if items.is_empty() {
        return Ok(report);
    }
    let pc = PageContent::load(tx.doc(), page.0)?;
    let (ctm, _) = pc.end_state();
    let inv = ctm.inverse().ok_or_else(|| {
        crate::error::EngineError::Unsupported("the page ends with a degenerate transform".into())
    })?;

    // 1. Take the original text out of the page content where the editor can.
    let mut doomed: Vec<ObjRef> = Vec::new();
    // Per block: how many original runs were bold / italic / in total (to keep the look).
    let mut look = vec![(0usize, 0usize, 0usize); items.len()];
    for run in pc.text_runs() {
        let c = run.quad.bounds();
        let center = Point::new((c.x0 + c.x1) / 2.0, (c.y0 + c.y1) / 2.0);
        let Some(k) = items
            .iter()
            .position(|it| it.block.rect.inflate(1.0, 1.0).contains(center))
        else {
            continue;
        };
        let f = run.base_font.to_lowercase();
        look[k].0 += usize::from(f.contains("bold") || f.contains("black") || f.contains("heavy"));
        look[k].1 += usize::from(f.contains("italic") || f.contains("oblique"));
        look[k].2 += 1;
        if run.editable.is_ok() {
            doomed.push(run.id);
        }
    }
    report.runs_removed = pc.delete_texts(tx, &doomed)?;

    // 2. Cover and write, all in one appended content stream.
    let mut builders: HashMap<FontStyle, (FontBuilder, String)> = HashMap::new();
    let mut content = String::new();
    for (k, it) in items.iter().enumerate() {
        let b = &it.block;
        let (nb, ni, n) = look[k];
        let (style, text) = choose_font(
            &it.translation,
            n > 0 && nb * 2 > n,
            n > 0 && ni * 2 > n,
            &mut report,
        );
        let next = builders.len();
        let (fb, res) = match builders.entry(style) {
            std::collections::hash_map::Entry::Occupied(o) => o.into_mut(),
            std::collections::hash_map::Entry::Vacant(v) => v.insert((
                FontBuilder::new(BundledFace::Styled(style))?,
                format!("BergTr{next}"),
            )),
        };
        let rot = b.rotation;
        let (w, h) = local_size(b.rect, rot);
        let fs0 = b.font_size.clamp(4.0, 72.0);
        let (mut fs, mut lines) = (fs0, Vec::new());
        let mut fits = false;
        for k in 0..14 {
            fs = fs0 * 0.94f64.powi(k);
            lines = wrap_lines(fb, &text, fs, w);
            if lines.len() as f64 * fb.line_height_em() * fs <= h * 1.03 {
                fits = true;
                break;
            }
        }
        if fs < fs0 - 1e-9 {
            report.shrunk += 1;
        }
        if !fits {
            report.overflowing += 1;
        }
        let lh = fb.line_height_em() * fs;
        let bg = it.background;
        let fg: (f64, f64, f64) = if luminance(bg) < 0.45 {
            (1.0, 1.0, 1.0)
        } else {
            (0.0, 0.0, 0.0)
        };
        let cover = b.rect.abs().inflate(1.5, 1.5);
        content.push_str("q\n");
        content.push_str(&format!("{} cm\n", inv.operands()));
        content.push_str(&format!(
            "{} {} {} rg\n{} {} {} {} re f\n",
            fmt_num_prec(f64::from(bg.0)),
            fmt_num_prec(f64::from(bg.1)),
            fmt_num_prec(f64::from(bg.2)),
            fmt_num_prec(cover.x0),
            fmt_num_prec(cover.y0),
            fmt_num_prec(cover.width()),
            fmt_num_prec(cover.height())
        ));
        content.push_str("q\n");
        content.push_str(&cm_op(rotated_frame(b.rect, rot)));
        content.push_str(&format!(
            "BT\n/{res} {} Tf\n{} {} {} rg\n",
            fmt_num_prec(fs),
            fmt_num_prec(fg.0),
            fmt_num_prec(fg.1),
            fmt_num_prec(fg.2),
        ));
        let mut y = h - fb.ascent_em() * fs;
        let mut first = true;
        let mut prev_x = 0.0;
        for line in &lines {
            let codes = fb.encode_str(line)?;
            content.push_str(&format!(
                "{} {} Td\n",
                fmt_num_prec(-prev_x),
                fmt_num_prec(if first { y } else { -lh })
            ));
            if !codes.is_empty() {
                content.push_str(&format!("{} Tj\n", crate::fontembed::hex_string(&codes)));
            }
            prev_x = 0.0;
            first = false;
            y -= lh;
        }
        content.push_str("ET\nQ\nQ\n");
        report.blocks += 1;
    }
    for (_, (fb, res)) in builders {
        let id = fb.finish(tx)?;
        pagecontent::add_resource(tx, page.0, b"Font", &res, Object::Reference(id))?;
    }
    pagecontent::append_content(tx, page.0, content.as_bytes())?;
    Ok(report)
}
