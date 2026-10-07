//! Command execution: every user-visible action funnels through [`App::run_command`].

use crate::app::OS;
use crate::i18n::tr;
use crate::state::*;
use crate::tf;
use editor_core::command::{self, CommandId};
use editor_core::session::DocumentSession;
use editor_core::tools::{Tool, ToolFamily};
use editor_core::view::{self, ViewMode, ZoomMode};
use pdf_engine::annot;
use pdf_engine::doc::PageId;
use pdf_engine::geom::Rotation;
use pdf_engine::pageops::{self, FormPolicy};
use std::path::{Path, PathBuf};

impl App {
    /// Commands that are implemented; unfinished ones are hidden everywhere (ribbon, palette).
    pub fn implemented(_id: CommandId) -> bool {
        true
    }

    /// Whether a command can run right now (drives enabled state and the palette).
    pub fn command_enabled(&self, id: CommandId) -> bool {
        use CommandId as C;
        if !Self::implemented(id) {
            return false;
        }
        let tab = self.active_tab();
        let has = tab.is_some();
        let can_edit = tab.is_some_and(|t| t.session.doc().capabilities().can_edit);
        match id {
            C::FileOpen
            | C::FileQuit
            | C::CommandPalette
            | C::Preferences
            | C::ShortcutReference
            | C::About
            | C::NextTab
            | C::PreviousTab => true,
            C::Escape => true,
            // The Microsoft Store looks after updates of its own packages.
            C::CheckForUpdates => !platform::distribution::is_store(),
            C::FileSave => tab.is_some_and(|t| t.session.is_dirty()),
            C::FileSaveAs
            | C::FileClose
            | C::Find
            | C::FindNext
            | C::FindPrevious
            | C::GoToPage => has,
            C::EditUndo => tab.is_some_and(|t| t.session.can_undo()),
            C::EditRedo => tab.is_some_and(|t| t.session.can_redo()),
            C::EditCopy => tab.is_some_and(|t| t.session.selection.text.is_some()),
            C::EditSelectAll => has,
            C::EditDelete => {
                tab.is_some_and(|t| {
                    !t.session.selection.annotations.is_empty()
                        || t.session.selection.content.is_some()
                }) && can_edit
            }
            C::EditDuplicate => {
                tab.is_some_and(|t| !t.session.selection.annotations.is_empty()) && can_edit
            }
            id if Tool::from_command(id).is_some() => {
                let t = Tool::from_command(id).unwrap_or(Tool::Select);
                has && (t.family() == ToolFamily::Navigate || can_edit)
            }
            C::PageRotateClockwise
            | C::PageRotateCounterClockwise
            | C::PageDelete
            | C::PageInsertBlank
            | C::PageInsertImage
            | C::PageDuplicate
            | C::PageMoveUp
            | C::PageMoveDown
            | C::DocumentMerge => has && can_edit,
            C::PageExtract => has,
            C::FileProperties
            | C::FileExportImage
            | C::ShowSignatures
            | C::FileSaveOptimized
            | C::FileConvertPdfA => has,
            C::ToggleCopilot => true,
            C::CopilotSummarize | C::CopilotSummarizeAnnotations => has && !self.ai_busy(),
            C::TranslateDocument => has,
            C::SignDocument => has && can_edit,
            C::OcrDocument => has && can_edit && self.ocr_job.is_none(),
            C::DrawSignature => true,
            C::ToolSignArea => self.sign_return.is_some(),
            C::FormFlatten => {
                has && can_edit
                    && tab.is_some_and(|t| {
                        t.ui.form
                            .as_ref()
                            .is_some_and(|(_, f)| !f.fields.is_empty())
                    })
            }
            C::ExportMeasurements => has,
            _ => has,
        }
    }

    pub fn tooltip_for(&self, id: CommandId) -> String {
        let info = command::info(id);
        match self.prefs.keybindings.primary_label(id, OS) {
            Some(s) => format!("{} ({})\n{}", tr(info.title), s, tr(info.description)),
            None => format!("{}\n{}", tr(info.title), tr(info.description)),
        }
    }

    pub fn set_tool(&mut self, t: Tool) {
        self.tool = t;
        self.prefs.push_recent_tool(t.command());
        self.prefs_dirty = true;
        if let Some(tab) = self.active_tab_mut() {
            tab.ui.interaction = Interaction::None;
            tab.ui.polygon_points.clear();
            if !matches!(t, Tool::Select) {
                tab.session.selection.annotations.clear();
            }
        }
    }

    pub fn selected_pages(&self) -> Vec<PageId> {
        let Some(tab) = self.active_tab() else {
            return Vec::new();
        };
        if !tab.session.selection.pages.is_empty() {
            return tab.session.selection.pages.clone();
        }
        let Ok(ids) = tab.session.doc().page_ids() else {
            return Vec::new();
        };
        ids.get(tab.session.view.current_page)
            .copied()
            .into_iter()
            .collect()
    }

    pub fn run_command(&mut self, ctx: &egui::Context, id: CommandId) {
        use CommandId as C;
        if !self.command_enabled(id) {
            return;
        }
        if let Some(t) = Tool::from_command(id) {
            // Page-content tools are not implemented yet (hidden by `implemented`).
            self.set_tool(t);
            return;
        }
        match id {
            C::FileOpen => self.pick_and_open(ctx),
            C::FileSave => self.save_active(),
            C::FileSaveAs => self.save_active_as(),
            C::FileProperties => self.open_properties(),
            C::SignDocument => self.open_sign_dialog(),
            C::OcrDocument => self.open_ocr_dialog(),
            C::FileSaveOptimized => self.open_optimize_dialog(),
            C::FileConvertPdfA => self.open_pdfa_dialog(),
            C::ShowSignatures => self.open_signatures(),
            C::DrawSignature => self.open_draw_signature(),
            C::FileExportImage => self.export_page_image(),
            C::FormFlatten => self.request_flatten(),
            C::FileClose => self.request_close(self.active),
            C::FileQuit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            C::EditUndo => self.undo_redo(true),
            C::EditRedo => self.undo_redo(false),
            C::EditCopy => self.copy_selection(ctx),
            C::EditSelectAll => self.select_all_text(),
            C::EditDelete => {
                if self
                    .active_tab()
                    .is_some_and(|t| t.session.selection.content.is_some())
                {
                    self.delete_selected_content();
                } else {
                    self.delete_selected_annotations();
                }
            }
            C::EditDuplicate => self.duplicate_selected_annotations(),
            C::Find => {
                self.left_tab = LeftTab::Search;
                self.prefs.show_left_sidebar = true;
                self.search_focus = true;
            }
            C::FindNext => self.step_search(true),
            C::FindPrevious => self.step_search(false),
            C::CommandPalette => {
                self.palette_open = true;
                self.palette_query.clear();
                self.palette_sel = 0;
            }
            C::Preferences => {
                self.dialog = Some(Dialog::Preferences {
                    filter: String::new(),
                })
            }
            C::ShortcutReference => {
                self.dialog = Some(Dialog::Shortcuts {
                    filter: String::new(),
                    capture: None,
                })
            }
            C::About => self.dialog = Some(Dialog::About),
            C::CheckForUpdates => self.start_update_check(true),
            C::ViewZoomIn => self.zoom_step(true),
            C::ViewZoomOut => self.zoom_step(false),
            C::ViewZoomActual => self.request_zoom(1.0, None),
            C::ViewFitPage => self.set_zoom_mode(ZoomMode::FitPage),
            C::ViewFitWidth => self.set_zoom_mode(ZoomMode::FitWidth),
            C::ViewModeContinuous => self.set_view_mode(ViewMode::Continuous),
            C::ViewModeSingle => self.set_view_mode(ViewMode::SinglePage),
            C::ViewModeFacing => self.set_view_mode(ViewMode::Facing),
            C::ViewRotateClockwise => self.rotate_view(1),
            C::ViewRotateCounterClockwise => self.rotate_view(-1),
            C::ViewToggleLeftSidebar => {
                self.prefs.show_left_sidebar = !self.prefs.show_left_sidebar;
                self.prefs_dirty = true;
            }
            C::ViewToggleRightSidebar => {
                self.prefs.show_right_sidebar = !self.prefs.show_right_sidebar;
                self.prefs_dirty = true;
            }
            C::ViewDarkPages => {
                self.prefs.dark_page_filter = !self.prefs.dark_page_filter;
                self.restyle(ctx);
            }
            C::ToggleCopilot => self.toggle_copilot(),
            C::CopilotSummarize => self.copilot_summarize(ctx),
            C::CopilotSummarizeAnnotations => self.copilot_summarize_annotations(ctx),
            C::TranslateDocument => {
                if self.ai_config().is_err() {
                    self.dialog = Some(Dialog::Preferences {
                        filter: "ai".into(),
                    });
                    self.notify(tr("Add an AI provider key first."));
                } else if self.ai_consent_or_ask(crate::copilot_ui::AiPending::OpenTranslate) {
                    self.open_translate_dialog(ctx);
                }
            }
            C::ToggleSnap => {
                self.prefs.snap_to_geometry = !self.prefs.snap_to_geometry;
                self.prefs_dirty = true;
                let on = self.prefs.snap_to_geometry;
                self.notify(if on {
                    tr("Snap to drawing geometry on")
                } else {
                    tr("Snap to drawing geometry off")
                });
            }
            C::GoNextPage => self.go_relative(1),
            C::GoPreviousPage => self.go_relative(-1),
            C::GoFirstPage => self.go_to(0),
            C::GoLastPage => self.go_to(usize::MAX),
            C::GoToPage => {
                self.dialog = Some(Dialog::GoToPage {
                    text: String::new(),
                })
            }
            C::PageRotateClockwise => self.rotate_pages(1),
            C::PageRotateCounterClockwise => self.rotate_pages(-1),
            C::PageDelete => {
                let n = self.selected_pages().len();
                if n > 0 {
                    self.dialog = Some(Dialog::ConfirmDeletePages { count: n });
                }
            }
            C::PageInsertBlank => self.insert_blank(),
            C::ExportMeasurements => self.export_measurements(),
            C::PageInsertImage => self.insert_image_pages(),
            C::PageDuplicate => self.duplicate_pages(),
            C::PageExtract => self.extract_pages(ctx),
            C::PageMoveUp => self.move_pages_by(-1),
            C::PageMoveDown => self.move_pages_by(1),
            C::DocumentMerge => self.merge_documents(),
            C::NextTab => {
                if !self.tabs.is_empty() {
                    self.active = (self.active + 1) % self.tabs.len();
                }
            }
            C::PreviousTab => {
                if !self.tabs.is_empty() {
                    self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
                }
            }
            C::Escape => self.escape(),
            _ => {}
        }
    }

    fn escape(&mut self) {
        if self.palette_open {
            self.palette_open = false;
            return;
        }
        // Esc first cancels whatever is in progress, and always leaves the current tool for the
        // Select tool; only when already in Select does it clear the page-content selection.
        let mut in_progress = false;
        if let Some(tab) = self.active_tab_mut() {
            in_progress = !matches!(tab.ui.interaction, Interaction::None)
                || !tab.ui.polygon_points.is_empty()
                || tab.ui.pending_region.is_some();
            if in_progress {
                tab.ui.interaction = Interaction::None;
                tab.ui.polygon_points.clear();
                tab.ui.pending_region = None;
            }
        }
        if self.sign_return.is_some() {
            self.finish_sign_area(None);
            return;
        }
        if self.tool != Tool::Select {
            self.set_tool(Tool::Select);
        } else if !in_progress && let Some(tab) = self.active_tab_mut() {
            tab.session.selection.clear_content();
        }
    }

    // ---- files -------------------------------------------------------------------------

    pub fn pick_and_open(&mut self, ctx: &egui::Context) {
        let start = self
            .active_tab()
            .and_then(|t| t.session.path.as_ref())
            .and_then(|p| p.parent().map(Path::to_path_buf));
        for p in platform::dialogs::pick_open_documents(start.as_deref()) {
            self.open_path(ctx, &p);
        }
    }

    pub fn open_path(&mut self, ctx: &egui::Context, path: &Path) {
        if let Some(i) = self
            .tabs
            .iter()
            .position(|t| t.session.path.as_deref() == Some(path))
        {
            self.active = i;
            return;
        }
        if platform::dialogs::is_image_path(path) {
            self.open_image(ctx, path);
            return;
        }
        match DocumentSession::open_path(path) {
            Ok(session) => {
                let caps = session.doc().capabilities().clone();
                self.prefs.push_recent(&path.to_string_lossy());
                self.prefs_dirty = true;
                self.add_tab(session);
                let mut lines: Vec<String> = Vec::new();
                for b in &caps.edit_blockers {
                    lines.push(b.0.clone());
                }
                for w in &caps.warnings {
                    lines.push(w.0.clone());
                }
                if caps.has_javascript {
                    lines.push(
                        tr("This document contains JavaScript. BergPDF never runs document scripts.")
                            .into(),
                    );
                }
                if !lines.is_empty() {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    self.dialog = Some(Dialog::OpenWarnings { title: name, lines });
                }
                ctx.request_repaint();
            }
            Err(e) => {
                self.dialog = Some(Dialog::Error {
                    title: tr("Cannot open document").into(),
                    detail: format!("{}\n\n{e}", path.display()),
                });
            }
        }
    }

    pub fn add_tab(&mut self, session: DocumentSession) {
        let mut session = session;
        match self.prefs.default_zoom {
            editor_core::prefs::DefaultZoom::FitPage => session.view.zoom_mode = ZoomMode::FitPage,
            editor_core::prefs::DefaultZoom::FitWidth => {
                session.view.zoom_mode = ZoomMode::FitWidth
            }
            editor_core::prefs::DefaultZoom::Actual => {
                session.view.zoom_mode = ZoomMode::Custom;
                session.view.zoom = 1.0;
            }
        }
        // Always open at the top, centred: scroll origin is (0,0) and page 0 is brought into view.
        session.view.scroll = pdf_engine::geom::Vec2::ZERO;
        session.view.current_page = 0;
        let ui_state = TabState {
            goto: Some(0),
            ..TabState::default()
        };
        self.tabs.push(Tab {
            session,
            ui: ui_state,
        });
        self.active = self.tabs.len() - 1;
    }

    pub fn save_active(&mut self) {
        let Some(tab) = self.active_tab() else { return };
        if tab.session.path.is_none() {
            self.save_active_as();
            return;
        }
        let i = self.active;
        self.save_tab(i);
    }

    fn save_tab(&mut self, i: usize) -> bool {
        let Some(tab) = self.tabs.get_mut(i) else {
            return false;
        };
        match tab.session.save() {
            Ok(()) => {
                let name = tab.session.title.clone();
                self.notify(tf!("Saved {}", name));
                true
            }
            Err(e) => {
                self.dialog = Some(Dialog::Error {
                    title: tr("Could not save").into(),
                    detail: tf!(
                        "{}\n\nYour original file was not modified. Your changes are still open; use Save As to write a copy elsewhere.",
                        e
                    ),
                });
                false
            }
        }
    }

    pub fn save_active_as(&mut self) {
        let Some(tab) = self.active_tab() else { return };
        let suggested = tab.session.title.clone();
        let start = tab
            .session
            .path
            .as_ref()
            .and_then(|p| p.parent().map(Path::to_path_buf));
        let Some(dest) = platform::dialogs::pick_save_pdf(&suggested, start.as_deref()) else {
            return;
        };
        let i = self.active;
        if let Some(tab) = self.tabs.get_mut(i) {
            match tab.session.save_as(&dest) {
                Ok(()) => {
                    self.prefs.push_recent(&dest.to_string_lossy());
                    self.prefs_dirty = true;
                    self.notify(tf!("Saved {}", dest.display()));
                }
                Err(e) => {
                    self.dialog = Some(Dialog::Error {
                        title: tr("Could not save").into(),
                        detail: tf!("{}\n\nThe destination was not modified.", e),
                    });
                }
            }
        }
    }

    pub fn request_close(&mut self, i: usize) {
        let Some(tab) = self.tabs.get(i) else { return };
        if tab.session.is_dirty() {
            self.dialog = Some(Dialog::ConfirmClose { tab: i });
        } else {
            self.close_tab(i);
        }
    }

    pub fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        let t = self.tabs.remove(i);
        self.hub.close(t.session.id);
        platform::recovery::remove(
            &platform::dirs::data_dir().join("recovery"),
            &crate::app::recovery_token(t.session.id.0),
        );
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        } else if i < self.active {
            self.active -= 1;
        }
    }

    /// Resolve a ConfirmClose dialog: `save` = Some(true) saves first.
    pub fn resolve_close(&mut self, i: usize, save: bool) {
        if save && !self.save_tab_with_dialog(i) {
            return;
        }
        self.close_tab(i);
    }

    fn save_tab_with_dialog(&mut self, i: usize) -> bool {
        if self.tabs.get(i).is_some_and(|t| t.session.path.is_none()) {
            let old = self.active;
            self.active = i;
            self.save_active_as();
            self.active = old;
            return self.tabs.get(i).is_some_and(|t| !t.session.is_dirty());
        }
        self.save_tab(i)
    }

    // ---- editing -----------------------------------------------------------------------

    pub fn undo_redo(&mut self, undo: bool) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let label = if undo {
            tab.session.undo()
        } else {
            tab.session.redo()
        };
        tab.session.selection.clear_content();
        tab.ui.interaction = Interaction::None;
        tab.ui.edit_draft = None;
        if let Some(l) = label {
            self.notify(format!(
                "{} {}",
                if undo { tr("Undid") } else { tr("Redid") },
                l.to_lowercase()
            ));
        }
    }

    fn copy_selection(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.active_tab() else { return };
        let Some(sel) = &tab.session.selection.text else {
            return;
        };
        if let Some((_, tp)) = self.text.get(&(tab.session.id, sel.page)) {
            let s = tp.text_of(sel.glyphs.clone());
            ctx.copy_text(s);
        }
    }

    fn select_all_text(&mut self) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let Ok(ids) = tab.session.doc().page_ids() else {
            return;
        };
        let Some(page) = ids.get(tab.session.view.current_page).copied() else {
            return;
        };
        let id = tab.session.id;
        if let Some((_, tp)) = self.text.get(&(id, page)) {
            let n = tp.glyphs.len();
            if let Some(tab) = self.tabs.get_mut(self.active) {
                tab.session.selection.annotations.clear();
                tab.session.selection.text =
                    Some(editor_core::selection::TextSelection { page, glyphs: 0..n });
            }
        }
    }

    pub fn delete_selected_annotations(&mut self) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let sel = tab.session.selection.annotations.clone();
        if sel.is_empty() {
            return;
        }
        let r = tab.session.execute(
            if sel.len() == 1 {
                tr("Delete annotation")
            } else {
                tr("Delete annotations")
            },
            |tx| {
                for (page, id) in &sel {
                    annot::delete_annotation(tx, *page, *id)?;
                }
                Ok(())
            },
        );
        tab.session.selection.annotations.clear();
        if let Err(e) = r {
            self.notify_error(e.to_string());
        }
    }

    fn duplicate_selected_annotations(&mut self) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let sel = tab.session.selection.annotations.clone();
        let doc = tab.session.doc();
        let specs: Vec<_> = sel
            .iter()
            .filter_map(|(p, id)| {
                annot::read_annotation(doc.lopdf(), *id)
                    .and_then(|a| a.spec)
                    .map(|s| (*p, s))
            })
            .collect();
        if specs.is_empty() {
            return;
        }
        let r = tab.session.execute(tr("Duplicate annotations"), |tx| {
            let mut out = Vec::new();
            for (page, spec) in &specs {
                out.push((
                    *page,
                    annot::add_annotation(tx, *page, &spec.translated(12.0, -12.0))?,
                ));
            }
            Ok(out)
        });
        match r {
            Ok(new) => tab.session.selection.annotations = new,
            Err(e) => self.notify_error(e.to_string()),
        }
    }

    // ---- view --------------------------------------------------------------------------

    pub fn request_zoom(&mut self, zoom: f64, anchor: Option<egui::Vec2>) {
        if let Some(tab) = self.active_tab_mut() {
            tab.session.view.zoom_mode = ZoomMode::Custom;
            tab.ui.zoom_request = Some((zoom.clamp(view::MIN_ZOOM, view::MAX_ZOOM), anchor));
        }
    }

    fn zoom_step(&mut self, up: bool) {
        let Some(tab) = self.active_tab() else { return };
        let z = tab.session.view.zoom;
        self.request_zoom(
            if up {
                view::zoom_in_step(z)
            } else {
                view::zoom_out_step(z)
            },
            None,
        );
    }

    fn set_zoom_mode(&mut self, m: ZoomMode) {
        if let Some(tab) = self.active_tab_mut() {
            tab.session.view.zoom_mode = m;
        }
    }

    fn set_view_mode(&mut self, m: ViewMode) {
        if let Some(tab) = self.active_tab_mut() {
            tab.session.view.mode = m;
            tab.ui.goto = Some(tab.session.view.current_page);
        }
    }

    fn rotate_view(&mut self, quarter_turns: i64) {
        if let Some(tab) = self.active_tab_mut() {
            tab.session.view.rotation = tab.session.view.rotation.rotated_by(quarter_turns);
            tab.ui.goto = Some(tab.session.view.current_page);
        }
    }

    pub fn go_to(&mut self, page: usize) {
        if let Some(tab) = self.active_tab_mut() {
            let n = tab.session.doc().page_count();
            if n > 0 {
                tab.ui.goto = Some(page.min(n - 1));
            }
        }
    }

    fn go_relative(&mut self, d: i64) {
        let Some(tab) = self.active_tab() else { return };
        let cur = tab.session.view.current_page as i64;
        let step = if tab.session.view.mode == ViewMode::Facing {
            2 * d
        } else {
            d
        };
        self.go_to((cur + step).max(0) as usize);
    }

    // ---- search ------------------------------------------------------------------------

    pub fn start_search(&mut self) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let q = tab.session.search.query.trim().to_string();
        tab.session.search.clear_results();
        tab.session.search.request += 1;
        if q.is_empty() {
            return;
        }
        let Ok(pages) = tab.session.pages() else {
            return;
        };
        let Ok(snap) = tab.session.snapshot() else {
            return;
        };
        let case = tab.session.search.case_sensitive;
        let request = tab.session.search.request;
        tab.session.search.running = true;
        tab.session.search.progress = (0, pages.len());
        let doc = tab.session.id;
        let revision = snap.revision;
        self.hub.ensure(&snap);
        self.hub.submit(editor_core::jobs::Job {
            doc,
            revision,
            request,
            priority: 20,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            kind: editor_core::jobs::JobKind::Search {
                query: q,
                case_sensitive: case,
                pages: pages
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (p.id, i, p.geometry))
                    .collect(),
            },
        });
    }

    fn step_search(&mut self, next: bool) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        if tab.session.search.matches.is_empty() {
            return;
        }
        if next {
            tab.session.search.next();
        } else {
            tab.session.search.previous();
        }
        if let Some(c) = tab.session.search.current {
            tab.ui.goto = Some(tab.session.search.matches[c].page_index);
        }
    }

    // ---- pages -------------------------------------------------------------------------

    fn rotate_pages(&mut self, turns: i64) {
        let pages = self.selected_pages();
        if pages.is_empty() {
            return;
        }
        let label = if turns > 0 {
            tr("Rotate pages clockwise")
        } else {
            tr("Rotate pages counter-clockwise")
        };
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        if let Err(e) = tab
            .session
            .execute(label, |tx| pageops::rotate_pages(tx, &pages, turns))
        {
            self.notify_error(e.to_string());
        }
    }

    pub fn delete_pages_confirmed(&mut self) {
        let pages = self.selected_pages();
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let r = tab
            .session
            .execute(tr("Delete pages"), |tx| pageops::delete_pages(tx, &pages));
        tab.session.selection.pages.clear();
        let n = tab.session.doc().page_count();
        tab.session.view.current_page = tab.session.view.current_page.min(n.saturating_sub(1));
        if let Err(e) = r {
            self.notify_error(e.to_string());
        }
    }

    fn insert_blank(&mut self) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let Ok(pages) = tab.session.pages() else {
            return;
        };
        let cur = tab
            .session
            .view
            .current_page
            .min(pages.len().saturating_sub(1));
        let size = pages
            .get(cur)
            .map(|p| p.geometry.view_size(Rotation::R0))
            .unwrap_or(pdf_engine::geom::Size::new(595.0, 842.0));
        let r = tab.session.execute(tr("Insert blank page"), |tx| {
            pageops::insert_blank_page(tx, cur + 1, size.width, size.height)
        });
        match r {
            Ok(id) => {
                tab.session.selection.pages = vec![id];
                tab.ui.goto = Some(cur + 1);
            }
            Err(e) => self.notify_error(e.to_string()),
        }
    }

    /// Whether any of `pages` carries form widgets.
    fn pages_have_widgets(&self, pages: &[PageId]) -> bool {
        self.active_tab().is_some_and(|t| {
            pages
                .iter()
                .any(|p| !pdf_engine::forms::page_widgets(t.session.doc().lopdf(), p.0).is_empty())
        })
    }

    fn duplicate_pages(&mut self) {
        self.duplicate_pages_with(None);
    }

    fn duplicate_pages_with(&mut self, policy: Option<FormPolicy>) {
        let pages = self.selected_pages();
        if policy.is_none() && self.pages_have_widgets(&pages) {
            self.dialog = Some(Dialog::FormPolicy(FormPolicyState {
                op: FormOp::Duplicate,
                policy: FormPolicy::Independent,
            }));
            return;
        }
        let policy = policy.unwrap_or(FormPolicy::Independent);
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let r = tab.session.execute(tr("Duplicate pages"), |tx| {
            pageops::duplicate_pages(tx, &pages, policy)
        });
        match r {
            Ok(new) => tab.session.selection.pages = new,
            Err(e) => self.notify_error(e.to_string()),
        }
    }

    fn move_pages_by(&mut self, d: i64) {
        let pages = self.selected_pages();
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let Ok(all) = tab.session.doc().page_ids() else {
            return;
        };
        let Some(first) = all.iter().position(|p| pages.contains(p)) else {
            return;
        };
        let target = (first as i64 + d).max(0) as usize;
        if let Err(e) = tab.session.execute(tr("Move pages"), |tx| {
            pageops::move_pages(tx, &pages, target)
        }) {
            self.notify_error(e.to_string());
        }
    }

    fn extract_pages(&mut self, ctx: &egui::Context) {
        self.extract_pages_with(ctx, None);
    }

    fn extract_pages_with(&mut self, ctx: &egui::Context, policy: Option<FormPolicy>) {
        let pages = self.selected_pages();
        if pages.is_empty() {
            return;
        }
        if policy.is_none() && self.pages_have_widgets(&pages) {
            self.dialog = Some(Dialog::FormPolicy(FormPolicyState {
                op: FormOp::Extract,
                policy: FormPolicy::Independent,
            }));
            return;
        }
        let policy = policy.unwrap_or(FormPolicy::Independent);
        let Some(tab) = self.active_tab() else { return };
        let suggested = format!("{}-extract.pdf", tab.session.title.trim_end_matches(".pdf"));
        let start = tab
            .session
            .path
            .as_ref()
            .and_then(|p| p.parent().map(Path::to_path_buf));
        let Some(dest) = platform::dialogs::pick_save_pdf(&suggested, start.as_deref()) else {
            return;
        };
        let result = (|| -> Result<(), pdf_engine::EngineError> {
            let tab =
                self.tabs
                    .get(self.active)
                    .ok_or(pdf_engine::EngineError::InvalidArgument(
                        tr("no document").into(),
                    ))?;
            let mut new_doc = pdf_engine::doc::PdfDocument::new_empty()?;
            new_doc.transact(|tx| {
                pageops::import_pages(tx, tab.session.doc(), &pages, 0, policy).map(|_| ())
            })?;
            let mut s = DocumentSession::from_new_document(new_doc, "extract");
            s.save_as(&dest)
        })();
        match result {
            Ok(()) => {
                self.notify(tf!(
                    "Extracted {} page(s) to {}",
                    pages.len(),
                    dest.display()
                ));
                self.open_path(ctx, &dest);
            }
            Err(e) => {
                self.dialog = Some(Dialog::Error {
                    title: tr("Could not extract pages").into(),
                    detail: e.to_string(),
                })
            }
        }
    }

    fn merge_documents(&mut self) {
        let files = platform::dialogs::pick_open_pdfs(None);
        if files.is_empty() {
            return;
        }
        self.merge_files_with(files, None);
    }

    fn merge_files_with(&mut self, files: Vec<PathBuf>, policy: Option<FormPolicy>) {
        let mut sources: Vec<(PathBuf, pdf_engine::doc::PdfDocument)> = Vec::new();
        for f in &files {
            match std::fs::read(f)
                .map_err(pdf_engine::EngineError::from)
                .and_then(|b| {
                    pdf_engine::doc::PdfDocument::open(b, &pdf_engine::doc::OpenOptions::default())
                }) {
                Ok(d) => sources.push((f.clone(), d)),
                Err(e) => {
                    self.dialog = Some(Dialog::Error {
                        title: tr("Cannot merge").into(),
                        detail: format!("{}\n\n{e}", f.display()),
                    });
                    return;
                }
            }
        }
        let any_form = sources
            .iter()
            .any(|(_, d)| !pdf_engine::forms::read_form(d).fields.is_empty());
        if policy.is_none() && any_form {
            self.dialog = Some(Dialog::FormPolicy(FormPolicyState {
                op: FormOp::Merge(files),
                policy: FormPolicy::Independent,
            }));
            return;
        }
        let policy = policy.unwrap_or(FormPolicy::Independent);
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let n = tab.session.doc().page_count();
        let r = tab.session.execute(tr("Merge documents"), |tx| {
            let mut at = n;
            let mut total = 0;
            for (_, src) in &sources {
                let ids = src.page_ids()?;
                let added = pageops::import_pages(tx, src, &ids, at, policy)?;
                at += added.len();
                total += added.len();
            }
            Ok(total)
        });
        match r {
            Ok(total) => {
                tab.ui.goto = Some(n);
                self.notify(tf!("Added {} page(s)", total));
            }
            Err(e) => {
                self.dialog = Some(Dialog::Error {
                    title: tr("Cannot merge").into(),
                    detail: e.to_string(),
                })
            }
        }
    }
}

impl App {
    /// Continue a page operation after the user chose how form fields are treated.
    pub fn finish_form_policy(&mut self, ctx: &egui::Context, op: FormOp, policy: FormPolicy) {
        match op {
            FormOp::Duplicate => self.duplicate_pages_with(Some(policy)),
            FormOp::Extract => self.extract_pages_with(ctx, Some(policy)),
            FormOp::Merge(files) => self.merge_files_with(files, Some(policy)),
        }
    }
}

impl App {
    /// Open a picture as a new one-page PDF (it has no file until the user saves).
    fn open_image(&mut self, ctx: &egui::Context, path: &Path) {
        let result = std::fs::read(path)
            .map_err(pdf_engine::EngineError::from)
            .and_then(|bytes| pdf_engine::doc::PdfDocument::from_image(&bytes));
        match result {
            Ok(doc) => {
                let stem = path
                    .file_stem()
                    .map_or_else(|| "image".into(), |n| n.to_string_lossy().into_owned());
                let session = DocumentSession::from_new_document(doc, &format!("{stem}.pdf"));
                self.add_tab(session);
                self.notify(tr(
                    "Opened the picture as a PDF page. Save to create the PDF file.",
                ));
                ctx.request_repaint();
            }
            Err(e) => {
                self.dialog = Some(Dialog::Error {
                    title: tr("Cannot open image").into(),
                    detail: format!("{}\n\n{e}", path.display()),
                });
            }
        }
    }

    /// Insert pictures as new pages after the current page.
    pub fn insert_image_pages(&mut self) {
        let files = platform::dialogs::pick_open_images();
        if files.is_empty() {
            return;
        }
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let at = tab.session.view.current_page + 1;
        let mut datas = Vec::new();
        for f in &files {
            match std::fs::read(f) {
                Ok(b) => datas.push(b),
                Err(e) => {
                    self.notify_error(tf!("Could not read {}: {}", f.display(), e));
                    return;
                }
            }
        }
        let n = datas.len();
        let r = tab.session.execute(tr("Insert image pages"), |tx| {
            let mut last = None;
            for (i, d) in datas.iter().enumerate() {
                last = Some(pdf_engine::imagedoc::insert_image_page(tx, at + i, d)?);
            }
            Ok(last)
        });
        match r {
            Ok(_) => {
                tab.ui.goto = Some(at);
                self.notify(tf!("Inserted {} image page(s).", n));
            }
            Err(e) => self.notify_error(e.to_string()),
        }
    }
}
