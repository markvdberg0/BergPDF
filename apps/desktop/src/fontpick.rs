//! The font chooser shared by Add Text, text boxes, Edit Text and Preferences.
//!
//! The bundled fonts are also registered with egui, so every entry is drawn in its own face and
//! the text box in the Add Text dialog previews what will end up on the page.

use egui::{FontFamily, RichText, Ui};
use pdf_engine::fontembed::{FontFamily as Family, FontStyle};

/// The egui family under which a bundled face is registered.
pub fn egui_family(style: FontStyle) -> FontFamily {
    FontFamily::Name(style.base_name().into())
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

/// Family menu plus Bold and Italic toggles. Returns whether anything changed.
pub fn font_picker(ui: &mut Ui, id: &str, style: &mut FontStyle) -> bool {
    let mut fam = style.family;
    let (mut bold, mut italic) = (style.bold, style.italic);
    egui::ComboBox::from_id_salt(id)
        .selected_text(
            RichText::new(fam.title()).family(egui_family(FontStyle::new(fam, false, false))),
        )
        .width(210.0)
        .show_ui(ui, |ui| {
            for f in Family::ALL {
                ui.selectable_value(
                    &mut fam,
                    f,
                    RichText::new(f.title()).family(egui_family(FontStyle::new(f, false, false))),
                );
            }
        });
    ui.checkbox(&mut bold, RichText::new("Bold").strong());
    ui.add_enabled(
        fam.has_italic(),
        egui::Checkbox::new(&mut italic, RichText::new("Italic").italics()),
    )
    .on_disabled_hover_text("This font has no italic style");
    let new = FontStyle::new(fam, bold, italic);
    let changed = new != *style;
    *style = new;
    changed
}

/// Just the Bold and Italic toggles (the family is chosen elsewhere).
pub fn font_picker_toggles(ui: &mut Ui, style: &mut FontStyle) -> bool {
    let (mut bold, mut italic) = (style.bold, style.italic);
    ui.checkbox(&mut bold, RichText::new("Bold").strong());
    ui.add_enabled(
        style.family.has_italic(),
        egui::Checkbox::new(&mut italic, RichText::new("Italic").italics()),
    )
    .on_disabled_hover_text("This font has no italic style");
    let new = FontStyle::new(style.family, bold, italic);
    let changed = new != *style;
    *style = new;
    changed
}
