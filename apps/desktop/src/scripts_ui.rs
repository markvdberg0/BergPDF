//! Form JavaScript: asking whether a document's scripts may run, running them when a field changes
//! or a button is clicked, writing what they changed back as one undoable step, and the Form Scripts
//! and Document JavaScript dialogs.
//!
//! Scripts run in `pdf_js`, a sandbox without any access to files, the network or other programs
//! (docs/DECISIONS.md D-040). They run only when the person allowed it for the document (or chose
//! "always" in the Form Scripts dialog).

use crate::dialogs::modal;
use crate::i18n::tr;
use crate::state::*;
use crate::tf;
use editor_core::prefs::ScriptPolicy;
use egui::RichText;
use pdf_engine::ObjectId;
use pdf_engine::forms::FieldKind;
use pdf_engine::formscripts::{self, FormScripts};

/// Whether a document's scripts may run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScriptPermission {
    /// Not asked yet.
    #[default]
    Unknown,
    Allowed,
    Refused,
}

/// What to do after the person answered the "run scripts?" question.
#[derive(Clone, Debug)]
pub enum ScriptsPending {
    Click { field: ObjectId },
    CalculateAll,
}

/// The Form Scripts dialog.
pub struct ScriptsDialog {
    allowed: bool,
    always: bool,
    list: Vec<(String, String, String)>,
}

/// The Document JavaScript dialog.
pub struct DocJsDialog {
    scripts: Vec<(String, String)>,
    selected: usize,
    error: Option<String>,
}

fn kind_of(k: &FieldKind) -> pdf_js::Kind {
    match k {
        FieldKind::Text { .. } | FieldKind::Unknown => pdf_js::Kind::Text,
        FieldKind::Checkbox => pdf_js::Kind::Checkbox,
        FieldKind::Radio => pdf_js::Kind::Radio,
        FieldKind::Choice { combo: true, .. } => pdf_js::Kind::Combo,
        FieldKind::Choice { combo: false, .. } => pdf_js::Kind::List,
        FieldKind::PushButton => pdf_js::Kind::Button,
        FieldKind::Signature => pdf_js::Kind::Signature,
    }
}

fn to_js(f: &FormScripts, file_name: &str) -> pdf_js::Form {
    pdf_js::Form {
        fields: f
            .fields
            .iter()
            .map(|x| pdf_js::Field {
                name: x.name.clone(),
                kind: kind_of(&x.kind),
                value: x.value.clone(),
                default_value: x.default_value.clone(),
                read_only: x.read_only,
                required: x.required,
                hidden: x.hidden,
                options: match &x.kind {
                    FieldKind::Choice { options, .. } => options.clone(),
                    _ => Vec::new(),
                },
                export_values: x.export_values.clone(),
                scripts: pdf_js::Scripts {
                    calculate: x.scripts.calculate.clone(),
                    validate: x.scripts.validate.clone(),
                    format: x.scripts.format.clone(),
                    keystroke: x.scripts.keystroke.clone(),
                    action: x.scripts.action.clone(),
                },
            })
            .collect(),
        order: f
            .order
            .iter()
            .filter_map(|id| {
                f.fields
                    .iter()
                    .find(|x| x.id == *id)
                    .map(|x| x.name.clone())
            })
            .collect(),
        document: f.document.iter().map(|(_, c)| c.clone()).collect(),
        pages: f.pages,
        file_name: file_name.to_string(),
    }
}

impl App {
    fn form_scripts(&self) -> Option<FormScripts> {
        let tab = self.active_tab()?;
        Some(formscripts::read_form_scripts(tab.session.doc()))
    }

    /// Whether the active document's scripts may run now.
    pub fn scripts_allowed(&self) -> bool {
        let Some(tab) = self.active_tab() else {
            return false;
        };
        match tab.ui.scripts {
            ScriptPermission::Allowed => true,
            ScriptPermission::Refused => false,
            ScriptPermission::Unknown => self.prefs.form_scripts == ScriptPolicy::Always,
        }
    }

    /// Run the scripts for a change (when allowed and needed) and write the value and everything the
    /// scripts changed as one step. `Err` holds what to tell the person when the value was refused.
    pub fn commit_field_value(
        &mut self,
        field: ObjectId,
        value: &str,
        label: &'static str,
    ) -> Result<(), String> {
        let Some(form) = self.form_scripts() else {
            return Ok(());
        };
        let Some(f) = form.fields.iter().find(|x| x.id == field).cloned() else {
            return Err(tr("The field was not found.").into());
        };
        let ti = self.active;
        let has = form.has_scripts();
        if has && self.tabs[ti].ui.scripts == ScriptPermission::Unknown {
            match self.prefs.form_scripts {
                ScriptPolicy::Ask => {
                    // Keep the value now; ask, and run the scripts once the answer is yes.
                    self.write_plain_value(&f, value, label)?;
                    // Shown after the fill dialog closes (dialogs are taken out while they run).
                    self.dialog = Some(Dialog::ScriptsAsk(ScriptsPending::CalculateAll));
                    return Ok(());
                }
                ScriptPolicy::Always => self.tabs[ti].ui.scripts = ScriptPermission::Allowed,
                ScriptPolicy::Never => self.tabs[ti].ui.scripts = ScriptPermission::Refused,
            }
        }
        if !has || !self.scripts_allowed() {
            return self.write_plain_value(&f, value, label);
        }
        let js = to_js(&form, &self.tabs[ti].session.title);
        let out = pdf_js::run(
            &js,
            &pdf_js::Event::Commit {
                field: f.name.clone(),
                value: value.to_string(),
            },
            pdf_js::Limits::default(),
        )
        .map_err(|e| e.to_string())?;
        if out.rejected.is_some() {
            let why = out.alerts.join("\n");
            return Err(if why.is_empty() {
                tr("The form does not accept this value.").into()
            } else {
                why
            });
        }
        self.apply_outcome(&form, out, label);
        Ok(())
    }

    fn write_plain_value(
        &mut self,
        f: &formscripts::ScriptField,
        value: &str,
        label: &'static str,
    ) -> Result<(), String> {
        let (id, kind) = (f.id, f.kind.clone());
        let v = value.to_string();
        self.tabs[self.active]
            .session
            .execute(label, |tx| match kind {
                FieldKind::Checkbox => {
                    pdf_engine::forms::set_checkbox(tx, id, !matches!(v.as_str(), "" | "Off"))
                }
                FieldKind::Radio => pdf_engine::forms::set_radio(tx, id, &v),
                _ => pdf_engine::forms::set_text_value(tx, id, &v),
            })
            .map_err(|e| match e {
                pdf_engine::EngineError::MissingGlyphs { chars, .. } => tf!(
                    "The bundled font cannot show these characters: {}. Remove or replace them.",
                    chars
                ),
                other => other.to_string(),
            })
    }

    /// Write what the scripts changed (one undoable step) and tell what they said.
    fn apply_outcome(&mut self, form: &FormScripts, out: pdf_js::Outcome, label: &'static str) {
        let by_name: std::collections::HashMap<&str, &formscripts::ScriptField> =
            form.fields.iter().map(|f| (f.name.as_str(), f)).collect();
        let changes: Vec<(formscripts::ScriptField, pdf_js::Change)> = out
            .changes
            .iter()
            .filter_map(|(n, c)| by_name.get(n.as_str()).map(|f| ((*f).clone(), c.clone())))
            .collect();
        if !changes.is_empty() {
            let r = self.tabs[self.active].session.execute(label, |tx| {
                for (f, c) in &changes {
                    match &f.kind {
                        FieldKind::Checkbox => {
                            if let Some(v) = &c.value {
                                pdf_engine::forms::set_checkbox(
                                    tx,
                                    f.id,
                                    !matches!(v.as_str(), "" | "Off"),
                                )?;
                            }
                        }
                        FieldKind::Radio => {
                            if let Some(v) = &c.value {
                                pdf_engine::forms::set_radio(
                                    tx,
                                    f.id,
                                    if v.is_empty() { "Off" } else { v },
                                )?;
                            }
                        }
                        FieldKind::Text { .. } | FieldKind::Choice { .. }
                            if c.value.is_some() || c.display.is_some() =>
                        {
                            let v = c.value.clone().unwrap_or_else(|| f.value.clone());
                            // A value the bundled font cannot draw is kept unformatted rather than lost.
                            let shown = c.display.as_deref().filter(|d| *d != v);
                            if pdf_engine::forms::set_text_value_as(tx, f.id, &v, shown, true)
                                .is_err()
                            {
                                pdf_engine::forms::set_text_value_as(tx, f.id, &v, None, true)?;
                            }
                        }
                        _ => {}
                    }
                    if let Some(h) = c.hidden {
                        formscripts::set_field_hidden(tx, f.id, h)?;
                    }
                    if c.read_only.is_some() || c.required.is_some() {
                        formscripts::set_field_flags(tx, f.id, c.read_only, c.required)?;
                    }
                }
                Ok(())
            });
            if let Err(e) = r {
                self.notify_error(tf!("The form's scripts could not be applied: {}", e));
            }
        }
        for a in out.alerts {
            self.notify(a);
        }
        if !out.errors.is_empty() {
            self.notify_error(tf!(
                "A script of this form failed: {}",
                out.errors.first().cloned().unwrap_or_default()
            ));
        }
        if !out.blocked.is_empty() {
            self.notify(tf!(
                "The form asked for {}; BergPDF never lets a form do that.",
                out.blocked.join(", ")
            ));
        }
    }

    /// A push button was clicked: its reset action, or its script.
    pub fn click_button(&mut self, field: ObjectId) {
        let Some(form) = self.form_scripts() else {
            return;
        };
        let ti = self.active;
        let resets = form.fields.iter().find(|f| f.id == field).is_some_and(|_| {
            let d = self.tabs[ti].session.doc().lopdf();
            pdf_engine::forms::read_form(self.tabs[ti].session.doc())
                .fields
                .iter()
                .find(|f| f.id == field)
                .and_then(|f| f.widgets.first())
                .and_then(|w| d.get_dictionary(w.id).ok())
                .and_then(|wd| wd.get(b"A").ok())
                .and_then(|a| pdf_engine::objutil::deref(d, a))
                .and_then(|a| a.as_dict().ok())
                .is_some_and(|a| pdf_engine::objutil::dict_name(d, a, b"S") == Some(b"ResetForm"))
        });
        if resets {
            match self.tabs[ti]
                .session
                .execute(tr("Reset form"), |tx| formscripts::reset_fields(tx, &form))
            {
                Ok(_) => self.notify(tr("The form was reset.")),
                Err(e) => self.notify_error(e.to_string()),
            }
            return;
        }
        let has_action = form
            .fields
            .iter()
            .find(|f| f.id == field)
            .is_some_and(|f| !f.scripts.action.trim().is_empty());
        if !has_action {
            self.notify(tr("This button does nothing in BergPDF."));
            return;
        }
        if self.tabs[ti].ui.scripts == ScriptPermission::Unknown {
            match self.prefs.form_scripts {
                ScriptPolicy::Ask => {
                    self.dialog = Some(Dialog::ScriptsAsk(ScriptsPending::Click { field }));
                    return;
                }
                ScriptPolicy::Always => self.tabs[ti].ui.scripts = ScriptPermission::Allowed,
                ScriptPolicy::Never => self.tabs[ti].ui.scripts = ScriptPermission::Refused,
            }
        }
        if !self.scripts_allowed() {
            self.notify(tr("This button runs a script, and scripts are off for this document (Forms ▸ Form Scripts)."));
            return;
        }
        self.run_scripts_event(&form, ScriptsPending::Click { field });
    }

    fn run_scripts_event(&mut self, form: &FormScripts, what: ScriptsPending) {
        let ti = self.active;
        let js = to_js(form, &self.tabs[ti].session.title);
        let (event, label) = match &what {
            ScriptsPending::Click { field } => {
                let Some(f) = form.fields.iter().find(|f| f.id == *field) else {
                    return;
                };
                (
                    pdf_js::Event::Click {
                        field: f.name.clone(),
                    },
                    tr("Run button"),
                )
            }
            ScriptsPending::CalculateAll => (pdf_js::Event::CalculateAll, tr("Calculate form")),
        };
        match pdf_js::run(&js, &event, pdf_js::Limits::default()) {
            Ok(out) => {
                if out.rejected.is_some() {
                    self.notify_error(out.alerts.join("\n"));
                } else {
                    self.apply_outcome(form, out, label);
                }
            }
            Err(e) => self.notify_error(e.to_string()),
        }
    }

    /// Forms ▸ Calculate Now.
    pub fn calculate_now(&mut self) {
        let Some(form) = self.form_scripts() else {
            return;
        };
        if !form.has_scripts() {
            self.notify(tr("This form has no calculations or scripts."));
            return;
        }
        if !self.scripts_allowed() {
            self.dialog = Some(Dialog::ScriptsAsk(ScriptsPending::CalculateAll));
            return;
        }
        self.run_scripts_event(&form, ScriptsPending::CalculateAll);
    }

    /// "This form contains scripts. Run them?"
    pub fn dialog_scripts_ask(&mut self, ctx: &egui::Context, pending: &ScriptsPending) -> bool {
        let mut choice: Option<u8> = None;
        let mut always = false;
        modal(ctx, "scripts_ask", |ui| {
            ui.heading(tr("Run this form's scripts?"));
            ui.label(tr("The form contains JavaScript for calculations, formats and checks. BergPDF runs it in a closed sandbox: the scripts can only fill in fields of this form and show messages. They cannot open files, links or the internet."));
            ui.checkbox(
                &mut always,
                tr("Always run form scripts (can be changed in Forms ▸ Form Scripts)"),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if crate::ui_kit::primary_button(ui, tr("Run scripts")).clicked() {
                    choice = Some(1);
                }
                if ui.button(tr("Not for this document")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    choice = Some(0);
                }
            });
        });
        let Some(c) = choice else { return true };
        let ti = self.active;
        if c == 1 {
            self.tabs[ti].ui.scripts = ScriptPermission::Allowed;
            if always {
                self.prefs.form_scripts = ScriptPolicy::Always;
                self.prefs_dirty = true;
            }
            if let Some(form) = self.form_scripts() {
                self.run_scripts_event(&form, pending.clone());
            }
        } else {
            self.tabs[ti].ui.scripts = ScriptPermission::Refused;
        }
        false
    }

    /// Forms ▸ Form Scripts…
    pub fn open_scripts_dialog(&mut self) {
        let Some(form) = self.form_scripts() else {
            return;
        };
        let mut list = Vec::new();
        for (n, c) in &form.document {
            list.push((tr("Document").to_string(), n.clone(), c.clone()));
        }
        for f in &form.fields {
            for (what, code) in [
                (tr("Calculate"), &f.scripts.calculate),
                (tr("Validate"), &f.scripts.validate),
                (tr("Format"), &f.scripts.format),
                (tr("Keystroke"), &f.scripts.keystroke),
                (tr("Action"), &f.scripts.action),
            ] {
                if !code.trim().is_empty() {
                    list.push((what.to_string(), f.name.clone(), code.clone()));
                }
            }
        }
        self.dialog = Some(Dialog::Scripts(Box::new(ScriptsDialog {
            allowed: self.scripts_allowed(),
            always: self.prefs.form_scripts == ScriptPolicy::Always,
            list,
        })));
    }

    pub fn dialog_scripts(&mut self, ctx: &egui::Context, st: &mut ScriptsDialog) -> bool {
        let mut close = false;
        let dim = self.pal.text_dim;
        modal(ctx, "form_scripts", |ui| {
            ui.set_max_width(620.0);
            ui.heading(tr("Form scripts"));
            ui.label(
                RichText::new(tr("Scripts run in a closed sandbox: they can fill in fields of this form and show messages, nothing else. Submitting, e-mailing, printing, links and other files are always blocked."))
                    .size(12.0)
                    .color(dim),
            );
            ui.checkbox(&mut st.allowed, tr("Run the scripts of this document"));
            ui.checkbox(
                &mut st.always,
                tr("Always run form scripts in every document"),
            );
            ui.add_space(4.0);
            if st.list.is_empty() {
                ui.label(RichText::new(tr("This document has no scripts.")).color(dim));
            } else {
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for (what, name, code) in &st.list {
                            egui::CollapsingHeader::new(format!("{what} · {name}"))
                                .id_salt((what, name))
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(code).monospace().size(11.0),
                                        )
                                        .wrap(),
                                    );
                                });
                        }
                    });
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if crate::ui_kit::primary_button(ui, tr("Close")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    close = true;
                }
            });
        });
        if !close {
            return true;
        }
        let ti = self.active;
        let was = self.scripts_allowed();
        self.tabs[ti].ui.scripts = if st.allowed {
            ScriptPermission::Allowed
        } else {
            ScriptPermission::Refused
        };
        let policy = if st.always {
            ScriptPolicy::Always
        } else if self.prefs.form_scripts == ScriptPolicy::Always {
            ScriptPolicy::Ask
        } else {
            self.prefs.form_scripts
        };
        if policy != self.prefs.form_scripts {
            self.prefs.form_scripts = policy;
            self.prefs_dirty = true;
        }
        if st.allowed && !was && !st.list.is_empty() {
            self.calculate_now();
        }
        false
    }

    /// Forms ▸ Document JavaScript…
    pub fn open_document_js(&mut self) {
        let Some(tab) = self.active_tab() else { return };
        let mut scripts = formscripts::document_scripts(tab.session.doc().lopdf());
        if scripts.is_empty() {
            scripts.push(("Script1".into(), String::new()));
        }
        self.dialog = Some(Dialog::DocJs(Box::new(DocJsDialog {
            scripts,
            selected: 0,
            error: None,
        })));
    }

    pub fn dialog_document_js(&mut self, ctx: &egui::Context, st: &mut DocJsDialog) -> bool {
        let mut choice: Option<bool> = None;
        let dim = self.pal.text_dim;
        modal(ctx, "doc_js", |ui| {
            ui.set_max_width(640.0);
            ui.heading(tr("Document JavaScript"));
            ui.label(
                RichText::new(tr("Scripts that belong to the whole document; they run before every field script, so functions defined here can be used in calculations."))
                    .size(12.0)
                    .color(dim),
            );
            ui.horizontal_wrapped(|ui| {
                for (i, (n, _)) in st.scripts.iter().enumerate() {
                    ui.selectable_value(&mut st.selected, i, n);
                }
                if ui
                    .small_button("+")
                    .on_hover_text(tr("Add a script"))
                    .clicked()
                {
                    st.scripts
                        .push((format!("Script{}", st.scripts.len() + 1), String::new()));
                    st.selected = st.scripts.len() - 1;
                }
            });
            if let Some((n, code)) = st.scripts.get_mut(st.selected) {
                crate::ui_kit::form_row(ui, tr("Name"), |ui| {
                    ui.add(crate::ui_kit::singleline(n).desired_width(200.0));
                });
                ui.add(
                    crate::ui_kit::multiline(code)
                        .code_editor()
                        .desired_rows(14)
                        .desired_width(f32::INFINITY),
                );
            }
            if st.scripts.len() > 1 && ui.button(tr("Remove this script")).clicked() {
                st.scripts.remove(st.selected);
                st.selected = st.selected.saturating_sub(1);
            }
            if let Some(e) = &st.error {
                ui.colored_label(self.pal.danger, e);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if crate::ui_kit::primary_button(ui, "OK").clicked() {
                    choice = Some(true);
                }
                if ui.button(tr("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    choice = Some(false);
                }
            });
        });
        match choice {
            None => true,
            Some(false) => false,
            Some(true) => {
                let scripts: Vec<(String, String)> = st
                    .scripts
                    .iter()
                    .map(|(n, c)| (n.trim().to_string(), c.clone()))
                    .filter(|(n, c)| !n.is_empty() && !c.trim().is_empty())
                    .collect();
                let before = self
                    .active_tab()
                    .map(|t| formscripts::document_scripts(t.session.doc().lopdf()))
                    .unwrap_or_default();
                if before == scripts {
                    return false;
                }
                match self.tabs[self.active]
                    .session
                    .execute(tr("Change document JavaScript"), |tx| {
                        formscripts::set_document_scripts(tx, &scripts)
                    }) {
                    Ok(()) => false,
                    Err(e) => {
                        st.error = Some(e.to_string());
                        true
                    }
                }
            }
        }
    }
}
