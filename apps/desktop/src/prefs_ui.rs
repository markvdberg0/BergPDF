//! The Preferences window: a page list on the left, one row per setting on the right
//! (name and explanation at the left, the control at the right), and a search box that
//! shows matching rows from every page.

use crate::i18n::tr;
use crate::state::*;
use crate::tf;
use crate::theme::Palette;
use editor_core::prefs::{
    DefaultZoom, Density, GfxBackend, Language, PresentChoice, SETTINGS, ThemeChoice, UpdateCheck,
    Workspace,
};
use egui::{Align, Color32, CornerRadius, Layout, RichText, Sense, Stroke, Vec2};

const NAV_WIDTH: f32 = 220.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    General,
    Appearance,
    Documents,
    Performance,
    Copilot,
    Shortcuts,
}

const PAGES: [Page; 6] = [
    Page::General,
    Page::Appearance,
    Page::Documents,
    Page::Performance,
    Page::Copilot,
    Page::Shortcuts,
];

impl Page {
    fn title(self) -> &'static str {
        match self {
            Page::General => tr("General"),
            Page::Appearance => tr("Appearance"),
            Page::Documents => tr("Documents"),
            Page::Performance => tr("Performance"),
            Page::Copilot => tr("PDF Copilot"),
            Page::Shortcuts => tr("Keyboard shortcuts"),
        }
    }

    /// Setting keys on the page, in display order.
    fn keys(self) -> &'static [&'static str] {
        match self {
            Page::General => &["language", "workspace", "author", "updates"],
            Page::Appearance => &["theme", "dark_page_filter", "density", "ui_scale"],
            Page::Documents => &["default_zoom", "default_font", "snap_to_geometry"],
            Page::Performance => &["graphics", "render_cache_mb"],
            Page::Copilot => &["ai"],
            Page::Shortcuts => &["shortcuts"],
        }
    }
}

/// What the window asks the caller to do.
#[derive(Default)]
pub struct PrefsOutcome {
    pub changed: bool,
    pub close: bool,
    pub check_now: bool,
}

impl App {
    pub fn preferences_ui(&mut self, ctx: &egui::Context, filter: &mut String) -> PrefsOutcome {
        let pal = self.pal;
        let mut out = PrefsOutcome::default();
        let screen = ctx.content_rect();
        let width = (screen.width() - 80.0).clamp(560.0, 980.0);
        let height = (screen.height() - 80.0).clamp(380.0, 700.0);
        let page_id = egui::Id::new("prefs_page");
        let mut page_ix: usize = ctx
            .memory_mut(|m| m.data.get_temp(page_id))
            .unwrap_or(0)
            .min(PAGES.len() - 1);

        let frame = egui::Frame::new()
            .fill(pal.panel)
            .stroke(Stroke::new(1.0, pal.border))
            .corner_radius(12)
            .inner_margin(0);
        let modal = egui::Modal::new(egui::Id::new("preferences"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.set_height(height);
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                ui.horizontal_top(|ui| {
                    // Page list.
                    let nav = egui::Frame::new()
                        .fill(pal.chrome)
                        .corner_radius(CornerRadius {
                            nw: 12,
                            sw: 12,
                            ne: 0,
                            se: 0,
                        })
                        .inner_margin(12);
                    nav.show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.set_width(NAV_WIDTH - 24.0);
                            ui.set_height(height - 24.0);
                            ui.spacing_mut().item_spacing = Vec2::new(0.0, 4.0);
                            ui.add(
                                egui::TextEdit::singleline(filter)
                                    .hint_text(tr("Search settings"))
                                    .desired_width(f32::INFINITY)
                                    .margin(Vec2::new(10.0, 7.0)),
                            );
                            ui.add_space(10.0);
                            for (i, p) in PAGES.iter().enumerate() {
                                let selected = filter.is_empty() && i == page_ix;
                                let (rect, resp) = ui.allocate_exact_size(
                                    Vec2::new(ui.available_width(), 34.0),
                                    Sense::click(),
                                );
                                if selected || resp.hovered() {
                                    ui.painter().rect_filled(
                                        rect,
                                        8.0,
                                        if selected {
                                            pal.hover
                                        } else {
                                            pal.accent_soft.gamma_multiply(0.4)
                                        },
                                    );
                                }
                                ui.painter().text(
                                    egui::pos2(rect.min.x + 12.0, rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    p.title(),
                                    egui::FontId::proportional(14.0),
                                    if selected { pal.text } else { pal.text_dim },
                                );
                                if resp.clicked() {
                                    page_ix = i;
                                    filter.clear();
                                }
                            }
                        })
                    });

                    // Rows.
                    let content_w = width - NAV_WIDTH;
                    ui.allocate_ui_with_layout(
                        Vec2::new(content_w, height),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.spacing_mut().item_spacing = Vec2::new(8.0, 4.0);
                            egui::Frame::new()
                                .inner_margin(egui::Margin::symmetric(28, 20))
                                .show(ui, |ui| {
                                    ui.set_width(content_w - 56.0);
                                    ui.horizontal(|ui| {
                                        let title = if filter.is_empty() {
                                            PAGES[page_ix].title()
                                        } else {
                                            tr("Search settings")
                                        };
                                        ui.label(
                                            RichText::new(title)
                                                .size(20.0)
                                                .strong()
                                                .color(pal.text),
                                        );
                                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                                            let close = ui.add(
                                                egui::Button::new(
                                                    RichText::new("✕")
                                                        .size(16.0)
                                                        .color(pal.text_dim),
                                                )
                                                .frame(false),
                                            );
                                            if close.clicked() {
                                                out.close = true;
                                            }
                                        });
                                    });
                                    ui.add_space(8.0);
                                    egui::ScrollArea::vertical()
                                        .auto_shrink([false, false])
                                        .max_height(height - 90.0)
                                        .show(ui, |ui| {
                                            ui.set_width(content_w - 70.0);
                                            let f = filter.to_lowercase();
                                            let mut any = false;
                                            for p in PAGES {
                                                for key in
                                                    p.keys().iter().filter(|k| setting_available(k))
                                                {
                                                    let visible = if f.is_empty() {
                                                        p == PAGES[page_ix]
                                                    } else {
                                                        matches_filter(key, &f)
                                                    };
                                                    if visible {
                                                        any = true;
                                                        self.pref_row(ui, ctx, key, &mut out);
                                                    }
                                                }
                                            }
                                            if !any {
                                                ui.label(
                                                    RichText::new(tr("No matching commands"))
                                                        .color(pal.text_dim),
                                                );
                                            }
                                        });
                                });
                        },
                    );
                });
            });
        if modal.should_close() {
            out.close = true;
        }
        ctx.memory_mut(|m| m.data.insert_temp(page_id, page_ix));
        out
    }

    fn pref_row(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        key: &str,
        out: &mut PrefsOutcome,
    ) {
        let pal = self.pal;
        let changed = &mut out.changed;
        match key {
            "language" => {
                row(
                    ui,
                    &pal,
                    tr("Language"),
                    tr(
                        "Interface language: English, Nederlands or Deutsch (or follow the system).",
                    ),
                    |ui| {
                        let shown = match self.prefs.language {
                            Language::System => tr("System default"),
                            Language::English => "English",
                            Language::Dutch => "Nederlands",
                            Language::German => "Deutsch",
                        };
                        egui::ComboBox::from_id_salt("language")
                            .selected_text(shown)
                            .show_ui(ui, |ui| {
                                *changed |= ui
                                    .selectable_value(
                                        &mut self.prefs.language,
                                        Language::System,
                                        tr("System default"),
                                    )
                                    .changed();
                                *changed |= ui
                                    .selectable_value(
                                        &mut self.prefs.language,
                                        Language::English,
                                        "English",
                                    )
                                    .changed();
                                *changed |= ui
                                    .selectable_value(
                                        &mut self.prefs.language,
                                        Language::Dutch,
                                        "Nederlands",
                                    )
                                    .changed();
                                *changed |= ui
                                    .selectable_value(
                                        &mut self.prefs.language,
                                        Language::German,
                                        "Deutsch",
                                    )
                                    .changed();
                            });
                    },
                );
            }
            "workspace" => {
                row(
                    ui,
                    &pal,
                    tr("Workspace"),
                    tr(
                        "Essential shows the common tools; Professional shows everything. Both can reach every command via search.",
                    ),
                    |ui| {
                        *changed |= segmented(
                            ui,
                            &pal,
                            &mut self.prefs.workspace,
                            &[
                                (Workspace::Essential, tr("Essential")),
                                (Workspace::Professional, tr("Professional")),
                            ],
                        );
                    },
                );
            }
            "author" => {
                row(
                    ui,
                    &pal,
                    tr("Author name"),
                    tr("Stored on comments and markup you create."),
                    |ui| {
                        *changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut self.prefs.author)
                                    .desired_width(200.0),
                            )
                            .changed();
                    },
                );
            }
            "updates" => {
                row(
                    ui,
                    &pal,
                    tr("Updates"),
                    tr(
                        "Let BergPDF look for a newer version when it starts (once a day). Only the program name and version are sent; nothing is downloaded or installed.",
                    ),
                    |ui| {
                        let mut on = self.prefs.update_check == UpdateCheck::On;
                        if toggle(ui, &pal, &mut on).changed() {
                            self.prefs.update_check = if on {
                                UpdateCheck::On
                            } else {
                                UpdateCheck::Off
                            };
                            self.prefs.last_update_check = 0;
                            *changed = true;
                        }
                        if ui.button(tr("Check for Updates…")).clicked() {
                            out.check_now = true;
                        }
                    },
                );
            }
            "theme" => {
                row(
                    ui,
                    &pal,
                    tr("Application theme"),
                    tr("Light, Dark or follow the system. Never changes document colours."),
                    |ui| {
                        *changed |= segmented(
                            ui,
                            &pal,
                            &mut self.prefs.theme,
                            &[
                                (ThemeChoice::System, tr("System")),
                                (ThemeChoice::Light, tr("Light")),
                                (ThemeChoice::Dark, tr("Dark")),
                            ],
                        );
                    },
                );
            }
            "dark_page_filter" => {
                row(
                    ui,
                    &pal,
                    tr("Dark page view"),
                    tr(
                        "Comfortable dark reading filter. Display only — saved PDFs are never changed.",
                    ),
                    |ui| {
                        *changed |= toggle(ui, &pal, &mut self.prefs.dark_page_filter).changed();
                    },
                );
            }
            "density" => {
                row(
                    ui,
                    &pal,
                    tr("Interface density"),
                    tr("Compact, Comfortable or Touch/Pen hit targets."),
                    |ui| {
                        *changed |= segmented(
                            ui,
                            &pal,
                            &mut self.prefs.density,
                            &[
                                (Density::Compact, tr("Compact")),
                                (Density::Comfortable, tr("Comfortable")),
                                (Density::Touch, tr("Touch / Pen")),
                            ],
                        );
                    },
                );
            }
            "ui_scale" => {
                row(
                    ui,
                    &pal,
                    tr("Interface scale"),
                    tr(
                        "Scales toolbar icons, text and hit targets. Page rendering stays sharp at any scale.",
                    ),
                    |ui| {
                        let r = ui.add(egui::Slider::new(&mut self.prefs.ui_scale, 0.75..=2.5));
                        // Rescaling re-renders every page, so it is applied when the slider is released.
                        *changed |= (r.changed() && !r.dragged()) || r.drag_stopped();
                    },
                );
            }
            "default_zoom" => {
                row(
                    ui,
                    &pal,
                    tr("Zoom when opening a document"),
                    tr("Applies to documents you open from now on; you can still zoom freely."),
                    |ui| {
                        *changed |= segmented(
                            ui,
                            &pal,
                            &mut self.prefs.default_zoom,
                            &[
                                (DefaultZoom::FitPage, tr("Fit page")),
                                (DefaultZoom::FitWidth, tr("Fit width")),
                                (DefaultZoom::Actual, "100 %"),
                            ],
                        );
                    },
                );
            }
            "default_font" => {
                row(
                    ui,
                    &pal,
                    tr("Default font for new text"),
                    tr(
                        "Font, bold and italic used when you add text or a text box. Text you add is stored with an embedded subset of the font, so it looks the same everywhere.",
                    ),
                    |ui| {
                        let mut st = self.prefs.tool_defaults.font_style();
                        ui.horizontal(|ui| {
                            if crate::fontpick::font_picker(ui, "default_font_pick", &mut st) {
                                self.prefs.tool_defaults.set_font_style(st);
                                *changed = true;
                            }
                        });
                    },
                );
            }
            "snap_to_geometry" => {
                row(
                    ui,
                    &pal,
                    tr("Snap to drawing geometry"),
                    tr(
                        "Measurements jump to line ends, corners, intersections and midpoints of the page when the pointer is close.",
                    ),
                    |ui| {
                        *changed |= toggle(ui, &pal, &mut self.prefs.snap_to_geometry).changed();
                    },
                );
            }
            "graphics" => {
                let adapter = self.gpu_info.clone().unwrap_or_else(|| "unknown".into());
                let software = adapter.contains("Cpu")
                    || adapter.contains("llvmpipe")
                    || adapter.contains("WARP")
                    || adapter.contains("Basic Render");
                row(
                    ui,
                    &pal,
                    tr("Graphics (drawing backend, frame pacing)"),
                    tr(
                        "If resizing the window or zooming feels slow, try another drawing API (restart needed) or uncapped frames.",
                    ),
                    |ui| {
                        ui.label(
                            RichText::new(tf!("In use: {}", adapter))
                                .size(12.0)
                                .color(pal.text_dim),
                        );
                    },
                );
                if software {
                    ui.colored_label(pal.danger, tr("This is a software renderer, not a graphics card. Install the graphics driver of your computer's maker; drawing will stay slow until then."));
                }
                row(
                    ui,
                    &pal,
                    tr("Drawing API"),
                    tr("(applies after restart)"),
                    |ui| {
                        egui::ComboBox::from_id_salt("gfx_backend")
                            .selected_text(tr(self.prefs.graphics.backend.title()))
                            .show_ui(ui, |ui| {
                                for b in [
                                    GfxBackend::Auto,
                                    GfxBackend::Dx12,
                                    GfxBackend::Vulkan,
                                    GfxBackend::Gl,
                                ] {
                                    *changed |= ui
                                        .selectable_value(
                                            &mut self.prefs.graphics.backend,
                                            b,
                                            tr(b.title()),
                                        )
                                        .changed();
                                }
                            });
                    },
                );
                row(ui, &pal, tr("Frame pacing"), "", |ui| {
                    egui::ComboBox::from_id_salt("gfx_present")
                        .selected_text(tr(self.prefs.graphics.present.title()))
                        .show_ui(ui, |ui| {
                            for p in [
                                PresentChoice::Smooth,
                                PresentChoice::LowLatency,
                                PresentChoice::Uncapped,
                            ] {
                                *changed |= ui
                                    .selectable_value(
                                        &mut self.prefs.graphics.present,
                                        p,
                                        tr(p.title()),
                                    )
                                    .changed();
                            }
                        });
                });
            }
            "render_cache_mb" => {
                row(
                    ui,
                    &pal,
                    tr("Render cache size"),
                    tr("Memory budget for cached page tiles."),
                    |ui| {
                        *changed |= ui
                            .add(
                                egui::Slider::new(&mut self.prefs.render_cache_mb, 64..=4096)
                                    .suffix(" MiB"),
                            )
                            .changed();
                    },
                );
            }
            "ai" => {
                ui.label(
                    RichText::new(tr("PDF Copilot (AI provider and key)"))
                        .size(14.0)
                        .color(pal.text),
                );
                ui.label(RichText::new(tr("Use your own OpenAI or Anthropic account (or a server of your own) for Copilot and Translate.")).size(12.0).color(pal.text_dim));
                ui.add_space(10.0);
                *changed |= self.ai_prefs_section(ui, ctx);
            }
            "shortcuts" => {
                row(
                    ui,
                    &pal,
                    tr("Keyboard shortcuts"),
                    tr("View and change shortcuts; conflicts are flagged."),
                    |ui| {
                        if ui.button(tr("Customize shortcuts…")).clicked() {
                            out.close = true;
                            self.dialog_next = Some(Dialog::Shortcuts {
                                filter: String::new(),
                                capture: None,
                            });
                        }
                    },
                );
            }
            _ => {}
        }
    }
}

/// Whether a setting applies to this copy of BergPDF: the Microsoft Store updates its own packages, so a copy
/// installed from there has no update setting.
pub fn setting_available(key: &str) -> bool {
    !(key == "updates" && platform::distribution::is_store())
}

/// Whether the search text (lower case) matches a setting's title, description or keywords.
fn matches_filter(key: &str, f: &str) -> bool {
    SETTINGS.iter().any(|s| {
        s.key == key
            && format!(
                "{} {} {} {} {}",
                s.title,
                tr(s.title),
                s.description,
                tr(s.description),
                s.keywords
            )
            .to_lowercase()
            .contains(f)
    })
}

/// One setting: name and explanation at the left, the control at the right, a hairline below.
fn row(
    ui: &mut egui::Ui,
    pal: &Palette,
    title: &str,
    desc: &str,
    control: impl FnOnce(&mut egui::Ui),
) {
    ui.add_space(12.0);
    let full = ui.available_width();
    ui.horizontal(|ui| {
        let left = (full * 0.58).max(180.0);
        ui.allocate_ui_with_layout(Vec2::new(left, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(left);
            ui.label(RichText::new(title).size(14.0).color(pal.text));
            if !desc.is_empty() {
                ui.label(RichText::new(desc).size(12.0).color(pal.text_dim));
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), control);
    });
    ui.add_space(12.0);
    let r = ui.available_rect_before_wrap();
    ui.painter()
        .hline(r.x_range(), r.top(), Stroke::new(1.0, pal.border));
    ui.add_space(1.0);
}

/// A switch like the ones in Claude's settings.
fn toggle(ui: &mut egui::Ui, pal: &Palette, on: &mut bool) -> egui::Response {
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(40.0, 22.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, "")
    });
    let t = ui.ctx().animate_bool(resp.id, *on);
    let off_bg = if pal.dark {
        Color32::from_gray(70)
    } else {
        Color32::from_gray(190)
    };
    let bg = off_bg.lerp_to_gamma(pal.accent, t);
    ui.painter().rect_filled(rect, 11.0, bg);
    let x = egui::lerp((rect.left() + 11.0)..=(rect.right() - 11.0), t);
    ui.painter()
        .circle_filled(egui::pos2(x, rect.center().y), 8.0, Color32::WHITE);
    resp
}

/// Two to three exclusive choices in one rounded strip. Returns whether the value changed.
fn segmented<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    pal: &Palette,
    value: &mut T,
    options: &[(T, &str)],
) -> bool {
    let mut changed = false;
    egui::Frame::new()
        .fill(pal.chrome)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(9)
        .inner_margin(2)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            // right_to_left parent: lay the options out left to right regardless.
            ui.horizontal(|ui| {
                for (v, label) in options {
                    let selected = *value == *v;
                    let b = egui::Button::new(RichText::new(*label).color(if selected {
                        pal.text
                    } else {
                        pal.text_dim
                    }))
                    .fill(if selected {
                        pal.hover
                    } else {
                        Color32::TRANSPARENT
                    })
                    .stroke(Stroke::NONE)
                    .corner_radius(7);
                    if ui.add(b).clicked() && !selected {
                        *value = *v;
                        changed = true;
                    }
                }
            });
        });
    changed
}
