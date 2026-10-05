//! Font decoding/encoding for text-run editing.
//!
//! Supported subset (anything else yields an explicit `unsupported` reason):
//!
//! * Simple fonts (`Type1`, `MMType1`, `TrueType`) with `/Encoding` (`WinAnsi`, `MacRoman`,
//!   `Standard`, or a dictionary with `/BaseEncoding` + `/Differences`) and/or `/ToUnicode`.
//!   Widths from `/Widths`, or from the Adobe base-14 metrics for the standard fonts.
//! * `Type0` fonts with an `Identity-H` encoding and a `CIDFontType0/2` descendant, with
//!   `/ToUnicode`; widths from `/W` and `/DW`.
//!
//! Not supported: Type 3 fonts, other CMaps (vertical writing, CJK predefined CMaps),
//! `MacExpertEncoding`, and fonts for which no Unicode mapping is known.
//!
//! **Glyph availability.** Embedded fonts are usually *subsets*: the encoding table may name
//! 224 characters while the font program contains a dozen glyphs. Encoding a new character
//! with such a font would silently render nothing. For embedded fonts we therefore only
//! offer characters whose glyph is verified in the font program (TrueType/CFF), or — when the
//! font program cannot be inspected — whose code is already used in the document text.

use crate::content::FontMetrics;
use crate::objutil;
use hayro::hayro_interpret::hayro_cmap::{BfString, CMap};
use lopdf::{Dictionary, Document, Object};
use pdf_base14_metrics::Base14Font;
use std::collections::{HashMap, HashSet};

/// One element of encoded output.
#[derive(Clone, Debug, PartialEq)]
pub enum Enc {
    /// A character code string (1 or 2 bytes per code).
    Codes(Vec<u8>),
    /// A horizontal gap in 1/1000 em (used for spaces the font cannot encode).
    Gap(f64),
}

/// Result of encoding text for a font.
#[derive(Clone, Debug, Default)]
pub struct Encoded {
    /// Encoded pieces in order.
    pub items: Vec<Enc>,
    /// Characters that cannot be shown with this font.
    pub missing: Vec<char>,
    /// Spaces approximated by gaps (informational).
    pub gap_spaces: usize,
}

/// Parsed font information.
#[derive(Clone)]
pub struct FontInfo {
    /// `/BaseFont` name without the subset tag.
    pub base_font: String,
    /// Font subtype name.
    pub subtype: String,
    /// A font program is embedded.
    pub embedded: bool,
    /// `BaseFont` carries a subset tag (`ABCDEF+Name`).
    pub subset: bool,
    /// Bytes per character code.
    pub code_len: usize,
    /// Reason text editing with this font is unsupported.
    pub unsupported: Option<String>,
    widths: Widths,
    unicode: HashMap<u32, String>,
    reverse: HashMap<String, u32>,
    max_key_chars: usize,
    names: HashMap<u32, String>,
    verified: Option<HashSet<u32>>,
    ascent: f64,
    descent: f64,
}

#[derive(Clone)]
enum Widths {
    None,
    Simple {
        first: u32,
        widths: Vec<f64>,
        missing: f64,
    },
    Cid {
        dw: f64,
        map: HashMap<u32, f64>,
    },
    Base14 {
        font: Base14Font,
        names: Vec<Option<String>>,
    },
}

fn strip_subset(name: &str) -> (String, bool) {
    let b = name.as_bytes();
    if b.len() > 7 && b[6] == b'+' && b[..6].iter().all(u8::is_ascii_uppercase) {
        (name[7..].to_string(), true)
    } else {
        (name.to_string(), false)
    }
}

fn base14_for(name: &str) -> Option<Base14Font> {
    let n: String = name
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | ',' | '_'))
        .collect::<String>()
        .to_ascii_lowercase();
    let bold = n.contains("bold");
    let italic = n.contains("italic") || n.contains("oblique");
    let family = if n.starts_with("helvetica") || n.starts_with("arial") {
        0
    } else if n.starts_with("times") {
        1
    } else if n.starts_with("courier") {
        2
    } else if n == "symbol" {
        return Some(Base14Font::Symbol);
    } else if n.starts_with("zapfdingbats") {
        return Some(Base14Font::ZapfDingbats);
    } else {
        return None;
    };
    Some(match (family, bold, italic) {
        (0, false, false) => Base14Font::Helvetica,
        (0, true, false) => Base14Font::HelveticaBold,
        (0, false, true) => Base14Font::HelveticaOblique,
        (0, true, true) => Base14Font::HelveticaBoldOblique,
        (1, false, false) => Base14Font::TimesRoman,
        (1, true, false) => Base14Font::TimesBold,
        (1, false, true) => Base14Font::TimesItalic,
        (1, true, true) => Base14Font::TimesBoldItalic,
        (_, false, false) => Base14Font::Courier,
        (_, true, false) => Base14Font::CourierBold,
        (_, false, true) => Base14Font::CourierOblique,
        (_, true, true) => Base14Font::CourierBoldOblique,
    })
}

fn bf_to_string(b: BfString) -> String {
    match b {
        BfString::Char(c) => c.to_string(),
        BfString::String(s) => s,
    }
}

impl FontInfo {
    /// Load from a font dictionary.
    pub fn load(doc: &Document, font: &Dictionary) -> FontInfo {
        let subtype = objutil::dict_name(doc, font, b"Subtype")
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .unwrap_or_default();
        let raw_base = objutil::dict_name(doc, font, b"BaseFont")
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .unwrap_or_default();
        let (base_font, subset) = strip_subset(&raw_base);
        let mut fi = FontInfo {
            base_font,
            subtype: subtype.clone(),
            embedded: false,
            subset,
            code_len: 1,
            unsupported: None,
            widths: Widths::None,
            unicode: HashMap::new(),
            reverse: HashMap::new(),
            max_key_chars: 1,
            names: HashMap::new(),
            verified: None,
            ascent: 800.0,
            descent: -200.0,
        };
        match subtype.as_str() {
            "Type0" => fi.load_type0(doc, font),
            "Type1" | "MMType1" | "TrueType" => fi.load_simple(doc, font),
            "Type3" => fi.unsupported = Some("Type 3 fonts cannot be edited".into()),
            other => fi.unsupported = Some(format!("font type “{other}” is not supported")),
        }
        fi.finish_reverse();
        fi
    }

    fn load_descriptor(&mut self, doc: &Document, d: &Dictionary) -> Option<Vec<u8>> {
        let fd = objutil::dict_dict(doc, d, b"FontDescriptor")?;
        if let Some(a) = objutil::dict_num(doc, fd, b"Ascent")
            && a != 0.0
        {
            self.ascent = a;
        }
        if let Some(dd) = objutil::dict_num(doc, fd, b"Descent")
            && dd != 0.0
        {
            self.descent = -dd.abs();
        }
        for key in [&b"FontFile2"[..], b"FontFile3", b"FontFile"] {
            if let Some(Object::Stream(s)) = fd.get(key).ok().and_then(|o| objutil::deref(doc, o)) {
                self.embedded = true;
                return s.decompressed_content_with_limit(64 * 1024 * 1024).ok();
            }
        }
        None
    }

    fn parse_to_unicode(
        &mut self,
        doc: &Document,
        font: &Dictionary,
        codes: impl Iterator<Item = u32>,
    ) {
        let Some(Object::Stream(s)) = font
            .get(b"ToUnicode")
            .ok()
            .and_then(|o| objutil::deref(doc, o))
        else {
            return;
        };
        let Ok(data) = s.decompressed_content_with_limit(8 * 1024 * 1024) else {
            return;
        };
        let Some(cmap) = CMap::parse(&data, |_| None) else {
            return;
        };
        for c in codes {
            if let Some(b) = cmap.lookup_bf_string(c) {
                let s = bf_to_string(b);
                if !s.is_empty() && s != "\u{FFFD}" {
                    self.unicode.insert(c, s);
                }
            }
        }
    }

    fn load_type0(&mut self, doc: &Document, font: &Dictionary) {
        let enc = objutil::deref(doc, font.get(b"Encoding").unwrap_or(&Object::Null))
            .and_then(|o| o.as_name().ok());
        if enc != Some(b"Identity-H") {
            self.unsupported =
                Some("this font uses a non-identity CMap or vertical writing".into());
            return;
        }
        self.code_len = 2;
        let Some(desc) = objutil::dict_array(doc, font, b"DescendantFonts")
            .and_then(|a| a.first())
            .and_then(|o| objutil::deref(doc, o))
            .and_then(|o| o.as_dict().ok())
        else {
            self.unsupported = Some("missing descendant font".into());
            return;
        };
        let sub = objutil::dict_name(doc, desc, b"Subtype").unwrap_or_default();
        if sub != b"CIDFontType2" && sub != b"CIDFontType0" {
            self.unsupported = Some("unsupported descendant font type".into());
            return;
        }
        let prog = self.load_descriptor(doc, desc);
        // Widths.
        let dw = objutil::dict_num(doc, desc, b"DW").unwrap_or(1000.0);
        let mut map = HashMap::new();
        if let Some(w) = objutil::dict_array(doc, desc, b"W") {
            let mut i = 0;
            while i < w.len() {
                let Some(first) = objutil::num(doc, &w[i]) else {
                    break;
                };
                match w.get(i + 1).and_then(|o| objutil::deref(doc, o)) {
                    Some(Object::Array(list)) => {
                        for (k, wv) in list.iter().enumerate() {
                            if let Some(v) = objutil::num(doc, wv) {
                                map.insert(first as u32 + k as u32, v);
                            }
                        }
                        i += 2;
                    }
                    Some(_) => {
                        let (Some(last), Some(v)) = (
                            w.get(i + 1).and_then(|o| objutil::num(doc, o)),
                            w.get(i + 2).and_then(|o| objutil::num(doc, o)),
                        ) else {
                            break;
                        };
                        for c in (first as u32)..=(last as u32).min(first as u32 + 65_535) {
                            map.insert(c, v);
                        }
                        i += 3;
                    }
                    None => break,
                }
            }
        }
        self.widths = Widths::Cid { dw, map };
        self.parse_to_unicode(doc, font, 0..=0xFFFF);
        if self.unicode.is_empty() {
            self.unsupported =
                Some("this font has no ToUnicode map, so its characters are unknown".into());
            return;
        }
        // Verify glyphs exist in the embedded TrueType program (identity CID→GID only).
        if let Some(data) = prog
            && sub == b"CIDFontType2"
            && objutil::dict_name(doc, desc, b"CIDToGIDMap").is_none_or(|n| n == b"Identity")
            && let Ok(face) = ttf_parser::Face::parse(&data, 0)
        {
            let mut ok = HashSet::new();
            for (code, text) in &self.unicode {
                let gid = ttf_parser::GlyphId(*code as u16);
                if (*code as u16) < face.number_of_glyphs()
                    && (face.glyph_bounding_box(gid).is_some()
                        || text.chars().all(char::is_whitespace))
                {
                    ok.insert(*code);
                }
            }
            self.verified = Some(ok);
        }
    }

    fn load_simple(&mut self, doc: &Document, font: &Dictionary) {
        let prog = self.load_descriptor(doc, font);
        let enc_obj = font
            .get(b"Encoding")
            .ok()
            .and_then(|o| objutil::deref(doc, o));
        let fd = objutil::dict_dict(doc, font, b"FontDescriptor");
        let flags = fd
            .and_then(|d| objutil::dict_num(doc, d, b"Flags"))
            .unwrap_or(0.0) as u32;
        let symbolic = flags & 4 != 0 && flags & 32 == 0;
        let b14 = if self.embedded {
            None
        } else {
            base14_for(&self.base_font)
        };
        // Encoding table: code -> glyph name (and unicode).
        let mut table: Vec<Option<String>> = vec![None; 256];
        let name_table = |f: fn(u8) -> Option<&'static str>| -> Vec<Option<String>> {
            (0..=255u8).map(|c| f(c).map(str::to_string)).collect()
        };
        let mut have_encoding = false;
        let mut base_name: Option<String> = None;
        let mut diffs: Option<&Vec<Object>> = None;
        match enc_obj {
            Some(Object::Name(n)) => base_name = Some(String::from_utf8_lossy(n).into_owned()),
            Some(Object::Dictionary(d)) => {
                base_name = objutil::dict_name(doc, d, b"BaseEncoding")
                    .map(|n| String::from_utf8_lossy(n).into_owned());
                diffs = objutil::dict_array(doc, d, b"Differences");
            }
            _ => {}
        }
        match base_name.as_deref() {
            Some("WinAnsiEncoding") => {
                table = name_table(pdfboss_encoding::win_ansi_glyph_name);
                have_encoding = true;
            }
            Some("MacRomanEncoding") => {
                table = name_table(pdfboss_encoding::mac_roman_glyph_name);
                have_encoding = true;
            }
            Some("StandardEncoding") => {
                table = name_table(pdfboss_encoding::standard_encoding_name);
                have_encoding = true;
            }
            Some("MacExpertEncoding") => {
                self.unsupported = Some("MacExpertEncoding is not supported".into());
                return;
            }
            Some(_) => {}
            None => {
                // No base encoding: standard fonts default to their built-in/Standard encoding.
                if let Some(b) = b14 {
                    table = match b {
                        Base14Font::Symbol => name_table(pdfboss_encoding::symbol_glyph_name),
                        Base14Font::ZapfDingbats => {
                            name_table(pdfboss_encoding::zapf_dingbats_glyph_name)
                        }
                        _ => name_table(pdfboss_encoding::standard_encoding_name),
                    };
                    have_encoding = true;
                } else if !symbolic && diffs.is_some() {
                    table = name_table(pdfboss_encoding::standard_encoding_name);
                    have_encoding = true;
                }
            }
        }
        if let Some(d) = diffs {
            let mut code = 0usize;
            for o in d {
                match objutil::deref(doc, o) {
                    Some(Object::Integer(n)) if *n >= 0 => code = *n as usize,
                    Some(Object::Real(r)) if *r >= 0.0 => code = *r as usize,
                    Some(Object::Name(n)) => {
                        if code < 256 {
                            table[code] = Some(String::from_utf8_lossy(n).into_owned());
                        }
                        code += 1;
                    }
                    _ => {}
                }
            }
            have_encoding = true;
        }
        for (c, name) in table.iter().enumerate() {
            if let Some(n) = name {
                self.names.insert(c as u32, n.clone());
                if let Some(text) = pdfboss_encoding::glyph_to_text(n) {
                    self.unicode.insert(c as u32, text);
                }
            }
        }
        // ToUnicode overrides/extends (what viewers use for text extraction).
        let before = self.unicode.clone();
        self.parse_to_unicode(doc, font, 0..=255);
        let has_tu = self.unicode != before || font.has(b"ToUnicode");
        if self.unicode.is_empty() {
            self.unsupported = Some(
                "no character encoding or ToUnicode map: the characters of this font are unknown"
                    .into(),
            );
            return;
        }
        if !have_encoding && !has_tu {
            self.unsupported = Some("no character encoding is defined for this font".into());
            return;
        }
        // Widths.
        let first = objutil::dict_num(doc, font, b"FirstChar").unwrap_or(0.0) as u32;
        let missing = fd
            .and_then(|d| objutil::dict_num(doc, d, b"MissingWidth"))
            .unwrap_or(0.0);
        if let Some(w) = objutil::dict_array(doc, font, b"Widths") {
            self.widths = Widths::Simple {
                first,
                widths: w
                    .iter()
                    .map(|o| objutil::num(doc, o).unwrap_or(0.0))
                    .collect(),
                missing,
            };
        } else if let Some(b) = b14 {
            self.widths = Widths::Base14 {
                font: b,
                names: table.clone(),
            };
        } else {
            self.unsupported = Some("no width information for this font".into());
            return;
        }
        // Verify glyphs in the embedded font program.
        if let Some(data) = prog {
            self.verified = self.verify_simple(&data);
        }
    }

    fn verify_simple(&self, data: &[u8]) -> Option<HashSet<u32>> {
        let mut ok = HashSet::new();
        if let Ok(face) = ttf_parser::Face::parse(data, 0) {
            let has_unicode_cmap = face
                .tables()
                .cmap
                .is_some_and(|c| c.subtables.into_iter().any(|s| s.is_unicode()));
            if !has_unicode_cmap {
                return None;
            }
            for (code, text) in &self.unicode {
                if let Some(ch) = text.chars().next()
                    && let Some(g) = face.glyph_index(ch)
                    && (face.glyph_bounding_box(g).is_some() || ch.is_whitespace())
                {
                    ok.insert(*code);
                }
            }
            return Some(ok);
        }
        if let Some(cff) = ttf_parser::cff::Table::parse(data) {
            for (code, name) in &self.names {
                if cff.glyph_index_by_name(name).is_some() {
                    ok.insert(*code);
                }
            }
            return Some(ok);
        }
        None
    }

    fn finish_reverse(&mut self) {
        // Prefer the lowest code for each text.
        let mut codes: Vec<_> = self.unicode.iter().collect();
        codes.sort_by_key(|(c, _)| **c);
        for (c, t) in codes {
            self.max_key_chars = self.max_key_chars.max(t.chars().count());
            self.reverse.entry(t.clone()).or_insert(*c);
        }
    }

    /// Text of a code, if known.
    pub fn unicode_of(&self, code: u32) -> Option<&str> {
        self.unicode.get(&code).map(String::as_str)
    }

    /// Whether editing with this font is possible at all.
    pub fn can_edit(&self) -> Result<(), String> {
        match &self.unsupported {
            Some(r) => Err(r.clone()),
            None => Ok(()),
        }
    }

    /// Width of the space character in 1/1000 em (best effort).
    pub fn space_width(&self) -> f64 {
        if let Some(&c) = self.reverse.get(" ") {
            return self.width1000(c);
        }
        280.0
    }

    /// Encode `text`. `used` are codes already shown with this font in the document; they are
    /// trusted when the font program cannot be inspected.
    pub fn encode(&self, text: &str, used: &HashSet<u32>) -> Encoded {
        let mut out = Encoded::default();
        let chars: Vec<char> = text.chars().collect();
        let mut buf: Vec<u8> = Vec::new();
        let flush = |buf: &mut Vec<u8>, out: &mut Encoded| {
            if !buf.is_empty() {
                out.items.push(Enc::Codes(std::mem::take(buf)));
            }
        };
        let mut i = 0;
        while i < chars.len() {
            // Longest match first (ligature strings).
            let mut found: Option<(usize, u32)> = None;
            for len in (1..=self.max_key_chars.min(chars.len() - i)).rev() {
                let key: String = chars[i..i + len].iter().collect();
                if let Some(&code) = self.reverse.get(&key)
                    && self.code_available(code, used)
                {
                    found = Some((len, code));
                    break;
                }
            }
            match found {
                Some((len, code)) => {
                    if self.code_len == 2 {
                        buf.extend_from_slice(&(code as u16).to_be_bytes());
                    } else {
                        buf.push(code as u8);
                    }
                    i += len;
                }
                None => {
                    let c = chars[i];
                    if c == ' ' || c == '\u{00A0}' {
                        flush(&mut buf, &mut out);
                        out.items.push(Enc::Gap(self.space_width()));
                        out.gap_spaces += 1;
                    } else if !c.is_control() {
                        out.missing.push(c);
                    }
                    i += 1;
                }
            }
        }
        flush(&mut buf, &mut out);
        out.missing.sort_unstable();
        out.missing.dedup();
        out
    }

    fn code_available(&self, code: u32, used: &HashSet<u32>) -> bool {
        if !self.embedded {
            return true;
        }
        match &self.verified {
            Some(v) => v.contains(&code),
            None => used.contains(&code),
        }
    }

    /// Width in 1/1000 em of an encoded text (without spacing parameters).
    pub fn text_width1000(&self, enc: &Encoded) -> f64 {
        let mut w = 0.0;
        for it in &enc.items {
            match it {
                Enc::Gap(g) => w += g,
                Enc::Codes(b) => {
                    for c in self.codes(b) {
                        w += self.width1000(c);
                    }
                }
            }
        }
        w
    }
}

impl FontMetrics for FontInfo {
    fn codes(&self, s: &[u8]) -> Vec<u32> {
        if self.code_len == 2 {
            s.chunks(2)
                .map(|c| {
                    if c.len() == 2 {
                        u32::from(c[0]) << 8 | u32::from(c[1])
                    } else {
                        u32::from(c[0])
                    }
                })
                .collect()
        } else {
            s.iter().map(|b| u32::from(*b)).collect()
        }
    }

    fn width1000(&self, code: u32) -> f64 {
        match &self.widths {
            Widths::Simple {
                first,
                widths,
                missing,
            } => code
                .checked_sub(*first)
                .and_then(|i| widths.get(i as usize))
                .copied()
                .unwrap_or(*missing),
            Widths::Cid { dw, map } => map.get(&code).copied().unwrap_or(*dw),
            Widths::Base14 { font, names } => names
                .get(code as usize)
                .and_then(|n| n.as_deref())
                .and_then(|n| {
                    if matches!(font, Base14Font::Symbol | Base14Font::ZapfDingbats) {
                        font.glyph_width(n)
                    } else {
                        font.glyph_width_by_name(n)
                    }
                })
                .map_or(0.0, f64::from),
            Widths::None => 0.0,
        }
    }

    fn is_word_space(&self, code: u32) -> bool {
        self.code_len == 1 && code == 32
    }

    fn ascent1000(&self) -> f64 {
        self.ascent
    }

    fn descent1000(&self) -> f64 {
        self.descent
    }

    fn text_of(&self, code: u32) -> Option<String> {
        self.unicode.get(&code).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    fn helvetica(doc: &Document) -> FontInfo {
        let d = dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" };
        FontInfo::load(doc, &d)
    }

    #[test]
    fn base14_helvetica_widths_and_encoding() {
        let f = helvetica(&Document::new());
        assert!(f.can_edit().is_ok());
        assert!(!f.embedded);
        assert_eq!(f.width1000(u32::from(b'A')), 667.0);
        assert_eq!(f.width1000(32), 278.0);
        let e = f.encode("Añ€", &HashSet::new());
        assert!(e.missing.is_empty(), "{:?}", e.missing);
        assert_eq!(e.items, vec![Enc::Codes(vec![b'A', 0xF1, 0x80])]);
        // Characters outside WinAnsi are reported, not dropped.
        let e = f.encode("a→b", &HashSet::new());
        assert_eq!(e.missing, vec!['→']);
    }

    #[test]
    fn differences_override_the_base_encoding() {
        let d = dictionary! {
            "Subtype" => "Type1", "BaseFont" => "Times-Roman",
            "Encoding" => dictionary! { "BaseEncoding" => "WinAnsiEncoding", "Differences" => vec![Object::Integer(65), Object::Name(b"eacute".to_vec()), Object::Name(b"fi".to_vec())] },
        };
        let f = FontInfo::load(&Document::new(), &d);
        assert_eq!(f.unicode_of(65), Some("é"));
        assert_eq!(f.unicode_of(66), Some("ﬁ"));
        assert_eq!(
            f.encode("é", &HashSet::new()).items,
            vec![Enc::Codes(vec![65])]
        );
    }

    #[test]
    fn unsupported_fonts_say_why() {
        let doc = Document::new();
        let t3 = FontInfo::load(&doc, &dictionary! { "Subtype" => "Type3" });
        assert!(t3.can_edit().unwrap_err().contains("Type 3"));
        let noenc = FontInfo::load(
            &doc,
            &dictionary! { "Subtype" => "TrueType", "BaseFont" => "Foo" },
        );
        assert!(noenc.can_edit().is_err());
        let cjk = FontInfo::load(
            &doc,
            &dictionary! { "Subtype" => "Type0", "BaseFont" => "X", "Encoding" => "UniJIS-UCS2-H" },
        );
        assert!(cjk.can_edit().unwrap_err().contains("CMap"));
    }

    #[test]
    fn unencodable_spaces_become_gaps_not_missing_glyphs() {
        // A subset-style font that has letters but no space glyph.
        let mut f = helvetica(&Document::new());
        f.embedded = true;
        f.verified = Some((u32::from(b'a')..=u32::from(b'z')).collect());
        f.reverse.remove(" ");
        let e = f.encode("a b", &HashSet::new());
        assert_eq!(e.gap_spaces, 1);
        assert!(e.missing.is_empty());
        assert_eq!(e.items.len(), 3);
        // But capital letters are not in the verified subset: reported missing.
        assert_eq!(f.encode("A", &HashSet::new()).missing, vec!['A']);
        // Unless the document already uses that code with this font.
        let used: HashSet<u32> = [u32::from(b'A')].into_iter().collect();
        assert_eq!(
            f.encode("A", &used).missing,
            vec!['A'],
            "verification of the font program wins over document usage"
        );
        // With no way to inspect the program, previously used codes are trusted.
        f.verified = None;
        assert!(f.encode("A", &used).missing.is_empty());
        assert_eq!(f.encode("B", &used).missing, vec!['B']);
    }

    #[test]
    fn subset_tags_are_stripped() {
        assert_eq!(
            strip_subset("ABCDEF+DejaVuSans"),
            ("DejaVuSans".to_string(), true)
        );
        assert_eq!(strip_subset("Helvetica"), ("Helvetica".to_string(), false));
        assert_eq!(base14_for("ArialMT"), Some(Base14Font::Helvetica));
        assert_eq!(
            base14_for("Times-BoldItalic"),
            Some(Base14Font::TimesBoldItalic)
        );
        assert_eq!(base14_for("MyriadPro"), None);
    }
}
