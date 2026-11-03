//! Organize ▸ Header & Footer, Bates Numbering, Watermark, Page Labels and Crop: the dialogs, the
//! crop tool and the crop preview. The work itself is done by `pdf_engine::pagemarks`,
//! `pdf_engine::pagelabels` and `pageops::set_crop_box`, each as one undoable step.

use crate::canvas::ViewCtx;
use crate::dialogs::modal;
use crate::i18n::tr;
use crate::state::*;
use crate::tf;
use egui::{Color32, Pos2, RichText, Stroke};
use pdf_engine::EngineError;
use pdf_engine::doc::PageId;
use pdf_engine::fontembed::FontStyle;
use pdf_engine::geom::{Point, Rect as PRect, Rotation};
use pdf_engine::pagelabels::{self, LabelRange, LabelStyle};
use pdf_engine::pagemarks::{
    self, Bates, HeaderFooter, MarkContext, MarkKind, Watermark, WatermarkContent,
};
use pdf_engine::pageops;

const PT_PER_MM: f64 = 72.0 / 25.4;

/// Which pages a dialog works on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageScope {
    All,
    Current,
    Range,
}

/// The "Pages: all / this page / range" row.
#[derive(Clone, Debug)]
pub struct PageChoice {
    pub scope: PageScope,
    pub range: String,
}

impl PageChoice {
    fn new(scope: PageScope) -> Self {
        Self {
            scope,
            range: String::new(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        crate::ui_kit::form_row(ui, tr("Pages"), |ui| {
            ui.radio_value(&mut self.scope, PageScope::All, tr("All"));
            ui.radio_value(&mut self.scope, PageScope::Current, tr("This page"));
            ui.radio_value(&mut self.scope, PageScope::Range, tr("Pages"));
            let r = ui.add(
                crate::ui_kit::singleline(&mut self.range)
                    .hint_text("1-3, 5")
                    .desired_width(90.0),
            );
            if r.changed() {
                self.scope = PageScope::Range;
            }
        });
    }

    /// Zero-based page indexes, in document order, without repeats.
    fn resolve(&self, current: usize, total: usize) -> Result<Vec<usize>, String> {
        let mut v = match self.scope {
            PageScope::All => (0..total).collect(),
            PageScope::Current => vec![current.min(total.saturating_sub(1))],
            PageScope::Range => printing::parse_page_ranges(&self.range, total)?,
        };
        v.sort_unstable();
        v.dedup();
        if v.is_empty() {
            return Err(tr("No pages chosen.").into());
        }
        Ok(v)
    }
}

/// Header & Footer (and Bates numbering).
pub struct HeaderFooterState {
    slots: [String; 6],
    font: FontStyle,
    size: f64,
    color: [f32; 3],
    margin_top_mm: f64,
    margin_bottom_mm: f64,
    margin_side_mm: f64,
    first_number: usize,
    bates: bool,
    bates_prefix: String,
    bates_suffix: String,
    bates_start: u64,
    bates_digits: usize,
    pages: PageChoice,
    error: Option<String>,
}

impl HeaderFooterState {
    fn new(bates: bool) -> Self {
        let mut slots: [String; 6] = Default::default();
        if bates {
            slots[5] = "{bates}".into();
        } else {
            slots[4] = tr("Page {page} of {pages}").into();
        }
        Self {
            slots,
            font: FontStyle::default(),
            size: 10.0,
            color: [0.0, 0.0, 0.0],
            margin_top_mm: 10.0,
            margin_bottom_mm: 10.0,
            margin_side_mm: 12.0,
            first_number: 1,
            bates,
            bates_prefix: String::new(),
            bates_suffix: String::new(),
            bates_start: 1,
            bates_digits: 6,
            pages: PageChoice::new(PageScope::All),
            error: None,
        }
    }
}

/// Watermark.
pub struct WatermarkState {
    picture_mode: bool,
    text: String,
    font: FontStyle,
    auto_size: bool,
    size: f64,
    color: [f32; 3],
    picture: Option<(Vec<u8>, String)>,
    width_pct: f64,
    opacity_pct: f64,
    rotation: f64,
    behind: bool,
    pages: PageChoice,
    error: Option<String>,
}

impl Default for WatermarkState {
    fn default() -> Self {
        Self {
            picture_mode: false,
            text: tr("CONFIDENTIAL").into(),
            font: FontStyle::new(pdf_engine::fontembed::FontFamily::DejaVuSans, true, false),
            auto_size: true,
            size: 60.0,
            color: [0.75, 0.1, 0.1],
            picture: None,
            width_pct: 50.0,
            opacity_pct: 25.0,
            rotation: 45.0,
            behind: false,
            pages: PageChoice::new(PageScope::All),
            error: None,
        }
    }
}

/// One editable row of the page labels dialog.
#[derive(Clone, Debug)]
pub struct LabelRow {
    /// First page, 1-based as people count.
    from: usize,
    style: LabelStyle,
    prefix: String,
    first: u32,
}

/// Page Labels.
pub struct LabelsState {
    rows: Vec<LabelRow>,
    total: usize,
    error: Option<String>,
}

/// Crop Pages: margins measured on the page as shown, in millimetres.
pub struct CropState {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
    pub pages: PageChoice,
    pub error: Option<String>,
}

/// Remove marks: which kinds.
pub struct RemoveMarksState {
    header_footer: bool,
    watermark: bool,
    has_hf: bool,
    has_wm: bool,
}

/// Today's date as `YYYY-MM-DD` (UTC).
fn today() -> String {
    let d = pdf_engine::annot::now_pdf_date();
    let digits: String = d.chars().filter(char::is_ascii_digit).take(8).collect();
    if digits.len() == 8 {
        format!("{}-{}-{}", &digits[0..4], &digits[4..6], &digits[6..8])
    } else {
        String::new()
    }
}

fn color_tuple(c: [f32; 3]) -> (f32, f32, f32) {
    (c[0], c[1], c[2])
}

fn mm_field(ui: &mut egui::Ui, v: &mut f64) {
    ui.add(
        egui::DragValue::new(v)
            .range(0.0..=500.0)
            .speed(0.5)
            .suffix(" mm"),
    );
}

impl App {
    fn page_ids_and_current(&mut self) -> Option<(Vec<PageId>, usize)> {
        let tab = self.tabs.get_mut(self.active)?;
        let ids = tab.session.doc().page_ids().ok()?;
        Some((ids, tab.session.view.current_page))
    }

    fn missing_glyphs_message(e: &EngineError) -> String {
        match e {
            EngineError::MissingGlyphs { chars, font } => tf!(
                "The font {} cannot show these characters: {}. Choose another font or remove them.",
                font,
                chars
            ),
            other => other.to_string(),
        }
    }

    // ---- header & footer --------------------------------------------------------------------

    /// Organize ▸ Header & Footer… (or Bates Numbering…).
    pub fn open_header_footer(&mut self, bates: bool) {
        self.dialog = Some(Dialog::HeaderFooter(Box::new(HeaderFooterState::new(
            bates,
        ))));
    }

    pub fn dialog_header_footer(
        &mut self,
        ctx: &egui::Context,
        st: &mut HeaderFooterState,
    ) -> bool {
        let mut choice: Option<bool> = None;
        let dim = self.pal.text_dim;
        let danger = self.pal.danger;
        modal(ctx, "header_footer", |ui| {
            ui.set_max_width(620.0);
            ui.heading(if st.bates {
                tr("Bates numbering")
            } else {
                tr("Header & footer")
            });
            ui.label(
                RichText::new(tr("The text becomes part of the pages, so every reader and printer shows it. Placeholders: {page}, {pages}, {label}, {date}, {file} and {bates}."))
                    .size(12.0)
                    .color(dim),
            );
            ui.add_space(4.0);
            egui::Grid::new("hf_slots")
                .num_columns(4)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    ui.label("");
                    ui.label(RichText::new(tr("Left")).color(dim));
                    ui.label(RichText::new(tr("Centre")).color(dim));
                    ui.label(RichText::new(tr("Right")).color(dim));
                    ui.end_row();
                    for (row, title) in [(0, tr("Header")), (1, tr("Footer"))] {
                        ui.label(title);
                        for col in 0..3 {
                            ui.add(
                                crate::ui_kit::singleline(&mut st.slots[row * 3 + col])
                                    .char_limit(pagemarks::MAX_SLOT)
                                    .desired_width(160.0),
                            );
                        }
                        ui.end_row();
                    }
                });
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(tr("Insert:")).size(12.0).color(dim));
                for ph in [
                    "{page}", "{pages}", "{label}", "{date}", "{file}", "{bates}",
                ] {
                    if ui
                        .small_button(ph)
                        .on_hover_text(tr("Adds it to the centre of the footer"))
                        .clicked()
                    {
                        st.slots[4].push_str(ph);
                    }
                }
            });
            ui.add_space(4.0);
            crate::ui_kit::form_row(ui, tr("Font"), |ui| {
                crate::fontpick::font_picker(ui, "hf_font", &mut st.font);
            });
            crate::ui_kit::form_row(ui, tr("Size"), |ui| {
                ui.add(
                    egui::DragValue::new(&mut st.size)
                        .range(4.0..=72.0)
                        .speed(0.2)
                        .suffix(" pt"),
                );
                ui.color_edit_button_rgb(&mut st.color);
            });
            crate::ui_kit::form_row(ui, tr("Margins"), |ui| {
                ui.label(tr("top"));
                mm_field(ui, &mut st.margin_top_mm);
                ui.label(tr("bottom"));
                mm_field(ui, &mut st.margin_bottom_mm);
                ui.label(tr("sides"));
                mm_field(ui, &mut st.margin_side_mm);
            });
            crate::ui_kit::form_row(ui, tr("First page"), |ui| {
                ui.label(tr("{page} starts at"));
                ui.add(egui::DragValue::new(&mut st.first_number).range(0..=999_999));
            });
            ui.checkbox(&mut st.bates, tr("Bates numbering for {bates}"));
            if st.bates {
                crate::ui_kit::form_row(ui, tr("Bates"), |ui| {
                    ui.add(
                        crate::ui_kit::singleline(&mut st.bates_prefix)
                            .hint_text(tr("prefix"))
                            .desired_width(90.0),
                    );
                    ui.add(egui::DragValue::new(&mut st.bates_start).range(0..=999_999_999_999u64));
                    ui.add(
                        crate::ui_kit::singleline(&mut st.bates_suffix)
                            .hint_text(tr("suffix"))
                            .desired_width(70.0),
                    );
                    ui.label(tr("digits"));
                    ui.add(egui::DragValue::new(&mut st.bates_digits).range(1..=12));
                });
                let b = Bates {
                    prefix: st.bates_prefix.clone(),
                    suffix: st.bates_suffix.clone(),
                    start: st.bates_start,
                    digits: st.bates_digits,
                };
                ui.label(
                    RichText::new(tf!("First pages: {}, {}, …", b.format(0), b.format(1)))
                        .size(12.0)
                        .color(dim),
                );
            }
            st.pages.ui(ui);
            if let Some(e) = &st.error {
                ui.colored_label(danger, e);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if crate::ui_kit::primary_button(ui, tr("Add")).clicked() {
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
            Some(true) => {
                let Some((ids, current)) = self.page_ids_and_current() else {
                    return false;
                };
                let idx = match st.pages.resolve(current, ids.len()) {
                    Ok(v) => v,
                    Err(e) => {
                        st.error = Some(e);
                        return true;
                    }
                };
                let pages: Vec<PageId> = idx.iter().filter_map(|i| ids.get(*i).copied()).collect();
                let uses_bates = st.slots.iter().any(|s| s.contains("{bates}"));
                let spec = HeaderFooter {
                    slots: st.slots.clone(),
                    font: st.font,
                    size: st.size,
                    color: color_tuple(st.color),
                    margin_top: st.margin_top_mm * PT_PER_MM,
                    margin_bottom: st.margin_bottom_mm * PT_PER_MM,
                    margin_side: st.margin_side_mm * PT_PER_MM,
                    first_number: st.first_number,
                    bates: (st.bates || uses_bates).then(|| Bates {
                        prefix: st.bates_prefix.clone(),
                        suffix: st.bates_suffix.clone(),
                        start: st.bates_start,
                        digits: st.bates_digits,
                    }),
                };
                let tab = &mut self.tabs[self.active];
                let ranges = pagelabels::read_page_labels(tab.session.doc().lopdf());
                let mctx = MarkContext {
                    date: today(),
                    file: tab.session.title.clone(),
                    labels: if ranges.is_empty() {
                        Vec::new()
                    } else {
                        pagelabels::all_labels(&ranges, ids.len())
                    },
                };
                let label = if st.bates {
                    tr("Add Bates numbers")
                } else {
                    tr("Add header and footer")
                };
                match tab.session.execute(label, |tx| {
                    pagemarks::add_header_footer(tx, &pages, &spec, &mctx)
                }) {
                    Ok(n) => {
                        self.notify(tf!("Added to {} page(s).", n));
                        false
                    }
                    Err(e) => {
                        st.error = Some(Self::missing_glyphs_message(&e));
                        true
                    }
                }
            }
        }
    }

    // ---- watermark --------------------------------------------------------------------------

    /// Organize ▸ Watermark…
    pub fn open_watermark(&mut self) {
        self.dialog = Some(Dialog::Watermark(Box::default()));
    }

    pub fn dialog_watermark(&mut self, ctx: &egui::Context, st: &mut WatermarkState) -> bool {
        let mut choice: Option<bool> = None;
        let mut choose = false;
        let dim = self.pal.text_dim;
        let danger = self.pal.danger;
        modal(ctx, "watermark", |ui| {
            ui.set_max_width(520.0);
            ui.heading(tr("Watermark"));
            ui.label(
                RichText::new(tr("A text or a picture across the pages. It becomes part of the pages; Remove headers, footers and watermarks takes it off again."))
                    .size(12.0)
                    .color(dim),
            );
            ui.horizontal(|ui| {
                ui.radio_value(&mut st.picture_mode, false, tr("Text"));
                ui.radio_value(&mut st.picture_mode, true, tr("Picture"));
            });
            if st.picture_mode {
                ui.horizontal(|ui| {
                    if ui.button(tr("Choose picture…")).clicked() {
                        choose = true;
                    }
                    if let Some((_, name)) = &st.picture {
                        ui.label(name);
                    }
                });
                crate::ui_kit::form_row(ui, tr("Width"), |ui| {
                    ui.add(egui::Slider::new(&mut st.width_pct, 5.0..=100.0).suffix(" %"));
                });
            } else {
                ui.add(
                    crate::ui_kit::multiline(&mut st.text)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
                crate::ui_kit::form_row(ui, tr("Font"), |ui| {
                    crate::fontpick::font_picker(ui, "wm_font", &mut st.font);
                });
                crate::ui_kit::form_row(ui, tr("Size"), |ui| {
                    ui.checkbox(&mut st.auto_size, tr("Fit the page"));
                    ui.add_enabled(
                        !st.auto_size,
                        egui::DragValue::new(&mut st.size)
                            .range(6.0..=300.0)
                            .suffix(" pt"),
                    );
                    ui.color_edit_button_rgb(&mut st.color);
                });
            }
            crate::ui_kit::form_row(ui, tr("Opacity"), |ui| {
                ui.add(egui::Slider::new(&mut st.opacity_pct, 5.0..=100.0).suffix(" %"));
            });
            crate::ui_kit::form_row(ui, tr("Angle"), |ui| {
                ui.add(egui::Slider::new(&mut st.rotation, -90.0..=90.0).suffix("°"));
            });
            crate::ui_kit::form_row(ui, tr("Place"), |ui| {
                ui.radio_value(&mut st.behind, false, tr("On top of the page"));
                ui.radio_value(&mut st.behind, true, tr("Behind the page"));
            });
            if st.behind {
                ui.label(
                    RichText::new(tr("Behind the page it is hidden by pages that have a white background, such as most scans."))
                        .size(12.0)
                        .color(dim),
                );
            }
            st.pages.ui(ui);
            if let Some(e) = &st.error {
                ui.colored_label(danger, e);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if crate::ui_kit::primary_button(ui, tr("Add")).clicked() {
                    choice = Some(true);
                }
                if ui.button(tr("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    choice = Some(false);
                }
            });
        });
        if choose && let Some(p) = platform::dialogs::pick_image() {
            match std::fs::read(&p) {
                Ok(bytes) => {
                    let name = p
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    st.picture = Some((bytes, name));
                    st.error = None;
                }
                Err(e) => st.error = Some(tf!("Could not read the picture: {}", e)),
            }
        }
        match choice {
            None => true,
            Some(false) => false,
            Some(true) => {
                let content = if st.picture_mode {
                    let Some((data, _)) = &st.picture else {
                        st.error = Some(tr("Choose a picture first.").into());
                        return true;
                    };
                    WatermarkContent::Image {
                        data: data.clone(),
                        width: st.width_pct / 100.0,
                    }
                } else {
                    WatermarkContent::Text {
                        text: st.text.clone(),
                        font: st.font,
                        size: if st.auto_size { 0.0 } else { st.size },
                        color: color_tuple(st.color),
                    }
                };
                let spec = Watermark {
                    content,
                    opacity: st.opacity_pct / 100.0,
                    rotation: st.rotation,
                    behind: st.behind,
                };
                let Some((ids, current)) = self.page_ids_and_current() else {
                    return false;
                };
                let idx = match st.pages.resolve(current, ids.len()) {
                    Ok(v) => v,
                    Err(e) => {
                        st.error = Some(e);
                        return true;
                    }
                };
                let pages: Vec<PageId> = idx.iter().filter_map(|i| ids.get(*i).copied()).collect();
                match self.tabs[self.active]
                    .session
                    .execute(tr("Add watermark"), |tx| {
                        pagemarks::add_watermark(tx, &pages, &spec)
                    }) {
                    Ok(n) => {
                        self.notify(tf!("Added to {} page(s).", n));
                        false
                    }
                    Err(e) => {
                        st.error = Some(Self::missing_glyphs_message(&e));
                        true
                    }
                }
            }
        }
    }

    // ---- removing marks ---------------------------------------------------------------------

    /// Organize ▸ Remove Headers, Footers and Watermarks…
    pub fn open_remove_marks(&mut self) {
        let Some(tab) = self.active_tab() else { return };
        let d = tab.session.doc().lopdf();
        let (has_hf, has_wm) = (
            pagemarks::has_marks(d, MarkKind::HeaderFooter),
            pagemarks::has_marks(d, MarkKind::Watermark),
        );
        if !has_hf && !has_wm {
            self.notify(tr(
                "This document has no headers, footers or watermarks added with BergPDF.",
            ));
            return;
        }
        self.dialog = Some(Dialog::RemoveMarks(RemoveMarksState {
            header_footer: has_hf,
            watermark: has_wm,
            has_hf,
            has_wm,
        }));
    }

    pub fn dialog_remove_marks(&mut self, ctx: &egui::Context, st: &mut RemoveMarksState) -> bool {
        let mut choice: Option<bool> = None;
        modal(ctx, "remove_marks", |ui| {
            ui.heading(tr("Remove headers, footers and watermarks"));
            ui.label(
                RichText::new(tr("Only what was added with BergPDF can be found and removed; text that is part of the original pages stays."))
                    .size(12.0)
                    .color(self.pal.text_dim),
            );
            ui.add_enabled(
                st.has_hf,
                egui::Checkbox::new(
                    &mut st.header_footer,
                    tr("Headers, footers and Bates numbers"),
                ),
            );
            ui.add_enabled(
                st.has_wm,
                egui::Checkbox::new(&mut st.watermark, tr("Watermarks")),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if crate::ui_kit::primary_enabled(
                    ui,
                    st.header_footer || st.watermark,
                    tr("Remove"),
                )
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
        match choice {
            None => true,
            Some(false) => false,
            Some(true) => {
                let (hf, wm) = (st.header_footer, st.watermark);
                let r = self.tabs[self.active].session.execute(
                    tr("Remove headers, footers and watermarks"),
                    |tx| {
                        let mut n = 0;
                        if hf {
                            n += pagemarks::remove_marks(tx, MarkKind::HeaderFooter)?;
                        }
                        if wm {
                            n += pagemarks::remove_marks(tx, MarkKind::Watermark)?;
                        }
                        Ok(n)
                    },
                );
                match r {
                    Ok(_) => self.notify(tr("Removed.")),
                    Err(e) => self.notify_error(e.to_string()),
                }
                false
            }
        }
    }

    // ---- page labels ------------------------------------------------------------------------

    /// Organize ▸ Page Labels…
    pub fn open_page_labels(&mut self) {
        let Some(tab) = self.active_tab() else { return };
        let doc = tab.session.doc();
        let total = doc.page_count();
        let current = tab.session.view.current_page;
        let mut rows: Vec<LabelRow> = pagelabels::read_page_labels(doc.lopdf())
            .into_iter()
            .map(|r| LabelRow {
                from: r.start + 1,
                style: r.style,
                prefix: r.prefix,
                first: r.first,
            })
            .collect();
        if rows.is_empty() {
            rows.push(LabelRow {
                from: 1,
                style: LabelStyle::Decimal,
                prefix: String::new(),
                first: 1,
            });
            if current > 0 {
                rows.push(LabelRow {
                    from: current + 1,
                    style: LabelStyle::Decimal,
                    prefix: String::new(),
                    first: 1,
                });
                rows[0].style = LabelStyle::RomanLower;
            }
        }
        self.dialog = Some(Dialog::PageLabels(Box::new(LabelsState {
            rows,
            total,
            error: None,
        })));
    }

    fn label_ranges(rows: &[LabelRow]) -> Vec<LabelRange> {
        rows.iter()
            .map(|r| LabelRange {
                start: r.from.saturating_sub(1),
                style: r.style,
                prefix: r.prefix.clone(),
                first: r.first.max(1),
            })
            .collect()
    }

    pub fn dialog_page_labels(&mut self, ctx: &egui::Context, st: &mut LabelsState) -> bool {
        let mut choice: Option<u8> = None;
        let dim = self.pal.text_dim;
        modal(ctx, "page_labels", |ui| {
            ui.set_max_width(560.0);
            ui.heading(tr("Page labels"));
            ui.label(
                RichText::new(tr("Readers show these instead of the plain page number, for example i–iv for a preface and 1, 2, 3 after it. The pages themselves do not change."))
                    .size(12.0)
                    .color(dim),
            );
            let mut remove: Option<usize> = None;
            egui::Grid::new("label_rows")
                .num_columns(5)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    ui.label(RichText::new(tr("From page")).color(dim));
                    ui.label(RichText::new(tr("Style")).color(dim));
                    ui.label(RichText::new(tr("Prefix")).color(dim));
                    ui.label(RichText::new(tr("Start at")).color(dim));
                    ui.label("");
                    ui.end_row();
                    for (i, r) in st.rows.iter_mut().enumerate() {
                        if i == 0 {
                            r.from = 1;
                            ui.label("1");
                        } else {
                            ui.add(egui::DragValue::new(&mut r.from).range(2..=st.total.max(2)));
                        }
                        egui::ComboBox::from_id_salt(("label_style", i))
                            .selected_text(tr(r.style.title()))
                            .show_ui(ui, |ui| {
                                for s in LabelStyle::ALL {
                                    ui.selectable_value(&mut r.style, s, tr(s.title()));
                                }
                            });
                        ui.add(crate::ui_kit::singleline(&mut r.prefix).desired_width(80.0));
                        ui.add_enabled(
                            r.style != LabelStyle::None,
                            egui::DragValue::new(&mut r.first).range(1..=99_999),
                        );
                        if i > 0
                            && ui
                                .small_button("✕")
                                .on_hover_text(tr("Remove this range"))
                                .clicked()
                        {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                });
            if let Some(i) = remove {
                st.rows.remove(i);
            }
            if ui.button(tr("Add range")).clicked() {
                let from = st
                    .rows
                    .last()
                    .map_or(1, |r| r.from + 1)
                    .min(st.total.max(1));
                st.rows.push(LabelRow {
                    from,
                    style: LabelStyle::Decimal,
                    prefix: String::new(),
                    first: 1,
                });
            }
            let mut ranges = Self::label_ranges(&st.rows);
            ranges.sort_by_key(|r| r.start);
            let preview: Vec<String> = pagelabels::all_labels(&ranges, st.total.min(12));
            ui.label(
                RichText::new(tf!(
                    "Pages: {}{}",
                    preview.join(", "),
                    if st.total > 12 { ", …" } else { "" }
                ))
                .size(12.0)
                .color(dim),
            );
            if let Some(e) = &st.error {
                ui.colored_label(self.pal.danger, e);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if crate::ui_kit::primary_button(ui, "OK").clicked() {
                    choice = Some(1);
                }
                if ui
                    .button(tr("Remove labels"))
                    .on_hover_text(tr("Readers show 1, 2, 3 again"))
                    .clicked()
                {
                    choice = Some(2);
                }
                if ui.button(tr("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    choice = Some(0);
                }
            });
        });
        let ranges = match choice {
            None => return true,
            Some(0) => return false,
            Some(1) => Self::label_ranges(&st.rows),
            Some(_) => Vec::new(),
        };
        let total = st.total;
        match self.tabs[self.active]
            .session
            .execute(tr("Change page labels"), |tx| {
                pagelabels::set_page_labels(tx, &ranges, total)
            }) {
            Ok(()) => false,
            Err(e) => {
                st.error = Some(e.to_string());
                true
            }
        }
    }

    /// The label shown for page `index` of the active document, when the document has labels.
    pub fn page_label(&mut self, index: usize) -> Option<String> {
        let tab = self.tabs.get_mut(self.active)?;
        let rev = tab.session.revision();
        if tab.ui.labels.as_ref().is_none_or(|(r, _)| *r != rev) {
            let ranges = pagelabels::read_page_labels(tab.session.doc().lopdf());
            tab.ui.labels = Some((rev, std::sync::Arc::new(ranges)));
        }
        let ranges = tab.ui.labels.as_ref()?.1.clone();
        (!ranges.is_empty()).then(|| pagelabels::label_for(&ranges, index))
    }

    // ---- crop -------------------------------------------------------------------------------

    /// Organize ▸ Crop Pages…
    pub fn open_crop(&mut self, margins_pt: Option<[f64; 4]>) {
        let m = margins_pt.unwrap_or([0.0; 4]);
        self.dialog = Some(Dialog::Crop(Box::new(CropState {
            left: m[0] / PT_PER_MM,
            right: m[1] / PT_PER_MM,
            top: m[2] / PT_PER_MM,
            bottom: m[3] / PT_PER_MM,
            pages: PageChoice::new(if margins_pt.is_some() {
                PageScope::Current
            } else {
                PageScope::All
            }),
            error: None,
        })));
    }

    /// The crop box a page gets for margins (left, right, top, bottom in points on the page as shown).
    fn cropped_box(geometry: &pdf_engine::geom::PageGeometry, m: [f64; 4]) -> Option<PRect> {
        let shown = geometry.view_size(Rotation::R0);
        let (x0, y0) = (m[0], m[2]);
        let (x1, y1) = (shown.width - m[1], shown.height - m[3]);
        if x1 - x0 < 10.0 || y1 - y0 < 10.0 {
            return None;
        }
        let t = geometry.view_to_pdf(Rotation::R0);
        let a = t * Point::new(x0, y0);
        let b = t * Point::new(x1, y1);
        Some(PRect::new(a.x, a.y, b.x, b.y).abs())
    }

    pub fn dialog_crop(&mut self, ctx: &egui::Context, st: &mut CropState) -> bool {
        let mut choice: Option<u8> = None;
        let dim = self.pal.text_dim;
        // Anchored to the side, so the preview on the page stays visible.
        egui::Window::new(tr("Crop pages"))
            .id(egui::Id::new("crop_window"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-24.0, 120.0))
            .frame(crate::dialogs::dialog_frame(ctx))
            .show(ctx, |ui| {
                ui.set_max_width(380.0);
                ui.label(
                    RichText::new(tr("Margins to cut away, on the page as you see it. The blue frame shows what stays. Nothing is deleted: Remove crop shows the whole page again."))
                        .size(12.0)
                        .color(dim),
                );
                egui::Grid::new("crop_grid").num_columns(4).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label(tr("Left"));
                    mm_field(ui, &mut st.left);
                    ui.label(tr("Right"));
                    mm_field(ui, &mut st.right);
                    ui.end_row();
                    ui.label(tr("Top"));
                    mm_field(ui, &mut st.top);
                    ui.label(tr("Bottom"));
                    mm_field(ui, &mut st.bottom);
                    ui.end_row();
                });
                st.pages.ui(ui);
                if let Some(e) = &st.error {
                    ui.colored_label(self.pal.danger, e);
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if crate::ui_kit::primary_button(ui, tr("Crop")).clicked() {
                        choice = Some(1);
                    }
                    if ui.button(tr("Remove crop")).clicked() {
                        choice = Some(2);
                    }
                    if ui.button(tr("Cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        choice = Some(0);
                    }
                });
            });
        let remove = match choice {
            None => return true,
            Some(0) => return false,
            Some(1) => false,
            Some(_) => true,
        };
        let Some((ids, current)) = self.page_ids_and_current() else {
            return false;
        };
        let idx = match st.pages.resolve(current, ids.len()) {
            Ok(v) => v,
            Err(e) => {
                st.error = Some(e);
                return true;
            }
        };
        let m = [st.left, st.right, st.top, st.bottom].map(|v| v.max(0.0) * PT_PER_MM);
        let tab = &mut self.tabs[self.active];
        let mut boxes: Vec<(PageId, Option<PRect>)> = Vec::new();
        for i in &idx {
            let p = ids[*i];
            if remove {
                boxes.push((p, None));
                continue;
            }
            let Ok(g) = tab.session.doc().page_geometry(p) else {
                continue;
            };
            match Self::cropped_box(&g, m) {
                Some(b) => boxes.push((p, Some(b))),
                None => {
                    st.error = Some(tf!("The margins leave nothing of page {}.", i + 1));
                    return true;
                }
            }
        }
        let label = if remove {
            tr("Remove crop")
        } else {
            tr("Crop pages")
        };
        let r = tab.session.execute(label, |tx| {
            for (p, b) in &boxes {
                pageops::set_crop_box(tx, *p, *b)?;
            }
            Ok(boxes.len())
        });
        match r {
            Ok(n) => {
                self.notify(tf!("Changed {} page(s).", n));
                false
            }
            Err(e) => {
                st.error = Some(e.to_string());
                true
            }
        }
    }

    /// Crop tool: drag the area to keep, then confirm in the crop dialog.
    pub fn crop_tool(&mut self, response: &egui::Response, vc: &ViewCtx, pos: Option<Pos2>) {
        let ti = self.active;
        let Some(pos) = pos else { return };
        if response.drag_started_by(egui::PointerButton::Primary) {
            let Some(i) = vc.page_at(pos) else { return };
            let p = vc.screen_to_pdf(i, pos);
            self.tabs[ti].ui.interaction = Interaction::Draw {
                page: vc.pages[i].id,
                tool: editor_core::tools::Tool::Crop,
                points: vec![p, p],
            };
        } else if response.dragged_by(egui::PointerButton::Primary)
            && let Interaction::Draw { points, page, .. } = &mut self.tabs[ti].ui.interaction
            && let Some(i) = vc.pages.iter().position(|p| p.id == *page)
        {
            points.truncate(1);
            points.push(vc.screen_to_pdf(i, pos));
        } else if response.drag_stopped()
            && let Interaction::Draw { page, points, .. } =
                std::mem::take(&mut self.tabs[ti].ui.interaction)
            && points.len() == 2
        {
            let Ok(g) = self.tabs[ti].session.doc().page_geometry(page) else {
                return;
            };
            let t = g.pdf_to_view(Rotation::R0);
            let a = t * points[0];
            let b = t * points[1];
            let r = PRect::new(a.x, a.y, b.x, b.y).abs();
            if r.width() < 10.0 || r.height() < 10.0 {
                return;
            }
            let shown = g.view_size(Rotation::R0);
            let margins = [
                r.x0.max(0.0),
                (shown.width - r.x1).max(0.0),
                r.y0.max(0.0),
                (shown.height - r.y1).max(0.0),
            ];
            if let Some(i) = vc.pages.iter().position(|p| p.id == page) {
                self.tabs[ti].session.view.current_page = i;
            }
            self.set_tool(editor_core::tools::Tool::Select);
            self.open_crop(Some(margins));
        }
    }

    /// While the crop dialog is open: dim what will be cut away on the current page.
    pub fn paint_crop_preview(&mut self, painter: &egui::Painter, vc: &ViewCtx) {
        let Some(Dialog::Crop(st)) = &self.dialog else {
            return;
        };
        let m = [st.left, st.right, st.top, st.bottom].map(|v| v.max(0.0) * PT_PER_MM);
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let i = tab.session.view.current_page;
        let Some(page) = vc.pages.get(i) else { return };
        let Ok(g) = tab.session.doc().page_geometry(page.id) else {
            return;
        };
        let Some(keep) = Self::cropped_box(&g, m) else {
            return;
        };
        let full = Self::rect_screen(vc, i, g.crop_box);
        let k = Self::rect_screen(vc, i, keep);
        let shade = Color32::from_rgba_unmultiplied(0, 0, 0, 90);
        for r in [
            egui::Rect::from_min_max(full.min, Pos2::new(full.max.x, k.min.y)),
            egui::Rect::from_min_max(Pos2::new(full.min.x, k.max.y), full.max),
            egui::Rect::from_min_max(Pos2::new(full.min.x, k.min.y), Pos2::new(k.min.x, k.max.y)),
            egui::Rect::from_min_max(Pos2::new(k.max.x, k.min.y), Pos2::new(full.max.x, k.max.y)),
        ] {
            painter.rect_filled(r, 0.0, shade);
        }
        painter.rect_stroke(
            k,
            0.0,
            Stroke::new(2.0, Color32::from_rgb(40, 120, 240)),
            egui::StrokeKind::Middle,
        );
    }
}
