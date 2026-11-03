//! Window chrome (docs/DESIGN.md §3–4): title row, ribbon, document tabs, status bar.

use crate::app::OS;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::state::*;
use crate::tf;
use crate::theme;
use editor_core::command::CommandId as C;
use editor_core::prefs::{Density, Workspace};
use editor_core::tools::{Tool, ToolFamily};
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
        Tool::Crop => Icon::Crop,
        Tool::FieldText => Icon::FieldText,
        Tool::FieldCheckbox => Icon::FieldCheck,
        Tool::FieldRadio => Icon::FieldRadio,
        Tool::FieldCombo => Icon::FieldCombo,
        Tool::FieldList => Icon::FieldList,
        Tool::FieldButton => Icon::FieldButton,
        Tool::FieldSignature => Icon::FieldSign,
        Tool::FormDesign => Icon::FormDesign,
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
        C::PageHeaderFooter | C::PageBates => Icon::HeaderFooter,
        C::PageWatermark => Icon::Watermark,
        C::PageRemoveMarks => Icon::Trash,
        C::PageLabels => Icon::PageLabel,
        C::PageCrop => Icon::Crop,
        C::BookmarkAdd => Icon::BookmarkAdd,
        C::DocumentCompare => Icon::Compare,
        C::FormScripts | C::DocumentJavaScript => Icon::Script,
        C::FormCalculateNow => Icon::Calibrate,
        _ => Icon::Page,
    }
}

impl App {
    /// Whether the start screen fills the window (no document, or its "+" tab).
    pub fn on_start_screen(&self) -> bool {
        self.tabs.is_empty() || self.show_start
    }

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

    // ---- title row (docs/DESIGN.md §4.1) ------------------------------------------------------

    /// A quick-access icon button: 30 × 30, radius 7, icon 17.
    fn title_icon_button(&mut self, ui: &mut egui::Ui, id: C) -> bool {
        let enabled = self.command_enabled(id);
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
        let pal = self.pal;
        if resp.hovered() && enabled {
            let fill = if resp.is_pointer_button_down_on() {
                pal.pressed
            } else {
                pal.hover
            };
            ui.painter().rect_filled(rect, theme::radius::MD, fill);
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(rect.center(), Vec2::splat(17.0)),
            command_icon(id),
            if enabled { pal.text } else { pal.text_disabled },
        );
        let info = editor_core::command::info(id);
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tr(info.title))
        });
        let resp = resp.on_hover_text(self.tooltip_for(id));
        enabled && resp.clicked()
    }

    /// The top row: logo, name, quick access, and the command search centred on the window.
    pub fn quick_access_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let m = theme::metrics(self.prefs.density);
        let full = ui.max_rect();
        let start = self.on_start_screen();
        ui.allocate_ui_with_layout(
            Vec2::new(full.width(), m.title_h),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(10.0);
                let (r, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                icons::paint_logo(ui.painter(), r, self.pal.text, self.pal.accent);
                ui.add_space(6.0);
                ui.add(
                    egui::Label::new(
                        RichText::new("BergPDF")
                            .font(theme::font(600, 13.0))
                            .color(self.pal.text),
                    )
                    .selectable(false),
                );
                ui.add_space(10.0);
                let ids: &[C] = if start {
                    &[C::FileOpen]
                } else {
                    &[C::FileOpen, C::FileSave, C::EditUndo, C::EditRedo]
                };
                for &id in ids {
                    if self.title_icon_button(ui, id) {
                        self.run_command(ctx, id);
                    }
                    ui.add_space(2.0);
                }
            },
        );
        // The command search: 400 × 28, centred on the window rather than on the free space.
        let w = 400.0_f32.min(full.width() - 360.0).max(180.0);
        let rect = Rect::from_center_size(
            egui::pos2(full.center().x, full.top() + m.title_h / 2.0),
            Vec2::new(w, 28.0),
        );
        let resp = ui.interact(rect, ui.id().with("command_search"), Sense::click());
        let pal = self.pal;
        let p = ui.painter();
        p.rect(
            rect,
            theme::radius::LG,
            if resp.hovered() { pal.hover } else { pal.card },
            egui::Stroke::new(1.0, pal.border),
            egui::StrokeKind::Inside,
        );
        icons::paint(
            p,
            Rect::from_center_size(
                egui::pos2(rect.left() + 10.0 + 7.5, rect.center().y),
                Vec2::splat(15.0),
            ),
            Icon::Find,
            pal.text_muted,
        );
        let chip = self
            .prefs
            .keybindings
            .primary_label(C::CommandPalette, OS)
            .unwrap_or_default();
        let chip_g = p.layout_no_wrap(chip.clone(), theme::font(500, 11.0), pal.text_muted);
        let chip_rect = Rect::from_min_size(
            egui::pos2(
                rect.right() - 8.0 - chip_g.size().x - 12.0,
                rect.center().y - (chip_g.size().y + 2.0) / 2.0,
            ),
            chip_g.size() + Vec2::new(12.0, 2.0),
        );
        if !chip.is_empty() {
            p.rect(
                chip_rect,
                theme::radius::SM,
                pal.track,
                egui::Stroke::new(1.0, pal.border),
                egui::StrokeKind::Inside,
            );
            p.galley(
                chip_rect.center() - chip_g.size() / 2.0,
                chip_g,
                pal.text_muted,
            );
        }
        let hint = p.layout_no_wrap(
            tr("Search commands, e.g. “rotate” or “OCR”").to_string(),
            egui::FontId::proportional(13.0),
            pal.text_muted,
        );
        let text_clip = Rect::from_min_max(
            egui::pos2(rect.left() + 32.0, rect.top()),
            egui::pos2(chip_rect.left() - 6.0, rect.bottom()),
        );
        p.with_clip_rect(text_clip).galley(
            egui::pos2(text_clip.left(), rect.center().y - hint.size().y / 2.0),
            hint,
            pal.text_muted,
        );
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tr("Search commands…"))
        });
        if resp
            .on_hover_text(tr("Find and run any command, tool or setting"))
            .clicked()
        {
            self.run_command(ctx, C::CommandPalette);
        }
    }

    // ---- ribbon (docs/DESIGN.md §4.2, §4.3) --------------------------------------------------

    fn ribbon_tabs_list(&mut self) -> Vec<(RibbonTab, &'static str)> {
        let essential = self.prefs.workspace == Workspace::Essential;
        let has_form = self.has_form();
        let mut v = vec![(RibbonTab::File, tr("File")), (RibbonTab::Home, tr("Home"))];
        v.push((RibbonTab::Edit, tr("Edit")));
        v.push((RibbonTab::Comment, tr("Comment")));
        // Filling is for documents with a form; making one is always possible in Professional.
        if has_form || !essential || self.tool.family() == ToolFamily::Form {
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
    }

    /// The groups of the active ribbon tab, shared by the single-line and the classic ribbon.
    fn ribbon_groups(&mut self) -> Vec<RibbonGroup> {
        let essential = self.prefs.workspace == Workspace::Essential;
        let g = |title: &'static str, cmds: &[C]| RibbonGroup {
            title,
            cmds: cmds.to_vec(),
            extra: GroupExtra::None,
        };
        let note = |text: String| RibbonGroup {
            title: "",
            cmds: Vec::new(),
            extra: GroupExtra::Note(text),
        };
        match self.ribbon_tab {
            RibbonTab::File => {
                let app: &[C] = if platform::distribution::is_store() {
                    &[C::Preferences, C::ShortcutReference, C::About]
                } else {
                    &[C::Preferences, C::ShortcutReference, C::CheckForUpdates, C::About]
                };
                vec![
                    g(tr("Document"), &[C::FileOpen, C::FileSave, C::FileSaveAs, C::FilePrint, C::FileSaveOptimized, C::FileConvertPdfA, C::FileProtection, C::FileClose]),
                    RibbonGroup {
                        title: tr("Recent"),
                        cmds: Vec::new(),
                        extra: GroupExtra::Recent,
                    },
                    g(tr("Compare"), &[C::DocumentCompare]),
                    g(tr("Application"), app),
                ]
            }
            RibbonTab::Home => {
                let favs = self.prefs.favorites.clone();
                vec![
                    g(tr("History"), &[C::EditUndo, C::EditRedo]),
                    g(tr("Tools"), &[C::ToolSelect, C::ToolHand, C::ToolTextSelect]),
                    g(tr("Quick markup"), &favs),
                    g(tr("View"), &[C::ViewZoomIn, C::ViewZoomOut, C::ViewFitWidth, C::ViewFitPage]),
                    g(tr("Search"), &[C::Find]),
                ]
            }
            RibbonTab::Comment => vec![
                g(tr("Text markup"), &[C::ToolHighlight, C::ToolUnderline, C::ToolStrikeOut]),
                g(tr("Notes"), &[C::ToolNote, C::ToolFreeText, C::ToolCallout]),
                if essential {
                    g(tr("Drawing"), &[C::ToolRectangle, C::ToolEllipse, C::ToolCloud, C::ToolLine, C::ToolArrow, C::ToolInk])
                } else {
                    g(tr("Drawing"), &[C::ToolRectangle, C::ToolEllipse, C::ToolCloud, C::ToolLine, C::ToolArrow, C::ToolPolygon, C::ToolPolyline, C::ToolInk])
                },
                g(tr("Stamps"), &[C::ToolStamp, C::NewStamp]),
                g(tr("Selection"), &[C::ToolSelect, C::EditDuplicate, C::EditDelete]),
            ],
            RibbonTab::Forms => {
                let n = self.form_for().fields.len();
                let text = if self.scripts_allowed() {
                    tf!("{} form field(s). Scripts of this form run (Form scripts to change).", n)
                } else {
                    tf!("{} form field(s). Scripts of this form do not run (Form scripts to allow).", n)
                };
                vec![
                    g(tr("Fill"), &[C::ToolFillForm]),
                    g(tr("Prepare"), &[C::ToolFormDesign, C::ToolFieldText, C::ToolFieldCheckbox, C::ToolFieldRadio, C::ToolFieldCombo, C::ToolFieldList, C::ToolFieldButton, C::ToolFieldSignature]),
                    g(tr("Scripts"), &[C::FormScripts, C::FormCalculateNow, C::DocumentJavaScript]),
                    g(tr("Finish"), &[C::FormFlatten]),
                    note(text),
                ]
            }
            RibbonTab::Sign => vec![
                g(tr("Digital signature"), &[C::SignDocument, C::ShowSignatures]),
                g(tr("Handwritten signature"), &[C::DrawSignature, C::SignFromImage, C::ToolPlaceSignature]),
                note(tr("A digital signature proves the file is unchanged since signing.\nA handwritten signature is only a picture of your signature.").into()),
            ],
            RibbonTab::Redact => vec![
                g(tr("Mark"), &[C::ToolRedact, C::RedactMarkSelection]),
                g(tr("Remove"), &[C::RedactApply]),
                note(tr("Mark what must go, review the red frames, then apply.
Applying cannot be undone: the page becomes a picture with black boxes.").into()),
            ],
            RibbonTab::Copilot => vec![
                g(tr("Ask"), &[C::ToggleCopilot, C::CopilotSummarize, C::CopilotSummarizeAnnotations]),
                g(tr("Translate"), &[C::TranslateDocument]),
                note(tr("Uses your own OpenAI or Anthropic key (Preferences).\nText is sent only when you press a button.").into()),
            ],
            RibbonTab::Measure => vec![
                g(tr("Scale"), &[C::ToolCalibrate]),
                g(tr("Measure"), &[C::ToolMeasureDistance, C::ToolMeasurePerimeter, C::ToolMeasureArea, C::ToolMeasureRect, C::ToolMeasureRadius, C::ToolMeasureAngle]),
                g(tr("Count"), &[C::ToolCount]),
                g(tr("Snap"), &[C::ToggleSnap]),
                g(tr("Report"), &[C::ExportMeasurements]),
            ],
            RibbonTab::Organize => vec![
                g(tr("Pages"), &[C::PageRotateCounterClockwise, C::PageRotateClockwise, C::PageInsertBlank, C::PageInsertImage, C::PageDuplicate, C::PageDelete]),
                g(tr("Order"), &[C::PageMoveUp, C::PageMoveDown]),
                g(tr("Documents"), &[C::PageExtract, C::DocumentMerge, C::DocumentCompare]),
                g(tr("Page marks"), &[C::PageHeaderFooter, C::PageBates, C::PageWatermark, C::PageRemoveMarks]),
                g(tr("Page setup"), &[C::PageLabels, C::PageCrop, C::ToolCrop]),
            ],
            RibbonTab::View => vec![
                g(tr("Zoom"), &[C::ViewZoomIn, C::ViewZoomOut, C::ViewZoomActual, C::ViewFitPage, C::ViewFitWidth]),
                g(tr("Layout"), &[C::ViewModeContinuous, C::ViewModeSingle, C::ViewModeFacing]),
                g(tr("Rotate view"), &[C::ViewRotateCounterClockwise, C::ViewRotateClockwise]),
                g(tr("Reading"), &[C::ViewDarkPages]),
                g(tr("Panels"), &[C::ViewToggleLeftSidebar, C::ViewToggleRightSidebar]),
            ],
            RibbonTab::Edit => vec![
                g(tr("Page content"), &[C::ToolEditText, C::ToolAddText, C::ToolAddImage]),
                g(tr("Scanned pages"), &[C::OcrDocument]),
                g(tr("Selection"), &[C::EditDelete]),
                note(tr("These tools change the page's real content —\nnot comments. Annotations live on the Comment tab.").into()),
            ],
        }
    }

    fn cmd_selected(&self, id: C) -> bool {
        Tool::from_command(id).is_some_and(|t| t == self.tool)
            || (id == C::ViewDarkPages && self.prefs.dark_page_filter)
            || (id == C::ToggleSnap && self.prefs.snap_to_geometry)
            || (id == C::ToggleCopilot
                && self.prefs.show_right_sidebar
                && self.right_tab == RightTab::Copilot)
            || (id == C::ViewToggleLeftSidebar && self.prefs.show_left_sidebar)
            || (id == C::ViewToggleRightSidebar && self.prefs.show_right_sidebar)
            || self.view_mode_selected(id)
    }

    /// Width of a single-line ribbon button.
    fn line_button_width(&self, ui: &egui::Ui, id: C, with_label: bool) -> f32 {
        let m = theme::metrics(self.prefs.density);
        if !with_label {
            return m.ribbon_button_h;
        }
        let info = editor_core::command::info(id);
        let text = ribbon_label(id, info.title).trim_end_matches('…');
        let w = ui
            .painter()
            .layout_no_wrap(text.to_string(), theme::font(600, 13.0), Color32::WHITE)
            .size()
            .x;
        11.0 + m.icon + 7.0 + w + 11.0
    }

    /// A single-line ribbon button: icon and label side by side (docs/DESIGN.md §4.3).
    fn line_button(&mut self, ui: &mut egui::Ui, id: C, with_label: bool) -> bool {
        let m = theme::metrics(self.prefs.density);
        let enabled = self.command_enabled(id);
        let selected = self.cmd_selected(id);
        let info = editor_core::command::info(id);
        let text = ribbon_label(id, info.title).trim_end_matches('…');
        let w = self.line_button_width(ui, id, with_label);
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, m.ribbon_button_h), Sense::click());
        let pal = self.pal;
        if ui.is_rect_visible(rect) {
            let fill = if selected {
                Some(pal.accent_soft)
            } else if enabled && resp.is_pointer_button_down_on() {
                Some(pal.pressed)
            } else if enabled && resp.hovered() {
                Some(pal.hover)
            } else {
                None
            };
            if let Some(f) = fill {
                ui.painter().rect_filled(rect, theme::radius::LG, f);
            }
            let (icon_col, text_col) = if !enabled {
                (pal.text_disabled, pal.text_disabled)
            } else if selected {
                (pal.accent, pal.accent_text)
            } else {
                (pal.text, pal.text)
            };
            let icon_rect = if with_label {
                Rect::from_center_size(
                    egui::pos2(rect.left() + 11.0 + m.icon / 2.0, rect.center().y),
                    Vec2::splat(m.icon),
                )
            } else {
                Rect::from_center_size(rect.center(), Vec2::splat(m.icon))
            };
            icons::paint(ui.painter(), icon_rect, command_icon(id), icon_col);
            if with_label {
                let font = if selected {
                    theme::font(600, 13.0)
                } else {
                    egui::FontId::proportional(13.0)
                };
                let g = ui
                    .painter()
                    .layout_no_wrap(text.to_string(), font, text_col);
                ui.painter().galley(
                    egui::pos2(icon_rect.right() + 7.0, rect.center().y - g.size().y / 2.0),
                    g,
                    text_col,
                );
            }
        }
        resp.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, tr(info.title))
        });
        let resp = resp.on_hover_text(self.tooltip_for(id));
        enabled && resp.clicked()
    }

    /// A row in the "More" menu: icon and full title.
    fn menu_cmd_row(&mut self, ui: &mut egui::Ui, id: C) -> bool {
        let enabled = self.command_enabled(id);
        let info = editor_core::command::info(id);
        let pal = self.pal;
        let g = ui.painter().layout_no_wrap(
            tr(info.title).to_string(),
            egui::FontId::proportional(13.0),
            pal.text,
        );
        let w = (16.0 + 10.0 + g.size().x + 16.0).max(200.0);
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 30.0), Sense::click());
        if enabled && resp.hovered() {
            ui.painter().rect_filled(rect, theme::radius::MD, pal.hover);
        }
        let col = if enabled { pal.text } else { pal.text_disabled };
        let ir = Rect::from_center_size(
            egui::pos2(rect.left() + 8.0 + 8.0, rect.center().y),
            Vec2::splat(16.0),
        );
        icons::paint(
            ui.painter(),
            ir,
            command_icon(id),
            if self.cmd_selected(id) {
                pal.accent
            } else {
                col
            },
        );
        ui.painter().galley(
            egui::pos2(ir.right() + 10.0, rect.center().y - g.size().y / 2.0),
            g,
            col,
        );
        let resp = resp.on_hover_text(self.tooltip_for(id));
        enabled && resp.clicked()
    }

    fn group_width(&self, ui: &egui::Ui, g: &RibbonGroup, labels: bool) -> f32 {
        let m = theme::metrics(self.prefs.density);
        let mut w: f32 = g
            .cmds
            .iter()
            .map(|c| self.line_button_width(ui, *c, labels))
            .sum::<f32>()
            + 2.0 * g.cmds.len().saturating_sub(1) as f32;
        match &g.extra {
            GroupExtra::Recent => w += 110.0,
            GroupExtra::Note(_) => w += m.ribbon_button_h,
            GroupExtra::None => {}
        }
        w
    }

    /// The ribbon: tabs row and the content row, either single-line or classic.
    pub fn ribbon(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let m = theme::metrics(self.prefs.density);
        let pal = self.pal;
        let tabs = self.ribbon_tabs_list();
        if !tabs.iter().any(|(t, _)| *t == self.ribbon_tab) {
            self.ribbon_tab = RibbonTab::Home;
        }
        let single = self.prefs.ribbon_style == editor_core::prefs::RibbonStyle::SingleLine;
        let full = ui.max_rect();
        let tabs_rect = Rect::from_min_size(full.min, Vec2::new(full.width(), m.ribbon_tabs_h));
        let row_h = if single {
            m.ribbon_h
        } else if m.show_labels {
            m.ribbon_button.y + 18.0 + 14.0
        } else {
            m.button_h + 18.0
        };
        let row_rect = Rect::from_min_size(
            egui::pos2(full.left(), tabs_rect.bottom()),
            Vec2::new(full.width(), row_h),
        );
        ui.allocate_rect(Rect::from_min_max(full.min, row_rect.max), Sense::hover());
        // The content row: surface with a hairline above and below.
        let p = ui.painter().clone();
        p.rect_filled(row_rect, 0.0, pal.card);
        p.hline(
            row_rect.x_range(),
            row_rect.top() + 0.5,
            egui::Stroke::new(1.0, pal.border),
        );
        p.hline(
            row_rect.x_range(),
            row_rect.bottom() - 0.5,
            egui::Stroke::new(1.0, pal.border),
        );
        // Tabs, aligned to the bottom; the active one joins the row below.
        let mut x = tabs_rect.left() + 10.0;
        let tab_top = tabs_rect.bottom() - m.ribbon_tab_h;
        for (t, name) in &tabs {
            let sel = self.ribbon_tab == *t;
            let font = if sel {
                theme::font(600, 13.0)
            } else {
                egui::FontId::proportional(13.0)
            };
            let col = if sel { pal.accent_text } else { pal.text };
            let g = p.layout_no_wrap((*name).to_string(), font, col);
            let r = Rect::from_min_size(
                egui::pos2(x, tab_top),
                Vec2::new(g.size().x + 28.0, m.ribbon_tab_h),
            );
            let resp = ui.interact(r, ui.id().with(("rtab", *name)), Sense::click());
            let corners = egui::CornerRadius {
                nw: theme::radius::LG,
                ne: theme::radius::LG,
                sw: 0,
                se: 0,
            };
            if sel {
                let cover = Rect::from_min_max(r.min, egui::pos2(r.max.x, r.max.y + 1.0));
                p.rect_filled(cover, corners, pal.card);
                let s = egui::Stroke::new(1.0, pal.border);
                p.vline(r.left() + 0.5, (r.top() + 6.0)..=(r.bottom() + 1.0), s);
                p.vline(r.right() - 0.5, (r.top() + 6.0)..=(r.bottom() + 1.0), s);
                p.hline((r.left() + 6.0)..=(r.right() - 6.0), r.top() + 0.5, s);
                p.add(egui::Shape::Path(egui::epaint::PathShape::line(
                    arc_pts(egui::pos2(r.left() + 8.0, r.top() + 8.0), 7.5, 180.0, 270.0),
                    s,
                )));
                p.add(egui::Shape::Path(egui::epaint::PathShape::line(
                    arc_pts(
                        egui::pos2(r.right() - 8.0, r.top() + 8.0),
                        7.5,
                        270.0,
                        360.0,
                    ),
                    s,
                )));
            } else if resp.hovered() {
                p.rect_filled(r, corners, pal.hover);
            }
            p.galley(r.center() - g.size() / 2.0, g, col);
            resp.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::Button, true, sel, *name)
            });
            if resp.clicked() {
                self.ribbon_tab = *t;
            }
            x = r.right() + 2.0;
        }
        // Far right: the "Single-line ribbon" switch.
        {
            let label = p.layout_no_wrap(
                tr("Single-line ribbon").to_string(),
                egui::FontId::proportional(12.0),
                pal.text_muted,
            );
            let sw = Rect::from_min_size(
                egui::pos2(tabs_rect.right() - 12.0 - 28.0, tabs_rect.center().y - 8.0),
                Vec2::new(28.0, 16.0),
            );
            if sw.left() - label.size().x - 8.0 > x + 16.0 {
                p.galley(
                    egui::pos2(
                        sw.left() - 8.0 - label.size().x,
                        tabs_rect.center().y - label.size().y / 2.0,
                    ),
                    label,
                    pal.text_muted,
                );
                let mut on = single;
                let resp = ui
                    .scope_builder(egui::UiBuilder::new().max_rect(sw), |ui| {
                        crate::ui_kit::switch(ui, &mut on)
                    })
                    .inner;
                if resp
                    .on_hover_text(tr("Off shows the classic ribbon: large icons above their labels, with group names"))
                    .changed()
                {
                    self.prefs.ribbon_style = if on {
                        editor_core::prefs::RibbonStyle::SingleLine
                    } else {
                        editor_core::prefs::RibbonStyle::Classic
                    };
                    self.prefs_dirty = true;
                }
            }
        }
        let groups = self.ribbon_groups();
        let inner = if single {
            row_rect.shrink2(Vec2::new(10.0, 0.0))
        } else {
            Rect::from_min_max(
                row_rect.min + Vec2::new(10.0, 6.0),
                row_rect.max - Vec2::new(10.0, 2.0),
            )
        };
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::left_to_right(if single {
                    egui::Align::Center
                } else {
                    egui::Align::Min
                })),
            |ui| {
                ui.set_clip_rect(inner);
                if single {
                    self.ribbon_single(ui, ctx, &groups, m.show_labels);
                } else {
                    self.ribbon_classic(ui, ctx, &groups);
                }
            },
        );
    }

    fn ribbon_single(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        groups: &[RibbonGroup],
        labels: bool,
    ) {
        ui.spacing_mut().item_spacing.x = 2.0;
        let more_w = 76.0;
        let avail = ui.available_width();
        // Whole groups, from the left, as long as they fit; the rest goes into "More".
        let mut used = 0.0;
        let mut shown = groups.len();
        let total: f32 = groups
            .iter()
            .enumerate()
            .map(|(i, g)| self.group_width(ui, g, labels) + if i > 0 { 17.0 } else { 0.0 })
            .sum();
        if total > avail {
            for (i, g) in groups.iter().enumerate() {
                let w = self.group_width(ui, g, labels) + if i > 0 { 17.0 } else { 0.0 };
                if used + w > avail - more_w {
                    shown = i;
                    break;
                }
                used += w;
            }
        }
        let pal = self.pal;
        for (i, g) in groups.iter().take(shown).enumerate() {
            if i > 0 {
                ui.add_space(6.0);
                let (r, _) = ui.allocate_exact_size(Vec2::new(1.0, 26.0), Sense::hover());
                ui.painter().rect_filled(r, 0.0, pal.border);
                ui.add_space(6.0);
            }
            for &id in &g.cmds {
                if self.line_button(ui, id, labels) {
                    self.run_command(ctx, id);
                }
            }
            match &g.extra {
                GroupExtra::Recent => self.recent_menu(ui, ctx),
                GroupExtra::Note(text) => {
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, theme::radius::MD, pal.hover);
                    }
                    icons::paint(ui.painter(), r.shrink(7.0), Icon::Info, pal.text_muted);
                    resp.on_hover_text(text.as_str());
                }
                GroupExtra::None => {}
            }
        }
        if shown < groups.len() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let more = egui::Button::new(RichText::new(tr("More ▾")).color(pal.text_muted))
                    .fill(Color32::TRANSPARENT)
                    .stroke(egui::Stroke::NONE);
                egui::containers::menu::MenuButton::from_button(more).ui(ui, |ui| {
                    for (j, g) in groups.iter().skip(shown).enumerate() {
                        if j > 0 {
                            ui.separator();
                        }
                        if !g.title.is_empty() {
                            ui.label(RichText::new(g.title).size(12.0).color(pal.text_muted));
                        }
                        for &id in &g.cmds {
                            if self.menu_cmd_row(ui, id) {
                                self.run_command(ctx, id);
                                ui.close();
                            }
                        }
                        match &g.extra {
                            GroupExtra::Recent => self.recent_list(ui, ctx),
                            GroupExtra::Note(text) => {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(text.as_str()).size(12.0).color(pal.text_dim),
                                    )
                                    .wrap(),
                                );
                            }
                            GroupExtra::None => {}
                        }
                    }
                });
            });
        }
    }

    fn recent_list(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let recents = self.prefs.start_files();
        if recents.is_empty() {
            ui.label(RichText::new(tr("No recent files")).color(self.pal.text_dim));
        }
        for p in recents.iter().take(8) {
            let name = std::path::Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.clone());
            if ui.button(name).on_hover_text(p).clicked() {
                self.open_path(ctx, std::path::Path::new(p));
                ui.close();
            }
        }
    }

    fn recent_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let b = egui::Button::new(RichText::new(tf!("{} ▾", tr("Recent"))).color(self.pal.text))
            .fill(Color32::TRANSPARENT)
            .stroke(egui::Stroke::NONE)
            .min_size(Vec2::new(
                0.0,
                theme::metrics(self.prefs.density).ribbon_button_h,
            ));
        egui::containers::menu::MenuButton::from_button(b).ui(ui, |ui| self.recent_list(ui, ctx));
    }

    fn ribbon_classic(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, groups: &[RibbonGroup]) {
        egui::ScrollArea::horizontal()
            .id_salt("ribbon_scroll")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for g in groups {
                        match &g.extra {
                            GroupExtra::Note(text) => {
                                ui.vertical(|ui| {
                                    ui.add_space(6.0);
                                    ui.label(
                                        RichText::new(text.as_str())
                                            .size(11.0)
                                            .color(self.pal.text_dim),
                                    );
                                });
                            }
                            GroupExtra::Recent => {
                                self.group(ui, g.title, |s, ui| {
                                    let recents = s.prefs.start_files();
                                    if recents.is_empty() {
                                        ui.label(
                                            RichText::new(tr("No recent files"))
                                                .color(s.pal.text_dim),
                                        );
                                    } else {
                                        ui.vertical(|ui| {
                                            for p in recents.iter().take(3) {
                                                let name = std::path::Path::new(p)
                                                    .file_name()
                                                    .map(|n| n.to_string_lossy().into_owned())
                                                    .unwrap_or_else(|| p.clone());
                                                if ui.link(name).on_hover_text(p).clicked() {
                                                    s.open_path(ctx, std::path::Path::new(p));
                                                }
                                            }
                                        });
                                    }
                                });
                            }
                            GroupExtra::None => {
                                let cmds = g.cmds.clone();
                                self.group(ui, g.title, |s, ui| s.cmds(ui, ctx, &cmds));
                            }
                        }
                    }
                });
            });
    }

    // ---- document tabs (docs/DESIGN.md §4.4) -------------------------------------------------

    pub fn tab_bar(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        let m = theme::metrics(self.prefs.density);
        let pal = self.pal;
        let full = ui.max_rect();
        let row = Rect::from_min_size(full.min, Vec2::new(full.width(), m.doc_tabs_h));
        ui.allocate_rect(row, Sense::hover());
        let p = ui.painter().clone();
        let mut x = row.left() + 10.0;
        let top = row.bottom() - m.tab_h;
        let mut close: Option<usize> = None;
        let mut activate: Option<usize> = None;
        let start = self.show_start;
        let corners = egui::CornerRadius {
            nw: theme::radius::LG,
            ne: theme::radius::LG,
            sw: 0,
            se: 0,
        };
        let outline = |r: Rect| {
            let s = egui::Stroke::new(1.0, pal.border);
            p.rect_filled(r, corners, pal.card);
            p.vline(r.left() + 0.5, (r.top() + 6.0)..=r.bottom(), s);
            p.vline(r.right() - 0.5, (r.top() + 6.0)..=r.bottom(), s);
            p.hline((r.left() + 6.0)..=(r.right() - 6.0), r.top() + 0.5, s);
            p.add(egui::Shape::Path(egui::epaint::PathShape::line(
                arc_pts(egui::pos2(r.left() + 8.0, r.top() + 8.0), 7.5, 180.0, 270.0),
                s,
            )));
            p.add(egui::Shape::Path(egui::epaint::PathShape::line(
                arc_pts(
                    egui::pos2(r.right() - 8.0, r.top() + 8.0),
                    7.5,
                    270.0,
                    360.0,
                ),
                s,
            )));
        };
        let titles: Vec<(String, bool, String)> = self
            .tabs
            .iter()
            .map(|t| {
                (
                    t.session.title.clone(),
                    t.session.is_dirty(),
                    t.session
                        .path
                        .as_ref()
                        .map_or_else(|| t.session.title.clone(), |p| p.display().to_string()),
                )
            })
            .collect();
        for (i, (title, dirty, tip)) in titles.iter().enumerate() {
            let sel = i == self.active && !start;
            let font = if sel {
                theme::font(500, 13.0)
            } else {
                egui::FontId::proportional(13.0)
            };
            let col = if sel { pal.text } else { pal.text_dim };
            let g = p.layout_no_wrap(title.clone(), font, col);
            let name_w = g.size().x.min(200.0);
            let w = 12.0 + 15.0 + 8.0 + name_w + if *dirty { 14.0 } else { 0.0 } + 8.0 + 16.0 + 8.0;
            let r = Rect::from_min_size(egui::pos2(x, top), Vec2::new(w, m.tab_h));
            if r.left() > row.right() - 40.0 {
                break;
            }
            let resp = ui.interact(r, ui.id().with(("doctab", i)), Sense::click());
            if sel {
                outline(r);
            } else if resp.hovered() {
                p.rect_filled(r, corners, pal.hover);
            }
            let icon = Rect::from_center_size(
                egui::pos2(r.left() + 12.0 + 7.5, r.center().y),
                Vec2::splat(15.0),
            );
            icons::paint(
                &p,
                icon,
                Icon::Page,
                if sel { pal.accent } else { pal.text_dim },
            );
            let name_rect = Rect::from_min_max(
                egui::pos2(icon.right() + 8.0, r.top()),
                egui::pos2(icon.right() + 8.0 + name_w, r.bottom()),
            );
            p.with_clip_rect(name_rect).galley(
                egui::pos2(name_rect.left(), r.center().y - g.size().y / 2.0),
                g,
                col,
            );
            if *dirty {
                p.circle_filled(
                    egui::pos2(name_rect.right() + 8.0, r.center().y),
                    3.0,
                    pal.accent,
                );
            }
            let x_rect = Rect::from_center_size(
                egui::pos2(r.right() - 8.0 - 8.0, r.center().y),
                Vec2::splat(16.0),
            );
            let x_resp = ui.interact(x_rect, ui.id().with(("tabx", i)), Sense::click());
            if sel || resp.hovered() || x_resp.hovered() {
                if x_resp.hovered() {
                    p.rect_filled(x_rect, theme::radius::SM, pal.pressed);
                }
                icons::paint(&p, x_rect.shrink(1.5), Icon::Close, pal.text_muted);
            }
            x_resp.clone().on_hover_text(tr("Close tab"));
            if x_resp.clicked() || resp.middle_clicked() {
                close = Some(i);
            } else if resp.clicked() {
                activate = Some(i);
            }
            resp.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::Button, true, sel, title)
            });
            resp.on_hover_text(tip);
            x = r.right() + 4.0;
        }
        if start && !self.tabs.is_empty() {
            let g = p.layout_no_wrap(tr("Start").to_string(), theme::font(500, 13.0), pal.text);
            let r = Rect::from_min_size(
                egui::pos2(x, top),
                Vec2::new(12.0 + 15.0 + 8.0 + g.size().x + 12.0, m.tab_h),
            );
            outline(r);
            let icon = Rect::from_center_size(
                egui::pos2(r.left() + 12.0 + 7.5, r.center().y),
                Vec2::splat(15.0),
            );
            icons::paint(&p, icon, Icon::Open, pal.accent);
            p.galley(
                egui::pos2(icon.right() + 8.0, r.center().y - g.size().y / 2.0),
                g,
                pal.text,
            );
            x = r.right() + 4.0;
        }
        // "+": the start screen, in a tab of its own.
        let plus =
            Rect::from_center_size(egui::pos2(x + 14.0, top + m.tab_h / 2.0), Vec2::splat(28.0));
        let resp = ui.interact(plus, ui.id().with("doctab_plus"), Sense::click());
        if resp.hovered() {
            p.rect_filled(plus, theme::radius::MD, pal.hover);
        }
        let c = plus.center();
        let s = egui::Stroke::new(1.6, pal.text);
        p.line_segment([c - Vec2::new(5.5, 0.0), c + Vec2::new(5.5, 0.0)], s);
        p.line_segment([c - Vec2::new(0.0, 5.5), c + Vec2::new(0.0, 5.5)], s);
        if resp
            .on_hover_text(tr("Start screen: open or find a file"))
            .clicked()
        {
            self.show_start = true;
        }
        p.hline(
            row.x_range(),
            row.bottom() - 0.5,
            egui::Stroke::new(1.0, pal.border),
        );
        if let Some(i) = activate {
            self.active = i;
            self.show_start = false;
        }
        if let Some(i) = close {
            self.request_close(i);
        }
    }

    // ---- status bar (docs/DESIGN.md §4.12) ---------------------------------------------------

    pub fn status_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let pal = self.pal;
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            let hint = if self.tabs.is_empty() {
                tr("Open a PDF or image to begin").to_string()
            } else if self
                .active_tab()
                .is_some_and(|t| t.ui.inline_edit.is_some())
            {
                tr("Type to change the text. Esc stops editing.").to_string()
            } else if self.tool == Tool::FormDesign
                && let Some(h) = self.selected_field_hint()
            {
                h
            } else {
                tr(self.tool.hint()).to_string()
            };
            let small = |t: String, c: Color32| RichText::new(t).size(12.0).color(c);
            ui.add(
                egui::Label::new(small(hint, pal.text_dim))
                    .truncate()
                    .selectable(false),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                let Some(info) = self.status_info() else {
                    return;
                };
                // Zoom: slider 110 wide plus the value (a menu with presets and fit).
                let (z, mode, zmode) = (info.zoom, info.mode, info.zoom_mode);
                let label = match zmode {
                    ZoomMode::FitWidth | ZoomMode::FitPage | ZoomMode::Custom => {
                        format!("{:.0}%", z * 100.0)
                    }
                };
                let zb = egui::Button::new(RichText::new(label).size(12.0).color(pal.text))
                    .fill(Color32::TRANSPARENT)
                    .stroke(egui::Stroke::NONE)
                    .min_size(Vec2::new(36.0, 20.0));
                egui::containers::menu::MenuButton::from_button(zb).ui(ui, |ui| {
                    for pz in [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 4.0, 8.0] {
                        if ui.button(format!("{:.0}%", pz * 100.0)).clicked() {
                            self.request_zoom(pz, None);
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
                if let Some(nz) = zoom_slider(ui, z, &pal) {
                    self.request_zoom(nz, None);
                }
                // Page layout: single page, continuous, two pages.
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (vm, icon, cmd) in [
                        (ViewMode::Facing, Icon::Spread, C::ViewModeFacing),
                        (ViewMode::Continuous, Icon::Scroll, C::ViewModeContinuous),
                        (ViewMode::SinglePage, Icon::Single, C::ViewModeSingle),
                    ] {
                        let (r, resp) =
                            ui.allocate_exact_size(Vec2::new(24.0, 22.0), Sense::click());
                        let on = mode == vm;
                        if on {
                            ui.painter()
                                .rect_filled(r, theme::radius::SM, pal.accent_soft);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(r, theme::radius::SM, pal.hover);
                        }
                        icons::paint(
                            ui.painter(),
                            Rect::from_center_size(r.center(), Vec2::splat(14.0)),
                            icon,
                            if on { pal.accent } else { pal.text_dim },
                        );
                        let tip = tr(editor_core::command::info(cmd).title);
                        resp.widget_info(|| {
                            egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, tip)
                        });
                        if resp.on_hover_text(tip).clicked() {
                            self.run_command(ctx, cmd);
                        }
                    }
                });
                if let Some((w, h)) = info.page_size {
                    ui.label(small(format_size(w, h), pal.text_dim));
                }
                // Page [n] of N.
                ui.label(small(tf!("of {}", info.page_count), pal.text_dim));
                let mut text = self
                    .page_input
                    .clone()
                    .unwrap_or_else(|| (info.current + 1).to_string());
                let te = egui::TextEdit::singleline(&mut text)
                    .desired_width(28.0)
                    .horizontal_align(egui::Align::Center)
                    .font(egui::FontId::proportional(12.0))
                    .margin(egui::Margin::symmetric(2, 1))
                    .background_color(pal.field);
                let r = ui
                    .add_sized([28.0, 20.0], te)
                    .on_hover_text(tr("Go to page…"));
                if r.has_focus() {
                    self.page_input = Some(text.clone());
                }
                if r.lost_focus() {
                    if ui.input(|i| i.key_pressed(egui::Key::Enter))
                        && let Ok(n) = text.trim().parse::<usize>()
                        && n >= 1
                    {
                        self.go_to(n - 1);
                    }
                    self.page_input = None;
                }
                ui.label(small(tr("Page").to_string(), pal.text_dim));
                if info.rotated != 0 {
                    ui.label(small(
                        tf!("view rotated {}°", info.rotated),
                        pal.accent_text,
                    ));
                }
                if let Some((txt, ok)) = self.status_scale() {
                    ui.label(small(txt, if ok { pal.text_dim } else { pal.danger }));
                }
                if self
                    .active_tab()
                    .is_some_and(|t| t.session.doc().capabilities().has_signatures)
                    && ui
                        .add(
                            egui::Label::new(small(tr("Signed").into(), pal.accent_text))
                                .sense(Sense::click()),
                        )
                        .on_hover_text(tr(
                            "This document contains digital signatures. Click to check them.",
                        ))
                        .clicked()
                {
                    self.open_signatures();
                }
                if let Some(r) = self.update.available.clone()
                    && ui
                        .add(
                            egui::Label::new(small(
                                tf!("Update available: {}", r.version),
                                pal.accent_text,
                            ))
                            .sense(Sense::click()),
                        )
                        .on_hover_text(tr("Open the release page"))
                        .clicked()
                {
                    self.open_update_page();
                }
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
                    ui.label(small(tf!("{} rendering", queued), pal.text_muted));
                    ui.add(egui::Spinner::new().size(12.0));
                }
            });
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

/// Short ribbon captions for commands whose full title is too long for a ribbon button.
/// (The palette, tooltips and menus keep the full title.)
fn ribbon_label(id: C, title: &'static str) -> &'static str {
    match id {
        C::PageRotateCounterClockwise => tr("Rotate left"),
        C::PageRotateClockwise => tr("Rotate right"),
        C::ViewRotateCounterClockwise => tr("Rotate view left"),
        C::ViewRotateClockwise => tr("Rotate view right"),
        C::PageInsertBlank => tr("Blank page"),
        C::PageInsertImage => tr("Image page"),
        C::PageDuplicate => tr("Duplicate"),
        C::PageDelete => tr("Delete"),
        C::PageMoveUp => tr("Move up"),
        C::PageMoveDown => tr("Move down"),
        C::PageExtract => tr("Extract"),
        C::DocumentMerge => tr("Merge"),
        C::DocumentCompare => tr("Compare"),
        C::PageHeaderFooter => tr("Header & footer"),
        C::PageBates => tr("Bates"),
        C::PageWatermark => tr("Watermark"),
        C::PageRemoveMarks => tr("Remove marks"),
        C::PageLabels => tr("Page labels"),
        C::PageCrop => tr("Crop pages"),
        C::ToolCrop => tr("Crop tool"),
        C::BookmarkAdd => tr("Add bookmark"),
        C::ToolFormDesign => tr("Edit fields"),
        C::FormScripts => tr("Scripts"),
        C::FormCalculateNow => tr("Calculate"),
        C::DocumentJavaScript => tr("Document JS"),
        C::ToolMeasurePerimeter => tr("Path length"),
        C::ToolMeasureDistance => tr("Distance"),
        C::ToolMeasureRect => tr("Rectangle"),
        C::ExportMeasurements => tr("Export CSV"),
        C::OcrDocument => "OCR",
        C::ToggleSnap => tr("Snap"),
        C::FileSaveOptimized => tr("Optimize"),
        C::FileConvertPdfA => "PDF/A",
        C::FileProtection => tr("Protect"),
        C::FilePrint => tr("Print"),
        C::ToolRedact => tr("Mark area"),
        C::RedactMarkSelection => tr("Mark text"),
        C::RedactApply => tr("Apply"),
        C::ToggleCopilot => tr("Copilot"),
        C::CopilotSummarize => tr("Summarize"),
        C::CopilotSummarizeAnnotations => tr("Summarize notes"),
        C::TranslateDocument => tr("Translate"),
        C::SignDocument => tr("Sign"),
        C::ShowSignatures => tr("Signatures"),
        C::DrawSignature => tr("Draw signature"),
        C::SignFromImage => tr("Signature from picture"),
        C::NewStamp => tr("New stamp"),
        C::ToolPlaceSignature => tr("Place signature"),
        C::ToolFillForm => tr("Fill form"),
        C::FormFlatten => tr("Flatten"),
        C::FileExportImage => tr("Export image"),
        C::FileProperties => tr("Properties"),
        C::ToolHighlight => tr("Highlight"),
        C::ToolUnderline => tr("Underline"),
        C::ToolStrikeOut => tr("Strikeout"),
        C::ToolCalibrate => tr("Calibrate"),
        C::ViewZoomActual => "100 %",
        C::ViewDarkPages => tr("Dark pages"),
        C::ViewToggleLeftSidebar => tr("Navigation"),
        C::ViewToggleRightSidebar => tr("Properties"),
        C::ShortcutReference => tr("Shortcuts"),
        _ => tr(title),
    }
}

/// One group of ribbon commands.
pub struct RibbonGroup {
    /// Caption in the classic ribbon and the "More" menu.
    pub title: &'static str,
    pub cmds: Vec<C>,
    pub extra: GroupExtra,
}

/// What a ribbon group holds besides commands.
pub enum GroupExtra {
    None,
    /// The recent files.
    Recent,
    /// A short explanation (an info button with a tooltip in the single-line ribbon).
    Note(String),
}

/// Points on a circular arc; angles in degrees with 0 = right and 90 = down (screen coordinates).
pub fn arc_pts(c: egui::Pos2, r: f32, from: f32, to: f32) -> Vec<egui::Pos2> {
    (0..=8)
        .map(|k| {
            let a = (from + (to - from) * k as f32 / 8.0).to_radians();
            egui::pos2(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect()
}

/// The status-bar zoom slider (110 wide, logarithmic from 10 % to 800 %). Returns a new zoom when moved.
fn zoom_slider(ui: &mut egui::Ui, zoom: f64, pal: &theme::Palette) -> Option<f64> {
    let (lo, hi) = (0.1_f64, 8.0_f64);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(110.0, 20.0), Sense::click_and_drag());
    let t = ((zoom.clamp(lo, hi) / lo).ln() / (hi / lo).ln()) as f32;
    let y = rect.center().y;
    let x0 = rect.left() + 7.0;
    let x1 = rect.right() - 7.0;
    let x = egui::lerp(x0..=x1, t);
    let p = ui.painter();
    p.rect_filled(
        Rect::from_min_max(egui::pos2(x0, y - 2.0), egui::pos2(x1, y + 2.0)),
        2.0,
        pal.border,
    );
    p.rect_filled(
        Rect::from_min_max(egui::pos2(x0, y - 2.0), egui::pos2(x, y + 2.0)),
        2.0,
        pal.accent,
    );
    p.circle_filled(egui::pos2(x, y), 7.0, Color32::WHITE);
    p.circle_stroke(egui::pos2(x, y), 7.0, egui::Stroke::new(1.0, pal.accent));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, tr("Zoom")));
    let resp = resp.on_hover_text(tr("Zoom"));
    if let Some(pos) = resp.interact_pointer_pos()
        && (resp.dragged() || resp.clicked())
    {
        let nt = ((pos.x - x0) / (x1 - x0)).clamp(0.0, 1.0);
        let nz = lo * (hi / lo).powf(f64::from(nt));
        if (nz - zoom).abs() / zoom > 0.005 {
            return Some(nz);
        }
    }
    None
}
