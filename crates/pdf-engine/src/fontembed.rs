//! Font embedding for new content (annotations, stamps, added text, text-edit fallbacks).
//!
//! We embed a *subset* of a bundled, redistributable TrueType font (DejaVu — see
//! `assets/fonts/LICENSE-DejaVu.txt`) as a Type0/CIDFontType2 font with `Identity-H` and a
//! `ToUnicode` map. Characters absent from the font are reported to the caller — they are
//! never silently dropped or replaced.

use crate::doc::Tx;
use crate::error::{EngineError, Result};
use lopdf::{Dictionary, Object, Stream, StringFormat, dictionary};
use std::collections::BTreeMap;
use std::io::Write;
use subsetter::GlyphRemapper;
use ttf_parser::Face;

/// Regular weight of the bundled font.
pub static DEJAVU_SANS: &[u8] = include_bytes!("../assets/fonts/DejaVuSans.ttf");
/// Bold weight of the bundled font.
pub static DEJAVU_SANS_BOLD: &[u8] = include_bytes!("../assets/fonts/DejaVuSans-Bold.ttf");

/// Which bundled face to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BundledFace {
    /// DejaVu Sans.
    Sans,
    /// DejaVu Sans Bold.
    SansBold,
}

impl BundledFace {
    fn data(self) -> &'static [u8] {
        match self {
            BundledFace::Sans => DEJAVU_SANS,
            BundledFace::SansBold => DEJAVU_SANS_BOLD,
        }
    }
    fn base_name(self) -> &'static str {
        match self {
            BundledFace::Sans => "DejaVuSans",
            BundledFace::SansBold => "DejaVuSans-Bold",
        }
    }
}

struct CidInfo {
    text: String,
    width_1000: f64,
}

/// Collects used glyphs and finally writes the subset font objects.
pub struct FontBuilder {
    face_kind: BundledFace,
    face: Face<'static>,
    remap: GlyphRemapper,
    cids: BTreeMap<u16, CidInfo>,
}

impl FontBuilder {
    /// Create a builder for a bundled face.
    pub fn new(kind: BundledFace) -> Result<Self> {
        let face = Face::parse(kind.data(), 0)
            .map_err(|e| EngineError::Unsupported(format!("bundled font unreadable: {e}")))?;
        Ok(Self {
            face_kind: kind,
            face,
            remap: GlyphRemapper::new(),
            cids: BTreeMap::new(),
        })
    }

    /// Original glyph id for a character, if the font covers it.
    pub fn glyph_for(&self, ch: char) -> Option<ttf_parser::GlyphId> {
        self.face.glyph_index(ch)
    }

    /// Whether every non-control character of `text` is covered.
    pub fn missing_chars(&self, text: &str) -> Vec<char> {
        let mut m: Vec<char> = text
            .chars()
            .filter(|c| !c.is_control() && self.glyph_for(*c).is_none())
            .collect();
        m.sort_unstable();
        m.dedup();
        m
    }

    /// Advance of a character in em units (1.0 = font size).
    pub fn advance_em(&self, ch: char) -> f64 {
        let upem = f64::from(self.face.units_per_em());
        self.glyph_for(ch)
            .and_then(|g| self.face.glyph_hor_advance(g))
            .map_or(0.5, |a| f64::from(a) / upem)
    }

    /// Kerning-free text width in points.
    pub fn text_width(&self, text: &str, font_size: f64) -> f64 {
        text.chars().map(|c| self.advance_em(c)).sum::<f64>() * font_size
    }

    /// Line height factor (em) from the font's metrics.
    pub fn line_height_em(&self) -> f64 {
        let upem = f64::from(self.face.units_per_em());
        (f64::from(self.face.ascender()) - f64::from(self.face.descender())
            + f64::from(self.face.line_gap()))
            / upem
    }

    /// Ascender in em.
    pub fn ascent_em(&self) -> f64 {
        f64::from(self.face.ascender()) / f64::from(self.face.units_per_em())
    }

    /// Descender in em (negative).
    pub fn descent_em(&self) -> f64 {
        f64::from(self.face.descender()) / f64::from(self.face.units_per_em())
    }

    /// Register a character as used and return the 2-byte CID code that encodes it in the
    /// subset. Errors if the font has no glyph for it.
    pub fn encode_char(&mut self, ch: char) -> Result<u16> {
        let gid = self.glyph_for(ch).ok_or_else(|| {
            EngineError::Unsupported(format!(
                "the character U+{:04X} is not available in the bundled font",
                ch as u32
            ))
        })?;
        let cid = self.remap.remap(gid.0);
        let upem = f64::from(self.face.units_per_em());
        let w = self
            .face
            .glyph_hor_advance(gid)
            .map_or(500.0, |a| f64::from(a) * 1000.0 / upem);
        self.cids.entry(cid).or_insert(CidInfo {
            text: ch.to_string(),
            width_1000: w,
        });
        Ok(cid)
    }

    /// Encode a string to a big-endian 2-byte code string (for `Tj`).
    pub fn encode_str(&mut self, s: &str) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(s.len() * 2);
        for ch in s.chars() {
            out.extend_from_slice(&self.encode_char(ch)?.to_be_bytes());
        }
        Ok(out)
    }

    /// Write the font objects into the document and return the Type0 font's id.
    pub fn finish(self, tx: &mut Tx<'_>) -> Result<ObjectId2> {
        let sub = subsetter::subset(self.face_kind.data(), 0, &self.remap)
            .map_err(|e| EngineError::Unsupported(format!("font subsetting failed: {e:?}")))?;
        let tag = subset_tag(&self.cids);
        let base = format!("{tag}+{}", self.face_kind.base_name());
        let mut file_stream = Stream::new(
            dictionary! { "Length1" => sub.len() as i64 },
            compress(&sub),
        );
        file_stream
            .dict
            .set("Filter", Object::Name(b"FlateDecode".to_vec()));
        file_stream.allows_compression = false;
        let file_id = tx.add(Object::Stream(file_stream));

        let upem = f64::from(self.face.units_per_em());
        let sc = |v: i16| (f64::from(v) * 1000.0 / upem).round() as i64;
        let bbox = self.face.global_bounding_box();
        let desc_id = tx.add(Object::Dictionary(dictionary! {
            "Type" => "FontDescriptor",
            "FontName" => Object::Name(base.clone().into_bytes()),
            "Flags" => 4i64,
            "FontBBox" => vec![sc(bbox.x_min).into(), sc(bbox.y_min).into(), sc(bbox.x_max).into(), sc(bbox.y_max).into()],
            "ItalicAngle" => 0i64,
            "Ascent" => sc(self.face.ascender()),
            "Descent" => sc(self.face.descender()),
            "CapHeight" => sc(self.face.capital_height().unwrap_or(self.face.ascender())),
            "StemV" => 80i64,
            "FontFile2" => file_id,
        }));

        let mut w = Vec::new();
        for (cid, info) in &self.cids {
            w.push(Object::Integer(i64::from(*cid)));
            w.push(Object::Array(vec![Object::Integer(
                info.width_1000.round() as i64,
            )]));
        }
        let cid_font = tx.add(Object::Dictionary(dictionary! {
            "Type" => "Font",
            "Subtype" => "CIDFontType2",
            "BaseFont" => Object::Name(base.clone().into_bytes()),
            "CIDSystemInfo" => dictionary! {
                "Registry" => Object::String(b"Adobe".to_vec(), StringFormat::Literal),
                "Ordering" => Object::String(b"Identity".to_vec(), StringFormat::Literal),
                "Supplement" => 0i64,
            },
            "FontDescriptor" => desc_id,
            "CIDToGIDMap" => "Identity",
            "DW" => 1000i64,
            "W" => Object::Array(w),
        }));

        let tu = to_unicode_cmap(&self.cids);
        let mut tu_stream = Stream::new(dictionary! {}, compress(tu.as_bytes()));
        tu_stream
            .dict
            .set("Filter", Object::Name(b"FlateDecode".to_vec()));
        tu_stream.allows_compression = false;
        let tu_id = tx.add(Object::Stream(tu_stream));

        Ok(tx.add(Object::Dictionary(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type0",
            "BaseFont" => Object::Name(base.into_bytes()),
            "Encoding" => "Identity-H",
            "DescendantFonts" => vec![Object::Reference(cid_font)],
            "ToUnicode" => tu_id,
        })))
    }
}

/// Object id alias to keep signatures short.
pub type ObjectId2 = lopdf::ObjectId;

fn subset_tag(cids: &BTreeMap<u16, CidInfo>) -> String {
    // Deterministic 6-letter tag derived from the used code points.
    let mut h: u32 = 0x811c_9dc5;
    for (cid, info) in cids {
        for b in cid.to_be_bytes().iter().chain(info.text.as_bytes()) {
            h ^= u32::from(*b);
            h = h.wrapping_mul(16_777_619);
        }
    }
    (0..6)
        .map(|i| char::from(b'A' + ((h >> (i * 5)) % 26) as u8))
        .collect()
}

fn compress(data: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    // Writing to a Vec cannot fail.
    let _ = enc.write_all(data);
    enc.finish().unwrap_or_default()
}

/// Flate-compress bytes (zlib) for use with `/FlateDecode`.
pub fn zlib(data: &[u8]) -> Vec<u8> {
    compress(data)
}

fn to_unicode_cmap(cids: &BTreeMap<u16, CidInfo>) -> String {
    let mut s = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let entries: Vec<_> = cids.iter().collect();
    for chunk in entries.chunks(100) {
        s.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (cid, info) in chunk {
            let utf16: String = info
                .text
                .encode_utf16()
                .map(|u| format!("{u:04X}"))
                .collect();
            s.push_str(&format!("<{cid:04X}> <{utf16}>\n"));
        }
        s.push_str("endbfchar\n");
    }
    s.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    s
}

/// Escape bytes as a PDF hex string for content streams.
pub fn hex_string(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2 + 2);
    s.push('<');
    for b in bytes {
        s.push_str(&format!("{b:02X}"));
    }
    s.push('>');
    s
}

/// Make a dictionary of `{ name => reference }` for `/Resources /Font`.
pub fn font_resource(name: &str, id: lopdf::ObjectId) -> Dictionary {
    let mut d = Dictionary::new();
    d.set(name, Object::Reference(id));
    d
}
