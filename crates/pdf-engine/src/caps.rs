//! Document capability and risk detection.
//!
//! The UI enables commands based on what was actually detected here, never on assumptions.

use lopdf::{Document, Object};

/// A reason an edit/save path is limited, shown verbatim to the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limitation(pub String);

/// What we know about a document after opening it.
#[derive(Clone, Debug, Default)]
pub struct Capabilities {
    /// Header / catalog PDF version.
    pub version: String,
    /// `/Encrypt` present (we do not edit encrypted files in v0.1).
    pub encrypted: bool,
    /// Digital signature fields with values exist.
    pub has_signatures: bool,
    /// Document certification (`/DocMDP`) present.
    pub certified: bool,
    /// XFA form data present (unsupported).
    pub has_xfa: bool,
    /// An AcroForm exists.
    pub has_acroform: bool,
    /// Document or catalog-level JavaScript present (never executed).
    pub has_javascript: bool,
    /// Embedded files (attachments) present (never opened automatically).
    pub has_attachments: bool,
    /// Optional content (layers) present.
    pub has_layers: bool,
    /// Newer than PDF 1.7 (e.g. 2.0): compatibility is best-effort.
    pub newer_than_1_7: bool,
    /// The file's cross-reference data is valid for appending an incremental update.
    pub incremental_ok: bool,
    /// Whether editing commands are allowed at all.
    pub can_edit: bool,
    /// Why editing is blocked (empty when `can_edit`).
    pub edit_blockers: Vec<Limitation>,
    /// Non-blocking cautions shown before saving.
    pub warnings: Vec<Limitation>,
}

impl Capabilities {
    /// Run all detectors.
    pub fn detect(doc: &Document, bytes: &[u8], was_encrypted: bool, incremental_ok: bool) -> Self {
        let mut c = Capabilities {
            version: doc.version.clone(),
            encrypted: was_encrypted || doc.trailer.has(b"Encrypt"),
            incremental_ok,
            ..Default::default()
        };
        let catalog = doc
            .trailer
            .get(b"Root")
            .and_then(Object::as_reference)
            .ok()
            .and_then(|id| doc.get_dictionary(id).ok());
        if let Some(cat) = catalog {
            if let Some(v) = crate::objutil::dict_name(doc, cat, b"Version")
                && let Ok(v) = std::str::from_utf8(v)
                && v > c.version.as_str()
            {
                c.version = v.to_string();
            }
            c.has_layers = cat.has(b"OCProperties");
            if cat.has(b"OpenAction") || cat.has(b"AA") {
                c.has_javascript |= action_has_js(doc, cat.get(b"OpenAction").ok())
                    || action_has_js(doc, cat.get(b"AA").ok());
            }
            if let Some(names) = crate::objutil::dict_dict(doc, cat, b"Names") {
                c.has_javascript |= names.has(b"JavaScript");
                c.has_attachments |= names.has(b"EmbeddedFiles");
            }
            if let Some(af) = crate::objutil::dict_dict(doc, cat, b"AcroForm") {
                c.has_acroform = true;
                c.has_xfa = af.has(b"XFA");
                if let Ok(flags) = af.get(b"SigFlags").and_then(Object::as_i64) {
                    c.has_signatures |= flags & 1 != 0;
                }
                if let Some(fields) = crate::objutil::dict_array(doc, af, b"Fields") {
                    c.has_signatures |= fields_have_signature(doc, fields, 0);
                }
            }
            if let Some(perms) = crate::objutil::dict_dict(doc, cat, b"Perms") {
                c.certified = perms.has(b"DocMDP");
                c.has_signatures |= perms.has(b"DocMDP") || perms.has(b"UR3");
            }
        }
        // Header version, e.g. "2.0".
        c.newer_than_1_7 = c.version.as_str() > "1.7";
        c.has_signatures |= bytes_contain(bytes, b"/ByteRange") && bytes_contain(bytes, b"/Sig");

        if c.encrypted {
            c.edit_blockers.push(Limitation(
                "This document is encrypted. BergPDF opens it read-only; it never strips \
                 or re-writes encryption."
                    .into(),
            ));
        }
        if c.has_xfa {
            c.warnings.push(Limitation(
                "XFA form content detected. XFA forms are not supported; the AcroForm fallback \
                 (if any) is shown."
                    .into(),
            ));
        }
        if c.has_signatures {
            c.warnings.push(Limitation(
                "The document contains digital signatures. Saving appends a new revision and \
                 keeps signed bytes intact, but the signature will report that the document \
                 was modified after signing. Signature validity is not verified by BergPDF."
                    .into(),
            ));
        }
        if c.newer_than_1_7 {
            c.warnings.push(Limitation(
                "PDF 2.0 or newer constructs may not be fully understood; unknown structures \
                 are preserved but not editable."
                    .into(),
            ));
        }
        if !incremental_ok && !c.encrypted {
            c.warnings.push(Limitation(
                "The file's cross-reference data needed repair. Saving rewrites the whole file."
                    .into(),
            ));
        }
        c.can_edit = c.edit_blockers.is_empty();
        c
    }

    /// One-line explanation used in error messages when editing is blocked.
    pub fn edit_blocker_summary(&self) -> String {
        self.edit_blockers
            .iter()
            .map(|l| l.0.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn bytes_contain(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn action_has_js(doc: &Document, obj: Option<&Object>) -> bool {
    let Some(obj) = obj.and_then(|o| crate::objutil::deref(doc, o)) else {
        return false;
    };
    match obj {
        Object::Dictionary(d) => {
            crate::objutil::dict_name(doc, d, b"S") == Some(b"JavaScript")
                || d.iter().any(|(_, v)| {
                    matches!(v, Object::Reference(_) | Object::Dictionary(_))
                        && action_has_js_shallow(doc, v)
                })
        }
        _ => false,
    }
}

fn action_has_js_shallow(doc: &Document, obj: &Object) -> bool {
    match crate::objutil::deref(doc, obj) {
        Some(Object::Dictionary(d)) => {
            crate::objutil::dict_name(doc, d, b"S") == Some(b"JavaScript")
        }
        _ => false,
    }
}

fn fields_have_signature(doc: &Document, fields: &[Object], depth: usize) -> bool {
    if depth > 16 {
        return false;
    }
    fields.iter().any(|f| {
        let Some(Object::Dictionary(d)) = crate::objutil::deref(doc, f) else {
            return false;
        };
        if crate::objutil::dict_name(doc, d, b"FT") == Some(b"Sig") && d.has(b"V") {
            return true;
        }
        crate::objutil::dict_array(doc, d, b"Kids")
            .is_some_and(|k| fields_have_signature(doc, k, depth + 1))
    })
}
