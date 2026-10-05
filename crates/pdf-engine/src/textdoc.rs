//! Build a plain text PDF (used to save a translation): A4 pages, the bundled DejaVu Sans
//! embedded as a subset, wrapped paragraphs, a heading per source page. The text is real,
//! selectable and searchable.

use crate::doc::{PdfDocument, Tx};
use crate::error::Result;
use crate::fontembed::{BundledFace, FontBuilder};
use crate::meta::{InfoEdit, set_info};
use crate::pagecontent::{add_resource, append_content};
use crate::pageops::insert_blank_page;
use lopdf::Object;
use std::collections::BTreeSet;

const PAGE_W: f64 = 595.0;
const PAGE_H: f64 = 842.0;
const MARGIN: f64 = 56.0;
const BODY: f64 = 11.0;
const HEAD: f64 = 12.0;
const TITLE: f64 = 16.0;

/// What was produced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextDocReport {
    /// Pages written.
    pub pages: usize,
    /// Characters the built-in font cannot draw; each was replaced by `?`.
    pub replaced: Vec<char>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Title,
    Heading,
    Body,
    Note,
}

struct Line {
    text: String,
    style: Style,
    /// Blank space (pt) before the line.
    before: f64,
}

fn size(s: Style) -> f64 {
    match s {
        Style::Title => TITLE,
        Style::Heading => HEAD,
        Style::Body => BODY,
        Style::Note => 9.0,
    }
}

/// Join source lines into paragraphs: a line continues the paragraph unless it ends a sentence,
/// the next one starts a list item, or there is a blank line.
fn paragraphs(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let lines: Vec<&str> = text.lines().collect();
    for (i, raw) in lines.iter().enumerate() {
        let l = raw.trim();
        if l.is_empty() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(l);
        let ends = l.ends_with(['.', '!', '?', ':', ';', '。']);
        let next_starts_item = lines.get(i + 1).map(|n| n.trim()).is_some_and(|n| {
            n.starts_with(['-', '•', '*', '–'])
                || n.split_once(['.', ')']).is_some_and(|(a, _)| {
                    !a.is_empty() && a.len() <= 3 && a.chars().all(|c| c.is_ascii_digit())
                })
        });
        if ends || next_starts_item {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn wrap(fb: &FontBuilder, text: &str, font_size: f64, width: f64) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let candidate = if cur.is_empty() {
            word.to_string()
        } else {
            format!("{cur} {word}")
        };
        if fb.text_width(&candidate, font_size) <= width {
            cur = candidate;
            continue;
        }
        if !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
        }
        // A single word wider than the line is broken by characters.
        let mut piece = String::new();
        for ch in word.chars() {
            let mut t = piece.clone();
            t.push(ch);
            if fb.text_width(&t, font_size) > width && !piece.is_empty() {
                lines.push(std::mem::take(&mut piece));
            }
            piece.push(ch);
        }
        cur = piece;
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

fn hex(codes: &[u8]) -> String {
    let mut s = String::from("<");
    for b in codes {
        s.push_str(&format!("{b:02X}"));
    }
    s.push('>');
    s
}

/// Create a PDF with a title page header and one heading per `(heading, body)` section.
/// `note` is printed under the title (for example how the text was produced).
pub fn build_text_pdf(
    title: &str,
    note: &str,
    sections: &[(String, String)],
) -> Result<(Vec<u8>, TextDocReport)> {
    let mut regular = FontBuilder::new(BundledFace::Sans)?;
    let mut bold = FontBuilder::new(BundledFace::SansBold)?;
    let usable = PAGE_W - 2.0 * MARGIN;

    // Layout into lines, then lines into pages.
    let mut replaced: BTreeSet<char> = BTreeSet::new();
    let mut clean = |s: &str, fb: &FontBuilder| -> String {
        s.chars()
            .map(|c| {
                if c.is_control() && c != ' ' {
                    ' '
                } else if c == ' ' || fb.glyph_for(c).is_some() {
                    c
                } else {
                    replaced.insert(c);
                    '?'
                }
            })
            .collect()
    };
    let mut lines: Vec<Line> = Vec::new();
    for l in wrap(&bold, &clean(title, &bold), TITLE, usable) {
        lines.push(Line {
            text: l,
            style: Style::Title,
            before: 0.0,
        });
    }
    for l in wrap(&regular, &clean(note, &regular), 9.0, usable) {
        lines.push(Line {
            text: l,
            style: Style::Note,
            before: 4.0,
        });
    }
    for (heading, body) in sections {
        let h = clean(heading, &bold);
        lines.push(Line {
            text: h,
            style: Style::Heading,
            before: 20.0,
        });
        for p in paragraphs(body) {
            let p = clean(&p, &regular);
            for (i, l) in wrap(&regular, &p, BODY, usable).into_iter().enumerate() {
                lines.push(Line {
                    text: l,
                    style: Style::Body,
                    before: if i == 0 { 7.0 } else { 0.0 },
                });
            }
        }
    }
    // Paginate and write the content streams (text must be encoded before the fonts are
    // finished, because only the used glyphs are embedded).
    let mut pages: Vec<String> = Vec::new();
    let mut cur = String::from("BT\n");
    let mut y = PAGE_H - MARGIN;
    let mut first_on_page = true;
    for l in &lines {
        let fs = size(l.style);
        let lead = fs * 1.3;
        let drop = if first_on_page { fs } else { l.before + lead };
        if y - drop < MARGIN && !first_on_page {
            cur.push_str("ET\n");
            pages.push(std::mem::replace(&mut cur, String::from("BT\n")));
            y = PAGE_H - MARGIN;
            first_on_page = true;
        }
        let drop = if first_on_page { fs } else { l.before + lead };
        y -= drop;
        first_on_page = false;
        let (name, fb) = if matches!(l.style, Style::Title | Style::Heading) {
            ("F2", &mut bold)
        } else {
            ("F1", &mut regular)
        };
        let codes = fb.encode_str(&l.text)?;
        let gray = if l.style == Style::Note {
            "0.35 g"
        } else {
            "0 g"
        };
        cur.push_str(&format!(
            "{gray}\n/{name} {fs} Tf\n1 0 0 1 {MARGIN} {y:.2} Tm\n{} Tj\n",
            hex(&codes)
        ));
    }
    cur.push_str("ET\n");
    pages.push(cur);

    let mut doc = PdfDocument::new_empty()?;
    let n_pages = pages.len();
    doc.transact(|tx: &mut Tx<'_>| {
        let f1 = regular.finish(tx)?;
        let f2 = bold.finish(tx)?;
        for (i, content) in pages.iter().enumerate() {
            let page = insert_blank_page(tx, i, PAGE_W, PAGE_H)?;
            add_resource(tx, page.0, b"Font", "F1", Object::Reference(f1))?;
            add_resource(tx, page.0, b"Font", "F2", Object::Reference(f2))?;
            append_content(tx, page.0, content.as_bytes())?;
        }
        set_info(
            tx,
            &InfoEdit {
                title: title.chars().take(300).collect(),
                ..InfoEdit::default()
            },
        )
    })?;
    let bytes = doc.rewrite_bytes()?;
    Ok((
        bytes,
        TextDocReport {
            pages: n_pages,
            replaced: replaced.into_iter().collect(),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentences_and_list_items_start_paragraphs() {
        let t = "Dit is een zin die over\nmeerdere regels loopt.\nNieuwe zin hier\nen nog iets.\n\n- punt een\n- punt twee\n1. eerste\n2. tweede";
        let p = paragraphs(t);
        assert_eq!(p[0], "Dit is een zin die over meerdere regels loopt.");
        assert_eq!(p[1], "Nieuwe zin hier en nog iets.");
        assert_eq!(
            &p[2..],
            ["- punt een", "- punt twee", "1. eerste", "2. tweede"]
        );
    }
}
