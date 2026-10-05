//! Small helpers over `lopdf::Object` used by every engine module.

use crate::geom::Rect;
use lopdf::{Dictionary, Document, Object, ObjectId};

/// Maximum depth when following `/Parent` chains or nested structures.
pub const MAX_DEPTH: usize = 64;

/// Dereference an object through (possibly chained) references.
pub fn deref<'a>(doc: &'a Document, mut obj: &'a Object) -> Option<&'a Object> {
    for _ in 0..MAX_DEPTH {
        match obj {
            Object::Reference(id) => obj = doc.objects.get(id)?,
            other => return Some(other),
        }
    }
    None
}

/// Numeric value of an object (integer or real), following references.
pub fn num(doc: &Document, obj: &Object) -> Option<f64> {
    match deref(doc, obj)? {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(f64::from(*r)),
        _ => None,
    }
}

/// Parse a 4-number rectangle array.
pub fn rect(doc: &Document, obj: &Object) -> Option<Rect> {
    let arr = deref(doc, obj)?.as_array().ok()?;
    if arr.len() < 4 {
        return None;
    }
    let v: Option<Vec<f64>> = arr.iter().take(4).map(|o| num(doc, o)).collect();
    let v = v?;
    Some(Rect::new(v[0], v[1], v[2], v[3]).abs())
}

/// Read a dictionary entry as `f64`.
pub fn dict_num(doc: &Document, d: &Dictionary, key: &[u8]) -> Option<f64> {
    d.get(key).ok().and_then(|o| num(doc, o))
}

/// Read a dictionary entry as a name (bytes).
pub fn dict_name<'a>(doc: &'a Document, d: &'a Dictionary, key: &[u8]) -> Option<&'a [u8]> {
    deref(doc, d.get(key).ok()?)?.as_name().ok()
}

/// Read a dictionary entry as a dictionary (direct or indirect). Streams yield their dict.
pub fn dict_dict<'a>(doc: &'a Document, d: &'a Dictionary, key: &[u8]) -> Option<&'a Dictionary> {
    match deref(doc, d.get(key).ok()?)? {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

/// Read a dictionary entry as an array of objects.
pub fn dict_array<'a>(doc: &'a Document, d: &'a Dictionary, key: &[u8]) -> Option<&'a Vec<Object>> {
    deref(doc, d.get(key).ok()?)?.as_array().ok()
}

/// Decode a PDF text string (PDFDocEncoding or UTF-16BE with BOM, UTF-8 with BOM) lossily.
pub fn decode_text_string(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else if bytes.len() >= 3 && bytes[..3] == [0xEF, 0xBB, 0xBF] {
        String::from_utf8_lossy(&bytes[3..]).into_owned()
    } else {
        // PDFDocEncoding is Latin-1 compatible for the printable range we care about.
        bytes.iter().map(|&b| b as char).collect()
    }
}

/// Encode text as a PDF text string: PDFDocEncoding-compatible Latin-1 when possible,
/// otherwise UTF-16BE with BOM.
pub fn encode_text_string(s: &str) -> Vec<u8> {
    if s.chars()
        .all(|c| (c as u32) < 0x80 || ((c as u32) >= 0xA0 && (c as u32) <= 0xFF))
    {
        s.chars().map(|c| c as u32 as u8).collect()
    } else {
        let mut out = vec![0xFE, 0xFF];
        for u in s.encode_utf16() {
            out.extend_from_slice(&u.to_be_bytes());
        }
        out
    }
}

/// Make an `Object::String` from text.
pub fn text_obj(s: &str) -> Object {
    Object::String(encode_text_string(s), lopdf::StringFormat::Literal)
}

/// Make a name object.
pub fn name(s: &str) -> Object {
    Object::Name(s.as_bytes().to_vec())
}

/// Make a real-number array from f64 values.
pub fn num_array(v: &[f64]) -> Object {
    Object::Array(v.iter().map(|&x| real(x)).collect())
}

/// Make a real number object (integers stay integers when exact).
pub fn real(x: f64) -> Object {
    if x.is_finite() && x.fract() == 0.0 && x.abs() < 1e9 {
        Object::Integer(x as i64)
    } else if x.is_finite() {
        Object::Real(x as f32)
    } else {
        Object::Integer(0)
    }
}

/// Convert an `ObjectId` into a reference object.
pub fn reference(id: ObjectId) -> Object {
    Object::Reference(id)
}

/// Format a float for content streams (no exponent, trimmed zeros).
pub fn fmt_num(x: f64) -> String {
    if !x.is_finite() {
        return "0".to_string();
    }
    let s = format!("{x:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" || s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

/// Format a float with up to 6 decimals (content-stream matrices need more precision than
/// [`fmt_num`]'s 4 decimals).
pub fn fmt_num_prec(x: f64) -> String {
    if !x.is_finite() {
        return "0".to_string();
    }
    let s = format!("{x:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" || s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

/// Look up an inheritable page attribute starting at `page` and walking `/Parent` links.
pub fn inherited_obj(doc: &Document, page: ObjectId, key: &[u8]) -> Option<Object> {
    let mut cur = page;
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..MAX_DEPTH {
        if !seen.insert(cur) {
            return None;
        }
        let Some(Object::Dictionary(d)) = doc.objects.get(&cur) else {
            return None;
        };
        if let Ok(v) = d.get(key) {
            return Some(v.clone());
        }
        cur = d.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

/// Number of objects (and the trailer) that reference `target`.
pub fn reference_count(doc: &Document, target: ObjectId) -> usize {
    fn count(o: &Object, target: ObjectId) -> usize {
        match o {
            Object::Reference(r) => usize::from(*r == target),
            Object::Array(a) => a.iter().map(|x| count(x, target)).sum(),
            Object::Dictionary(d) => d.iter().map(|(_, v)| count(v, target)).sum(),
            Object::Stream(s) => s.dict.iter().map(|(_, v)| count(v, target)).sum(),
            _ => 0,
        }
    }
    doc.objects
        .values()
        .map(|o| count(o, target))
        .sum::<usize>()
        + doc
            .trailer
            .iter()
            .map(|(_, v)| count(v, target))
            .sum::<usize>()
}
