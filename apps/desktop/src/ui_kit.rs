//! Small building blocks that give the whole interface one look: inputs with proper padding, filled primary and
//! quiet secondary buttons, a slider with a value field, section headings, cards and icon buttons.
//!
//! They read their colours from the palette `theme::apply` stores in the egui context (docs/DESIGN.md §2.1, §4),
//! so they follow light and dark mode without being handed one.

use crate::theme::Palette;
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

/// The palette of the current theme (stored by `theme::apply`).
pub fn pal(ui: &Ui) -> Palette {
    pal_ctx(ui.ctx())
}

/// [`pal`] from a context.
pub fn pal_ctx(ctx: &egui::Context) -> Palette {
    ctx.data(|d| d.get_temp::<Palette>(egui::Id::new(PALETTE_ID)))
        .unwrap_or(Palette::LIGHT)
}

pub(crate) const PALETTE_ID: &str = "berg_palette";

/// The accent colour of the current theme.
pub fn accent(ui: &Ui) -> Color32 {
    pal(ui).accent
}

/// Text that is present but secondary.
pub fn dim(ui: &Ui) -> Color32 {
    pal(ui).text_dim
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

/// The main action of a dialog or form (one per view): `primary.bg`, SemiBold `primary.text`.
pub fn primary_button(ui: &mut Ui, text: impl Into<String>) -> Response {
    let p = pal(ui);
    let id = ui.next_auto_id();
    let hovered = ui.ctx().read_response(id).is_some_and(|r| r.hovered());
    let text = RichText::new(text.into())
        .font(crate::theme::font(600, 13.0))
        .color(p.on_primary);
    ui.add(
        egui::Button::new(text)
            .fill(if hovered { p.primary_hover } else { p.primary })
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(crate::theme::radius::LG))
            .min_size(Vec2::new(0.0, 34.0)),
    )
}

/// The large primary button of the start screen (42 high, 14 / SemiBold, radius 12), with an icon.
pub fn primary_large(ui: &mut Ui, icon: crate::icons::Icon, text: &str) -> Response {
    let p = pal(ui);
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        crate::theme::font(700, 14.0),
        p.on_primary,
    );
    let size = Vec2::new(18.0 + 17.0 + 8.0 + galley.size().x + 18.0, 42.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.hovered() {
        p.primary_hover
    } else {
        p.primary
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(crate::theme::radius::XXL), fill);
    let ir = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 18.0 + 8.5, rect.center().y),
        Vec2::splat(17.0),
    );
    crate::icons::paint(ui.painter(), ir, icon, p.on_primary);
    ui.painter().galley(
        egui::pos2(ir.right() + 8.0, rect.center().y - galley.size().y / 2.0),
        galley,
        p.on_primary,
    );
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    resp
}

/// [`primary_button`] that can be switched off.
pub fn primary_enabled(ui: &mut Ui, enabled: bool, text: impl Into<String>) -> Response {
    ui.add_enabled_ui(enabled, |ui| primary_button(ui, text))
        .inner
}

/// A destructive action: `danger.text` on the secondary button.
pub fn danger_button(ui: &mut Ui, text: impl Into<String>) -> Response {
    let p = pal(ui);
    ui.add(
        egui::Button::new(RichText::new(text.into()).color(p.danger))
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(crate::theme::radius::LG))
            .min_size(Vec2::new(0.0, 34.0)),
    )
}

/// A rounded panel that groups related controls.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = pal(ui);
    egui::Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(crate::theme::radius::LG))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Kind of [`note`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteKind {
    Success,
    Warning,
    Danger,
}

/// A note (docs/DESIGN.md §4.9): padding 10 12, radius 8, `small` text in the kind's colours.
pub fn note(ui: &mut Ui, kind: NoteKind, text: &str) {
    let p = pal(ui);
    let (bg, fg) = match kind {
        NoteKind::Success => (p.success_bg, p.success),
        NoteKind::Warning => (p.warning_bg, p.warning),
        NoteKind::Danger => (p.danger_bg, p.danger),
    };
    egui::Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(crate::theme::radius::LG))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(text).size(12.0).color(fg)).wrap());
        });
}

/// A keyboard-shortcut chip ("Ctrl K"): `micro`, `bg.track`, `border.default`, radius 5.
pub fn kbd(ui: &mut Ui, text: &str) -> Response {
    let p = pal(ui);
    let g = ui.painter().layout_no_wrap(
        text.to_string(),
        crate::theme::font(500, 11.0),
        p.text_muted,
    );
    let size = g.size() + Vec2::new(12.0, 2.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(crate::theme::radius::SM),
        p.track,
        Stroke::new(1.0, p.border),
        StrokeKind::Inside,
    );
    ui.painter()
        .galley(rect.center() - g.size() / 2.0, g, p.text_muted);
    resp
}

/// A switch (docs/DESIGN.md §4.11): track 28 × 16, knob 12; `accent` when on, `border.strong` when off.
pub fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    let p = pal(ui);
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(28.0, 16.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.08);
    let track = if *on { p.accent } else { p.border_strong };
    ui.painter().rect_filled(rect, CornerRadius::same(8), track);
    let x = egui::lerp((rect.left() + 8.0)..=(rect.right() - 8.0), t);
    ui.painter()
        .circle_filled(egui::pos2(x, rect.center().y), 6.0, Color32::WHITE);
    let state = *on;
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, state, ""));
    resp
}

/// A segmented control (docs/DESIGN.md §4.5): a `bg.track` track with equal segments; the selected one is
/// raised on `bg.segment`. Returns the chosen value when it changed.
pub fn segmented<T: Copy + PartialEq>(ui: &mut Ui, current: &mut T, options: &[(T, &str)]) -> bool {
    let p = pal(ui);
    let w = ui.available_width();
    let (track, _) = ui.allocate_exact_size(Vec2::new(w, 32.0), Sense::hover());
    ui.painter()
        .rect_filled(track, CornerRadius::same(crate::theme::radius::LG), p.track);
    let n = options.len().max(1) as f32;
    let inner = track.shrink(3.0);
    let seg_w = inner.width() / n;
    // 13 px, 12 with four segments (docs/DESIGN.md §4.5); smaller still when a label would not fit.
    let widest = |size: f32| {
        options
            .iter()
            .map(|(_, l)| {
                ui.painter()
                    .layout_no_wrap((*l).to_string(), crate::theme::font(600, size), p.text)
                    .size()
                    .x
            })
            .fold(0.0_f32, f32::max)
    };
    let natural = |size: f32| -> Vec<f32> {
        options
            .iter()
            .map(|(_, l)| {
                ui.painter()
                    .layout_no_wrap((*l).to_string(), crate::theme::font(600, size), p.text)
                    .size()
                    .x
                    + 8.0
            })
            .collect()
    };
    // Equal segments at 13 px (12 with four) when the labels fit; otherwise each as wide as its label,
    // stepping the text down to 10.5 px until all of them fit.
    let mut size: f32 = if options.len() >= 4 { 12.0 } else { 13.0 };
    let widths: Vec<f32> = if widest(size) <= seg_w - 8.0 {
        vec![seg_w; options.len()]
    } else {
        let mut nat = natural(size);
        while size > 10.5 && nat.iter().sum::<f32>() > inner.width() {
            size -= 0.5;
            nat = natural(size);
        }
        let sum: f32 = nat.iter().sum();
        nat.iter().map(|w| w / sum * inner.width()).collect()
    };
    let mut changed = false;
    let mut x = inner.left();
    for (i, (v, label)) in options.iter().enumerate() {
        let r = egui::Rect::from_min_size(
            egui::pos2(x, inner.top()),
            Vec2::new(widths[i], inner.height()),
        );
        x += widths[i];
        let resp = ui.interact(r, ui.id().with(("seg", i)), Sense::click());
        let sel = *current == *v;
        if sel {
            ui.painter().rect_filled(
                r.translate(Vec2::new(0.0, 1.0)),
                CornerRadius::same(crate::theme::radius::SEGMENT),
                Color32::from_black_alpha(if p.dark { 100 } else { 20 }),
            );
            ui.painter().rect_filled(
                r,
                CornerRadius::same(crate::theme::radius::SEGMENT),
                p.segment,
            );
        } else if resp.hovered() {
            ui.painter().rect_filled(
                r,
                CornerRadius::same(crate::theme::radius::SEGMENT),
                p.hover,
            );
        }
        let font = if sel {
            crate::theme::font(600, size)
        } else {
            egui::FontId::proportional(size)
        };
        let col = if sel { p.text } else { p.text_dim };
        let g = ui.painter().layout_no_wrap((*label).to_string(), font, col);
        let clip = r.shrink2(Vec2::new(4.0, 0.0));
        let pos = egui::pos2(
            (r.center().x - g.size().x / 2.0).max(clip.left()),
            r.center().y - g.size().y / 2.0,
        );
        ui.painter().with_clip_rect(clip).galley(pos, g, col);
        let resp = resp.on_hover_text(*label);
        resp.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, sel, *label)
        });
        if resp.clicked() && !sel {
            *current = *v;
            changed = true;
        }
    }
    changed
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
                    egui::Label::new(
                        RichText::new(label)
                            .font(crate::theme::font(500, 12.0))
                            .color(dim(ui)),
                    )
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
        let rail = pal(ui).border;
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
        let r = if hot { 8.0 } else { 7.0 };
        painter.circle_filled(egui::pos2(x, y), r, Color32::WHITE);
        painter.circle_stroke(egui::pos2(x, y), r, Stroke::new(1.0, accent));
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
    let size = Vec2::new(18.0, 32.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hovered = resp.hovered();
    let pl = pal(ui);
    let bg = if hovered {
        pl.hover
    } else {
        Color32::TRANSPARENT
    };
    let ink = pl.text_muted;
    let p = ui.painter();
    p.rect(
        rect,
        CornerRadius::same(crate::theme::radius::MD),
        bg,
        Stroke::NONE,
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
