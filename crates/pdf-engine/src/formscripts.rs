//! The JavaScript of a form, as data: field scripts (`/AA` keystroke, format, validate, calculate),
//! button actions (`/A`), the calculation order (`/CO`) and the document-level scripts
//! (`/Names /JavaScript`).
//!
//! This module only *reads and writes* scripts. Running them is the job of the separate `pdf-js`
//! crate, which the application calls only when the person allowed it for the document (D-040).
//! Nothing here executes anything.

use crate::doc::{PdfDocument, Tx};
use crate::error::{EngineError, Result};
use crate::forms::{self, FieldKind};
use crate::objutil::{self, reference, text_obj};
use lopdf::{Dictionary, Document, Object, ObjectId, dictionary};
use std::collections::BTreeSet;

/// Largest script read or written, in bytes.
pub const MAX_SCRIPT: usize = 256 * 1024;
/// Most document-level scripts read.
pub const MAX_DOC_SCRIPTS: usize = 500;

/// The scripts of one field. Empty = none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldScripts {
    /// `/AA /C`: computes the field's value from others.
    pub calculate: String,
    /// `/AA /V`: accepts or rejects a new value.
    pub validate: String,
    /// `/AA /F`: how the value is shown.
    pub format: String,
    /// `/AA /K`: checks what is typed (run on commit).
    pub keystroke: String,
    /// `/A` (mouse up) of a button.
    pub action: String,
}

impl FieldScripts {
    /// Whether any script is set.
    pub fn is_empty(&self) -> bool {
        self.calculate.is_empty()
            && self.validate.is_empty()
            && self.format.is_empty()
            && self.keystroke.is_empty()
            && self.action.is_empty()
    }
}

/// A JavaScript action dictionary.
pub fn js_action(code: &str) -> Object {
    Object::Dictionary(dictionary! {
        "Type" => "Action",
        "S" => "JavaScript",
        "JS" => text_obj(code),
    })
}

/// The code of a JavaScript action (string or stream), if `obj` is one.
pub fn read_js(doc: &Document, obj: &Object) -> Option<String> {
    let d = objutil::deref(doc, obj)?.as_dict().ok()?;
    if objutil::dict_name(doc, d, b"S") != Some(b"JavaScript") {
        return None;
    }
    let js = d.get(b"JS").ok()?;
    let text = match objutil::deref(doc, js)? {
        Object::String(s, _) => objutil::decode_text_string(s),
        Object::Stream(s) => {
            let bytes = s
                .decompressed_content()
                .unwrap_or_else(|_| s.content.clone());
            if bytes.len() > MAX_SCRIPT {
                return None;
            }
            objutil::decode_text_string(&bytes)
        }
        _ => return None,
    };
    (text.len() <= MAX_SCRIPT).then_some(text)
}

fn aa_script(doc: &Document, d: &Dictionary, key: &[u8]) -> String {
    objutil::dict_dict(doc, d, b"AA")
        .and_then(|aa| aa.get(key).ok())
        .and_then(|o| read_js(doc, o))
        .unwrap_or_default()
}

/// The scripts of a field (and of its first widget, for merged or button fields).
pub fn field_scripts(doc: &Document, field: ObjectId) -> FieldScripts {
    let Ok(fd) = doc.get_dictionary(field) else {
        return FieldScripts::default();
    };
    let widget = forms::widgets_of(doc, field)
        .first()
        .and_then(|w| doc.get_dictionary(*w).ok());
    let from = |key: &[u8]| {
        let s = aa_script(doc, fd, key);
        if s.is_empty() {
            widget.map(|w| aa_script(doc, w, key)).unwrap_or_default()
        } else {
            s
        }
    };
    let action = [Some(fd), widget]
        .into_iter()
        .flatten()
        .find_map(|d| {
            d.get(b"A").ok().and_then(|o| read_js(doc, o)).or_else(|| {
                objutil::dict_dict(doc, d, b"AA")
                    .and_then(|aa| aa.get(b"U").ok())
                    .and_then(|o| read_js(doc, o))
            })
        })
        .unwrap_or_default();
    FieldScripts {
        calculate: from(b"C"),
        validate: from(b"V"),
        format: from(b"F"),
        keystroke: from(b"K"),
        action,
    }
}

/// Write the scripts of a field: keystroke, format, validate and calculate in the field's `/AA`,
/// a button's action in its widget's `/A`. Keeps `/CO` in step with the calculate script.
pub fn set_field_scripts(tx: &mut Tx<'_>, field: ObjectId, s: &FieldScripts) -> Result<()> {
    for code in [
        &s.calculate,
        &s.validate,
        &s.format,
        &s.keystroke,
        &s.action,
    ] {
        if code.len() > MAX_SCRIPT {
            return Err(EngineError::LimitExceeded("the script is too long".into()));
        }
    }
    let mut aa = match tx
        .doc()
        .get_dictionary(field)?
        .get(b"AA")
        .ok()
        .and_then(|o| objutil::deref(tx.doc(), o).cloned())
    {
        Some(Object::Dictionary(d)) => d,
        _ => Dictionary::new(),
    };
    for (key, code) in [
        (b"C".as_slice(), &s.calculate),
        (b"V", &s.validate),
        (b"F", &s.format),
        (b"K", &s.keystroke),
    ] {
        if code.trim().is_empty() {
            aa.remove(key);
        } else {
            aa.set(key.to_vec(), js_action(code));
        }
    }
    let fd = tx.dict_mut(field)?;
    if aa.is_empty() {
        fd.remove(b"AA");
    } else {
        fd.set("AA", Object::Dictionary(aa));
    }
    let widget = forms::widgets_of(tx.doc(), field)
        .first()
        .copied()
        .unwrap_or(field);
    let is_button = matches!(
        forms::inherited_field(tx.doc(), tx.doc().get_dictionary(field)?, b"FT")
            .and_then(|o| o.as_name().ok()),
        Some(b"Btn")
    );
    if is_button {
        let wd = tx.dict_mut(widget)?;
        if s.action.trim().is_empty() {
            // Only our own kind of action is removed; a ResetForm or SubmitForm action stays.
            if wd.get(b"A").ok().is_some_and(|a| {
                a.as_dict()
                    .ok()
                    .and_then(|d| d.get(b"S").ok())
                    .and_then(|s| s.as_name().ok())
                    == Some(b"JavaScript")
            }) {
                wd.remove(b"A");
            }
        } else {
            wd.set("A", js_action(&s.action));
        }
    }
    set_in_calculation_order(tx, field, !s.calculate.trim().is_empty())
}

fn acroform_id(tx: &mut Tx<'_>) -> Result<Option<ObjectId>> {
    let cat = tx
        .doc()
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)?;
    let af = tx.doc().get_dictionary(cat)?.get(b"AcroForm").ok().cloned();
    Ok(match af {
        Some(Object::Reference(r)) => Some(r),
        Some(Object::Dictionary(d)) => {
            // Make the form dictionary its own object, so it can be changed like any other.
            let id = tx.add(Object::Dictionary(d));
            tx.dict_mut(cat)?.set("AcroForm", reference(id));
            Some(id)
        }
        _ => None,
    })
}

/// The `/AcroForm` dictionary's object, created when the document has none. Also makes sure there
/// is a Helvetica `/Helv` in the default resources for other readers that rebuild appearances.
pub fn ensure_acroform(tx: &mut Tx<'_>) -> Result<ObjectId> {
    let id = match acroform_id(tx)? {
        Some(id) => id,
        None => {
            let cat = tx
                .doc()
                .trailer
                .get(b"Root")
                .and_then(Object::as_reference)?;
            let id = tx.add(Object::Dictionary(dictionary! {
                "Fields" => Object::Array(Vec::new()),
            }));
            tx.dict_mut(cat)?.set("AcroForm", reference(id));
            id
        }
    };
    let mut dr = match tx
        .doc()
        .get_dictionary(id)?
        .get(b"DR")
        .ok()
        .and_then(|o| objutil::deref(tx.doc(), o).cloned())
    {
        Some(Object::Dictionary(d)) => d,
        _ => Dictionary::new(),
    };
    let mut fonts = match dr
        .get(b"Font")
        .ok()
        .and_then(|o| objutil::deref(tx.doc(), o).cloned())
    {
        Some(Object::Dictionary(d)) => d,
        _ => Dictionary::new(),
    };
    if !fonts.has(b"Helv") {
        let helv = tx.add(Object::Dictionary(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        }));
        fonts.set("Helv", reference(helv));
        dr.set("Font", Object::Dictionary(fonts));
        let afd = tx.dict_mut(id)?;
        afd.set("DR", Object::Dictionary(dr));
        if !afd.has(b"DA") {
            afd.set("DA", text_obj("/Helv 0 Tf 0 g"));
        }
    }
    if !tx.doc().get_dictionary(id)?.has(b"Fields") {
        tx.dict_mut(id)?.set("Fields", Object::Array(Vec::new()));
    }
    Ok(id)
}

fn set_in_calculation_order(tx: &mut Tx<'_>, field: ObjectId, present: bool) -> Result<()> {
    let Some(af) = acroform_id(tx)? else {
        return Ok(());
    };
    let mut co: Vec<Object> = tx
        .doc()
        .get_dictionary(af)?
        .get(b"CO")
        .ok()
        .and_then(|o| objutil::deref(tx.doc(), o).cloned())
        .and_then(|o| o.as_array().ok().cloned())
        .unwrap_or_default();
    let has = co.iter().any(|o| o.as_reference().ok() == Some(field));
    if present && !has {
        co.push(reference(field));
    } else if !present && has {
        co.retain(|o| o.as_reference().ok() != Some(field));
    } else {
        return Ok(());
    }
    let d = tx.dict_mut(af)?;
    if co.is_empty() {
        d.remove(b"CO");
    } else {
        d.set("CO", Object::Array(co));
    }
    Ok(())
}

/// Take a field out of the calculation order (when it is deleted).
pub fn forget_field(tx: &mut Tx<'_>, field: ObjectId) -> Result<()> {
    set_in_calculation_order(tx, field, false)
}

/// The calculation order as field objects.
pub fn calculation_order(doc: &Document) -> Vec<ObjectId> {
    forms::acroform(doc)
        .and_then(|af| objutil::dict_array(doc, af, b"CO"))
        .map(|a| a.iter().filter_map(|o| o.as_reference().ok()).collect())
        .unwrap_or_default()
}

// ------------------------------------------------------------------------------------------
// Document-level scripts
// ------------------------------------------------------------------------------------------

fn names_root(doc: &Document) -> Option<&Dictionary> {
    let cat = doc
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .ok()
        .and_then(|r| doc.get_dictionary(r).ok())?;
    let names = objutil::dict_dict(doc, cat, b"Names")?;
    objutil::dict_dict(doc, names, b"JavaScript")
}

fn read_name_tree(
    doc: &Document,
    node: &Dictionary,
    depth: usize,
    seen: &mut BTreeSet<ObjectId>,
    out: &mut Vec<(String, String)>,
) {
    if depth > 32 || out.len() >= MAX_DOC_SCRIPTS {
        return;
    }
    if let Some(names) = objutil::dict_array(doc, node, b"Names") {
        for pair in names.chunks(2) {
            let [k, v] = pair else { break };
            let name = objutil::deref(doc, k)
                .and_then(|o| o.as_str().ok())
                .map(objutil::decode_text_string)
                .unwrap_or_default();
            if let Some(code) = read_js(doc, v) {
                out.push((name, code));
            }
        }
    }
    if let Some(kids) = objutil::dict_array(doc, node, b"Kids") {
        for k in kids {
            if let Ok(id) = k.as_reference()
                && seen.insert(id)
                && let Ok(kd) = doc.get_dictionary(id)
            {
                read_name_tree(doc, kd, depth + 1, seen, out);
            }
        }
    }
}

/// The document-level scripts, `(name, code)`, in name order.
pub fn document_scripts(doc: &Document) -> Vec<(String, String)> {
    let Some(root) = names_root(doc) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    read_name_tree(doc, root, 0, &mut BTreeSet::new(), &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Replace the document-level scripts. An empty list removes them.
pub fn set_document_scripts(tx: &mut Tx<'_>, scripts: &[(String, String)]) -> Result<()> {
    if scripts.len() > MAX_DOC_SCRIPTS || scripts.iter().any(|(_, c)| c.len() > MAX_SCRIPT) {
        return Err(EngineError::LimitExceeded(
            "too many or too long scripts".into(),
        ));
    }
    let cat = tx
        .doc()
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)?;
    let mut names = match tx
        .doc()
        .get_dictionary(cat)?
        .get(b"Names")
        .ok()
        .and_then(|o| objutil::deref(tx.doc(), o).cloned())
    {
        Some(Object::Dictionary(d)) => d,
        _ => Dictionary::new(),
    };
    let mut sorted: Vec<&(String, String)> = scripts
        .iter()
        .filter(|(_, c)| !c.trim().is_empty())
        .collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    sorted.dedup_by(|a, b| a.0 == b.0);
    if sorted.is_empty() {
        names.remove(b"JavaScript");
    } else {
        let mut arr = Vec::new();
        for (n, c) in sorted {
            arr.push(text_obj(n));
            arr.push(reference(tx.add(js_action(c))));
        }
        let tree = tx.add(Object::Dictionary(
            dictionary! { "Names" => Object::Array(arr) },
        ));
        names.set("JavaScript", reference(tree));
    }
    let cd = tx.dict_mut(cat)?;
    if names.is_empty() {
        cd.remove(b"Names");
    } else {
        cd.set("Names", Object::Dictionary(names));
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------
// Everything a script run needs
// ------------------------------------------------------------------------------------------

/// One field as the script runtime sees it.
#[derive(Clone, Debug)]
pub struct ScriptField {
    /// Field object.
    pub id: ObjectId,
    /// Fully qualified name.
    pub name: String,
    /// Kind and options.
    pub kind: FieldKind,
    /// Current value.
    pub value: String,
    /// Default value (`/DV`), what a reset sets.
    pub default_value: String,
    /// Read-only.
    pub read_only: bool,
    /// Required.
    pub required: bool,
    /// Hidden (widget flag).
    pub hidden: bool,
    /// The field's scripts.
    pub scripts: FieldScripts,
    /// On-state names of the widgets (check boxes and radio buttons).
    pub export_values: Vec<String>,
}

/// The whole form with its scripts.
#[derive(Clone, Debug, Default)]
pub struct FormScripts {
    /// Terminal fields.
    pub fields: Vec<ScriptField>,
    /// Fields with a calculation, in the document's calculation order.
    pub order: Vec<ObjectId>,
    /// Document-level scripts.
    pub document: Vec<(String, String)>,
    /// Number of pages (`this.numPages`).
    pub pages: usize,
}

impl FormScripts {
    /// Whether there is anything to run.
    pub fn has_scripts(&self) -> bool {
        !self.document.is_empty() || self.fields.iter().any(|f| !f.scripts.is_empty())
    }
}

/// Read the form and every script in it.
pub fn read_form_scripts(doc: &PdfDocument) -> FormScripts {
    let d = doc.lopdf();
    let info = forms::read_form(doc);
    let fields: Vec<ScriptField> = info
        .fields
        .iter()
        .map(|f| ScriptField {
            id: f.id,
            name: f.name.clone(),
            kind: f.kind.clone(),
            value: f.value.clone(),
            default_value: d
                .get_dictionary(f.id)
                .ok()
                .map(|fd| forms::text_of(d, forms::inherited_field(d, fd, b"DV")))
                .unwrap_or_default(),
            read_only: f.read_only,
            required: f.required,
            hidden: f.widgets.first().is_some_and(|w| {
                d.get_dictionary(w.id)
                    .ok()
                    .and_then(|wd| objutil::dict_num(d, wd, b"F"))
                    .is_some_and(|fl| (fl as i64) & 2 != 0)
            }),
            scripts: field_scripts(d, f.id),
            export_values: f
                .widgets
                .iter()
                .filter_map(|w| w.on_state.clone())
                .collect(),
        })
        .collect();
    let mut order = calculation_order(d);
    // Fields with a calculation that the order forgot still calculate, after the listed ones.
    for f in &fields {
        if !f.scripts.calculate.is_empty() && !order.contains(&f.id) {
            order.push(f.id);
        }
    }
    order.retain(|id| {
        fields
            .iter()
            .any(|f| f.id == *id && !f.scripts.calculate.is_empty())
    });
    FormScripts {
        fields,
        order,
        document: document_scripts(d),
        pages: doc.page_count(),
    }
}

/// Show or hide every widget of a field (widget flag 2), as a script asked.
pub fn set_field_hidden(tx: &mut Tx<'_>, field: ObjectId, hidden: bool) -> Result<()> {
    for w in forms::widgets_of(tx.doc(), field) {
        let flags = tx
            .doc()
            .get_dictionary(w)
            .ok()
            .and_then(|wd| objutil::dict_num(tx.doc(), wd, b"F"))
            .map_or(4, |f| f as i64);
        let new = if hidden { flags | 2 } else { flags & !2 };
        if new != flags {
            tx.dict_mut(w)?.set("F", new);
        }
    }
    Ok(())
}

/// Set or clear the read-only and required flags of a field, as a script asked.
pub fn set_field_flags(
    tx: &mut Tx<'_>,
    field: ObjectId,
    read_only: Option<bool>,
    required: Option<bool>,
) -> Result<()> {
    let ff = forms::inherited_field(tx.doc(), tx.doc().get_dictionary(field)?, b"Ff")
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(0);
    let mut new = ff;
    if let Some(r) = read_only {
        new = if r { new | 1 } else { new & !1 };
    }
    if let Some(r) = required {
        new = if r { new | 2 } else { new & !2 };
    }
    if new != ff {
        tx.dict_mut(field)?.set("Ff", new);
    }
    Ok(())
}

/// Put every field back to its default value (a "reset form" button). Returns the number of fields.
pub fn reset_fields(tx: &mut Tx<'_>, form: &FormScripts) -> Result<usize> {
    let mut n = 0;
    for f in &form.fields {
        match &f.kind {
            FieldKind::Text { .. } | FieldKind::Choice { .. } => {
                if f.value != f.default_value {
                    forms::set_text_value_as(tx, f.id, &f.default_value, None, true)?;
                    n += 1;
                }
            }
            FieldKind::Checkbox => {
                let on = !matches!(f.default_value.as_str(), "" | "Off");
                forms::set_checkbox(tx, f.id, on)?;
                n += 1;
            }
            FieldKind::Radio => {
                let state = if f.default_value.is_empty() {
                    "Off"
                } else {
                    f.default_value.as_str()
                };
                if forms::set_radio(tx, f.id, state).is_ok() {
                    n += 1;
                }
            }
            _ => {}
        }
    }
    Ok(n)
}
