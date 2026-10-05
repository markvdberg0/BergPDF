//! Form filling: click handling, field highlighting, the fill dialog and the dialog that asks
//! how form fields are treated when pages are duplicated, extracted or merged.
//!
//! Field values are written with the engine's form functions as ordinary undoable commands.
//! No field action, calculation or script is ever executed.

use crate::canvas::ViewCtx;
use crate::dialogs::modal;
use crate::state::*;
use editor_core::tools::Tool;
use egui::{Color32, Pos2, RichText, Stroke};
use pdf_engine::EngineError;
use pdf_engine::forms::{self, FieldKind, FormField, FormInfo, WidgetInfo};
use pdf_engine::pageops::FormPolicy;
use std::sync::Arc;

impl App {
    /// Parsed form for the active document at the current revision (cached).
    pub fn form_for(&mut self) -> Arc<FormInfo> {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return Arc::new(FormInfo::default());
        };
        let rev = tab.session.revision();
        if let Some((r, f)) = &tab.ui.form
            && *r == rev
        {
            return f.clone();
        }
        let f = Arc::new(forms::read_form(tab.session.doc()));
        tab.ui.form = Some((rev, f.clone()));
        f
    }

    /// Whether the active document contains fillable fields.
    pub fn has_form(&mut self) -> bool {
        !self.tabs.is_empty() && !self.form_for().fields.is_empty()
    }

    /// Handle a click in the fill-form tool (and, like a reader, in the navigation tools).
    pub fn form_click(&mut self, response: &egui::Response, vc: &ViewCtx, pos: Option<Pos2>) {
        if !response.clicked_by(egui::PointerButton::Primary) {
            return;
        }
        let Some(pos) = pos else { return };
        let Some(i) = vc.page_at(pos) else { return };
        let page = vc.pages[i].id;
        let pt = vc.screen_to_pdf(i, pos);
        let form = self.form_for();
        let hit = form.fields.iter().rev().find_map(|f| {
            f.widgets
                .iter()
                .find(|w| w.page == Some(page) && w.rect.abs().contains(pt))
                .map(|w| (f, w))
        });
        if let Some((f, w)) = hit {
            self.activate_widget(f, w);
        } else if self.tool == Tool::FillForm && form.fields.is_empty() {
            self.notify("This document has no form fields.");
        }
    }

    fn activate_widget(&mut self, f: &FormField, w: &WidgetInfo) {
        if f.read_only {
            self.notify(format!("“{}” is read-only.", f.name));
            return;
        }
        let ti = self.active;
        match &f.kind {
            FieldKind::Text {
                multiline,
                password,
                max_len,
                ..
            } => {
                self.dialog = Some(Dialog::FillField(Box::new(FillFieldState {
                    field: f.id,
                    name: f.name.clone(),
                    value: f.value.clone(),
                    multiline: *multiline,
                    password: *password,
                    max_len: *max_len,
                    options: Vec::new(),
                    error: None,
                })));
            }
            FieldKind::Choice { options, .. } => {
                self.dialog = Some(Dialog::FillField(Box::new(FillFieldState {
                    field: f.id,
                    name: f.name.clone(),
                    value: f.value.clone(),
                    multiline: false,
                    password: false,
                    max_len: None,
                    options: options.clone(),
                    error: None,
                })));
            }
            FieldKind::Checkbox => {
                let on = !matches!(f.value.as_str(), "" | "Off");
                let id = f.id;
                let r = self.tabs[ti]
                    .session
                    .execute(if on { "Uncheck box" } else { "Check box" }, |tx| {
                        forms::set_checkbox(tx, id, !on)
                    });
                if let Err(e) = r {
                    self.notify_error(e.to_string());
                }
            }
            FieldKind::Radio => {
                let Some(state) = w.on_state.clone() else {
                    self.notify("This radio button has no selectable state.");
                    return;
                };
                let id = f.id;
                let r = self.tabs[ti]
                    .session
                    .execute("Select option", |tx| forms::set_radio(tx, id, &state));
                if let Err(e) = r {
                    self.notify_error(e.to_string());
                }
            }
            FieldKind::PushButton => {
                self.notify(
                    "Buttons that run actions are not executed: Ferrum PDF never runs scripts.",
                );
            }
            FieldKind::Signature => {
                self.notify("Signature fields can be viewed, but signing is not supported.");
            }
            FieldKind::Unknown => self.notify("This field type is not supported."),
        }
    }

    /// Tint form fields so they are discoverable (a form is a visible, first-class thing).
    pub fn paint_form_overlays(&mut self, painter: &egui::Painter, vc: &ViewCtx) {
        if self.tabs.is_empty() {
            return;
        }
        let form = self.form_for();
        if form.fields.is_empty() {
            return;
        }
        let band = (
            f64::from(vc.scroll.y - 50.0),
            f64::from(vc.scroll.y + vc.viewport.height() + 50.0),
        );
        let pages = vc.layout.pages_in_band(band.0, band.1);
        let strong = self.tool == Tool::FillForm;
        for i in pages {
            let page = vc.pages[i].id;
            for f in &form.fields {
                for w in f.widgets.iter().filter(|w| w.page == Some(page)) {
                    let r = Self::rect_screen(vc, i, w.rect);
                    let (fill, line) = if f.read_only {
                        (
                            Color32::from_rgba_unmultiplied(150, 150, 150, 28),
                            Color32::from_rgba_unmultiplied(120, 120, 120, 90),
                        )
                    } else if strong {
                        (
                            Color32::from_rgba_unmultiplied(90, 150, 255, 60),
                            Color32::from_rgba_unmultiplied(40, 100, 230, 200),
                        )
                    } else {
                        (
                            Color32::from_rgba_unmultiplied(90, 150, 255, 30),
                            Color32::from_rgba_unmultiplied(40, 100, 230, 110),
                        )
                    };
                    painter.rect_filled(r, 1.0, fill);
                    painter.rect_stroke(r, 1.0, Stroke::new(1.0, line), egui::StrokeKind::Inside);
                }
            }
        }
    }

    /// The fill-field dialog. Returns whether it stays open.
    pub fn dialog_fill_field(&mut self, ctx: &egui::Context, st: &mut FillFieldState) -> bool {
        let mut choice: Option<bool> = None;
        modal(ctx, "fill_field", |ui| {
            ui.heading("Fill field");
            ui.label(RichText::new(&st.name).size(12.0).color(self.pal.text_dim));
            ui.add_space(4.0);
            if st.options.is_empty() {
                let mut te = if st.multiline {
                    egui::TextEdit::multiline(&mut st.value).desired_rows(5)
                } else {
                    egui::TextEdit::singleline(&mut st.value)
                }
                .desired_width(380.0);
                if st.password {
                    te = te.password(true);
                }
                let r = ui.add(te);
                if self.frame_counter.is_multiple_of(2) && !r.has_focus() && st.error.is_none() {
                    r.request_focus();
                }
                if let Some(m) = st.max_len {
                    ui.label(
                        RichText::new(format!("{} / {m} characters", st.value.chars().count()))
                            .size(11.0)
                            .color(self.pal.text_dim),
                    );
                }
            } else {
                let shown = st
                    .options
                    .iter()
                    .find(|(e, _)| *e == st.value)
                    .map_or(st.value.clone(), |(_, d)| d.clone());
                egui::ComboBox::from_id_salt("fill_choice")
                    .selected_text(shown)
                    .width(300.0)
                    .show_ui(ui, |ui| {
                        for (export, display) in &st.options {
                            ui.selectable_value(&mut st.value, export.clone(), display);
                        }
                    });
            }
            if let Some(e) = &st.error {
                ui.add_space(4.0);
                ui.colored_label(self.pal.danger, e);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    choice = Some(true);
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    choice = Some(false);
                }
            });
        });
        match choice {
            None => true,
            Some(false) => false,
            Some(true) => {
                let (id, value) = (st.field, st.value.clone());
                let r = self.tabs[self.active]
                    .session
                    .execute("Fill field", |tx| forms::set_text_value(tx, id, &value));
                match r {
                    Ok(()) => false,
                    Err(EngineError::MissingGlyphs { chars, .. }) => {
                        st.error = Some(format!(
                            "The bundled font cannot show these characters: {chars}. Remove or replace them."
                        ));
                        true
                    }
                    Err(e) => {
                        st.error = Some(e.to_string());
                        true
                    }
                }
            }
        }
    }

    /// The "what should happen to form fields?" dialog. Returns whether it stays open.
    pub fn dialog_form_policy(&mut self, ctx: &egui::Context, st: &mut FormPolicyState) -> bool {
        let mut choice: Option<bool> = None;
        let cross_document = !matches!(st.op, FormOp::Duplicate);
        if cross_document && st.policy == FormPolicy::Linked {
            st.policy = FormPolicy::Independent;
        }
        modal(ctx, "form_policy", |ui| {
            ui.heading("These pages contain form fields");
            ui.label(
                RichText::new("Choose what happens to the fields on the new copies.")
                    .size(12.0)
                    .color(self.pal.text_dim),
            );
            ui.add_space(6.0);
            let mut opt =
                |ui: &mut egui::Ui, p: FormPolicy, title: &str, body: &str, enabled: bool| {
                    ui.add_enabled_ui(enabled, |ui| {
                        ui.radio_value(&mut st.policy, p, RichText::new(title).strong());
                        ui.label(RichText::new(body).size(12.0).color(self.pal.text_dim));
                    });
                    ui.add_space(4.0);
                };
            opt(
                ui,
                FormPolicy::Independent,
                "Independent fields (recommended)",
                "Each copy gets its own fields with new names (for example name_2). Filling one copy never changes another.",
                true,
            );
            opt(
                ui,
                FormPolicy::Linked,
                "Linked fields",
                "Copies share the same fields, so typing in one updates every copy. Only possible within one document.",
                !cross_document,
            );
            opt(
                ui,
                FormPolicy::Flatten,
                "Flatten (static copy)",
                "Field contents are drawn onto the copy as plain page content and the fields are removed there.",
                true,
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Continue").clicked() {
                    choice = Some(true);
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    choice = Some(false);
                }
            });
        });
        match choice {
            None => true,
            Some(false) => false,
            Some(true) => {
                let (op, policy) = (st.op.clone(), st.policy);
                self.finish_form_policy(ctx, op, policy);
                false
            }
        }
    }
}
