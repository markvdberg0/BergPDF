//! Text extraction with glyph quads, Unicode mapping, line grouping, selection and search.
//!
//! Glyphs come from hayro's interpreter via a custom [`Device`]; hayro supplies the
//! per-glyph Unicode (ToUnicode → glyph names → conventions), advance widths and
//! transforms. Everything is stored in PDF user space so it is independent of zoom,
//! rotation and view mode.
//!
//! Known limitations (also in docs/FEATURE_MATRIX.md): reading order follows content-stream
//! order within a page (no column analysis); glyph box heights use a fixed em-relative
//! ascent/descent because hayro does not expose font descriptors; vertical writing is not
//! specially handled.

use crate::geom::{Affine, Point, Quad, Rect};
use hayro::hayro_interpret::font::{Glyph, GlyphRun};
use hayro::hayro_interpret::hayro_syntax::Pdf;
use hayro::hayro_interpret::hayro_syntax::page::Page;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, InterpreterSettings, SoftMask, interpret_page,
};
use hayro::kurbo::BezPath;
use std::ops::Range;

const ASCENT: f64 = 0.85;
const DESCENT: f64 = 0.22;
/// Hard cap on glyphs collected per page (decompression/pathological-content guard).
pub const MAX_GLYPHS_PER_PAGE: usize = 2_000_000;

/// One glyph (or a synthetic separator) in reading order.
#[derive(Clone, Debug)]
pub struct TextGlyph {
    /// Unicode text this glyph represents (may be several chars for ligatures).
    pub text: String,
    /// Box in PDF user space. Degenerate for synthetic glyphs.
    pub quad: Quad,
    /// Approximate font size in user-space units.
    pub font_size: f64,
    /// Baseline direction angle in radians (0 = left-to-right upright).
    pub angle: f64,
    /// Index into [`TextPage::lines`].
    pub line: usize,
    /// True for inserted spaces / line breaks that are not in the content stream.
    pub synthetic: bool,
    /// True when the glyph was drawn invisibly (e.g. an OCR text layer).
    pub invisible: bool,
}

/// A run of glyphs sharing a baseline.
#[derive(Clone, Debug)]
pub struct TextLine {
    /// Glyph index range (synthetic separators belong to the line they follow).
    pub glyphs: Range<usize>,
    /// Union of glyph bounds in user space.
    pub bounds: Rect,
}

/// All text of one page.
#[derive(Clone, Debug, Default)]
pub struct TextPage {
    /// Glyphs in reading order including synthetic separators.
    pub glyphs: Vec<TextGlyph>,
    /// Line segmentation.
    pub lines: Vec<TextLine>,
    /// True if the glyph cap was hit and extraction is incomplete.
    pub truncated: bool,
}

/// A search hit.
#[derive(Clone, Debug)]
pub struct SearchHit {
    /// Glyph index range covered by the match.
    pub glyphs: Range<usize>,
    /// One quad per line segment.
    pub quads: Vec<Quad>,
}

struct Collector {
    glyphs: Vec<RawGlyph>,
    truncated: bool,
}

struct RawGlyph {
    text: String,
    origin: Point,
    quad: Quad,
    font_size: f64,
    angle: f64,
    invisible: bool,
}

impl<'a> Device<'a> for Collector {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, mode: &DrawMode) {
        let invisible = matches!(mode, DrawMode::Invisible);
        for g in run.glyphs() {
            if self.glyphs.len() >= MAX_GLYPHS_PER_PAGE {
                self.truncated = true;
                return;
            }
            let (unicode, adv) = match &**g {
                Glyph::Outline(o) => (o.as_unicode(), o.advance_width()),
                Glyph::Type3(t) => (t.as_unicode(), None),
            };
            let Some(unicode) = unicode else { continue };
            let text = match unicode {
                hayro::hayro_interpret::hayro_cmap::BfString::Char(c) => c.to_string(),
                hayro::hayro_interpret::hayro_cmap::BfString::String(s) => s,
            };
            if text.is_empty() {
                continue;
            }
            let full: Affine = props.transform * g.transform();
            let adv = f64::from(adv.unwrap_or(500.0)).max(0.0);
            let q = Quad([
                full * Point::new(0.0, -DESCENT * 1000.0),
                full * Point::new(adv, -DESCENT * 1000.0),
                full * Point::new(adv, ASCENT * 1000.0),
                full * Point::new(0.0, ASCENT * 1000.0),
            ]);
            let origin = full * Point::new(0.0, 0.0);
            let dx = full * Point::new(1.0, 0.0) - origin;
            self.glyphs.push(RawGlyph {
                text,
                origin,
                quad: q,
                font_size: full.determinant().abs().sqrt() * 1000.0,
                angle: dx.y.atan2(dx.x),
                invisible,
            });
        }
    }
}

/// Collect the text of a page. Coordinates are returned in PDF user space.
pub fn collect<'a>(
    page: &Page<'a>,
    cache: &InterpreterCache<'a>,
    pdf: &'a Pdf,
    crop: Rect,
) -> TextPage {
    let settings = InterpreterSettings {
        render_annotations: false,
        ..InterpreterSettings::default()
    };
    let mut ctx = Context::new(Affine::IDENTITY, crop, cache, pdf.xref(), settings);
    let mut dev = Collector {
        glyphs: Vec::new(),
        truncated: false,
    };
    interpret_page(page, &mut ctx, &mut dev);
    build_text_page(dev.glyphs, dev.truncated)
}

fn build_text_page(raw: Vec<RawGlyph>, truncated: bool) -> TextPage {
    let mut tp = TextPage {
        truncated,
        ..Default::default()
    };
    let mut prev: Option<usize> = None; // index into tp.glyphs of the previous real glyph
    for r in raw {
        let mut start_new_line = true;
        let mut need_space = false;
        if let Some(pi) = prev {
            let p = &tp.glyphs[pi];
            let same_dir = angle_diff(p.angle, r.angle) < 0.17; // ~10°
            if same_dir {
                let (ux, uy) = (p.angle.cos(), p.angle.sin());
                let pend = p.quad.0[1];
                let dxv = r.origin.x - pend.x;
                let dyv = r.origin.y - pend.y;
                let along = dxv * ux + dyv * uy;
                let across = -dxv * uy + dyv * ux;
                let fs = p.font_size.max(r.font_size).max(1.0);
                if across.abs() < 0.45 * fs && along > -0.6 * fs && along < 12.0 * fs {
                    start_new_line = false;
                    need_space =
                        along > 0.22 * fs && !p.text.ends_with(' ') && !r.text.starts_with(' ');
                }
            }
        }
        if start_new_line {
            if let Some(pi) = prev {
                // Line break separator belongs to the finished line.
                let (end, li, fs, ang) = {
                    let last = &tp.glyphs[pi];
                    (last.quad.0[1], last.line, last.font_size, last.angle)
                };
                push_synth(&mut tp, "\n", end, fs, ang, li);
            }
            tp.lines.push(TextLine {
                glyphs: tp.glyphs.len()..tp.glyphs.len(),
                bounds: Rect::new(f64::MAX, f64::MAX, f64::MIN, f64::MIN),
            });
        } else if need_space && let Some(pi) = prev {
            let last = &tp.glyphs[pi];
            let end = last.quad.0[1];
            let li = last.line;
            let (fs, ang) = (last.font_size, last.angle);
            push_synth(&mut tp, " ", end, fs, ang, li);
        }
        let li = tp.lines.len() - 1;
        let b = r.quad.bounds();
        tp.glyphs.push(TextGlyph {
            text: r.text,
            quad: r.quad,
            font_size: r.font_size,
            angle: r.angle,
            line: li,
            synthetic: false,
            invisible: r.invisible,
        });
        prev = Some(tp.glyphs.len() - 1);
        let line = &mut tp.lines[li];
        line.glyphs.end = tp.glyphs.len();
        line.bounds = if line.bounds.x0 > line.bounds.x1 {
            b
        } else {
            line.bounds.union(b)
        };
    }
    tp
}

fn push_synth(tp: &mut TextPage, s: &str, at: Point, font_size: f64, angle: f64, line: usize) {
    tp.glyphs.push(TextGlyph {
        text: s.to_string(),
        quad: Quad([at, at, at, at]),
        font_size,
        angle,
        line,
        synthetic: true,
        invisible: false,
    });
    tp.lines[line].glyphs.end = tp.glyphs.len();
}

fn angle_diff(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(std::f64::consts::TAU);
    d.min(std::f64::consts::TAU - d)
}

impl TextPage {
    /// Plain text of the page (lines separated by `\n`).
    pub fn plain_text(&self) -> String {
        self.text_of(0..self.glyphs.len())
    }

    /// Text of a glyph range (synthetic separators included).
    pub fn text_of(&self, range: Range<usize>) -> String {
        let end = range.end.min(self.glyphs.len());
        let start = range.start.min(end);
        self.glyphs[start..end]
            .iter()
            .map(|g| g.text.as_str())
            .collect()
    }

    /// Nearest real glyph to a point (user space). Prefers glyphs containing the point; else
    /// the closest glyph centre within `max_dist` user units.
    pub fn hit_test(&self, p: Point, max_dist: f64) -> Option<usize> {
        let mut best: Option<(usize, f64)> = None;
        for (i, g) in self.glyphs.iter().enumerate() {
            if g.synthetic {
                continue;
            }
            if g.quad.contains(p) {
                return Some(i);
            }
            let b = g.quad.bounds();
            let c = Point::new((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
            let d = ((c.x - p.x).powi(2) + (c.y - p.y).powi(2)).sqrt();
            if d <= max_dist && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((i, d));
            }
        }
        best.map(|(i, _)| i)
    }

    /// Selection quads for an inclusive-exclusive glyph range: one merged quad per line
    /// segment, honouring rotated baselines.
    pub fn selection_quads(&self, range: Range<usize>) -> Vec<Quad> {
        let end = range.end.min(self.glyphs.len());
        let mut out = Vec::new();
        let mut i = range.start.min(end);
        while i < end {
            if self.glyphs[i].synthetic {
                i += 1;
                continue;
            }
            let line = self.glyphs[i].line;
            let mut j = i;
            while j + 1 < end && (self.glyphs[j + 1].line == line) {
                j += 1;
            }
            // Merge first..last real glyph on this line: use first.bl, last.br etc.
            let first = (i..=j).find(|&k| !self.glyphs[k].synthetic);
            let last = (i..=j).rev().find(|&k| !self.glyphs[k].synthetic);
            if let (Some(f), Some(l)) = (first, last) {
                let a = &self.glyphs[f].quad.0;
                let b = &self.glyphs[l].quad.0;
                out.push(Quad([a[0], b[1], b[2], a[3]]));
            }
            i = j + 1;
        }
        out
    }

    /// Search for `needle`. Whitespace runs in both haystack and needle match any
    /// whitespace (including line breaks); ligature code points are expanded.
    pub fn search(&self, needle: &str, case_sensitive: bool) -> Vec<SearchHit> {
        let n = fold(needle, case_sensitive);
        let n_chars: Vec<char> = collapse_ws(&n);
        if n_chars.is_empty() {
            return Vec::new();
        }
        // Flattened haystack with char -> glyph mapping.
        let mut hay: Vec<char> = Vec::new();
        let mut map: Vec<usize> = Vec::new();
        for (gi, g) in self.glyphs.iter().enumerate() {
            for c in fold(&g.text, case_sensitive).chars() {
                let c = if c.is_whitespace() { ' ' } else { c };
                if c == ' ' && hay.last() == Some(&' ') {
                    continue;
                }
                hay.push(c);
                map.push(gi);
            }
        }
        let mut hits = Vec::new();
        let mut i = 0;
        while i + n_chars.len() <= hay.len() {
            if hay[i..i + n_chars.len()] == n_chars[..] {
                let g0 = map[i];
                let g1 = map[i + n_chars.len() - 1] + 1;
                hits.push(SearchHit {
                    glyphs: g0..g1,
                    quads: self.selection_quads(g0..g1),
                });
                i += n_chars.len();
            } else {
                i += 1;
            }
        }
        hits
    }

    /// Word range containing the glyph at `idx` (for double-click selection).
    pub fn word_range(&self, idx: usize) -> Range<usize> {
        let is_sep = |g: &TextGlyph| g.synthetic || g.text.chars().all(char::is_whitespace);
        if idx >= self.glyphs.len() || is_sep(&self.glyphs[idx]) {
            return idx..idx;
        }
        let mut s = idx;
        while s > 0 && !is_sep(&self.glyphs[s - 1]) {
            s -= 1;
        }
        let mut e = idx + 1;
        while e < self.glyphs.len() && !is_sep(&self.glyphs[e]) {
            e += 1;
        }
        s..e
    }
}

fn collapse_ws(s: &str) -> Vec<char> {
    let mut out: Vec<char> = Vec::new();
    for c in s.chars() {
        let c = if c.is_whitespace() { ' ' } else { c };
        if c == ' ' && out.last() == Some(&' ') {
            continue;
        }
        out.push(c);
    }
    while out.first() == Some(&' ') {
        out.remove(0);
    }
    while out.last() == Some(&' ') {
        out.pop();
    }
    out
}

fn fold(s: &str, case_sensitive: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let exp = match c {
            '\u{FB00}' => Some("ff"),
            '\u{FB01}' => Some("fi"),
            '\u{FB02}' => Some("fl"),
            '\u{FB03}' => Some("ffi"),
            '\u{FB04}' => Some("ffl"),
            '\u{FB05}' | '\u{FB06}' => Some("st"),
            '\u{00A0}' => Some(" "),
            _ => None,
        };
        match exp {
            Some(e) => out.push_str(e),
            None if case_sensitive => out.push(c),
            None => out.extend(c.to_lowercase()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(text: &str, x: f64, y: f64, fs: f64) -> RawGlyph {
        let w = fs * 0.5;
        RawGlyph {
            text: text.into(),
            origin: Point::new(x, y),
            quad: Quad::from_rect(Rect::new(x, y - 0.2 * fs, x + w, y + 0.8 * fs)),
            font_size: fs,
            angle: 0.0,
            invisible: false,
        }
    }

    fn page(words: &[(&str, f64, f64)]) -> TextPage {
        let mut raw = Vec::new();
        for (w, x0, y) in words {
            let mut x = *x0;
            for c in w.chars() {
                raw.push(glyph(&c.to_string(), x, *y, 10.0));
                x += 5.0;
            }
        }
        build_text_page(raw, false)
    }

    #[test]
    fn lines_spaces_and_breaks() {
        let tp = page(&[
            ("Hello", 10.0, 100.0),
            ("world", 45.0, 100.0),
            ("Next", 10.0, 80.0),
        ]);
        assert_eq!(tp.plain_text(), "Hello world\nNext");
        assert_eq!(tp.lines.len(), 2);
    }

    #[test]
    fn search_spans_lines_and_is_case_insensitive() {
        let tp = page(&[
            ("Hello", 10.0, 100.0),
            ("world", 45.0, 100.0),
            ("Next", 10.0, 80.0),
        ]);
        assert_eq!(tp.search("hello WORLD", false).len(), 1);
        assert_eq!(tp.search("world next", false).len(), 1);
        assert!(tp.search("world next", false)[0].quads.len() == 2);
        assert!(tp.search("HELLO", true).is_empty());
        assert!(tp.search("", false).is_empty());
    }

    #[test]
    fn ligature_expansion_matches() {
        let mut raw = vec![glyph("\u{FB01}", 10.0, 10.0, 10.0)];
        raw.push(glyph("t", 15.0, 10.0, 10.0));
        let tp = build_text_page(raw, false);
        assert_eq!(tp.search("fit", false).len(), 1);
    }

    #[test]
    fn word_range_and_hit_test() {
        let tp = page(&[("Hello", 10.0, 100.0), ("world", 45.0, 100.0)]);
        let idx = tp.hit_test(Point::new(12.0, 102.0), 1.0).unwrap();
        assert_eq!(tp.text_of(tp.word_range(idx)), "Hello");
        assert!(tp.hit_test(Point::new(500.0, 500.0), 3.0).is_none());
    }

    #[test]
    fn rotated_text_is_not_merged_with_horizontal_line() {
        let mut raw = vec![glyph("A", 10.0, 10.0, 10.0)];
        let mut b = glyph("B", 15.0, 10.0, 10.0);
        b.angle = std::f64::consts::FRAC_PI_2;
        raw.push(b);
        let tp = build_text_page(raw, false);
        assert_eq!(tp.lines.len(), 2);
    }
}
