//! Original Berg visual identity: slate chrome with a copper accent.
//! Applies light/dark chrome, density and UI scale. It never touches page colours.

use editor_core::prefs::{Density, Preferences, ThemeChoice};
use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke, Visuals};
use std::sync::Arc;

/// Semantic colours used by custom-painted widgets.
#[derive(Clone, Copy)]
pub struct Palette {
    pub chrome: Color32,
    pub panel: Color32,
    pub canvas: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub border: Color32,
    pub hover: Color32,
    pub page_shadow: Color32,
    pub danger: Color32,
    pub dark: bool,
}

impl Palette {
    pub const LIGHT: Palette = Palette {
        chrome: Color32::from_rgb(0xEC, 0xEE, 0xF2),
        panel: Color32::from_rgb(0xF7, 0xF8, 0xFA),
        canvas: Color32::from_rgb(0xC9, 0xCE, 0xD6),
        text: Color32::from_rgb(0x1D, 0x23, 0x30),
        text_dim: Color32::from_rgb(0x5B, 0x64, 0x76),
        accent: Color32::from_rgb(0xB9, 0x55, 0x24),
        accent_soft: Color32::from_rgb(0xF4, 0xDC, 0xCD),
        border: Color32::from_rgb(0xD0, 0xD5, 0xDD),
        hover: Color32::from_rgb(0xE0, 0xE4, 0xEA),
        page_shadow: Color32::from_black_alpha(60),
        danger: Color32::from_rgb(0xB4, 0x2B, 0x2B),
        dark: false,
    };
    pub const DARK: Palette = Palette {
        chrome: Color32::from_rgb(0x1F, 0x23, 0x2B),
        panel: Color32::from_rgb(0x27, 0x2C, 0x36),
        canvas: Color32::from_rgb(0x12, 0x15, 0x1A),
        text: Color32::from_rgb(0xE6, 0xE9, 0xEF),
        text_dim: Color32::from_rgb(0x9A, 0xA3, 0xB5),
        accent: Color32::from_rgb(0xE8, 0x80, 0x4A),
        accent_soft: Color32::from_rgb(0x4A, 0x2E, 0x20),
        border: Color32::from_rgb(0x3A, 0x41, 0x4D),
        hover: Color32::from_rgb(0x33, 0x3A, 0x47),
        page_shadow: Color32::from_black_alpha(140),
        danger: Color32::from_rgb(0xE0, 0x6A, 0x6A),
        dark: true,
    };
}

/// Resolve the effective palette (System follows the OS theme reported by egui).
pub fn palette(prefs: &Preferences, system_dark: bool) -> Palette {
    let dark = match prefs.theme {
        ThemeChoice::Light => false,
        ThemeChoice::Dark => true,
        ThemeChoice::System => system_dark,
    };
    if dark { Palette::DARK } else { Palette::LIGHT }
}

/// Metrics derived from density.
#[derive(Clone, Copy)]
pub struct Metrics {
    pub icon: f32,
    pub button_h: f32,
    pub ribbon_button: egui::Vec2,
    pub tab_h: f32,
    pub show_labels: bool,
}

pub fn metrics(d: Density) -> Metrics {
    match d {
        Density::Compact => Metrics {
            icon: 16.0,
            button_h: 24.0,
            ribbon_button: egui::vec2(30.0, 30.0),
            tab_h: 28.0,
            show_labels: false,
        },
        Density::Comfortable => Metrics {
            icon: 20.0,
            button_h: 28.0,
            ribbon_button: egui::vec2(60.0, 58.0),
            tab_h: 32.0,
            show_labels: true,
        },
        Density::Touch => Metrics {
            icon: 26.0,
            button_h: 40.0,
            ribbon_button: egui::vec2(72.0, 70.0),
            tab_h: 42.0,
            show_labels: true,
        },
    }
}

/// Install fonts once: egui's defaults plus DejaVu Sans as a fallback so shortcut symbols
/// (⌘ ⇧ ⌥ ⌃) and Latin/Greek/Cyrillic UI text render everywhere.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "dejavu".into(),
        Arc::new(FontData::from_static(pdf_engine::fontembed::DEJAVU_SANS)),
    );
    for fam in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts.families.entry(fam).or_default().push("dejavu".into());
    }
    // Every bundled PDF font, so pickers and previews are drawn in the font itself.
    crate::fontpick::register(&mut fonts);
    ctx.set_fonts(fonts);
}

/// Apply theme, density and scale to the context.
pub fn apply(ctx: &egui::Context, prefs: &Preferences, pal: &Palette) {
    let mut v = if pal.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.panel_fill = pal.panel;
    v.window_fill = pal.panel;
    v.extreme_bg_color = if pal.dark {
        Color32::from_rgb(0x1A, 0x1E, 0x25)
    } else {
        Color32::WHITE
    };
    v.faint_bg_color = pal.hover;
    v.override_text_color = Some(pal.text);
    v.hyperlink_color = pal.accent;
    v.selection.bg_fill = pal.accent_soft;
    v.selection.stroke = Stroke::new(1.0, pal.accent);
    v.window_stroke = Stroke::new(1.0, pal.border);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    let r = CornerRadius::same(5);
    v.widgets.noninteractive.bg_fill = pal.chrome;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, pal.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, pal.text);
    v.widgets.noninteractive.corner_radius = r;
    // Buttons use `weak_bg_fill` (transparent until hovered); input controls (sliders,
    // checkboxes, text fields) use `bg_fill`/`bg_stroke` and must stay visible.
    v.widgets.inactive.bg_fill = if pal.dark {
        Color32::from_rgb(0x3A, 0x41, 0x4D)
    } else {
        Color32::from_rgb(0xD7, 0xDC, 0xE4)
    };
    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, pal.border);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, pal.text);
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.bg_fill = pal.hover;
    v.widgets.hovered.weak_bg_fill = pal.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, pal.border);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, pal.text);
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.bg_fill = pal.accent_soft;
    v.widgets.active.weak_bg_fill = pal.accent_soft;
    v.widgets.active.bg_stroke = Stroke::new(1.0, pal.accent);
    v.widgets.active.fg_stroke = Stroke::new(1.5, pal.text);
    v.widgets.active.corner_radius = r;
    v.widgets.open = v.widgets.active;
    ctx.set_visuals(v);

    let m = metrics(prefs.density);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(
            6.0,
            if matches!(prefs.density, Density::Compact) {
                3.0
            } else {
                5.0
            },
        );
        s.spacing.button_padding = egui::vec2(8.0, (m.button_h - 16.0) / 2.0);
        s.spacing.interact_size.y = m.button_h;
        s.spacing.icon_width = 16.0;
        s.spacing.scroll.bar_width = 10.0;
        s.spacing.slider_width = 120.0;
        s.interaction.tooltip_delay = 0.35;
        s.visuals.striped = false;
    });
    ctx.set_zoom_factor(prefs.ui_scale.clamp(0.75, 2.5));
}
