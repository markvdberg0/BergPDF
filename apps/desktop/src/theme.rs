//! Berg visual identity: warm neutral chrome with a terracotta accent.
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
    // Warm neutrals and a terracotta accent, close to the Claude desktop app.
    pub const LIGHT: Palette = Palette {
        chrome: Color32::from_rgb(0xF0, 0xEE, 0xE6),
        panel: Color32::from_rgb(0xFA, 0xF9, 0xF5),
        canvas: Color32::from_rgb(0xDD, 0xDA, 0xCF),
        text: Color32::from_rgb(0x14, 0x14, 0x13),
        text_dim: Color32::from_rgb(0x6B, 0x6A, 0x64),
        accent: Color32::from_rgb(0xC9, 0x64, 0x42),
        accent_soft: Color32::from_rgb(0xF3, 0xDD, 0xD2),
        border: Color32::from_rgb(0xE3, 0xE0, 0xD5),
        hover: Color32::from_rgb(0xE8, 0xE5, 0xDA),
        page_shadow: Color32::from_black_alpha(50),
        danger: Color32::from_rgb(0xB4, 0x2B, 0x2B),
        dark: false,
    };
    pub const DARK: Palette = Palette {
        chrome: Color32::from_rgb(0x1F, 0x1E, 0x1D),
        panel: Color32::from_rgb(0x26, 0x26, 0x24),
        canvas: Color32::from_rgb(0x14, 0x14, 0x13),
        text: Color32::from_rgb(0xF5, 0xF4, 0xEF),
        text_dim: Color32::from_rgb(0xA3, 0xA1, 0x97),
        accent: Color32::from_rgb(0xD9, 0x77, 0x57),
        accent_soft: Color32::from_rgb(0x4A, 0x30, 0x27),
        border: Color32::from_rgb(0x3A, 0x39, 0x36),
        hover: Color32::from_rgb(0x33, 0x32, 0x30),
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
        Color32::from_rgb(0x1B, 0x1B, 0x1A)
    } else {
        Color32::WHITE
    };
    v.faint_bg_color = pal.hover;
    v.override_text_color = Some(pal.text);
    v.hyperlink_color = pal.accent;
    v.selection.bg_fill = pal.accent_soft;
    v.selection.stroke = Stroke::new(1.0, pal.accent);
    v.window_stroke = Stroke::new(1.0, pal.border);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(10);
    let r = CornerRadius::same(8);
    v.widgets.noninteractive.bg_fill = pal.chrome;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, pal.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, pal.text);
    v.widgets.noninteractive.corner_radius = r;
    // Buttons use `weak_bg_fill` (transparent until hovered); input controls (sliders,
    // checkboxes, text fields) use `bg_fill`/`bg_stroke` and must stay visible.
    v.widgets.inactive.bg_fill = if pal.dark {
        Color32::from_rgb(0x3A, 0x39, 0x36)
    } else {
        Color32::from_rgb(0xE3, 0xE0, 0xD5)
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
