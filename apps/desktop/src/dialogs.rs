//! Modal dialogs, the command palette and transient notices.

use crate::app::OS;
use crate::i18n::tr;
use crate::state::*;
use crate::tf;
use editor_core::command::{self, CommandId as C, Key, PaletteItem, Shortcut};
use editor_core::prefs::{
    DefaultZoom, Density, GfxBackend, Language, PresentChoice, SETTINGS, ThemeChoice, UpdateCheck,
    Workspace,
};
use editor_core::tools::Tool;
use egui::{Align2, Color32, RichText, Vec2};

enum Action {
    None,
    Close,
}

impl App {
    pub fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let mut keep = true;
        match &mut dialog {
            Dialog::ConfirmClose { tab } => {
                let tab = *tab;
                let name = self
                    .tabs
                    .get(tab)
                    .map(|t| t.session.title.clone())
                    .unwrap_or_default();
                let mut choice: Option<u8> = None;
                modal(ctx, "confirm_close", |ui| {
                    ui.heading(tr("Save changes?"));
                    ui.label(tf!("“{}” has unsaved changes.", name));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button(tr("Save")).clicked() {
                            choice = Some(0);
                        }
                        if ui.button(tr("Don't save")).clicked() {
                            choice = Some(1);
                        }
                        if ui.button(tr("Cancel")).clicked() {
                            choice = Some(2);
                        }
                    });
                });
                match choice {
                    Some(0) => {
                        keep = false;
                        self.resolve_close(tab, true);
                    }
                    Some(1) => {
                        keep = false;
                        self.resolve_close(tab, false);
                    }
                    Some(_) => keep = false,
                    None => {}
                }
            }
            Dialog::ConfirmQuit => {
                let dirty: Vec<String> = self
                    .tabs
                    .iter()
                    .filter(|t| t.session.is_dirty())
                    .map(|t| t.session.title.clone())
                    .collect();
                let mut choice: Option<u8> = None;
                modal(ctx, "confirm_quit", |ui| {
                    ui.heading(tr("Quit with unsaved changes?"));
                    for n in &dirty {
                        ui.label(format!("• {n}"));
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button(tr("Save all and quit")).clicked() {
                            choice = Some(0);
                        }
                        if ui.button(tr("Quit without saving")).clicked() {
                            choice = Some(1);
                        }
                        if ui.button(tr("Cancel")).clicked() {
                            choice = Some(2);
                        }
                    });
                });
                match choice {
                    Some(0) => {
                        keep = false;
                        let mut all_ok = true;
                        for i in 0..self.tabs.len() {
                            if self.tabs[i].session.is_dirty() {
                                let old = self.active;
                                self.active = i;
                                self.save_active();
                                self.active = old;
                                all_ok &= !self.tabs[i].session.is_dirty();
                            }
                        }
                        if all_ok {
                            self.quit_confirmed = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                    Some(1) => {
                        keep = false;
                        self.quit_confirmed = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    Some(_) => keep = false,
                    None => {}
                }
            }
            Dialog::Error { title, detail } => {
                let mut act = Action::None;
                modal(ctx, "error_dialog", |ui| {
                    ui.heading(title.as_str());
                    ui.add_space(4.0);
                    ui.label(detail.as_str());
                    ui.add_space(10.0);
                    if ui.button("OK").clicked() {
                        act = Action::Close;
                    }
                });
                if matches!(act, Action::Close) {
                    keep = false;
                }
            }
            Dialog::OpenWarnings { title, lines } => {
                let mut act = Action::None;
                modal(ctx, "warnings_dialog", |ui| {
                    ui.heading(tf!("About “{}”", title));
                    ui.add_space(4.0);
                    for l in lines.iter() {
                        ui.label(format!("• {l}"));
                        ui.add_space(2.0);
                    }
                    ui.add_space(10.0);
                    if ui.button("OK").clicked() {
                        act = Action::Close;
                    }
                });
                if matches!(act, Action::Close) {
                    keep = false;
                }
            }
            Dialog::ConfirmLink { uri } => {
                let allowed = platform::links::is_allowed_uri(uri);
                let mut choice: Option<bool> = None;
                modal(ctx, "link_dialog", |ui| {
                    ui.heading(tr("Open external link?"));
                    ui.label(tr(
                        "This document wants to open a web address in your browser:",
                    ));
                    ui.add_space(4.0);
                    ui.add(
                        egui::TextEdit::multiline(&mut uri.clone())
                            .desired_rows(2)
                            .desired_width(380.0)
                            .interactive(false),
                    );
                    if !allowed {
                        ui.colored_label(
                            self.pal.danger,
                            tr("This kind of link cannot be opened from a document."),
                        );
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(allowed, egui::Button::new(tr("Open in browser")))
                            .clicked()
                        {
                            choice = Some(true);
                        }
                        if ui.button(tr("Cancel")).clicked() {
                            choice = Some(false);
                        }
                    });
                });
                if let Some(open) = choice {
                    keep = false;
                    if open && let Err(e) = platform::links::open_confirmed(uri) {
                        self.notify_error(e);
                    }
                }
            }
            Dialog::GoToPage { text } => {
                let mut go: Option<bool> = None;
                modal(ctx, "goto_dialog", |ui| {
                    ui.heading(tr("Go to page"));
                    let r = ui.add(
                        egui::TextEdit::singleline(text)
                            .hint_text(tr("Page number"))
                            .desired_width(160.0),
                    );
                    r.request_focus();
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button(tr("Go")).clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Enter))
                        {
                            go = Some(true);
                        }
                        if ui.button(tr("Cancel")).clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Escape))
                        {
                            go = Some(false);
                        }
                    });
                });
                if let Some(g) = go {
                    keep = false;
                    if g && let Ok(n) = text.trim().parse::<usize>() {
                        self.go_to(n.saturating_sub(1));
                    }
                }
            }
            Dialog::ConfirmDeletePages { count } => {
                let n = *count;
                let mut choice: Option<bool> = None;
                modal(ctx, "delete_pages", |ui| {
                    ui.heading(tf!("Delete {} page(s)?", n));
                    ui.label(tr(
                        "You can undo this with Undo until you close the document.",
                    ));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button(RichText::new(tr("Delete")).color(self.pal.danger))
                            .clicked()
                        {
                            choice = Some(true);
                        }
                        if ui.button(tr("Cancel")).clicked() {
                            choice = Some(false);
                        }
                    });
                });
                if let Some(c) = choice {
                    keep = false;
                    if c {
                        self.delete_pages_confirmed();
                    }
                }
            }
            Dialog::TextEntry {
                page,
                tool,
                rect,
                text,
                callout,
            } => {
                let (page, tool, rect) = (*page, *tool, *rect);
                let callout = callout.clone();
                let mut choice: Option<bool> = None;
                let title = match tool {
                    Tool::Note => tr("Sticky note"),
                    Tool::Stamp => tr("Stamp"),
                    Tool::Callout => tr("Callout text"),
                    _ => tr("Text box"),
                };
                modal(ctx, "text_entry", |ui| {
                    ui.heading(title);
                    if tool == Tool::Stamp {
                        ui.horizontal_wrapped(|ui| {
                            for s in [
                                "APPROVED",
                                "DRAFT",
                                "CONFIDENTIAL",
                                "REVIEWED",
                                "FINAL",
                                "VOID",
                            ] {
                                if ui.button(s).clicked() {
                                    *text = s.into();
                                }
                            }
                        });
                    }
                    let boxed = matches!(tool, Tool::FreeText | Tool::Callout);
                    let mut font_style = self.prefs.tool_defaults.font_style();
                    if boxed {
                        ui.horizontal(|ui| {
                            ui.label(tr("Font"));
                            if crate::fontpick::font_picker(ui, "textbox_font", &mut font_style) {
                                // Remembered as the default for the next text box too.
                                self.prefs.tool_defaults.set_font_style(font_style);
                                self.prefs_dirty = true;
                            }
                        });
                    }
                    let mut edit = egui::TextEdit::multiline(text)
                        .desired_rows(if tool == Tool::Stamp { 1 } else { 5 })
                        .desired_width(380.0)
                        .hint_text(tr("Type here…"));
                    if boxed {
                        edit = edit.font(egui::FontId::new(
                            self.prefs.tool_defaults.font_size.clamp(10.0, 22.0) as f32,
                            crate::fontpick::egui_family(font_style),
                        ));
                    }
                    let r = ui.add(edit);
                    if self.frame_counter.is_multiple_of(2) && !r.has_focus() {
                        r.request_focus();
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let ok = !text.trim().is_empty() || tool == Tool::Note;
                        if ui.add_enabled(ok, egui::Button::new(tr("Add"))).clicked() {
                            choice = Some(true);
                        }
                        if ui.button(tr("Cancel")).clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Escape))
                        {
                            choice = Some(false);
                        }
                    });
                });
                if let Some(c) = choice {
                    keep = false;
                    if c {
                        let t = text.clone();
                        self.commit_text_entry(page, tool, rect, t, callout);
                    }
                }
            }
            Dialog::AddText {
                page,
                at,
                text,
                size,
                font,
                turns,
            } => {
                let (page, at) = (*page, *at);
                let mut choice: Option<bool> = None;
                modal(ctx, "add_text", |ui| {
                    ui.heading(tr("Add text"));
                    ui.label(
                        RichText::new(tr(
                            "Adds real text to the page content (not an annotation).",
                        ))
                        .size(12.0)
                        .color(self.pal.text_dim),
                    );
                    ui.horizontal(|ui| {
                        ui.label(tr("Font"));
                        crate::fontpick::font_picker(ui, "add_text_font", font);
                    });
                    // The box previews the chosen font (at most 22 px so it stays tidy).
                    let r = ui.add(
                        egui::TextEdit::multiline(text)
                            .desired_rows(4)
                            .desired_width(380.0)
                            .font(egui::FontId::new(
                                size.clamp(10.0, 22.0) as f32,
                                crate::fontpick::egui_family(*font),
                            ))
                            .hint_text(tr("Type here…")),
                    );
                    if self.frame_counter.is_multiple_of(2) && !r.has_focus() {
                        r.request_focus();
                    }
                    ui.horizontal(|ui| {
                        ui.label(tr("Size"));
                        ui.add(egui::DragValue::new(size).range(4.0..=200.0).suffix(" pt"));
                        ui.separator();
                        ui.label(tr("Direction"));
                        if ui
                            .button("⟲")
                            .on_hover_text(tr("Rotate 90° counter-clockwise"))
                            .clicked()
                        {
                            *turns = (*turns + 1).rem_euclid(4);
                        }
                        if ui
                            .button("⟳")
                            .on_hover_text(tr("Rotate 90° clockwise"))
                            .clicked()
                        {
                            *turns = (*turns + 3).rem_euclid(4);
                        }
                        ui.label(match *turns {
                            0 => "horizontal",
                            1 => "up",
                            2 => tr("upside down"),
                            _ => "down",
                        });
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!text.trim().is_empty(), egui::Button::new(tr("Add")))
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
                });
                if let Some(c) = choice {
                    keep = false;
                    if c {
                        let (t, s, f, tn) = (text.clone(), *size, *font, *turns);
                        // Remember the choice for the next piece of text.
                        self.prefs.tool_defaults.set_font_style(f);
                        self.prefs_dirty = true;
                        self.commit_add_text(page, at, &t, s, f, tn);
                    }
                }
            }
            Dialog::AiConsent => {
                keep = self.dialog_ai_consent(ctx);
            }
            Dialog::Translate(st) => {
                keep = self.dialog_translate(ctx, st);
            }
            Dialog::PdfA(st) => {
                keep = self.dialog_pdfa(ctx, st);
            }
            Dialog::Optimize(st) => {
                keep = self.dialog_optimize(ctx, st);
            }
            Dialog::Ocr(st) => {
                keep = self.dialog_ocr(ctx, st);
            }
            Dialog::OcrProgress => {
                keep = self.dialog_ocr_progress(ctx);
            }
            Dialog::Sign(st) => {
                keep = self.dialog_sign(ctx, st);
            }
            Dialog::Signatures(list) => {
                let l = list.clone();
                keep = self.dialog_signatures(ctx, &l);
            }
            Dialog::DrawSignature(st) => {
                keep = self.dialog_draw_signature(ctx, st);
            }
            Dialog::Properties(st) => {
                keep = self.dialog_properties(ctx, st);
            }
            Dialog::ConfirmFlatten { fields } => {
                let n = *fields;
                keep = self.dialog_confirm_flatten(ctx, n);
            }
            Dialog::Recovery(entries) => {
                keep = self.dialog_recovery(ctx, entries);
            }
            Dialog::FillField(st) => {
                keep = self.dialog_fill_field(ctx, st);
            }
            Dialog::FormPolicy(st) => {
                keep = self.dialog_form_policy(ctx, st);
            }
            Dialog::Scale(st) => {
                keep = self.dialog_scale(ctx, st);
            }
            Dialog::About => {
                let mut close = false;
                modal(ctx, "about", |ui| {
                    let (r, _) =
                        ui.allocate_exact_size(egui::vec2(64.0, 64.0), egui::Sense::hover());
                    crate::icons::paint_logo(ui.painter(), r, self.pal.text, self.pal.accent);
                    ui.heading("BergPDF");
                    ui.label(tf!("Version {}", env!("CARGO_PKG_VERSION")));
                    ui.add_space(6.0);
                    ui.label(
                        tr("Free software under the GNU General Public License, version 3 or (at your option) any later version. It comes with NO WARRANTY. You may share and change it under those terms; the source code of this version is at https://github.com/markvdberg0/BergPDF and the licence text is the file LICENSE next to the program."),
                    );
                    ui.add_space(8.0);
                    ui.label(tr("Offline-first. No account, no telemetry, no network access for core editing."));
                    ui.add_space(6.0);
                    ui.label(RichText::new(tr("Native components")).strong());
                    ui.label(tr("Rust (egui/eframe, wgpu, hayro, lopdf). Native dependencies are the OS windowing/graphics stack and GPU drivers; full audit in docs/DEPENDENCIES.md."));
                    ui.add_space(6.0);
                    ui.label(RichText::new(tr("Bundled font")).strong());
                    ui.label(tr("Fonts for new and replaced text, embedded as subsets in your documents: Liberation Sans, Serif and Mono (SIL Open Font License 1.1), DejaVu Sans and Serif (Bitstream Vera licence). DejaVu Sans is also used for interface symbols. Licence texts: assets/fonts/LICENSE-Liberation.txt and LICENSE-DejaVu.txt."));
                    ui.add_space(10.0);
                    if ui.button(tr("Close")).clicked() {
                        close = true;
                    }
                });
                if close {
                    keep = false;
                }
            }
            Dialog::UpdateAsk => {
                keep = self.dialog_update_ask(ctx);
            }
            Dialog::Update(st) => {
                keep = self.dialog_update(ctx, st);
            }
            Dialog::FirstRun => {
                let mut done = false;
                modal(ctx, "first_run", |ui| {
                    ui.heading(tr("Welcome to BergPDF"));
                    ui.label(tr("Choose how much to show. You can change this any time in Preferences, and every tool stays reachable through Search commands."));
                    ui.add_space(8.0);
                    ui.radio_value(
                        &mut self.prefs.workspace,
                        Workspace::Essential,
                        tr("Essential — the common tools: open, markup, organize pages, save"),
                    );
                    ui.radio_value(
                        &mut self.prefs.workspace,
                        Workspace::Professional,
                        tr("Professional — all tools and panels"),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(tr("Appearance"));
                        ui.selectable_value(
                            &mut self.prefs.theme,
                            ThemeChoice::System,
                            tr("System"),
                        );
                        ui.selectable_value(&mut self.prefs.theme, ThemeChoice::Light, tr("Light"));
                        ui.selectable_value(&mut self.prefs.theme, ThemeChoice::Dark, tr("Dark"));
                    });
                    ui.add_space(10.0);
                    if ui.button(tr("Get started")).clicked() {
                        done = true;
                    }
                });
                // Live preview of the theme choice.
                self.restyle(ctx);
                if done {
                    self.prefs.first_run_done = true;
                    self.prefs_dirty = true;
                    keep = false;
                }
            }
            Dialog::Preferences { filter } => {
                let mut close = false;
                let mut changed = false;
                let mut check_now = false;
                let f = filter.to_lowercase();
                let show = |key: &str| -> bool {
                    f.is_empty()
                        || SETTINGS.iter().any(|s| {
                            s.key == key
                                && format!(
                                    "{} {} {} {} {}",
                                    s.title,
                                    tr(s.title),
                                    s.description,
                                    tr(s.description),
                                    s.keywords
                                )
                                .to_lowercase()
                                .contains(&f)
                        })
                };
                egui::Window::new(tr("Preferences")).collapsible(false).resizable(true).default_width(520.0).anchor(Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
                    ui.add(egui::TextEdit::singleline(filter).hint_text(tr("Search settings")).desired_width(f32::INFINITY));
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                        if show("language") {
                            section(ui, tr("Language"), tr("Interface language: English, Nederlands or Deutsch (or follow the system)."));
                            let shown = match self.prefs.language {
                                Language::System => tr("System default"),
                                Language::English => "English",
                                Language::Dutch => "Nederlands",
                                Language::German => "Deutsch",
                            };
                            egui::ComboBox::from_id_salt("language").selected_text(shown).show_ui(ui, |ui| {
                                changed |= ui.selectable_value(&mut self.prefs.language, Language::System, tr("System default")).changed();
                                changed |= ui.selectable_value(&mut self.prefs.language, Language::English, "English").changed();
                                changed |= ui.selectable_value(&mut self.prefs.language, Language::Dutch, "Nederlands").changed();
                                changed |= ui.selectable_value(&mut self.prefs.language, Language::German, "Deutsch").changed();
                            });
                        }
                        if show("updates") {
                            section(ui, tr("Updates"), tr("Let BergPDF look for a newer version when it starts (once a day). Only the program name and version are sent; nothing is downloaded or installed."));
                            let mut on = self.prefs.update_check == UpdateCheck::On;
                            if ui.checkbox(&mut on, tr("Look for updates when BergPDF starts")).changed() {
                                self.prefs.update_check = if on { UpdateCheck::On } else { UpdateCheck::Off };
                                self.prefs.last_update_check = 0;
                                changed = true;
                            }
                            if ui.button(tr("Check for Updates…")).clicked() {
                                check_now = true;
                            }
                        }
                        if show("theme") {
                            section(ui, tr("Application theme"), tr("Light, Dark or follow the system. Never changes document colours."));
                            ui.horizontal(|ui| {
                                changed |= ui.selectable_value(&mut self.prefs.theme, ThemeChoice::System, tr("System")).changed();
                                changed |= ui.selectable_value(&mut self.prefs.theme, ThemeChoice::Light, tr("Light")).changed();
                                changed |= ui.selectable_value(&mut self.prefs.theme, ThemeChoice::Dark, tr("Dark")).changed();
                            });
                        }
                        if show("dark_page_filter") {
                            section(ui, tr("Dark page view"), tr("Comfortable dark reading filter. Display only — saved PDFs are never changed."));
                            changed |= ui.checkbox(&mut self.prefs.dark_page_filter, tr("Use dark page view")).changed();
                        }
                        if show("default_font") {
                            section(ui, tr("Default font for new text"), tr("Font, bold and italic used when you add text or a text box. Text you add is stored with an embedded subset of the font, so it looks the same everywhere."));
                            let mut st = self.prefs.tool_defaults.font_style();
                            ui.horizontal(|ui| {
                                if crate::fontpick::font_picker(ui, "default_font_pick", &mut st) {
                                    self.prefs.tool_defaults.set_font_style(st);
                                    changed = true;
                                }
                            });
                        }
                        if show("snap_to_geometry") {
                            section(ui, tr("Snap to drawing geometry"), tr("Measurements jump to line ends, corners, intersections and midpoints of the page when the pointer is close."));
                            changed |= ui.checkbox(&mut self.prefs.snap_to_geometry, tr("Snap to drawing geometry")).changed();
                        }
                        if show("graphics") {
                            section(ui, tr("Graphics (drawing backend, frame pacing)"), tr("If resizing the window or zooming feels slow, try another drawing API (restart needed) or uncapped frames."));
                            let adapter = self.gpu_info.clone().unwrap_or_else(|| "unknown".into());
                            ui.label(RichText::new(tf!("In use: {}", adapter)).size(12.0));
                            if adapter.contains("Cpu") || adapter.contains("llvmpipe") || adapter.contains("WARP") || adapter.contains("Basic Render") {
                                ui.colored_label(self.pal.danger, tr("This is a software renderer, not a graphics card. Install the graphics driver of your computer's maker; drawing will stay slow until then."));
                            }
                            ui.horizontal(|ui| {
                                ui.label(tr("Drawing API"));
                                egui::ComboBox::from_id_salt("gfx_backend")
                                    .selected_text(tr(self.prefs.graphics.backend.title()))
                                    .show_ui(ui, |ui| {
                                        for b in [GfxBackend::Auto, GfxBackend::Dx12, GfxBackend::Vulkan, GfxBackend::Gl] {
                                            changed |= ui.selectable_value(&mut self.prefs.graphics.backend, b, tr(b.title())).changed();
                                        }
                                    });
                                ui.label(RichText::new(tr("(applies after restart)")).size(11.0).color(self.pal.text_dim));
                            });
                            ui.horizontal(|ui| {
                                ui.label(tr("Frame pacing"));
                                egui::ComboBox::from_id_salt("gfx_present")
                                    .selected_text(tr(self.prefs.graphics.present.title()))
                                    .show_ui(ui, |ui| {
                                        for p in [PresentChoice::Smooth, PresentChoice::LowLatency, PresentChoice::Uncapped] {
                                            changed |= ui.selectable_value(&mut self.prefs.graphics.present, p, tr(p.title())).changed();
                                        }
                                    });
                            });
                        }
                        if show("ai") {
                            section(ui, tr("PDF Copilot (AI provider and key)"), tr("Use your own OpenAI or Anthropic account (or a server of your own) for Copilot and Translate."));
                            changed |= self.ai_prefs_section(ui, ctx);
                        }
                        if show("density") {
                            section(ui, tr("Interface density"), tr("Compact, Comfortable or Touch/Pen hit targets."));
                            ui.horizontal(|ui| {
                                changed |= ui.selectable_value(&mut self.prefs.density, Density::Compact, tr("Compact")).changed();
                                changed |= ui.selectable_value(&mut self.prefs.density, Density::Comfortable, tr("Comfortable")).changed();
                                changed |= ui.selectable_value(&mut self.prefs.density, Density::Touch, tr("Touch / Pen")).changed();
                            });
                        }
                        if show("ui_scale") {
                            section(ui, tr("Interface scale"), tr("Scales toolbar icons, text and hit targets. Page rendering stays sharp at any scale."));
                            let r = ui.add(egui::Slider::new(&mut self.prefs.ui_scale, 0.75..=2.5).text("scale"));
                            // Rescaling re-renders every page, so it is applied when the slider is released.
                            changed |= (r.changed() && !r.dragged()) || r.drag_stopped();
                        }
                        if show("workspace") {
                            section(ui, tr("Workspace"), tr("Essential shows the common tools; Professional shows everything. Both can reach every command via search."));
                            ui.horizontal(|ui| {
                                changed |= ui.selectable_value(&mut self.prefs.workspace, Workspace::Essential, tr("Essential")).changed();
                                changed |= ui.selectable_value(&mut self.prefs.workspace, Workspace::Professional, tr("Professional")).changed();
                            });
                        }
                        if show("default_zoom") {
                            section(ui, tr("Zoom when opening a document"), tr("Applies to documents you open from now on; you can still zoom freely."));
                            ui.horizontal(|ui| {
                                changed |= ui.selectable_value(&mut self.prefs.default_zoom, DefaultZoom::FitPage, tr("Fit page")).changed();
                                changed |= ui.selectable_value(&mut self.prefs.default_zoom, DefaultZoom::FitWidth, tr("Fit width")).changed();
                                changed |= ui.selectable_value(&mut self.prefs.default_zoom, DefaultZoom::Actual, "100 %").changed();
                            });
                        }
                        if show("author") {
                            section(ui, tr("Author name"), tr("Stored on comments and markup you create."));
                            changed |= ui.text_edit_singleline(&mut self.prefs.author).changed();
                        }
                        if show("render_cache_mb") {
                            section(ui, tr("Render cache size"), tr("Memory budget for cached page tiles."));
                            changed |= ui.add(egui::Slider::new(&mut self.prefs.render_cache_mb, 64..=4096).suffix(" MiB")).changed();
                        }
                        if show("shortcuts") {
                            section(ui, tr("Keyboard shortcuts"), tr("View and change shortcuts; conflicts are flagged."));
                            if ui.button(tr("Customize shortcuts…")).clicked() {
                                close = true;
                                self.dialog_next = Some(Dialog::Shortcuts { filter: String::new(), capture: None });
                            }
                        }
                    });
                    ui.add_space(8.0);
                    if ui.button(tr("Done")).clicked() {
                        close = true;
                    }
                });
                if changed {
                    self.restyle(ctx);
                }
                if close {
                    keep = false;
                }
                if check_now {
                    keep = false;
                    self.start_update_check(true);
                }
            }
            Dialog::Shortcuts { filter, capture } => {
                let mut close = false;
                // Capture the next chord for a rebind.
                if let Some(cmd) = *capture {
                    let chord = ctx.input(|i| {
                        i.events.iter().find_map(|e| match e {
                            egui::Event::Key {
                                key,
                                pressed: true,
                                modifiers,
                                ..
                            } => crate::app::map_key_pub(*key).map(|k| Shortcut {
                                command: modifiers.command,
                                shift: modifiers.shift,
                                alt: modifiers.alt,
                                ctrl: OS.uses_command_key() && modifiers.ctrl,
                                key: k,
                            }),
                            _ => None,
                        })
                    });
                    if let Some(ch) = chord {
                        if ch.key == Key::Escape && !ch.command && !ch.alt {
                            *capture = None;
                        } else {
                            let conflicts = self.prefs.keybindings.conflicts(&ch, cmd, OS);
                            if conflicts.is_empty() {
                                self.prefs.keybindings.set(cmd, vec![ch]);
                                self.prefs_dirty = true;
                                *capture = None;
                            } else {
                                let names: Vec<_> = conflicts
                                    .iter()
                                    .map(|c| tr(command::info(*c).title))
                                    .collect();
                                self.notify_error(tf!(
                                    "{} is already used by: {}",
                                    ch.label(OS),
                                    names.join(", ")
                                ));
                                *capture = None;
                            }
                        }
                    }
                }
                let f = filter.to_lowercase();
                let cap_now = *capture;
                let mut start_capture: Option<C> = None;
                let mut reset: Option<C> = None;
                egui::Window::new(tr("Keyboard shortcuts"))
                    .collapsible(false)
                    .resizable(true)
                    .default_size([560.0, 480.0])
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                    .show(ctx, |ui| {
                        ui.label(
                            RichText::new(tf!(
                                "Shown for {}. Generated from the command registry.",
                                match OS {
                                    editor_core::platform_kind::OsKind::MacOs => "macOS",
                                    editor_core::platform_kind::OsKind::Windows => "Windows",
                                    _ => "Linux",
                                }
                            ))
                            .size(12.0)
                            .color(self.pal.text_dim),
                        );
                        ui.add(
                            egui::TextEdit::singleline(filter)
                                .hint_text(tr("Filter"))
                                .desired_width(f32::INFINITY),
                        );
                        ui.add_space(4.0);
                        egui::ScrollArea::vertical()
                            .max_height(380.0)
                            .show(ui, |ui| {
                                let mut last_cat = None;
                                for info in command::REGISTRY {
                                    if !f.is_empty()
                                        && !format!(
                                            "{} {} {}",
                                            info.title,
                                            tr(info.title),
                                            info.keywords
                                        )
                                        .to_lowercase()
                                        .contains(&f)
                                    {
                                        continue;
                                    }
                                    if !App::implemented(info.id) {
                                        continue;
                                    }
                                    if last_cat != Some(info.category) {
                                        ui.add_space(6.0);
                                        ui.label(
                                            RichText::new(tr(info.category.title()))
                                                .strong()
                                                .color(self.pal.accent),
                                        );
                                        last_cat = Some(info.category);
                                    }
                                    ui.horizontal(|ui| {
                                        ui.label(tr(info.title));
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if cap_now == Some(info.id) {
                                                    ui.label(
                                                        RichText::new(tr(
                                                            "Press new shortcut… (Esc cancels)",
                                                        ))
                                                        .color(self.pal.accent),
                                                    );
                                                } else {
                                                    if ui.small_button(tr("Change")).clicked() {
                                                        start_capture = Some(info.id);
                                                    }
                                                    if self
                                                        .prefs
                                                        .keybindings
                                                        .overrides
                                                        .contains_key(&info.id)
                                                        && ui.small_button(tr("Reset")).clicked()
                                                    {
                                                        reset = Some(info.id);
                                                    }
                                                    let labels: Vec<String> = self
                                                        .prefs
                                                        .keybindings
                                                        .shortcuts(info.id, OS)
                                                        .iter()
                                                        .map(|s| s.label(OS))
                                                        .collect();
                                                    ui.label(
                                                        RichText::new(if labels.is_empty() {
                                                            "—".to_string()
                                                        } else {
                                                            labels.join("  or  ")
                                                        })
                                                        .monospace(),
                                                    );
                                                }
                                            },
                                        );
                                    });
                                }
                            });
                        ui.add_space(6.0);
                        if ui.button(tr("Close")).clicked() {
                            close = true;
                        }
                    });
                if let Some(c) = start_capture {
                    *capture = Some(c);
                }
                if let Some(c) = reset {
                    self.prefs.keybindings.reset(c);
                    self.prefs_dirty = true;
                }
                if close {
                    keep = false;
                }
            }
        }
        if keep {
            self.dialog = Some(dialog);
        }
        if let Some(d) = self.dialog_next.take() {
            self.dialog = Some(d);
        }
    }

    pub fn palette_ui(&mut self, ctx: &egui::Context) {
        if !self.palette_open {
            return;
        }
        let results = self.palette_results();
        let n = results.len();
        let (down, up, enter, esc) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if down && n > 0 {
            self.palette_sel = (self.palette_sel + 1) % n;
        }
        if up && n > 0 {
            self.palette_sel = (self.palette_sel + n - 1) % n;
        }
        self.palette_sel = self.palette_sel.min(n.saturating_sub(1));
        let mut run: Option<PaletteItem> = None;
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::new("palette_scrim"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let r = ui.allocate_rect(screen, egui::Sense::click());
                ui.painter()
                    .rect_filled(screen, 0.0, Color32::from_black_alpha(90));
                if r.clicked() {
                    self.palette_open = false;
                }
            });
        egui::Area::new(egui::Id::new("palette"))
            .order(egui::Order::Tooltip)
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 90.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .fill(self.pal.panel)
                    .corner_radius(10)
                    .inner_margin(10)
                    .show(ui, |ui| {
                        ui.set_width(560.0);
                        let te = ui.add(
                            egui::TextEdit::singleline(&mut self.palette_query)
                                .hint_text(tr("Type a command, tool or setting…"))
                                .desired_width(f32::INFINITY)
                                .font(egui::TextStyle::Heading),
                        );
                        te.request_focus();
                        if te.changed() {
                            self.palette_sel = 0;
                        }
                        ui.add_space(6.0);
                        if results.is_empty() {
                            ui.label(
                                RichText::new(tr("No matching commands")).color(self.pal.text_dim),
                            );
                        }
                        egui::ScrollArea::vertical()
                            .max_height(360.0)
                            .show(ui, |ui| {
                                for (i, item) in results.iter().enumerate().take(40) {
                                    let (title, desc, shortcut, icon) = match item {
                                        PaletteItem::Command(c) => {
                                            let info = command::info(*c);
                                            (
                                                tr(info.title).to_string(),
                                                tr(info.description).to_string(),
                                                self.prefs
                                                    .keybindings
                                                    .primary_label(*c, OS)
                                                    .unwrap_or_default(),
                                                Some(crate::chrome::command_icon(*c)),
                                            )
                                        }
                                        PaletteItem::Setting(k) => {
                                            let s = SETTINGS.iter().find(|s| s.key == *k);
                                            (
                                                s.map_or(String::new(), |s| {
                                                    tf!("Setting: {}", tr(s.title))
                                                }),
                                                s.map_or(String::new(), |s| {
                                                    tr(s.description).to_string()
                                                }),
                                                String::new(),
                                                Some(crate::icons::Icon::Settings),
                                            )
                                        }
                                    };
                                    let sel = i == self.palette_sel;
                                    let (rect, resp) = ui.allocate_exact_size(
                                        Vec2::new(ui.available_width(), 38.0),
                                        egui::Sense::click(),
                                    );
                                    if sel || resp.hovered() {
                                        ui.painter().rect_filled(
                                            rect,
                                            6.0,
                                            if sel {
                                                self.pal.accent_soft
                                            } else {
                                                self.pal.hover
                                            },
                                        );
                                    }
                                    if let Some(ic) = icon {
                                        crate::icons::paint(
                                            ui.painter(),
                                            egui::Rect::from_center_size(
                                                egui::pos2(rect.min.x + 18.0, rect.center().y),
                                                Vec2::splat(18.0),
                                            ),
                                            ic,
                                            self.pal.text,
                                        );
                                    }
                                    ui.painter().text(
                                        egui::pos2(rect.min.x + 36.0, rect.min.y + 11.0),
                                        Align2::LEFT_CENTER,
                                        title,
                                        egui::FontId::proportional(14.0),
                                        self.pal.text,
                                    );
                                    ui.painter().text(
                                        egui::pos2(rect.min.x + 36.0, rect.min.y + 27.0),
                                        Align2::LEFT_CENTER,
                                        desc,
                                        egui::FontId::proportional(11.0),
                                        self.pal.text_dim,
                                    );
                                    if !shortcut.is_empty() {
                                        ui.painter().text(
                                            egui::pos2(rect.max.x - 10.0, rect.center().y),
                                            Align2::RIGHT_CENTER,
                                            shortcut,
                                            egui::FontId::monospace(12.0),
                                            self.pal.text_dim,
                                        );
                                    }
                                    if resp.clicked() {
                                        run = Some(item.clone());
                                    }
                                    if sel {
                                        resp.scroll_to_me(None);
                                    }
                                }
                            });
                    });
            });
        if esc {
            self.palette_open = false;
        }
        if enter && run.is_none() {
            run = results.get(self.palette_sel).cloned();
        }
        if let Some(item) = run {
            self.palette_open = false;
            match item {
                PaletteItem::Command(c) => self.run_command(ctx, c),
                PaletteItem::Setting(_) => {
                    self.dialog = Some(Dialog::Preferences {
                        filter: self.palette_query.clone(),
                    })
                }
            }
        }
    }

    fn palette_results(&self) -> Vec<PaletteItem> {
        let q = self.palette_query.trim();
        let mut out: Vec<PaletteItem> =
            command::search_commands_with(q, |c| self.command_enabled(c), tr)
                .into_iter()
                .map(PaletteItem::Command)
                .collect();
        if !q.is_empty() {
            for s in SETTINGS {
                let hay = format!(
                    "{} {} {} {} {}",
                    s.title,
                    tr(s.title),
                    s.description,
                    tr(s.description),
                    s.keywords
                );
                if command::score(q, &hay).is_some() {
                    out.push(PaletteItem::Setting(s.key));
                }
            }
        }
        out
    }

    pub fn notices_ui(&mut self, ctx: &egui::Context) {
        if self.notices.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("notices"))
            .order(egui::Order::Foreground)
            .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-14.0, -40.0))
            .show(ctx, |ui| {
                for n in &self.notices {
                    egui::Frame::popup(ui.style())
                        .fill(if n.error {
                            self.pal.danger
                        } else {
                            self.pal.text
                        })
                        .corner_radius(8)
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.label(RichText::new(&n.text).color(if n.error {
                                Color32::WHITE
                            } else {
                                self.pal.panel
                            }));
                        });
                    ui.add_space(4.0);
                }
            });
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}

fn section(ui: &mut egui::Ui, title: &str, desc: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(title).strong());
    ui.label(RichText::new(desc).size(12.0).weak());
}

pub(crate) fn modal(ctx: &egui::Context, id: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Modal::new(egui::Id::new(id)).show(ctx, |ui| {
        ui.set_max_width(460.0);
        add(ui);
    });
}

impl App {
    /// The crash-recovery dialog. Returns whether it stays open.
    fn dialog_recovery(
        &mut self,
        ctx: &egui::Context,
        entries: &mut Vec<platform::recovery::RecoveryEntry>,
    ) -> bool {
        enum Act {
            Restore(usize),
            Discard(usize),
            Later,
        }
        let mut act: Option<Act> = None;
        modal(ctx, "recovery", |ui| {
            ui.heading(tr("Recover unsaved work?"));
            ui.label(
                RichText::new(
                    tr("BergPDF found copies of documents that were open with unsaved changes when it last stopped unexpectedly."),
                )
                .size(12.0)
                .color(self.pal.text_dim),
            );
            ui.add_space(6.0);
            for (i, e) in entries.iter().enumerate() {
                let name = e
                    .original
                    .clone()
                    .unwrap_or_else(|| "Untitled document".into());
                let age = std::time::SystemTime::now()
                    .duration_since(e.modified)
                    .map_or(0, |d| d.as_secs());
                let when = if age < 3600 {
                    tf!("{} min ago", age / 60)
                } else if age < 86_400 {
                    tf!("{} h ago", age / 3600)
                } else {
                    tf!("{} days ago", age / 86_400)
                };
                ui.group(|ui| {
                    ui.label(RichText::new(name).strong());
                    ui.label(
                        RichText::new(tf!("saved {} · {} KB", when, e.len / 1024))
                            .size(12.0)
                            .color(self.pal.text_dim),
                    );
                    ui.horizontal(|ui| {
                        if ui.button(tr("Restore as new tab")).clicked() {
                            act = Some(Act::Restore(i));
                        }
                        if ui.button(tr("Discard")).clicked() {
                            act = Some(Act::Discard(i));
                        }
                    });
                });
            }
            ui.add_space(6.0);
            if ui.button(tr("Decide later")).clicked() {
                act = Some(Act::Later);
            }
        });
        match act {
            None => true,
            Some(Act::Later) => false,
            Some(Act::Discard(i)) => {
                let e = entries.remove(i);
                remove_recovery_entry(&e);
                !entries.is_empty()
            }
            Some(Act::Restore(i)) => {
                let e = entries.remove(i);
                match std::fs::read(&e.pdf)
                    .map_err(pdf_engine::EngineError::from)
                    .and_then(|b| {
                        let name = e
                            .original
                            .as_deref()
                            .and_then(|p| std::path::Path::new(p).file_stem())
                            .map_or("recovered".to_string(), |s| {
                                s.to_string_lossy().into_owned()
                            });
                        editor_core::session::DocumentSession::open_bytes(
                            b,
                            &format!("{name} (recovered).pdf"),
                        )
                    }) {
                    Ok(mut s) => {
                        s.mark_unsaved();
                        self.add_tab(s);
                        remove_recovery_entry(&e);
                        // Write a fresh recovery copy for the new tab right away.
                        self.last_autosave =
                            std::time::Instant::now() - std::time::Duration::from_secs(60);
                        self.notify(tr("Recovered. Use Save As to keep it."));
                    }
                    Err(err) => {
                        self.notify_error(tf!("Could not recover this document: {}", err));
                    }
                }
                !entries.is_empty()
            }
        }
    }
}

fn remove_recovery_entry(e: &platform::recovery::RecoveryEntry) {
    if let Some(token) = e
        .pdf
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_suffix(".recovery.pdf"))
    {
        platform::recovery::remove(&platform::dirs::data_dir().join("recovery"), token);
    }
}
