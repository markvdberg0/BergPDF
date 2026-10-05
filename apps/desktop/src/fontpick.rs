//! The font chooser shared by Add Text, text boxes, Edit Text and Preferences.
//!
//! The bundled fonts are also registered with egui, so every entry is drawn in its own face and
//! the text box in the Add Text dialog previews what will end up on the page.

use egui::{FontFamily, RichText, Ui};
use pdf_engine::fontembed::{FontFamily as Family, FontStyle};

use crate::i18n::tr;
use crate::tf;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

/// Names of the system faces already handed to egui.
fn registered() -> &'static Mutex<HashSet<String>> {
    static R: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashSet::new()))
}

/// System faces that were asked for but are not registered with egui yet.
fn pending() -> &'static Mutex<Vec<FontStyle>> {
    static P: OnceLock<Mutex<Vec<FontStyle>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(Vec::new()))
}

static CTX: OnceLock<egui::Context> = OnceLock::new();

/// Remember the egui context so [`egui_family`] can ask for a repaint after queueing a font.
pub fn init(ctx: &egui::Context) {
    let _ = CTX.set(ctx.clone());
}

/// The egui family under which a face is registered. A system font is registered lazily (the file
/// is only read once it is actually shown); until then the interface font stands in.
pub fn egui_family(style: FontStyle) -> FontFamily {
    if matches!(style.family, Family::System(_)) {
        let name = style.base_name();
        let known = registered().lock().is_ok_and(|r| r.contains(&name));
        if !known {
            if let Ok(mut p) = pending().lock()
                && !p.contains(&style)
            {
                p.push(style);
            }
            if let Some(c) = CTX.get() {
                c.request_repaint();
            }
            return FontFamily::Proportional;
        }
    }
    FontFamily::Name(style.base_name().into())
}

/// Fonts given to egui that become usable only at the start of the next pass.
fn awaiting() -> &'static Mutex<Vec<String>> {
    static A: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    A.get_or_init(|| Mutex::new(Vec::new()))
}

/// Hand queued system fonts to egui (call once per frame, before drawing). A font only counts as
/// registered one frame after egui was given it, so no widget asks for a family egui lacks.
pub fn flush_pending(ctx: &egui::Context) {
    if let (Ok(mut a), Ok(mut r)) = (awaiting().lock(), registered().lock()) {
        r.extend(a.drain(..));
    }
    let Ok(mut p) = pending().lock() else { return };
    let mut gave = false;
    for style in p.drain(..) {
        let name = style.base_name();
        if registered().lock().is_ok_and(|r| r.contains(&name))
            || awaiting().lock().is_ok_and(|a| a.contains(&name))
        {
            continue;
        }
        let mut data = egui::FontData::from_static(style.data());
        data.index = style.face_index();
        ctx.add_font(egui::epaint::text::FontInsert::new(
            &name,
            data,
            vec![egui::epaint::text::InsertFontFamily {
                family: FontFamily::Name(name.clone().into()),
                priority: egui::epaint::text::FontPriority::Highest,
            }],
        ));
        if let Ok(mut a) = awaiting().lock() {
            a.push(name);
        }
        gave = true;
    }
    if gave {
        ctx.request_repaint();
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}

/// Register every bundled face with egui (called once at start-up).
pub fn register(fonts: &mut egui::FontDefinitions) {
    let mut seen: Vec<String> = Vec::new();
    for f in Family::ALL {
        for bold in [false, true] {
            for italic in [false, true] {
                let s = FontStyle::new(f, bold, italic);
                let name = s.base_name();
                if seen.contains(&name) {
                    continue;
                }
                seen.push(name.clone());
                fonts.font_data.insert(
                    name.clone(),
                    std::sync::Arc::new(egui::FontData::from_static(s.data())),
                );
                // The face first, the interface fonts behind it for any symbol it lacks.
                let chain = fonts
                    .families
                    .entry(FontFamily::Name(name.clone().into()))
                    .or_default();
                chain.push(name);
                chain.push("dejavu".into());
            }
        }
    }
}

/// The family list shared by the pickers: the bundled families (drawn in their own face), then
/// a searchable list of installed fonts. Returns whether `fam` changed.
pub fn family_menu(ui: &mut Ui, fam: &mut Family, search_id: egui::Id) -> bool {
    let before = *fam;
    for f in Family::ALL {
        ui.selectable_value(
            fam,
            f,
            RichText::new(f.title()).family(egui_family(FontStyle::new(f, false, false))),
        );
    }
    ui.separator();
    let installed = pdf_engine::sysfonts::list();
    if installed.is_empty() {
        ui.label(
            RichText::new(tr("Installed fonts: none found (or still scanning)"))
                .size(11.0)
                .weak(),
        );
        return *fam != before;
    }
    ui.label(
        RichText::new(tf!("Installed fonts ({})", installed.len()))
            .size(11.0)
            .weak(),
    );
    let mut filter: String = ui.data_mut(|d| d.get_temp(search_id).unwrap_or_default());
    ui.add(
        egui::TextEdit::singleline(&mut filter)
            .hint_text(tr("Search installed fonts"))
            .desired_width(200.0),
    );
    ui.data_mut(|d| d.insert_temp(search_id, filter.clone()));
    let f = filter.to_lowercase();
    let rows: Vec<(u32, &str)> = installed
        .into_iter()
        .filter(|(_, n)| f.is_empty() || n.to_lowercase().contains(&f))
        .collect();
    let row_h = ui.text_style_height(&egui::TextStyle::Button) + 4.0;
    egui::ScrollArea::vertical()
        .max_height(240.0)
        .auto_shrink([false, true])
        .show_rows(ui, row_h, rows.len(), |ui, range| {
            for (id, name) in &rows[range] {
                ui.selectable_value(fam, Family::System(*id), *name);
            }
        });
    *fam != before
}

/// Family menu plus Bold and Italic toggles. Returns whether anything changed.
pub fn font_picker(ui: &mut Ui, id: &str, style: &mut FontStyle) -> bool {
    let mut fam = style.family;
    let (mut bold, mut italic) = (style.bold, style.italic);
    let search_id = ui.id().with(id).with("search");
    egui::ComboBox::from_id_salt(id)
        .selected_text(
            RichText::new(fam.title()).family(egui_family(FontStyle::new(fam, false, false))),
        )
        .width(210.0)
        .height(470.0)
        .show_ui(ui, |ui| {
            family_menu(ui, &mut fam, search_id);
        });
    ui.add_enabled(
        fam.has_bold(),
        egui::Checkbox::new(&mut bold, RichText::new(tr("Bold")).strong()),
    )
    .on_disabled_hover_text(tr("This font has no bold style"));
    ui.add_enabled(
        fam.has_italic(),
        egui::Checkbox::new(&mut italic, RichText::new(tr("Italic")).italics()),
    )
    .on_disabled_hover_text(tr("This font has no italic style"));
    let new = FontStyle::new(fam, bold && fam.has_bold(), italic);
    let changed = new != *style;
    *style = new;
    changed
}

/// Just the Bold and Italic toggles (the family is chosen elsewhere).
pub fn font_picker_toggles(ui: &mut Ui, style: &mut FontStyle) -> bool {
    let (mut bold, mut italic) = (style.bold, style.italic);
    ui.add_enabled(
        style.family.has_bold(),
        egui::Checkbox::new(&mut bold, RichText::new(tr("Bold")).strong()),
    )
    .on_disabled_hover_text(tr("This font has no bold style"));
    ui.add_enabled(
        style.family.has_italic(),
        egui::Checkbox::new(&mut italic, RichText::new(tr("Italic")).italics()),
    )
    .on_disabled_hover_text(tr("This font has no italic style"));
    let new = FontStyle::new(style.family, bold && style.family.has_bold(), italic);
    let changed = new != *style;
    *style = new;
    changed
}
