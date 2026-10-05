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

/// Regular weight of the default bundled font.
pub static DEJAVU_SANS: &[u8] = include_bytes!("../assets/fonts/DejaVuSans.ttf");
/// Bold weight of the default bundled font.
pub static DEJAVU_SANS_BOLD: &[u8] = include_bytes!("../assets/fonts/DejaVuSans-Bold.ttf");

/// The bundled font families offered for new and replaced text. All are redistributable
/// (DejaVu licence, SIL OFL 1.1 for Liberation — see `assets/fonts/LICENSE-*.txt`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontFamily {
    /// DejaVu Sans — widest language coverage; the default.
    DejaVuSans,
    /// DejaVu Serif.
    DejaVuSerif,
    /// Liberation Sans — metric-compatible with Arial/Helvetica.
    LiberationSans,
    /// Liberation Serif — metric-compatible with Times New Roman.
    LiberationSerif,
    /// Liberation Mono — metric-compatible with Courier New.
    LiberationMono,
    /// A font installed on this computer (an index into [`crate::sysfonts`]).
    System(u32),
}

impl FontFamily {
    /// Every family, in the order shown to the user.
    pub const ALL: [FontFamily; 5] = [
        FontFamily::LiberationSans,
        FontFamily::LiberationSerif,
        FontFamily::LiberationMono,
        FontFamily::DejaVuSans,
        FontFamily::DejaVuSerif,
    ];

    /// Name shown in menus.
    pub fn title(self) -> &'static str {
        match self {
            FontFamily::DejaVuSans => "DejaVu Sans",
            FontFamily::DejaVuSerif => "DejaVu Serif",
            FontFamily::LiberationSans => "Liberation Sans (Arial-like)",
            FontFamily::LiberationSerif => "Liberation Serif (Times-like)",
            FontFamily::LiberationMono => "Liberation Mono (Courier-like)",
            FontFamily::System(id) => {
                crate::sysfonts::with_family(id, |f| f.name).unwrap_or("Unavailable system font")
            }
        }
    }

    /// Stable identifier used in files and preferences.
    pub fn key(self) -> &'static str {
        match self {
            FontFamily::DejaVuSans => "DejaVuSans",
            FontFamily::DejaVuSerif => "DejaVuSerif",
            FontFamily::LiberationSans => "LiberationSans",
            FontFamily::LiberationSerif => "LiberationSerif",
            FontFamily::LiberationMono => "LiberationMono",
            FontFamily::System(id) => crate::sysfonts::with_family(id, |f| f.key).unwrap_or("sys:"),
        }
    }

    /// Inverse of [`FontFamily::key`]. System fonts are found by name (`sys:Arial`) and only
    /// resolve once they have been scanned on this computer.
    pub fn from_key(k: &str) -> Option<FontFamily> {
        if let Some(name) = k.strip_prefix("sys:") {
            return crate::sysfonts::find_by_name(name).map(FontFamily::System);
        }
        FontFamily::ALL.into_iter().find(|f| f.key() == k)
    }

    /// Whether the family has an italic face. (DejaVu Sans/Serif are offered upright and bold
    /// only; Liberation has all four styles.)
    pub fn has_italic(self) -> bool {
        match self {
            FontFamily::System(id) => {
                crate::sysfonts::with_family(id, |f| f.has_italic()).unwrap_or(false)
            }
            _ => matches!(
                self,
                FontFamily::LiberationSans
                    | FontFamily::LiberationSerif
                    | FontFamily::LiberationMono
            ),
        }
    }

    /// Whether the family has a real bold face (system fonts may not; bundled ones always do).
    pub fn has_bold(self) -> bool {
        match self {
            FontFamily::System(id) => {
                crate::sysfonts::with_family(id, |f| f.has_bold()).unwrap_or(false)
            }
            _ => true,
        }
    }
}

/// A family with weight and slant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontStyle {
    /// Family.
    pub family: FontFamily,
    /// Bold weight.
    pub bold: bool,
    /// Italic slant (ignored, and normalised to `false`, for families without italics).
    pub italic: bool,
}

impl Default for FontStyle {
    fn default() -> Self {
        FontStyle::new(FontFamily::DejaVuSans, false, false)
    }
}

impl FontStyle {
    /// Build a style; italic is dropped when the family has none.
    pub fn new(family: FontFamily, bold: bool, italic: bool) -> Self {
        Self {
            family,
            bold,
            italic: italic && family.has_italic(),
        }
    }

    /// PostScript-like name of the face, used as the embedded font's base name and stored in
    /// annotations so the choice survives saving and re-opening.
    pub fn base_name(self) -> String {
        let suffix = match (self.bold, self.italic) {
            (false, false) => "",
            (true, false) => "-Bold",
            (false, true) => "-Italic",
            (true, true) => "-BoldItalic",
        };
        if let FontFamily::System(_) = self.family {
            return format!("{}{suffix}", self.family.key());
        }
        match (self.family, suffix) {
            (
                FontFamily::LiberationSans
                | FontFamily::LiberationSerif
                | FontFamily::LiberationMono,
                "",
            ) => {
                format!("{}-Regular", self.family.key())
            }
            _ => format!("{}{suffix}", self.family.key()),
        }
    }

    /// Inverse of [`FontStyle::base_name`] (also accepts names without `-Regular`).
    pub fn from_base_name(n: &str) -> Option<FontStyle> {
        if let Some(rest) = n.strip_prefix("sys:") {
            for (suffix, b, i) in [
                ("-BoldItalic", true, true),
                ("-Bold", true, false),
                ("-Italic", false, true),
                ("", false, false),
            ] {
                if let Some(name) = rest.strip_suffix(suffix)
                    && let Some(id) = crate::sysfonts::find_by_name(name)
                {
                    return Some(FontStyle::new(FontFamily::System(id), b, i));
                }
            }
            return None;
        }
        FontFamily::ALL.into_iter().find_map(|f| {
            let rest = n.strip_prefix(f.key())?;
            let (b, i) = match rest {
                "" | "-Regular" => (false, false),
                "-Bold" => (true, false),
                "-Italic" => (false, true),
                "-BoldItalic" => (true, true),
                _ => return None,
            };
            Some(FontStyle::new(f, b, i))
        })
    }

    /// Index of the face inside its file (non-zero only for system fonts in collections).
    pub fn face_index(self) -> u32 {
        match self.family {
            FontFamily::System(id) => crate::sysfonts::with_family(id, |f| {
                f.face(self.bold, self.italic).map_or(0, |x| x.index)
            })
            .unwrap_or(0),
            _ => 0,
        }
    }

    /// Name used inside the PDF font objects (no `sys:` prefix, no spaces).
    pub fn pdf_name(self) -> String {
        self.base_name()
            .trim_start_matches("sys:")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            .collect()
    }

    /// The font program's bytes. A system font that cannot be read falls back to DejaVu Sans.
    pub fn data(self) -> &'static [u8] {
        use FontFamily::*;
        if let System(id) = self.family {
            return crate::sysfonts::with_family(id, |f| {
                f.face(self.bold, self.italic)
                    .and_then(crate::sysfonts::SysFace::data)
            })
            .flatten()
            .unwrap_or(DEJAVU_SANS);
        }
        match (self.family, self.bold, self.italic) {
            (DejaVuSans, false, _) => DEJAVU_SANS,
            (DejaVuSans, true, _) => DEJAVU_SANS_BOLD,
            (DejaVuSerif, false, _) => include_bytes!("../assets/fonts/DejaVuSerif.ttf"),
            (DejaVuSerif, true, _) => include_bytes!("../assets/fonts/DejaVuSerif-Bold.ttf"),
            (LiberationSans, false, false) => {
                include_bytes!("../assets/fonts/LiberationSans-Regular.ttf")
            }
            (LiberationSans, true, false) => {
                include_bytes!("../assets/fonts/LiberationSans-Bold.ttf")
            }
            (LiberationSans, false, true) => {
                include_bytes!("../assets/fonts/LiberationSans-Italic.ttf")
            }
            (LiberationSans, true, true) => {
                include_bytes!("../assets/fonts/LiberationSans-BoldItalic.ttf")
            }
            (LiberationSerif, false, false) => {
                include_bytes!("../assets/fonts/LiberationSerif-Regular.ttf")
            }
            (LiberationSerif, true, false) => {
                include_bytes!("../assets/fonts/LiberationSerif-Bold.ttf")
            }
            (LiberationSerif, false, true) => {
                include_bytes!("../assets/fonts/LiberationSerif-Italic.ttf")
            }
            (LiberationSerif, true, true) => {
                include_bytes!("../assets/fonts/LiberationSerif-BoldItalic.ttf")
            }
            (LiberationMono, false, false) => {
                include_bytes!("../assets/fonts/LiberationMono-Regular.ttf")
            }
            (LiberationMono, true, false) => {
                include_bytes!("../assets/fonts/LiberationMono-Bold.ttf")
            }
            (LiberationMono, false, true) => {
                include_bytes!("../assets/fonts/LiberationMono-Italic.ttf")
            }
            (LiberationMono, true, true) => {
                include_bytes!("../assets/fonts/LiberationMono-BoldItalic.ttf")
            }
            (System(_), _, _) => DEJAVU_SANS,
        }
    }
}

/// Which bundled face to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BundledFace {
    /// DejaVu Sans.
    Sans,
    /// DejaVu Sans Bold.
    SansBold,
    /// Any bundled family and style.
    Styled(FontStyle),
}

impl From<FontStyle> for BundledFace {
    fn from(s: FontStyle) -> Self {
        BundledFace::Styled(s)
    }
}

impl BundledFace {
    fn style(self) -> FontStyle {
        match self {
            BundledFace::Sans => FontStyle::new(FontFamily::DejaVuSans, false, false),
            BundledFace::SansBold => FontStyle::new(FontFamily::DejaVuSans, true, false),
            BundledFace::Styled(s) => s,
        }
    }
    fn data(self) -> &'static [u8] {
        self.style().data()
    }
    fn index(self) -> u32 {
        self.style().face_index()
    }
    fn base_name(self) -> String {
        self.style().pdf_name()
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
        let face = Face::parse(kind.data(), kind.index())
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
        let sub = subsetter::subset(self.face_kind.data(), self.face_kind.index(), &self.remap)
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

#[cfg(test)]
mod font_style_tests {
    use super::*;

    #[test]
    fn names_round_trip_for_every_family_and_style() {
        for f in FontFamily::ALL {
            for bold in [false, true] {
                for italic in [false, true] {
                    let s = FontStyle::new(f, bold, italic);
                    assert_eq!(FontStyle::from_base_name(&s.base_name()), Some(s), "{s:?}");
                    assert_eq!(FontFamily::from_key(f.key()), Some(f));
                    // The font program must parse and cover basic Latin.
                    let face = ttf_parser::Face::parse(s.data(), 0).unwrap();
                    assert!(face.glyph_index('A').is_some() && face.glyph_index('é').is_some());
                }
            }
        }
        assert_eq!(FontStyle::from_base_name("Comic-Sans"), None);
        // Historic names stay valid.
        assert_eq!(
            FontStyle::from_base_name("DejaVuSans-Bold"),
            Some(FontStyle::new(FontFamily::DejaVuSans, true, false))
        );
    }

    #[test]
    fn italic_is_dropped_where_the_family_has_none() {
        assert!(!FontStyle::new(FontFamily::DejaVuSerif, false, true).italic);
        assert!(FontStyle::new(FontFamily::LiberationSerif, false, true).italic);
        assert_eq!(FontStyle::default().base_name(), "DejaVuSans");
        assert_eq!(
            FontStyle::new(FontFamily::LiberationMono, true, true).base_name(),
            "LiberationMono-BoldItalic"
        );
    }

    #[test]
    fn liberation_covers_latin_greek_and_cyrillic() {
        for f in [
            FontFamily::LiberationSans,
            FontFamily::LiberationSerif,
            FontFamily::LiberationMono,
        ] {
            let fb =
                FontBuilder::new(BundledFace::Styled(FontStyle::new(f, false, false))).unwrap();
            assert!(
                fb.missing_chars("Dutch: ëïöü €. Ελληνικά. Русский.")
                    .is_empty(),
                "{f:?}"
            );
            assert!(!fb.missing_chars("漢字").is_empty());
        }
    }
}
