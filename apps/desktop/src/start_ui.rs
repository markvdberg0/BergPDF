//! The start screen (docs/DESIGN.md §5): shown when no document is open and in the "+" tab. Navigation on
//! the left (Home, Recent, Pinned, Signatures and stamps, Preferences), a drop zone, quick tasks and the
//! recent files with a picture of their first page.
//!
//! The pictures are rendered from page 1 when a document is saved or closed, 300 px wide, and kept in the
//! user data folder (`thumbnails/<hash of the path>.png`). They never leave the computer; removing a file
//! from the list deletes its picture. Password-protected documents get none.

use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::state::*;
use crate::tf;
use crate::theme::{self, Tint, radius};
use editor_core::command::CommandId as C;
use egui::{Color32, CornerRadius, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::path::{Path, PathBuf};

/// The page of the start screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StartPage {
    #[default]
    Home,
    Recent,
    Pinned,
    Stamps,
}

/// A quick task: asks for a file, opens it, then starts something.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Task {
    Merge,
    Ocr,
    Sign,
    Fill,
    Shrink,
    PdfA,
    Password,
    Redact,
}

const TASKS: [(Task, &str, &str, Tint, Icon); 8] = [
    (
        Task::Merge,
        "Merge PDFs",
        "Combine files into one",
        Tint::Accent,
        Icon::Merge,
    ),
    (
        Task::Ocr,
        "Make a scan searchable",
        "Text recognition, on this computer",
        Tint::Blue,
        Icon::Ocr,
    ),
    (
        Task::Sign,
        "Sign a document",
        "Handwritten or with a certificate",
        Tint::Purple,
        Icon::Signature,
    ),
    (
        Task::Fill,
        "Fill in a form",
        "Type into the form fields",
        Tint::Green,
        Icon::FormField,
    ),
    (
        Task::Shrink,
        "Shrink a file",
        "Save as optimized",
        Tint::Amber,
        Icon::Optimize,
    ),
    (
        Task::PdfA,
        "Convert to PDF/A",
        "For long-term archiving",
        Tint::Neutral,
        Icon::Archive,
    ),
    (
        Task::Password,
        "Add a password",
        "Protect who can open it",
        Tint::Neutral,
        Icon::Lock,
    ),
    (
        Task::Redact,
        "Redact",
        "Remove content for real",
        Tint::Red,
        Icon::Redact,
    ),
];

/// FNV-1a of the path: a stable file name for its picture.
fn path_hash(p: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in p.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Where the picture of a recent file is kept.
pub fn thumbnail_file(path: &str) -> PathBuf {
    platform::dirs::data_dir()
        .join("thumbnails")
        .join(format!("{:016x}.png", path_hash(path)))
}

/// "10 minutes ago", "yesterday", or the date.
fn ago(secs: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let d = now.saturating_sub(secs);
    match d {
        0..60 => tr("just now").into(),
        60..3600 => tf!("{} minutes ago", d / 60),
        3600..86_400 => tf!("{} hours ago", d / 3600),
        86_400..172_800 => tr("yesterday").into(),
        _ if d < 30 * 86_400 => tf!("{} days ago", d / 86_400),
        _ => {
            let s = pdf_engine::annot::pdf_date_from_unix(secs);
            let digits: String = s.chars().filter(char::is_ascii_digit).take(8).collect();
            if digits.len() == 8 {
                format!("{}-{}-{}", &digits[0..4], &digits[4..6], &digits[6..8])
            } else {
                String::new()
            }
        }
    }
}

/// A rounded rectangle as a closed list of points (for the dashed drop-zone border).
fn rounded_outline(r: Rect, rad: f32) -> Vec<Pos2> {
    let mut v = Vec::new();
    let corners = [
        (Pos2::new(r.right() - rad, r.top() + rad), 270.0, 360.0),
        (Pos2::new(r.right() - rad, r.bottom() - rad), 0.0, 90.0),
        (Pos2::new(r.left() + rad, r.bottom() - rad), 90.0, 180.0),
        (Pos2::new(r.left() + rad, r.top() + rad), 180.0, 270.0),
    ];
    for (c, a, b) in corners {
        v.extend(crate::chrome::arc_pts(c, rad, a, b));
    }
    v.push(v[0]);
    v
}

impl App {
    /// Render the picture of page 1 of tab `i` for the start screen, on a background thread.
    pub fn save_thumbnail(&mut self, i: usize) {
        let Some(tab) = self.tabs.get_mut(i) else {
            return;
        };
        let Some(path) = tab.session.path.clone() else {
            return;
        };
        if tab.session.protection().is_some() {
            return;
        }
        let Ok(pages) = tab.session.pages() else {
            return;
        };
        let Some(first) = pages.first() else { return };
        let geometry = first.geometry;
        let Ok(snap) = tab.session.snapshot() else {
            return;
        };
        let key = path.to_string_lossy().into_owned();
        let dest = thumbnail_file(&key);
        self.start_thumbs.remove(&key);
        std::thread::spawn(move || {
            let size = geometry.view_size(pdf_engine::geom::Rotation::R0);
            let scale = 300.0 / size.width.max(1.0);
            let png = pdf_engine::render::with_session(snap.bytes.clone(), |s| {
                s.render_page(0, geometry, pdf_engine::geom::Rotation::R0, scale)?
                    .to_png()
            });
            if let Ok(png) = png
                && let Some(dir) = dest.parent()
                && std::fs::create_dir_all(dir).is_ok()
            {
                let tmp = dest.with_extension("png.partial");
                if std::fs::write(&tmp, &png).is_ok() {
                    let _ = std::fs::rename(&tmp, &dest);
                }
            }
        });
    }

    /// The picture of a recent file as a texture (loaded once).
    fn start_thumb(&mut self, ctx: &egui::Context, path: &str) -> Option<egui::TextureHandle> {
        if let Some(t) = self.start_thumbs.get(path) {
            return t.clone();
        }
        let tex = std::fs::read(thumbnail_file(path)).ok().and_then(|bytes| {
            let dec = png::Decoder::new(std::io::Cursor::new(bytes));
            let mut r = dec.read_info().ok()?;
            let mut buf = vec![0; r.output_buffer_size()?];
            let info = r.next_frame(&mut buf).ok()?;
            let (w, h) = (info.width as usize, info.height as usize);
            if w == 0 || h == 0 || w > 2000 || h > 4000 {
                return None;
            }
            let data = &buf[..info.buffer_size()];
            let img = match info.color_type {
                png::ColorType::Rgba => egui::ColorImage::from_rgba_unmultiplied([w, h], data),
                png::ColorType::Rgb => egui::ColorImage::from_rgb([w, h], data),
                _ => return None,
            };
            Some(ctx.load_texture(
                format!("start-thumb-{path}"),
                img,
                egui::TextureOptions::LINEAR,
            ))
        });
        self.start_thumbs.insert(path.to_string(), tex.clone());
        tex
    }

    fn forget_recent(&mut self, path: &str) {
        self.prefs.remove_recent(path);
        let _ = std::fs::remove_file(thumbnail_file(path));
        self.start_thumbs.remove(path);
        self.prefs_dirty = true;
    }

    fn clear_recent_list(&mut self) {
        let gone: Vec<String> = self
            .prefs
            .recent_files
            .iter()
            .filter(|p| !self.prefs.pinned_files.contains(p))
            .cloned()
            .collect();
        for p in &gone {
            let _ = std::fs::remove_file(thumbnail_file(p));
            self.start_thumbs.remove(p);
        }
        self.prefs.clear_recent();
        self.prefs_dirty = true;
    }

    fn show_in_folder(path: &Path) {
        #[cfg(windows)]
        let r = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn();
        #[cfg(target_os = "macos")]
        let r = std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn();
        #[cfg(not(any(windows, target_os = "macos")))]
        let r = std::process::Command::new("xdg-open")
            .arg(path.parent().unwrap_or(path))
            .spawn();
        if let Err(e) = r {
            tracing::warn!("show in folder: {e}");
        }
    }

    fn run_task(&mut self, ctx: &egui::Context, task: Task) {
        let Some(path) = platform::dialogs::pick_open_documents(None)
            .into_iter()
            .next()
        else {
            return;
        };
        let before = self.tabs.len();
        self.open_path(ctx, &path);
        let opened = self.tabs.len() > before
            || self
                .active_tab()
                .is_some_and(|t| t.session.path.as_deref() == Some(path.as_path()));
        if !opened {
            return;
        }
        self.show_start = false;
        match task {
            Task::Merge => {
                self.ribbon_tab = RibbonTab::Organize;
                self.run_command(ctx, C::DocumentMerge);
            }
            Task::Ocr => {
                self.ribbon_tab = RibbonTab::Edit;
                self.run_command(ctx, C::OcrDocument);
            }
            Task::Sign => self.ribbon_tab = RibbonTab::Sign,
            Task::Fill => {
                self.ribbon_tab = RibbonTab::Forms;
                self.run_command(ctx, C::ToolFillForm);
            }
            Task::Shrink => self.run_command(ctx, C::FileSaveOptimized),
            Task::PdfA => self.run_command(ctx, C::FileConvertPdfA),
            Task::Password => self.run_command(ctx, C::FileProtection),
            Task::Redact => self.ribbon_tab = RibbonTab::Redact,
        }
    }

    /// The start screen.
    pub fn welcome(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let full = ui.max_rect();
        let nav = Rect::from_min_size(full.min, Vec2::new(228.0, full.height()));
        let main = Rect::from_min_max(egui::pos2(nav.right(), full.top()), full.max);
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(nav.shrink2(Vec2::new(14.0, 14.0))),
            |ui| {
                self.start_nav(ui, ctx);
            },
        );
        let inner = Rect::from_min_max(
            main.min + Vec2::new(24.0, 14.0),
            main.max - Vec2::new(36.0, 0.0),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("start_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width(inner.width());
                    match self.start_page {
                        StartPage::Home => {
                            self.drop_zone(ui, ctx);
                            ui.add_space(26.0);
                            self.quick_tasks(ui, ctx);
                            ui.add_space(26.0);
                            self.recent_section(ui, ctx, tr("Recent"), false, 6, true);
                        }
                        StartPage::Recent => {
                            self.recent_section(ui, ctx, tr("Recent"), false, usize::MAX, true)
                        }
                        StartPage::Pinned => {
                            self.recent_section(ui, ctx, tr("Pinned"), true, usize::MAX, false)
                        }
                        StartPage::Stamps => self.stamps_page(ui, ctx),
                    }
                    ui.add_space(24.0);
                });
        });
    }

    fn nav_item(&mut self, ui: &mut egui::Ui, icon: Icon, label: &str, active: bool) -> bool {
        let pal = self.pal;
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 38.0), Sense::click());
        if active {
            ui.painter().add(
                egui::epaint::Shadow {
                    offset: [0, 1],
                    blur: 2,
                    spread: 0,
                    color: Color32::from_black_alpha(if pal.dark { 100 } else { 20 }),
                }
                .as_shape(rect, radius::XL),
            );
            ui.painter().rect_filled(rect, radius::XL, pal.card);
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, radius::XL, pal.hover);
        }
        let ir = Rect::from_center_size(
            egui::pos2(rect.left() + 12.0 + 9.0, rect.center().y),
            Vec2::splat(18.0),
        );
        icons::paint(
            ui.painter(),
            ir,
            icon,
            if active { pal.accent } else { pal.text },
        );
        let font = theme::font(if active { 700 } else { 500 }, 13.0);
        let g = ui
            .painter()
            .layout_no_wrap(label.to_string(), font, pal.text);
        ui.painter().galley(
            egui::pos2(ir.right() + 10.0, rect.center().y - g.size().y / 2.0),
            g,
            pal.text,
        );
        resp.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, label)
        });
        resp.clicked()
    }

    fn start_nav(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let pal = self.pal;
        ui.spacing_mut().item_spacing.y = 2.0;
        for (page, icon, label) in [
            (StartPage::Home, Icon::Page, tr("Home")),
            (StartPage::Recent, Icon::Scroll, tr("Recent")),
            (StartPage::Pinned, Icon::Bookmark, tr("Pinned")),
            (
                StartPage::Stamps,
                Icon::Signature,
                tr("Signatures and stamps"),
            ),
        ] {
            if self.nav_item(ui, icon, label, self.start_page == page) {
                self.start_page = page;
            }
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            egui::Frame::new()
                .fill(pal.track)
                .corner_radius(CornerRadius::same(radius::XL))
                .inner_margin(egui::Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                        icons::paint(ui.painter(), r, Icon::Shield, pal.success);
                        ui.add(
                            egui::Label::new(
                                RichText::new(tr(
                                    "Offline. No account. Your files stay on this computer.",
                                ))
                                .size(12.0)
                                .color(pal.text_dim),
                            )
                            .wrap(),
                        );
                    });
                });
            ui.add_space(8.0);
            if self.nav_item(ui, Icon::Settings, tr("Preferences"), false) {
                self.run_command(ctx, C::Preferences);
            }
        });
    }

    fn drop_zone(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let pal = self.pal;
        let dragging = ctx.input(|i| !i.raw.hovered_files.is_empty());
        let w = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 64.0 + 56.0), Sense::hover());
        let p = ui.painter();
        if dragging {
            p.rect(
                rect,
                radius::XXXXL,
                pal.accent_soft,
                Stroke::new(2.0, pal.accent),
                egui::StrokeKind::Inside,
            );
        } else {
            p.rect_filled(rect, radius::XXXXL, pal.panel);
            let pts = rounded_outline(rect.shrink(0.75), 18.0);
            p.extend(egui::Shape::dashed_line(
                &pts,
                Stroke::new(1.5, pal.border_dashed),
                6.0,
                4.0,
            ));
        }
        let tile = Rect::from_min_size(
            egui::pos2(rect.left() + 32.0, rect.center().y - 32.0),
            Vec2::splat(64.0),
        );
        p.rect_filled(tile, 16, pal.accent_soft);
        icons::paint(
            p,
            Rect::from_center_size(tile.center(), Vec2::splat(30.0)),
            Icon::Export,
            pal.accent_text,
        );
        let heading = if dragging {
            tr("Drop to open")
        } else {
            tr("Drop a PDF or picture here")
        };
        let hg = p.layout_no_wrap(heading.to_string(), theme::font(700, 24.0), pal.text);
        let sg = p.layout_no_wrap(
            tr("PDF, PNG and JPEG. Pictures open as a one-page PDF.").to_string(),
            egui::FontId::proportional(14.0),
            pal.text_dim,
        );
        let tx = tile.right() + 28.0;
        let block = hg.size().y + 6.0 + sg.size().y;
        let ty = rect.center().y - block / 2.0;
        let hy = hg.size().y;
        p.galley(egui::pos2(tx, ty), hg, pal.text);
        p.galley(egui::pos2(tx, ty + hy + 6.0), sg, pal.text_dim);
        // Right: the large primary button and its shortcut.
        let chip = self
            .prefs
            .keybindings
            .primary_label(C::FileOpen, crate::app::OS)
            .unwrap_or_default();
        let area = Rect::from_min_max(
            egui::pos2(rect.right() - 280.0, rect.top()),
            egui::pos2(rect.right() - 32.0, rect.bottom()),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(area)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if !chip.is_empty() {
                    crate::ui_kit::kbd(ui, &chip);
                }
                if crate::ui_kit::primary_large(ui, Icon::Open, tr("Open a file")).clicked() {
                    self.run_command(ctx, C::FileOpen);
                }
            },
        );
    }

    fn quick_tasks(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let pal = self.pal;
        ui.label(
            RichText::new(tr("Quick tasks"))
                .font(theme::font(700, 15.0))
                .color(pal.text),
        );
        ui.add_space(10.0);
        let cols = if ctx.content_rect().width() < 1100.0 {
            3
        } else {
            4
        };
        let gap = 10.0;
        let w = ui.available_width();
        let cw = (w - gap * (cols as f32 - 1.0)) / cols as f32;
        let mut chosen: Option<Task> = None;
        for row in TASKS.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (task, title, desc, tint, icon) in row {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(cw, 62.0), Sense::click());
                    let fill = if resp.hovered() { pal.hover } else { pal.card };
                    ui.painter().rect(
                        rect,
                        radius::XXXL,
                        fill,
                        Stroke::new(1.0, pal.border),
                        egui::StrokeKind::Inside,
                    );
                    let (bg, fg) = tint.colors(pal.dark);
                    let tile = Rect::from_min_size(
                        egui::pos2(rect.left() + 14.0, rect.center().y - 19.0),
                        Vec2::splat(38.0),
                    );
                    ui.painter().rect_filled(tile, radius::XL, bg);
                    icons::paint(
                        ui.painter(),
                        Rect::from_center_size(tile.center(), Vec2::splat(19.0)),
                        *icon,
                        fg,
                    );
                    let tx = tile.right() + 12.0;
                    let clip = Rect::from_min_max(
                        egui::pos2(tx, rect.top()),
                        egui::pos2(rect.right() - 12.0, rect.bottom()),
                    );
                    let tg = ui.painter().layout_no_wrap(
                        tr(title).to_string(),
                        theme::font(700, 13.0),
                        pal.text,
                    );
                    let mut job = egui::text::LayoutJob::simple_singleline(
                        tr(desc).to_string(),
                        egui::FontId::proportional(12.0),
                        pal.text_muted,
                    );
                    job.wrap.max_width = clip.width();
                    job.wrap.max_rows = 1;
                    job.wrap.break_anywhere = true;
                    let dg = ui.painter().layout_job(job);
                    let block = tg.size().y + 2.0 + dg.size().y;
                    let y = rect.center().y - block / 2.0;
                    let th = tg.size().y;
                    ui.painter()
                        .with_clip_rect(clip)
                        .galley(egui::pos2(tx, y), tg, pal.text);
                    ui.painter().with_clip_rect(clip).galley(
                        egui::pos2(tx, y + th + 2.0),
                        dg,
                        pal.text_muted,
                    );
                    resp.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tr(title))
                    });
                    if resp.on_hover_text(tr(desc)).clicked() {
                        chosen = Some(*task);
                    }
                }
            });
            ui.add_space(gap - ui.spacing().item_spacing.y);
        }
        if let Some(t) = chosen {
            self.run_task(ctx, t);
        }
    }

    fn recent_section(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        title: &str,
        pinned_only: bool,
        limit: usize,
        clear_link: bool,
    ) {
        let pal = self.pal;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(title)
                    .font(theme::font(700, 15.0))
                    .color(pal.text),
            );
            if clear_link && !self.prefs.recent_files.is_empty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let r = ui.add(
                        egui::Label::new(
                            RichText::new(tr("Clear list"))
                                .font(theme::font(600, 12.0))
                                .color(pal.accent_text),
                        )
                        .sense(Sense::click()),
                    );
                    if r.on_hover_text(tr("Pinned files stay")).clicked() {
                        self.clear_recent_list();
                    }
                });
            }
        });
        ui.add_space(10.0);
        let files: Vec<String> = if pinned_only {
            self.prefs.pinned_files.clone()
        } else {
            self.prefs.start_files()
        };
        if files.is_empty() {
            ui.label(
                RichText::new(if pinned_only {
                    tr("No pinned files. Right-click a recent file to pin it.")
                } else {
                    tr("Files you open appear here.")
                })
                .size(12.0)
                .color(pal.text_muted),
            );
            return;
        }
        let cols = if ctx.content_rect().width() < 1100.0 {
            4
        } else {
            6
        };
        let gap = 14.0;
        let w = ui.available_width();
        let cw = (w - gap * (cols as f32 - 1.0)) / cols as f32;
        let mut open: Option<String> = None;
        let mut pin: Option<String> = None;
        let mut remove: Option<String> = None;
        let mut reveal: Option<String> = None;
        let mut missing_click = false;
        for row in files.iter().take(limit).collect::<Vec<_>>().chunks(cols) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for path in row {
                    let exists = Path::new(path.as_str()).exists();
                    let pinned = self.prefs.pinned_files.contains(path);
                    let tex = if exists {
                        self.start_thumb(ctx, path)
                    } else {
                        None
                    };
                    let (card, resp) =
                        ui.allocate_exact_size(Vec2::new(cw, 150.0 + 10.0 + 40.0), Sense::click());
                    let alpha = if exists { 1.0 } else { 0.4 };
                    let area = Rect::from_min_size(card.min, Vec2::new(cw, 150.0));
                    let p = ui.painter();
                    p.rect_filled(
                        area,
                        radius::XXXL,
                        if resp.hovered() {
                            pal.pressed
                        } else {
                            pal.canvas
                        },
                    );
                    // The first page, at the bottom centre with square bottom corners.
                    let (pw, ph) = match &tex {
                        Some(t) if t.size()[0] > t.size()[1] => (150.0_f32.min(cw - 16.0), 112.0),
                        _ => (96.0, 128.0),
                    };
                    let page = Rect::from_min_size(
                        egui::pos2(area.center().x - pw / 2.0, area.bottom() - ph),
                        Vec2::new(pw, ph),
                    );
                    let top_round = CornerRadius {
                        nw: radius::XS,
                        ne: radius::XS,
                        sw: 0,
                        se: 0,
                    };
                    p.add(
                        egui::epaint::Shadow {
                            offset: [0, 1],
                            blur: 3,
                            spread: 0,
                            color: Color32::from_black_alpha(if pal.dark { 128 } else { 36 }),
                        }
                        .as_shape(page, top_round),
                    );
                    p.rect_filled(page, top_round, Color32::WHITE.gamma_multiply(alpha));
                    if let Some(t) = &tex {
                        let s = t.size();
                        let uv_h = (f32::from(u16::try_from(s[0]).unwrap_or(1)) * ph / pw)
                            / s[1].max(1) as f32;
                        p.image(
                            t.id(),
                            page.shrink(0.5),
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, uv_h.min(1.0))),
                            Color32::WHITE.gamma_multiply(alpha),
                        );
                    } else if !exists || self.prefs.recent_files.contains(path) {
                        icons::paint(
                            p,
                            Rect::from_center_size(page.center(), Vec2::splat(28.0)),
                            Icon::Page,
                            pal.text_disabled,
                        );
                    }
                    if pinned {
                        let badge = Rect::from_min_size(
                            egui::pos2(area.right() - 8.0 - 24.0, area.top() + 8.0),
                            Vec2::splat(24.0),
                        );
                        p.rect_filled(badge, radius::LG, pal.card);
                        icons::paint(p, badge.shrink(5.0), Icon::Bookmark, pal.accent_text);
                    }
                    let name = Path::new(path.as_str())
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.to_string());
                    let folder = Path::new(path.as_str())
                        .parent()
                        .and_then(|d| d.file_name())
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let text_clip =
                        Rect::from_min_max(egui::pos2(card.left(), area.bottom() + 10.0), card.max);
                    let mut job = egui::text::LayoutJob::simple_singleline(
                        name.clone(),
                        theme::font(700, 13.0),
                        pal.text.gamma_multiply(alpha),
                    );
                    job.wrap.max_width = cw;
                    job.wrap.max_rows = 1;
                    job.wrap.break_anywhere = true;
                    let ng = p.layout_job(job);
                    let nh = ng.size().y;
                    p.with_clip_rect(text_clip)
                        .galley(text_clip.min, ng, pal.text);
                    let (meta, col) = if exists {
                        let when = self
                            .prefs
                            .recent_opened
                            .get(path.as_str())
                            .map(|t| ago(*t))
                            .unwrap_or_default();
                        let m = match (folder.is_empty(), when.is_empty()) {
                            (false, false) => format!("{folder} · {when}"),
                            (false, true) => folder,
                            (true, _) => when,
                        };
                        (m, pal.text_muted)
                    } else {
                        (tr("Not found").to_string(), pal.danger)
                    };
                    let mut job = egui::text::LayoutJob::simple_singleline(
                        meta,
                        egui::FontId::proportional(12.0),
                        col,
                    );
                    job.wrap.max_width = cw;
                    job.wrap.max_rows = 1;
                    job.wrap.break_anywhere = true;
                    let mg = p.layout_job(job);
                    p.with_clip_rect(text_clip).galley(
                        text_clip.min + Vec2::new(0.0, nh + 2.0),
                        mg,
                        col,
                    );
                    resp.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name)
                    });
                    let resp = resp.on_hover_text(path.as_str());
                    if resp.clicked() {
                        if exists {
                            open = Some(path.to_string());
                        } else {
                            missing_click = true;
                        }
                    }
                    resp.context_menu(|ui| {
                        if exists && ui.button(tr("Open")).clicked() {
                            open = Some(path.to_string());
                            ui.close();
                        }
                        if ui
                            .button(if pinned { tr("Unpin") } else { tr("Pin") })
                            .clicked()
                        {
                            pin = Some(path.to_string());
                            ui.close();
                        }
                        if exists && ui.button(tr("Show in folder")).clicked() {
                            reveal = Some(path.to_string());
                            ui.close();
                        }
                        if ui.button(tr("Remove from list")).clicked() {
                            remove = Some(path.to_string());
                            ui.close();
                        }
                    });
                }
            });
            ui.add_space(gap - ui.spacing().item_spacing.y);
        }
        if let Some(p) = open {
            self.open_path(ctx, Path::new(&p));
            if self
                .active_tab()
                .is_some_and(|t| t.session.path.as_deref() == Some(Path::new(&p)))
            {
                self.show_start = false;
            }
        }
        if let Some(p) = pin {
            self.prefs.toggle_pin(&p);
            self.prefs_dirty = true;
        }
        if let Some(p) = remove {
            self.forget_recent(&p);
        }
        if let Some(p) = reveal {
            Self::show_in_folder(Path::new(&p));
        }
        if missing_click {
            self.notify(tr(
                "This file was not found. Right-click it to remove it from the list.",
            ));
        }
    }

    fn stamps_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let pal = self.pal;
        ui.label(
            RichText::new(tr("Signatures and stamps"))
                .font(theme::font(700, 15.0))
                .color(pal.text),
        );
        ui.add_space(10.0);
        let has_sig = self.signature_image.is_some() || !self.handwriting.is_empty();
        crate::ui_kit::card(ui, |ui| {
            ui.label(RichText::new(tr("Your signature")).font(theme::font(600, 14.0)));
            ui.label(
                RichText::new(if has_sig {
                    tr("A signature is saved on this computer. Place it with Sign ▸ Place signature.")
                } else {
                    tr("No signature saved yet.")
                })
                .size(12.0)
                .color(pal.text_dim),
            );
            ui.horizontal(|ui| {
                if ui.button(tr("Draw signature…")).clicked() {
                    self.run_command(ctx, C::DrawSignature);
                }
                if ui.button(tr("Signature from picture…")).clicked() {
                    self.run_command(ctx, C::SignFromImage);
                }
            });
        });
        ui.add_space(14.0);
        crate::ui_kit::card(ui, |ui| {
            ui.label(RichText::new(tr("Your stamps")).font(theme::font(600, 14.0)));
            if self.stamps.stamps.is_empty() {
                ui.label(
                    RichText::new(tr("You have no stamps of your own yet."))
                        .size(12.0)
                        .color(pal.text_dim),
                );
            }
            for d in self.stamps.stamps.clone() {
                ui.label(RichText::new(d.name).color(pal.text));
            }
            if ui.button(tr("New stamp…")).clicked() {
                self.run_command(ctx, C::NewStamp);
            }
        });
    }
}
