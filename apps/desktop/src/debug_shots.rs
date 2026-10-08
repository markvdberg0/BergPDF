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
    /// Frames spent waiting for background work (OCR, translation) of the current scenario.
    holds: u32,
    /// The search after OCR was started.
    searched: bool,
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
        holds: 0,
        searched: false,
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
    /// True while a scenario still waits for background work (text recognition, translation);
    /// gives up after a while so a failing job cannot hang the run.
    fn waiting_for_work(&mut self, app: &mut App, name: &str) -> bool {
        self.holds += 1;
        if self.holds > 6000 {
            return false;
        }
        match name {
            "ocr_search" => {
                if app.ocr_job.is_some() {
                    return true;
                }
                if !self.searched {
                    // Recognition is done: search the page that was a picture a moment ago.
                    self.searched = true;
                    app.left_tab = LeftTab::Search;
                    if let Some(t) = app.tabs.first_mut() {
                        t.session.search.query = "quick brown".into();
                    }
                    app.start_search();
                    self.phase = Phase::Settle(40);
                }
                false
            }
            "translate_inline" => matches!(app.dialog, Some(Dialog::Translate(_))),
            _ => false,
        }
    }

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
        if self.started
            && matches!(self.phase, Phase::Settle(_))
            && self.waiting_for_work(app, &name)
        {
            return;
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
                    self.holds = 0;
                    self.searched = false;
                    apply(app, ctx, &name);
                    // Screens that first read the document text on a worker thread need a little longer.
                    self.phase = Phase::Settle(match name.as_str() {
                        n if n.starts_with("translate") => 90,
                        "copilot_answer" => 220,
                        "forms_policy" => 200,
                        _ => 25,
                    });
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

/// A made-up scan of a signature: dark blue ink on slightly grey paper.
fn demo_signature_png() -> Vec<u8> {
    let (w, h) = (360u32, 140u32);
    let mut rgba = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let t = x as f32 / w as f32 * 6.0 * std::f32::consts::PI;
            let cy = h as f32 / 2.0 + (t.sin() * 28.0) * (1.0 - x as f32 / w as f32 * 0.5);
            let d = (y as f32 - cy).abs();
            let ink = (1.0 - (d / 2.5)).clamp(0.0, 1.0);
            let paper = 244.0 - ((x * 7 + y * 3) % 5) as f32;
            let c = |ink_c: f32| (paper * (1.0 - ink) + ink_c * ink) as u8;
            rgba.extend_from_slice(&[c(20.0), c(30.0), c(110.0), 255]);
        }
    }
    pdf_engine::stampimage::StampImage::from_rgba(w, h, &rgba)
        .and_then(|i| i.to_png())
        .unwrap_or_default()
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
    app.right_tab = RightTab::Properties;
    app.ribbon_tab = RibbonTab::Home;
    app.tool = editor_core::tools::Tool::Select;
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
        "optimize" => {
            app.ribbon_tab = RibbonTab::File;
            app.run_command(ctx, C::FileSaveOptimized)
        }
        "pdfa" => {
            app.ribbon_tab = RibbonTab::File;
            app.run_command(ctx, C::FileConvertPdfA)
        }
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
        "imagesig" => {
            if let Ok(p) =
                crate::stamp_ui::PictureState::from_bytes(demo_signature_png(), "scan".into())
            {
                app.dialog = Some(Dialog::ImageSignature(Box::new(p)));
            }
        }
        "newstamp" | "newstamp_pic" => {
            let mut st = crate::stamp_ui::NewStampState::demo(name == "newstamp_pic");
            if name == "newstamp_pic" {
                st.set_demo_picture(demo_signature_png());
            }
            app.dialog = Some(Dialog::NewStamp(Box::new(st)));
        }
        "stamps" => {
            if let Some(page) = page {
                use editor_core::stamps::{StampDef, StampKind};
                use pdf_engine::geom::Rect;
                app.set_tool(editor_core::tools::Tool::Stamp);
                if app.stamps.stamps.is_empty() {
                    let _ = app.stamps.add(StampDef {
                        name: "Betaald".into(),
                        kind: StampKind::Text {
                            label: "BETAALD".into(),
                            color: [0.1, 0.5, 0.2],
                        },
                    });
                    let _ = app.stamps.add(StampDef {
                        name: "Logo".into(),
                        kind: StampKind::Image {
                            file: "stamp-1.png".into(),
                        },
                    });
                    app.current_stamp = Some("Betaald".into());
                    if let Ok(p) =
                        crate::stamp_ui::PictureState::from_bytes(demo_signature_png(), "x".into())
                        && let Some(img) = p.image()
                    {
                        app.place_picture(
                            page,
                            img,
                            Rect::new(300.0, 600.0, 480.0, 650.0),
                            "Signature",
                            "demo",
                        );
                    }
                    let mut spec = app.new_spec(pdf_engine::annot::AnnotationKind::StampText {
                        rect: Rect::new(80.0, 600.0, 220.0, 640.0),
                        label: "BETAALD".into(),
                    });
                    spec.color = pdf_engine::annot::Rgb(0.1, 0.5, 0.2);
                    app.add_annotation(page, spec, "demo");
                    if let Some(t) = app.tabs.first_mut() {
                        t.session.selection.annotations.clear();
                    }
                }
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
        // ---- screens of the website's feature pages -------------------------------------------
        "copilot_answer" => {
            show_copilot(app);
            app.ribbon_tab = RibbonTab::Home;
            copilot_conversation(app);
        }
        "copilot_consent" => {
            show_copilot(app);
            app.prefs.ai_consent_provider = None;
            app.dialog = Some(Dialog::AiConsent);
        }
        "prefs_ai" => {
            show_copilot(app);
            app.dialog = Some(Dialog::Preferences {
                filter: "ai".into(),
            });
        }
        "translate_dialog" => {
            show_copilot(app);
            app.prefs.ai_consent_provider = Some(app.prefs.ai.provider);
            app.open_translate_dialog(ctx);
        }
        "translate_inline" => {
            app.ribbon_tab = RibbonTab::Copilot;
            app.prefs.ai_consent_provider = Some(app.prefs.ai.provider);
            crate::translate_ui::DEBUG_AUTO.store(true, std::sync::atomic::Ordering::Relaxed);
            app.open_translate_dialog(ctx);
        }
        "ocr_models" => {
            app.ribbon_tab = RibbonTab::Edit;
            app.open_ocr_dialog();
        }
        "ocr_search" => {
            app.ribbon_tab = RibbonTab::Edit;
            app.open_ocr_dialog();
            if let Some(Dialog::Ocr(st)) = app.dialog.take() {
                if let Err(e) = app.start_ocr(&st) {
                    eprintln!("OCR could not start: {e}");
                }
            }
        }
        "measure_plan" | "measure_panel" => {
            if let Some(page) = page {
                measure_demo(app, page);
                if name == "measure_plan" {
                    app.ribbon_tab = RibbonTab::Measure;
                    app.set_tool(editor_core::tools::Tool::MeasureDistance);
                    app.prefs.show_left_sidebar = false;
                    app.prefs.show_right_sidebar = false;
                } else {
                    app.right_tab = RightTab::Measure;
                }
            }
        }
        "forms_policy" => {
            app.ribbon_tab = RibbonTab::Home;
            // A form that has been filled in, as it would be when someone copies its pages.
            let form = app.form_for();
            if let Some(t) = app.tabs.first_mut() {
                for f in &form.fields {
                    let (id, name) = (f.id, f.name.as_str());
                    let _ = t.session.execute("Fill field", |tx| match name {
                        "name" => pdf_engine::forms::set_text_value(tx, id, "Anna de Vries"),
                        "address.city" => pdf_engine::forms::set_text_value(tx, id, "Utrecht"),
                        "agree" => pdf_engine::forms::set_checkbox(tx, id, true),
                        _ => Ok(()),
                    });
                }
            }

            app.dialog = Some(Dialog::FormPolicy(FormPolicyState {
                op: FormOp::Duplicate,
                policy: pdf_engine::pageops::FormPolicy::Independent,
            }));
        }
        "recovery" => {
            let ago =
                |secs: u64| std::time::SystemTime::now() - std::time::Duration::from_secs(secs);
            app.dialog = Some(Dialog::Recovery(vec![
                platform::recovery::RecoveryEntry {
                    pdf: PathBuf::new(),
                    original: Some(r"C:\Users\Anna\Documents\Offer Van Dijk BV.pdf".into()),
                    modified: ago(25 * 60),
                    len: 412 * 1024,
                },
                platform::recovery::RecoveryEntry {
                    pdf: PathBuf::new(),
                    original: None,
                    modified: ago(3 * 3600),
                    len: 86 * 1024,
                },
            ]));
        }
        _ => {}
    }
}

/// Open the Copilot panel on the right.
fn show_copilot(app: &mut App) {
    app.prefs.show_right_sidebar = true;
    app.right_tab = RightTab::Copilot;
    app.prefs.show_copilot = true;
}

/// A question about the report and the answer with two supporting passages, highlighted on the page.
fn copilot_conversation(app: &mut App) {
    use crate::copilot_ui::{ChatKind, ChatMsg, ChatPoint};
    let nl = crate::i18n::current() == crate::i18n::Lang::Nl;
    let point = |quote: &str, note: &str| ChatPoint {
        page: 2,
        quote: quote.into(),
        note: note.into(),
        verified: true,
        applied: false,
    };
    let (question, answer, n1, n2) = if nl {
        (
            "Wat is er met de omzet gebeurd?",
            "De omzet is met twaalf procent gegroeid ten opzichte van het vorige kwartaal. In de tabel staat Noord voor op Zuid, met 1200 tegen 900 eenheden.",
            "groei van de omzet",
            "omzet per regio",
        )
    } else {
        (
            "What happened to revenue?",
            "Revenue grew by twelve percent compared with the previous quarter. The table shows the North region ahead of the South, with 1200 against 900 units.",
            "revenue growth",
            "revenue by region",
        )
    };
    if let Some(t) = app.tabs.first_mut() {
        t.ui.copilot.msgs = vec![
            ChatMsg {
                kind: ChatKind::User,
                text: question.into(),
                points: Vec::new(),
            },
            ChatMsg {
                kind: ChatKind::Assistant,
                text: answer.into(),
                points: vec![
                    point(
                        "Revenue grew by twelve percent over the previous quarter",
                        n1,
                    ),
                    point("North 1200 48000", n2),
                ],
            },
        ];
        t.ui.copilot.stick_bottom = true;
    }
    app.go_to(1);
    for pi in 0..2 {
        if !app.highlight_point(1, pi) {
            eprintln!("copilot passage {pi} could not be highlighted");
        }
    }
    if let Some(t) = app.tabs.first_mut() {
        t.session.selection.annotations.clear();
    }
}

/// A floor plan with its scale set and a few measurements on it.
fn measure_demo(app: &mut App, page: pdf_engine::doc::PageId) {
    use pdf_engine::geom::Point;
    use pdf_engine::measure::{self, MeasureKind, Scale, Unit};
    if app.tabs.first().is_none_or(|t| t.session.revision() != 0) {
        return;
    }
    let scale = Scale::from_one_to(50.0, Unit::M).unwrap_or_else(|_| Scale::uncalibrated());
    let mut set = (*app.scales_for()).clone();
    set.document = Some(scale.clone());
    if let Some(t) = app.tabs.first_mut() {
        let _ = t
            .session
            .execute("Set scale", |tx| measure::write_scales(tx, &set));
    }
    let p = |x: f64, y: f64| Point::new(x, y);
    let shapes: Vec<(MeasureKind, Vec<Point>, &str)> = vec![
        (
            MeasureKind::Distance,
            vec![p(100.0, 70.0), p(700.0, 70.0)],
            "",
        ),
        (
            MeasureKind::Distance,
            vec![p(740.0, 100.0), p(740.0, 500.0)],
            "",
        ),
        (
            MeasureKind::Area,
            vec![
                p(100.0, 100.0),
                p(400.0, 100.0),
                p(400.0, 300.0),
                p(100.0, 300.0),
            ],
            "",
        ),
        (MeasureKind::Count, vec![p(180.0, 380.0)], "Sockets"),
        (MeasureKind::Count, vec![p(300.0, 440.0)], "Sockets"),
        (MeasureKind::Count, vec![p(560.0, 420.0)], "Sockets"),
    ];
    for (kind, pts, cat) in shapes {
        let cat = if cat.is_empty() { "Count 1" } else { cat };
        match measure::spec_for(kind, &pts, &scale, cat) {
            Ok(spec) => {
                app.add_annotation(page, spec, "demo");
            }
            Err(e) => eprintln!("measurement: {e}"),
        }
    }
    if let Some(t) = app.tabs.first_mut() {
        t.session.selection.annotations.clear();
    }
}
