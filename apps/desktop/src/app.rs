//! Application shell: construction, per-frame logic and top-level layout.

use crate::state::*;
use crate::theme;
use editor_core::command::{CommandId, Key, Shortcut};
use editor_core::jobs::{JobOutput, WorkerHub};
use editor_core::platform_kind::OsKind;
use editor_core::prefs::Preferences;
use editor_core::tiles::ByteLru;
use editor_core::tools::Tool;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const OS: OsKind = OsKind::current();

impl App {
    pub fn new_app(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        let mut prefs = platform::dirs::read_text(&platform::dirs::prefs_file())
            .and_then(|s| Preferences::from_toml(&s).ok())
            .unwrap_or_default();
        prefs.sanitize();
        let ctx = cc.egui_ctx.clone();
        let hub = WorkerHub::new(Arc::new(move || ctx.request_repaint()));
        let system_dark = cc.egui_ctx.global_style().visuals.dark_mode;
        let pal = theme::palette(&prefs, system_dark);
        theme::apply(&cc.egui_ctx, &prefs, &pal);
        let budget = prefs.render_cache_mb as usize * 1024 * 1024;
        let mut app = App {
            prefs,
            pal,
            tabs: Vec::new(),
            active: 0,
            hub,
            tiles: ByteLru::new(budget),
            in_flight: HashSet::new(),
            failed: HashSet::new(),
            text: HashMap::new(),
            text_pending: HashSet::new(),
            tool: Tool::Select,
            left_tab: LeftTab::Thumbnails,
            right_tab: RightTab::Properties,
            ribbon_tab: RibbonTab::Home,
            dialog: None,
            palette_open: false,
            palette_query: String::new(),
            palette_sel: 0,
            notices: Vec::new(),
            prefs_dirty: false,
            frame_counter: 0,
            search_focus: false,
            system_dark,
            quit_confirmed: false,
            dark_filter_applied: false,
            pending_open: files,
            last_autosave: Instant::now(),
            dialog_next: None,
            last_title: String::new(),
            exports: Vec::new(),
            sign_return: None,
            ocr_job: None,
            handwriting: App::load_handwriting(),
        };
        app.dark_filter_applied = app.prefs.dark_page_filter;
        if !app.prefs.first_run_done {
            app.dialog = Some(Dialog::FirstRun);
        }
        app.check_recovery();
        app
    }

    pub fn save_prefs(&mut self) {
        match self.prefs.to_toml() {
            Ok(s) => {
                if let Err(e) = platform::dirs::write_text_atomic(&platform::dirs::prefs_file(), &s)
                {
                    tracing::warn!("could not save preferences: {e}");
                }
            }
            Err(e) => tracing::warn!("could not serialise preferences: {e}"),
        }
        self.prefs_dirty = false;
    }

    /// Apply theme/scale after a preference change.
    pub fn restyle(&mut self, ctx: &egui::Context) {
        self.pal = theme::palette(&self.prefs, self.system_dark);
        theme::apply(ctx, &self.prefs, &self.pal);
        self.tiles
            .set_budget(self.prefs.render_cache_mb as usize * 1024 * 1024);
        if self.dark_filter_applied != self.prefs.dark_page_filter {
            // Page filter is baked into tile pixels: drop them so they re-render filtered.
            self.dark_filter_applied = self.prefs.dark_page_filter;
            self.tiles.retain(|_| false);
            self.in_flight.clear();
        }
        self.prefs_dirty = true;
    }

    /// Translate this frame's key events into semantic commands.
    fn shortcut_commands(&self, ctx: &egui::Context) -> Vec<CommandId> {
        let typing = ctx.egui_wants_keyboard_input();
        let mut out = Vec::new();
        ctx.input(|i| {
            for ev in &i.events {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    repeat,
                    ..
                } = ev
                {
                    let Some(k) = map_key(*key) else { continue };
                    let chord = Shortcut {
                        command: modifiers.command,
                        shift: modifiers.shift,
                        alt: modifiers.alt,
                        ctrl: OS.uses_command_key() && modifiers.ctrl,
                        key: k,
                    };
                    let Some(cmd) = self.prefs.keybindings.resolve(&chord, OS) else {
                        continue;
                    };
                    if typing && !allowed_while_typing(&chord, cmd) {
                        continue;
                    }
                    if *repeat && !repeatable(cmd) {
                        continue;
                    }
                    out.push(cmd);
                }
            }
        });
        out
    }

    fn handle_worker_results(&mut self, ctx: &egui::Context) {
        for r in self.hub.poll() {
            tracing::debug!(doc = r.doc.0, rev = r.revision, req = r.request, out = ?std::mem::discriminant(&r.output), "worker result");
            match r.output {
                JobOutput::Tile { key, bitmap } => {
                    self.in_flight.remove(&key);
                    let Some(tab) = self.tabs.iter().find(|t| t.session.id == key.doc) else {
                        continue;
                    };
                    if tab.session.revision() < key.revision {
                        continue;
                    }
                    let mut rgba = bitmap.rgba;
                    if self.prefs.dark_page_filter {
                        crate::pagefilter::dark_reading(&mut rgba);
                    }
                    let img = egui::ColorImage::from_rgba_premultiplied(
                        [bitmap.width as usize, bitmap.height as usize],
                        &rgba,
                    );
                    let bytes = rgba.len();
                    let tex = ctx.load_texture(
                        format!("tile-{}-{}", key.page.0.0, key.tx),
                        img,
                        egui::TextureOptions::LINEAR,
                    );
                    self.tiles.insert(key, tex, bytes);
                }
                JobOutput::Text { page, text } => {
                    self.text_pending.remove(&(r.doc, page, r.revision));
                    if self
                        .tabs
                        .iter()
                        .any(|t| t.session.id == r.doc && t.session.revision() == r.revision)
                    {
                        self.text.insert((r.doc, page), (r.revision, text));
                    }
                }
                JobOutput::SearchPage {
                    page,
                    page_index,
                    hits,
                    scanned,
                    total,
                } => {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.session.id == r.doc) {
                        let s = &mut tab.session.search;
                        if s.request == r.request {
                            for (hit, snippet) in hits {
                                s.matches.push(editor_core::search::Match {
                                    page,
                                    page_index,
                                    hit,
                                    snippet,
                                });
                            }
                            s.progress = (scanned, total);
                            if s.current.is_none() && !s.matches.is_empty() {
                                s.current = Some(0);
                                tab.ui.goto = Some(s.matches[0].page_index);
                            }
                        }
                    }
                }
                JobOutput::SearchDone { cancelled } => {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.session.id == r.doc)
                        && tab.session.search.request == r.request
                    {
                        tab.session.search.running = false;
                        let _ = cancelled;
                    }
                }
                JobOutput::Dropped { key } => {
                    if let Some(k) = key {
                        self.in_flight.remove(&k);
                    }
                }
                JobOutput::Failed { key, message } => {
                    if let Some(k) = key {
                        self.in_flight.remove(&k);
                        self.failed.insert(k);
                    }
                    tracing::warn!("worker job failed: {message}");
                }
            }
        }
        // Drop text caches for closed documents / stale revisions lazily.
        let live: HashSet<_> = self.tabs.iter().map(|t| t.session.id).collect();
        self.text.retain(|(d, _), _| live.contains(d));
        self.tiles.retain(|k| live.contains(&k.doc));
    }

    fn handle_drops_and_pending(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        let mut to_open = std::mem::take(&mut self.pending_open);
        to_open.extend(dropped);
        for p in to_open {
            self.open_path(ctx, &p);
        }
    }

    /// Offer recovery files left behind by a previous crash.
    fn check_recovery(&mut self) {
        let dir = platform::dirs::data_dir().join("recovery");
        let stale = platform::recovery::list_stale(&dir, Duration::from_secs(90));
        if stale.is_empty() {
            return;
        }
        let d = Dialog::Recovery(stale);
        if self.dialog.is_none() {
            self.dialog = Some(d);
        } else {
            self.dialog_next = Some(d);
        }
    }

    fn autosave(&mut self) {
        if self.last_autosave.elapsed() < Duration::from_secs(30) {
            return;
        }
        self.last_autosave = Instant::now();
        let dir = platform::dirs::data_dir().join("recovery");
        for t in &mut self.tabs {
            let token = recovery_token(t.session.id.0);
            if t.session.is_dirty() {
                if let Ok(bytes) = t.session.recovery_bytes() {
                    let original = t
                        .session
                        .path
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned());
                    if let Err(e) =
                        platform::recovery::write(&dir, &token, original.as_deref(), &bytes)
                    {
                        tracing::warn!("recovery write failed: {e}");
                    }
                }
            } else {
                platform::recovery::remove(&dir, &token);
            }
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame_counter += 1;
        // Follow OS theme changes when the preference is System.
        let sys_dark = ctx.global_style().visuals.dark_mode;
        let _ = sys_dark;
        self.handle_drops_and_pending(ctx);
        self.handle_worker_results(ctx);
        self.poll_exports(ctx);
        self.poll_ocr(ctx);
        let cmds = if self.dialog.is_none()
            || matches!(self.dialog, Some(Dialog::Shortcuts { capture: None, .. }))
        {
            self.shortcut_commands(ctx)
        } else {
            Vec::new()
        };
        for c in cmds {
            self.run_command(ctx, c);
        }
        // Clipboard copy arrives as a dedicated event (consumed by text fields otherwise).
        if !ctx.egui_wants_keyboard_input()
            && ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)))
        {
            self.run_command(ctx, CommandId::EditCopy);
        }
        self.autosave();
        self.notices.retain(|n| n.until > Instant::now());
        if self.prefs_dirty {
            self.save_prefs();
        }
        // Window close: confirm if there are unsaved documents.
        if ctx.input(|i| i.viewport().close_requested())
            && !self.quit_confirmed
            && self.tabs.iter().any(|t| t.session.is_dirty())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dialog = Some(Dialog::ConfirmQuit);
        }
        // Window title.
        let title = match self.active_tab() {
            Some(t) => format!(
                "{}{} — BergPDF",
                if t.session.is_dirty() { "● " } else { "" },
                t.session.title
            ),
            None => "BergPDF".to_string(),
        };
        // Only send when it changed: a viewport command every frame would force a repaint loop.
        if self.last_title != title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui::Panel::top("quick_access")
            .frame(
                egui::Frame::new()
                    .fill(self.pal.chrome)
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(ui, |ui| self.quick_access_bar(ui, &ctx));
        egui::Panel::top("ribbon")
            .frame(
                egui::Frame::new()
                    .fill(self.pal.chrome)
                    .inner_margin(egui::Margin::symmetric(8, 2)),
            )
            .show(ui, |ui| self.ribbon(ui, &ctx));
        egui::Panel::top("tabs")
            .frame(egui::Frame::new().fill(self.pal.chrome))
            .show(ui, |ui| self.tab_bar(ui, &ctx));
        egui::Panel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(self.pal.chrome)
                    .inner_margin(egui::Margin::symmetric(10, 3)),
            )
            .show(ui, |ui| self.status_bar(ui, &ctx));
        if self.prefs.show_left_sidebar && !self.tabs.is_empty() {
            let w = self.prefs.left_sidebar_width;
            let r = egui::Panel::left("left_sidebar")
                .resizable(true)
                .default_size(w)
                .size_range(130.0..=480.0)
                .frame(egui::Frame::new().fill(self.pal.panel))
                .show(ui, |ui| self.left_sidebar(ui, &ctx));
            let nw = r.response.rect.width();
            if (nw - self.prefs.left_sidebar_width).abs() > 1.0 {
                self.prefs.left_sidebar_width = nw;
                self.prefs_dirty = true;
            }
        }
        if self.prefs.show_right_sidebar && !self.tabs.is_empty() {
            let w = self.prefs.right_sidebar_width;
            let r = egui::Panel::right("right_sidebar")
                .resizable(true)
                .default_size(w)
                .size_range(170.0..=520.0)
                .frame(egui::Frame::new().fill(self.pal.panel))
                .show(ui, |ui| self.right_sidebar(ui, &ctx));
            let nw = r.response.rect.width();
            if (nw - self.prefs.right_sidebar_width).abs() > 1.0 {
                self.prefs.right_sidebar_width = nw;
                self.prefs_dirty = true;
            }
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(self.pal.canvas))
            .show(ui, |ui| {
                if self.tabs.is_empty() {
                    self.welcome(ui, &ctx);
                } else {
                    self.canvas(ui, &ctx);
                }
            });
        self.dialogs(&ctx);
        self.palette_ui(&ctx);
        self.notices_ui(&ctx);
    }
}

pub fn map_key_pub(k: egui::Key) -> Option<Key> {
    map_key(k)
}

fn map_key(k: egui::Key) -> Option<Key> {
    use egui::Key as E;
    Some(match k {
        E::A => Key::Char('A'),
        E::B => Key::Char('B'),
        E::C => Key::Char('C'),
        E::D => Key::Char('D'),
        E::E => Key::Char('E'),
        E::F => Key::Char('F'),
        E::G => Key::Char('G'),
        E::H => Key::Char('H'),
        E::I => Key::Char('I'),
        E::J => Key::Char('J'),
        E::K => Key::Char('K'),
        E::L => Key::Char('L'),
        E::M => Key::Char('M'),
        E::N => Key::Char('N'),
        E::O => Key::Char('O'),
        E::P => Key::Char('P'),
        E::Q => Key::Char('Q'),
        E::R => Key::Char('R'),
        E::S => Key::Char('S'),
        E::T => Key::Char('T'),
        E::U => Key::Char('U'),
        E::V => Key::Char('V'),
        E::W => Key::Char('W'),
        E::X => Key::Char('X'),
        E::Y => Key::Char('Y'),
        E::Z => Key::Char('Z'),
        E::Num0 => Key::Char('0'),
        E::Num1 => Key::Char('1'),
        E::Num2 => Key::Char('2'),
        E::Num3 => Key::Char('3'),
        E::Num4 => Key::Char('4'),
        E::Num5 => Key::Char('5'),
        E::Num6 => Key::Char('6'),
        E::Num7 => Key::Char('7'),
        E::Num8 => Key::Char('8'),
        E::Num9 => Key::Char('9'),
        E::Comma => Key::Char(','),
        E::Slash => Key::Char('/'),
        E::OpenBracket => Key::Char('['),
        E::CloseBracket => Key::Char(']'),
        E::F1 => Key::F(1),
        E::F2 => Key::F(2),
        E::F3 => Key::F(3),
        E::F4 => Key::F(4),
        E::F5 => Key::F(5),
        E::F6 => Key::F(6),
        E::F7 => Key::F(7),
        E::F8 => Key::F(8),
        E::F9 => Key::F(9),
        E::F10 => Key::F(10),
        E::F11 => Key::F(11),
        E::F12 => Key::F(12),
        E::Delete => Key::Delete,
        E::Backspace => Key::Backspace,
        E::Escape => Key::Escape,
        E::Enter => Key::Enter,
        E::Tab => Key::Tab,
        E::Space => Key::Space,
        E::ArrowLeft => Key::ArrowLeft,
        E::ArrowRight => Key::ArrowRight,
        E::ArrowUp => Key::ArrowUp,
        E::ArrowDown => Key::ArrowDown,
        E::PageUp => Key::PageUp,
        E::PageDown => Key::PageDown,
        E::Home => Key::Home,
        E::End => Key::End,
        E::Plus => Key::Plus,
        E::Minus => Key::Minus,
        E::Equals => Key::Equals,
        _ => return None,
    })
}

/// While a text field has focus only modifier chords that are not text-editing commands pass.
fn allowed_while_typing(chord: &Shortcut, cmd: CommandId) -> bool {
    // Plain keys, Shift and Alt chords belong to the focused text field. Only primary-modifier
    // chords that are not text-editing commands (Open, Save, Find, Palette…) pass through.
    if !chord.command {
        return false;
    }
    !matches!(
        cmd,
        CommandId::EditUndo
            | CommandId::EditRedo
            | CommandId::EditCopy
            | CommandId::EditSelectAll
            | CommandId::EditDelete
            | CommandId::EditDuplicate
    )
}

fn repeatable(cmd: CommandId) -> bool {
    !matches!(
        cmd,
        CommandId::FileOpen
            | CommandId::FileSave
            | CommandId::FileSaveAs
            | CommandId::FileClose
            | CommandId::FileQuit
            | CommandId::CommandPalette
            | CommandId::Find
    )
}

/// Recovery-file token, unique per process and per document, so a file left by a crashed
/// instance is never overwritten or deleted by a new one before the user decides about it.
pub fn recovery_token(doc: u64) -> String {
    static PREFIX: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let p = PREFIX.get_or_init(|| {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        format!("p{}-t{secs}", std::process::id())
    });
    format!("{p}-d{doc}")
}
