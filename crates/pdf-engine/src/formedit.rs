//! Making forms: create, change, move and delete AcroForm fields.
//!
//! New fields are ordinary AcroForm fields that every reader understands: a text field, check box,
//! radio button (buttons with the same name form one group), drop-down, list box, push button or
//! signature field. Each gets a generated appearance (drawn with the bundled DejaVu Sans, embedded
//! as a subset) and a `/DA` naming Helvetica from the form's `/DR`, so other readers that rebuild
//! appearances can do so too.
//!
//! Scripts are stored with [`crate::formscripts`]; nothing here runs them.

use crate::doc::{PageId, Tx};
use crate::error::{EngineError, Result};
use crate::fontembed::{BundledFace, FontBuilder, hex_string, zlib};
use crate::forms;
use crate::formscripts::{self, FieldScripts};
use crate::geom::Rect;
use crate::objutil::{self, fmt_num_prec, name, num_array, reference, text_obj};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};

/// Longest field name.
pub const MAX_NAME: usize = 120;
/// Most options of a drop-down or list.
pub const MAX_OPTIONS: usize = 500;

/// The kind of field to make.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewField {
    /// A text field.
    Text {
        /// Several lines.
        multiline: bool,
        /// Shown as dots.
        password: bool,
        /// Equally spaced cells (needs `max_len`).
        comb: bool,
        /// At most this many characters.
        max_len: Option<usize>,
    },
    /// A check box.
    Checkbox,
    /// A radio button.
    Radio,
    /// A drop-down list.
    Combo {
        /// People may also type their own value.
        editable: bool,
    },
    /// A list box.
    List {
        /// Several entries can be selected.
        multi: bool,
    },
    /// A push button.
    Button,
    /// An empty signature field.
    Signature,
}

/// Everything about a field that the properties dialog shows.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldProps {
    /// Fully qualified name; a radio button's name is its group's.
    pub name: String,
    /// Kind (fixed once created).
    pub kind: NewField,
    /// Tooltip (`/TU`).
    pub tooltip: String,
    /// Must be filled in.
    pub required: bool,
    /// Cannot be changed by people.
    pub read_only: bool,
    /// Not shown or printed.
    pub hidden: bool,
    /// Value: text, the chosen option's export value, or `Off`/the on-state for boxes.
    pub value: String,
    /// Options of a drop-down or list: `(export value, shown text)`.
    pub options: Vec<(String, String)>,
    /// The on-state name of a check box or radio button ("Yes", "Choice1", …).
    pub export: String,
    /// Caption of a button.
    pub caption: String,
    /// The button resets the form.
    pub reset_form: bool,
    /// Font size; `0` = fit the field.
    pub font_size: f64,
    /// Text colour.
    pub text_color: [f32; 3],
    /// Border colour (`None` = no border).
    pub border: Option<[f32; 3]>,
    /// Background colour (`None` = transparent).
    pub background: Option<[f32; 3]>,
    /// 0 left, 1 centre, 2 right.
    pub align: i64,
    /// Scripts.
    pub scripts: FieldScripts,
}

impl FieldProps {
    /// Defaults for a new field of `kind` named `name`.
    pub fn new(kind: NewField, name: &str) -> Self {
        let export = match kind {
            NewField::Radio => "Choice1".to_string(),
            _ => "Yes".to_string(),
        };
        Self {
            name: name.to_string(),
            kind: kind.clone(),
            tooltip: String::new(),
            required: false,
            read_only: false,
            hidden: false,
            value: String::new(),
            options: Vec::new(),
            export,
            caption: if kind == NewField::Button {
                "Button".into()
            } else {
                String::new()
            },
            reset_form: false,
            font_size: 0.0,
            text_color: [0.0, 0.0, 0.0],
            border: Some([0.45, 0.45, 0.45]),
            background: match kind {
                NewField::Button => Some([0.86, 0.88, 0.92]),
                _ => Some([0.95, 0.97, 1.0]),
            },
            align: 0,
            scripts: FieldScripts::default(),
        }
    }

    fn ff(&self) -> i64 {
        let mut ff = 0;
        if self.read_only {
            ff |= 1;
        }
        if self.required {
            ff |= 2;
        }
        match &self.kind {
            NewField::Text {
                multiline,
                password,
                comb,
                ..
            } => {
                if *multiline {
                    ff |= 1 << 12;
                }
                if *password {
                    ff |= 1 << 13;
                }
                if *comb {
                    ff |= 1 << 24;
                }
            }
            NewField::Radio => ff |= (1 << 15) | (1 << 14),
            NewField::Button => ff |= 1 << 16,
            NewField::Combo { editable } => {
                ff |= 1 << 17;
                if *editable {
                    ff |= 1 << 18;
                }
            }
            NewField::List { multi } => {
                if *multi {
                    ff |= 1 << 21;
                }
            }
            NewField::Checkbox | NewField::Signature => {}
        }
        ff
    }

    fn ft(&self) -> &'static str {
        match self.kind {
            NewField::Text { .. } => "Tx",
            NewField::Checkbox | NewField::Radio | NewField::Button => "Btn",
            NewField::Combo { .. } | NewField::List { .. } => "Ch",
            NewField::Signature => "Sig",
        }
    }

    fn da(&self) -> String {
        let c = self.text_color;
        format!(
            "/Helv {} Tf {} {} {} rg",
            fmt_num_prec(self.font_size),
            fmt_num_prec(f64::from(c[0])),
            fmt_num_prec(f64::from(c[1])),
            fmt_num_prec(f64::from(c[2]))
        )
    }

    fn mk(&self) -> Dictionary {
        let mut mk = Dictionary::new();
        let arr = |c: [f32; 3]| num_array(&c.map(f64::from));
        if let Some(b) = self.border {
            mk.set("BC", arr(b));
        }
        if let Some(b) = self.background {
            mk.set("BG", arr(b));
        }
        match self.kind {
            NewField::Checkbox => mk.set("CA", text_obj("4")),
            NewField::Radio => mk.set("CA", text_obj("l")),
            NewField::Button => mk.set("CA", text_obj(&self.caption)),
            _ => {}
        }
        mk
    }
}

/// A name no field has yet: `base`, or `base_2`, `base_3`, …
pub fn unique_field_name(doc: &Document, base: &str) -> String {
    let names = forms::all_field_names(doc);
    let base = clean_name(base);
    if !names.contains(&base) {
        return base;
    }
    (2..100_000)
        .map(|n| format!("{base}_{n}"))
        .find(|n| !names.contains(n))
        .unwrap_or_else(|| format!("{base}_new"))
}

fn clean_name(s: &str) -> String {
    let s: String = s
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| if c == '.' { '_' } else { c })
        .take(MAX_NAME)
        .collect();
    if s.is_empty() { "Field".into() } else { s }
}

fn check(props: &FieldProps) -> Result<()> {
    if props.options.len() > MAX_OPTIONS {
        return Err(EngineError::LimitExceeded("too many options".into()));
    }
    if !(props.font_size == 0.0 || (2.0..=144.0).contains(&props.font_size)) {
        return Err(EngineError::InvalidArgument(
            "the font size must be between 2 and 144 points (or automatic)".into(),
        ));
    }
    if matches!(props.kind, NewField::Checkbox | NewField::Radio) {
        let e = props.export.trim();
        if e.is_empty()
            || e == "Off"
            || e.contains(['/', ' ', '(', ')', '<', '>', '[', ']', '%', '#'])
        {
            return Err(EngineError::InvalidArgument(
                "the export value must be a single word other than “Off”".into(),
            ));
        }
    }
    Ok(())
}

fn add_annot_to_page(tx: &mut Tx<'_>, page: ObjectId, annot: ObjectId) -> Result<()> {
    let mut annots: Vec<Object> =
        objutil::dict_array(tx.doc(), tx.doc().get_dictionary(page)?, b"Annots")
            .cloned()
            .unwrap_or_default();
    annots.push(reference(annot));
    tx.dict_mut(page)?.set("Annots", Object::Array(annots));
    Ok(())
}

fn remove_annot_from_page(tx: &mut Tx<'_>, page: ObjectId, annot: ObjectId) -> Result<()> {
    let Some(annots) =
        objutil::dict_array(tx.doc(), tx.doc().get_dictionary(page)?, b"Annots").cloned()
    else {
        return Ok(());
    };
    let keep: Vec<Object> = annots
        .into_iter()
        .filter(|o| o.as_reference().ok() != Some(annot))
        .collect();
    tx.dict_mut(page)?.set("Annots", Object::Array(keep));
    Ok(())
}

/// The field of a radio group named `name`, if there is one.
fn radio_group(doc: &crate::doc::PdfDocument, name: &str) -> Option<ObjectId> {
    forms::read_form(doc)
        .fields
        .into_iter()
        .find(|f| f.name == name && f.kind == forms::FieldKind::Radio)
        .map(|f| f.id)
}

fn widget_flags(props: &FieldProps) -> i64 {
    // Print, and hidden when asked.
    if props.hidden { 4 | 2 } else { 4 }
}

/// Make a new field with one widget on `page` in `rect`. A radio button whose name is an existing
/// radio group joins that group. Returns the widget.
pub fn create_field(
    tx: &mut Tx<'_>,
    doc_view: &crate::doc::PdfDocument,
    page: PageId,
    rect: Rect,
    props: &FieldProps,
) -> Result<ObjectId> {
    check(props)?;
    let rect = rect.abs();
    if rect.width() < 4.0 || rect.height() < 4.0 {
        return Err(EngineError::InvalidArgument(
            "the field is too small".into(),
        ));
    }
    formscripts::ensure_acroform(tx)?;
    let name = clean_name(&props.name);
    let mut widget = dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "Rect" => num_array(&[rect.x0, rect.y0, rect.x1, rect.y1]),
        "P" => reference(page.0),
        "F" => widget_flags(props),
        "MK" => Object::Dictionary(props.mk()),
        "BS" => dictionary! { "W" => 1i64, "S" => "S" },
    };
    if props.kind == NewField::Radio {
        let group = match radio_group(doc_view, &name) {
            Some(g) => g,
            None => {
                let g = tx.add(Object::Dictionary(dictionary! {
                    "FT" => "Btn",
                    "Ff" => props.ff(),
                    "T" => text_obj(&name),
                    "V" => name_obj("Off"),
                    "DA" => text_obj(&props.da()),
                    "Kids" => Object::Array(Vec::new()),
                }));
                if !props.tooltip.is_empty() {
                    tx.dict_mut(g)?.set("TU", text_obj(&props.tooltip));
                }
                forms::register_field(tx, g)?;
                g
            }
        };
        widget.set("Parent", reference(group));
        widget.set("AS", name_obj("Off"));
        let w = tx.add(Object::Dictionary(widget));
        let mut kids: Vec<Object> =
            objutil::dict_array(tx.doc(), tx.doc().get_dictionary(group)?, b"Kids")
                .cloned()
                .unwrap_or_default();
        kids.push(reference(w));
        tx.dict_mut(group)?.set("Kids", Object::Array(kids));
        add_annot_to_page(tx, page.0, w)?;
        draw_widget(tx, group, w, props)?;
        if props.value == props.export {
            forms::set_radio(tx, group, &props.export)?;
        }
        return Ok(w);
    }
    if forms::all_field_names(tx.doc()).contains(&name) {
        return Err(EngineError::InvalidArgument(format!(
            "a field named “{name}” already exists"
        )));
    }
    widget.set("FT", Object::Name(props.ft().as_bytes().to_vec()));
    widget.set("T", text_obj(&name));
    widget.set("Ff", props.ff());
    let w = tx.add(Object::Dictionary(widget));
    write_attributes(tx, w, w, props, true)?;
    forms::register_field(tx, w)?;
    add_annot_to_page(tx, page.0, w)?;
    draw_widget(tx, w, w, props)?;
    Ok(w)
}

fn name_obj(s: &str) -> Object {
    name(s)
}

/// Write everything but the name and kind onto the field (and, for a merged field, its widget).
fn write_attributes(
    tx: &mut Tx<'_>,
    field: ObjectId,
    widget: ObjectId,
    props: &FieldProps,
    new: bool,
) -> Result<()> {
    let fd = tx.dict_mut(field)?;
    fd.set("DA", text_obj(&props.da()));
    fd.set("Q", props.align.clamp(0, 2));
    if props.tooltip.trim().is_empty() {
        fd.remove(b"TU");
    } else {
        fd.set("TU", text_obj(props.tooltip.trim()));
    }
    let old_ff = fd
        .get(b"Ff")
        .ok()
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(0);
    // Keep flags this dialog does not show (do-not-spell-check, rich text, …).
    let shown = 1 | 2 | (1 << 12) | (1 << 13) | (1 << 24) | (1 << 18) | (1 << 21);
    fd.set("Ff", (old_ff & !shown) | props.ff());
    match &props.kind {
        NewField::Text { max_len, .. } => {
            match max_len.filter(|n| *n > 0) {
                Some(n) => fd.set("MaxLen", n as i64),
                None => {
                    fd.remove(b"MaxLen");
                }
            }
            fd.set("V", text_obj(&props.value));
            if new {
                fd.set("DV", text_obj(&props.value));
            }
        }
        NewField::Combo { .. } | NewField::List { .. } => {
            let opts: Vec<Object> = props
                .options
                .iter()
                .map(|(e, d)| {
                    if e == d {
                        text_obj(d)
                    } else {
                        Object::Array(vec![text_obj(e), text_obj(d)])
                    }
                })
                .collect();
            fd.set("Opt", Object::Array(opts));
            fd.set("V", text_obj(&props.value));
            if new {
                fd.set("DV", text_obj(&props.value));
            }
        }
        NewField::Checkbox => {
            let on = props.value == props.export || props.value == "Yes" && props.export == "Yes";
            fd.set("V", name_obj(if on { &props.export } else { "Off" }));
            if new {
                fd.set("DV", name_obj("Off"));
            }
        }
        NewField::Button => {
            if props.reset_form {
                tx.dict_mut(widget)?
                    .set("A", dictionary! { "Type" => "Action", "S" => "ResetForm" });
            } else if tx
                .doc()
                .get_dictionary(widget)?
                .get(b"A")
                .ok()
                .and_then(|a| a.as_dict().ok())
                .and_then(|d| d.get(b"S").ok())
                .and_then(|s| s.as_name().ok())
                == Some(b"ResetForm")
            {
                tx.dict_mut(widget)?.remove(b"A");
            }
        }
        NewField::Radio | NewField::Signature => {}
    }
    let wd = tx.dict_mut(widget)?;
    wd.set("MK", Object::Dictionary(props.mk()));
    let flags = wd.get(b"F").ok().and_then(|o| o.as_i64().ok()).unwrap_or(4);
    wd.set("F", (flags & !2) | widget_flags(props));
    formscripts::set_field_scripts(tx, field, &props.scripts)
}

// ------------------------------------------------------------------------------------------
// Appearances
// ------------------------------------------------------------------------------------------

fn rgb(c: [f32; 3]) -> String {
    format!(
        "{} {} {}",
        fmt_num_prec(f64::from(c[0])),
        fmt_num_prec(f64::from(c[1])),
        fmt_num_prec(f64::from(c[2]))
    )
}

fn form_xobject(tx: &mut Tx<'_>, w: f64, h: f64, content: &str, resources: Dictionary) -> ObjectId {
    let mut s = Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "FormType" => 1i64,
            "BBox" => num_array(&[0.0, 0.0, w, h]),
            "Resources" => Object::Dictionary(resources),
            "Filter" => "FlateDecode",
        },
        zlib(content.as_bytes()),
    );
    s.allows_compression = false;
    tx.add(Object::Stream(s))
}

fn frame(props: &FieldProps, w: f64, h: f64, round: bool) -> String {
    let mut c = String::new();
    if round {
        let r = w.min(h) / 2.0 - 0.5;
        let (cx, cy) = (w / 2.0, h / 2.0);
        let k = 0.5523 * r;
        let circle = format!(
            "{} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c h",
            cx + r,
            cy,
            cx + r,
            cy + k,
            cx + k,
            cy + r,
            cx,
            cy + r,
            cx - k,
            cy + r,
            cx - r,
            cy + k,
            cx - r,
            cy,
            cx - r,
            cy - k,
            cx - k,
            cy - r,
            cx,
            cy - r,
            cx + k,
            cy - r,
            cx + r,
            cy - k,
            cx + r,
            cy
        );
        if let Some(bg) = props.background {
            c.push_str(&format!("{} rg {circle} f\n", rgb(bg)));
        }
        if let Some(bc) = props.border {
            c.push_str(&format!("{} RG 1 w {circle} S\n", rgb(bc)));
        }
    } else {
        if let Some(bg) = props.background {
            c.push_str(&format!(
                "{} rg 0 0 {} {} re f\n",
                rgb(bg),
                fmt_num_prec(w),
                fmt_num_prec(h)
            ));
        }
        if let Some(bc) = props.border {
            c.push_str(&format!(
                "{} RG 1 w 0.5 0.5 {} {} re S\n",
                rgb(bc),
                fmt_num_prec(w - 1.0),
                fmt_num_prec(h - 1.0)
            ));
        }
    }
    c
}

fn centred_text(
    fb: &mut FontBuilder,
    text: &str,
    w: f64,
    h: f64,
    size: f64,
    color: [f32; 3],
) -> Result<String> {
    let size = if size > 0.0 {
        size
    } else {
        (h * 0.55).clamp(4.0, 14.0)
    };
    let tw = fb.text_width(text, size);
    let x = (w - tw) / 2.0;
    let y = (h - size * (fb.ascent_em() + fb.descent_em())) / 2.0 + size * fb.descent_em();
    let codes = fb.encode_str(text)?;
    Ok(format!(
        "BT /F1 {} Tf {} rg {} {} Td {} Tj ET\n",
        fmt_num_prec(size),
        rgb(color),
        fmt_num_prec(x),
        fmt_num_prec(y),
        hex_string(&codes)
    ))
}

/// Draw (again) the appearance of one widget of a field.
fn draw_widget(
    tx: &mut Tx<'_>,
    field: ObjectId,
    widget: ObjectId,
    props: &FieldProps,
) -> Result<()> {
    let rect = tx
        .doc()
        .get_dictionary(widget)?
        .get(b"Rect")
        .ok()
        .and_then(|r| objutil::rect(tx.doc(), r))
        .ok_or_else(|| EngineError::Malformed("widget without /Rect".into()))?
        .abs();
    let (w, h) = (rect.width(), rect.height());
    match &props.kind {
        NewField::Text { .. } | NewField::Combo { .. } => {
            let ff = props.ff();
            let display = match props.kind {
                NewField::Combo { .. } => props
                    .options
                    .iter()
                    .find(|(e, _)| *e == props.value)
                    .map_or(props.value.clone(), |(_, d)| d.clone()),
                _ => props.value.clone(),
            };
            forms::regenerate_text_appearance(tx, field, widget, &display, ff)
        }
        NewField::List { .. } => {
            let lines: Vec<String> = props
                .options
                .iter()
                .map(|(e, d)| {
                    if *e == props.value {
                        format!("▸ {d}")
                    } else {
                        d.clone()
                    }
                })
                .collect();
            let ff = props.ff() | (1 << 12);
            forms::regenerate_text_appearance(tx, field, widget, &lines.join("\n"), ff)
        }
        NewField::Checkbox | NewField::Radio => {
            let round = props.kind == NewField::Radio;
            let off = frame(props, w, h, round);
            let s = w.min(h);
            let (ox, oy) = ((w - s) / 2.0, (h - s) / 2.0);
            let mark = if round {
                let r = s * 0.22;
                let (cx, cy) = (w / 2.0, h / 2.0);
                let k = 0.5523 * r;
                format!(
                    "{} rg {} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c f\n",
                    rgb(props.text_color),
                    cx + r,
                    cy,
                    cx + r,
                    cy + k,
                    cx + k,
                    cy + r,
                    cx,
                    cy + r,
                    cx - k,
                    cy + r,
                    cx - r,
                    cy + k,
                    cx - r,
                    cy,
                    cx - r,
                    cy - k,
                    cx - k,
                    cy - r,
                    cx,
                    cy - r,
                    cx + k,
                    cy - r,
                    cx + r,
                    cy - k,
                    cx + r,
                    cy
                )
            } else {
                format!(
                    "{} RG {} w 1 J 1 j {} {} m {} {} l {} {} l S\n",
                    rgb(props.text_color),
                    fmt_num_prec((s * 0.12).max(1.0)),
                    fmt_num_prec(ox + s * 0.22),
                    fmt_num_prec(oy + s * 0.52),
                    fmt_num_prec(ox + s * 0.42),
                    fmt_num_prec(oy + s * 0.28),
                    fmt_num_prec(ox + s * 0.78),
                    fmt_num_prec(oy + s * 0.74)
                )
            };
            let on_id = form_xobject(tx, w, h, &format!("{off}{mark}"), Dictionary::new());
            let off_id = form_xobject(tx, w, h, &off, Dictionary::new());
            let export = props.export.trim().to_string();
            let mut n = Dictionary::new();
            n.set(export.as_bytes().to_vec(), reference(on_id));
            n.set("Off", reference(off_id));
            let wd = tx.dict_mut(widget)?;
            wd.set("AP", dictionary! { "N" => Object::Dictionary(n) });
            let on = if round {
                props.value == export
            } else {
                props.value == export || props.value == "Yes" && export == "Yes"
            };
            if !round || !wd.has(b"AS") {
                wd.set("AS", name_obj(if on { &export } else { "Off" }));
            }
            Ok(())
        }
        NewField::Button | NewField::Signature => {
            let mut c = frame(props, w, h, false);
            let mut resources = Dictionary::new();
            if props.kind == NewField::Button && !props.caption.trim().is_empty() {
                let mut fb = FontBuilder::new(BundledFace::Sans)?;
                let missing = fb.missing_chars(&props.caption);
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
                c.push_str(&centred_text(
                    &mut fb,
                    props.caption.trim(),
                    w,
                    h,
                    props.font_size,
                    props.text_color,
                )?);
                let font = fb.finish(tx)?;
                resources.set("Font", dictionary! { "F1" => font });
            }
            let id = form_xobject(tx, w, h, &c, resources);
            tx.dict_mut(widget)?.set("AP", dictionary! { "N" => id });
            Ok(())
        }
    }
}

// ------------------------------------------------------------------------------------------
// Reading, changing, moving, deleting
// ------------------------------------------------------------------------------------------

fn color3(doc: &Document, d: Option<&Dictionary>, key: &[u8]) -> Option<[f32; 3]> {
    let a = d.and_then(|d| objutil::dict_array(doc, d, key))?;
    let v: Vec<f32> = a
        .iter()
        .filter_map(|o| objutil::num(doc, o))
        .map(|x| x as f32)
        .collect();
    match v.len() {
        1 => Some([v[0]; 3]),
        3 => Some([v[0], v[1], v[2]]),
        _ => None,
    }
}

/// The properties of `field` as seen through `widget`.
pub fn read_props(
    doc: &crate::doc::PdfDocument,
    field: ObjectId,
    widget: ObjectId,
) -> Option<FieldProps> {
    let d = doc.lopdf();
    let info = forms::read_form(doc);
    let f = info.fields.iter().find(|f| f.id == field)?;
    let fd = d.get_dictionary(field).ok()?;
    let wd = d.get_dictionary(widget).ok()?;
    let ff = forms::inherited_field(d, fd, b"Ff")
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(0);
    let kind = match &f.kind {
        forms::FieldKind::Text {
            multiline,
            password,
            comb,
            max_len,
        } => NewField::Text {
            multiline: *multiline,
            password: *password,
            comb: *comb,
            max_len: *max_len,
        },
        forms::FieldKind::Checkbox => NewField::Checkbox,
        forms::FieldKind::Radio => NewField::Radio,
        forms::FieldKind::Choice { combo: true, .. } => NewField::Combo {
            editable: ff & (1 << 18) != 0,
        },
        forms::FieldKind::Choice { combo: false, .. } => NewField::List {
            multi: ff & (1 << 21) != 0,
        },
        forms::FieldKind::PushButton => NewField::Button,
        forms::FieldKind::Signature => NewField::Signature,
        forms::FieldKind::Unknown => return None,
    };
    let options = match &f.kind {
        forms::FieldKind::Choice { options, .. } => options.clone(),
        _ => Vec::new(),
    };
    let da = forms::text_of(d, forms::inherited_field(d, fd, b"DA"));
    let mut size = 0.0;
    let mut color = [0.0f32; 3];
    let t: Vec<&str> = da.split_whitespace().collect();
    for (i, w) in t.iter().enumerate() {
        match *w {
            "Tf" if i >= 1 => size = t[i - 1].parse().unwrap_or(0.0),
            "g" if i >= 1 => color = [t[i - 1].parse().unwrap_or(0.0); 3],
            "rg" if i >= 3 => {
                let p = |s: &str| s.parse::<f32>().unwrap_or(0.0);
                color = [p(t[i - 3]), p(t[i - 2]), p(t[i - 1])];
            }
            _ => {}
        }
    }
    let mk = objutil::dict_dict(d, wd, b"MK");
    let reset_form = wd
        .get(b"A")
        .ok()
        .and_then(|a| objutil::deref(d, a))
        .and_then(|a| a.as_dict().ok())
        .is_some_and(|a| objutil::dict_name(d, a, b"S") == Some(b"ResetForm"));
    Some(FieldProps {
        name: f.name.clone(),
        kind,
        tooltip: forms::text_of(d, forms::inherited_field(d, fd, b"TU")),
        required: f.required,
        read_only: f.read_only,
        hidden: objutil::dict_num(d, wd, b"F").is_some_and(|fl| (fl as i64) & 2 != 0),
        value: f.value.clone(),
        options,
        export: forms::on_state_of(d, wd).unwrap_or_else(|| "Yes".into()),
        caption: mk
            .map(|m| forms::text_of(d, m.get(b"CA").ok()))
            .unwrap_or_default(),
        reset_form,
        font_size: size,
        text_color: color,
        border: color3(d, mk, b"BC"),
        background: color3(d, mk, b"BG"),
        align: forms::inherited_field(d, fd, b"Q")
            .and_then(|o| o.as_i64().ok())
            .unwrap_or(0),
        scripts: formscripts::field_scripts(d, field),
    })
}

/// Change a field: name (the last part of it), flags, value, options, look and scripts. Every
/// widget of the field is drawn again.
pub fn update_field(
    tx: &mut Tx<'_>,
    field: ObjectId,
    widget: ObjectId,
    props: &FieldProps,
) -> Result<()> {
    check(props)?;
    let current = forms::all_field_names(tx.doc());
    let old_full = {
        let d = tx.doc();
        // The qualified name of this field: parent names + its own.
        let mut parts = Vec::new();
        let mut cur = Some(field);
        for _ in 0..32 {
            let Some(id) = cur else { break };
            let Ok(fd) = d.get_dictionary(id) else { break };
            let t = forms::text_of(d, fd.get(b"T").ok());
            if !t.is_empty() {
                parts.push(t);
            }
            cur = fd.get(b"Parent").ok().and_then(|p| p.as_reference().ok());
        }
        parts.reverse();
        parts.join(".")
    };
    let wanted = props.name.trim();
    if wanted != old_full {
        // Only the last part can change here; a dot would move the field into another group.
        let last = clean_name(wanted.rsplit('.').next().unwrap_or(wanted));
        let prefix = old_full.rsplit_once('.').map(|(p, _)| p.to_string());
        let full = match &prefix {
            Some(p) => format!("{p}.{last}"),
            None => last.clone(),
        };
        if full != old_full && current.contains(&full) {
            return Err(EngineError::InvalidArgument(format!(
                "a field named “{full}” already exists"
            )));
        }
        tx.dict_mut(field)?.set("T", text_obj(&last));
    }
    if props.kind == NewField::Radio {
        // The group's flags and look; this button's export value.
        let fd = tx.dict_mut(field)?;
        fd.set("DA", text_obj(&props.da()));
        let old_ff = fd
            .get(b"Ff")
            .ok()
            .and_then(|o| o.as_i64().ok())
            .unwrap_or(0);
        fd.set("Ff", (old_ff & !3) | props.ff());
        if props.tooltip.trim().is_empty() {
            fd.remove(b"TU");
        } else {
            fd.set("TU", text_obj(props.tooltip.trim()));
        }
        let wd = tx.dict_mut(widget)?;
        wd.set("MK", Object::Dictionary(props.mk()));
        let flags = wd.get(b"F").ok().and_then(|o| o.as_i64().ok()).unwrap_or(4);
        wd.set("F", (flags & !2) | widget_flags(props));
        let was_on = wd
            .get(b"AS")
            .ok()
            .and_then(|o| o.as_name().ok())
            .is_some_and(|n| n != b"Off");
        draw_widget(tx, field, widget, props)?;
        if was_on {
            tx.dict_mut(widget)?
                .set("AS", name_obj(props.export.trim()));
            tx.dict_mut(field)?.set("V", name_obj(props.export.trim()));
        }
        return formscripts::set_field_scripts(tx, field, &props.scripts);
    }
    write_attributes(tx, field, widget, props, false)?;
    for w in forms::widgets_of(tx.doc(), field) {
        draw_widget(tx, field, w, props)?;
    }
    Ok(())
}

/// Move or resize a widget, and draw it again at its new size.
pub fn move_widget(
    tx: &mut Tx<'_>,
    doc_view: &crate::doc::PdfDocument,
    widget: ObjectId,
    rect: Rect,
) -> Result<()> {
    let rect = rect.abs();
    if rect.width() < 4.0 || rect.height() < 4.0 {
        return Err(EngineError::InvalidArgument(
            "the field is too small".into(),
        ));
    }
    let field = forms::terminal_field(tx.doc(), widget);
    let props = read_props(doc_view, field, widget)
        .ok_or_else(|| EngineError::Unsupported("this field cannot be changed".into()))?;
    tx.dict_mut(widget)?
        .set("Rect", num_array(&[rect.x0, rect.y0, rect.x1, rect.y1]));
    let was = tx.doc().get_dictionary(widget)?.get(b"AS").ok().cloned();
    draw_widget(tx, field, widget, &props)?;
    if let Some(a) = was {
        tx.dict_mut(widget)?.set("AS", a);
    }
    Ok(())
}

/// Delete a widget; the field goes when it was its last widget.
pub fn delete_widget(tx: &mut Tx<'_>, widget: ObjectId) -> Result<()> {
    let field = forms::terminal_field(tx.doc(), widget);
    let page = tx
        .doc()
        .get_dictionary(widget)?
        .get(b"P")
        .ok()
        .and_then(|p| p.as_reference().ok());
    let pages: Vec<ObjectId> = match page {
        Some(p) => vec![p],
        None => crate::pageops::page_order(tx.doc())?
            .into_iter()
            .map(|p| p.0)
            .collect(),
    };
    for p in pages {
        remove_annot_from_page(tx, p, widget)?;
    }
    if field == widget {
        formscripts::forget_field(tx, field)?;
        forms::detach_field(tx, field)?;
        return Ok(());
    }
    let kids: Vec<Object> = objutil::dict_array(tx.doc(), tx.doc().get_dictionary(field)?, b"Kids")
        .map(|k| {
            k.iter()
                .filter(|o| o.as_reference().ok() != Some(widget))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    if kids.is_empty() {
        formscripts::forget_field(tx, field)?;
        forms::detach_field(tx, field)?;
    } else {
        tx.dict_mut(field)?.set("Kids", Object::Array(kids));
    }
    Ok(())
}
