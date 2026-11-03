//! Page labels (`/PageLabels` in the catalog): the "number" a reader shows for a page, such as
//! `i`, `ii`, `iii` for a preface and `1`, `2`, … after it, or `A-1`.
//!
//! The labels are a number tree from page index to a label range: a numbering style, a prefix and
//! the number of the range's first page. Reading follows `/Kids` (depth- and size-limited); writing
//! always produces one flat `/Nums` array.

use crate::doc::Tx;
use crate::error::{EngineError, Result};
use crate::objutil::{self, text_obj};
use lopdf::{Dictionary, Document, Object, dictionary};
use std::collections::BTreeSet;

/// Most label ranges read or written.
pub const MAX_RANGES: usize = 10_000;

/// How the number of a page is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LabelStyle {
    /// 1, 2, 3
    #[default]
    Decimal,
    /// I, II, III
    RomanUpper,
    /// i, ii, iii
    RomanLower,
    /// A, B, … Z, AA, BB
    LettersUpper,
    /// a, b, … z, aa, bb
    LettersLower,
    /// No number: the prefix alone.
    None,
}

impl LabelStyle {
    fn pdf_name(self) -> Option<&'static str> {
        match self {
            LabelStyle::Decimal => Some("D"),
            LabelStyle::RomanUpper => Some("R"),
            LabelStyle::RomanLower => Some("r"),
            LabelStyle::LettersUpper => Some("A"),
            LabelStyle::LettersLower => Some("a"),
            LabelStyle::None => None,
        }
    }

    fn from_pdf_name(n: Option<&[u8]>) -> LabelStyle {
        match n {
            Some(b"D") => LabelStyle::Decimal,
            Some(b"R") => LabelStyle::RomanUpper,
            Some(b"r") => LabelStyle::RomanLower,
            Some(b"A") => LabelStyle::LettersUpper,
            Some(b"a") => LabelStyle::LettersLower,
            _ => LabelStyle::None,
        }
    }

    /// Display name.
    pub fn title(self) -> &'static str {
        match self {
            LabelStyle::Decimal => "1, 2, 3",
            LabelStyle::RomanUpper => "I, II, III",
            LabelStyle::RomanLower => "i, ii, iii",
            LabelStyle::LettersUpper => "A, B, C",
            LabelStyle::LettersLower => "a, b, c",
            LabelStyle::None => "No number",
        }
    }

    /// All styles, for pickers.
    pub const ALL: [LabelStyle; 6] = [
        LabelStyle::Decimal,
        LabelStyle::RomanLower,
        LabelStyle::RomanUpper,
        LabelStyle::LettersLower,
        LabelStyle::LettersUpper,
        LabelStyle::None,
    ];
}

/// One label range: from page `start` (0-based) up to the next range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelRange {
    /// First page of the range (0-based page index).
    pub start: usize,
    /// Numbering style.
    pub style: LabelStyle,
    /// Text before the number.
    pub prefix: String,
    /// Number of the range's first page (1 or more).
    pub first: u32,
}

impl LabelRange {
    /// A plain 1, 2, 3 range from `start`.
    pub fn decimal(start: usize) -> Self {
        Self {
            start,
            style: LabelStyle::Decimal,
            prefix: String::new(),
            first: 1,
        }
    }
}

fn roman(mut n: u32) -> String {
    const TABLE: [(u32, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut s = String::new();
    for (v, r) in TABLE {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

fn letters(n: u32) -> String {
    // PDF: a..z, then aa..zz, then aaa..zzz.
    let n = n.max(1) - 1;
    let ch = char::from(b'a' + (n % 26) as u8);
    std::iter::repeat_n(ch, (n / 26) as usize + 1).collect()
}

/// The number part of a label.
pub fn format_number(style: LabelStyle, n: u32) -> String {
    match style {
        LabelStyle::Decimal => n.to_string(),
        LabelStyle::RomanLower => roman(n),
        LabelStyle::RomanUpper => roman(n).to_uppercase(),
        LabelStyle::LettersLower => letters(n),
        LabelStyle::LettersUpper => letters(n).to_uppercase(),
        LabelStyle::None => String::new(),
    }
}

/// The label of page `index` (0-based) under `ranges`; without ranges, `index + 1`.
pub fn label_for(ranges: &[LabelRange], index: usize) -> String {
    let Some(r) = ranges.iter().rev().find(|r| r.start <= index) else {
        return (index + 1).to_string();
    };
    let n = r
        .first
        .saturating_add(u32::try_from(index - r.start).unwrap_or(u32::MAX));
    format!("{}{}", r.prefix, format_number(r.style, n))
}

/// Labels of all `pages` pages.
pub fn all_labels(ranges: &[LabelRange], pages: usize) -> Vec<String> {
    (0..pages).map(|i| label_for(ranges, i)).collect()
}

fn catalog(doc: &Document) -> Option<&Dictionary> {
    doc.trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .ok()
        .and_then(|r| doc.get_dictionary(r).ok())
}

fn read_tree(
    doc: &Document,
    node: &Dictionary,
    depth: usize,
    seen: &mut BTreeSet<lopdf::ObjectId>,
    out: &mut Vec<LabelRange>,
) {
    if depth > 32 || out.len() >= MAX_RANGES {
        return;
    }
    if let Some(nums) = objutil::dict_array(doc, node, b"Nums") {
        for pair in nums.chunks(2) {
            let [k, v] = pair else { break };
            let Some(start) = objutil::num(doc, k)
                .filter(|n| *n >= 0.0 && n.is_finite())
                .map(|n| n as usize)
            else {
                continue;
            };
            let Some(d) = objutil::deref(doc, v).and_then(|o| o.as_dict().ok()) else {
                continue;
            };
            let prefix = d
                .get(b"P")
                .ok()
                .and_then(|o| objutil::deref(doc, o))
                .and_then(|o| o.as_str().ok())
                .map(objutil::decode_text_string)
                .unwrap_or_default();
            let first = objutil::dict_num(doc, d, b"St")
                .filter(|n| *n >= 1.0 && n.is_finite())
                .map_or(1, |n| n.min(f64::from(u32::MAX)) as u32);
            out.push(LabelRange {
                start,
                style: LabelStyle::from_pdf_name(objutil::dict_name(doc, d, b"S")),
                prefix: prefix.chars().filter(|c| !c.is_control()).collect(),
                first,
            });
        }
    }
    if let Some(kids) = objutil::dict_array(doc, node, b"Kids") {
        for k in kids {
            let Ok(id) = k.as_reference() else { continue };
            if !seen.insert(id) {
                continue;
            }
            if let Ok(kd) = doc.get_dictionary(id) {
                read_tree(doc, kd, depth + 1, seen, out);
            }
        }
    }
}

/// The document's label ranges, sorted by first page (empty: the document has no labels).
pub fn read_page_labels(doc: &Document) -> Vec<LabelRange> {
    let Some(tree) = catalog(doc).and_then(|c| objutil::dict_dict(doc, c, b"PageLabels")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    read_tree(doc, tree, 0, &mut BTreeSet::new(), &mut out);
    out.sort_by_key(|r| r.start);
    out.dedup_by_key(|r| r.start);
    out
}

/// Replace the document's page labels. An empty list removes them (readers then show 1, 2, 3).
/// A list that does not start at page 0 gets a plain decimal range there, as the PDF standard asks.
pub fn set_page_labels(tx: &mut Tx<'_>, ranges: &[LabelRange], pages: usize) -> Result<()> {
    let cat = tx
        .doc()
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .map_err(|_| EngineError::Malformed("missing /Root".into()))?;
    if ranges.is_empty() {
        tx.dict_mut(cat)?.remove(b"PageLabels");
        return Ok(());
    }
    if ranges.len() > MAX_RANGES {
        return Err(EngineError::LimitExceeded("too many label ranges".into()));
    }
    let mut sorted: Vec<LabelRange> = ranges.to_vec();
    sorted.sort_by_key(|r| r.start);
    sorted.dedup_by_key(|r| r.start);
    if sorted.iter().any(|r| r.start >= pages.max(1)) {
        return Err(EngineError::InvalidArgument(
            "a label range starts after the last page".into(),
        ));
    }
    if sorted.first().is_some_and(|r| r.start != 0) {
        sorted.insert(0, LabelRange::decimal(0));
    }
    let mut nums = Vec::new();
    for r in &sorted {
        let mut d = Dictionary::new();
        d.set("Type", Object::Name(b"PageLabel".to_vec()));
        if let Some(s) = r.style.pdf_name() {
            d.set("S", Object::Name(s.as_bytes().to_vec()));
        }
        if !r.prefix.is_empty() {
            d.set("P", text_obj(&r.prefix));
        }
        if r.first != 1 {
            d.set("St", i64::from(r.first.max(1)));
        }
        nums.push(Object::Integer(r.start as i64));
        nums.push(Object::Dictionary(d));
    }
    let tree = tx.add(Object::Dictionary(
        dictionary! { "Nums" => Object::Array(nums) },
    ));
    tx.dict_mut(cat)?.set("PageLabels", Object::Reference(tree));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_in_every_style() {
        assert_eq!(format_number(LabelStyle::RomanLower, 4), "iv");
        assert_eq!(format_number(LabelStyle::RomanUpper, 1994), "MCMXCIV");
        assert_eq!(format_number(LabelStyle::LettersLower, 1), "a");
        assert_eq!(format_number(LabelStyle::LettersUpper, 27), "AA");
        assert_eq!(format_number(LabelStyle::LettersLower, 53), "aaa");
        assert_eq!(format_number(LabelStyle::None, 5), "");
    }

    #[test]
    fn labels_follow_the_ranges() {
        let ranges = vec![
            LabelRange {
                start: 0,
                style: LabelStyle::RomanLower,
                prefix: String::new(),
                first: 1,
            },
            LabelRange {
                start: 3,
                style: LabelStyle::Decimal,
                prefix: "A-".into(),
                first: 7,
            },
        ];
        assert_eq!(all_labels(&ranges, 5), vec!["i", "ii", "iii", "A-7", "A-8"]);
        assert_eq!(label_for(&[], 4), "5");
    }
}
