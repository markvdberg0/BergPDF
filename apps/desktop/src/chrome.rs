//! Window chrome: quick-access bar, ribbon, document tabs, status bar, welcome screen.

use crate::app::OS;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::state::*;
use crate::tf;
use crate::theme;
use editor_core::command::CommandId as C;
use editor_core::prefs::{Density, Workspace};
use editor_core::tools::Tool;
use editor_core::view::{ViewMode, ZoomMode};
use egui::{Color32, Rect, RichText, Sense, Vec2};

fn tool_icon(t: Tool) -> Icon {
    match t {
        Tool::Select => Icon::Cursor,
        Tool::Hand => Icon::Hand,
        Tool::TextSelect => Icon::TextCursor,
        Tool::Highlight => Icon::Highlight,
        Tool::Underline => Icon::Underline,
        Tool::StrikeOut => Icon::Strike,
        Tool::Note => Icon::Note,
        Tool::FreeText => Icon::TextBox,
        Tool::Callout => Icon::Callout,
        Tool::Rectangle => Icon::Rect,
        Tool::Ellipse => Icon::Ellipse,
        Tool::Cloud => Icon::Cloud,
        Tool::Line => Icon::Line,
        Tool::Arrow => Icon::Arrow,
        Tool::Polygon => Icon::Polygon,
        Tool::Polyline => Icon::Polyline,
        Tool::Ink => Icon::Pencil,
        Tool::Stamp => Icon::Stamp,
        Tool::EditText => Icon::EditText,
        Tool::AddText => Icon::AddText,
        Tool::AddImage => Icon::AddImage,
        Tool::FillForm => Icon::FormField,
        Tool::MeasureDistance => Icon::Ruler,
        Tool::MeasurePerimeter => Icon::PathLen,
        Tool::MeasureArea => Icon::AreaPoly,
        Tool::MeasureRect => Icon::RectMeasure,
        Tool::MeasureRadius => Icon::RadiusMeasure,
        Tool::MeasureAngle => Icon::AngleMeasure,
        Tool::Count => Icon::CountMark,
        Tool::Calibrate => Icon::Calibrate,
        Tool::PlaceSignature => Icon::Signature,
        Tool::SignArea => Icon::Rect,
        Tool::Redact => Icon::Redact,
    }
}

pub fn command_icon(c: C) -> Icon {
    if let Some(t) = Tool::from_command(c) {
        return tool_icon(t);
    }
    match c {
        C::FileOpen => Icon::Open,
        C::FileSave => Icon::Save,
        C::FileSaveAs => Icon::SaveAs,
        C::FileClose => Icon::Close,
        C::ExportMeasurements => Icon::Export,
        C::FileExportImage => Icon::AddImage,
        C::SignDocument => Icon::Certificate,
        C::OcrDocument => Icon::Ocr,
        C::ToggleSnap => Icon::Snap,
        C::FileSaveOptimized => Icon::Optimize,
        C::FileConvertPdfA => Icon::Archive,
        C::FileProtection => Icon::Lock,
        C::FilePrint => Icon::Print,
        C::RedactMarkSelection => Icon::Redact,
        C::RedactApply => Icon::RedactApply,
        C::ToggleCopilot | C::CopilotSummarize | C::CopilotSummarizeAnnotations => Icon::Sparkle,
        C::TranslateDocument => Icon::Translate,
        C::ShowSignatures => Icon::Shield,
        C::DrawSignature => Icon::Pencil,
        C::SignFromImage => Icon::AddImage,
        C::NewStamp => Icon::Stamp,
        C::FileProperties => Icon::Info,
        C::FormFlatten => Icon::Merge,
        C::EditUndo => Icon::Undo,
        C::EditRedo => Icon::Redo,
        C::EditDelete => Icon::Trash,
        C::EditDuplicate => Icon::Duplicate,
        C::Find | C::FindNext | C::FindPrevious => Icon::Find,
        C::CommandPalette => Icon::Palette,
        C::Preferences => Icon::Settings,
        C::ShortcutReference => Icon::Keys,
        C::About => Icon::Info,
        C::CheckForUpdates => Icon::Info,
        C::ViewZoomIn => Icon::ZoomIn,
        C::ViewZoomOut => Icon::ZoomOut,
        C::ViewZoomActual => Icon::Page,
        C::ViewFitPage => Icon::FitPage,
        C::ViewFitWidth => Icon::FitWidth,
        C::ViewModeContinuous => Icon::Scroll,
        C::ViewModeSingle => Icon::Single,
        C::ViewModeFacing => Icon::Spread,
        C::ViewRotateClockwise | C::PageRotateClockwise => Icon::RotateCw,
        C::ViewRotateCounterClockwise | C::PageRotateCounterClockwise => Icon::RotateCcw,
        C::ViewToggleLeftSidebar => Icon::SidebarL,
        C::ViewToggleRightSidebar => Icon::SidebarR,
        C::ViewDarkPages => Icon::Moon,
        C::PageDelete => Icon::Trash,
        C::PageInsertBlank => Icon::InsertPage,
        C::PageInsertImage => Icon::AddImage,
        C::PageDuplicate => Icon::Duplicate,
        C::PageExtract => Icon::Extract,
        C::PageMoveUp => Icon::ArrowUp,
        C::PageMoveDown => Icon::ArrowDown,
        C::DocumentMerge => Icon::Merge,
        _ => Icon::Page,
    }
}

impl App {
    /// A ribbon/quick-access button. Returns true when clicked.
    pub fn cmd_button(
        &mut self,
        ui: &mut egui::Ui,
        id: C,
        selected: bool,
        label: Option<&str>,
        compact: bool,
    ) -> bool {
        let m = theme::metrics(self.prefs.density);
        let enabled = self.command_enabled(id);
        let info = editor_core::command::info(id);
        let text = label.unwrap_or_else(|| ribbon_label(id, info.title));
        let short = text.trim_end_matches('…');
        let with_label = m.show_labels && !compact;
        // Lay the caption out first and size the button to it, so neighbouring buttons never
        // overlap: one line if it fits in 84 px, otherwise at most two word-wrapped lines.
        let caption = with_label.then(|| {
            let font = egui::FontId::proportional(11.0);
            let one = ui
                .painter()
                .layout_no_wrap(short.to_string(), font.clone(), Color32::WHITE);
            if one.size().x <= 84.0 {
                (one, false)
            } else {
                let mut job =
                    egui::text::LayoutJob::simple(short.to_string(), font, Color32::WHITE, 76.0);
                job.wrap.max_rows = 2;
                job.halign = egui::Align::Center;
                // A centre-aligned galley is anchored at its centre line.
                (ui.painter().layout_job(job), true)
            }
        });
        let size = if let Some((g, _)) = &caption {
            egui::vec2(m.ribbon_button.x.max(g.size().x + 12.0), m.ribbon_button.y)
        } else {
            egui::vec2(m.button_h.max(28.0), m.button_h.max(28.0))
        };
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let pal = self.pal;
        if ui.is_rect_visible(rect) {
            let hovered = resp.hovered() && enabled;
            if selected {
                ui.painter().rect_filled(rect, 5.0, pal.accent_soft);
                ui.painter().rect_stroke(
                    rect,
                    5.0,
                    egui::Stroke::new(1.0, pal.accent),
                    egui::StrokeKind::Inside,
                );
            } else if hovered {
                ui.painter().rect_filled(rect, 5.0, pal.hover);
            }
            let col = if enabled {
                pal.text
            } else {
                pal.text_dim.gamma_multiply(0.5)
            };
            let icon_rect = if with_label {
                Rect::from_center_size(
                    egui::pos2(rect.center().x, rect.min.y + 6.0 + m.icon / 2.0),
                    Vec2::splat(m.icon),
                )
            } else {
                Rect::from_center_size(rect.center(), Vec2::splat(m.icon))
            };
            icons::paint(
                ui.painter(),
                icon_rect,
                command_icon(id),
                if selected { pal.accent } else { col },
            );
            if let Some((galley, centre_anchored)) = caption {
                // Starts just below the icon, so it can never climb into it.
                let pos = egui::pos2(
                    if centre_anchored {
                        rect.center().x
                    } else {
                        rect.center().x - galley.size().x / 2.0
                    },
                    rect.min.y + 6.0 + m.icon + 3.0,
                );
                ui.painter()
                    .galley_with_override_text_color(pos, galley, col);
            }
        }
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tr(info.title))
        });
        let tip = self.tooltip_for(id);
        let resp = resp.on_hover_text(tip);
        enabled && resp.clicked()
    }

    fn group(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        add: impl FnOnce(&mut Self, &mut egui::Ui),
    ) {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                add(self, ui);
            });
            ui.add(
                egui::Label::new(RichText::new(title).size(10.0).color(self.pal.text_dim))
                    .selectable(false),
            );
        });
        let r = ui.min_rect();
        let x = ui.cursor().min.x + 3.0;
        ui.painter().line_segment(
            [egui::pos2(x, r.min.y + 4.0), egui::pos2(x, r.max.y - 2.0)],
            egui::Stroke::new(1.0, self.pal.border),
        );
        ui.add_space(8.0);
    }

    fn cmds(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, ids: &[C]) {
        for &id in ids {
            let sel = Tool::from_command(id).is_some_and(|t| t == self.tool)
                || (id == C::ViewDarkPages && self.prefs.dark_page_filter)
                || (id == C::ToggleSnap && self.prefs.snap_to_geometry)
                || (id == C::ToggleCopilot
                    && self.prefs.show_right_sidebar
                    && self.right_tab == RightTab::Copilot)
                || (id == C::ViewToggleLeftSidebar && self.prefs.show_left_sidebar)
                || (id == C::ViewToggleRightSidebar && self.prefs.show_right_sidebar)
                || self.view_mode_selected(id);
            if self.cmd_button(ui, id, sel, None, false) {
                self.run_command(ctx, id);
            }
        }
    }

    fn view_mode_selected(&self, id: C) -> bool {
        let Some(t) = self.active_tab() else {
            return false;
        };
        match id {
            C::ViewModeContinuous => t.session.view.mode == ViewMode::Continuous,
            C::ViewModeSingle => t.session.view.mode == ViewMode::SinglePage,
            C::ViewModeFacing => t.session.view.mode == ViewMode::Facing,
            C::ViewFitWidth => t.session.view.zoom_mode == ZoomMode::FitWidth,
            C::ViewFitPage => t.session.view.zoom_mode == ZoomMode::FitPage,
            _ => false,
        }
    }

    pub fn quick_access_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::new(26.0, 26.0), Sense::hover());
            icons::paint_logo(ui.painter(), r, self.pal.text, self.pal.accent);
            ui.label(RichText::new("BergPDF").strong());
            ui.add_space(10.0);
            for id in [C::FileOpen, C::FileSave, C::EditUndo, C::EditRedo] {
                if self.cmd_button(ui, id, false, None, true) {
                    self.run_command(ctx, id);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = match self.prefs.keybindings.primary_label(C::CommandPalette, OS) {
                    Some(s) => tf!("Search commands…   {}", s),
                    None => tr("Search commands…").into(),
                };
                let b = egui::Button::new(RichText::new(label).color(self.pal.text_dim))
                    .min_size(Vec2::new(230.0, 24.0));
                if ui
                    .add(b)
                    .on_hover_text(tr("Find and run any command, tool or setting"))
                    .clicked()
                {
                    self.run_command(ctx, C::CommandPalette);
                }
            });
        });
    }

    pub fn ribbon(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let essential = self.prefs.workspace == Workspace::Essential;
        let has_form = self.has_form();
        let tabs: Vec<(RibbonTab, &str)> = {
            let mut v = vec![(RibbonTab::File, tr("File")), (RibbonTab::Home, tr("Home"))];
            v.push((RibbonTab::Edit, tr("Edit")));
            v.push((RibbonTab::Comment, tr("Comment")));
            if has_form {
                v.push((RibbonTab::Forms, tr("Forms")));
            }
            v.push((RibbonTab::Sign, tr("Sign")));
            v.push((RibbonTab::Redact, tr("Redact")));
            v.push((RibbonTab::Copilot, tr("Copilot")));
            if !essential || self.tool_is_measure() {
                v.push((RibbonTab::Measure, tr("Measure")));
            }
            v.push((RibbonTab::Organize, tr("Organize")));
            v.push((RibbonTab::View, tr("View")));
            v
        };
        if !tabs.iter().any(|(t, _)| *t == self.ribbon_tab) {
            self.ribbon_tab = RibbonTab::Home;
        }
        ui.horizontal(|ui| {
            for (t, name) in &tabs {
                let sel = self.ribbon_tab == *t;
                let resp = ui.add(
                    egui::Label::new(RichText::new(*name).strong().color(if sel {
                        self.pal.accent
                    } else {
                        self.pal.text
                    }))
                    .sense(Sense::click())
                    .selectable(false),
                );
                if sel {
                    let r = resp.rect;
                    ui.painter().line_segment(
                        [
                            egui::pos2(r.min.x - 2.0, r.max.y + 2.0),
                            egui::pos2(r.max.x + 2.0, r.max.y + 2.0),
                        ],
                        egui::Stroke::new(2.5, self.pal.accent),
                    );
                }
                if resp.clicked() {
                    self.ribbon_tab = *t;
                }
                ui.add_space(12.0);
            }
        });
        ui.separator();
        let m = theme::metrics(self.prefs.density);
        let height = if m.show_labels {
            m.ribbon_button.y + 18.0
        } else {
            m.button_h + 18.0
        };
        egui::ScrollArea::horizontal().id_salt("ribbon_scroll").auto_shrink([false, true]).show(ui, |ui| {
            ui.set_min_height(height);
            ui.horizontal(|ui| match self.ribbon_tab {
                RibbonTab::File => {
                    self.group(ui, tr("Document"), |s, ui| s.cmds(ui, ctx, &[C::FileOpen, C::FileSave, C::FileSaveAs, C::FilePrint, C::FileSaveOptimized, C::FileConvertPdfA, C::FileProtection, C::FileClose]));
                    self.group(ui, tr("Recent"), |s, ui| {
                        let recents = s.prefs.recent_files.clone();
                        if recents.is_empty() {
                            ui.label(RichText::new(tr("No recent files")).color(s.pal.text_dim));
                        } else {
                            ui.vertical(|ui| {
                                for p in recents.iter().take(3) {
                                    let name = std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.clone());
                                    if ui.link(name).on_hover_text(p).clicked() {
                                        s.open_path(ctx, std::path::Path::new(p));
                                    }
                                }
                            });
                        }
                    });
                    self.group(ui, tr("Application"), |s, ui| {
                        // A Microsoft Store copy is updated by the Store: no "Check for Updates".
                        if platform::distribution::is_store() {
                            s.cmds(ui, ctx, &[C::Preferences, C::ShortcutReference, C::About]);
                        } else {
                            s.cmds(ui, ctx, &[C::Preferences, C::ShortcutReference, C::CheckForUpdates, C::About]);
                        }
                    });
                }
                RibbonTab::Home => {
                    self.group(ui, tr("History"), |s, ui| s.cmds(ui, ctx, &[C::EditUndo, C::EditRedo]));
                    self.group(ui, tr("Tools"), |s, ui| s.cmds(ui, ctx, &[C::ToolSelect, C::ToolHand, C::ToolTextSelect]));
                    self.group(ui, tr("Quick markup"), |s, ui| {
                        let favs = s.prefs.favorites.clone();
                        s.cmds(ui, ctx, &favs);
                    });
                    self.group(ui, tr("View"), |s, ui| s.cmds(ui, ctx, &[C::ViewZoomIn, C::ViewZoomOut, C::ViewFitWidth, C::ViewFitPage]));
                    self.group(ui, tr("Search"), |s, ui| s.cmds(ui, ctx, &[C::Find]));
                }
                RibbonTab::Comment => {
                    self.group(ui, tr("Text markup"), |s, ui| s.cmds(ui, ctx, &[C::ToolHighlight, C::ToolUnderline, C::ToolStrikeOut]));
                    self.group(ui, tr("Notes"), |s, ui| s.cmds(ui, ctx, &[C::ToolNote, C::ToolFreeText, C::ToolCallout]));
                    self.group(ui, tr("Drawing"), |s, ui| {
                        let ids: &[C] = if essential {
                            &[C::ToolRectangle, C::ToolEllipse, C::ToolCloud, C::ToolLine, C::ToolArrow, C::ToolInk]
                        } else {
                            &[C::ToolRectangle, C::ToolEllipse, C::ToolCloud, C::ToolLine, C::ToolArrow, C::ToolPolygon, C::ToolPolyline, C::ToolInk]
                        };
                        s.cmds(ui, ctx, ids);
                    });
                    self.group(ui, tr("Stamps"), |s, ui| s.cmds(ui, ctx, &[C::ToolStamp, C::NewStamp]));
                    self.group(ui, tr("Selection"), |s, ui| s.cmds(ui, ctx, &[C::ToolSelect, C::EditDuplicate, C::EditDelete]));
                }
                RibbonTab::Forms => {
                    self.group(ui, tr("Fill"), |s, ui| s.cmds(ui, ctx, &[C::ToolFillForm]));
                    self.group(ui, tr("Finish"), |s, ui| s.cmds(ui, ctx, &[C::FormFlatten]));
                    let n = self.form_for().fields.len();
                    ui.vertical(|ui| {
                        ui.add_space(6.0);
                        ui.label(RichText::new(tf!("{} form field(s). Click a field to fill it.\nScripts and calculations are never run.", n)).size(11.0).color(s_dim(&self.pal)));
                    });
                }
                RibbonTab::Sign => {
                    self.group(ui, tr("Digital signature"), |s, ui| s.cmds(ui, ctx, &[C::SignDocument, C::ShowSignatures]));
                    self.group(ui, tr("Handwritten signature"), |s, ui| s.cmds(ui, ctx, &[C::DrawSignature, C::SignFromImage, C::ToolPlaceSignature]));
                    ui.vertical(|ui| {
                        ui.add_space(6.0);
                        ui.label(RichText::new(tr("A digital signature proves the file is unchanged since signing.\nA handwritten signature is only a picture of your signature.")).size(11.0).color(s_dim(&self.pal)));
                    });
                }
                RibbonTab::Redact => {
                    self.group(ui, tr("Mark"), |s, ui| s.cmds(ui, ctx, &[C::ToolRedact, C::RedactMarkSelection]));
                    self.group(ui, tr("Remove"), |s, ui| s.cmds(ui, ctx, &[C::RedactApply]));
                    ui.vertical(|ui| {
                        ui.add_space(6.0);
                        ui.label(RichText::new(tr("Mark what must go, review the red frames, then apply.
Applying cannot be undone: the page becomes a picture with black boxes.")).size(11.0).color(s_dim(&self.pal)));
                    });
                }
                RibbonTab::Copilot => {
                    self.group(ui, tr("Ask"), |s, ui| s.cmds(ui, ctx, &[C::ToggleCopilot, C::CopilotSummarize, C::CopilotSummarizeAnnotations]));
                    self.group(ui, tr("Translate"), |s, ui| s.cmds(ui, ctx, &[C::TranslateDocument]));
                    ui.vertical(|ui| {
                        ui.add_space(6.0);
                        ui.label(RichText::new(tr("Uses your own OpenAI or Anthropic key (Preferences).\nText is sent only when you press a button.")).size(11.0).color(s_dim(&self.pal)));
                    });
                }
                RibbonTab::Measure => {
                    self.group(ui, tr("Scale"), |s, ui| s.cmds(ui, ctx, &[C::ToolCalibrate]));
                    self.group(ui, tr("Measure"), |s, ui| s.cmds(ui, ctx, &[C::ToolMeasureDistance, C::ToolMeasurePerimeter, C::ToolMeasureArea, C::ToolMeasureRect, C::ToolMeasureRadius, C::ToolMeasureAngle]));
                    self.group(ui, tr("Count"), |s, ui| s.cmds(ui, ctx, &[C::ToolCount]));
                    self.group(ui, tr("Snap"), |s, ui| s.cmds(ui, ctx, &[C::ToggleSnap]));
                    self.group(ui, tr("Report"), |s, ui| s.cmds(ui, ctx, &[C::ExportMeasurements]));
                }
                RibbonTab::Organize => {
                    self.group(ui, tr("Pages"), |s, ui| s.cmds(ui, ctx, &[C::PageRotateCounterClockwise, C::PageRotateClockwise, C::PageInsertBlank, C::PageInsertImage, C::PageDuplicate, C::PageDelete]));
                    self.group(ui, tr("Order"), |s, ui| s.cmds(ui, ctx, &[C::PageMoveUp, C::PageMoveDown]));
                    self.group(ui, tr("Documents"), |s, ui| s.cmds(ui, ctx, &[C::PageExtract, C::DocumentMerge]));
                }
                RibbonTab::View => {
                    self.group(ui, tr("Zoom"), |s, ui| s.cmds(ui, ctx, &[C::ViewZoomIn, C::ViewZoomOut, C::ViewZoomActual, C::ViewFitPage, C::ViewFitWidth]));
                    self.group(ui, tr("Layout"), |s, ui| s.cmds(ui, ctx, &[C::ViewModeContinuous, C::ViewModeSingle, C::ViewModeFacing]));
                    self.group(ui, tr("Rotate view"), |s, ui| s.cmds(ui, ctx, &[C::ViewRotateCounterClockwise, C::ViewRotateClockwise]));
                    self.group(ui, tr("Reading"), |s, ui| s.cmds(ui, ctx, &[C::ViewDarkPages]));
                    self.group(ui, tr("Panels"), |s, ui| s.cmds(ui, ctx, &[C::ViewToggleLeftSidebar, C::ViewToggleRightSidebar]));
                }
                RibbonTab::Edit => {
                    self.group(ui, tr("Page content"), |s, ui| s.cmds(ui, ctx, &[C::ToolEditText, C::ToolAddText, C::ToolAddImage]));
                    self.group(ui, tr("Scanned pages"), |s, ui| s.cmds(ui, ctx, &[C::OcrDocument]));
                    self.group(ui, tr("Selection"), |s, ui| s.cmds(ui, ctx, &[C::EditDelete]));
                    ui.vertical(|ui| {
                        ui.add_space(6.0);
                        ui.label(RichText::new(tr("These tools change the page's real content —\nnot comments. Annotations live on the Comment tab.")).size(11.0).color(s_dim(&self.pal)));
                    });
                }
            });
        });
    }

    pub fn tab_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let m = theme::metrics(self.prefs.density);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let mut close: Option<usize> = None;
            let mut activate: Option<usize> = None;
            for (i, t) in self.tabs.iter().enumerate() {
                let sel = i == self.active;
                let title = format!(
                    "{}{}",
                    if t.session.is_dirty() { "● " } else { "" },
                    t.session.title
                );
                let galley = ui.painter().layout_no_wrap(
                    title,
                    egui::FontId::proportional(13.0),
                    self.pal.text,
                );
                let w = (galley.size().x + 44.0).clamp(110.0, 260.0);
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, m.tab_h), Sense::click());
                let fill = if sel {
                    self.pal.panel
                } else if resp.hovered() {
                    self.pal.hover
                } else {
                    self.pal.chrome
                };
                ui.painter().rect_filled(
                    rect,
                    egui::CornerRadius {
                        nw: 6,
                        ne: 6,
                        sw: 0,
                        se: 0,
                    },
                    fill,
                );
                if sel {
                    ui.painter().line_segment(
                        [rect.left_top(), rect.right_top()],
                        egui::Stroke::new(2.5, self.pal.accent),
                    );
                }
                let mut text_rect = rect;
                text_rect.min.x += 10.0;
                text_rect.max.x -= 26.0;
                ui.painter().with_clip_rect(text_rect).galley(
                    egui::pos2(text_rect.min.x, rect.center().y - galley.size().y / 2.0),
                    galley,
                    self.pal.text,
                );
                let x_rect = Rect::from_center_size(
                    egui::pos2(rect.max.x - 14.0, rect.center().y),
                    Vec2::splat(16.0),
                );
                let x_resp = ui.interact(x_rect, ui.id().with(("tabx", i)), Sense::click());
                if x_resp.hovered() {
                    ui.painter().rect_filled(x_rect, 3.0, self.pal.hover);
                }
                icons::paint(
                    ui.painter(),
                    x_rect.shrink(3.0),
                    Icon::Close,
                    self.pal.text_dim,
                );
                x_resp.clone().on_hover_text(tr("Close tab"));
                if x_resp.clicked() || resp.middle_clicked() {
                    close = Some(i);
                } else if resp.clicked() {
                    activate = Some(i);
                }
                resp.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &t.session.title)
                });
                resp.on_hover_text(
                    t.session
                        .path
                        .as_ref()
                        .map_or_else(|| t.session.title.clone(), |p| p.display().to_string()),
                );
            }
            if let Some(i) = activate {
                self.active = i;
            }
            if let Some(i) = close {
                self.request_close(i);
            }
            if self.cmd_button(ui, C::FileOpen, false, Some(tr("Open")), true) {
                self.run_command(ctx, C::FileOpen);
            }
        });
    }

    pub fn status_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            let hint = if self.tabs.is_empty() {
                tr("Open a PDF or image to begin").to_string()
            } else {
                tr(self.tool.hint()).to_string()
            };
            ui.label(RichText::new(hint).color(self.pal.text_dim).size(12.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Background work.
                let queued = self.hub.queued() + self.in_flight.len();
                let search = self.active_tab().map(|t| t.session.search.clone());
                if let Some(s) = &search
                    && s.running
                {
                    ui.add(
                        egui::ProgressBar::new(s.progress.0 as f32 / s.progress.1.max(1) as f32)
                            .desired_width(90.0)
                            .text(tr("Searching")),
                    );
                } else if queued > 0 {
                    ui.add(egui::Spinner::new().size(12.0));
                    ui.label(
                        RichText::new(tf!("{} rendering", queued))
                            .color(self.pal.text_dim)
                            .size(12.0),
                    );
                }
                if let Some(r) = self.update.available.clone() {
                    if ui
                        .add(
                            egui::Label::new(
                                RichText::new(tf!("Update available: {}", r.version))
                                    .color(self.pal.accent)
                                    .size(12.0),
                            )
                            .sense(Sense::click()),
                        )
                        .on_hover_text(tr("Open the release page"))
                        .clicked()
                    {
                        self.open_update_page();
                    }
                    ui.separator();
                }
                let Some(info) = self.status_info() else {
                    return;
                };
                let (z, mode, zmode) = (info.zoom, info.mode, info.zoom_mode);
                if ui.small_button("+").on_hover_text(tr("Zoom in")).clicked() {
                    self.run_command(ctx, C::ViewZoomIn);
                }
                let label = match zmode {
                    ZoomMode::FitWidth => tf!("{}% (fit width)", format!("{:.0}", z * 100.0)),
                    ZoomMode::FitPage => tf!("{}% (fit page)", format!("{:.0}", z * 100.0)),
                    ZoomMode::Custom => format!("{:.0}%", z * 100.0),
                };
                ui.menu_button(label, |ui| {
                    for p in [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 4.0, 8.0] {
                        if ui.button(format!("{:.0}%", p * 100.0)).clicked() {
                            self.request_zoom(p, None);
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui.button(tr("Fit width")).clicked() {
                        self.run_command(ctx, C::ViewFitWidth);
                        ui.close();
                    }
                    if ui.button(tr("Fit page")).clicked() {
                        self.run_command(ctx, C::ViewFitPage);
                        ui.close();
                    }
                });
                if ui.small_button("−").on_hover_text(tr("Zoom out")).clicked() {
                    self.run_command(ctx, C::ViewZoomOut);
                }
                ui.separator();
                ui.label(
                    RichText::new(match mode {
                        ViewMode::Continuous => tr("Continuous"),
                        ViewMode::SinglePage => tr("Single page"),
                        ViewMode::Facing => tr("Facing"),
                    })
                    .color(self.pal.text_dim)
                    .size(12.0),
                );
                ui.separator();
                let cur = info.current;
                let rotated = info.rotated;
                {
                    if let Some((w, h)) = info.page_size {
                        ui.label(
                            RichText::new(format_size(w, h))
                                .color(self.pal.text_dim)
                                .size(12.0),
                        );
                        if rotated != 0 {
                            ui.label(
                                RichText::new(tf!("view rotated {}°", rotated))
                                    .color(self.pal.accent)
                                    .size(12.0),
                            );
                        }
                    }
                    if self
                        .active_tab()
                        .is_some_and(|t| t.session.doc().capabilities().has_signatures)
                        && ui
                            .add(
                                egui::Label::new(
                                    RichText::new(tr("Signed"))
                                        .color(self.pal.accent)
                                        .size(12.0),
                                )
                                .sense(Sense::click()),
                            )
                            .on_hover_text(tr(
                                "This document contains digital signatures. Click to check them.",
                            ))
                            .clicked()
                    {
                        self.open_signatures();
                    }
                    if let Some((txt, ok)) = self.status_scale() {
                        ui.separator();
                        ui.label(
                            RichText::new(txt)
                                .color(if ok {
                                    self.pal.text_dim
                                } else {
                                    self.pal.danger
                                })
                                .size(12.0),
                        );
                    }
                    ui.separator();
                    let txt = tf!("Page {} of {}", cur + 1, info.page_count);
                    if ui
                        .add(egui::Label::new(RichText::new(txt).size(12.0)).sense(Sense::click()))
                        .on_hover_text(tr("Go to page…"))
                        .clicked()
                    {
                        self.run_command(ctx, C::GoToPage);
                    }
                }
            });
        });
    }

    pub fn welcome(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.18);
            let (r, _) = ui.allocate_exact_size(Vec2::new(96.0, 96.0), Sense::hover());
            icons::paint_logo(ui.painter(), r, self.pal.text, self.pal.accent);
            ui.add_space(8.0);
            ui.label(
                RichText::new("BergPDF")
                    .size(34.0)
                    .strong()
                    .color(self.pal.text),
            );
            ui.label(
                RichText::new(tr("Fast, offline PDF editing"))
                    .size(15.0)
                    .color(self.pal.text_dim),
            );
            ui.add_space(18.0);
            if ui
                .add(
                    egui::Button::new(
                        RichText::new(tr("Open a PDF or image…"))
                            .size(15.0)
                            .color(Color32::WHITE),
                    )
                    .fill(self.pal.accent)
                    .min_size(Vec2::new(180.0, 38.0)),
                )
                .clicked()
            {
                self.run_command(ctx, C::FileOpen);
            }
            ui.add_space(6.0);
            ui.label(
                RichText::new(tr("or drop a PDF or picture anywhere in this window"))
                    .color(self.pal.text_dim)
                    .size(12.0),
            );
            if !self.prefs.recent_files.is_empty() {
                ui.add_space(22.0);
                ui.label(RichText::new(tr("Recent")).strong());
                for p in self.prefs.recent_files.clone().into_iter().take(6) {
                    let name = std::path::Path::new(&p)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| p.clone());
                    if ui.link(name).on_hover_text(&p).clicked() {
                        self.open_path(ctx, std::path::Path::new(&p));
                    }
                }
            }
        });
    }
}

/// Page size label using metric or imperial depending on the user's locale.
pub fn format_size(w_pt: f64, h_pt: f64) -> String {
    let imperial = ["LC_MEASUREMENT", "LC_ALL", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty())
        .is_some_and(|v| {
            v.starts_with("en_US") || v.starts_with("en_LR") || v.starts_with("my_MM")
        });
    if imperial {
        format!("{:.2} × {:.2} in", w_pt / 72.0, h_pt / 72.0)
    } else {
        format!("{:.0} × {:.0} mm", w_pt / 72.0 * 25.4, h_pt / 72.0 * 25.4)
    }
}

#[allow(dead_code)]
fn density_name(d: Density) -> &'static str {
    match d {
        Density::Compact => tr("Compact"),
        Density::Comfortable => tr("Comfortable"),
        Density::Touch => tr("Touch / Pen"),
    }
}

/// Values the status bar needs, copied out so the tab borrow ends before widgets run.
pub struct StatusInfo {
    pub zoom: f64,
    pub mode: ViewMode,
    pub zoom_mode: ZoomMode,
    pub current: usize,
    pub rotated: i64,
    pub page_count: usize,
    pub page_size: Option<(f64, f64)>,
}

impl App {
    fn status_info(&mut self) -> Option<StatusInfo> {
        let t = self.tabs.get_mut(self.active)?;
        let pages = t.session.pages().ok()?;
        let cur = t
            .session
            .view
            .current_page
            .min(pages.len().saturating_sub(1));
        let rot = t.session.view.rotation;
        Some(StatusInfo {
            zoom: t.session.view.zoom,
            mode: t.session.view.mode,
            zoom_mode: t.session.view.zoom_mode,
            current: cur,
            rotated: rot.degrees(),
            page_count: pages.len(),
            page_size: pages.get(cur).map(|p| {
                let s = p.geometry.view_size(rot);
                (s.width, s.height)
            }),
        })
    }
}

fn s_dim(p: &crate::theme::Palette) -> egui::Color32 {
    p.text_dim
}

/// Short ribbon captions for commands whose full title is too long for a ribbon button.
/// (The palette, tooltips and menus keep the full title.)
fn ribbon_label(id: C, title: &'static str) -> &'static str {
    match id {
        C::PageRotateCounterClockwise => tr("Rotate Left"),
        C::PageRotateClockwise => tr("Rotate Right"),
        C::ViewRotateCounterClockwise => tr("Rotate View Left"),
        C::ViewRotateClockwise => tr("Rotate View Right"),
        C::PageInsertBlank => tr("Blank Page"),
        C::PageInsertImage => tr("Image Page"),
        C::PageDuplicate => tr("Duplicate"),
        C::PageDelete => tr("Delete"),
        C::PageMoveUp => tr("Move Up"),
        C::PageMoveDown => tr("Move Down"),
        C::PageExtract => tr("Extract"),
        C::DocumentMerge => tr("Merge"),
        C::ToolMeasurePerimeter => tr("Path Length"),
        C::ToolMeasureDistance => tr("Distance"),
        C::ToolMeasureRect => tr("Rectangle"),
        C::ExportMeasurements => tr("Export CSV"),
        C::OcrDocument => "OCR",
        C::ToggleSnap => tr("Snap"),
        C::FileSaveOptimized => tr("Optimize"),
        C::FileConvertPdfA => "PDF/A",
        C::FileProtection => tr("Protect"),
        C::FilePrint => tr("Print"),
        C::ToolRedact => tr("Mark Area"),
        C::RedactMarkSelection => tr("Mark Text"),
        C::RedactApply => tr("Apply"),
        C::ToggleCopilot => tr("Copilot"),
        C::CopilotSummarize => tr("Summarize"),
        C::CopilotSummarizeAnnotations => tr("Summarize notes"),
        C::TranslateDocument => tr("Translate"),
        C::SignDocument => tr("Sign"),
        C::ShowSignatures => tr("Signatures"),
        C::DrawSignature => tr("Draw Signature"),
        C::SignFromImage => tr("Signature from Picture"),
        C::NewStamp => tr("New Stamp"),
        C::ToolPlaceSignature => tr("Place Signature"),
        C::ToolFillForm => tr("Fill Form"),
        C::FormFlatten => tr("Flatten"),
        C::FileExportImage => tr("Export Image"),
        C::FileProperties => tr("Properties"),
        C::ToolHighlight => tr("Highlight"),
        C::ToolUnderline => tr("Underline"),
        C::ToolStrikeOut => tr("Strikeout"),
        C::ToolCalibrate => tr("Calibrate"),
        C::ViewZoomActual => "100 %",
        C::ViewDarkPages => tr("Dark Pages"),
        C::ViewToggleLeftSidebar => tr("Navigation"),
        C::ViewToggleRightSidebar => tr("Properties"),
        C::ShortcutReference => tr("Shortcuts"),
        _ => tr(title),
    }
}
