//! Persistent preferences (serialised as TOML; file location is chosen by the platform layer).

use crate::command::{CommandId, KeyBindings};
use serde::{Deserialize, Serialize};

/// Application chrome theme. Never affects document colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeChoice {
    /// Follow the OS.
    #[default]
    System,
    Light,
    Dark,
}

/// UI density.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Density {
    Compact,
    #[default]
    Comfortable,
    /// Larger hit targets for touch and pen.
    Touch,
}

/// Workspace complexity. Essential hides advanced panels; it never changes documents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Workspace {
    Essential,
    #[default]
    Professional,
}

/// Defaults applied to newly created annotations.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolDefaults {
    /// Highlight colour (RGB 0..1).
    pub highlight: [f32; 3],
    /// Stroke colour for shapes, lines and ink.
    pub stroke: [f32; 3],
    /// Stroke width in points.
    pub stroke_width: f64,
    /// Annotation opacity.
    pub opacity: f64,
    /// FreeText font size.
    pub font_size: f64,
}

impl Default for ToolDefaults {
    fn default() -> Self {
        Self {
            highlight: [1.0, 0.92, 0.0],
            stroke: [0.85, 0.1, 0.1],
            stroke_width: 1.5,
            opacity: 1.0,
            font_size: 12.0,
        }
    }
}

/// All persisted preferences.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub theme: ThemeChoice,
    pub density: Density,
    /// UI scale for chrome (icons, hit targets, text); independent of PDF rendering.
    pub ui_scale: f32,
    pub workspace: Workspace,
    /// Whether the first-run workspace choice has been made.
    pub first_run_done: bool,
    /// Comfortable dark reading filter for page bitmaps (display only).
    pub dark_page_filter: bool,
    /// Author name stamped on new annotations.
    pub author: String,
    pub recent_files: Vec<String>,
    pub keybindings: KeyBindings,
    pub favorites: Vec<CommandId>,
    pub recent_tools: Vec<CommandId>,
    pub tool_defaults: ToolDefaults,
    pub show_left_sidebar: bool,
    pub show_right_sidebar: bool,
    pub left_sidebar_width: f32,
    pub right_sidebar_width: f32,
    /// Render cache budget in MiB.
    pub render_cache_mb: u32,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            density: Density::Comfortable,
            ui_scale: 1.0,
            workspace: Workspace::Professional,
            first_run_done: false,
            dark_page_filter: false,
            author: String::new(),
            recent_files: Vec::new(),
            keybindings: KeyBindings::default(),
            favorites: vec![
                CommandId::ToolHighlight,
                CommandId::ToolFreeText,
                CommandId::ToolRectangle,
            ],
            recent_tools: Vec::new(),
            tool_defaults: ToolDefaults::default(),
            show_left_sidebar: true,
            show_right_sidebar: true,
            left_sidebar_width: 190.0,
            right_sidebar_width: 260.0,
            render_cache_mb: 384,
        }
    }
}

impl Preferences {
    /// Parse TOML; unknown/missing fields fall back to defaults.
    pub fn from_toml(s: &str) -> Result<Self, String> {
        toml::from_str(s).map_err(|e| e.to_string())
    }

    /// Serialise to TOML.
    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|e| e.to_string())
    }

    /// Record a file as most-recent (deduplicated, capped).
    pub fn push_recent(&mut self, path: &str) {
        self.recent_files.retain(|p| p != path);
        self.recent_files.insert(0, path.to_string());
        self.recent_files.truncate(12);
    }

    /// Record a tool as recently used.
    pub fn push_recent_tool(&mut self, c: CommandId) {
        self.recent_tools.retain(|x| *x != c);
        self.recent_tools.insert(0, c);
        self.recent_tools.truncate(6);
    }

    /// Clamp values to sane ranges after loading a possibly hand-edited file.
    pub fn sanitize(&mut self) {
        self.ui_scale = self.ui_scale.clamp(0.75, 2.5);
        self.left_sidebar_width = self.left_sidebar_width.clamp(120.0, 480.0);
        self.right_sidebar_width = self.right_sidebar_width.clamp(160.0, 520.0);
        self.render_cache_mb = self.render_cache_mb.clamp(64, 4096);
        self.tool_defaults.opacity = self.tool_defaults.opacity.clamp(0.05, 1.0);
        self.tool_defaults.stroke_width = self.tool_defaults.stroke_width.clamp(0.0, 50.0);
        self.tool_defaults.font_size = self.tool_defaults.font_size.clamp(4.0, 200.0);
    }
}

/// A searchable setting (for the preferences search box and the command palette).
pub struct SettingInfo {
    /// Stable key.
    pub key: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub keywords: &'static str,
}

/// All settings exposed in Preferences.
pub static SETTINGS: &[SettingInfo] = &[
    SettingInfo {
        key: "theme",
        title: "Application theme",
        description: "Light, Dark or follow the system. Never changes document colours.",
        keywords: "dark mode light appearance chrome",
    },
    SettingInfo {
        key: "dark_page_filter",
        title: "Dark page view",
        description: "Comfortable dark reading filter for pages. Display only; the PDF is never modified.",
        keywords: "night invert reading accessibility",
    },
    SettingInfo {
        key: "density",
        title: "Interface density",
        description: "Compact, Comfortable or Touch/Pen hit targets.",
        keywords: "size spacing touch pen compact",
    },
    SettingInfo {
        key: "ui_scale",
        title: "Interface scale",
        description: "Scale toolbar icons and text independently of page rendering.",
        keywords: "icons zoom 4k retina hidpi toolbar size",
    },
    SettingInfo {
        key: "workspace",
        title: "Workspace",
        description: "Essential shows the common tools; Professional shows everything.",
        keywords: "simple advanced essential professional",
    },
    SettingInfo {
        key: "author",
        title: "Author name",
        description: "Name stored on comments and markup you create.",
        keywords: "comments user name",
    },
    SettingInfo {
        key: "shortcuts",
        title: "Keyboard shortcuts",
        description: "View and change shortcuts; conflicts are flagged.",
        keywords: "keys bindings hotkeys",
    },
    SettingInfo {
        key: "tool_defaults",
        title: "Markup defaults",
        description: "Default colours, line width, opacity and font size for new annotations.",
        keywords: "color colour width opacity",
    },
    SettingInfo {
        key: "render_cache_mb",
        title: "Render cache size",
        description: "Memory budget for cached page tiles.",
        keywords: "memory performance cache",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_roundtrip_and_tolerance() {
        let mut p = Preferences {
            author: "Ada".into(),
            theme: ThemeChoice::Dark,
            ..Default::default()
        };
        p.push_recent("/a.pdf");
        p.push_recent("/b.pdf");
        p.push_recent("/a.pdf");
        assert_eq!(p.recent_files, vec!["/a.pdf", "/b.pdf"]);
        p.keybindings.set(
            CommandId::FileOpen,
            vec![crate::command::Shortcut::cmd(crate::command::Key::Char(
                'L',
            ))],
        );
        let s = p.to_toml().unwrap();
        let q = Preferences::from_toml(&s).unwrap();
        assert_eq!(q.author, "Ada");
        assert_eq!(q.theme, ThemeChoice::Dark);
        assert_eq!(q.keybindings.overrides.len(), 1);
        // Partial / unknown-field files still load.
        let r = Preferences::from_toml("author = \"X\"\nunknown_key = 3\n").unwrap();
        assert_eq!(r.author, "X");
        assert_eq!(r.ui_scale, 1.0);
        // Garbage is an error, not a panic.
        assert!(Preferences::from_toml("= = =").is_err());
    }

    #[test]
    fn sanitize_clamps() {
        let mut p = Preferences {
            ui_scale: 99.0,
            render_cache_mb: 1,
            ..Default::default()
        };
        p.sanitize();
        assert_eq!(p.ui_scale, 2.5);
        assert_eq!(p.render_cache_mb, 64);
    }

    #[test]
    fn theme_change_does_not_touch_document_related_prefs() {
        // The dark *chrome* theme and the dark *page filter* are independent switches.
        let p = Preferences {
            theme: ThemeChoice::Dark,
            ..Preferences::default()
        };
        assert!(!p.dark_page_filter);
    }
}
