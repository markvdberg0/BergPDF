//! Signing: digital signatures from a certificate file, the list of signatures in a document,
//! and the reusable handwritten signature (a drawing placed as an ink annotation).
//!
//! The wording matters here: a *digital* signature proves the signed bytes are unchanged and who
//! holds the key; BergPDF does not decide whether a certificate is trustworthy. A *handwritten*
//! signature is just a picture. The dialogs say so.

use crate::canvas::ViewCtx;
use crate::dialogs::modal;
use crate::i18n::tr;
use crate::state::*;
use crate::tf;
use editor_core::handwriting::HandwrittenSignature;
use editor_core::tools::Tool;
use egui::{Color32, Pos2, RichText, Sense, Stroke, Vec2};
use pdf_engine::annot::{self, AnnotationKind, AnnotationSpec, Rgb};
use pdf_engine::geom::{Point, Rect as PRect};
use pdf_engine::sign::{Identity, SignOptions, SignatureInfo, SignatureStatus, list_signatures};

/// Width of a placed handwritten signature in points.
const PLACED_WIDTH_PT: f64 = 150.0;
/// Colour of "unchanged" results (independent of the theme accent).
const OK_GREEN: Color32 = Color32::from_rgb(70, 170, 100);

fn signature_file() -> std::path::PathBuf {
    platform::dirs::config_dir().join("signature.toml")
}

impl App {
    /// Load the saved handwritten signature (if any) at startup.
    pub fn load_handwriting() -> HandwrittenSignature {
        platform::dirs::read_text(&signature_file())
            .and_then(|t| HandwrittenSignature::from_toml(&t))
            .unwrap_or_default()
    }

    // ---- digital signature -------------------------------------------------------------

    /// Open the "Sign with certificate" dialog.
    pub fn open_sign_dialog(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        let can_edit = self.tabs[self.active].session.doc().capabilities().can_edit;
        if !can_edit {
            self.notify_error(tr(
                "This document cannot be modified, so it cannot be signed.",
            ));
            return;
        }
        self.dialog = Some(Dialog::Sign(Box::default()));
    }

    /// The sign dialog. Returns whether it stays open.
    pub fn dialog_sign(&mut self, ctx: &egui::Context, st: &mut SignDialogState) -> bool {
        enum Act {
            PickCert,
            Check,
            PickArea,
            Sign,
            Cancel,
        }
        let mut act: Option<Act> = None;
        modal(ctx, "sign_dialog", |ui| {
            ui.heading(tr("Sign with a certificate"));
            ui.label(
                RichText::new(tr("Creates a digital signature that shows whether the document changes after signing."))
                    .size(12.0)
                    .color(self.pal.text_dim),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(tr("Certificate"));
                let name = st
                    .cert_path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map_or(tr("none chosen").to_string(), |n| {
                        n.to_string_lossy().into_owned()
                    });
                ui.label(RichText::new(name).strong());
                if ui.button(tr("Choose…")).clicked() {
                    act = Some(Act::PickCert);
                }
            });
            ui.horizontal(|ui| {
                ui.label(tr("Password"));
                ui.add(
                    egui::TextEdit::singleline(&mut st.password)
                        .password(true)
                        .desired_width(220.0),
                );
                if ui
                    .add_enabled(st.cert_path.is_some(), egui::Button::new(tr("Check")))
                    .on_hover_text(tr("Unlock the file and show who it identifies"))
                    .clicked()
                {
                    act = Some(Act::Check);
                }
            });
            if let Some(h) = &st.signer_hint {
                ui.label(RichText::new(format!("✔ {h}")).size(12.0).color(OK_GREEN));
            }
            ui.add_space(4.0);
            egui::Grid::new("sign_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    ui.label(tr("Reason"));
                    ui.add(egui::TextEdit::singleline(&mut st.reason).desired_width(300.0));
                    ui.end_row();
                    ui.label(tr("Location"));
                    ui.add(egui::TextEdit::singleline(&mut st.location).desired_width(300.0));
                    ui.end_row();
                    ui.label(tr("Contact"));
                    ui.add(egui::TextEdit::singleline(&mut st.contact).desired_width(300.0));
                    ui.end_row();
                });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut st.visible, tr("Show the signature on a page"));
                if st.visible && ui.button(tr("Choose area…")).clicked() {
                    act = Some(Act::PickArea);
                }
            });
            if st.visible {
                ui.label(
                    RichText::new(match &st.area {
                        Some(_) => tr("Area chosen."),
                        None => tr("No area chosen yet — drag a rectangle on the page."),
                    })
                    .size(12.0)
                    .color(self.pal.text_dim),
                );
            }
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    tr("The signed file is saved right away under a name you choose, and signing cannot be undone. \
                     Other programs will show your certificate as “not trusted” unless the reader trusts it; \
                     BergPDF only creates the signature."),
                )
                .size(11.5)
                .color(self.pal.text_dim),
            );
            if let Some(e) = &st.error {
                ui.add_space(4.0);
                ui.colored_label(self.pal.danger, e);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        st.cert_path.is_some(),
                        egui::Button::new(tr("Sign and save as…")),
                    )
                    .clicked()
                {
                    act = Some(Act::Sign);
                }
                if ui.button(tr("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    act = Some(Act::Cancel);
                }
            });
        });
        match act {
            None => true,
            Some(Act::Cancel) => {
                st.password.clear();
                false
            }
            Some(Act::PickCert) => {
                if let Some(p) = platform::dialogs::pick_certificate() {
                    st.cert_path = Some(p);
                    st.signer_hint = None;
                    st.error = None;
                }
                true
            }
            Some(Act::Check) => {
                match Self::load_identity(st) {
                    Ok(id) => {
                        st.signer_hint = Some(tf!(
                            "Signs as {} ({} certificate(s) in the file)",
                            id.common_name(),
                            id.chain_len()
                        ));
                        st.error = None;
                    }
                    Err(e) => {
                        st.signer_hint = None;
                        st.error = Some(e);
                    }
                }
                true
            }
            Some(Act::PickArea) => {
                self.sign_return = Some(Box::new(st.clone()));
                st.password.clear();
                self.set_tool(Tool::SignArea);
                self.notify(tr(
                    "Drag the rectangle where the signature should appear. Esc goes back.",
                ));
                false
            }
            Some(Act::Sign) => match self.do_sign(st) {
                Ok(()) => {
                    st.password.clear();
                    false
                }
                Err(e) => {
                    st.error = Some(e);
                    true
                }
            },
        }
    }

    fn load_identity(st: &SignDialogState) -> Result<Identity, String> {
        let path = st
            .cert_path
            .as_ref()
            .ok_or(tr("Choose a certificate file first."))?;
        let bytes =
            std::fs::read(path).map_err(|e| tf!("Could not read the certificate file: {}", e))?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err(tr("That file is too large to be a certificate.").into());
        }
        Identity::from_pkcs12(&bytes, &st.password).map_err(|e| e.to_string())
    }

    fn do_sign(&mut self, st: &SignDialogState) -> Result<(), String> {
        let id = Self::load_identity(st)?;
        if st.visible && st.area.is_none() {
            return Err(tr(
                "Choose where the signature is shown, or untick “Show the signature on a page”.",
            )
            .into());
        }
        let ti = self.active;
        let tab = self.tabs.get(ti).ok_or(tr("No document is open."))?;
        let suggested = format!("{}-signed.pdf", tab.session.title.trim_end_matches(".pdf"));
        let start = tab
            .session
            .path
            .as_ref()
            .and_then(|p| p.parent().map(std::path::Path::to_path_buf));
        let Some(dest) = platform::dialogs::pick_save_pdf(&suggested, start.as_deref()) else {
            return Err(tr("Signing was cancelled: no file name was chosen.").into());
        };
        let opts = SignOptions {
            reason: st.reason.trim().to_string(),
            location: st.location.trim().to_string(),
            contact: st.contact.trim().to_string(),
            page: st.area.map(|(p, _)| p),
            rect: st.visible.then(|| st.area.map(|(_, r)| r)).flatten(),
        };
        self.tabs[ti]
            .session
            .sign_and_save(&dest, &id, &opts)
            .map_err(|e| e.to_string())?;
        self.notify(tf!(
            "Signed as {} and saved to {}.",
            id.common_name(),
            dest.display()
        ));
        Ok(())
    }

    /// Area tool for a visible signature: drag a rectangle.
    pub fn sign_area_tool(&mut self, response: &egui::Response, vc: &ViewCtx, pos: Option<Pos2>) {
        let ti = self.active;
        let Some(pos) = pos else { return };
        if response.drag_started_by(egui::PointerButton::Primary) {
            let Some(i) = vc.page_at(pos) else { return };
            let p = vc.screen_to_pdf(i, pos);
            self.tabs[ti].ui.interaction = Interaction::Draw {
                page: vc.pages[i].id,
                tool: Tool::SignArea,
                points: vec![p, p],
            };
        } else if response.dragged_by(egui::PointerButton::Primary)
            && let Interaction::Draw { points, page, .. } = &mut self.tabs[ti].ui.interaction
            && let Some(i) = vc.pages.iter().position(|p| p.id == *page)
        {
            points.truncate(1);
            points.push(vc.screen_to_pdf(i, pos));
        } else if response.drag_stopped()
            && let Interaction::Draw { page, points, .. } =
                std::mem::take(&mut self.tabs[ti].ui.interaction)
            && points.len() == 2
        {
            let r = PRect::new(points[0].x, points[0].y, points[1].x, points[1].y).abs();
            if r.width() < 40.0 || r.height() < 20.0 {
                self.notify(tr("Drag a larger rectangle (at least 40 × 20 points)."));
                return;
            }
            self.finish_sign_area(Some((page, r)));
        }
    }

    /// Return to the sign dialog, with a new area (or the old one when `None`).
    pub fn finish_sign_area(&mut self, area: Option<(pdf_engine::doc::PageId, PRect)>) {
        if let Some(mut st) = self.sign_return.take() {
            if area.is_some() {
                st.area = area;
            }
            self.set_tool(Tool::Select);
            self.dialog = Some(Dialog::Sign(st));
        }
    }

    // ---- listing signatures --------------------------------------------------------------

    /// Open the signatures list for the active document.
    pub fn open_signatures(&mut self) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let list = list_signatures(tab.session.doc());
        self.dialog = Some(Dialog::Signatures(list));
    }

    /// The signatures dialog. Returns whether it stays open.
    pub fn dialog_signatures(&mut self, ctx: &egui::Context, list: &[SignatureInfo]) -> bool {
        let mut close = false;
        modal(ctx, "signatures", |ui| {
            ui.heading(tr("Digital signatures"));
            if list.is_empty() {
                ui.label(tr("This document has no digital signatures."));
            }
            for s in list {
                ui.group(|ui| {
                    let (mark, text, color) = match &s.status {
                        SignatureStatus::IntegrityOk if s.covers_whole_file => (
                            "✔",
                            tr("The signed content is unchanged and nothing was added after signing.").to_string(),
                            OK_GREEN,
                        ),
                        SignatureStatus::IntegrityOk => (
                            "✔",
                            tr("The signed content is unchanged. The file was changed after signing (new revisions were added).")
                                .to_string(),
                            OK_GREEN,
                        ),
                        SignatureStatus::DigestMismatch => (
                            "✖",
                            tr("The signed content was altered after signing.").to_string(),
                            self.pal.danger,
                        ),
                        SignatureStatus::BadSignature => (
                            "✖",
                            tr("The signature itself is not valid.").to_string(),
                            self.pal.danger,
                        ),
                        SignatureStatus::Unsupported(why) => ("?", tf!("Could not be checked: {}", why), self.pal.text_dim),
                    };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(mark).color(color).strong());
                        ui.label(RichText::new(if s.signer.is_empty() { tr("Unknown signer") } else { &s.signer }).strong());
                        ui.label(RichText::new(format!("({})", s.field)).size(11.0).color(self.pal.text_dim));
                    });
                    ui.label(RichText::new(text).color(color).size(12.5));
                    for (k, v) in [
                        (tr("Time claimed by the signer"), s.claimed_time.clone().unwrap_or_default()),
                        (tr("Reason"), s.reason.clone()),
                        (tr("Location"), s.location.clone()),
                    ] {
                        if !v.is_empty() {
                            ui.label(RichText::new(format!("{k}: {v}")).size(12.0).color(self.pal.text_dim));
                        }
                    }
                });
            }
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    tr("BergPDF checks that the signed bytes still match the signature. It does not check whether the \
                     signer's certificate is trusted, valid today, or revoked, and the signing time is only what the \
                     signer claimed. Confirm who the signer is by other means."),
                )
                .size(11.5)
                .color(self.pal.text_dim),
            );
            ui.add_space(6.0);
            if ui.button(tr("Close")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
        });
        !close
    }

    // ---- handwritten signature -----------------------------------------------------------

    /// Open the drawing pad.
    pub fn open_draw_signature(&mut self) {
        self.dialog = Some(Dialog::DrawSignature(Box::default()));
    }

    /// The drawing-pad dialog. Returns whether it stays open.
    pub fn dialog_draw_signature(&mut self, ctx: &egui::Context, st: &mut DrawSigState) -> bool {
        let mut choice: Option<bool> = None;
        modal(ctx, "draw_signature", |ui| {
            ui.heading(tr("Draw your signature"));
            ui.label(
                RichText::new(tr("Sign inside the box with the mouse, pen or finger. This is a picture of your signature, not a digital signature."))
                    .size(12.0)
                    .color(self.pal.text_dim),
            );
            ui.add_space(4.0);
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::new(440.0, 170.0), Sense::click_and_drag());
            let p = ui.painter();
            p.rect_filled(rect, 4.0, Color32::WHITE);
            p.rect_stroke(
                rect,
                4.0,
                Stroke::new(1.0, self.pal.border),
                egui::StrokeKind::Inside,
            );
            let base = rect.min + Vec2::new(16.0, 130.0);
            p.line_segment(
                [base, base + Vec2::new(408.0, 0.0)],
                Stroke::new(1.0, Color32::from_gray(200)),
            );
            if resp.drag_started()
                && let Some(pos) = resp.interact_pointer_pos()
            {
                st.strokes
                    .push(vec![[pos.x - rect.min.x, pos.y - rect.min.y]]);
            } else if resp.dragged()
                && let Some(pos) = resp.interact_pointer_pos()
                && let Some(cur) = st.strokes.last_mut()
                && cur.len() < editor_core::handwriting::MAX_POINTS_PER_STROKE
            {
                let q = [pos.x - rect.min.x, pos.y - rect.min.y];
                if cur
                    .last()
                    .is_none_or(|l| (l[0] - q[0]).hypot(l[1] - q[1]) > 1.5)
                {
                    cur.push(q);
                }
            }
            let ink = Stroke::new(2.2, Color32::from_rgb(20, 30, 110));
            for s in &st.strokes {
                let pts: Vec<Pos2> = s.iter().map(|q| rect.min + Vec2::new(q[0], q[1])).collect();
                if pts.len() >= 2 {
                    p.add(egui::epaint::PathShape::line(pts, ink));
                } else if let Some(q) = pts.first() {
                    p.circle_filled(*q, 1.2, ink.color);
                }
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(tr("Clear")).clicked() {
                    st.strokes.clear();
                }
                if ui
                    .add_enabled(
                        !st.strokes.is_empty(),
                        egui::Button::new(tr("Save signature")),
                    )
                    .clicked()
                {
                    choice = Some(true);
                }
                if ui.button(tr("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    choice = Some(false);
                }
            });
            if let Some(e) = &st.error {
                ui.colored_label(self.pal.danger, e);
            }
        });
        match choice {
            None => true,
            Some(false) => false,
            Some(true) => match HandwrittenSignature::from_canvas(&st.strokes) {
                Some(sig) => {
                    match sig.to_toml() {
                        Ok(t) => {
                            if let Err(e) = platform::dirs::write_text_atomic(&signature_file(), &t)
                            {
                                self.notify_error(tf!("Could not save the signature: {}", e));
                            }
                        }
                        Err(e) => self.notify_error(tf!("Could not save the signature: {}", e)),
                    }
                    self.handwriting = sig;
                    self.notify(tr(
                        "Signature saved. Use Place Signature to put it on a page.",
                    ));
                    self.set_tool(Tool::PlaceSignature);
                    false
                }
                None => {
                    st.error =
                        Some(tr("That looks like a dot, not a signature. Draw it again.").into());
                    true
                }
            },
        }
    }

    /// Place the saved handwritten signature where the user clicks.
    pub fn place_signature_tool(
        &mut self,
        response: &egui::Response,
        vc: &ViewCtx,
        pos: Option<Pos2>,
    ) {
        if !response.clicked_by(egui::PointerButton::Primary) {
            return;
        }
        let Some(pos) = pos else { return };
        let Some(i) = vc.page_at(pos) else { return };
        if self.handwriting.is_empty() {
            self.notify(tr("Draw your signature first (Sign → Draw Signature)."));
            self.open_draw_signature();
            return;
        }
        let page = vc.pages[i].id;
        let c = vc.screen_to_pdf(i, pos);
        let strokes: Vec<Vec<Point>> = self
            .handwriting
            .to_user_space((c.x, c.y), PLACED_WIDTH_PT)
            .into_iter()
            .map(|s| s.into_iter().map(|(x, y)| Point::new(x, y)).collect())
            .collect();
        let mut spec = AnnotationSpec::new(AnnotationKind::Ink { strokes });
        spec.color = Rgb(0.08, 0.12, 0.45);
        spec.border_width = 1.4;
        spec.author = self.prefs.author.clone();
        spec.subject = tr("Handwritten signature").into();
        let ti = self.active;
        match self.tabs[ti].session.execute(tr("Place signature"), |tx| {
            annot::add_annotation(tx, page, &spec)
        }) {
            Ok(id) => {
                self.tabs[ti].session.selection.annotations = vec![(page, id)];
                self.set_tool(Tool::Select);
            }
            Err(e) => self.notify_error(e.to_string()),
        }
    }
}
