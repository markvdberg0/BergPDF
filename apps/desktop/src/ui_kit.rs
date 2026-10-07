//! Small building blocks that give the whole interface one look: inputs with proper padding, filled primary and
//! quiet secondary buttons, a slider with a value field, section headings, cards and icon buttons.
//!
//! They read their colours from the egui visuals set up in `theme.rs` (accent = `selection.stroke`, card fill =
//! `faint_bg_color`), so they follow light and dark mode without being handed a palette.

use egui::{
    Align, Color32, CornerRadius, Layout, Margin, Response, RichText, Sense, Stroke, StrokeKind,
    TextEdit, Ui, Vec2, epaint::PathShape, text_edit::TextBuffer,
};

/// Padding inside every text field.
pub const INPUT_MARGIN: Margin = Margin {
    left: 10,
    right: 10,
    top: 7,
    bottom: 7,
};

/// Height of buttons and fields.
pub const CONTROL_H: f32 = 32.0;

/// The accent colour of the current theme.
pub fn accent(ui: &Ui) -> Color32 {
    ui.visuals().selection.stroke.color
}

/// Text that is present but secondary.
pub fn dim(ui: &Ui) -> Color32 {
    ui.visuals().weak_text_color()
}

/// A single-line text field with comfortable padding.
pub fn singleline(text: &mut dyn TextBuffer) -> TextEdit<'_> {
    TextEdit::singleline(text)
        .margin(INPUT_MARGIN)
        .min_size(Vec2::new(0.0, CONTROL_H))
}

/// A multi-line text field with comfortable padding.
pub fn multiline(text: &mut dyn TextBuffer) -> TextEdit<'_> {
    TextEdit::multiline(text).margin(INPUT_MARGIN)
}

/// The text colour that reads on the accent colour.
fn on_accent(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(0x1B, 0x1A, 0x18)
    } else {
        Color32::WHITE
    }
}

/// The main action of a dialog or form: filled with the accent colour.
pub fn primary_button(ui: &mut Ui, text: impl Into<String>) -> Response {
    let fill = accent(ui);
    let text = RichText::new(text.into()).strong().color(on_accent(ui));
    ui.add(
        egui::Button::new(text)
            .fill(fill)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(8))
            .min_size(Vec2::new(0.0, CONTROL_H)),
    )
}

/// [`primary_button`] that can be switched off.
pub fn primary_enabled(ui: &mut Ui, enabled: bool, text: impl Into<String>) -> Response {
    ui.add_enabled_ui(enabled, |ui| primary_button(ui, text))
        .inner
}

/// A destructive action: red text on the quiet button.
pub fn danger_button(ui: &mut Ui, text: impl Into<String>) -> Response {
    let c = ui.visuals().error_fg_color;
    ui.add(
        egui::Button::new(RichText::new(text.into()).color(c))
            .corner_radius(CornerRadius::same(8))
            .min_size(Vec2::new(0.0, CONTROL_H)),
    )
}

/// A rounded panel that groups related controls.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let fill = ui.visuals().faint_bg_color;
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A label on the left and a control that takes the rest of the row.
pub fn form_row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let w = 88.0;
        ui.allocate_ui_with_layout(
            Vec2::new(w, CONTROL_H),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.add(
                    egui::Label::new(RichText::new(label).color(dim(ui)).size(13.0))
                        .truncate()
                        .selectable(false),
                );
            },
        );
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.set_min_height(CONTROL_H);
            add(ui);
        });
    });
}

/// A slider across the whole width with a value field beside it. Returns whether the value changed.
pub fn slider(
    ui: &mut Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
) -> bool {
    let (lo, hi) = (*range.start(), *range.end());
    // Wide enough for the longest value ("100.0%", "12.5 pt") with the narrower padding set below; a field that
    // needs more room than it is given would push the row wider every frame.
    let chip_w = 80.0;
    let mut changed = false;
    ui.horizontal(|ui| {
        let rail_w = (ui.available_width() - chip_w - ui.spacing().item_spacing.x).max(40.0);
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(rail_w, CONTROL_H), Sense::click_and_drag());
        if let Some(p) = resp.interact_pointer_pos()
            && (resp.dragged() || resp.clicked())
        {
            let t = ((p.x - rect.left() - 8.0) / (rect.width() - 16.0)).clamp(0.0, 1.0);
            let v = lo + t * (hi - lo);
            if (v - *value).abs() > f32::EPSILON {
                *value = v;
                changed = true;
            }
        }
        let t = ((*value - lo) / (hi - lo).max(f32::EPSILON)).clamp(0.0, 1.0);
        let x = rect.left() + 8.0 + t * (rect.width() - 16.0);
        let y = rect.center().y;
        let accent = accent(ui);
        let rail = ui.visuals().widgets.inactive.bg_fill;
        let painter = ui.painter();
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(rect.left() + 4.0, y - 2.0),
                egui::pos2(rect.right() - 4.0, y + 2.0),
            ),
            2.0,
            rail,
        );
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(rect.left() + 4.0, y - 2.0),
                egui::pos2(x, y + 2.0),
            ),
            2.0,
            accent,
        );
        let hot = resp.hovered() || resp.dragged();
        painter.circle_filled(
            egui::pos2(x, y),
            if hot { 9.0 } else { 8.0 },
            Color32::WHITE,
        );
        painter.circle_stroke(
            egui::pos2(x, y),
            if hot { 9.0 } else { 8.0 },
            Stroke::new(2.0, accent),
        );
        let mut v = *value;
        ui.spacing_mut().button_padding = Vec2::new(6.0, 4.0);
        let drag = egui::DragValue::new(&mut v)
            .range(lo..=hi)
            .speed((hi - lo) / 200.0)
            .suffix(suffix.to_string())
            .max_decimals(1);
        if ui.add_sized([chip_w, CONTROL_H], drag).changed() {
            *value = v;
            changed = true;
        }
    });
    changed
}

/// [`slider`] for an `f64`.
pub fn slider64(
    ui: &mut Ui,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    suffix: &str,
) -> bool {
    let mut v = *value as f32;
    let changed = slider(
        ui,
        &mut v,
        (*range.start() as f32)..=(*range.end() as f32),
        suffix,
    );
    if changed {
        *value = f64::from(v);
    }
    changed
}

/// A round-cornered square button with a chevron: hides or shows a side panel.
pub fn chevron_button(ui: &mut Ui, pointing_left: bool, tip: &str) -> Response {
    let size = Vec2::splat(28.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hovered = resp.hovered();
    let bg = if hovered {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        ui.visuals().widgets.inactive.bg_fill
    };
    let ink = ui.visuals().text_color();
    let p = ui.painter();
    p.rect(
        rect,
        CornerRadius::same(8),
        bg,
        Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        StrokeKind::Inside,
    );
    let c = rect.center();
    let d = if pointing_left { -1.0 } else { 1.0 };
    p.add(PathShape::line(
        vec![
            egui::pos2(c.x - 2.5 * d, c.y - 5.0),
            egui::pos2(c.x + 2.5 * d, c.y),
            egui::pos2(c.x - 2.5 * d, c.y + 5.0),
        ],
        Stroke::new(1.8, ink),
    ));
    resp.on_hover_text(tip)
}

/// A tab of a side panel: an icon, and the name next to it on the selected tab only, so any number of tabs fit
/// in a narrow panel. Hovering a tab shows its name.
pub fn icon_tab(
    ui: &mut Ui,
    icon: crate::icons::Icon,
    label: &str,
    selected: bool,
    show_label: bool,
) -> Response {
    let accent = accent(ui);
    let soft = ui.visuals().selection.bg_fill;
    let ink = if selected {
        accent
    } else {
        ui.visuals().text_color()
    };
    let galley = (selected && show_label).then(|| {
        ui.painter()
            .layout_no_wrap(label.to_string(), egui::FontId::proportional(13.5), ink)
    });
    let icon_s = 18.0;
    let w = 12.0 + icon_s + galley.as_ref().map_or(0.0, |g| 7.0 + g.size().x) + 12.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 32.0), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, CornerRadius::same(8), soft);
    } else if resp.hovered() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(8),
            ui.visuals().widgets.hovered.bg_fill,
        );
    }
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 12.0 + icon_s / 2.0, rect.center().y),
        Vec2::splat(icon_s),
    );
    crate::icons::paint(ui.painter(), icon_rect, icon, ink);
    if let Some(g) = galley {
        let pos = egui::pos2(icon_rect.right() + 7.0, rect.center().y - g.size().y / 2.0);
        ui.painter().galley(pos, g, ink);
    }
    resp.on_hover_text(label)
}
