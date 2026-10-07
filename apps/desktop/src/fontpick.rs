//! The font chooser shared by Add Text, text boxes, Edit Text and Preferences.
//!
//! The bundled fonts are also registered with egui, so every entry is drawn in its own face and
//! the text box in the Add Text dialog previews what will end up on the page.

use egui::{FontFamily, RichText, Ui};
use pdf_engine::fontembed::{FontFamily as Family, FontStyle};

use crate::i18n::tr;
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

/// Development aid (debug builds): open the next font menu that is drawn (see `debug_shots.rs`).
#[cfg(debug_assertions)]
pub static DEBUG_OPEN_FONT_MENU: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

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

/// The one list of fonts shared by the pickers: the built-in families and the installed ones together,
/// alphabetical, with a search field on top. Choosing a font closes the menu; clicking the search field does
/// not (the combo box that holds this list must not close on a click inside it). Returns whether `fam` changed.
pub fn family_menu(ui: &mut Ui, fam: &mut Family, search_id: egui::Id) -> bool {
    let before = *fam;
    ui.set_min_width(250.0);
    let mut filter: String = ui.data_mut(|d| d.get_temp(search_id).unwrap_or_default());
    let field = ui.add(
        crate::ui_kit::singleline(&mut filter)
            .hint_text(tr("Search fonts"))
            .desired_width(f32::INFINITY),
    );
    // The search field is ready to type in as soon as the list opens.
    if ui.memory(|m| m.focused().is_none()) {
        field.request_focus();
    }
    ui.data_mut(|d| d.insert_temp(search_id, filter.clone()));
    let needle = filter.to_lowercase();

    let mut rows: Vec<(Family, String)> = Family::ALL
        .into_iter()
        .map(|f| (f, f.title().to_string()))
        .collect();
    let built_in: HashSet<String> = rows.iter().map(|(_, n)| n.to_lowercase()).collect();
    for (id, name) in pdf_engine::sysfonts::list() {
        if !built_in.contains(&name.to_lowercase()) {
            rows.push((Family::System(id), name.to_string()));
        }
    }
    rows.retain(|(_, n)| needle.is_empty() || n.to_lowercase().contains(&needle));
    rows.sort_by_key(|(_, n)| n.to_lowercase());
    ui.add_space(4.0);
    if rows.is_empty() {
        ui.label(RichText::new(tr("No font matches")).weak());
        return false;
    }
    let row_h = ui.text_style_height(&egui::TextStyle::Button) + 10.0;
    let mut chosen = None;
    egui::ScrollArea::vertical()
        .max_height(300.0)
        .auto_shrink([false, true])
        .show_rows(ui, row_h, rows.len(), |ui, range| {
            for (f, name) in &rows[range] {
                // Drawn by hand so the name is left-aligned and shown in its own font.
                let (rect, r) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), row_h),
                    egui::Sense::click(),
                );
                let selected = *fam == *f;
                if selected {
                    ui.painter()
                        .rect_filled(rect, 6.0, ui.visuals().selection.bg_fill);
                } else if r.hovered() {
                    ui.painter()
                        .rect_filled(rect, 6.0, ui.visuals().widgets.hovered.bg_fill);
                }
                let ink = if selected {
                    ui.visuals().selection.stroke.color
                } else {
                    ui.visuals().text_color()
                };
                let galley = ui.painter().layout_no_wrap(
                    name.clone(),
                    egui::FontId::new(15.0, egui_family(FontStyle::new(*f, false, false))),
                    ink,
                );
                let pos = egui::pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0);
                ui.painter().galley(pos, galley, ink);
                if r.clicked() {
                    chosen = Some(*f);
                }
            }
        });
    if let Some(f) = chosen {
        *fam = f;
        ui.close();
    }
    *fam != before
}

/// The font chooser: one searchable list of fonts, then Bold and Italic as toggle buttons. In a narrow
/// column the toggles go under the list instead of beside it.
pub fn font_picker(ui: &mut Ui, id: &str, style: &mut FontStyle) -> bool {
    let mut fam = style.family;
    let (mut bold, mut italic) = (style.bold, style.italic);
    let search_id = ui.id().with(id).with("search");
    let narrow = ui.available_width() < 210.0;
    let (has_bold, has_italic) = (fam.has_bold(), fam.has_italic());
    let mut toggles = |ui: &mut Ui| {
        let b = ui
            .add_enabled(
                has_bold,
                egui::Button::selectable(bold, RichText::new("B").strong())
                    .frame_when_inactive(true)
                    .min_size(egui::vec2(32.0, 32.0)),
            )
            .on_hover_text(tr("Bold"))
            .on_disabled_hover_text(tr("This font has no bold style"));
        if b.clicked() {
            bold = !bold;
        }
        let i = ui
            .add_enabled(
                has_italic,
                egui::Button::selectable(italic, RichText::new("I").italics())
                    .frame_when_inactive(true)
                    .min_size(egui::vec2(32.0, 32.0)),
            )
            .on_hover_text(tr("Italic"))
            .on_disabled_hover_text(tr("This font has no italic style"));
        if i.clicked() {
            italic = !italic;
        }
    };
    let combo = |ui: &mut Ui, fam: &mut Family, width: f32| {
        #[cfg(debug_assertions)]
        if DEBUG_OPEN_FONT_MENU.load(std::sync::atomic::Ordering::Relaxed) {
            egui::Popup::open_id(ui.ctx(), ui.make_persistent_id(id).with("popup"));
        }
        egui::ComboBox::from_id_salt(id)
            .selected_text(
                RichText::new(fam.title()).family(egui_family(FontStyle::new(*fam, false, false))),
            )
            .width(width)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show_ui(ui, |ui| {
                family_menu(ui, fam, search_id);
            });
    };
    if narrow {
        ui.vertical(|ui| {
            combo(ui, &mut fam, ui.available_width());
            ui.horizontal(|ui| toggles(ui));
        });
    } else {
        let w = (ui.available_width() - 2.0 * 36.0 - 8.0).clamp(110.0, 240.0);
        combo(ui, &mut fam, w);
        toggles(ui);
    }
    let new = FontStyle::new(fam, bold && fam.has_bold(), italic && fam.has_italic());
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
