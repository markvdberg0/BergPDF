//! Document information (`/Info`): read and edit title, author, subject and keywords.
//!
//! Only the classic Info dictionary is edited. If the document also carries an XMP packet
//! (`/Metadata` on the catalog) it is **not** rewritten, so the two can disagree; callers
//! should tell the user (see [`has_xmp`]).

use crate::annot::now_pdf_date;
use crate::doc::{PdfDocument, Tx};
use crate::error::{EngineError, Result};
use crate::objutil::{self, text_obj};
use lopdf::{Dictionary, Document, Object, ObjectId};

/// Maximum accepted length of one metadata string (characters).
pub const MAX_LEN: usize = 4096;

/// Document information as read from `/Info`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocInfo {
    /// `/Title`.
    pub title: String,
    /// `/Author`.
    pub author: String,
    /// `/Subject`.
    pub subject: String,
    /// `/Keywords`.
    pub keywords: String,
    /// `/Creator` (read-only).
    pub creator: String,
    /// `/Producer` (read-only).
    pub producer: String,
    /// `/CreationDate` as stored.
    pub created: String,
    /// `/ModDate` as stored.
    pub modified: String,
}

/// The editable fields.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InfoEdit {
    /// New title.
    pub title: String,
    /// New author.
    pub author: String,
    /// New subject.
    pub subject: String,
    /// New keywords.
    pub keywords: String,
}

fn info_dict(doc: &Document) -> Option<&Dictionary> {
    let o = doc.trailer.get(b"Info").ok()?;
    objutil::deref(doc, o)?.as_dict().ok()
}

fn text(doc: &Document, d: &Dictionary, k: &[u8]) -> String {
    d.get(k)
        .ok()
        .and_then(|o| objutil::deref(doc, o))
        .and_then(|o| o.as_str().ok())
        .map(objutil::decode_text_string)
        .unwrap_or_default()
}

/// Read the document information.
pub fn read_info(doc: &PdfDocument) -> DocInfo {
    let d = doc.lopdf();
    let Some(i) = info_dict(d) else {
        return DocInfo::default();
    };
    DocInfo {
        title: text(d, i, b"Title"),
        author: text(d, i, b"Author"),
        subject: text(d, i, b"Subject"),
        keywords: text(d, i, b"Keywords"),
        creator: text(d, i, b"Creator"),
        producer: text(d, i, b"Producer"),
        created: text(d, i, b"CreationDate"),
        modified: text(d, i, b"ModDate"),
    }
}

/// Whether the catalog has an XMP metadata stream.
pub fn has_xmp(doc: &PdfDocument) -> bool {
    let d = doc.lopdf();
    d.trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .ok()
        .and_then(|r| d.get_dictionary(r).ok())
        .is_some_and(|c| c.has(b"Metadata"))
}

/// Write the editable fields (empty string removes the entry) and stamp `/ModDate`.
pub fn set_info(tx: &mut Tx<'_>, edit: &InfoEdit) -> Result<()> {
    for (n, s) in [
        ("title", &edit.title),
        ("author", &edit.author),
        ("subject", &edit.subject),
        ("keywords", &edit.keywords),
    ] {
        if s.chars().count() > MAX_LEN {
            return Err(EngineError::InvalidArgument(format!(
                "the {n} is longer than {MAX_LEN} characters"
            )));
        }
    }
    let existing: Option<ObjectId> = tx
        .doc()
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|o| o.as_reference().ok());
    let id = match existing {
        Some(id) => id,
        None => {
            // No Info reference (absent or direct dictionary): carry any direct contents over.
            let seed = info_dict(tx.doc()).cloned().unwrap_or_default();
            let id = tx.add(Object::Dictionary(seed));
            tx.trailer_mut().set("Info", Object::Reference(id));
            id
        }
    };
    let d = tx.dict_mut(id)?;
    for (k, v) in [
        ("Title", &edit.title),
        ("Author", &edit.author),
        ("Subject", &edit.subject),
        ("Keywords", &edit.keywords),
    ] {
        if v.is_empty() {
            d.remove(k.as_bytes());
        } else {
            d.set(k, text_obj(v));
        }
    }
    d.set("ModDate", Object::string_literal(now_pdf_date()));
    Ok(())
}

/// `D:20251005061725Z` → `2025-10-05 06:17` (falls back to the raw text).
pub fn format_pdf_date(s: &str) -> String {
    let t = s.strip_prefix("D:").unwrap_or(s);
    let digits: String = t.chars().take_while(char::is_ascii_digit).collect();
    if digits.len() >= 8 {
        let (y, m, d) = (&digits[0..4], &digits[4..6], &digits[6..8]);
        let hm = if digits.len() >= 12 {
            format!(" {}:{}", &digits[8..10], &digits[10..12])
        } else {
            String::new()
        };
        format!("{y}-{m}-{d}{hm}")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_format() {
        assert_eq!(format_pdf_date("D:20251005061725Z"), "2025-10-05 06:17");
        assert_eq!(format_pdf_date("D:20251005"), "2025-10-05");
        assert_eq!(format_pdf_date("garbage"), "garbage");
        assert_eq!(format_pdf_date(""), "");
    }
}
