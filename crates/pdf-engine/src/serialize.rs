//! Object serialisation and the incremental-update writer.
//!
//! Incremental updates append changed objects plus a new cross-reference section to the
//! untouched original bytes. This preserves the original file byte-for-byte (including
//! signed revisions), is cheap for large files, and is also what we hand to the renderer
//! as a revision snapshot.

use lopdf::{Dictionary, Object, StringFormat};
use std::io::Write;

/// Write a PDF object in file syntax.
pub fn write_object(out: &mut Vec<u8>, obj: &Object) {
    match obj {
        Object::Null => out.extend_from_slice(b"null"),
        Object::Boolean(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Object::Integer(i) => {
            let _ = write!(out, "{i}");
        }
        Object::Real(r) => write_real(out, *r),
        Object::Name(n) => write_name(out, n),
        Object::String(s, fmt) => write_string(out, s, *fmt),
        Object::Array(a) => {
            out.push(b'[');
            for (i, o) in a.iter().enumerate() {
                if i > 0 {
                    out.push(b' ');
                }
                write_object(out, o);
            }
            out.push(b']');
        }
        Object::Dictionary(d) => write_dict(out, d),
        Object::Stream(s) => {
            let mut d = s.dict.clone();
            d.set("Length", Object::Integer(s.content.len() as i64));
            write_dict(out, &d);
            out.extend_from_slice(b"\nstream\n");
            out.extend_from_slice(&s.content);
            out.extend_from_slice(b"\nendstream");
        }
        Object::Reference((n, g)) => {
            let _ = write!(out, "{n} {g} R");
        }
    }
}

fn write_real(out: &mut Vec<u8>, r: f32) {
    if !r.is_finite() {
        out.push(b'0');
        return;
    }
    let s = format!("{r:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" || s == "-0" {
        out.push(b'0');
    } else {
        out.extend_from_slice(s.as_bytes());
    }
}

fn write_name(out: &mut Vec<u8>, n: &[u8]) {
    out.push(b'/');
    for &b in n {
        let regular = b > 0x20
            && b < 0x7f
            && !matches!(
                b,
                b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%' | b'#'
            );
        if regular {
            out.push(b);
        } else {
            let _ = write!(out, "#{b:02X}");
        }
    }
}

fn write_string(out: &mut Vec<u8>, s: &[u8], fmt: StringFormat) {
    match fmt {
        StringFormat::Hexadecimal => {
            out.push(b'<');
            for b in s {
                let _ = write!(out, "{b:02X}");
            }
            out.push(b'>');
        }
        StringFormat::Literal => {
            out.push(b'(');
            for &b in s {
                match b {
                    b'(' | b')' | b'\\' => {
                        out.push(b'\\');
                        out.push(b);
                    }
                    b'\r' => out.extend_from_slice(b"\\r"),
                    b'\n' => out.extend_from_slice(b"\\n"),
                    _ => out.push(b),
                }
            }
            out.push(b')');
        }
    }
}

/// Write a dictionary in file syntax.
pub fn write_dict(out: &mut Vec<u8>, d: &Dictionary) {
    out.extend_from_slice(b"<<");
    for (k, v) in d.iter() {
        write_name(out, k);
        out.push(b' ');
        write_object(out, v);
        out.push(b' ');
    }
    out.extend_from_slice(b">>");
}

/// How the base file's last cross-reference section is encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XrefKind {
    /// Classic `xref` table.
    Table,
    /// Cross-reference stream (PDF 1.5+).
    Stream,
}

/// Facts about the original bytes needed to append an incremental update.
#[derive(Clone, Copy, Debug)]
pub struct BaseInfo {
    /// Byte offset of the newest cross-reference section (`startxref` value).
    pub startxref: usize,
    /// Encoding of that section.
    pub kind: XrefKind,
}

/// Locate the final `startxref` and verify that it points to a real xref section.
/// Returns `None` when the file would need repair (then only a full rewrite is safe).
pub fn find_base_info(data: &[u8]) -> Option<BaseInfo> {
    let tail_start = data.len().saturating_sub(4096);
    let tail = &data[tail_start..];
    let pos = rfind(tail, b"startxref")? + tail_start;
    let mut i = pos + b"startxref".len();
    while i < data.len() && data[i].is_ascii_whitespace() {
        i += 1;
    }
    let mut j = i;
    while j < data.len() && data[j].is_ascii_digit() {
        j += 1;
    }
    let off: usize = std::str::from_utf8(&data[i..j]).ok()?.parse().ok()?;
    if off >= data.len() {
        return None;
    }
    let mut k = off;
    while k < data.len() && data[k].is_ascii_whitespace() {
        k += 1;
    }
    if data[k..].starts_with(b"xref") {
        return Some(BaseInfo {
            startxref: off,
            kind: XrefKind::Table,
        });
    }
    // "N G obj" => xref stream
    let rest = &data[k..data.len().min(k + 40)];
    let text = String::from_utf8_lossy(rest);
    let mut it = text.split_whitespace();
    let a = it.next()?;
    let b = it.next()?;
    let c = it.next()?;
    if a.bytes().all(|x| x.is_ascii_digit())
        && b.bytes().all(|x| x.is_ascii_digit())
        && c.starts_with("obj")
    {
        return Some(BaseInfo {
            startxref: off,
            kind: XrefKind::Stream,
        });
    }
    None
}

fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if hay.len() < needle.len() {
        return None;
    }
    (0..=hay.len() - needle.len())
        .rev()
        .find(|&i| &hay[i..i + needle.len()] == needle)
}

/// One object to append.
pub struct AppendObject<'a> {
    /// Object number.
    pub id: (u32, u16),
    /// The object.
    pub obj: &'a Object,
}

/// Trailer values copied into the appended section.
pub struct TrailerInfo<'a> {
    /// `/Root` reference.
    pub root: (u32, u16),
    /// Optional `/Info` reference.
    pub info: Option<(u32, u16)>,
    /// Optional `/ID` array.
    pub id: Option<&'a Object>,
    /// Highest object number in use (`/Size` is derived from it).
    pub max_id: u32,
}

/// Append an incremental update to `base` and return the new full file bytes.
pub fn append_incremental(
    base: &[u8],
    base_info: BaseInfo,
    objects: &[AppendObject<'_>],
    trailer: &TrailerInfo<'_>,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(base.len() + 4096);
    out.extend_from_slice(base);
    if !out.ends_with(b"\n") && !out.ends_with(b"\r") {
        out.push(b'\n');
    }
    let mut offsets: Vec<(u32, u16, usize)> = Vec::with_capacity(objects.len());
    for o in objects {
        offsets.push((o.id.0, o.id.1, out.len()));
        let _ = writeln!(out, "{} {} obj", o.id.0, o.id.1);
        write_object(&mut out, o.obj);
        out.extend_from_slice(b"\nendobj\n");
    }
    offsets.sort_unstable();
    let xref_off = out.len();
    let mut size = trailer.max_id + 1;
    match base_info.kind {
        XrefKind::Table => {
            out.extend_from_slice(b"xref\n");
            let mut i = 0;
            while i < offsets.len() {
                let mut j = i;
                while j + 1 < offsets.len() && offsets[j + 1].0 == offsets[j].0 + 1 {
                    j += 1;
                }
                let _ = writeln!(out, "{} {}", offsets[i].0, j - i + 1);
                for (n, g, off) in &offsets[i..=j] {
                    let _ = n;
                    let _ = writeln!(out, "{off:010} {g:05} n ");
                }
                i = j + 1;
            }
            let mut t = Dictionary::new();
            t.set("Size", Object::Integer(i64::from(size)));
            fill_trailer(&mut t, trailer, base_info.startxref);
            out.extend_from_slice(b"trailer\n");
            write_dict(&mut out, &t);
            out.push(b'\n');
        }
        XrefKind::Stream => {
            let xref_id = size;
            size += 1;
            let mut entries = offsets.clone();
            entries.push((xref_id, 0, xref_off));
            entries.sort_unstable();
            let mut index = Vec::new();
            let mut data = Vec::new();
            let mut i = 0;
            while i < entries.len() {
                let mut j = i;
                while j + 1 < entries.len() && entries[j + 1].0 == entries[j].0 + 1 {
                    j += 1;
                }
                index.push(Object::Integer(i64::from(entries[i].0)));
                index.push(Object::Integer((j - i + 1) as i64));
                for (_, g, off) in &entries[i..=j] {
                    data.push(1u8);
                    data.extend_from_slice(&(*off as u32).to_be_bytes());
                    data.extend_from_slice(&g.to_be_bytes());
                }
                i = j + 1;
            }
            let mut d = Dictionary::new();
            d.set("Type", Object::Name(b"XRef".to_vec()));
            d.set("Size", Object::Integer(i64::from(size)));
            d.set(
                "W",
                Object::Array(vec![
                    Object::Integer(1),
                    Object::Integer(4),
                    Object::Integer(2),
                ]),
            );
            d.set("Index", Object::Array(index));
            fill_trailer(&mut d, trailer, base_info.startxref);
            let stream = lopdf::Stream::new(d, data).with_compression(false);
            let _ = writeln!(out, "{xref_id} 0 obj");
            write_object(&mut out, &Object::Stream(stream));
            out.extend_from_slice(b"\nendobj\n");
        }
    }
    let _ = write!(out, "startxref\n{xref_off}\n%%EOF\n");
    out
}

fn fill_trailer(t: &mut Dictionary, info: &TrailerInfo<'_>, prev: usize) {
    t.set("Root", Object::Reference(info.root));
    if let Some(i) = info.info {
        t.set("Info", Object::Reference(i));
    }
    if let Some(id) = info.id {
        t.set("ID", id.clone());
    }
    t.set("Prev", Object::Integer(prev as i64));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_strings_are_escaped() {
        let mut out = Vec::new();
        write_object(&mut out, &Object::Name(b"A B/#".to_vec()));
        assert_eq!(out, b"/A#20B#2F#23");
        out.clear();
        write_object(
            &mut out,
            &Object::String(b"a(b)\\\n".to_vec(), StringFormat::Literal),
        );
        assert_eq!(out, b"(a\\(b\\)\\\\\\n)");
        out.clear();
        write_object(&mut out, &Object::Real(1.5));
        assert_eq!(out, b"1.5");
    }

    #[test]
    fn base_info_rejects_bad_startxref() {
        assert!(find_base_info(b"%PDF-1.4\nstartxref\n999999\n%%EOF").is_none());
        assert!(find_base_info(b"junk").is_none());
        let ok = b"%PDF-1.4\nxref\n0 1\n0000000000 65535 f \ntrailer\n<<>>\nstartxref\n9\n%%EOF";
        assert_eq!(find_base_info(ok).map(|b| b.kind), Some(XrefKind::Table));
    }
}
