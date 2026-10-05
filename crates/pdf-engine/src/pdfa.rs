//! Conversion to PDF/A-2b (ISO 19005-2, level B: reliable visual reproduction).
//!
//! What this does **and does not** do, so nobody has to guess:
//!
//! * It repairs what can be repaired without changing how the pages look: file identifier,
//!   XMP metadata (`pdfaid:part=2`, `conformance=B`, matching the Info dictionary), an sRGB
//!   output intent with an embedded ICC profile, PDF 1.7 header, annotation flags, forbidden
//!   actions/JavaScript/attachments/XFA, `/CIDToGIDMap`, stream keys such as `/F`.
//! * It **refuses** (with the reason) when conformance would need something it cannot supply
//!   without altering the document: fonts that are not embedded, CMYK colours without a CMYK
//!   output intent, annotations without an appearance, encrypted files.
//! * It cannot *prove* conformance. The tests validate its output with veraPDF (an independent
//!   validator) on a set of documents; the application itself carries no validator, so the
//!   result is labelled "prepared for PDF/A-2b", not "certified".
//!
//! Digital signatures are invalidated (the file is rewritten) and are reported as such.

use crate::annot::now_pdf_date;
use crate::content::{self, Operand};
use crate::error::{EngineError, Result};
use crate::icc;
use crate::objutil;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat, dictionary};
use std::collections::BTreeSet;

const MAX_STREAM_BYTES: usize = 64 * 1024 * 1024;

/// What the analysis found / the conversion did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PdfaReport {
    /// Things that were (or will be) repaired.
    pub fixes: Vec<String>,
    /// Things that were (or will be) removed because PDF/A forbids them.
    pub removed: Vec<String>,
    /// Reasons the document cannot be converted without changing it. Non-empty ⇒ no output.
    pub blockers: Vec<String>,
    /// Things to know that do not stop the conversion.
    pub notes: Vec<String>,
}

impl PdfaReport {
    /// Whether the conversion can go ahead.
    pub fn can_convert(&self) -> bool {
        self.blockers.is_empty()
    }
}

fn err(m: impl Into<String>) -> EngineError {
    EngineError::Save(m.into())
}

fn name_of(o: Option<&Object>) -> Option<&[u8]> {
    o.and_then(|o| o.as_name().ok())
}

fn catalog_id(doc: &Document) -> Option<ObjectId> {
    doc.trailer.get(b"Root").ok()?.as_reference().ok()
}

fn dict_of<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Dictionary> {
    objutil::deref(doc, o)?.as_dict().ok()
}

fn strip_subset(name: &[u8]) -> String {
    let s = String::from_utf8_lossy(name).into_owned();
    match s.split_once('+') {
        Some((p, rest)) if p.len() == 6 && p.chars().all(|c| c.is_ascii_uppercase()) => {
            rest.to_string()
        }
        _ => s,
    }
}

// ---- analysis -------------------------------------------------------------------------------

fn descriptor_embedded(doc: &Document, fd: Option<&Object>) -> bool {
    let Some(d) = fd.and_then(|o| dict_of(doc, o)) else {
        return false;
    };
    [&b"FontFile"[..], b"FontFile2", b"FontFile3"]
        .iter()
        .any(|k| d.has(k))
}

fn unembedded_fonts(doc: &Document) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for obj in doc.objects.values() {
        let Object::Dictionary(d) = obj else { continue };
        if name_of(d.get(b"Type").ok()) != Some(b"Font") {
            continue;
        }
        let sub = name_of(d.get(b"Subtype").ok()).unwrap_or(b"");
        let base = d
            .get(b"BaseFont")
            .ok()
            .and_then(|o| o.as_name().ok())
            .map(strip_subset)
            .unwrap_or_else(|| "(unnamed font)".into());
        match sub {
            b"Type1" | b"MMType1" | b"TrueType" | b"CIDFontType0" | b"CIDFontType2"
                if !descriptor_embedded(doc, d.get(b"FontDescriptor").ok()) =>
            {
                out.insert(base);
            }
            _ => {}
        }
    }
    out
}

#[derive(Default)]
struct ColourUse {
    rgb: bool,
    gray: bool,
    cmyk: bool,
}

fn note_space(name: &[u8], u: &mut ColourUse) {
    match name {
        b"DeviceRGB" | b"RGB" => u.rgb = true,
        b"DeviceGray" | b"G" => u.gray = true,
        b"DeviceCMYK" | b"CMYK" => u.cmyk = true,
        _ => {}
    }
}

fn scan_colours(doc: &Document) -> ColourUse {
    let mut u = ColourUse::default();
    // Images and colour-space resources anywhere in the file.
    for obj in doc.objects.values() {
        let d = match obj {
            Object::Stream(s) => &s.dict,
            Object::Dictionary(d) => d,
            _ => continue,
        };
        if let Some(cs) = d
            .get(b"ColorSpace")
            .ok()
            .and_then(|o| objutil::deref(doc, o))
        {
            match cs {
                Object::Name(n) => note_space(n, &mut u),
                Object::Dictionary(m) => {
                    for (_, v) in m.iter() {
                        if let Some(Object::Name(n)) = objutil::deref(doc, v) {
                            note_space(n, &mut u);
                        }
                    }
                }
                Object::Array(a) => {
                    // [/Indexed /DeviceRGB ...], [/Separation name /DeviceCMYK ...] etc.
                    for v in a {
                        if let Some(Object::Name(n)) = objutil::deref(doc, v) {
                            note_space(n, &mut u);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    // Colour operators in page content and forms.
    let pages: Vec<ObjectId> = doc.get_pages().values().copied().collect();
    for page in pages {
        let mut data = Vec::new();
        for id in crate::pagecontent::content_stream_ids(doc, page) {
            if let Some(Object::Stream(s)) = doc.objects.get(&id)
                && let Ok(d) = s.decompressed_content_with_limit(MAX_STREAM_BYTES)
            {
                data.extend_from_slice(&d);
                data.push(b'\n');
            }
        }
        scan_ops(&data, &mut u);
    }
    for obj in doc.objects.values() {
        if let Object::Stream(s) = obj
            && name_of(s.dict.get(b"Subtype").ok()) == Some(b"Form")
            && let Ok(d) = s.decompressed_content_with_limit(MAX_STREAM_BYTES)
        {
            scan_ops(&d, &mut u);
        }
    }
    u
}

fn scan_ops(data: &[u8], u: &mut ColourUse) {
    let Ok(ops) = content::scan(data) else { return };
    for op in &ops {
        match op.name.as_slice() {
            b"rg" | b"RG" => u.rgb = true,
            b"g" | b"G" => u.gray = true,
            b"k" | b"K" => u.cmyk = true,
            b"cs" | b"CS" => {
                if let Some(Operand::Name(n)) = op.operands.first() {
                    note_space(n, u);
                }
            }
            _ => {}
        }
    }
}

/// The destination-profile component count of an existing output intent, if any.
fn output_intent_components(doc: &Document) -> Option<Vec<i64>> {
    let cat = doc.get_dictionary(catalog_id(doc)?).ok()?;
    let arr = cat
        .get(b"OutputIntents")
        .ok()
        .and_then(|o| objutil::deref(doc, o))
        .and_then(|o| o.as_array().ok())?;
    let mut v = Vec::new();
    for o in arr {
        let n = dict_of(doc, o)
            .and_then(|d| d.get(b"DestOutputProfile").ok())
            .and_then(|p| objutil::deref(doc, p))
            .and_then(|p| match p {
                Object::Stream(s) => objutil::dict_num(doc, &s.dict, b"N"),
                _ => None,
            })
            .map_or(0, |n| n as i64);
        v.push(n);
    }
    Some(v)
}

fn all_annotations(doc: &Document) -> Vec<(ObjectId, ObjectId)> {
    let mut out = Vec::new();
    for page in doc.get_pages().values() {
        let Ok(p) = doc.get_dictionary(*page) else {
            continue;
        };
        let Some(arr) = p
            .get(b"Annots")
            .ok()
            .and_then(|o| objutil::deref(doc, o))
            .and_then(|o| o.as_array().ok())
        else {
            continue;
        };
        for a in arr {
            if let Ok(id) = a.as_reference() {
                out.push((*page, id));
            }
        }
    }
    out
}

fn has_appearance(doc: &Document, d: &Dictionary) -> bool {
    d.get(b"AP")
        .ok()
        .and_then(|o| dict_of(doc, o))
        .is_some_and(|ap| ap.has(b"N"))
}

/// Inspect a document and say what a conversion would do and whether it can go ahead.
pub fn analyze(input: &[u8]) -> Result<PdfaReport> {
    let doc = load(input)?;
    Ok(plan(&doc))
}

fn load(input: &[u8]) -> Result<Document> {
    let doc =
        Document::load_mem(input).map_err(|e| err(format!("cannot read the document: {e}")))?;
    if doc.is_encrypted() {
        return Err(EngineError::Unsupported(
            "encrypted documents cannot be converted to PDF/A".into(),
        ));
    }
    Ok(doc)
}

fn plan(doc: &Document) -> PdfaReport {
    let mut r = PdfaReport::default();
    // ---- blockers
    let fonts = unembedded_fonts(doc);
    if !fonts.is_empty() {
        r.blockers.push(format!(
            "These fonts are not embedded in the document: {}. PDF/A requires every font to be embedded, and replacing a font would change how the pages look. Re-create the PDF from its source with fonts embedded.",
            fonts.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    let col = scan_colours(doc);
    let oi = output_intent_components(doc);
    match &oi {
        None => {
            if col.cmyk {
                r.blockers.push(
                    "The document uses CMYK colours and has no CMYK output intent. BergPDF only adds an sRGB profile and cannot convert CMYK colours without changing them.".into(),
                );
            }
        }
        Some(v) => {
            if col.rgb && !v.contains(&3) {
                r.blockers.push(
                    "The document uses RGB colours but its output intent is not an RGB profile."
                        .into(),
                );
            }
            if col.cmyk && !v.contains(&4) {
                r.blockers.push(
                    "The document uses CMYK colours but its output intent is not a CMYK profile."
                        .into(),
                );
            }
        }
    }
    for (_, id) in all_annotations(doc) {
        let Ok(d) = doc.get_dictionary(id) else {
            continue;
        };
        let sub = name_of(d.get(b"Subtype").ok()).unwrap_or(b"");
        if matches!(sub, b"Popup" | b"Link") || has_appearance(doc, d) {
            continue;
        }
        if sub == b"FileAttachment" {
            continue; // removed below
        }
        // Zero-size rectangles are exempt in the standard.
        r.blockers.push(format!(
            "A {} annotation has no appearance stream, so its look is not defined.",
            String::from_utf8_lossy(sub)
        ));
        break;
    }
    // ---- fixes and removals (descriptive; `apply` does them)
    if doc.version.as_str() < "1.7" {
        r.fixes
            .push(format!("PDF version raised from {} to 1.7", doc.version));
    }
    if !doc.trailer.has(b"ID") {
        r.fixes.push("File identifier (/ID) added".into());
    }
    let cat = catalog_id(doc).and_then(|c| doc.get_dictionary(c).ok());
    let had_xmp = cat.is_some_and(|c| c.has(b"Metadata"));
    r.fixes.push(if had_xmp {
        "XMP metadata rewritten with the PDF/A-2b identification".into()
    } else {
        "XMP metadata added with the PDF/A-2b identification".into()
    });
    if oi.is_none() {
        r.fixes
            .push("sRGB output intent with embedded colour profile added".into());
    }
    if oi.is_none() && (col.rgb || col.gray) {
        r.notes.push(
            "Device colours are kept as they are and interpreted as sRGB by the output intent."
                .into(),
        );
    }
    let (acts, files, others) = forbidden_inventory(doc);
    if acts > 0 {
        r.removed
            .push(format!("{acts} script, launch or other action(s)"));
    }
    if files > 0 {
        r.removed
            .push(format!("{files} embedded file(s) / attachment(s)"));
    }
    for o in others {
        r.removed.push(o);
    }
    r.notes.push(
        "BergPDF cannot validate PDF/A itself. For archival or legal use, check the result with a validator such as veraPDF.".into(),
    );
    r
}

/// Count forbidden features: (actions, embedded files, other removals).
fn forbidden_inventory(doc: &Document) -> (usize, usize, Vec<String>) {
    let mut actions = 0;
    let mut files = 0;
    let mut other = Vec::new();
    for obj in doc.objects.values() {
        let d = match obj {
            Object::Dictionary(d) => d,
            Object::Stream(s) => &s.dict,
            _ => continue,
        };
        if d.has(b"AA") {
            actions += 1;
        }
        if let Some(a) = d.get(b"A").ok().and_then(|o| dict_of(doc, o))
            && action_forbidden(a)
        {
            actions += 1;
        }
        if d.has(b"EF") {
            files += 1;
        }
        if name_of(d.get(b"Subtype").ok()) == Some(b"FileAttachment") {
            files += 1;
        }
    }
    if let Some(cat) = catalog_id(doc).and_then(|c| doc.get_dictionary(c).ok()) {
        if cat.has(b"OpenAction") {
            actions += 1;
        }
        if cat.has(b"AF") || cat.has(b"Collection") {
            files += 1;
        }
        if let Some(af) = cat.get(b"AcroForm").ok().and_then(|o| dict_of(doc, o))
            && af.has(b"XFA")
        {
            other.push("XFA form data".to_string());
        }
    }
    (actions, files, other)
}

fn action_forbidden(a: &Dictionary) -> bool {
    match name_of(a.get(b"S").ok()) {
        Some(b"GoTo" | b"GoToR" | b"URI" | b"SubmitForm" | b"Thread") => false,
        Some(b"Named") => !matches!(
            name_of(a.get(b"N").ok()),
            Some(b"NextPage" | b"PrevPage" | b"FirstPage" | b"LastPage")
        ),
        _ => true,
    }
}

// ---- conversion -----------------------------------------------------------------------------

fn xml_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' => {}
            c => o.push(c),
        }
    }
    o
}

/// `D:YYYYMMDDHHmmSS+HH'mm'` → ISO 8601 (`YYYY-MM-DDTHH:mm:SS+HH:mm`).
pub fn pdf_date_to_iso(s: &str) -> Option<String> {
    let s = s.strip_prefix("D:").unwrap_or(s);
    let b = s.as_bytes();
    let digits = |from: usize, n: usize| -> Option<&str> {
        let t = s.get(from..from + n)?;
        t.bytes().all(|c| c.is_ascii_digit()).then_some(t)
    };
    let y = digits(0, 4)?;
    let mo = digits(4, 2).unwrap_or("01");
    let d = digits(6, 2).unwrap_or("01");
    let h = digits(8, 2).unwrap_or("00");
    let mi = digits(10, 2).unwrap_or("00");
    let se = digits(12, 2).unwrap_or("00");
    let tz = match b.get(14) {
        Some(b'+') | Some(b'-') => {
            let sign = b[14] as char;
            let th = digits(15, 2).unwrap_or("00");
            let tm = s.get(18..).map(|r| r.trim_matches('\'')).and_then(|r| {
                (r.len() >= 2 && r.bytes().take(2).all(|c| c.is_ascii_digit())).then(|| &r[..2])
            });
            format!("{sign}{th}:{}", tm.unwrap_or("00"))
        }
        _ => "Z".to_string(),
    };
    Some(format!("{y}-{mo}-{d}T{h}:{mi}:{se}{tz}"))
}

struct InfoStrings {
    title: String,
    author: String,
    subject: String,
    keywords: String,
    creator: String,
    producer: String,
    created: String,
    modified: String,
}

fn build_xmp(i: &InfoStrings) -> Vec<u8> {
    let mut x = String::new();
    x.push_str("<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n");
    x.push_str("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n");
    x.push_str("<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n");
    x.push_str("<rdf:Description rdf:about=\"\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\">\n<pdfaid:part>2</pdfaid:part>\n<pdfaid:conformance>B</pdfaid:conformance>\n</rdf:Description>\n");
    x.push_str("<rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n<dc:format>application/pdf</dc:format>\n");
    if !i.title.is_empty() {
        x.push_str(&format!(
            "<dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:title>\n",
            xml_escape(&i.title)
        ));
    }
    if !i.author.is_empty() {
        x.push_str(&format!(
            "<dc:creator><rdf:Seq><rdf:li>{}</rdf:li></rdf:Seq></dc:creator>\n",
            xml_escape(&i.author)
        ));
    }
    if !i.subject.is_empty() {
        x.push_str(&format!(
            "<dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:description>\n",
            xml_escape(&i.subject)
        ));
    }
    x.push_str("</rdf:Description>\n");
    x.push_str("<rdf:Description rdf:about=\"\" xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\">\n");
    x.push_str(&format!(
        "<pdf:Producer>{}</pdf:Producer>\n",
        xml_escape(&i.producer)
    ));
    if !i.keywords.is_empty() {
        x.push_str(&format!(
            "<pdf:Keywords>{}</pdf:Keywords>\n",
            xml_escape(&i.keywords)
        ));
    }
    x.push_str("</rdf:Description>\n");
    x.push_str("<rdf:Description rdf:about=\"\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\">\n");
    x.push_str(&format!(
        "<xmp:CreatorTool>{}</xmp:CreatorTool>\n<xmp:CreateDate>{}</xmp:CreateDate>\n<xmp:ModifyDate>{}</xmp:ModifyDate>\n<xmp:MetadataDate>{}</xmp:MetadataDate>\n",
        xml_escape(&i.creator),
        i.created,
        i.modified,
        i.modified
    ));
    x.push_str("</rdf:Description>\n</rdf:RDF>\n</x:xmpmeta>\n");
    // Padding is customary so a writer can update the packet in place.
    for _ in 0..8 {
        x.push_str(&" ".repeat(100));
        x.push('\n');
    }
    x.push_str("<?xpacket end=\"w\"?>");
    x.into_bytes()
}

fn info_text(doc: &Document, d: &Dictionary, k: &[u8]) -> String {
    d.get(k)
        .ok()
        .and_then(|o| objutil::deref(doc, o))
        .and_then(|o| o.as_str().ok())
        .map(objutil::decode_text_string)
        .unwrap_or_default()
}

/// Convert `input` to PDF/A-2b. Returns the new bytes and the report, or an error naming the
/// blockers when the document cannot be converted without changing its content.
pub fn convert_bytes(input: &[u8]) -> Result<(Vec<u8>, PdfaReport)> {
    let mut doc = load(input)?;
    let pages_before = doc.get_pages().len();
    let report = plan(&doc);
    if !report.can_convert() {
        return Err(EngineError::Unsupported(report.blockers.join("\n")));
    }
    let mut report = report;

    // 1. Version.
    if doc.version.as_str() < "1.7" {
        doc.version = "1.7".to_string();
    }

    // 2. Strip forbidden things.
    sanitize(&mut doc);

    // 3. CIDToGIDMap on embedded TrueType CID fonts.
    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    for id in ids {
        let embedded = match doc.objects.get(&id) {
            Some(Object::Dictionary(d))
                if name_of(d.get(b"Subtype").ok()) == Some(b"CIDFontType2")
                    && !d.has(b"CIDToGIDMap") =>
            {
                descriptor_embedded(&doc, d.get(b"FontDescriptor").ok())
            }
            _ => false,
        };
        if embedded && let Some(Object::Dictionary(d)) = doc.objects.get_mut(&id) {
            d.set("CIDToGIDMap", Object::Name(b"Identity".to_vec()));
            report
                .fixes
                .push("CIDToGIDMap /Identity made explicit on an embedded CID font".into());
        }
    }

    // 4. Annotation flags.
    fix_annotation_flags(&mut doc);

    // 5. Info + XMP + dates.
    let now = now_pdf_date();
    let info_id = doc
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|o| o.as_reference().ok());
    let info_snapshot = info_id
        .and_then(|id| doc.get_dictionary(id).ok())
        .cloned()
        .unwrap_or_default();
    let mut strings = InfoStrings {
        title: info_text(&doc, &info_snapshot, b"Title"),
        author: info_text(&doc, &info_snapshot, b"Author"),
        subject: info_text(&doc, &info_snapshot, b"Subject"),
        keywords: info_text(&doc, &info_snapshot, b"Keywords"),
        creator: info_text(&doc, &info_snapshot, b"Creator"),
        producer: info_text(&doc, &info_snapshot, b"Producer"),
        created: info_text(&doc, &info_snapshot, b"CreationDate"),
        modified: now.clone(),
    };
    if strings.creator.is_empty() {
        strings.creator = "BergPDF".into();
    }
    if strings.producer.is_empty() {
        strings.producer = "BergPDF".into();
    }
    let created_iso = pdf_date_to_iso(&strings.created);
    let created_pdf = if created_iso.is_some() {
        strings.created.clone()
    } else {
        now.clone()
    };
    let iso_created = created_iso
        .or_else(|| pdf_date_to_iso(&now))
        .unwrap_or_default();
    let iso_modified = pdf_date_to_iso(&now).unwrap_or_default();
    let xmp = build_xmp(&InfoStrings {
        created: iso_created,
        modified: iso_modified,
        ..strings
    });
    // Info dictionary: same values, fresh ModDate.
    let mut info = info_snapshot;
    info.set(
        "Creator",
        pdf_text(&info_text(&doc, &info, b"Creator"), "BergPDF"),
    );
    info.set(
        "Producer",
        pdf_text(&info_text(&doc, &info, b"Producer"), "BergPDF"),
    );
    info.set(
        "CreationDate",
        Object::String(created_pdf.into_bytes(), StringFormat::Literal),
    );
    info.set(
        "ModDate",
        Object::String(now.into_bytes(), StringFormat::Literal),
    );
    let info_id = match info_id {
        Some(id) => {
            doc.objects.insert(id, Object::Dictionary(info));
            id
        }
        None => doc.add_object(Object::Dictionary(info)),
    };
    doc.trailer.set("Info", Object::Reference(info_id));
    let mut meta = Stream::new(
        dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
        xmp,
    );
    meta.allows_compression = false;
    let meta_id = doc.add_object(Object::Stream(meta));

    // 6. Catalog: metadata + output intent.
    let cat_id = catalog_id(&doc).ok_or_else(|| err("the document has no catalog"))?;
    let need_oi = output_intent_components(&doc).is_none();
    let oi_ref = if need_oi {
        let mut prof = Stream::new(
            dictionary! { "N" => 3i64, "Alternate" => "DeviceRGB" },
            icc::srgb_profile(),
        );
        prof.allows_compression = false;
        let prof_id = doc.add_object(Object::Stream(prof));
        let oi = dictionary! {
            "Type" => "OutputIntent",
            "S" => "GTS_PDFA1",
            "OutputConditionIdentifier" => Object::string_literal("sRGB"),
            "Info" => Object::string_literal("sRGB IEC61966-2.1"),
            "RegistryName" => Object::string_literal("http://www.color.org"),
            "DestOutputProfile" => Object::Reference(prof_id),
        };
        Some(doc.add_object(Object::Dictionary(oi)))
    } else {
        None
    };
    if let Some(Object::Dictionary(cat)) = doc.objects.get_mut(&cat_id) {
        cat.set("Metadata", Object::Reference(meta_id));
        if let Some(oi) = oi_ref {
            cat.set("OutputIntents", Object::Array(vec![Object::Reference(oi)]));
        }
    }

    // 7. File identifier.
    if !doc.trailer.has(b"ID") {
        let digest = pdf_sign::sha256(&[input]);
        let seed: String = digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
            + &now_pdf_date()
            + &input.len().to_string();
        let id = pdf_sign::sha256(&[seed.as_bytes()]);
        let id16 = id[..16].to_vec();
        doc.trailer.set(
            "ID",
            Object::Array(vec![
                Object::String(id16.clone(), StringFormat::Hexadecimal),
                Object::String(id16, StringFormat::Hexadecimal),
            ]),
        );
    }

    // 8. Streams: compress the ones that are not metadata/profiles (those opted out above).
    doc.compress();
    doc.prune_objects();
    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| err(format!("writing the PDF/A file failed: {e}")))?;

    // 9. Validate by reading it back and re-running our own checks.
    let check = load(&out)?;
    if check.get_pages().len() != pages_before {
        return Err(err(format!(
            "the converted file has {} pages, expected {pages_before}",
            check.get_pages().len()
        )));
    }
    let again = plan(&check);
    if !again.can_convert() {
        return Err(err(format!(
            "the converted file still has problems: {}",
            again.blockers.join("; ")
        )));
    }
    Ok((out, report))
}

fn pdf_text(value: &str, fallback: &str) -> Object {
    let v = if value.is_empty() { fallback } else { value };
    objutil::text_obj(v)
}

fn fix_annotation_flags(doc: &mut Document) {
    for (_, id) in all_annotations(doc) {
        if let Some(Object::Dictionary(d)) = doc.objects.get_mut(&id) {
            let f = d.get(b"F").ok().and_then(|o| o.as_i64().ok()).unwrap_or(0);
            // Print on; Invisible (1), Hidden (2), NoView (32) off.
            let nf = (f | 4) & !(1 | 2 | 32);
            if nf != f || !d.has(b"F") {
                d.set("F", Object::Integer(nf));
            }
        }
    }
}

/// Remove actions, scripts, attachments and other things PDF/A-2 forbids.
fn sanitize(doc: &mut Document) {
    // Annotations of type FileAttachment are dropped from the page lists first.
    let pages: Vec<ObjectId> = doc.get_pages().values().copied().collect();
    let attachments: BTreeSet<ObjectId> = all_annotations(doc)
        .into_iter()
        .filter(|(_, id)| {
            doc.get_dictionary(*id)
                .ok()
                .is_some_and(|d| name_of(d.get(b"Subtype").ok()) == Some(b"FileAttachment"))
        })
        .map(|(_, id)| id)
        .collect();
    if !attachments.is_empty() {
        for page in pages {
            let arr_id = doc
                .get_dictionary(page)
                .ok()
                .and_then(|p| p.get(b"Annots").ok())
                .and_then(|o| o.as_reference().ok());
            let filter = |a: &mut Vec<Object>| {
                a.retain(|o| !o.as_reference().is_ok_and(|r| attachments.contains(&r)));
            };
            if let Some(id) = arr_id {
                if let Some(Object::Array(a)) = doc.objects.get_mut(&id) {
                    filter(a);
                }
            } else if let Some(Object::Dictionary(p)) = doc.objects.get_mut(&page)
                && let Ok(Object::Array(a)) = p.get_mut(b"Annots")
            {
                filter(a);
            }
        }
    }
    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    for id in &ids {
        // Decide with an immutable view of the action targets first.
        let drop_a = match doc.objects.get(id) {
            Some(Object::Dictionary(d)) => d
                .get(b"A")
                .ok()
                .and_then(|o| dict_of(doc, o))
                .is_some_and(action_forbidden),
            Some(Object::Stream(s)) => s
                .dict
                .get(b"A")
                .ok()
                .and_then(|o| dict_of(doc, o))
                .is_some_and(action_forbidden),
            _ => false,
        };
        let dict = match doc.objects.get_mut(id) {
            Some(Object::Dictionary(d)) => d,
            Some(Object::Stream(s)) => &mut s.dict,
            _ => continue,
        };
        if drop_a {
            dict.remove(b"A");
        }
        dict.remove(b"AA");
        dict.remove(b"EF");
        dict.remove(b"TR");
        if dict.get(b"TR2").ok().and_then(|o| o.as_name().ok()) != Some(b"Default") {
            dict.remove(b"TR2");
        }
        // External-file keys of stream dictionaries.
        if dict.has(b"Length") {
            dict.remove(b"FFilter");
            dict.remove(b"FDecodeParms");
        }
        // Optional-content configurations must not carry an auto-state array.
        if dict.has(b"AS")
            && (dict.has(b"Order") || dict.has(b"ON") || dict.has(b"OFF") || dict.has(b"BaseState"))
        {
            dict.remove(b"AS");
        }
        // Widgets and others that reference a file specification keep working without /EF.
    }
    // Streams with /F: external file, forbidden.
    for obj in doc.objects.values_mut() {
        if let Object::Stream(s) = obj {
            s.dict.remove(b"F");
        }
    }
    // LZW is forbidden: re-encode as Flate.
    for obj in doc.objects.values_mut() {
        if let Object::Stream(s) = obj {
            let lzw = match s.dict.get(b"Filter") {
                Ok(Object::Name(n)) => n == b"LZWDecode",
                Ok(Object::Array(a)) => a.iter().any(|o| name_of(Some(o)) == Some(b"LZWDecode")),
                _ => false,
            };
            if lzw {
                let _ = s.decompress();
            }
        }
    }
    // Catalog.
    if let Some(cat_id) = catalog_id(doc) {
        let acro_id = doc
            .get_dictionary(cat_id)
            .ok()
            .and_then(|c| c.get(b"AcroForm").ok())
            .and_then(|o| o.as_reference().ok());
        if let Some(Object::Dictionary(cat)) = doc.objects.get_mut(&cat_id) {
            cat.remove(b"OpenAction");
            cat.remove(b"AF");
            cat.remove(b"Collection");
            cat.remove(b"Requirements");
            if let Ok(Object::Dictionary(af)) = cat.get_mut(b"AcroForm") {
                af.remove(b"XFA");
                af.remove(b"NeedAppearances");
            }
        }
        if let Some(id) = acro_id
            && let Some(Object::Dictionary(af)) = doc.objects.get_mut(&id)
        {
            af.remove(b"XFA");
            af.remove(b"NeedAppearances");
        }
        // Names tree: scripts and attachments.
        let names_id = doc
            .get_dictionary(cat_id)
            .ok()
            .and_then(|c| c.get(b"Names").ok())
            .and_then(|o| o.as_reference().ok());
        if let Some(Object::Dictionary(cat)) = doc.objects.get_mut(&cat_id)
            && let Ok(Object::Dictionary(n)) = cat.get_mut(b"Names")
        {
            n.remove(b"JavaScript");
            n.remove(b"EmbeddedFiles");
        }
        if let Some(id) = names_id
            && let Some(Object::Dictionary(n)) = doc.objects.get_mut(&id)
        {
            n.remove(b"JavaScript");
            n.remove(b"EmbeddedFiles");
        }
    }
}
