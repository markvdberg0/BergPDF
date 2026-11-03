//! Making forms (Forms ▸ Prepare): drawing new fields, selecting, moving, resizing and deleting them,
//! and the field properties dialog with its calculation, format and validation builders.
//!
//! The builders write the standard Acrobat functions (`AFSimple_Calculate`, `AFNumber_Format`,
//! `AFRange_Validate`, …) that every form-aware reader knows; anything else is a custom script. All
//! changes go through `pdf_engine::formedit` as undoable steps.

use crate::canvas::ViewCtx;
use crate::dialogs::modal;
use crate::i18n::tr;
use crate::state::*;
use crate::tf;
use editor_core::tools::Tool;
use egui::{Color32, Pos2, RichText, Stroke};
use pdf_engine::ObjectId;
use pdf_engine::doc::PageId;
use pdf_engine::formedit::{self, FieldProps, NewField};
use pdf_engine::geom::{Point, Rect as PRect};

/// What a field's value is calculated from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalcMode {
    None,
    Sum,
    Product,
    Average,
    Min,
    Max,
    Custom,
}

/// How a field's value is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatMode {
    None,
    Number,
    Percent,
    Date,
    Custom,
}

/// What a field accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidateMode {
    None,
    Range,
    Custom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropsTab {
    General,
    Options,
    Calculate,
    Actions,
}

/// The field properties dialog: for a new field (`target` = `None`) or an existing one.
pub struct FieldPropsState {
    pub target: Option<(ObjectId, ObjectId)>,
    pub page: PageId,
    pub rect: PRect,
    pub props: FieldProps,
    tab: PropsTab,
    options_text: String,
    calc: CalcMode,
    calc_fields: String,
    format: FormatMode,
    decimals: u32,
    currency: String,
    date_format: String,
    validate: ValidateMode,
    range_min: String,
    range_max: String,
    error: Option<String>,
}

/// The field being dragged in the Edit Fields tool.
#[derive(Clone, Debug)]
pub struct FieldDrag {
    pub widget: ObjectId,
    pub page: PageId,
    pub start: Point,
    pub rect: PRect,
    /// Dragging the bottom-right corner (resize) rather than the field (move).
    pub resize: bool,
    pub now: Point,
}

const SIZES: [(Tool, f64, f64); 7] = [
    (Tool::FieldText, 160.0, 22.0),
    (Tool::FieldCheckbox, 14.0, 14.0),
    (Tool::FieldRadio, 14.0, 14.0),
    (Tool::FieldCombo, 160.0, 22.0),
    (Tool::FieldList, 160.0, 64.0),
    (Tool::FieldButton, 90.0, 24.0),
    (Tool::FieldSignature, 180.0, 50.0),
];

fn kind_for(tool: Tool) -> Option<(NewField, &'static str)> {
    Some(match tool {
        Tool::FieldText => (
            NewField::Text {
                multiline: false,
                password: false,
                comb: false,
                max_len: None,
            },
            "Text",
        ),
        Tool::FieldCheckbox => (NewField::Checkbox, "CheckBox"),
        Tool::FieldRadio => (NewField::Radio, "Group"),
        Tool::FieldCombo => (NewField::Combo { editable: false }, "Dropdown"),
        Tool::FieldList => (NewField::List { multi: false }, "List"),
        Tool::FieldButton => (NewField::Button, "Button"),
        Tool::FieldSignature => (NewField::Signature, "Signature"),
        _ => return None,
    })
}

/// `"a", "b"` from `a, b`.
fn quoted_list(s: &str) -> String {
    s.split([',', ';', '\n'])
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(|n| format!("\"{}\"", n.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The names in `AFSimple_Calculate("SUM", new Array("a", "b"))` or `… ["a", "b"])`.
fn parse_simple_calc(script: &str) -> Option<(CalcMode, String)> {
    let s = script.trim();
    let rest = s.strip_prefix("AFSimple_Calculate(")?;
    let (op, rest) = rest.split_once(',')?;
    let mode = match op.trim().trim_matches(['"', '\'']) {
        "SUM" => CalcMode::Sum,
        "PRD" => CalcMode::Product,
        "AVG" => CalcMode::Average,
        "MIN" => CalcMode::Min,
        "MAX" => CalcMode::Max,
        _ => return None,
    };
    let names: Vec<String> = rest
        .split('"')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, n)| n.to_string())
        .collect();
    Some((mode, names.join(", ")))
}

fn parse_number_format(script: &str) -> Option<(FormatMode, u32, String)> {
    let s = script.trim();
    let (mode, rest) = match s.strip_prefix("AFNumber_Format(") {
        Some(r) => (FormatMode::Number, r),
        None => (FormatMode::Percent, s.strip_prefix("AFPercent_Format(")?),
    };
    let dec: u32 = rest.split(',').next()?.trim().parse().ok()?;
    let currency = rest
        .split('"')
        .nth(1)
        .map(str::to_string)
        .unwrap_or_default();
    Some((mode, dec, currency))
}

fn parse_date_format(script: &str) -> Option<String> {
    let rest = script.trim().strip_prefix("AFDate_FormatEx(")?;
    rest.split('"').nth(1).map(str::to_string)
}

fn parse_range(script: &str) -> Option<(String, String)> {
    let rest = script.trim().strip_prefix("AFRange_Validate(")?;
    let parts: Vec<&str> = rest
        .trim_end_matches([')', ';'])
        .split(',')
        .map(str::trim)
        .collect();
    if parts.len() != 4 {
        return None;
    }
    let min = if parts[0] == "true" {
        parts[1].to_string()
    } else {
        String::new()
    };
    let max = if parts[2] == "true" {
        parts[3].to_string()
    } else {
        String::new()
    };
    Some((min, max))
}

impl FieldPropsState {
    fn new(
        target: Option<(ObjectId, ObjectId)>,
        page: PageId,
        rect: PRect,
        props: FieldProps,
    ) -> Self {
        let options_text = props
            .options
            .iter()
            .map(|(e, d)| {
                if e == d {
                    d.clone()
                } else {
                    format!("{d} | {e}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let mut st = Self {
            target,
            page,
            rect,
            props,
            tab: PropsTab::General,
            options_text,
            calc: CalcMode::None,
            calc_fields: String::new(),
            format: FormatMode::None,
            decimals: 2,
            currency: String::new(),
            date_format: "dd-mm-yyyy".into(),
            validate: ValidateMode::None,
            range_min: String::new(),
            range_max: String::new(),
            error: None,
        };
        st.parse_builders();
        st
    }

    /// Set the builders from the scripts (standard functions are recognised, the rest is custom).
    fn parse_builders(&mut self) {
        let s = &self.props.scripts;
        (self.calc, self.calc_fields) = if s.calculate.trim().is_empty() {
            (CalcMode::None, String::new())
        } else {
            parse_simple_calc(&s.calculate).unwrap_or((CalcMode::Custom, String::new()))
        };
        self.format = FormatMode::None;
        if !s.format.trim().is_empty() {
            if let Some((m, d, c)) = parse_number_format(&s.format) {
                (self.format, self.decimals, self.currency) = (m, d, c);
            } else if let Some(f) = parse_date_format(&s.format) {
                (self.format, self.date_format) = (FormatMode::Date, f);
            } else {
                self.format = FormatMode::Custom;
            }
        }
        (self.validate, self.range_min, self.range_max) = if s.validate.trim().is_empty() {
            (ValidateMode::None, String::new(), String::new())
        } else {
            match parse_range(&s.validate) {
                Some((a, b)) => (ValidateMode::Range, a, b),
                None => (ValidateMode::Custom, String::new(), String::new()),
            }
        };
    }

    /// The options text as `(value, shown)` pairs.
    fn apply_options(&mut self) {
        self.props.options = self
            .options_text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|l| match l.split_once('|') {
                Some((d, e)) => (e.trim().to_string(), d.trim().to_string()),
                None => (l.to_string(), l.to_string()),
            })
            .collect();
    }

    /// Turn the builders into scripts (custom scripts are left as they are).
    fn apply_builders(&mut self) {
        let s = &mut self.props.scripts;
        let op = match self.calc {
            CalcMode::Sum => Some("SUM"),
            CalcMode::Product => Some("PRD"),
            CalcMode::Average => Some("AVG"),
            CalcMode::Min => Some("MIN"),
            CalcMode::Max => Some("MAX"),
            CalcMode::None | CalcMode::Custom => None,
        };
        match (self.calc, op) {
            (CalcMode::None, _) => s.calculate.clear(),
            (_, Some(op)) => {
                s.calculate = format!(
                    "AFSimple_Calculate(\"{op}\", new Array({}));",
                    quoted_list(&self.calc_fields)
                )
            }
            _ => {}
        }
        let cur = self.currency.replace('"', "");
        match self.format {
            FormatMode::None => {
                s.format.clear();
                s.keystroke.clear();
            }
            FormatMode::Number => {
                s.format = format!(
                    "AFNumber_Format({}, 0, 0, 0, \"{cur}\", true);",
                    self.decimals
                );
                s.keystroke = format!(
                    "AFNumber_Keystroke({}, 0, 0, 0, \"{cur}\", true);",
                    self.decimals
                );
            }
            FormatMode::Percent => {
                s.format = format!("AFPercent_Format({}, 0);", self.decimals);
                s.keystroke = format!("AFPercent_Keystroke({}, 0);", self.decimals);
            }
            FormatMode::Date => {
                let f = self.date_format.replace('"', "");
                s.format = format!("AFDate_FormatEx(\"{f}\");");
                s.keystroke = format!("AFDate_KeystrokeEx(\"{f}\");");
            }
            FormatMode::Custom => {}
        }
        match self.validate {
            ValidateMode::None => s.validate.clear(),
            ValidateMode::Range => {
                let num = |v: &str| v.trim().replace(',', ".").parse::<f64>().ok();
                let (a, b) = (num(&self.range_min), num(&self.range_max));
                s.validate = format!(
                    "AFRange_Validate({}, {}, {}, {});",
                    a.is_some(),
                    a.unwrap_or(0.0),
                    b.is_some(),
                    b.unwrap_or(0.0)
                );
            }
            ValidateMode::Custom => {}
        }
    }
}

fn script_box(ui: &mut egui::Ui, text: &mut String, rows: usize) {
    ui.add(
        crate::ui_kit::multiline(text)
            .code_editor()
            .desired_rows(rows)
            .desired_width(f32::INFINITY),
    );
}

impl App {
    fn field_tool_size(tool: Tool) -> (f64, f64) {
        SIZES
            .iter()
            .find(|(t, _, _)| *t == tool)
            .map_or((120.0, 22.0), |(_, w, h)| (*w, *h))
    }

    /// The widget under `pt` on page `page` (topmost first): `(field, widget, rect)`.
    fn widget_at(&mut self, page: PageId, pt: Point) -> Option<(ObjectId, ObjectId, PRect)> {
        let form = self.form_for();
        form.fields.iter().rev().find_map(|f| {
            f.widgets
                .iter()
                .find(|w| w.page == Some(page) && w.rect.abs().contains(pt))
                .map(|w| (f.id, w.id, w.rect.abs()))
        })
    }

    /// Forms ▸ the field tools and Edit Fields.
    pub fn form_design_tool(
        &mut self,
        response: &egui::Response,
        vc: &ViewCtx,
        pos: Option<Pos2>,
        tool: Tool,
        _mods: egui::Modifiers,
    ) {
        let ti = self.active;
        let Some(pos) = pos else { return };
        if tool == Tool::FormDesign {
            self.edit_fields_tool(response, vc, pos);
            return;
        }
        // Drawing a new field: drag a box, or click for a standard size.
        if response.drag_started_by(egui::PointerButton::Primary) {
            let Some(i) = vc.page_at(pos) else { return };
            let p = vc.screen_to_pdf(i, pos);
            self.tabs[ti].ui.interaction = Interaction::Draw {
                page: vc.pages[i].id,
                tool,
                points: vec![p, p],
            };
            return;
        }
        if response.dragged_by(egui::PointerButton::Primary)
            && let Interaction::Draw { points, page, .. } = &mut self.tabs[ti].ui.interaction
            && let Some(i) = vc.pages.iter().position(|p| p.id == *page)
        {
            points.truncate(1);
            points.push(vc.screen_to_pdf(i, pos));
            return;
        }
        let placed = if response.drag_stopped()
            && let Interaction::Draw { page, points, .. } =
                std::mem::take(&mut self.tabs[ti].ui.interaction)
            && points.len() == 2
        {
            let r = PRect::new(points[0].x, points[0].y, points[1].x, points[1].y).abs();
            Some((page, r, points[0]))
        } else if response.clicked_by(egui::PointerButton::Primary)
            && let Some(i) = vc.page_at(pos)
        {
            let p = vc.screen_to_pdf(i, pos);
            Some((vc.pages[i].id, PRect::new(p.x, p.y, p.x, p.y), p))
        } else {
            None
        };
        let Some((page, mut r, at)) = placed else {
            return;
        };
        let (w, h) = Self::field_tool_size(tool);
        if r.width() < 6.0 || r.height() < 6.0 {
            // A click: the standard size, with its top-left corner at the pointer.
            r = PRect::new(at.x, at.y - h, at.x + w, at.y);
        }
        let Some((kind, base)) = kind_for(tool) else {
            return;
        };
        let doc = self.tabs[ti].session.doc().lopdf();
        let name = if kind == NewField::Radio {
            // Buttons drawn one after another join the group that was drawn last.
            self.last_radio_group
                .clone()
                .unwrap_or_else(|| formedit::unique_field_name(doc, base))
        } else {
            formedit::unique_field_name(doc, base)
        };
        let mut props = FieldProps::new(kind, &name);
        if props.kind == NewField::Radio {
            let n = self
                .form_for()
                .fields
                .iter()
                .find(|f| f.name == name)
                .map_or(0, |f| f.widgets.len());
            props.export = format!("Choice{}", n + 1);
        }
        if props.kind == NewField::Button {
            props.caption = tr("Button").into();
        }
        self.dialog = Some(Dialog::FieldProps(Box::new(FieldPropsState::new(
            None, page, r, props,
        ))));
    }

    fn edit_fields_tool(&mut self, response: &egui::Response, vc: &ViewCtx, pos: Pos2) {
        let ti = self.active;
        let Some(i) = vc.page_at(pos) else {
            if response.clicked() {
                self.tabs[ti].ui.field_sel = None;
            }
            return;
        };
        let page = vc.pages[i].id;
        let pt = vc.screen_to_pdf(i, pos);
        if response.drag_started_by(egui::PointerButton::Primary) {
            // Grab the corner of the selected field, or any field.
            let corner = self.tabs[ti]
                .ui
                .field_sel
                .and_then(|(_, w, p)| (p == page).then_some(w));
            let mut drag = None;
            if let Some(w) = corner
                && let Some((_, _, r)) = self.form_for().fields.iter().find_map(|f| {
                    f.widgets
                        .iter()
                        .find(|x| x.id == w)
                        .map(|x| (f.id, x.id, x.rect.abs()))
                })
            {
                let sr = Self::rect_screen(vc, i, r);
                if (pos - sr.right_bottom()).length() < 10.0 {
                    drag = Some(FieldDrag {
                        widget: w,
                        page,
                        start: pt,
                        rect: r,
                        resize: true,
                        now: pt,
                    });
                }
            }
            if drag.is_none()
                && let Some((f, w, r)) = self.widget_at(page, pt)
            {
                self.tabs[ti].ui.field_sel = Some((f, w, page));
                drag = Some(FieldDrag {
                    widget: w,
                    page,
                    start: pt,
                    rect: r,
                    resize: false,
                    now: pt,
                });
            }
            self.tabs[ti].ui.field_drag = drag;
        } else if response.dragged_by(egui::PointerButton::Primary) {
            if let Some(d) = &mut self.tabs[ti].ui.field_drag
                && d.page == page
            {
                d.now = pt;
            }
        } else if response.drag_stopped() {
            if let Some(d) = self.tabs[ti].ui.field_drag.take() {
                let new = Self::dragged_rect(&d);
                if new != d.rect {
                    let snapshot = pdf_engine::doc::PdfDocument::open(
                        self.tabs[ti]
                            .session
                            .snapshot()
                            .map(|s| s.bytes.to_vec())
                            .unwrap_or_default(),
                        &pdf_engine::doc::OpenOptions::default(),
                    );
                    match snapshot {
                        Ok(view) => {
                            let label = if d.resize {
                                tr("Resize field")
                            } else {
                                tr("Move field")
                            };
                            if let Err(e) = self.tabs[ti].session.execute(label, |tx| {
                                formedit::move_widget(tx, &view, d.widget, new)
                            }) {
                                self.notify_error(e.to_string());
                            }
                        }
                        Err(e) => self.notify_error(e.to_string()),
                    }
                }
            }
        } else if response.double_clicked() {
            if let Some((f, w, _)) = self.widget_at(page, pt) {
                self.tabs[ti].ui.field_sel = Some((f, w, page));
                self.open_field_props(f, w, page);
            }
        } else if response.clicked() {
            self.tabs[ti].ui.field_sel = self.widget_at(page, pt).map(|(f, w, _)| (f, w, page));
        }
        if self.dialog.is_none()
            && self.tabs[ti].ui.field_sel.is_some()
            && response
                .ctx
                .input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
        {
            self.delete_selected_field();
        }
        if self.dialog.is_none()
            && response.ctx.input(|i| i.key_pressed(egui::Key::Enter))
            && let Some((f, w, p)) = self.tabs[ti].ui.field_sel
        {
            self.open_field_props(f, w, p);
        }
    }

    fn dragged_rect(d: &FieldDrag) -> PRect {
        let (dx, dy) = (d.now.x - d.start.x, d.now.y - d.start.y);
        if d.resize {
            // The top-left corner stays; the bottom-right follows the pointer.
            PRect::new(
                d.rect.x0,
                (d.rect.y0 + dy).min(d.rect.y1 - 6.0),
                (d.rect.x1 + dx).max(d.rect.x0 + 6.0),
                d.rect.y1,
            )
        } else {
            PRect::new(
                d.rect.x0 + dx,
                d.rect.y0 + dy,
                d.rect.x1 + dx,
                d.rect.y1 + dy,
            )
        }
    }

    fn delete_selected_field(&mut self) {
        let ti = self.active;
        let Some((_, w, _)) = self.tabs[ti].ui.field_sel.take() else {
            return;
        };
        if let Err(e) = self.tabs[ti]
            .session
            .execute(tr("Delete field"), |tx| formedit::delete_widget(tx, w))
        {
            self.notify_error(e.to_string());
        }
    }

    fn open_field_props(&mut self, field: ObjectId, widget: ObjectId, page: PageId) {
        let ti = self.active;
        let doc = self.tabs[ti].session.doc();
        let Some(props) = formedit::read_props(doc, field, widget) else {
            self.notify(tr("This field type cannot be changed."));
            return;
        };
        let rect = self
            .form_for()
            .fields
            .iter()
            .flat_map(|f| f.widgets.iter())
            .find(|w| w.id == widget)
            .map_or(PRect::ZERO, |w| w.rect);
        self.dialog = Some(Dialog::FieldProps(Box::new(FieldPropsState::new(
            Some((field, widget)),
            page,
            rect,
            props,
        ))));
    }

    /// Field names, outlines and the selection while preparing a form.
    pub fn paint_form_design(&mut self, painter: &egui::Painter, vc: &ViewCtx) {
        if self.tool.family() != editor_core::tools::ToolFamily::Form || self.tool == Tool::FillForm
        {
            return;
        }
        let ti = self.active;
        let form = self.form_for();
        let sel = self.tabs[ti].ui.field_sel;
        let drag = self.tabs[ti].ui.field_drag.clone();
        let accent = self.pal.accent;
        for f in &form.fields {
            for w in &f.widgets {
                let Some(page) = w.page else { continue };
                let Some(i) = vc.pages.iter().position(|p| p.id == page) else {
                    continue;
                };
                let rect = match &drag {
                    Some(d) if d.widget == w.id => Self::dragged_rect(d),
                    _ => w.rect.abs(),
                };
                let r = Self::rect_screen(vc, i, rect);
                let selected = sel.is_some_and(|(_, sw, _)| sw == w.id);
                let line = if selected {
                    accent
                } else {
                    Color32::from_rgb(40, 100, 230)
                };
                painter.rect_stroke(
                    r,
                    1.0,
                    Stroke::new(if selected { 2.0 } else { 1.0 }, line),
                    egui::StrokeKind::Outside,
                );
                painter.text(
                    r.left_top() + egui::vec2(2.0, -2.0),
                    egui::Align2::LEFT_BOTTOM,
                    &f.name,
                    egui::FontId::proportional(10.0),
                    line,
                );
                if selected {
                    painter.rect_filled(
                        egui::Rect::from_center_size(r.right_bottom(), egui::vec2(8.0, 8.0)),
                        1.0,
                        accent,
                    );
                }
            }
        }
    }

    /// The field properties dialog. Returns whether it stays open.
    pub fn dialog_field_props(&mut self, ctx: &egui::Context, st: &mut FieldPropsState) -> bool {
        let mut choice: Option<u8> = None;
        let dim = self.pal.text_dim;
        let danger = self.pal.danger;
        let is_new = st.target.is_none();
        let kind = st.props.kind.clone();
        let names: Vec<String> = self
            .form_for()
            .fields
            .iter()
            .map(|f| f.name.clone())
            .collect();
        modal(ctx, "field_props", |ui| {
            ui.set_max_width(560.0);
            ui.heading(if is_new {
                tr("New field")
            } else {
                tr("Field properties")
            });
            let before = st.tab;
            ui.horizontal(|ui| {
                ui.selectable_value(&mut st.tab, PropsTab::General, tr("General"));
                if matches!(kind, NewField::Combo { .. } | NewField::List { .. }) {
                    ui.selectable_value(&mut st.tab, PropsTab::Options, tr("Options"));
                }
                if matches!(kind, NewField::Text { .. } | NewField::Combo { .. }) {
                    ui.selectable_value(&mut st.tab, PropsTab::Calculate, tr("Calculate & format"));
                }
                ui.selectable_value(&mut st.tab, PropsTab::Actions, tr("Scripts"));
            });
            // The two views of the same scripts stay in step.
            if before != st.tab {
                if before == PropsTab::Calculate {
                    st.apply_builders();
                }
                if st.tab == PropsTab::Calculate {
                    st.parse_builders();
                }
            }
            ui.separator();
            let p = &mut st.props;
            match st.tab {
                PropsTab::General => {
                    crate::ui_kit::form_row(ui, tr("Name"), |ui| {
                        ui.add(
                            crate::ui_kit::singleline(&mut p.name)
                                .char_limit(formedit::MAX_NAME)
                                .desired_width(260.0),
                        );
                    });
                    if kind == NewField::Radio {
                        ui.label(RichText::new(tr("Radio buttons with the same name are one group: only one of them can be on.")).size(12.0).color(dim));
                    }
                    crate::ui_kit::form_row(ui, tr("Tooltip"), |ui| {
                        ui.add(crate::ui_kit::singleline(&mut p.tooltip).desired_width(260.0));
                    });
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut p.required, tr("Required"));
                        ui.checkbox(&mut p.read_only, tr("Read-only"));
                        ui.checkbox(&mut p.hidden, tr("Hidden"));
                    });
                    match &mut p.kind {
                        NewField::Text {
                            multiline,
                            password,
                            comb,
                            max_len,
                        } => {
                            ui.horizontal(|ui| {
                                ui.checkbox(multiline, tr("Several lines"));
                                ui.checkbox(password, tr("Password"));
                                ui.checkbox(comb, tr("Comb of characters"));
                            });
                            crate::ui_kit::form_row(ui, tr("At most"), |ui| {
                                let mut on = max_len.is_some();
                                if ui.checkbox(&mut on, "").changed() {
                                    *max_len = on.then_some(max_len.unwrap_or(10));
                                }
                                if let Some(n) = max_len {
                                    ui.add(egui::DragValue::new(n).range(1..=10_000));
                                    ui.label(tr("characters"));
                                }
                            });
                            crate::ui_kit::form_row(ui, tr("Value"), |ui| {
                                ui.add(
                                    crate::ui_kit::singleline(&mut p.value).desired_width(260.0),
                                );
                            });
                        }
                        NewField::Checkbox | NewField::Radio => {
                            crate::ui_kit::form_row(ui, tr("Export value"), |ui| {
                                ui.add(
                                    crate::ui_kit::singleline(&mut p.export).desired_width(140.0),
                                );
                            });
                            let mut on = p.value == p.export;
                            if ui.checkbox(&mut on, tr("On by default")).changed() {
                                p.value = if on { p.export.clone() } else { "Off".into() };
                            }
                        }
                        NewField::Combo { editable } => {
                            ui.checkbox(editable, tr("People may type their own value"));
                        }
                        NewField::List { multi } => {
                            ui.checkbox(multi, tr("Several entries can be selected"));
                        }
                        NewField::Button => {
                            crate::ui_kit::form_row(ui, tr("Caption"), |ui| {
                                ui.add(
                                    crate::ui_kit::singleline(&mut p.caption).desired_width(200.0),
                                );
                            });
                            ui.checkbox(&mut p.reset_form, tr("Clicking it resets the form"));
                        }
                        NewField::Signature => {
                            ui.label(RichText::new(tr("An empty field that shows where to sign. Sign ▸ Sign Document can put a digital signature in it.")).size(12.0).color(dim));
                        }
                    }
                    ui.add_space(4.0);
                    crate::ui_kit::form_row(ui, tr("Text"), |ui| {
                        let mut auto = p.font_size == 0.0;
                        if ui.checkbox(&mut auto, tr("Auto size")).changed() {
                            p.font_size = if auto { 0.0 } else { 11.0 };
                        }
                        if !auto {
                            ui.add(
                                egui::DragValue::new(&mut p.font_size)
                                    .range(2.0..=144.0)
                                    .suffix(" pt"),
                            );
                        }
                        ui.color_edit_button_rgb(&mut p.text_color);
                        egui::ComboBox::from_id_salt("field_align")
                            .selected_text(match p.align {
                                1 => tr("Centre"),
                                2 => tr("Right"),
                                _ => tr("Left"),
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut p.align, 0, tr("Left"));
                                ui.selectable_value(&mut p.align, 1, tr("Centre"));
                                ui.selectable_value(&mut p.align, 2, tr("Right"));
                            });
                    });
                    crate::ui_kit::form_row(ui, tr("Colours"), |ui| {
                        for (label, c) in [
                            (tr("Border"), &mut p.border),
                            (tr("Background"), &mut p.background),
                        ] {
                            let mut on = c.is_some();
                            if ui.checkbox(&mut on, label).changed() {
                                *c = on.then_some([0.6, 0.6, 0.6]);
                            }
                            if let Some(col) = c {
                                ui.color_edit_button_rgb(col);
                            }
                        }
                    });
                }
                PropsTab::Options => {
                    ui.label(RichText::new(tr("One choice per line. To store a different value than the one shown, write: shown text | value")).size(12.0).color(dim));
                    ui.add(
                        crate::ui_kit::multiline(&mut st.options_text)
                            .desired_rows(8)
                            .desired_width(f32::INFINITY),
                    );
                    crate::ui_kit::form_row(ui, tr("Chosen"), |ui| {
                        ui.add(
                            crate::ui_kit::singleline(&mut p.value)
                                .hint_text(tr("value of the chosen entry"))
                                .desired_width(200.0),
                        );
                    });
                }
                PropsTab::Calculate => {
                    ui.label(RichText::new(tr("These are run only when you allow scripts for the document (Forms ▸ Form Scripts).")).size(12.0).color(dim));
                    crate::ui_kit::form_row(ui, tr("Value"), |ui| {
                        egui::ComboBox::from_id_salt("calc_mode")
                            .selected_text(match st.calc {
                                CalcMode::None => tr("Typed in"),
                                CalcMode::Sum => tr("Sum of"),
                                CalcMode::Product => tr("Product of"),
                                CalcMode::Average => tr("Average of"),
                                CalcMode::Min => tr("Smallest of"),
                                CalcMode::Max => tr("Largest of"),
                                CalcMode::Custom => tr("Custom script"),
                            })
                            .show_ui(ui, |ui| {
                                for (m, t) in [
                                    (CalcMode::None, tr("Typed in")),
                                    (CalcMode::Sum, tr("Sum of")),
                                    (CalcMode::Product, tr("Product of")),
                                    (CalcMode::Average, tr("Average of")),
                                    (CalcMode::Min, tr("Smallest of")),
                                    (CalcMode::Max, tr("Largest of")),
                                    (CalcMode::Custom, tr("Custom script")),
                                ] {
                                    ui.selectable_value(&mut st.calc, m, t);
                                }
                            });
                    });
                    match st.calc {
                        CalcMode::None => {}
                        CalcMode::Custom => script_box(ui, &mut p.scripts.calculate, 4),
                        _ => {
                            ui.add(
                                crate::ui_kit::singleline(&mut st.calc_fields)
                                    .hint_text(tr("field names, separated by commas"))
                                    .desired_width(f32::INFINITY),
                            );
                            ui.horizontal_wrapped(|ui| {
                                for n in names.iter().filter(|n| **n != p.name).take(30) {
                                    if ui.small_button(n).clicked() {
                                        if !st.calc_fields.trim().is_empty() {
                                            st.calc_fields.push_str(", ");
                                        }
                                        st.calc_fields.push_str(n);
                                    }
                                }
                            });
                        }
                    }
                    crate::ui_kit::form_row(ui, tr("Format"), |ui| {
                        egui::ComboBox::from_id_salt("format_mode")
                            .selected_text(match st.format {
                                FormatMode::None => tr("As typed"),
                                FormatMode::Number => tr("Number"),
                                FormatMode::Percent => tr("Percentage"),
                                FormatMode::Date => tr("Date"),
                                FormatMode::Custom => tr("Custom script"),
                            })
                            .show_ui(ui, |ui| {
                                for (m, t) in [
                                    (FormatMode::None, tr("As typed")),
                                    (FormatMode::Number, tr("Number")),
                                    (FormatMode::Percent, tr("Percentage")),
                                    (FormatMode::Date, tr("Date")),
                                    (FormatMode::Custom, tr("Custom script")),
                                ] {
                                    ui.selectable_value(&mut st.format, m, t);
                                }
                            });
                        match st.format {
                            FormatMode::Number | FormatMode::Percent => {
                                ui.add(egui::DragValue::new(&mut st.decimals).range(0..=10));
                                ui.label(tr("decimals"));
                                if st.format == FormatMode::Number {
                                    ui.add(
                                        crate::ui_kit::singleline(&mut st.currency)
                                            .hint_text("€")
                                            .desired_width(40.0),
                                    );
                                }
                            }
                            FormatMode::Date => {
                                ui.add(
                                    crate::ui_kit::singleline(&mut st.date_format)
                                        .desired_width(110.0),
                                );
                            }
                            _ => {}
                        }
                    });
                    if st.format == FormatMode::Custom {
                        ui.label(RichText::new(tr("Format script")).color(dim));
                        script_box(ui, &mut p.scripts.format, 3);
                        ui.label(RichText::new(tr("Keystroke script")).color(dim));
                        script_box(ui, &mut p.scripts.keystroke, 3);
                    }
                    crate::ui_kit::form_row(ui, tr("Accept"), |ui| {
                        ui.radio_value(&mut st.validate, ValidateMode::None, tr("Anything"));
                        ui.radio_value(&mut st.validate, ValidateMode::Range, tr("A number from"));
                        if st.validate == ValidateMode::Range {
                            ui.add(
                                crate::ui_kit::singleline(&mut st.range_min).desired_width(60.0),
                            );
                            ui.label(tr("to"));
                            ui.add(
                                crate::ui_kit::singleline(&mut st.range_max).desired_width(60.0),
                            );
                        }
                        ui.radio_value(&mut st.validate, ValidateMode::Custom, tr("Custom script"));
                    });
                    if st.validate == ValidateMode::Custom {
                        script_box(ui, &mut p.scripts.validate, 3);
                    }
                }
                PropsTab::Actions => {
                    ui.label(RichText::new(tr("JavaScript as Acrobat runs it: event.value, event.rc, this.getField(\"name\").value, app.alert(…). It runs only when you allow scripts for the document.")).size(12.0).color(dim));
                    if kind == NewField::Button {
                        ui.label(RichText::new(tr("When the button is clicked")).color(dim));
                        script_box(ui, &mut p.scripts.action, 5);
                    }
                    if matches!(
                        kind,
                        NewField::Text { .. } | NewField::Combo { .. } | NewField::List { .. }
                    ) {
                        for (label, s) in [
                            (tr("Calculate"), &mut p.scripts.calculate),
                            (tr("Validate"), &mut p.scripts.validate),
                            (tr("Format"), &mut p.scripts.format),
                            (tr("Keystroke"), &mut p.scripts.keystroke),
                        ] {
                            ui.label(RichText::new(label).color(dim));
                            script_box(ui, s, 2);
                        }
                    }
                    if matches!(
                        kind,
                        NewField::Checkbox | NewField::Radio | NewField::Signature
                    ) {
                        ui.label(
                            RichText::new(tr("This kind of field has no scripts of its own here."))
                                .size(12.0)
                                .color(dim),
                        );
                    }
                }
            }
            if let Some(e) = &st.error {
                ui.colored_label(danger, e);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if crate::ui_kit::primary_button(ui, if is_new { tr("Add field") } else { "OK" })
                    .clicked()
                {
                    choice = Some(1);
                }
                if ui.button(tr("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    choice = Some(0);
                }
                if !is_new && crate::ui_kit::danger_button(ui, tr("Delete field")).clicked() {
                    choice = Some(2);
                }
            });
        });
        match choice {
            None => true,
            Some(0) => false,
            Some(2) => {
                if let Some((f, w)) = st.target {
                    self.tabs[self.active].ui.field_sel = Some((f, w, st.page));
                    self.delete_selected_field();
                }
                false
            }
            Some(_) => {
                st.apply_options();
                if st.tab == PropsTab::Calculate {
                    st.apply_builders();
                }
                let ti = self.active;
                let snapshot = self.tabs[ti]
                    .session
                    .snapshot()
                    .map(|s| s.bytes.to_vec())
                    .map_err(|e| e.to_string())
                    .and_then(|b| {
                        pdf_engine::doc::PdfDocument::open(
                            b,
                            &pdf_engine::doc::OpenOptions::default(),
                        )
                        .map_err(|e| e.to_string())
                    });
                let view = match snapshot {
                    Ok(v) => v,
                    Err(e) => {
                        st.error = Some(e);
                        return true;
                    }
                };
                let (page, rect, props) = (st.page, st.rect, st.props.clone());
                let r = match st.target {
                    None => self.tabs[ti].session.execute(tr("Add form field"), |tx| {
                        formedit::create_field(tx, &view, page, rect, &props).map(|_| ())
                    }),
                    Some((f, w)) => self.tabs[ti]
                        .session
                        .execute(tr("Change form field"), |tx| {
                            formedit::update_field(tx, f, w, &props)
                        }),
                };
                match r {
                    Ok(()) => {
                        if props.kind == NewField::Radio {
                            self.last_radio_group = Some(props.name.trim().to_string());
                        }
                        false
                    }
                    Err(e) => {
                        st.error = Some(e.to_string());
                        true
                    }
                }
            }
        }
    }

    /// The selected field's name for the status bar.
    pub fn selected_field_hint(&mut self) -> Option<String> {
        let ti = self.active;
        let (f, _, _) = self.tabs.get(ti)?.ui.field_sel?;
        let form = self.form_for();
        let name = form.fields.iter().find(|x| x.id == f)?.name.clone();
        Some(tf!(
            "Field “{}” selected. Double-click or Enter for its properties.",
            name
        ))
    }
}
