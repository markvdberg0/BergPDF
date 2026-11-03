//! Compare two versions of a document by their text.
//!
//! Both documents' words are read in reading order (with where each word is on its page) and
//! compared with a word diff (Myers, with a time limit for very different documents). The result is
//! a list of insertions, deletions and replacements. [`annotate`] marks them on (a copy of) the newer
//! document with ordinary annotations, so the comments panel lists them and they can be saved,
//! printed and shared:
//!
//! * inserted text: a green highlight;
//! * replaced text: an orange highlight whose comment says what was there before;
//! * deleted text: a red note where it was, whose comment holds the deleted words.
//!
//! Text inside pictures (scans) is not seen: run OCR on both versions first.

use crate::annot::{self, AnnotationKind, AnnotationSpec, Rgb};
use crate::doc::{OpenOptions, PageId, PdfDocument, Tx};
use crate::error::{EngineError, Result};
use crate::geom::{Point, Quad, Rect};
use crate::render::with_session;
use similar::{Algorithm, DiffOp};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Longest time spent on the diff itself; after it, the remaining differences are reported coarser.
const DIFF_DEADLINE: Duration = Duration::from_secs(20);
/// Most changes marked in the result (a completely different document is one big change anyway).
pub const MAX_MARKED: usize = 5_000;

/// A word and where it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    /// The word (no spaces).
    pub text: String,
    /// Page index.
    pub page: usize,
    /// Its box in the page's user space.
    pub rect: Rect,
}

/// What kind of change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Inserted,
    Deleted,
    Replaced,
}

/// One change between the versions.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub kind: ChangeKind,
    /// The words in the older version (empty for an insertion).
    pub old_text: String,
    /// The words in the newer version (empty for a deletion).
    pub new_text: String,
    /// Page of the older version where the old words start.
    pub old_page: Option<usize>,
    /// The new words, with their pages and boxes (empty for a deletion).
    pub new_words: Vec<Word>,
    /// For a deletion: the page and point in the newer version where the words were.
    pub anchor: Option<(usize, Point)>,
}

/// The result of a comparison.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Comparison {
    pub changes: Vec<Change>,
    /// Words in the older and the newer version.
    pub old_words: usize,
    pub new_words: usize,
    /// Pages of each version without any text (likely scans).
    pub old_pages_without_text: usize,
    pub new_pages_without_text: usize,
}

impl Comparison {
    /// Counts of inserted, deleted and replaced changes.
    pub fn counts(&self) -> (usize, usize, usize) {
        let c = |k| self.changes.iter().filter(|x| x.kind == k).count();
        (
            c(ChangeKind::Inserted),
            c(ChangeKind::Deleted),
            c(ChangeKind::Replaced),
        )
    }
}

/// The words of a document, page by page. Returns the words and the number of pages without text.
pub fn words_of(bytes: Arc<Vec<u8>>, cancel: &AtomicBool) -> Result<(Vec<Word>, usize)> {
    let doc = PdfDocument::open(bytes.to_vec(), &OpenOptions::default())?;
    let pages = doc.page_ids()?;
    let mut geometries = Vec::new();
    for p in &pages {
        geometries.push(doc.page_geometry(*p)?);
    }
    with_session(bytes, |s| {
        let mut words = Vec::new();
        let mut empty = 0;
        for (i, g) in geometries.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err(EngineError::Cancelled);
            }
            let tp = s.extract_text(i, g)?;
            let before = words.len();
            let mut cur = String::new();
            let mut rect: Option<Rect> = None;
            let flush = |cur: &mut String, rect: &mut Option<Rect>, words: &mut Vec<Word>| {
                if let Some(r) = rect.take()
                    && !cur.is_empty()
                {
                    words.push(Word {
                        text: std::mem::take(cur),
                        page: i,
                        rect: r,
                    });
                }
                cur.clear();
            };
            let mut line = usize::MAX;
            for gl in &tp.glyphs {
                let blank = gl.synthetic || gl.text.chars().all(char::is_whitespace);
                if blank || gl.line != line {
                    flush(&mut cur, &mut rect, &mut words);
                    line = gl.line;
                }
                if blank {
                    continue;
                }
                // Punctuation is a word of its own, so "roof." and "roof and the gutters." differ by
                // an insertion rather than a replacement.
                let punct = gl.text.chars().all(|c| ".,;:!?()[]{}\"“”„«»".contains(c));
                if punct {
                    flush(&mut cur, &mut rect, &mut words);
                }
                cur.push_str(&gl.text);
                let b = gl.quad.bounds();
                rect = Some(rect.map_or(b, |r| r.union(b)));
                if punct {
                    flush(&mut cur, &mut rect, &mut words);
                }
            }
            flush(&mut cur, &mut rect, &mut words);
            if words.len() == before {
                empty += 1;
            }
        }
        Ok((words, empty))
    })
}

/// Compare two word lists.
pub fn compare_words(old: &[Word], new: &[Word]) -> Vec<Change> {
    let a: Vec<&str> = old.iter().map(|w| w.text.as_str()).collect();
    let b: Vec<&str> = new.iter().map(|w| w.text.as_str()).collect();
    let ops = similar::capture_diff_slices_deadline(
        Algorithm::Myers,
        &a,
        &b,
        Some(Instant::now() + DIFF_DEADLINE),
    );
    let join = |ws: &[Word]| {
        let mut s = String::new();
        for w in ws {
            let tight = w.text.chars().all(|c| ".,;:!?)]}".contains(c));
            if !s.is_empty() && !tight && !s.ends_with(['(', '[', '{']) {
                s.push(' ');
            }
            s.push_str(&w.text);
        }
        s
    };
    // Where deleted words were: just before the next new word, or after the last one.
    let anchor = |new_index: usize| -> Option<(usize, Point)> {
        if let Some(w) = new.get(new_index) {
            Some((w.page, Point::new(w.rect.x0, w.rect.y1)))
        } else {
            new.last()
                .map(|w| (w.page, Point::new(w.rect.x1, w.rect.y1)))
        }
    };
    let mut out = Vec::new();
    for op in ops {
        match op {
            DiffOp::Equal { .. } => {}
            DiffOp::Delete {
                old_index,
                old_len,
                new_index,
            } => {
                let ws = &old[old_index..old_index + old_len];
                out.push(Change {
                    kind: ChangeKind::Deleted,
                    old_text: join(ws),
                    new_text: String::new(),
                    old_page: ws.first().map(|w| w.page),
                    new_words: Vec::new(),
                    anchor: anchor(new_index),
                });
            }
            DiffOp::Insert {
                new_index, new_len, ..
            } => {
                let ws = &new[new_index..new_index + new_len];
                out.push(Change {
                    kind: ChangeKind::Inserted,
                    old_text: String::new(),
                    new_text: join(ws),
                    old_page: None,
                    new_words: ws.to_vec(),
                    anchor: None,
                });
            }
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                let o = &old[old_index..old_index + old_len];
                let n = &new[new_index..new_index + new_len];
                out.push(Change {
                    kind: ChangeKind::Replaced,
                    old_text: join(o),
                    new_text: join(n),
                    old_page: o.first().map(|w| w.page),
                    new_words: n.to_vec(),
                    anchor: None,
                });
            }
        }
    }
    out
}

/// Compare two documents (their bytes). Runs off the UI thread; `cancel` stops it.
pub fn compare(old: Arc<Vec<u8>>, new: Arc<Vec<u8>>, cancel: &AtomicBool) -> Result<Comparison> {
    let (ow, oe) = words_of(old, cancel)?;
    let (nw, ne) = words_of(new, cancel)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(EngineError::Cancelled);
    }
    Ok(Comparison {
        changes: compare_words(&ow, &nw),
        old_words: ow.len(),
        new_words: nw.len(),
        old_pages_without_text: oe,
        new_pages_without_text: ne,
    })
}

fn clip(s: &str, n: usize) -> String {
    let mut t: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        t.push('…');
    }
    t
}

/// Mark the changes on the newer document (`pages` are its pages in order). `texts` are the words
/// used in the comments: (inserted, deleted, replaced, "was:"), so the caller can translate them.
/// Returns the number of annotations added.
pub fn annotate(
    tx: &mut Tx<'_>,
    pages: &[PageId],
    cmp: &Comparison,
    texts: [&str; 4],
) -> Result<usize> {
    let [ins, del, rep, was] = texts;
    let mut n = 0;
    for c in cmp.changes.iter().take(MAX_MARKED) {
        match c.kind {
            ChangeKind::Inserted | ChangeKind::Replaced => {
                // One highlight per page the new words are on.
                let mut by_page: Vec<(usize, Vec<Quad>)> = Vec::new();
                for w in &c.new_words {
                    let q = Quad::from_rect(w.rect);
                    match by_page.last_mut() {
                        Some((p, qs)) if *p == w.page => qs.push(q),
                        _ => by_page.push((w.page, vec![q])),
                    }
                }
                for (p, quads) in by_page {
                    let Some(page) = pages.get(p) else { continue };
                    let mut spec = AnnotationSpec::new(AnnotationKind::Highlight { quads });
                    if c.kind == ChangeKind::Inserted {
                        spec.color = Rgb(0.45, 0.85, 0.45);
                        spec.subject = ins.to_string();
                        spec.contents = format!("{ins}: {}", clip(&c.new_text, 2000));
                    } else {
                        spec.color = Rgb(1.0, 0.65, 0.2);
                        spec.subject = rep.to_string();
                        spec.contents = format!("{was} {}", clip(&c.old_text, 2000));
                    }
                    spec.author = "BergPDF Compare".into();
                    spec.opacity = 0.6;
                    annot::add_annotation(tx, *page, &spec)?;
                    n += 1;
                }
            }
            ChangeKind::Deleted => {
                let Some((p, at)) = c.anchor else { continue };
                let Some(page) = pages.get(p) else { continue };
                let mut spec = AnnotationSpec::new(AnnotationKind::Note { pos: at });
                spec.color = Rgb(0.9, 0.2, 0.2);
                spec.subject = del.to_string();
                spec.contents = format!("{del}: {}", clip(&c.old_text, 2000));
                spec.author = "BergPDF Compare".into();
                annot::add_annotation(tx, *page, &spec)?;
                n += 1;
            }
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<Word> {
        s.split_whitespace()
            .enumerate()
            .map(|(i, t)| Word {
                text: t.into(),
                page: 0,
                rect: Rect::new(i as f64 * 10.0, 0.0, i as f64 * 10.0 + 8.0, 10.0),
            })
            .collect()
    }

    #[test]
    fn insertions_deletions_and_replacements_are_found() {
        let old = words("the quick brown fox jumps over the lazy dog");
        let new = words("the quick red fox jumps high over the dog");
        let ch = compare_words(&old, &new);
        let kinds: Vec<(ChangeKind, &str, &str)> = ch
            .iter()
            .map(|c| (c.kind, c.old_text.as_str(), c.new_text.as_str()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (ChangeKind::Replaced, "brown", "red"),
                (ChangeKind::Inserted, "", "high"),
                (ChangeKind::Deleted, "lazy", ""),
            ]
        );
        // The deletion is anchored before "dog" in the new version.
        assert_eq!(ch[2].anchor, Some((0, Point::new(80.0, 10.0))));
        assert!(compare_words(&old, &old).is_empty());
    }
}
