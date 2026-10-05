//! Measurement tools, scale calibration, the measurement side panel and CSV export.
//!
//! Measurements are saved as standard PDF annotations carrying a `/Measure` dictionary; the
//! scale registry (document / page / region) lives in a private catalog entry. A measurement
//! made without a calibrated scale is reported in PDF points and flagged — the UI never
//! presents an uncalibrated number as if it were a real-world length.

use crate::canvas::ViewCtx;
use crate::dialogs::modal;
use crate::i18n::tr;
use crate::state::*;
use crate::tf;
use editor_core::tools::Tool;
use egui::{Align2, Color32, FontId, Pos2, RichText, Stroke};
use pdf_engine::annot::AnnotationKind;
use pdf_engine::doc::PageId;
use pdf_engine::geom::{Point, Rect as PRect};
use pdf_engine::measure::{
    self, MeasureKind, MeasureRow, Scale, ScaleRegion, ScaleSet, ScaleSource, Unit,
};
use std::sync::Arc;

/// Snap radius in screen pixels.
const SNAP_PX: f32 = 9.0;

/// Measurement kind a tool creates.
pub fn tool_kind(t: Tool) -> Option<MeasureKind> {
    Some(match t {
        Tool::MeasureDistance => MeasureKind::Distance,
        Tool::MeasurePerimeter => MeasureKind::Perimeter,
        Tool::MeasureArea => MeasureKind::Area,
        Tool::MeasureRect => MeasureKind::RectArea,
        Tool::MeasureRadius => MeasureKind::Radius,
        Tool::MeasureAngle => MeasureKind::Angle,
        Tool::Count => MeasureKind::Count,
        _ => return None,
    })
}

/// Number of clicks after which a tool finishes by itself (`None` = open-ended polyline).
fn fixed_points(k: MeasureKind) -> Option<usize> {
    match k {
        MeasureKind::Distance | MeasureKind::RectArea | MeasureKind::Radius => Some(2),
        MeasureKind::Angle => Some(3),
        MeasureKind::Count => Some(1),
        MeasureKind::Perimeter | MeasureKind::Area => None,
    }
}

fn vertices_of(k: &AnnotationKind) -> Vec<Point> {
    match k {
        AnnotationKind::Line { start, end, .. } => vec![*start, *end],
        AnnotationKind::PolyLine { points, .. } | AnnotationKind::Polygon { points } => {
            points.clone()
        }
        AnnotationKind::Rectangle { rect } => {
            let r = rect.abs();
            vec![
                Point::new(r.x0, r.y0),
                Point::new(r.x1, r.y0),
                Point::new(r.x1, r.y1),
                Point::new(r.x0, r.y1),
            ]
        }
        _ => Vec::new(),
    }
}

impl App {
    /// Scale registry of the active document (cached per revision).
    pub fn scales_for(&mut self) -> Arc<ScaleSet> {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return Arc::new(ScaleSet::default());
        };
        let rev = tab.session.revision();
        if let Some((r, s)) = &tab.ui.scales
            && *r == rev
        {
            return s.clone();
        }
        let s = Arc::new(measure::read_scales(tab.session.doc()));
        tab.ui.scales = Some((rev, s.clone()));
        s
    }

    /// Measurement report rows (cached per revision).
    pub fn measure_rows_for(&mut self) -> Arc<Vec<MeasureRow>> {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return Arc::new(Vec::new());
        };
        let rev = tab.session.revision();
        if let Some((r, s)) = &tab.ui.measure_rows
            && *r == rev
        {
            return s.clone();
        }
        let s = Arc::new(measure::collect_measurements(tab.session.doc()));
        tab.ui.measure_rows = Some((rev, s.clone()));
        s
    }

    /// The scale that applies at a point of a page.
    pub fn scale_at(&mut self, page: PageId, at: Option<Point>) -> (Scale, ScaleSource) {
        self.scales_for().resolve(page, at)
    }

    /// Text for the status bar: the scale of the current page.
    pub fn status_scale(&mut self) -> Option<(String, bool)> {
        let tab = self.tabs.get(self.active)?;
        let cur = tab.session.view.current_page;
        let page = *tab.session.doc().page_ids().ok()?.get(cur)?;
        let set = self.scales_for();
        let measuring = Tool::from_command(self.tool.command())
            .is_some_and(|t| t.family() == editor_core::tools::ToolFamily::Measure);
        let has_any = set.document.is_some() || !set.pages.is_empty() || !set.regions.is_empty();
        if !measuring && !has_any {
            return None;
        }
        let (s, src) = set.resolve(page, None);
        let where_ = match src {
            ScaleSource::Region => "region",
            ScaleSource::Page => "page",
            ScaleSource::Document => "document",
            ScaleSource::None => "",
        };
        Some(if s.calibrated {
            (tf!("Scale {} ({})", s.text, where_), true)
        } else {
            (
                tr("⚠ Scale not set — measuring in PDF points").to_string(),
                false,
            )
        })
    }

    /// Snap a pointer position to nearby annotation vertices and to the page's own geometry
    /// (line ends, corners, intersections, midpoints), or constrain the angle with Shift.
    pub(crate) fn snap_point(
        &mut self,
        vc: &ViewCtx,
        i: usize,
        pos: Pos2,
        extra: &[Point],
        prev: Option<Point>,
        shift: bool,
    ) -> (Point, Option<pdf_engine::snap::SnapKind>) {
        use pdf_engine::snap::SnapKind;
        let page = vc.pages[i].id;
        let raw = vc.screen_to_pdf(i, pos);
        let mut best: Option<(f32, Point)> = None;
        let annots = self.annots_for(page);
        let cands = annots
            .iter()
            .filter_map(|a| a.spec.as_ref())
            .flat_map(|s| vertices_of(&s.kind))
            .chain(extra.iter().copied());
        for c in cands {
            let d = vc.pdf_to_screen(i, c).distance(pos);
            if d <= SNAP_PX && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, c));
            }
        }
        if let Some((_, p)) = best {
            return (p, Some(SnapKind::Endpoint));
        }
        if self.prefs.snap_to_geometry
            && let Some(ix) = self.snap_index(i, vc)
            && let Some(h) = ix.query(raw, f64::from(SNAP_PX) / vc.px_per_pt)
        {
            return (h.point, Some(h.kind));
        }
        if shift && let Some(p0) = prev {
            return (crate::interaction::snap_angle(p0, raw), None);
        }
        (raw, None)
    }

    /// Pointer handling for the measurement tools and Count.
    pub fn measure_tool(
        &mut self,
        response: &egui::Response,
        vc: &ViewCtx,
        pos: Option<Pos2>,
        tool: Tool,
        mods: egui::Modifiers,
    ) {
        let Some(kind) = tool_kind(tool) else { return };
        let ti = self.active;
        let Some(pos) = pos else { return };
        let open_ended = fixed_points(kind).is_none();
        if response.double_clicked() && open_ended {
            self.finish_measure_poly(vc, tool);
            return;
        }
        if !response.clicked_by(egui::PointerButton::Primary) {
            return;
        }
        let page_idx = match &self.tabs[ti].ui.interaction {
            Interaction::Draw { page, .. } => vc.pages.iter().position(|p| p.id == *page),
            _ => vc.page_at(pos),
        };
        let Some(i) = page_idx else { return };
        let existing = self.tabs[ti].ui.polygon_points.clone();
        let (p, _) = self.snap_point(vc, i, pos, &existing, existing.last().copied(), mods.shift);
        let page = vc.pages[i].id;
        self.tabs[ti].ui.polygon_points.push(p);
        let pts = self.tabs[ti].ui.polygon_points.clone();
        self.tabs[ti].ui.interaction = Interaction::Draw {
            page,
            tool,
            points: pts.clone(),
        };
        if fixed_points(kind).is_some_and(|n| pts.len() >= n) {
            self.tabs[ti].ui.polygon_points.clear();
            self.tabs[ti].ui.interaction = Interaction::None;
            self.commit_measurement(page, kind, pts);
        }
    }

    /// Enter / double-click on an open-ended measurement.
    pub fn finish_measure_poly(&mut self, vc: &ViewCtx, tool: Tool) {
        let ti = self.active;
        let Some(kind) = tool_kind(tool) else { return };
        let mut pts = std::mem::take(&mut self.tabs[ti].ui.polygon_points);
        // A double-click adds the same point twice.
        while pts.len() >= 2 && pts[pts.len() - 1] == pts[pts.len() - 2] {
            pts.pop();
        }
        let inter = std::mem::take(&mut self.tabs[ti].ui.interaction);
        if let Interaction::Draw { page, .. } = inter
            && vc.pages.iter().any(|p| p.id == page)
        {
            self.commit_measurement(page, kind, pts);
        }
    }

    fn commit_measurement(&mut self, page: PageId, kind: MeasureKind, pts: Vec<Point>) {
        if pts.len() < kind.min_points() {
            return;
        }
        let (scale, _) = self.scale_at(page, pts.first().copied());
        let category = {
            let c = self.tabs[self.active].ui.count_category.trim().to_string();
            if c.is_empty() {
                "Count 1".to_string()
            } else {
                c
            }
        };
        let spec = match measure::spec_for(kind, &pts, &scale, &category) {
            Ok(mut s) => {
                s.author = self.prefs.author.clone();
                s
            }
            Err(e) => {
                self.notify_error(e.to_string());
                return;
            }
        };
        if !scale.calibrated && kind != MeasureKind::Count && kind != MeasureKind::Angle {
            self.notify(
                tr("No scale is set for this page, so the value is in PDF points. Use Calibrate Scale for real units."),
            );
        }
        let label = if kind == MeasureKind::Count {
            tr("Add count marker")
        } else {
            tr("Add measurement")
        };
        let r = self.tabs[self.active].session.execute(label, |tx| {
            pdf_engine::annot::add_annotation(tx, page, &spec)
        });
        match r {
            Ok(id) => {
                let t = &mut self.tabs[self.active];
                t.session.selection.annotations = vec![(page, id)];
            }
            Err(e) => self.notify_error(e.to_string()),
        }
    }

    /// Calibrate tool: two clicks (or, when a region scale is pending, a dragged rectangle).
    pub fn calibrate_tool(
        &mut self,
        response: &egui::Response,
        vc: &ViewCtx,
        pos: Option<Pos2>,
        mods: egui::Modifiers,
    ) {
        let ti = self.active;
        let Some(pos) = pos else { return };
        if let Some((page, scale)) = self.tabs[ti].ui.pending_region.clone() {
            self.region_drag(response, vc, pos, page, scale);
            return;
        }
        if !response.clicked_by(egui::PointerButton::Primary) {
            return;
        }
        let page_idx = match &self.tabs[ti].ui.interaction {
            Interaction::Draw { page, .. } => vc.pages.iter().position(|p| p.id == *page),
            _ => vc.page_at(pos),
        };
        let Some(i) = page_idx else { return };
        let existing = self.tabs[ti].ui.polygon_points.clone();
        let (p, _) = self.snap_point(vc, i, pos, &existing, existing.last().copied(), mods.shift);
        let page = vc.pages[i].id;
        self.tabs[ti].ui.polygon_points.push(p);
        let pts = self.tabs[ti].ui.polygon_points.clone();
        if pts.len() >= 2 {
            self.tabs[ti].ui.polygon_points.clear();
            self.tabs[ti].ui.interaction = Interaction::None;
            let len = (pts[0].x - pts[1].x).hypot(pts[0].y - pts[1].y);
            if len < 1.0 {
                self.notify(tr("Those two points are too close together to calibrate."));
                return;
            }
            self.open_scale_dialog(page, Some(len));
        } else {
            self.tabs[ti].ui.interaction = Interaction::Draw {
                page,
                tool: Tool::Calibrate,
                points: pts,
            };
        }
    }

    fn region_drag(
        &mut self,
        response: &egui::Response,
        vc: &ViewCtx,
        pos: Pos2,
        page: PageId,
        scale: Scale,
    ) {
        let ti = self.active;
        let Some(i) = vc.pages.iter().position(|p| p.id == page) else {
            return;
        };
        let p = vc.screen_to_pdf(i, pos);
        if response.drag_started_by(egui::PointerButton::Primary) {
            self.tabs[ti].ui.interaction = Interaction::Draw {
                page,
                tool: Tool::Calibrate,
                points: vec![p, p],
            };
        } else if response.dragged_by(egui::PointerButton::Primary)
            && let Interaction::Draw { points, .. } = &mut self.tabs[ti].ui.interaction
        {
            points.truncate(1);
            points.push(p);
        } else if response.drag_stopped()
            && let Interaction::Draw { points, .. } =
                std::mem::take(&mut self.tabs[ti].ui.interaction)
            && points.len() == 2
        {
            let r = PRect::new(points[0].x, points[0].y, points[1].x, points[1].y).abs();
            if r.width() < 8.0 || r.height() < 8.0 {
                self.notify(tr("Drag a larger rectangle for the region."));
                return;
            }
            self.tabs[ti].ui.pending_region = None;
            let mut set = (*self.scales_for()).clone();
            set.regions.push(ScaleRegion {
                page,
                rect: r,
                scale,
            });
            match self.tabs[ti]
                .session
                .execute(tr("Set region scale"), |tx| measure::write_scales(tx, &set))
            {
                Ok(()) => self.notify(tr("Region scale set.")),
                Err(e) => self.notify_error(e.to_string()),
            }
        }
    }

    /// Open the scale dialog for `page`.
    pub fn open_scale_dialog(&mut self, page: PageId, measured: Option<f64>) {
        self.dialog = Some(Dialog::Scale(Box::new(ScaleDialog {
            page,
            measured_pts: measured,
            mode: if measured.is_some() {
                ScaleMode::Calibrate
            } else {
                ScaleMode::Ratio
            },
            scope: ScaleScope::Page,
            known_len: String::new(),
            known_unit: Unit::M,
            ratio_n: "100".into(),
            ratio_unit: Unit::M,
            paper_len: "1".into(),
            paper_unit: Unit::In,
            real_len: "10".into(),
            real_unit: Unit::Ft,
            apply_existing: true,
            error: None,
        })));
    }

    /// The scale dialog. Returns whether it stays open.
    pub fn dialog_scale(&mut self, ctx: &egui::Context, st: &mut ScaleDialog) -> bool {
        let mut choice: Option<bool> = None;
        let unit_combo = |ui: &mut egui::Ui, id: &str, u: &mut Unit| {
            egui::ComboBox::from_id_salt(id)
                .selected_text(u.abbr())
                .width(60.0)
                .show_ui(ui, |ui| {
                    for c in Unit::ALL {
                        ui.selectable_value(u, c, c.abbr());
                    }
                });
        };
        modal(ctx, "scale_dialog", |ui| {
            ui.heading(tr("Set drawing scale"));
            ui.label(
                RichText::new(tr("Measurements use this scale. Pick how you know it."))
                    .size(12.0)
                    .color(self.pal.text_dim),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_enabled_ui(st.measured_pts.is_some(), |ui| {
                    ui.radio_value(&mut st.mode, ScaleMode::Calibrate, tr("Known length"));
                });
                ui.radio_value(&mut st.mode, ScaleMode::Ratio, tr("Ratio 1 : n"));
                ui.radio_value(&mut st.mode, ScaleMode::Custom, tr("Custom"));
            });
            ui.add_space(4.0);
            match st.mode {
                ScaleMode::Calibrate => {
                    if let Some(m) = st.measured_pts {
                        ui.label(tf!(
                            "The two points you clicked are {} pt ({} mm on paper) apart.",
                            format!("{:.1}", m),
                            format!("{:.1}", m / Unit::Mm.points())
                        ));
                    }
                    ui.horizontal(|ui| {
                        ui.label(tr("Their real length is"));
                        ui.add(egui::TextEdit::singleline(&mut st.known_len).desired_width(80.0));
                        unit_combo(ui, "cal_unit", &mut st.known_unit);
                    });
                }
                ScaleMode::Ratio => {
                    ui.horizontal(|ui| {
                        ui.label("1 :");
                        ui.add(egui::TextEdit::singleline(&mut st.ratio_n).desired_width(80.0));
                        ui.label(tr("show results in"));
                        unit_combo(ui, "ratio_unit", &mut st.ratio_unit);
                    });
                    ui.horizontal(|ui| {
                        for n in ["20", "50", "100", "200", "500"] {
                            if ui.small_button(format!("1:{n}")).clicked() {
                                st.ratio_n = n.into();
                            }
                        }
                    });
                }
                ScaleMode::Custom => {
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut st.paper_len).desired_width(60.0));
                        unit_combo(ui, "paper_unit", &mut st.paper_unit);
                        ui.label(tr("on paper  ="));
                        ui.add(egui::TextEdit::singleline(&mut st.real_len).desired_width(60.0));
                        unit_combo(ui, "real_unit", &mut st.real_unit);
                        ui.label("real");
                    });
                    ui.horizontal(|ui| {
                        for (label, pl, pu, rl, ru) in [
                            ("¼″ = 1′", "0.25", Unit::In, "1", Unit::Ft),
                            ("⅛″ = 1′", "0.125", Unit::In, "1", Unit::Ft),
                            ("1″ = 10′", "1", Unit::In, "10", Unit::Ft),
                            ("1 cm = 1 m", "1", Unit::Cm, "1", Unit::M),
                        ] {
                            if ui.small_button(label).clicked() {
                                st.paper_len = pl.into();
                                st.paper_unit = pu;
                                st.real_len = rl.into();
                                st.real_unit = ru;
                            }
                        }
                    });
                }
            }
            ui.add_space(8.0);
            ui.label(tr("Apply to"));
            ui.horizontal(|ui| {
                ui.radio_value(&mut st.scope, ScaleScope::Page, tr("This page"));
                ui.radio_value(&mut st.scope, ScaleScope::Document, tr("Whole document"));
                ui.radio_value(
                    &mut st.scope,
                    ScaleScope::Region,
                    tr("A region of this page"),
                );
            });
            if st.scope == ScaleScope::Region {
                ui.label(
                    RichText::new(tr(
                        "After OK, drag a rectangle on the page for a detail drawn at this scale.",
                    ))
                    .size(12.0)
                    .color(self.pal.text_dim),
                );
            } else {
                ui.checkbox(
                    &mut st.apply_existing,
                    tr("Update existing measurements to this scale"),
                );
            }
            if let Some(e) = &st.error {
                ui.colored_label(self.pal.danger, e);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    choice = Some(true);
                }
                if ui.button(tr("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    choice = Some(false);
                }
            });
        });
        match choice {
            None => true,
            Some(false) => false,
            Some(true) => match self.apply_scale_dialog(st) {
                Ok(()) => false,
                Err(e) => {
                    st.error = Some(e);
                    true
                }
            },
        }
    }

    fn apply_scale_dialog(&mut self, st: &ScaleDialog) -> Result<(), String> {
        let num = |s: &str| -> Result<f64, String> {
            s.trim()
                .replace(',', ".")
                .parse::<f64>()
                .map_err(|_| tf!("“{}” is not a number", s))
        };
        let scale = match st.mode {
            ScaleMode::Calibrate => {
                let m = st
                    .measured_pts
                    .ok_or(tr("Draw a calibration segment first"))?;
                Scale::from_calibration(m, num(&st.known_len)?, st.known_unit)
            }
            ScaleMode::Ratio => Scale::from_one_to(num(&st.ratio_n)?, st.ratio_unit),
            ScaleMode::Custom => Scale::from_ratio(
                num(&st.paper_len)?,
                st.paper_unit,
                num(&st.real_len)?,
                st.real_unit,
            ),
        }
        .map_err(|e| e.to_string())?;
        let ti = self.active;
        if st.scope == ScaleScope::Region {
            self.tabs[ti].ui.pending_region = Some((st.page, scale));
            self.set_tool(Tool::Calibrate);
            self.notify(tr("Drag a rectangle on the page to define the region."));
            return Ok(());
        }
        let mut set = (*self.scales_for()).clone();
        let all_pages: Vec<PageId> = self.tabs[ti].session.doc().page_ids().unwrap_or_default();
        let mut targets: Vec<PageId> = Vec::new();
        match st.scope {
            ScaleScope::Document => {
                set.document = Some(scale.clone());
                targets = all_pages
                    .into_iter()
                    .filter(|p| {
                        !set.pages.contains_key(p) && !set.regions.iter().any(|r| r.page == *p)
                    })
                    .collect();
            }
            ScaleScope::Page => {
                set.pages.insert(st.page, scale.clone());
                if !set.regions.iter().any(|r| r.page == st.page) {
                    targets.push(st.page);
                }
            }
            ScaleScope::Region => {}
        }
        let apply = st.apply_existing;
        let r = self.tabs[ti].session.execute(tr("Set scale"), |tx| {
            measure::write_scales(tx, &set)?;
            let mut n = 0;
            if apply {
                for p in &targets {
                    n += measure::rescale_page(tx, *p, &scale)?;
                }
            }
            Ok(n)
        });
        match r {
            Ok(n) => {
                if n > 0 {
                    self.notify(tf!("Scale set; {} measurement(s) updated.", n));
                } else {
                    self.notify(tr("Scale set."));
                }
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// Clear the page or document scale.
    fn clear_scale(&mut self, page: Option<PageId>) {
        let mut set = (*self.scales_for()).clone();
        match page {
            Some(p) => {
                set.pages.remove(&p);
                set.regions.retain(|r| r.page != p);
            }
            None => set.document = None,
        }
        if let Err(e) = self.tabs[self.active]
            .session
            .execute(tr("Clear scale"), |tx| measure::write_scales(tx, &set))
        {
            self.notify_error(e.to_string());
        }
    }

    /// Save all measurements and counts as CSV.
    pub fn export_measurements(&mut self) {
        let rows = self.measure_rows_for();
        if rows.is_empty() {
            self.notify(tr("There are no measurements to export."));
            return;
        }
        let Some(tab) = self.active_tab() else { return };
        let suggested = format!(
            "{}-measurements.csv",
            tab.session.title.trim_end_matches(".pdf")
        );
        let Some(dest) = platform::dialogs::pick_save_csv(&suggested) else {
            return;
        };
        let csv = measure::to_csv(&rows);
        match std::fs::write(&dest, csv) {
            Ok(()) => self.notify(tf!("Exported {} row(s) to {}", rows.len(), dest.display())),
            Err(e) => self.notify_error(tf!("Could not write {}: {}", dest.display(), e)),
        }
    }

    /// Right-hand "Measure" panel.
    pub fn measure_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.tabs.is_empty() {
            return;
        }
        let ti = self.active;
        let cur = self.tabs[ti].session.view.current_page;
        let pages: Vec<PageId> = self.tabs[ti].session.doc().page_ids().unwrap_or_default();
        let Some(&page) = pages.get(cur) else { return };
        let set = self.scales_for();
        let (scale, src) = set.resolve(page, None);
        ui.label(RichText::new(tr("Scale (this page)")).strong());
        if scale.calibrated {
            ui.label(format!(
                "{}  ·  {}",
                scale.text,
                match src {
                    ScaleSource::Region => "region",
                    ScaleSource::Page => "page",
                    ScaleSource::Document => "document",
                    ScaleSource::None => "",
                }
            ));
        } else {
            ui.colored_label(
                self.pal.danger,
                tr("Not calibrated. Values are in PDF points until you set a scale."),
            );
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button(tr("Set scale…")).clicked() {
                self.open_scale_dialog(page, None);
            }
            if ui.button(tr("Calibrate…")).clicked() {
                self.set_tool(Tool::Calibrate);
            }
            if (set.pages.contains_key(&page) || set.regions.iter().any(|r| r.page == page))
                && ui.small_button(tr("Clear page scale")).clicked()
            {
                self.clear_scale(Some(page));
            }
            if set.document.is_some() && ui.small_button(tr("Clear document scale")).clicked() {
                self.clear_scale(None);
            }
        });
        ui.separator();
        ui.label(RichText::new(tr("Count category")).strong());
        ui.add(
            egui::TextEdit::singleline(&mut self.tabs[ti].ui.count_category)
                .hint_text("Count 1")
                .desired_width(180.0),
        );
        let rows = self.measure_rows_for();
        let counts = measure::count_summary(&rows);
        for c in &counts {
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(
                        self.tabs[ti].ui.count_category == c.category,
                        format!("{} — {}", c.category, c.total),
                    )
                    .on_hover_text(tr("Click to place more markers in this category"))
                    .clicked()
                {
                    self.tabs[ti].ui.count_category = c.category.clone();
                    self.set_tool(Tool::Count);
                }
            });
        }
        ui.separator();
        // Measurements made at another scale than the current one are called out.
        let stale = rows
            .iter()
            .filter(|r| {
                r.page == page
                    && !matches!(r.kind, MeasureKind::Count | MeasureKind::Angle)
                    && r.scale != scale.text
            })
            .count();
        if stale > 0 {
            ui.colored_label(
                self.pal.danger,
                tf!(
                    "{} measurement(s) on this page use a different scale.",
                    stale
                ),
            );
            if ui
                .small_button(tr("Update them to the page scale"))
                .clicked()
            {
                let sc = scale.clone();
                match self.tabs[ti]
                    .session
                    .execute(tr("Update measurements"), |tx| {
                        measure::rescale_page(tx, page, &sc)
                    }) {
                    Ok(n) => self.notify(tf!("{} measurement(s) updated.", n)),
                    Err(e) => self.notify_error(e.to_string()),
                }
            }
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(tf!("Measurements ({})", rows.len())).strong());
            if ui
                .add_enabled(!rows.is_empty(), egui::Button::new(tr("Export CSV…")))
                .clicked()
            {
                self.export_measurements();
            }
        });
        // Totals per kind and unit.
        let mut totals: std::collections::BTreeMap<(MeasureKind, String), f64> = Default::default();
        for r in rows.iter().filter(|r| r.kind != MeasureKind::Count) {
            *totals.entry((r.kind, r.unit.clone())).or_insert(0.0) += r.value;
        }
        for ((k, u), v) in &totals {
            if !matches!(k, MeasureKind::Angle) {
                ui.label(
                    RichText::new(format!("Σ {}: {:.2} {u}", tr(k.title()), v))
                        .size(12.0)
                        .color(self.pal.text_dim),
                );
            }
        }
        let mut jump: Option<(usize, PageId, pdf_engine::annot::AnnotId)> = None;
        for r in rows.iter().filter(|r| r.kind != MeasureKind::Count) {
            let text = format!(
                "p{}  {}  {:.2} {}{}",
                r.page_number,
                tr(r.kind.title()),
                r.value,
                r.unit,
                if r.calibrated {
                    ""
                } else {
                    tr("  (uncalibrated)")
                }
            );
            if ui.selectable_label(false, text).clicked() {
                jump = Some((r.page_number - 1, r.page, r.id));
            }
        }
        if let Some((pi, pg, id)) = jump {
            self.go_to(pi);
            self.tabs[ti].session.selection.annotations = vec![(pg, id)];
            ctx.request_repaint();
        }
    }

    /// Overlays: region outlines, in-progress measurement with live value, snap marker.
    pub fn paint_measure_overlays(&mut self, painter: &egui::Painter, vc: &ViewCtx) {
        if self.tabs.is_empty() {
            return;
        }
        let ti = self.active;
        let accent = Color32::from_rgb(0, 115, 217);
        let measuring = tool_kind(self.tool).is_some() || self.tool == Tool::Calibrate;
        if measuring || self.right_tab == RightTab::Measure {
            let set = self.scales_for();
            for r in &set.regions {
                if let Some(i) = vc.pages.iter().position(|p| p.id == r.page) {
                    let sr = Self::rect_screen(vc, i, r.rect);
                    painter.add(egui::Shape::dashed_line(
                        &[
                            sr.left_top(),
                            sr.right_top(),
                            sr.right_bottom(),
                            sr.left_bottom(),
                            sr.left_top(),
                        ],
                        Stroke::new(1.5, Color32::from_rgb(180, 90, 0)),
                        6.0,
                        4.0,
                    ));
                    painter.text(
                        sr.left_top() + egui::vec2(3.0, 2.0),
                        Align2::LEFT_TOP,
                        &r.scale.text,
                        FontId::proportional(11.0),
                        Color32::from_rgb(180, 90, 0),
                    );
                }
            }
        }
        let inter = self.tabs[ti].ui.interaction.clone();
        let Interaction::Draw { page, tool, points } = inter else {
            return;
        };
        let Some(i) = vc.pages.iter().position(|p| p.id == page) else {
            return;
        };
        let hover = painter
            .ctx()
            .input(|inp| (inp.pointer.hover_pos(), inp.modifiers.shift));
        if tool == Tool::Calibrate {
            if self.tabs[ti].ui.pending_region.is_some() && points.len() == 2 {
                let r = Self::rect_screen(
                    vc,
                    i,
                    PRect::new(points[0].x, points[0].y, points[1].x, points[1].y).abs(),
                );
                painter.rect_stroke(r, 0.0, Stroke::new(1.5, accent), egui::StrokeKind::Middle);
            } else if let (Some(a), Some(h)) = (points.first(), hover.0) {
                let sa = vc.pdf_to_screen(i, *a);
                painter.line_segment([sa, h], Stroke::new(1.5, accent));
                painter.circle_filled(sa, 3.5, accent);
            }
            return;
        }
        let Some(kind) = tool_kind(tool) else { return };
        let mut pts = points.clone();
        if let Some(h) = hover.0
            && vc.page_at(h) == Some(i)
        {
            let (p, _) = self.snap_point(vc, i, h, &points, points.last().copied(), hover.1);
            pts.push(p);
        }
        let sp: Vec<Pos2> = pts.iter().map(|p| vc.pdf_to_screen(i, *p)).collect();
        let st = Stroke::new(1.5, accent);
        match kind {
            MeasureKind::RectArea if sp.len() >= 2 => {
                painter.rect_stroke(
                    egui::Rect::from_two_pos(sp[0], sp[1]),
                    0.0,
                    st,
                    egui::StrokeKind::Middle,
                );
            }
            MeasureKind::Radius if sp.len() >= 2 => {
                painter.circle_stroke(sp[0], sp[0].distance(sp[1]), st);
                painter.line_segment([sp[0], sp[1]], st);
            }
            _ => {
                if sp.len() >= 2 {
                    painter.add(egui::epaint::PathShape::line(sp.clone(), st));
                    if kind == MeasureKind::Area && sp.len() >= 3 {
                        painter.line_segment(
                            [sp[sp.len() - 1], sp[0]],
                            Stroke::new(1.0, accent.gamma_multiply(0.6)),
                        );
                    }
                }
            }
        }
        for p in &sp[..points.len().min(sp.len())] {
            painter.circle_filled(*p, 3.5, accent);
        }
        // Live value next to the cursor.
        if pts.len() >= kind.min_points() && kind != MeasureKind::Count {
            let (scale, _) = self.scale_at(page, pts.first().copied());
            if let Ok(m) = measure::measure(kind, &pts, &scale)
                && let Some(h) = hover.0
            {
                let text = m.label;
                let galley =
                    painter.layout_no_wrap(text, FontId::proportional(13.0), Color32::WHITE);
                let r = egui::Rect::from_min_size(h + egui::vec2(14.0, 14.0), galley.size())
                    .expand(4.0);
                painter.rect_filled(r, 3.0, Color32::from_rgba_unmultiplied(20, 40, 70, 230));
                painter.galley(r.min + egui::vec2(4.0, 4.0), galley, Color32::WHITE);
            }
        }
    }
}

impl App {
    /// Whether the active tool belongs to the measurement family.
    pub fn tool_is_measure(&self) -> bool {
        self.tool.family() == editor_core::tools::ToolFamily::Measure
    }
}
