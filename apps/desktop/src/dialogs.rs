//! Modal dialogs, the command palette and transient notices.

use crate::app::OS;
use crate::state::*;
use editor_core::command::{self, CommandId as C, Key, PaletteItem, Shortcut};
use editor_core::prefs::{DefaultZoom, Density, SETTINGS, ThemeChoice, Workspace};
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
                    ui.heading("Save changes?");
                    ui.label(format!("“{name}” has unsaved changes."));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() {
                            choice = Some(0);
                        }
                        if ui.button("Don't save").clicked() {
                            choice = Some(1);
                        }
                        if ui.button("Cancel").clicked() {
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
                    ui.heading("Quit with unsaved changes?");
                    for n in &dirty {
                        ui.label(format!("• {n}"));
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Save all and quit").clicked() {
                            choice = Some(0);
                        }
                        if ui.button("Quit without saving").clicked() {
                            choice = Some(1);
                        }
                        if ui.button("Cancel").clicked() {
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
                    ui.heading(format!("About “{title}”"));
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
                    ui.heading("Open external link?");
                    ui.label("This document wants to open a web address in your browser:");
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
                            "This kind of link cannot be opened from a document.",
                        );
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(allowed, egui::Button::new("Open in browser"))
                            .clicked()
                        {
                            choice = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
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
                    ui.heading("Go to page");
                    let r = ui.add(
                        egui::TextEdit::singleline(text)
                            .hint_text("Page number")
                            .desired_width(160.0),
                    );
                    r.request_focus();
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Go").clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Enter))
                        {
                            go = Some(true);
                        }
                        if ui.button("Cancel").clicked()
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
                    ui.heading(format!("Delete {n} page{}?", if n == 1 { "" } else { "s" }));
                    ui.label("You can undo this with Undo until you close the document.");
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button(RichText::new("Delete").color(self.pal.danger))
                            .clicked()
                        {
                            choice = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
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
            } => {
                let (page, tool, rect) = (*page, *tool, *rect);
                let mut choice: Option<bool> = None;
                let title = match tool {
                    Tool::Note => "Sticky note",
                    Tool::Stamp => "Stamp",
                    Tool::Callout => "Callout text",
                    _ => "Text box",
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
                    let r = ui.add(
                        egui::TextEdit::multiline(text)
                            .desired_rows(if tool == Tool::Stamp { 1 } else { 5 })
                            .desired_width(380.0)
                            .hint_text("Type here…"),
                    );
                    if self.frame_counter.is_multiple_of(2) && !r.has_focus() {
                        r.request_focus();
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let ok = !text.trim().is_empty() || tool == Tool::Note;
                        if ui.add_enabled(ok, egui::Button::new("Add")).clicked() {
                            choice = Some(true);
                        }
                        if ui.button("Cancel").clicked()
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
                        self.commit_text_entry(page, tool, rect, t);
                    }
                }
            }
            Dialog::AddText {
                page,
                at,
                text,
                size,
                bold,
            } => {
                let (page, at) = (*page, *at);
                let mut choice: Option<bool> = None;
                modal(ctx, "add_text", |ui| {
                    ui.heading("Add text");
                    ui.label(
                        RichText::new("Adds real text to the page content (not an annotation).")
                            .size(12.0)
                            .color(self.pal.text_dim),
                    );
                    let r = ui.add(
                        egui::TextEdit::multiline(text)
                            .desired_rows(4)
                            .desired_width(380.0)
                            .hint_text("Type here…"),
                    );
                    if self.frame_counter.is_multiple_of(2) && !r.has_focus() {
                        r.request_focus();
                    }
                    ui.horizontal(|ui| {
                        ui.label("Size");
                        ui.add(egui::DragValue::new(size).range(4.0..=200.0).suffix(" pt"));
                        ui.checkbox(bold, "Bold");
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!text.trim().is_empty(), egui::Button::new("Add"))
                            .clicked()
                        {
                            choice = Some(true);
                        }
                        if ui.button("Cancel").clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Escape))
                        {
                            choice = Some(false);
                        }
                    });
                });
                if let Some(c) = choice {
                    keep = false;
                    if c {
                        let (t, s, b) = (text.clone(), *size, *bold);
                        self.commit_add_text(page, at, &t, s, b);
                    }
                }
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
                    ui.label(format!(
                        "Version {}  (development build)",
                        env!("CARGO_PKG_VERSION")
                    ));
                    ui.label(
                        RichText::new(
                            "“BergPDF” is an internal working name, not a cleared product name.",
                        )
                        .size(12.0)
                        .color(self.pal.text_dim),
                    );
                    ui.add_space(8.0);
                    ui.label("Offline-first. No account, no telemetry, no network access for core editing.");
                    ui.add_space(6.0);
                    ui.label(RichText::new("Native components").strong());
                    ui.label("Rust (egui/eframe, wgpu, hayro, lopdf). Native dependencies are the OS windowing/graphics stack and GPU drivers; full audit in docs/DEPENDENCIES.md.");
                    ui.add_space(6.0);
                    ui.label(RichText::new("Bundled font").strong());
                    ui.label("DejaVu Sans (Bitstream Vera licence) — used for new text in annotations and for interface symbols. See assets/fonts/LICENSE-DejaVu.txt.");
                    ui.add_space(10.0);
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
                if close {
                    keep = false;
                }
            }
            Dialog::FirstRun => {
                let mut done = false;
                modal(ctx, "first_run", |ui| {
                    ui.heading("Welcome to BergPDF");
                    ui.label("Choose how much to show. You can change this any time in Preferences, and every tool stays reachable through Search commands.");
                    ui.add_space(8.0);
                    ui.radio_value(
                        &mut self.prefs.workspace,
                        Workspace::Essential,
                        "Essential — the common tools: open, markup, organize pages, save",
                    );
                    ui.radio_value(
                        &mut self.prefs.workspace,
                        Workspace::Professional,
                        "Professional — all tools and panels",
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label("Appearance");
                        ui.selectable_value(&mut self.prefs.theme, ThemeChoice::System, "System");
                        ui.selectable_value(&mut self.prefs.theme, ThemeChoice::Light, "Light");
                        ui.selectable_value(&mut self.prefs.theme, ThemeChoice::Dark, "Dark");
                    });
                    ui.add_space(10.0);
                    if ui.button("Get started").clicked() {
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
                let f = filter.to_lowercase();
                let show = |key: &str| -> bool {
                    f.is_empty()
                        || SETTINGS.iter().any(|s| {
                            s.key == key
                                && format!("{} {} {}", s.title, s.description, s.keywords)
                                    .to_lowercase()
                                    .contains(&f)
                        })
                };
                egui::Window::new("Preferences").collapsible(false).resizable(true).default_width(520.0).anchor(Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
                    ui.add(egui::TextEdit::singleline(filter).hint_text("Search settings").desired_width(f32::INFINITY));
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                        if show("theme") {
                            section(ui, "Application theme", "Light, Dark or follow the system. Never changes document colours.");
                            ui.horizontal(|ui| {
                                changed |= ui.selectable_value(&mut self.prefs.theme, ThemeChoice::System, "System").changed();
                                changed |= ui.selectable_value(&mut self.prefs.theme, ThemeChoice::Light, "Light").changed();
                                changed |= ui.selectable_value(&mut self.prefs.theme, ThemeChoice::Dark, "Dark").changed();
                            });
                        }
                        if show("dark_page_filter") {
                            section(ui, "Dark page view", "Comfortable dark reading filter. Display only — saved PDFs are never changed.");
                            changed |= ui.checkbox(&mut self.prefs.dark_page_filter, "Use dark page view").changed();
                        }
                        if show("density") {
                            section(ui, "Interface density", "Compact, Comfortable or Touch/Pen hit targets.");
                            ui.horizontal(|ui| {
                                changed |= ui.selectable_value(&mut self.prefs.density, Density::Compact, "Compact").changed();
                                changed |= ui.selectable_value(&mut self.prefs.density, Density::Comfortable, "Comfortable").changed();
                                changed |= ui.selectable_value(&mut self.prefs.density, Density::Touch, "Touch / Pen").changed();
                            });
                        }
                        if show("ui_scale") {
                            section(ui, "Interface scale", "Scales toolbar icons, text and hit targets. Page rendering stays sharp at any scale.");
                            changed |= ui.add(egui::Slider::new(&mut self.prefs.ui_scale, 0.75..=2.5).text("scale")).changed();
                        }
                        if show("workspace") {
                            section(ui, "Workspace", "Essential shows the common tools; Professional shows everything. Both can reach every command via search.");
                            ui.horizontal(|ui| {
                                changed |= ui.selectable_value(&mut self.prefs.workspace, Workspace::Essential, "Essential").changed();
                                changed |= ui.selectable_value(&mut self.prefs.workspace, Workspace::Professional, "Professional").changed();
                            });
                        }
                        if show("default_zoom") {
                            section(ui, "Zoom when opening a document", "Applies to documents you open from now on; you can still zoom freely.");
                            ui.horizontal(|ui| {
                                changed |= ui.selectable_value(&mut self.prefs.default_zoom, DefaultZoom::FitPage, "Fit page").changed();
                                changed |= ui.selectable_value(&mut self.prefs.default_zoom, DefaultZoom::FitWidth, "Fit width").changed();
                                changed |= ui.selectable_value(&mut self.prefs.default_zoom, DefaultZoom::Actual, "100 %").changed();
                            });
                        }
                        if show("author") {
                            section(ui, "Author name", "Stored on comments and markup you create.");
                            changed |= ui.text_edit_singleline(&mut self.prefs.author).changed();
                        }
                        if show("render_cache_mb") {
                            section(ui, "Render cache size", "Memory budget for cached page tiles.");
                            changed |= ui.add(egui::Slider::new(&mut self.prefs.render_cache_mb, 64..=4096).suffix(" MiB")).changed();
                        }
                        if show("shortcuts") {
                            section(ui, "Keyboard shortcuts", "View and change shortcuts; conflicts are flagged.");
                            if ui.button("Customize shortcuts…").clicked() {
                                close = true;
                                self.dialog_next = Some(Dialog::Shortcuts { filter: String::new(), capture: None });
                            }
                        }
                    });
                    ui.add_space(8.0);
                    if ui.button("Done").clicked() {
                        close = true;
                    }
                });
                if changed {
                    self.restyle(ctx);
                }
                if close {
                    keep = false;
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
                                let names: Vec<_> =
                                    conflicts.iter().map(|c| command::info(*c).title).collect();
                                self.notify_error(format!(
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
                egui::Window::new("Keyboard shortcuts")
                    .collapsible(false)
                    .resizable(true)
                    .default_size([560.0, 480.0])
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                    .show(ctx, |ui| {
                        ui.label(
                            RichText::new(format!(
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
                                .hint_text("Filter")
                                .desired_width(f32::INFINITY),
                        );
                        ui.add_space(4.0);
                        egui::ScrollArea::vertical()
                            .max_height(380.0)
                            .show(ui, |ui| {
                                let mut last_cat = None;
                                for info in command::REGISTRY {
                                    if !f.is_empty()
                                        && !format!("{} {}", info.title, info.keywords)
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
                                            RichText::new(info.category.title())
                                                .strong()
                                                .color(self.pal.accent),
                                        );
                                        last_cat = Some(info.category);
                                    }
                                    ui.horizontal(|ui| {
                                        ui.label(info.title);
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if cap_now == Some(info.id) {
                                                    ui.label(
                                                        RichText::new(
                                                            "Press new shortcut… (Esc cancels)",
                                                        )
                                                        .color(self.pal.accent),
                                                    );
                                                } else {
                                                    if ui.small_button("Change").clicked() {
                                                        start_capture = Some(info.id);
                                                    }
                                                    if self
                                                        .prefs
                                                        .keybindings
                                                        .overrides
                                                        .contains_key(&info.id)
                                                        && ui.small_button("Reset").clicked()
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
                        if ui.button("Close").clicked() {
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
                                .hint_text("Type a command, tool or setting…")
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
                                RichText::new("No matching commands").color(self.pal.text_dim),
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
                                                info.title.to_string(),
                                                info.description.to_string(),
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
                                                    format!("Setting: {}", s.title)
                                                }),
                                                s.map_or(String::new(), |s| {
                                                    s.description.to_string()
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
        let mut out: Vec<PaletteItem> = command::search_commands(q, |c| self.command_enabled(c))
            .into_iter()
            .map(PaletteItem::Command)
            .collect();
        if !q.is_empty() {
            for s in SETTINGS {
                let hay = format!("{} {} {}", s.title, s.description, s.keywords);
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
            ui.heading("Recover unsaved work?");
            ui.label(
                RichText::new(
                    "BergPDF found copies of documents that were open with unsaved changes when it last stopped unexpectedly.",
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
                    format!("{} min ago", age / 60)
                } else if age < 86_400 {
                    format!("{} h ago", age / 3600)
                } else {
                    format!("{} days ago", age / 86_400)
                };
                ui.group(|ui| {
                    ui.label(RichText::new(name).strong());
                    ui.label(
                        RichText::new(format!("saved {when} · {} KB", e.len / 1024))
                            .size(12.0)
                            .color(self.pal.text_dim),
                    );
                    ui.horizontal(|ui| {
                        if ui.button("Restore as new tab").clicked() {
                            act = Some(Act::Restore(i));
                        }
                        if ui.button("Discard").clicked() {
                            act = Some(Act::Discard(i));
                        }
                    });
                });
            }
            ui.add_space(6.0);
            if ui.button("Decide later").clicked() {
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
                        self.notify("Recovered. Use Save As to keep it.");
                    }
                    Err(err) => {
                        self.notify_error(format!("Could not recover this document: {err}"));
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
