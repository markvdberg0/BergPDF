//! Development aid (debug builds only): `BERG_SHOTS=<folder>;<scenario>,<scenario>…` makes the application draw
//! each named screen, save what it drew to `<folder>/<scenario>.png` (egui's own frame capture: only this
//! window's pixels, no screen capture, no input events) and exit. The window opens off-screen without taking focus.

use crate::state::*;
use std::path::PathBuf;

/// Where the run is.
enum Phase {
    /// Frames to wait before the next step.
    Settle(u32),
    /// The picture was requested.
    Waiting,
}

/// A point of a page a scenario wants "clicked", and where it is on screen (published by the canvas).
pub static CLICK_TARGET: std::sync::Mutex<
    Option<(pdf_engine::doc::PageId, pdf_engine::geom::Point)>,
> = std::sync::Mutex::new(None);
pub static CLICK_SCREEN: std::sync::Mutex<Option<egui::Pos2>> = std::sync::Mutex::new(None);

/// The canvas tells where the wanted page point is on screen.
pub fn publish_screen(vc: &crate::canvas::ViewCtx) {
    let Ok(target) = CLICK_TARGET.lock() else {
        return;
    };
    if let Some((page, pt)) = *target
        && let Some(i) = vc.pages.iter().position(|p| p.id == page)
        && let Ok(mut s) = CLICK_SCREEN.lock()
    {
        *s = Some(vc.pdf_to_screen(i, pt));
    }
}

pub struct DebugShots {
    /// Pointer events for the next frames (inside this window only), one list per frame.
    script: std::collections::VecDeque<Vec<egui::Event>>,
    dir: PathBuf,
    scenarios: Vec<String>,
    index: usize,
    phase: Phase,
    started: bool,
}

/// `Some` when `BERG_SHOTS` is set.
pub fn from_env() -> Option<DebugShots> {
    let v = std::env::var("BERG_SHOTS").ok()?;
    let (dir, list) = v.split_once(';')?;
    let scenarios: Vec<String> = list
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    std::fs::create_dir_all(dir).ok()?;
    Some(DebugShots {
        dir: PathBuf::from(dir),
        scenarios,
        index: 0,
        phase: Phase::Settle(40),
        started: false,
        script: Default::default(),
    })
}

/// Pointer events that double-click at `pos` (to try a scenario the way a hand would).
fn double_click(pos: egui::Pos2) -> Vec<Vec<egui::Event>> {
    let button = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    vec![
        vec![egui::Event::PointerMoved(pos)],
        vec![],
        vec![button(true)],
        vec![button(false)],
        vec![button(true)],
        vec![button(false)],
    ]
}

impl DebugShots {
    /// Hand the scripted pointer events of this frame to egui.
    pub fn inject(&mut self, raw: &mut egui::RawInput) {
        if let Some(events) = self.script.pop_front() {
            raw.events.extend(events);
        }
    }

    /// Called every frame.
    pub fn step(&mut self, app: &mut App, ctx: &egui::Context) {
        ctx.request_repaint();
        let Some(name) = self.scenarios.get(self.index).cloned() else {
            std::process::exit(0);
        };
        if name == "fontlist" {
            // The font list outside a combo box (popups are not part of the picture egui captures).
            egui::Window::new("fontlist")
                .fixed_pos([400.0, 120.0])
                .show(ctx, |ui| {
                    let mut fam = pdf_engine::fontembed::FontFamily::DejaVuSans;
                    crate::fontpick::family_menu(ui, &mut fam, egui::Id::new("dbg_font_search"));
                });
        }
        match &mut self.phase {
            Phase::Settle(n) => {
                if *n > 0 {
                    *n -= 1;
                    // Halfway through the settling the canvas has said where the wanted point is.
                    if self.started && *n == 15 && name == "calloutfont" {
                        // Typing while the font list is open must go to its search field.
                        self.script.push_back(vec![egui::Event::Text("Ver".into())]);
                    }
                    if self.started && *n == 5 && name == "calloutfont" {
                        let typed = match &app.dialog {
                            Some(Dialog::TextEntry { text, .. }) => text.clone(),
                            _ => "<no dialog>".into(),
                        };
                        eprintln!("CALLOUT TEXT AFTER TYPING: {typed:?}");
                    }
                    if self.started && *n == 15 && name.starts_with("dblclick") {
                        let at = CLICK_SCREEN.lock().ok().and_then(|s| *s);
                        if let Some(at) = at {
                            self.script.extend(double_click(at));
                        }
                    }
                    return;
                }
                if !self.started {
                    // The document is open and the fonts are loaded: set the scenario up and let it settle.
                    self.started = true;
                    apply(app, ctx, &name);
                    self.phase = Phase::Settle(25);
                    return;
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                self.phase = Phase::Waiting;
            }
            Phase::Waiting => {
                let shot = ctx.input(|i| {
                    i.events.iter().find_map(|e| match e {
                        egui::Event::Screenshot { image, .. } => Some(image.clone()),
                        _ => None,
                    })
                });
                if let Some(img) = shot {
                    save_png(&self.dir.join(format!("{name}.png")), &img);
                    self.index += 1;
                    self.started = false;
                    self.phase = Phase::Settle(6);
                }
            }
        }
    }
}

fn save_png(path: &std::path::Path, img: &egui::ColorImage) {
    let [w, h] = img.size;
    let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
    if let Ok(f) = std::fs::File::create(path) {
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        if let Ok(mut wr) = enc.write_header() {
            let _ = wr.write_image_data(&rgba);
        }
    }
}

/// Put the application into the state a scenario shows.
fn apply(app: &mut App, ctx: &egui::Context, name: &str) {
    use editor_core::command::CommandId as C;
    crate::fontpick::DEBUG_OPEN_FONT_MENU.store(false, std::sync::atomic::Ordering::Relaxed);
    app.dialog = None;
    app.palette_open = false;
    app.prefs.show_left_sidebar = true;
    app.prefs.show_right_sidebar = true;
    app.left_tab = LeftTab::Thumbnails;
    let page = app
        .tabs
        .first_mut()
        .and_then(|t| t.session.pages().ok())
        .and_then(|p| p.first().map(|i| i.id));
    match name {
        "search" => {
            app.left_tab = LeftTab::Search;
            if let Some(t) = app.tabs.first_mut() {
                t.session.search.query = "the".into();
            }
            app.start_search();
        }
        "prefs" => {
            app.dialog = Some(Dialog::Preferences {
                filter: String::new(),
            })
        }
        "note" => app.set_tool(editor_core::tools::Tool::Note),
        "freetext" => app.set_tool(editor_core::tools::Tool::FreeText),
        "fontmenu" => {
            app.set_tool(editor_core::tools::Tool::FreeText);
            crate::fontpick::DEBUG_OPEN_FONT_MENU.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        "collapsed" => {
            app.prefs.show_left_sidebar = false;
            app.prefs.show_right_sidebar = false;
        }
        "print" => app.run_command(ctx, C::FilePrint),
        "protect" => app.run_command(ctx, C::FileProtection),
        "optimize" => app.run_command(ctx, C::FileSaveOptimized),
        "pdfa" => app.run_command(ctx, C::FileConvertPdfA),
        "about" => app.dialog = Some(Dialog::About),
        "shortcuts" => {
            app.dialog = Some(Dialog::Shortcuts {
                filter: String::new(),
                capture: None,
            })
        }
        "palette" => app.palette_open = true,
        "confirm" => app.dialog = Some(Dialog::ConfirmQuit),
        "addtext" => {
            if let Some(page) = page {
                app.dialog = Some(Dialog::AddText {
                    page,
                    at: pdf_engine::geom::Point::new(100.0, 100.0),
                    text: String::new(),
                    size: 14.0,
                    font: pdf_engine::fontembed::FontStyle::default(),
                    turns: 0,
                });
            }
        }
        "textentry" => {
            if let Some(page) = page {
                app.dialog = Some(Dialog::TextEntry {
                    page,
                    tool: editor_core::tools::Tool::Note,
                    rect: pdf_engine::geom::Rect::new(100.0, 100.0, 100.0, 100.0),
                    text: String::new(),
                    callout: None,
                });
            }
        }
        "annots" | "inline" | "dblclick" => {
            if let Some(page) = page {
                use pdf_engine::annot::AnnotationKind;
                use pdf_engine::geom::Rect;
                app.set_tool(editor_core::tools::Tool::Select);
                // Only the first time: later scenarios start from the same document.
                if app.tabs.first().is_some_and(|t| t.session.revision() == 0) {
                    let cloud = app.new_spec(AnnotationKind::Cloud {
                        rect: Rect::new(60.0, 420.0, 300.0, 520.0),
                    });
                    app.add_annotation(page, cloud, "demo");
                    let rect = app.new_spec(AnnotationKind::Rectangle {
                        rect: Rect::new(330.0, 420.0, 450.0, 520.0),
                    });
                    app.add_annotation(page, rect, "demo");
                    app.commit_text_entry(
                        page,
                        editor_core::tools::Tool::FreeText,
                        Rect::new(60.0, 560.0, 300.0, 600.0),
                        "Edit this text".into(),
                        None,
                    );
                }
                if name == "dblclick"
                    && let Ok(mut t) = CLICK_TARGET.lock()
                {
                    *t = Some((page, pdf_engine::geom::Point::new(120.0, 580.0)));
                }
                if name == "inline" {
                    let list = app.annots_for(page);
                    if let Some(a) = list.iter().find(|a| a.subtype == "FreeText")
                        && let Some(t) = app.tabs.first_mut()
                    {
                        t.ui.inline_edit = crate::inline_edit::InlineEdit::start(page, a);
                    }
                }
            }
        }
        "calloutfont" => {
            if let Some(page) = page {
                app.dialog = Some(Dialog::TextEntry {
                    page,
                    tool: editor_core::tools::Tool::Callout,
                    rect: pdf_engine::geom::Rect::new(100.0, 100.0, 270.0, 148.0),
                    text: String::new(),
                    callout: None,
                });
                crate::fontpick::DEBUG_OPEN_FONT_MENU
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        "dblclick_text" => {
            if let Some(page) = page {
                app.set_tool(editor_core::tools::Tool::EditText);
                if let Ok(mut t) = CLICK_TARGET.lock() {
                    *t = Some((page, pdf_engine::geom::Point::new(110.0, 742.0)));
                }
            }
        }
        "redact" => {
            if let Some(page) = page {
                app.mark_redaction(
                    page,
                    vec![pdf_engine::geom::Rect::new(60.0, 600.0, 300.0, 640.0)],
                );
                app.open_redact_dialog();
            }
        }
        _ => {}
    }
}
