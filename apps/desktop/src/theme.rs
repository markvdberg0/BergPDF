//! Berg visual identity (docs/DESIGN.md): quiet warm-neutral chrome, one terracotta accent, Inter.
//! Applies light/dark chrome, density and UI scale. It never touches page colours.

use editor_core::prefs::{Density, Preferences, ThemeChoice};
use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, Visuals};
use std::sync::Arc;

/// Semantic colours (docs/DESIGN.md §2.1). Field names stay stable; the token each one is is noted.
#[derive(Clone, Copy)]
pub struct Palette {
    /// `bg.chrome`: title row, ribbon tab row, document tab row, status bar, start screen.
    pub chrome: Color32,
    /// `bg.panel`: left panel, drop zone.
    pub panel: Color32,
    /// `bg.canvas`: around the pages.
    pub canvas: Color32,
    /// `bg.surface`: ribbon row, right panel, active tabs, cards, buttons.
    pub card: Color32,
    /// `bg.field`: text fields, dropdowns.
    pub field: Color32,
    /// `bg.track`: segmented-control track, keyboard chips, privacy note.
    pub track: Color32,
    /// `bg.segment`: the selected segment.
    pub segment: Color32,
    /// `bg.hover`.
    pub hover: Color32,
    /// `bg.pressed`.
    pub pressed: Color32,
    /// `border.default`.
    pub border: Color32,
    /// `border.subtle`: lines inside a panel.
    pub border_subtle: Color32,
    /// `border.strong`: edges of fields, checkboxes, switch track (off).
    pub border_strong: Color32,
    /// `border.dashed`: the drop zone.
    pub border_dashed: Color32,
    /// `text.primary`.
    pub text: Color32,
    /// `text.secondary`.
    pub text_dim: Color32,
    /// `text.muted`.
    pub text_muted: Color32,
    /// `text.disabled`.
    pub text_disabled: Color32,
    /// `accent`: icons and fills only, never text on light.
    pub accent: Color32,
    /// `accent.text`: accent-coloured text.
    pub accent_text: Color32,
    /// `accent.soft`.
    pub accent_soft: Color32,
    /// `primary.bg`.
    pub primary: Color32,
    /// `primary.hover`.
    pub primary_hover: Color32,
    /// `primary.text`.
    pub on_primary: Color32,
    pub success_bg: Color32,
    pub success: Color32,
    pub warning_bg: Color32,
    pub warning: Color32,
    pub danger_bg: Color32,
    /// `danger.text`.
    pub danger: Color32,
    pub page_shadow: Color32,
    pub dark: bool,
}

const fn rgb(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

impl Palette {
    pub const LIGHT: Palette = Palette {
        chrome: rgb(0xF6F5F2),
        panel: rgb(0xFAFAF8),
        canvas: rgb(0xE9E8E3),
        card: rgb(0xFFFFFF),
        field: rgb(0xFFFFFF),
        track: rgb(0xEFEEEA),
        segment: rgb(0xFFFFFF),
        hover: rgb(0xEFEEEA),
        pressed: rgb(0xE6E4DE),
        border: rgb(0xE2E0DA),
        border_subtle: rgb(0xEFEEEA),
        border_strong: rgb(0x949189),
        border_dashed: rgb(0xD3D1CA),
        text: rgb(0x1C1C1A),
        text_dim: rgb(0x55544E),
        text_muted: rgb(0x66655F),
        text_disabled: rgb(0xAAA8A0),
        accent: rgb(0xC2603E),
        accent_text: rgb(0xA54D2F),
        accent_soft: rgb(0xF8E6DD),
        primary: rgb(0xB5532F),
        primary_hover: rgb(0xA04A2A),
        on_primary: rgb(0xFFFFFF),
        success_bg: rgb(0xEEF6EE),
        success: rgb(0x1F5A2A),
        warning_bg: rgb(0xF8EBC9),
        warning: rgb(0x7A5A12),
        danger_bg: rgb(0xF6DCDC),
        danger: rgb(0xA3302B),
        page_shadow: Color32::from_black_alpha(20),
        dark: false,
    };
    pub const DARK: Palette = Palette {
        chrome: rgb(0x1E1E1C),
        panel: rgb(0x232321),
        canvas: rgb(0x141413),
        card: rgb(0x262624),
        field: rgb(0x1C1C1B),
        track: rgb(0x1C1C1B),
        segment: rgb(0x34332F),
        hover: rgb(0x302F2D),
        pressed: rgb(0x383734),
        border: rgb(0x383734),
        border_subtle: rgb(0x2F2E2C),
        border_strong: rgb(0x7A7870),
        border_dashed: rgb(0x4A4945),
        text: rgb(0xF2F1EC),
        text_dim: rgb(0xC4C2B9),
        text_muted: rgb(0xA3A197),
        text_disabled: rgb(0x6E6C66),
        accent: rgb(0xD97757),
        accent_text: rgb(0xE8957A),
        accent_soft: rgb(0x4A3027),
        primary: rgb(0xD97757),
        primary_hover: rgb(0xE3876A),
        on_primary: rgb(0x1A1A19),
        success_bg: rgb(0x1F3324),
        success: rgb(0x9FD3A8),
        warning_bg: rgb(0x3A3020),
        warning: rgb(0xE3C27A),
        danger_bg: rgb(0x3E2424),
        danger: rgb(0xE58A80),
        page_shadow: Color32::from_black_alpha(128),
        dark: true,
    };

    /// Chips and tooltips drawn over the page (`overlay.bg`): the page is white in both themes.
    pub const OVERLAY_BG: Color32 = rgb(0x1C1C1A);
    /// `overlay.text`.
    pub const OVERLAY_TEXT: Color32 = Color32::WHITE;
}

/// Category tints for icon tiles (background, icon), docs/DESIGN.md §2.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tint {
    Accent,
    Blue,
    Purple,
    Green,
    Amber,
    Neutral,
    Red,
}

impl Tint {
    /// Background and icon colour.
    pub fn colors(self, dark: bool) -> (Color32, Color32) {
        let (l, d) = match self {
            Tint::Accent => ((0xF8E6DD, 0xA54D2F), (0x3D2A22, 0xE8957A)),
            Tint::Blue => ((0xDCEBF8, 0x2D6A9F), (0x1F2E3D, 0x8DB9E8)),
            Tint::Purple => ((0xE6E1F5, 0x5B4A9E), (0x2C2740, 0xB7A8EA)),
            Tint::Green => ((0xDFF0E2, 0x2F6B3B), (0x1F3324, 0x9FD3A8)),
            Tint::Amber => ((0xF8EBC9, 0x7A5A12), (0x3A3020, 0xE3C27A)),
            Tint::Neutral => ((0xEFEEEA, 0x4A4842), (0x33322F, 0xC4C2B9)),
            Tint::Red => ((0xF6DCDC, 0x9E2F2F), (0x3E2424, 0xEE9A92)),
        };
        let (bg, fg) = if dark { d } else { l };
        (rgb(bg), rgb(fg))
    }
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

/// Sizes that follow the density preference (docs/DESIGN.md §3, "Density").
#[derive(Clone, Copy)]
pub struct Metrics {
    /// Icon size in the ribbon and panels.
    pub icon: f32,
    /// Field and small-button height.
    pub button_h: f32,
    /// Classic ribbon button (icon above label).
    pub ribbon_button: egui::Vec2,
    /// Document tab height.
    pub tab_h: f32,
    /// Labels next to ribbon icons.
    pub show_labels: bool,
    pub title_h: f32,
    pub ribbon_tabs_h: f32,
    pub ribbon_tab_h: f32,
    pub ribbon_h: f32,
    pub ribbon_button_h: f32,
    pub doc_tabs_h: f32,
    pub status_h: f32,
}

pub fn metrics(d: Density) -> Metrics {
    match d {
        Density::Compact => Metrics {
            icon: 16.0,
            button_h: 28.0,
            ribbon_button: egui::vec2(30.0, 30.0),
            tab_h: 26.0,
            show_labels: false,
            title_h: 36.0,
            ribbon_tabs_h: 30.0,
            ribbon_tab_h: 28.0,
            ribbon_h: 40.0,
            ribbon_button_h: 28.0,
            doc_tabs_h: 32.0,
            status_h: 26.0,
        },
        Density::Comfortable => Metrics {
            icon: 18.0,
            button_h: 32.0,
            ribbon_button: egui::vec2(60.0, 58.0),
            tab_h: 30.0,
            show_labels: true,
            title_h: 40.0,
            ribbon_tabs_h: 34.0,
            ribbon_tab_h: 32.0,
            ribbon_h: 52.0,
            ribbon_button_h: 34.0,
            doc_tabs_h: 36.0,
            status_h: 30.0,
        },
        Density::Touch => Metrics {
            icon: 22.0,
            button_h: 44.0,
            ribbon_button: egui::vec2(72.0, 70.0),
            tab_h: 38.0,
            show_labels: true,
            title_h: 48.0,
            ribbon_tabs_h: 42.0,
            ribbon_tab_h: 40.0,
            ribbon_h: 64.0,
            ribbon_button_h: 44.0,
            doc_tabs_h: 44.0,
            status_h: 36.0,
        },
    }
}

/// Radii (docs/DESIGN.md §2.4).
pub mod radius {
    pub const XS: u8 = 3;
    pub const SM: u8 = 5;
    pub const SEGMENT: u8 = 6;
    pub const MD: u8 = 7;
    pub const LG: u8 = 8;
    pub const XL: u8 = 10;
    pub const XXL: u8 = 12;
    pub const XXXL: u8 = 14;
    pub const XXXXL: u8 = 18;
}

static INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
static INTER_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");
static INTER_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");

/// The interface font at a weight: 400, 500 or 600 (700 is drawn with SemiBold, the heaviest weight
/// bundled).
pub fn font(weight: u16, size: f32) -> FontId {
    match weight {
        0..=450 => FontId::proportional(size),
        451..=550 => FontId::new(size, FontFamily::Name("inter-500".into())),
        _ => FontId::new(size, FontFamily::Name("inter-600".into())),
    }
}

/// Install fonts once: Inter for the interface (Regular as the proportional font, Medium and SemiBold as
/// named families), DejaVu Sans behind it for symbols (⌘ ⇧ ⌥ ⌃) and scripts Inter lacks.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "dejavu".into(),
        Arc::new(FontData::from_static(pdf_engine::fontembed::DEJAVU_SANS)),
    );
    for (name, data) in [
        ("inter-400", INTER_REGULAR),
        ("inter-500", INTER_MEDIUM),
        ("inter-600", INTER_SEMIBOLD),
    ] {
        fonts
            .font_data
            .insert(name.into(), Arc::new(FontData::from_static(data)));
    }
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "inter-400".into());
    for fam in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts.families.entry(fam).or_default().push("dejavu".into());
    }
    for w in ["inter-500", "inter-600"] {
        fonts.families.insert(
            FontFamily::Name(w.into()),
            vec![w.into(), "inter-400".into(), "dejavu".into()],
        );
    }
    // Every bundled PDF font, so pickers and previews are drawn in the font itself.
    crate::fontpick::register(&mut fonts);
    ctx.set_fonts(fonts);
}

/// Installed fonts with Chinese characters, best first: simplified Chinese designs before the
/// Japanese and Korean ones that share the characters but draw them differently.
const CJK_FAMILIES: &[&str] = &[
    "Microsoft YaHei",
    "Microsoft YaHei UI",
    "PingFang SC",
    "Hiragino Sans GB",
    "Noto Sans CJK SC",
    "Noto Sans SC",
    "Source Han Sans SC",
    "WenQuanYi Micro Hei",
    "WenQuanYi Zen Hei",
    "Heiti SC",
    "STHeiti",
    "DengXian",
    "SimHei",
    "SimSun",
    "Droid Sans Fallback",
];

/// Add an installed Chinese font behind the interface fonts, so the Chinese interface (and Chinese
/// file names, bookmarks or form values in any language) is not drawn as empty boxes. Call after the
/// installed fonts were scanned. Returns whether a font was found.
pub fn install_cjk_fallback(ctx: &egui::Context) -> bool {
    use pdf_engine::sysfonts;
    let probe = ['中', '文', '简', '体'];
    let face_data = |id: u32| {
        sysfonts::with_family(id, |fam| {
            let face = fam.face(false, false)?;
            Some((face.data()?, face.index))
        })
        .flatten()
    };
    let found = CJK_FAMILIES
        .iter()
        .filter_map(|name| sysfonts::find_by_name(name))
        .find_map(face_data)
        .or_else(|| sysfonts::family_covering(&probe).and_then(face_data));
    let Some((data, index)) = found else {
        tracing::info!("no installed font with Chinese characters found");
        return false;
    };
    let mut font = FontData::from_static(data);
    font.index = index;
    ctx.add_font(egui::epaint::text::FontInsert::new(
        "cjk",
        font,
        [
            FontFamily::Proportional,
            FontFamily::Monospace,
            FontFamily::Name("inter-500".into()),
            FontFamily::Name("inter-600".into()),
        ]
        .into_iter()
        .map(|family| egui::epaint::text::InsertFontFamily {
            family,
            priority: egui::epaint::text::FontPriority::Lowest,
        })
        .collect(),
    ));
    ctx.request_repaint();
    true
}

/// Apply theme, density and scale to the context (docs/DESIGN.md §8.2).
pub fn apply(ctx: &egui::Context, prefs: &Preferences, pal: &Palette) {
    // egui keeps a light and a dark style and picks one by its own theme (the OS theme by default);
    // pin it to ours, so the style set below is the one that is used.
    ctx.set_theme(if pal.dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    let mut v = if pal.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.panel_fill = pal.card;
    v.window_fill = pal.card;
    v.extreme_bg_color = pal.field;
    v.faint_bg_color = pal.track;
    v.code_bg_color = pal.track;
    v.override_text_color = Some(pal.text);
    v.hyperlink_color = pal.accent_text;
    v.warn_fg_color = pal.warning;
    v.error_fg_color = pal.danger;
    v.selection.bg_fill = pal.accent_soft;
    v.selection.stroke = Stroke::new(1.0, pal.accent);
    v.window_stroke = Stroke::new(1.0, pal.border);
    v.window_corner_radius = CornerRadius::same(radius::XXL);
    v.menu_corner_radius = CornerRadius::same(radius::XL);
    let shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(if pal.dark { 115 } else { 31 }),
    };
    v.window_shadow = shadow;
    v.popup_shadow = shadow;
    let r = CornerRadius::same(radius::MD);
    v.widgets.noninteractive.bg_fill = pal.card;
    v.widgets.noninteractive.weak_bg_fill = pal.card;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, pal.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, pal.text);
    v.widgets.noninteractive.corner_radius = r;
    // Buttons fill `weak_bg_fill` with a hairline `border`; inputs (text fields, checkboxes, sliders)
    // use `bg_fill` and need the stronger edge to meet 3:1.
    v.widgets.inactive.bg_fill = pal.field;
    v.widgets.inactive.weak_bg_fill = pal.card;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, pal.border_strong);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, pal.text);
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.bg_fill = pal.hover;
    v.widgets.hovered.weak_bg_fill = pal.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, pal.border_strong);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, pal.text);
    v.widgets.hovered.corner_radius = r;
    v.widgets.hovered.expansion = 0.0;
    v.widgets.active.bg_fill = pal.pressed;
    v.widgets.active.weak_bg_fill = pal.pressed;
    v.widgets.active.bg_stroke = Stroke::new(1.0, pal.accent);
    v.widgets.active.fg_stroke = Stroke::new(1.5, pal.text);
    v.widgets.active.corner_radius = r;
    v.widgets.active.expansion = 0.0;
    v.widgets.open = v.widgets.active;
    v.widgets.open.bg_fill = pal.card;
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
        s.spacing.button_padding = egui::vec2(11.0, ((m.button_h - 16.0) / 2.0).max(4.0));
        s.spacing.interact_size.y = m.button_h;
        s.spacing.icon_width = 16.0;
        s.spacing.icon_width_inner = 9.0;
        s.spacing.icon_spacing = 8.0;
        s.spacing.menu_margin = egui::Margin::same(8);
        s.spacing.window_margin = egui::Margin::same(16);
        s.spacing.scroll.bar_width = 10.0;
        s.spacing.slider_width = 110.0;
        s.interaction.tooltip_delay = 0.35;
        s.visuals.striped = false;
        s.visuals.slider_trailing_fill = true;
        s.visuals.handle_shape = egui::style::HandleShape::Circle;
        use egui::TextStyle as T;
        s.text_styles.insert(T::Body, FontId::proportional(13.0));
        s.text_styles.insert(T::Button, FontId::proportional(13.0));
        s.text_styles.insert(T::Small, FontId::proportional(11.5));
        s.text_styles.insert(T::Heading, font(700, 15.0));
        s.text_styles.insert(T::Monospace, FontId::monospace(12.0));
    });
    ctx.set_zoom_factor(prefs.ui_scale.clamp(0.75, 2.5));
    // The palette for `ui_kit`, which has no `App` at hand.
    let pal = *pal;
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(crate::ui_kit::PALETTE_ID), pal));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lum(c: Color32) -> f64 {
        let ch = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// docs/DESIGN.md §2.1 and §7: text 4.5:1, icons and field edges 3:1, in both themes, so a later
    /// colour tweak that breaks readability fails here.
    #[test]
    fn the_palettes_keep_their_contrast() {
        for p in [Palette::LIGHT, Palette::DARK] {
            let text = [
                (p.text, p.card),
                (p.text, p.chrome),
                (p.text_dim, p.chrome),
                (p.text_dim, p.card),
                (p.text_muted, p.chrome),
                (p.text_muted, p.card),
                (p.accent_text, p.card),
                (p.accent_text, p.accent_soft),
                (p.on_primary, p.primary),
                (p.success, p.success_bg),
                (p.warning, p.warning_bg),
                (p.danger, p.card),
                (p.danger, p.danger_bg),
            ];
            for (i, (a, b)) in text.iter().enumerate() {
                let c = contrast(*a, *b);
                assert!(c >= 4.5, "text pair {i} (dark: {}) is {c:.2}:1", p.dark);
            }
            let edges = [
                (p.accent, p.card),
                (p.border_strong, p.card),
                (p.border_strong, p.field),
            ];
            for (i, (a, b)) in edges.iter().enumerate() {
                let c = contrast(*a, *b);
                assert!(
                    c >= 3.0,
                    "icon/edge pair {i} (dark: {}) is {c:.2}:1",
                    p.dark
                );
            }
            for t in [
                Tint::Accent,
                Tint::Blue,
                Tint::Purple,
                Tint::Green,
                Tint::Amber,
                Tint::Neutral,
                Tint::Red,
            ] {
                let (bg, fg) = t.colors(p.dark);
                assert!(contrast(fg, bg) >= 4.5, "{t:?} (dark: {})", p.dark);
            }
        }
    }
}
