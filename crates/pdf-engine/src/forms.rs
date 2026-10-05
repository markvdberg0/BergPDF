//! AcroForm support: reading, filling, flattening and field-aware page duplication/import.
//!
//! Scope: AcroForm text, checkbox, radio, combo/list and push-button *reading*, and filling of
//! text, checkbox, radio and choice fields with generated appearance streams. XFA forms and
//! calculation/format/validation JavaScript are **detected and reported, never executed**.
//!
//! **Duplicated form pages.** Copying a page whose widgets belong to a field raises a real
//! question the user must answer — [`FormPolicy`]:
//!
//! * `Linked` – the copy's widgets become additional kids of the *same* field: one value
//!   shown in several places. (A merged field+widget is split into a field with two kids.)
//! * `Independent` – the copy gets *new* fields with collision-free names, copying
//!   configuration, value and appearance; editing one never changes the other.
//! * `Flatten` – the copy gets the widgets' appearances baked into page content (no fields).

use crate::doc::{PageId, PdfDocument, Tx};
use crate::error::{EngineError, Result};
use crate::fontembed::{BundledFace, FontBuilder, hex_string, zlib};
use crate::geom::Rect;
use crate::objutil::{self, fmt_num_prec, name, num_array, reference, text_obj};
use crate::pagecontent::add_resource;
use crate::pageops::FormPolicy;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};
use std::collections::{BTreeMap, BTreeSet};

const MAX_FIELD_DEPTH: usize = 32;
const MAX_FIELDS: usize = 100_000;

/// Field type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldKind {
    /// Text field.
    Text {
        /// Multi-line.
        multiline: bool,
        /// Password (value is masked in the appearance).
        password: bool,
        /// Comb (equally spaced cells; needs `max_len`).
        comb: bool,
        /// `MaxLen`.
        max_len: Option<usize>,
    },
    /// Checkbox.
    Checkbox,
    /// Radio button group.
    Radio,
    /// Combo box or list box.
    Choice {
        /// Combo (drop-down) rather than list.
        combo: bool,
        /// `(export value, display text)` options.
        options: Vec<(String, String)>,
    },
    /// Push button (never executes actions).
    PushButton,
    /// Signature field (displayed; signing is out of scope).
    Signature,
    /// Anything else.
    Unknown,
}

/// A widget placement.
#[derive(Clone, Debug)]
pub struct WidgetInfo {
    /// Widget annotation object.
    pub id: ObjectId,
    /// Page the widget is on, if known.
    pub page: Option<PageId>,
    /// Rectangle in user space.
    pub rect: Rect,
    /// Name of the "on" appearance state (checkbox/radio).
    pub on_state: Option<String>,
}

/// A terminal form field.
#[derive(Clone, Debug)]
pub struct FormField {
    /// Field object (for a merged field+widget this is also the widget).
    pub id: ObjectId,
    /// Fully qualified name.
    pub name: String,
    /// Type and options.
    pub kind: FieldKind,
    /// Current value as text (checkbox/radio: the state name; empty/`Off` = unchecked).
    pub value: String,
    /// Read-only flag.
    pub read_only: bool,
    /// Required flag.
    pub required: bool,
    /// Widgets of this field.
    pub widgets: Vec<WidgetInfo>,
}

/// Document-level form information.
#[derive(Clone, Debug, Default)]
pub struct FormInfo {
    /// Terminal fields.
    pub fields: Vec<FormField>,
    /// XFA data present (unsupported).
    pub xfa: bool,
    /// Calculation order / calculate actions present (never executed).
    pub has_calculations: bool,
    /// Any field has a JavaScript action (never executed).
    pub has_scripts: bool,
}

fn acroform(doc: &Document) -> Option<&Dictionary> {
    let cat = doc
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .ok()
        .and_then(|r| doc.get_dictionary(r).ok())?;
    objutil::dict_dict(doc, cat, b"AcroForm")
}

fn inherited_field<'a>(doc: &'a Document, mut d: &'a Dictionary, key: &[u8]) -> Option<&'a Object> {
    for _ in 0..MAX_FIELD_DEPTH {
        if let Ok(v) = d.get(key) {
            return objutil::deref(doc, v);
        }
        d = objutil::dict_dict(doc, d, b"Parent")?;
    }
    None
}

fn text_of(doc: &Document, o: Option<&Object>) -> String {
    match o.and_then(|o| objutil::deref(doc, o)) {
        Some(Object::String(s, _)) => objutil::decode_text_string(s),
        Some(Object::Name(n)) => String::from_utf8_lossy(n).into_owned(),
        _ => String::new(),
    }
}

/// Read all form fields.
pub fn read_form(doc: &PdfDocument) -> FormInfo {
    let d = doc.lopdf();
    let mut info = FormInfo::default();
    let Some(af) = acroform(d) else { return info };
    info.xfa = af.has(b"XFA");
    info.has_calculations = af.has(b"CO");
    let page_of: BTreeMap<ObjectId, PageId> = doc
        .page_ids()
        .unwrap_or_default()
        .into_iter()
        .map(|p| (p.0, p))
        .collect();
    let Some(fields) = objutil::dict_array(d, af, b"Fields") else {
        return info;
    };
    let mut seen = BTreeSet::new();
    for f in fields {
        if let Ok(id) = f.as_reference() {
            collect(d, id, "", 0, &page_of, &mut seen, &mut info);
        }
    }
    info
}

fn collect(
    d: &Document,
    id: ObjectId,
    prefix: &str,
    depth: usize,
    page_of: &BTreeMap<ObjectId, PageId>,
    seen: &mut BTreeSet<ObjectId>,
    info: &mut FormInfo,
) {
    if depth > MAX_FIELD_DEPTH || !seen.insert(id) || info.fields.len() >= MAX_FIELDS {
        return;
    }
    let Ok(fd) = d.get_dictionary(id) else { return };
    let partial = text_of(d, fd.get(b"T").ok());
    let fqn = match (prefix.is_empty(), partial.is_empty()) {
        (true, _) => partial.clone(),
        (false, true) => prefix.to_string(),
        (false, false) => format!("{prefix}.{partial}"),
    };
    if let Some(aa) = objutil::dict_dict(d, fd, b"AA")
        && (aa.has(b"C") || aa.has(b"F") || aa.has(b"V") || aa.has(b"K"))
    {
        info.has_scripts = true;
        if aa.has(b"C") {
            info.has_calculations = true;
        }
    }
    let kids: Vec<ObjectId> = objutil::dict_array(d, fd, b"Kids")
        .map(|a| a.iter().filter_map(|o| o.as_reference().ok()).collect())
        .unwrap_or_default();
    // A kid is a *field* if it has /T; otherwise it is just a widget of this field.
    let field_kids: Vec<ObjectId> = kids
        .iter()
        .copied()
        .filter(|k| d.get_dictionary(*k).is_ok_and(|kd| kd.has(b"T")))
        .collect();
    if !field_kids.is_empty() {
        for k in field_kids {
            collect(d, k, &fqn, depth + 1, page_of, seen, info);
        }
        return;
    }
    // Terminal field.
    let ft = inherited_field(d, fd, b"FT")
        .and_then(|o| o.as_name().ok())
        .map(<[u8]>::to_vec);
    let ff = inherited_field(d, fd, b"Ff")
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(0);
    let value = text_of(d, inherited_field(d, fd, b"V"));
    let kind = match ft.as_deref() {
        Some(b"Tx") => FieldKind::Text {
            multiline: ff & (1 << 12) != 0,
            password: ff & (1 << 13) != 0,
            comb: ff & (1 << 24) != 0,
            max_len: inherited_field(d, fd, b"MaxLen")
                .and_then(|o| o.as_i64().ok())
                .and_then(|n| usize::try_from(n).ok()),
        },
        Some(b"Btn") => {
            if ff & (1 << 16) != 0 {
                FieldKind::PushButton
            } else if ff & (1 << 15) != 0 {
                FieldKind::Radio
            } else {
                FieldKind::Checkbox
            }
        }
        Some(b"Ch") => {
            let opts = inherited_field(d, fd, b"Opt")
                .and_then(|o| o.as_array().ok())
                .map(|a| {
                    a.iter()
                        .map(|o| match objutil::deref(d, o) {
                            Some(Object::Array(pair)) if pair.len() >= 2 => {
                                (text_of(d, pair.first()), text_of(d, pair.get(1)))
                            }
                            other => {
                                let t = text_of(d, other);
                                (t.clone(), t)
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            FieldKind::Choice {
                combo: ff & (1 << 17) != 0,
                options: opts,
            }
        }
        Some(b"Sig") => FieldKind::Signature,
        _ => FieldKind::Unknown,
    };
    let widget_ids: Vec<ObjectId> = if kids.is_empty() { vec![id] } else { kids };
    let widgets = widget_ids
        .iter()
        .filter_map(|w| {
            let wd = d.get_dictionary(*w).ok()?;
            let rect = wd.get(b"Rect").ok().and_then(|r| objutil::rect(d, r))?;
            let page = wd
                .get(b"P")
                .ok()
                .and_then(|p| p.as_reference().ok())
                .and_then(|p| page_of.get(&p).copied())
                .or_else(|| find_page_with_annot(d, page_of, *w));
            Some(WidgetInfo {
                id: *w,
                page,
                rect,
                on_state: on_state_of(d, wd),
            })
        })
        .collect();
    info.fields.push(FormField {
        id,
        name: fqn,
        kind,
        value,
        read_only: ff & 1 != 0,
        required: ff & 2 != 0,
        widgets,
    });
}

fn find_page_with_annot(
    d: &Document,
    page_of: &BTreeMap<ObjectId, PageId>,
    annot: ObjectId,
) -> Option<PageId> {
    page_of.iter().find_map(|(pid, p)| {
        let pd = d.get_dictionary(*pid).ok()?;
        let arr = objutil::dict_array(d, pd, b"Annots")?;
        arr.iter()
            .any(|o| o.as_reference().ok() == Some(annot))
            .then_some(*p)
    })
}

fn on_state_of(d: &Document, wd: &Dictionary) -> Option<String> {
    let ap = objutil::dict_dict(d, wd, b"AP")?;
    let n = objutil::dict_dict(d, ap, b"N")?;
    n.iter()
        .map(|(k, _)| String::from_utf8_lossy(k).into_owned())
        .find(|k| k != "Off")
}

// ------------------------------------------------------------------------------------------
// Filling
// ------------------------------------------------------------------------------------------

/// Parsed `/DA` default appearance.
struct Da {
    size: f64,
    color: (f32, f32, f32),
}

fn parse_da(s: &str) -> Da {
    let t: Vec<&str> = s.split_whitespace().collect();
    let mut da = Da {
        size: 0.0,
        color: (0.0, 0.0, 0.0),
    };
    for (i, w) in t.iter().enumerate() {
        match *w {
            "Tf" if i >= 1 => da.size = t[i - 1].parse().unwrap_or(0.0),
            "g" if i >= 1 => {
                let v = t[i - 1].parse().unwrap_or(0.0);
                da.color = (v, v, v);
            }
            "rg" if i >= 3 => {
                let p = |s: &str| s.parse::<f32>().unwrap_or(0.0);
                da.color = (p(t[i - 3]), p(t[i - 2]), p(t[i - 1]));
            }
            _ => {}
        }
    }
    da
}

fn field_attr(d: &Document, field: ObjectId, key: &[u8]) -> Option<Object> {
    let fd = d.get_dictionary(field).ok()?;
    inherited_field(d, fd, key).cloned()
}

fn widgets_of(d: &Document, field: ObjectId) -> Vec<ObjectId> {
    let Ok(fd) = d.get_dictionary(field) else {
        return Vec::new();
    };
    match objutil::dict_array(d, fd, b"Kids") {
        Some(k) if !k.is_empty() => k.iter().filter_map(|o| o.as_reference().ok()).collect(),
        _ => vec![field],
    }
}

fn acroform_da(d: &Document) -> String {
    acroform(d)
        .map(|af| text_of(d, af.get(b"DA").ok()))
        .unwrap_or_default()
}

/// Display text of a choice value (export value → option label).
fn choice_display(d: &Document, field: ObjectId, value: &str) -> String {
    if let Some(Object::Array(opts)) = field_attr(d, field, b"Opt") {
        for o in opts {
            if let Some(Object::Array(p)) = objutil::deref(d, &o)
                && p.len() >= 2
                && text_of(d, p.first()) == value
            {
                return text_of(d, p.get(1));
            }
        }
    }
    value.to_string()
}

/// For flattening: give a text/choice widget that has a value but no appearance one, so the
/// value is not lost. Read-only fields are allowed (flattening is not editing).
fn generate_missing_appearance(tx: &mut Tx<'_>, widget: ObjectId) -> Result<()> {
    let field = terminal_field(tx.doc(), widget);
    let (ft, ff, value) = {
        let d = tx.doc();
        let fd = d.get_dictionary(field)?;
        (
            inherited_field(d, fd, b"FT")
                .and_then(|o| o.as_name().ok())
                .map(<[u8]>::to_vec),
            inherited_field(d, fd, b"Ff")
                .and_then(|o| o.as_i64().ok())
                .unwrap_or(0),
            text_of(d, inherited_field(d, fd, b"V")),
        )
    };
    if !matches!(ft.as_deref(), Some(b"Tx") | Some(b"Ch")) || value.is_empty() {
        return Err(EngineError::Unsupported("nothing to draw".into()));
    }
    let display = if ft.as_deref() == Some(b"Ch") {
        choice_display(tx.doc(), field, &value)
    } else {
        value
    };
    regenerate_text_appearance(tx, field, widget, &display, ff)
}

/// Set a text/choice field value and regenerate the appearance of each of its widgets.
pub fn set_text_value(tx: &mut Tx<'_>, field: ObjectId, value: &str) -> Result<()> {
    let info = {
        let d = tx.doc();
        let fd = d
            .get_dictionary(field)
            .map_err(|_| EngineError::NoSuchObject(field.0, field.1))?;
        let ft = inherited_field(d, fd, b"FT")
            .and_then(|o| o.as_name().ok())
            .map(<[u8]>::to_vec);
        if !matches!(ft.as_deref(), Some(b"Tx") | Some(b"Ch")) {
            return Err(EngineError::InvalidArgument(
                "not a text or choice field".into(),
            ));
        }
        let ff = inherited_field(d, fd, b"Ff")
            .and_then(|o| o.as_i64().ok())
            .unwrap_or(0);
        if ff & 1 != 0 {
            return Err(EngineError::Unsupported("this field is read-only".into()));
        }
        let max_len = inherited_field(d, fd, b"MaxLen")
            .and_then(|o| o.as_i64().ok())
            .and_then(|n| usize::try_from(n).ok());
        (ft, ff, max_len)
    };
    let (ft, ff, max_len) = info;
    let mut text = value.to_string();
    if let Some(m) = max_len
        && text.chars().count() > m
    {
        return Err(EngineError::InvalidArgument(format!(
            "the value is longer than the field's maximum of {m} characters"
        )));
    }
    // Choice fields store the export value; show the display text.
    let display = if ft.as_deref() == Some(b"Ch") {
        choice_display(tx.doc(), field, &text)
    } else {
        text.clone()
    };
    tx.dict_mut(field)?.set("V", text_obj(&text));
    text.clear();
    let widgets = widgets_of(tx.doc(), field);
    for w in widgets {
        regenerate_text_appearance(tx, field, w, &display, ff)?;
    }
    Ok(())
}

fn regenerate_text_appearance(
    tx: &mut Tx<'_>,
    field: ObjectId,
    widget: ObjectId,
    display: &str,
    ff: i64,
) -> Result<()> {
    let d = tx.doc();
    let wd = d.get_dictionary(widget)?;
    let rect = wd
        .get(b"Rect")
        .ok()
        .and_then(|r| objutil::rect(d, r))
        .ok_or_else(|| EngineError::Malformed("widget without /Rect".into()))?;
    let da_str = {
        let s = text_of(d, inherited_field(d, d.get_dictionary(field)?, b"DA"));
        if s.is_empty() { acroform_da(d) } else { s }
    };
    let da = parse_da(&da_str);
    let q = inherited_field(d, d.get_dictionary(field)?, b"Q")
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(0);
    let multiline = ff & (1 << 12) != 0;
    let password = ff & (1 << 13) != 0;
    let comb = ff & (1 << 24) != 0;
    let max_len = inherited_field(d, d.get_dictionary(field)?, b"MaxLen")
        .and_then(|o| o.as_i64().ok())
        .and_then(|n| usize::try_from(n).ok());
    let mk = objutil::dict_dict(d, wd, b"MK");
    let bg = mk
        .and_then(|m| m.get(b"BG").ok())
        .and_then(|o| color_ops(d, o));
    let bc = mk
        .and_then(|m| m.get(b"BC").ok())
        .and_then(|o| color_ops(d, o));
    let bw = objutil::dict_dict(d, wd, b"BS")
        .and_then(|b| objutil::dict_num(d, b, b"W"))
        .unwrap_or(if bc.is_some() { 1.0 } else { 0.0 });
    let shown: String = if password {
        display.chars().map(|_| '•').collect()
    } else {
        display.to_string()
    };
    let (w, h) = (rect.width(), rect.height());
    let mut fb = FontBuilder::new(BundledFace::Sans)?;
    let missing = fb.missing_chars(&shown);
    if !missing.is_empty() {
        return Err(EngineError::MissingGlyphs {
            font: "DejaVu Sans".into(),
            chars: missing
                .iter()
                .map(char::to_string)
                .collect::<Vec<_>>()
                .join(" "),
        });
    }
    let pad = 2.0 + bw;
    let size = if da.size > 0.0 {
        da.size
    } else if multiline {
        12.0
    } else {
        (h - 2.0 * pad).clamp(4.0, 14.0)
    };
    let mut c = String::new();
    if let Some(bg) = &bg {
        c.push_str(&format!(
            "{bg} rg 0 0 {} {} re f\n",
            fmt_num_prec(w),
            fmt_num_prec(h)
        ));
    }
    if let Some(bc) = &bc
        && bw > 0.0
    {
        c.push_str(&format!(
            "{bc} RG {} w {} {} {} {} re S\n",
            fmt_num_prec(bw),
            fmt_num_prec(bw / 2.0),
            fmt_num_prec(bw / 2.0),
            fmt_num_prec(w - bw),
            fmt_num_prec(h - bw)
        ));
    }
    c.push_str("/Tx BMC\nq\n");
    c.push_str(&format!(
        "{} {} {} {} re W n\nBT\n/F1 {} Tf {} {} {} rg\n",
        fmt_num_prec(bw),
        fmt_num_prec(bw),
        fmt_num_prec(w - 2.0 * bw),
        fmt_num_prec(h - 2.0 * bw),
        fmt_num_prec(size),
        fmt_num_prec(f64::from(da.color.0)),
        fmt_num_prec(f64::from(da.color.1)),
        fmt_num_prec(f64::from(da.color.2))
    ));
    let lh = fb.line_height_em() * size;
    if comb && let Some(n) = max_len.filter(|n| *n > 0) {
        let cell = (w - 2.0 * bw) / n as f64;
        let y = (h - size * (fb.ascent_em() + fb.descent_em())) / 2.0;
        let mut prev_x = 0.0;
        for (i, ch) in shown.chars().take(n).enumerate() {
            let cw = fb.advance_em(ch) * size;
            let x = bw + cell * i as f64 + (cell - cw) / 2.0;
            let code = fb.encode_char(ch)?;
            c.push_str(&format!(
                "{} {} Td {} Tj\n",
                fmt_num_prec(x - prev_x),
                fmt_num_prec(if i == 0 { y } else { 0.0 }),
                hex_string(&code.to_be_bytes())
            ));
            prev_x = x;
        }
    } else {
        let lines: Vec<String> = if multiline {
            crate::annot::wrap_lines_pub(&fb, &shown, size, (w - 2.0 * pad).max(1.0))
        } else {
            vec![shown.replace('\n', " ")]
        };
        let mut cx = 0.0;
        for (i, line) in lines.iter().enumerate() {
            let tw = fb.text_width(line, size);
            let x = match q {
                1 => (w - tw) / 2.0,
                2 => w - pad - tw,
                _ => pad,
            };
            let y = if multiline {
                h - pad - fb.ascent_em() * size - lh * i as f64
            } else {
                (h - size * (fb.ascent_em() + fb.descent_em())) / 2.0
            };
            let codes = fb.encode_str(line)?;
            if i == 0 {
                c.push_str(&format!("{} {} Td\n", fmt_num_prec(x), fmt_num_prec(y)));
            } else {
                c.push_str(&format!(
                    "{} {} Td\n",
                    fmt_num_prec(x - cx),
                    fmt_num_prec(-lh)
                ));
            }
            if !codes.is_empty() {
                c.push_str(&format!("{} Tj\n", hex_string(&codes)));
            }
            cx = x;
        }
    }
    c.push_str("ET\nQ\nEMC\n");
    let font_id = fb.finish(tx)?;
    let mut stream = Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "FormType" => 1i64,
            "BBox" => num_array(&[0.0, 0.0, w, h]),
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } },
            "Filter" => "FlateDecode",
        },
        zlib(c.as_bytes()),
    );
    stream.allows_compression = false;
    let ap_id = tx.add(Object::Stream(stream));
    tx.dict_mut(widget)?.set("AP", dictionary! { "N" => ap_id });
    Ok(())
}

fn color_ops(d: &Document, o: &Object) -> Option<String> {
    let a = objutil::deref(d, o)?.as_array().ok()?;
    let v: Vec<f64> = a.iter().filter_map(|x| objutil::num(d, x)).collect();
    match v.len() {
        1 => Some(format!("{0} {0} {0}", fmt_num_prec(v[0]))),
        3 => Some(format!(
            "{} {} {}",
            fmt_num_prec(v[0]),
            fmt_num_prec(v[1]),
            fmt_num_prec(v[2])
        )),
        _ => None,
    }
}

/// Set a checkbox on/off.
pub fn set_checkbox(tx: &mut Tx<'_>, field: ObjectId, on: bool) -> Result<()> {
    let widgets = widgets_of(tx.doc(), field);
    if widgets.is_empty() {
        return Err(EngineError::Malformed("checkbox without widgets".into()));
    }
    let mut state = "Off".to_string();
    for w in &widgets {
        let on_name =
            on_state_of(tx.doc(), tx.doc().get_dictionary(*w)?).unwrap_or_else(|| "Yes".into());
        if on {
            state = on_name.clone();
        }
        let a = if on { on_name } else { "Off".to_string() };
        tx.dict_mut(*w)?.set("AS", name(&a));
    }
    tx.dict_mut(field)?.set("V", name(&state));
    Ok(())
}

/// Select a radio button of a group by its state name.
pub fn set_radio(tx: &mut Tx<'_>, field: ObjectId, state: &str) -> Result<()> {
    let widgets = widgets_of(tx.doc(), field);
    let known: Vec<Option<String>> = widgets
        .iter()
        .map(|w| {
            tx.doc()
                .get_dictionary(*w)
                .ok()
                .and_then(|wd| on_state_of(tx.doc(), wd))
        })
        .collect();
    if !known.iter().any(|k| k.as_deref() == Some(state)) && state != "Off" {
        return Err(EngineError::InvalidArgument(format!(
            "this radio group has no “{state}” button"
        )));
    }
    for (w, k) in widgets.iter().zip(&known) {
        let a = if k.as_deref() == Some(state) {
            state.to_string()
        } else {
            "Off".into()
        };
        tx.dict_mut(*w)?.set("AS", name(&a));
    }
    tx.dict_mut(field)?.set("V", name(state));
    Ok(())
}

// ------------------------------------------------------------------------------------------
// Flatten
// ------------------------------------------------------------------------------------------

/// Bake every widget of `page` into its content and remove the fields. Returns the number of
/// widgets flattened. Widgets without an appearance stream are removed without baking and
/// counted in the second value.
pub fn flatten_page_widgets(tx: &mut Tx<'_>, page: PageId) -> Result<(usize, usize)> {
    let pd = tx.doc().get_dictionary(page.0)?.clone();
    let annots: Vec<ObjectId> = objutil::dict_array(tx.doc(), &pd, b"Annots")
        .map(|a| a.iter().filter_map(|o| o.as_reference().ok()).collect())
        .unwrap_or_default();
    let mut baked = 0;
    let mut dropped = 0;
    let mut keep: Vec<Object> = Vec::new();
    let mut content = String::new();
    let pc = crate::pagecontent::PageContent::load(tx.doc(), page.0)?;
    let (ctm, _) = pc.end_state();
    let inv = ctm
        .inverse()
        .ok_or_else(|| EngineError::Unsupported("degenerate page transform".into()))?;
    let mut removed_fields: Vec<ObjectId> = Vec::new();
    for a in annots {
        let Ok(ad) = tx.doc().get_dictionary(a).cloned() else {
            continue;
        };
        if objutil::dict_name(tx.doc(), &ad, b"Subtype") != Some(b"Widget") {
            keep.push(reference(a));
            continue;
        }
        removed_fields.push(a);
        let hidden = ad
            .get(b"F")
            .ok()
            .and_then(|o| o.as_i64().ok())
            .is_some_and(|f| f & 2 != 0 || f & 32 != 0);
        let mut ad = ad;
        if !hidden
            && widget_appearance(tx.doc(), &ad).is_none()
            && generate_missing_appearance(tx, a).is_ok()
            && let Ok(fresh) = tx.doc().get_dictionary(a)
        {
            ad = fresh.clone();
        }
        let ap = widget_appearance(tx.doc(), &ad);
        match (hidden, ap) {
            (true, _) => dropped += 1,
            (false, None) => dropped += 1,
            (false, Some(ap_id)) => {
                let Some(rect) = ad
                    .get(b"Rect")
                    .ok()
                    .and_then(|r| objutil::rect(tx.doc(), r))
                else {
                    continue;
                };
                let (bbox, matrix) = match tx.doc().objects.get(&ap_id) {
                    Some(Object::Stream(s)) => (
                        s.dict
                            .get(b"BBox")
                            .ok()
                            .and_then(|b| objutil::rect(tx.doc(), b))
                            .unwrap_or(Rect::new(0.0, 0.0, rect.width(), rect.height())),
                        s.dict
                            .get(b"Matrix")
                            .ok()
                            .and_then(|m| matrix_of(tx.doc(), m))
                            .unwrap_or(crate::content::Mat::IDENTITY),
                    ),
                    _ => continue,
                };
                let corners = [
                    matrix.apply(bbox.x0, bbox.y0),
                    matrix.apply(bbox.x1, bbox.y0),
                    matrix.apply(bbox.x1, bbox.y1),
                    matrix.apply(bbox.x0, bbox.y1),
                ];
                let (tx0, ty0) = (
                    corners.iter().map(|c| c.0).fold(f64::MAX, f64::min),
                    corners.iter().map(|c| c.1).fold(f64::MAX, f64::min),
                );
                let (tx1, ty1) = (
                    corners.iter().map(|c| c.0).fold(f64::MIN, f64::max),
                    corners.iter().map(|c| c.1).fold(f64::MIN, f64::max),
                );
                let (sx, sy) = (
                    rect.width() / (tx1 - tx0).max(1e-9),
                    rect.height() / (ty1 - ty0).max(1e-9),
                );
                let am =
                    crate::content::Mat([sx, 0.0, 0.0, sy, rect.x0 - tx0 * sx, rect.y0 - ty0 * sy]);
                let nm = format!("BergFl{}", ap_id.0);
                add_resource(tx, page.0, b"XObject", &nm, reference(ap_id))?;
                content.push_str(&format!(
                    "q {} cm {} cm /{} Do Q\n",
                    inv.operands(),
                    am.operands(),
                    nm
                ));
                baked += 1;
            }
        }
    }
    if !content.is_empty() {
        let mut stream = Stream::new(
            dictionary! { "Filter" => "FlateDecode" },
            zlib(format!("q\n{content}Q\n").as_bytes()),
        );
        stream.allows_compression = false;
        let sid = tx.add(Object::Stream(stream));
        let mut arr: Vec<Object> = crate::pagecontent::content_stream_ids(tx.doc(), page.0)
            .into_iter()
            .map(reference)
            .collect();
        arr.push(reference(sid));
        tx.dict_mut(page.0)?.set("Contents", Object::Array(arr));
    }
    if keep.is_empty() {
        tx.dict_mut(page.0)?.remove(b"Annots");
    } else {
        tx.dict_mut(page.0)?.set("Annots", Object::Array(keep));
    }
    for w in removed_fields {
        detach_field(tx, w)?;
    }
    Ok((baked, dropped))
}

fn matrix_of(d: &Document, o: &Object) -> Option<crate::content::Mat> {
    let a = objutil::deref(d, o)?.as_array().ok()?;
    let v: Vec<f64> = a.iter().filter_map(|x| objutil::num(d, x)).collect();
    (v.len() == 6).then(|| crate::content::Mat([v[0], v[1], v[2], v[3], v[4], v[5]]))
}

fn widget_appearance(d: &Document, wd: &Dictionary) -> Option<ObjectId> {
    let ap = objutil::dict_dict(d, wd, b"AP")?;
    let n = ap.get(b"N").ok()?;
    match n {
        Object::Reference(r) => match d.objects.get(r)? {
            Object::Stream(_) => Some(*r),
            Object::Dictionary(states) => pick_state(d, states, wd),
            _ => None,
        },
        Object::Dictionary(states) => pick_state(d, states, wd),
        _ => None,
    }
}

fn pick_state(d: &Document, states: &Dictionary, wd: &Dictionary) -> Option<ObjectId> {
    let as_name = objutil::dict_name(d, wd, b"AS")?;
    states.get(as_name).ok()?.as_reference().ok()
}

/// Remove a widget/field from the AcroForm tree (and from its parent's `/Kids`).
fn detach_field(tx: &mut Tx<'_>, id: ObjectId) -> Result<()> {
    let d = tx.doc();
    let parent = d
        .get_dictionary(id)
        .ok()
        .and_then(|f| f.get(b"Parent").ok().and_then(|p| p.as_reference().ok()));
    if let Some(p) = parent {
        let remaining: Vec<Object> =
            objutil::dict_array(tx.doc(), tx.doc().get_dictionary(p)?, b"Kids")
                .map(|k| {
                    k.iter()
                        .filter(|o| o.as_reference().ok() != Some(id))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
        let empty = remaining.is_empty();
        tx.dict_mut(p)?.set("Kids", Object::Array(remaining));
        if empty {
            detach_field(tx, p)?;
        }
        return Ok(());
    }
    let cat = tx
        .doc()
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)?;
    let af_is_ref = tx
        .doc()
        .get_dictionary(cat)?
        .get(b"AcroForm")
        .ok()
        .and_then(|o| o.as_reference().ok());
    let filter = |fields: &Vec<Object>| -> Vec<Object> {
        fields
            .iter()
            .filter(|o| o.as_reference().ok() != Some(id))
            .cloned()
            .collect()
    };
    match af_is_ref {
        Some(r) => {
            if let Ok(Object::Array(f)) = tx.dict_mut(r)?.get(b"Fields").cloned() {
                tx.dict_mut(r)?.set("Fields", Object::Array(filter(&f)));
            }
        }
        None => {
            if let Ok(Object::Dictionary(af)) = tx.dict_mut(cat)?.get_mut(b"AcroForm")
                && let Ok(Object::Array(f)) = af.get(b"Fields").cloned()
            {
                af.set("Fields", Object::Array(filter(&f)));
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------
// Duplicating / importing pages with widgets
// ------------------------------------------------------------------------------------------

fn all_field_names(d: &Document) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let Some(af) = acroform(d) else { return names };
    let Some(fields) = objutil::dict_array(d, af, b"Fields") else {
        return names;
    };
    fn walk(
        d: &Document,
        id: ObjectId,
        prefix: &str,
        depth: usize,
        names: &mut BTreeSet<String>,
        seen: &mut BTreeSet<ObjectId>,
    ) {
        if depth > MAX_FIELD_DEPTH || !seen.insert(id) {
            return;
        }
        let Ok(fd) = d.get_dictionary(id) else { return };
        let t = text_of(d, fd.get(b"T").ok());
        let fqn = if prefix.is_empty() {
            t.clone()
        } else if t.is_empty() {
            prefix.to_string()
        } else {
            format!("{prefix}.{t}")
        };
        if !fqn.is_empty() {
            names.insert(fqn.clone());
        }
        if let Some(k) = objutil::dict_array(d, fd, b"Kids") {
            for o in k {
                if let Ok(r) = o.as_reference() {
                    walk(d, r, &fqn, depth + 1, names, seen);
                }
            }
        }
    }
    let mut seen = BTreeSet::new();
    for f in fields {
        if let Ok(r) = f.as_reference() {
            walk(d, r, "", 0, &mut names, &mut seen);
        }
    }
    names
}

fn unique_partial_name(existing: &BTreeSet<String>, prefix: &str, base: &str) -> String {
    let join = |n: &str| {
        if prefix.is_empty() {
            n.to_string()
        } else {
            format!("{prefix}.{n}")
        }
    };
    for n in 2..100_000 {
        let cand = format!("{base}_{n}");
        if !existing.contains(&join(&cand)) {
            return cand;
        }
    }
    format!("{base}_copy")
}

fn fqn_prefix(d: &Document, id: ObjectId) -> String {
    let mut parts = Vec::new();
    let mut cur = d
        .get_dictionary(id)
        .ok()
        .and_then(|f| f.get(b"Parent").ok().and_then(|p| p.as_reference().ok()));
    for _ in 0..MAX_FIELD_DEPTH {
        let Some(p) = cur else { break };
        let Ok(pd) = d.get_dictionary(p) else { break };
        let t = text_of(d, pd.get(b"T").ok());
        if !t.is_empty() {
            parts.push(t);
        }
        cur = pd.get(b"Parent").ok().and_then(|x| x.as_reference().ok());
    }
    parts.reverse();
    parts.join(".")
}

const WIDGET_KEYS: [&[u8]; 10] = [
    b"Type", b"Subtype", b"Rect", b"P", b"AP", b"AS", b"MK", b"F", b"BS", b"Border",
];

/// State shared by all widget clones of one page duplication (so e.g. the two radio buttons of
/// one group become *one* new group, not two).
#[derive(Default)]
pub struct DupState {
    field_map: BTreeMap<ObjectId, ObjectId>,
    names: Option<BTreeSet<String>>,
}

/// Clone a single widget for a duplicated page according to `policy`. Returns the new
/// widget's id to place on the new page's `/Annots`, or `None` when flattening (the caller
/// bakes appearances separately).
pub fn duplicate_widget(
    tx: &mut Tx<'_>,
    widget: ObjectId,
    new_page: ObjectId,
    policy: FormPolicy,
    st: &mut DupState,
) -> Result<Option<ObjectId>> {
    let wd = tx.doc().get_dictionary(widget)?.clone();
    match policy {
        FormPolicy::Flatten => {
            // A plain widget annotation with no field: `flatten_page_widgets` bakes it afterwards.
            let mut nd = widget_only(&wd);
            nd.set("P", reference(new_page));
            Ok(Some(tx.add(Object::Dictionary(nd))))
        }
        FormPolicy::Independent => {
            if st.names.is_none() {
                st.names = Some(all_field_names(tx.doc()));
            }
            let existing = st.names.clone().unwrap_or_default();
            let prefix = fqn_prefix(tx.doc(), widget);
            let mut nd = wd.clone();
            nd.set("P", reference(new_page));
            // The widget is itself the terminal field (has /T) → new name.
            if wd.has(b"T") {
                let base = text_of(tx.doc(), wd.get(b"T").ok());
                let fresh = unique_partial_name(&existing, &prefix, &base);
                if let Some(n) = st.names.as_mut() {
                    n.insert(if prefix.is_empty() {
                        fresh.clone()
                    } else {
                        format!("{prefix}.{fresh}")
                    });
                }
                nd.set("T", text_obj(&fresh));
            } else if let Some(parent) = wd.get(b"Parent").ok().and_then(|p| p.as_reference().ok())
            {
                // Widget is a kid of a field: clone the *field* (new name, all widgets that
                // live on the source page) — handled once per field by the caller via dedupe.
                return clone_field_with_widget(tx, parent, widget, new_page, st).map(Some);
            }
            let new = tx.add(Object::Dictionary(nd));
            attach_field(tx, widget, new)?;
            Ok(Some(new))
        }
        FormPolicy::Linked => {
            if wd.has(b"T") || wd.has(b"FT") {
                // Merged field+widget: split into a field with two widget kids.
                let field = split_merged(tx, widget)?;
                let mut nw = widget_only(&wd);
                nw.set("P", reference(new_page));
                nw.set("Parent", reference(field));
                let new = tx.add(Object::Dictionary(nw));
                push_kid(tx, field, new)?;
                Ok(Some(new))
            } else if let Some(parent) = wd.get(b"Parent").ok().and_then(|p| p.as_reference().ok())
            {
                let mut nw = wd.clone();
                nw.set("P", reference(new_page));
                let new = tx.add(Object::Dictionary(nw));
                push_kid(tx, parent, new)?;
                Ok(Some(new))
            } else {
                Err(EngineError::Malformed(
                    "widget is not attached to a field".into(),
                ))
            }
        }
    }
}

fn widget_only(wd: &Dictionary) -> Dictionary {
    let mut d = Dictionary::new();
    for k in WIDGET_KEYS {
        if let Ok(v) = wd.get(k) {
            d.set(k.to_vec(), v.clone());
        }
    }
    d
}

/// Turn a merged field+widget into a field with one kid widget (original widget id keeps its
/// place on its page; a new field object takes the field role).
fn split_merged(tx: &mut Tx<'_>, widget: ObjectId) -> Result<ObjectId> {
    let wd = tx.doc().get_dictionary(widget)?.clone();
    let mut field = Dictionary::new();
    for (k, v) in wd.iter() {
        if !WIDGET_KEYS.contains(&k.as_slice()) {
            field.set(k.clone(), v.clone());
        }
    }
    field.set("Kids", Object::Array(vec![reference(widget)]));
    let field_id = tx.add(Object::Dictionary(field));
    // Reduce the widget to widget-only keys + parent.
    let mut w = widget_only(&wd);
    w.set("Parent", reference(field_id));
    tx.set(widget, Object::Dictionary(w));
    // Re-point the field's parent (if any) and the AcroForm /Fields entry from widget to field.
    if let Some(p) = wd.get(b"Parent").ok().and_then(|p| p.as_reference().ok()) {
        let kids: Vec<Object> = objutil::dict_array(tx.doc(), tx.doc().get_dictionary(p)?, b"Kids")
            .map(|k| {
                k.iter()
                    .map(|o| {
                        if o.as_reference().ok() == Some(widget) {
                            reference(field_id)
                        } else {
                            o.clone()
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        tx.dict_mut(p)?.set("Kids", Object::Array(kids));
        tx.dict_mut(field_id)?.set("Parent", reference(p));
    } else {
        replace_in_fields(tx, widget, field_id)?;
    }
    Ok(field_id)
}

fn replace_in_fields(tx: &mut Tx<'_>, old: ObjectId, new: ObjectId) -> Result<()> {
    let cat = tx
        .doc()
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)?;
    let af_ref = tx
        .doc()
        .get_dictionary(cat)?
        .get(b"AcroForm")
        .ok()
        .and_then(|o| o.as_reference().ok());
    let swap = |fields: &Vec<Object>| -> Vec<Object> {
        fields
            .iter()
            .map(|o| {
                if o.as_reference().ok() == Some(old) {
                    reference(new)
                } else {
                    o.clone()
                }
            })
            .collect()
    };
    match af_ref {
        Some(r) => {
            if let Ok(Object::Array(f)) = tx.dict_mut(r)?.get(b"Fields").cloned() {
                tx.dict_mut(r)?.set("Fields", Object::Array(swap(&f)));
            }
        }
        None => {
            if let Ok(Object::Dictionary(af)) = tx.dict_mut(cat)?.get_mut(b"AcroForm")
                && let Ok(Object::Array(f)) = af.get(b"Fields").cloned()
            {
                af.set("Fields", Object::Array(swap(&f)));
            }
        }
    }
    Ok(())
}

fn push_kid(tx: &mut Tx<'_>, field: ObjectId, kid: ObjectId) -> Result<()> {
    let mut kids: Vec<Object> =
        objutil::dict_array(tx.doc(), tx.doc().get_dictionary(field)?, b"Kids")
            .cloned()
            .unwrap_or_default();
    kids.push(reference(kid));
    tx.dict_mut(field)?.set("Kids", Object::Array(kids));
    Ok(())
}

/// Register `new` next to `sibling` (same parent or AcroForm /Fields).
fn attach_field(tx: &mut Tx<'_>, sibling: ObjectId, new: ObjectId) -> Result<()> {
    if let Some(p) = tx
        .doc()
        .get_dictionary(sibling)?
        .get(b"Parent")
        .ok()
        .and_then(|p| p.as_reference().ok())
    {
        tx.dict_mut(new)?.set("Parent", reference(p));
        return push_kid(tx, p, new);
    }
    let cat = tx
        .doc()
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)?;
    let af_ref = tx
        .doc()
        .get_dictionary(cat)?
        .get(b"AcroForm")
        .ok()
        .and_then(|o| o.as_reference().ok());
    match af_ref {
        Some(r) => {
            let mut f: Vec<Object> = tx
                .doc()
                .get_dictionary(r)?
                .get(b"Fields")
                .ok()
                .and_then(|o| o.as_array().ok())
                .cloned()
                .unwrap_or_default();
            f.push(reference(new));
            tx.dict_mut(r)?.set("Fields", Object::Array(f));
        }
        None => {
            if let Ok(Object::Dictionary(af)) = tx.dict_mut(cat)?.get_mut(b"AcroForm") {
                let mut f: Vec<Object> = af
                    .get(b"Fields")
                    .ok()
                    .and_then(|o| o.as_array().ok())
                    .cloned()
                    .unwrap_or_default();
                f.push(reference(new));
                af.set("Fields", Object::Array(f));
            }
        }
    }
    Ok(())
}

/// A widget that is a kid of an existing field: create a new field (new name, copied
/// configuration) with a cloned widget.
fn clone_field_with_widget(
    tx: &mut Tx<'_>,
    parent: ObjectId,
    widget: ObjectId,
    new_page: ObjectId,
    st: &mut DupState,
) -> Result<ObjectId> {
    let wd = tx.doc().get_dictionary(widget)?.clone();
    let nf_id = match st.field_map.get(&parent) {
        Some(f) => *f,
        None => {
            let pd = tx.doc().get_dictionary(parent)?.clone();
            let prefix = fqn_prefix(tx.doc(), parent);
            let base = text_of(tx.doc(), pd.get(b"T").ok());
            let existing = st
                .names
                .get_or_insert_with(|| all_field_names(tx.doc()))
                .clone();
            let mut nf = Dictionary::new();
            for (k, v) in pd.iter() {
                if k != b"Kids" && k != b"T" {
                    nf.set(k.clone(), v.clone());
                }
            }
            let fresh = unique_partial_name(&existing, &prefix, &base);
            if let Some(n) = st.names.as_mut() {
                n.insert(if prefix.is_empty() {
                    fresh.clone()
                } else {
                    format!("{prefix}.{fresh}")
                });
            }
            nf.set("T", text_obj(&fresh));
            nf.set("Kids", Object::Array(Vec::new()));
            let id = tx.add(Object::Dictionary(nf));
            attach_field(tx, parent, id)?;
            st.field_map.insert(parent, id);
            id
        }
    };
    let mut nw = wd.clone();
    nw.set("P", reference(new_page));
    nw.set("Parent", reference(nf_id));
    let nw_id = tx.add(Object::Dictionary(nw));
    push_kid(tx, nf_id, nw_id)?;
    Ok(nw_id)
}

// ------------------------------------------------------------------------------------------
// Import from another document
// ------------------------------------------------------------------------------------------

const FIELD_KEYS: [&[u8]; 10] = [
    b"FT", b"T", b"TU", b"V", b"DV", b"Ff", b"DA", b"Q", b"Opt", b"MaxLen",
];

/// The terminal field a widget belongs to (the widget itself when it is a merged field+widget).
fn terminal_field(d: &Document, widget: ObjectId) -> ObjectId {
    let mut cur = widget;
    for _ in 0..MAX_FIELD_DEPTH {
        let Ok(wd) = d.get_dictionary(cur) else {
            return widget;
        };
        if wd.has(b"T") {
            return cur;
        }
        match wd.get(b"Parent").ok().and_then(|p| p.as_reference().ok()) {
            Some(p) => cur = p,
            None => return widget,
        }
    }
    widget
}

fn source_fqn(d: &Document, field: ObjectId) -> String {
    let prefix = fqn_prefix(d, field);
    let t = d
        .get_dictionary(field)
        .ok()
        .map(|f| text_of(d, f.get(b"T").ok()))
        .unwrap_or_default();
    match (prefix.is_empty(), t.is_empty()) {
        (true, _) => t,
        (false, true) => prefix,
        (false, false) => format!("{prefix}.{t}"),
    }
}

/// Converter callbacks supplied by the page importer (deep-copy source objects into the
/// destination document with id remapping).
pub trait ImportConv {
    /// Deep-copy a referenced source object, returning a reference to the copy.
    fn import_ref(&mut self, tx: &mut Tx<'_>, id: ObjectId) -> Result<Object>;
    /// Convert a source object (rewriting references).
    fn convert(&mut self, tx: &mut Tx<'_>, obj: &Object) -> Result<Object>;
}

fn ensure_acroform(tx: &mut Tx<'_>, src: &Document, conv: &mut dyn ImportConv) -> Result<()> {
    let cat = tx
        .doc()
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)?;
    let existing = tx.doc().get_dictionary(cat)?.get(b"AcroForm").ok().cloned();
    let src_af = acroform(src).cloned();
    match existing {
        None => {
            let mut af = Dictionary::new();
            af.set("Fields", Object::Array(Vec::new()));
            if let Some(s) = &src_af {
                if let Ok(da) = s.get(b"DA") {
                    af.set("DA", da.clone());
                }
                if let Ok(dr) = s.get(b"DR") {
                    af.set("DR", conv.convert(tx, dr)?);
                }
            }
            tx.dict_mut(cat)?.set("AcroForm", Object::Dictionary(af));
        }
        Some(_) => {
            // Merge missing /DR font entries so imported default appearances still resolve.
            if let Some(s) = &src_af
                && let Some(sdr) = objutil::dict_dict(src, s, b"DR")
                && let Some(sf) = objutil::dict_dict(src, sdr, b"Font")
            {
                let af_ref = tx
                    .doc()
                    .get_dictionary(cat)?
                    .get(b"AcroForm")
                    .ok()
                    .and_then(|o| o.as_reference().ok());
                let mut af = match af_ref {
                    Some(r) => tx.doc().get_dictionary(r)?.clone(),
                    None => {
                        objutil::dict_dict(tx.doc(), tx.doc().get_dictionary(cat)?, b"AcroForm")
                            .cloned()
                            .unwrap_or_default()
                    }
                };
                let mut dr = objutil::dict_dict(tx.doc(), &af, b"DR")
                    .cloned()
                    .unwrap_or_default();
                let mut fonts = objutil::dict_dict(tx.doc(), &dr, b"Font")
                    .cloned()
                    .unwrap_or_default();
                let mut changed = false;
                for (k, v) in sf.iter() {
                    if !fonts.has(k) {
                        fonts.set(k.clone(), conv.convert(tx, v)?);
                        changed = true;
                    }
                }
                if changed {
                    dr.set("Font", Object::Dictionary(fonts));
                    af.set("DR", Object::Dictionary(dr));
                    match af_ref {
                        Some(r) => tx.set(r, Object::Dictionary(af)),
                        None => {
                            tx.dict_mut(cat)?.set("AcroForm", Object::Dictionary(af));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Import the widgets of one source page onto `new_page`. `Independent` creates new fields with
/// collision-free names; `Flatten` returns plain widget annotations for baking; `Linked` is not
/// meaningful across documents and is refused. Returns objects to add to the page's `/Annots`.
pub fn import_widgets(
    tx: &mut Tx<'_>,
    src: &Document,
    widgets: &[ObjectId],
    new_page: ObjectId,
    policy: FormPolicy,
    conv: &mut dyn ImportConv,
) -> Result<Vec<Object>> {
    if policy == FormPolicy::Linked {
        return Err(EngineError::Unsupported(
            "linking form fields across documents is not possible; choose Independent or Flatten"
                .into(),
        ));
    }
    let mut out = Vec::new();
    if policy == FormPolicy::Flatten {
        for w in widgets {
            let nw = conv.import_ref(tx, *w)?;
            if let Object::Reference(id) = nw {
                let mut d = widget_only(tx.doc().get_dictionary(id)?);
                d.set("P", reference(new_page));
                tx.set(id, Object::Dictionary(d));
                out.push(reference(id));
            }
        }
        return Ok(out);
    }
    ensure_acroform(tx, src, conv)?;
    let mut names = all_field_names(tx.doc());
    let mut groups: BTreeMap<ObjectId, Vec<ObjectId>> = BTreeMap::new();
    for w in widgets {
        groups.entry(terminal_field(src, *w)).or_default().push(*w);
    }
    for (field, ws) in groups {
        let fd = src.get_dictionary(field)?;
        // Field attributes (including inherited ones) become the new field's own.
        let mut attrs = Dictionary::new();
        for k in FIELD_KEYS {
            if let Some(v) = inherited_field(src, fd, k) {
                attrs.set(k.to_vec(), v.clone());
            }
        }
        let base = source_fqn(src, field).replace('.', "_");
        let base = if base.is_empty() {
            "field".to_string()
        } else {
            base
        };
        let fresh = if names.contains(&base) {
            unique_partial_name(&names, "", &base)
        } else {
            base
        };
        names.insert(fresh.clone());
        let mut nf = match conv.convert(tx, &Object::Dictionary(attrs))? {
            Object::Dictionary(d) => d,
            _ => Dictionary::new(),
        };
        nf.set("T", text_obj(&fresh));
        let fid = tx.add(Object::Dictionary(nf));
        let mut kids = Vec::new();
        for w in ws {
            let Object::Reference(nw) = conv.import_ref(tx, w)? else {
                continue;
            };
            let mut d = tx.doc().get_dictionary(nw)?.clone();
            for k in FIELD_KEYS {
                d.remove(k);
            }
            d.set("P", reference(new_page));
            d.set("Parent", reference(fid));
            tx.set(nw, Object::Dictionary(d));
            kids.push(reference(nw));
            out.push(reference(nw));
        }
        tx.dict_mut(fid)?.set("Kids", Object::Array(kids));
        register_field(tx, fid)?;
    }
    Ok(out)
}

fn register_field(tx: &mut Tx<'_>, field: ObjectId) -> Result<()> {
    let cat = tx
        .doc()
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)?;
    let af_ref = tx
        .doc()
        .get_dictionary(cat)?
        .get(b"AcroForm")
        .ok()
        .and_then(|o| o.as_reference().ok());
    match af_ref {
        Some(r) => {
            let mut f: Vec<Object> = tx
                .doc()
                .get_dictionary(r)?
                .get(b"Fields")
                .ok()
                .and_then(|o| o.as_array().ok())
                .cloned()
                .unwrap_or_default();
            f.push(reference(field));
            tx.dict_mut(r)?.set("Fields", Object::Array(f));
        }
        None => {
            if let Ok(Object::Dictionary(af)) = tx.dict_mut(cat)?.get_mut(b"AcroForm") {
                let mut f: Vec<Object> = af
                    .get(b"Fields")
                    .ok()
                    .and_then(|o| o.as_array().ok())
                    .cloned()
                    .unwrap_or_default();
                f.push(reference(field));
                af.set("Fields", Object::Array(f));
            }
        }
    }
    Ok(())
}

/// Widget annotations on a page (object ids).
pub fn page_widgets(d: &Document, page: ObjectId) -> Vec<ObjectId> {
    let Ok(pd) = d.get_dictionary(page) else {
        return Vec::new();
    };
    objutil::dict_array(d, pd, b"Annots")
        .map(|a| {
            a.iter()
                .filter_map(|o| o.as_reference().ok())
                .filter(|r| {
                    d.get_dictionary(*r)
                        .ok()
                        .is_some_and(|wd| objutil::dict_name(d, wd, b"Subtype") == Some(b"Widget"))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Flatten every widget in the document and remove the (now empty) AcroForm. Returns
/// `(baked, dropped)` totals as for [`flatten_page_widgets`].
pub fn flatten_document(tx: &mut Tx<'_>) -> Result<(usize, usize)> {
    let pages: Vec<PageId> = page_ids_of(tx.doc());
    let (mut baked, mut dropped) = (0, 0);
    for p in pages {
        if page_widgets(tx.doc(), p.0).is_empty() {
            continue;
        }
        let (b, d) = flatten_page_widgets(tx, p)?;
        baked += b;
        dropped += d;
    }
    // Remove the AcroForm only when no fields remain (unflattenable leftovers keep it).
    let remaining = {
        let d = tx.doc();
        acroform(d)
            .and_then(|af| objutil::dict_array(d, af, b"Fields"))
            .is_some_and(|f| !f.is_empty())
    };
    if !remaining {
        let cat = tx
            .doc()
            .trailer
            .get(b"Root")
            .and_then(Object::as_reference)?;
        tx.dict_mut(cat)?.remove(b"AcroForm");
    }
    Ok((baked, dropped))
}

fn page_ids_of(doc: &Document) -> Vec<PageId> {
    doc.get_pages().values().map(|id| PageId(*id)).collect()
}
