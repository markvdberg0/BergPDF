//! Password-protected documents: the password question when opening one, and the dialog to protect, unprotect
//! or unlock the active document. The engine side is `pdf_engine::protect` (docs/DECISIONS.md D-035).

use crate::dialogs::modal;
use crate::i18n::tr;
use crate::state::*;
use crate::tf;
use editor_core::session::DocumentSession;
use egui::RichText;
use pdf_engine::EngineError;
use pdf_engine::protect::Rights;
use std::path::{Path, PathBuf};

/// "This document is protected: enter its password."
pub struct PasswordPrompt {
    path: PathBuf,
    input: String,
    error: Option<String>,
}

/// The protection dialog of the active document.
pub struct ProtectionDialog {
    /// Owner password typed to unlock editing.
    owner_input: String,
    user: String,
    user_again: String,
    owner: String,
    allow_print: bool,
    allow_copy: bool,
    allow_edit: bool,
    /// The form for new passwords is shown (always for an unprotected document).
    show_form: bool,
    error: Option<String>,
}

impl App {
    /// Ask for the password of `path`.
    pub fn ask_password(&mut self, path: &Path) {
        self.dialog = Some(Dialog::Password(Box::new(PasswordPrompt {
            path: path.to_path_buf(),
            input: String::new(),
            error: None,
        })));
    }

    /// The password question. Returns whether it stays open.
    pub fn dialog_password(&mut self, ctx: &egui::Context, st: &mut PasswordPrompt) -> bool {
        let name = st
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut submit = false;
        let mut cancel = false;
        modal(ctx, "password_prompt", |ui| {
            ui.heading(tr("Password required"));
            ui.label(tf!(
                "“{}” is protected with a password. Enter it to open the document.",
                name
            ));
            ui.add_space(6.0);
            let r = ui.add(
                egui::TextEdit::singleline(&mut st.input)
                    .password(true)
                    .hint_text(tr("Password"))
                    .desired_width(260.0),
            );
            if !r.has_focus() && st.error.is_none() {
                r.request_focus();
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                submit = true;
            }
            if let Some(e) = &st.error {
                ui.colored_label(self.pal.danger, e);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!st.input.is_empty(), egui::Button::new(tr("Open")))
                    .clicked()
                {
                    submit = true;
                }
                if ui.button(tr("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    cancel = true;
                }
            });
        });
        if cancel {
            return false;
        }
        if submit && !st.input.is_empty() {
            match DocumentSession::open_path_with_password(&st.path, &st.input) {
                Ok(session) => {
                    let path = st.path.clone();
                    self.finish_open(ctx, &path, session);
                    return false;
                }
                Err(EngineError::WrongPassword | EngineError::PasswordRequired) => {
                    st.error = Some(tr("That password is not correct.").into());
                    st.input.clear();
                }
                Err(e) => {
                    self.dialog = Some(Dialog::Error {
                        title: tr("Cannot open document").into(),
                        detail: format!("{}\n\n{e}", st.path.display()),
                    });
                    return false;
                }
            }
        }
        true
    }

    /// File ▸ Password Protection…
    pub fn open_protection_dialog(&mut self) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let protected = tab.session.protection().is_some();
        self.dialog = Some(Dialog::Protection(Box::new(ProtectionDialog {
            owner_input: String::new(),
            user: String::new(),
            user_again: String::new(),
            owner: String::new(),
            allow_print: true,
            allow_copy: true,
            allow_edit: true,
            show_form: !protected,
            error: None,
        })));
    }

    /// The protection dialog. Returns whether it stays open.
    pub fn dialog_protection(&mut self, ctx: &egui::Context, st: &mut ProtectionDialog) -> bool {
        let Some(tab) = self.tabs.get(self.active) else {
            return false;
        };
        let info = tab.session.protection();
        enum Act {
            Unlock,
            Remove,
            Apply,
            Close,
        }
        let mut act: Option<Act> = None;
        modal(ctx, "protection_dialog", |ui| {
            ui.heading(tr("Password protection"));
            match info {
                None => {
                    ui.label(tr("This document is not protected. Choose a password; it is needed to open the document after you save it."));
                }
                Some(i) => {
                    ui.label(tf!(
                        "This document is protected ({}).",
                        tr(i.cipher.label())
                    ));
                    if i.owner {
                        ui.label(tr("You have the owner password: nothing is restricted."));
                    } else {
                        ui.label(tr(
                            "You opened it with the document's password. What the author allows:",
                        ));
                        for (on, what) in [
                            (i.rights.print, tr("Printing")),
                            (i.rights.copy, tr("Copying text")),
                            (i.rights.modify, tr("Editing")),
                        ] {
                            ui.label(format!("{} {}", if on { "✓" } else { "✗" }, what));
                        }
                        ui.add_space(6.0);
                        ui.label(tr("Enter the owner password to lift the restrictions."));
                        ui.add(
                            egui::TextEdit::singleline(&mut st.owner_input)
                                .password(true)
                                .hint_text(tr("Owner password"))
                                .desired_width(260.0),
                        );
                        if ui
                            .add_enabled(
                                !st.owner_input.is_empty(),
                                egui::Button::new(tr("Unlock")),
                            )
                            .clicked()
                        {
                            act = Some(Act::Unlock);
                        }
                    }
                }
            }
            let owner = info.is_none_or(|i| i.owner);
            if owner && info.is_some() {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(tr("Remove password protection")).clicked() {
                        act = Some(Act::Remove);
                    }
                    if !st.show_form && ui.button(tr("Change password…")).clicked() {
                        st.show_form = true;
                    }
                });
                ui.label(
                    RichText::new(tr("The change is written when you save the document."))
                        .size(12.0)
                        .color(self.pal.text_dim),
                );
            }
            if owner && st.show_form {
                ui.add_space(8.0);
                ui.separator();
                let pw = |ui: &mut egui::Ui, s: &mut String, hint: String| {
                    ui.add(
                        egui::TextEdit::singleline(s)
                            .password(true)
                            .hint_text(hint)
                            .desired_width(260.0),
                    );
                };
                pw(ui, &mut st.user, tr("Password to open the document").into());
                pw(ui, &mut st.user_again, tr("Repeat the password").into());
                pw(ui, &mut st.owner, tr("Owner password (optional)").into());
                ui.label(
                    RichText::new(tr("With an owner password you can restrict what people who only know the password may do. Without one, the password above also unlocks everything."))
                        .size(12.0)
                        .color(self.pal.text_dim),
                );
                ui.checkbox(&mut st.allow_print, tr("Allow printing"));
                ui.checkbox(&mut st.allow_copy, tr("Allow copying text"));
                ui.checkbox(&mut st.allow_edit, tr("Allow editing"));
                ui.label(
                    RichText::new(tr("AES 256-bit encryption. If you forget the password the document cannot be opened."))
                        .size(12.0)
                        .color(self.pal.text_dim),
                );
                if ui.button(tr("Protect the document")).clicked() {
                    act = Some(Act::Apply);
                }
            }
            if let Some(e) = &st.error {
                ui.colored_label(self.pal.danger, e);
            }
            ui.add_space(8.0);
            if ui.button(tr("Close")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                act = Some(Act::Close);
            }
        });
        let Some(act) = act else {
            return true;
        };
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return false;
        };
        match act {
            Act::Close => return false,
            Act::Unlock => match tab.session.unlock_as_owner(&st.owner_input) {
                Ok(()) => {
                    self.notify(tr("Unlocked: you can edit this document now."));
                    return false;
                }
                Err(EngineError::WrongPassword) => {
                    st.error = Some(tr("That is not the owner password.").into());
                    st.owner_input.clear();
                }
                Err(e) => st.error = Some(e.to_string()),
            },
            Act::Remove => match tab.session.remove_protection() {
                Ok(()) => {
                    self.notify(tr(
                        "Protection removed. Save the document to write it without a password.",
                    ));
                    return false;
                }
                Err(e) => st.error = Some(e.to_string()),
            },
            Act::Apply => {
                if st.user.is_empty() {
                    st.error = Some(tr("Enter a password.").into());
                } else if st.user != st.user_again {
                    st.error = Some(tr("The two passwords are not the same.").into());
                } else if st.owner.is_empty() && !(st.allow_print && st.allow_copy && st.allow_edit)
                {
                    st.error =
                        Some(tr("Enter an owner password to restrict what others may do.").into());
                } else {
                    let rights = Rights {
                        print: st.allow_print,
                        copy: st.allow_copy,
                        modify: st.allow_edit,
                        annotate: st.allow_edit,
                        fill: st.allow_edit,
                        assemble: st.allow_edit,
                    };
                    match tab.session.set_protection(&st.user, &st.owner, rights) {
                        Ok(()) => {
                            self.notify(tr("Protected. Save the document to write the password."));
                            return false;
                        }
                        Err(e) => st.error = Some(e.to_string()),
                    }
                }
            }
        }
        true
    }
}
