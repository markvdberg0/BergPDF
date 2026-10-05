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

/// Zoom applied when a document is opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DefaultZoom {
    /// Whole page visible.
    #[default]
    FitPage,
    /// Page width fills the window.
    FitWidth,
    /// 100 % (actual size).
    Actual,
}

/// Which graphics API the window draws with (applied at the next start).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GfxBackend {
    /// DirectX 12 on Windows (falling back to the library's choice if it cannot start); the
    /// library's choice elsewhere. The `WGPU_BACKEND` environment variable still overrides.
    #[default]
    Auto,
    /// Direct3D 12 (Windows).
    Dx12,
    /// Vulkan.
    Vulkan,
    /// OpenGL, the most compatible and often the slowest.
    Gl,
}

impl GfxBackend {
    /// Display name.
    pub fn title(self) -> &'static str {
        match self {
            GfxBackend::Auto => "Automatic (DirectX 12 on Windows)",
            GfxBackend::Dx12 => "DirectX 12",
            GfxBackend::Vulkan => "Vulkan",
            GfxBackend::Gl => "OpenGL",
        }
    }
}

/// How finished frames are handed to the screen (applied immediately).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PresentChoice {
    /// Wait for the screen refresh, two frames in flight.
    #[default]
    Smooth,
    /// Wait for the screen refresh, one frame in flight (least input lag).
    LowLatency,
    /// Do not wait for the screen refresh (can tear; sometimes much smoother while resizing).
    Uncapped,
}

impl PresentChoice {
    /// Display name.
    pub fn title(self) -> &'static str {
        match self {
            PresentChoice::Smooth => "Smooth (vsync)",
            PresentChoice::LowLatency => "Low latency (vsync)",
            PresentChoice::Uncapped => "Uncapped (no vsync)",
        }
    }
}

/// Graphics settings; they only change how the window is drawn, never documents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GraphicsPrefs {
    pub backend: GfxBackend,
    pub present: PresentChoice,
}

/// The language of the interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Language {
    /// Use the operating system's language when BergPDF has it, otherwise English.
    #[default]
    System,
    English,
    Dutch,
    German,
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
    /// Font family for new text (`FontFamily::key`).
    pub font_family: String,
    /// Bold by default.
    pub font_bold: bool,
    /// Italic by default (ignored for families without italics).
    pub font_italic: bool,
}

impl ToolDefaults {
    /// The default font for new text and text boxes.
    pub fn font_style(&self) -> pdf_engine::fontembed::FontStyle {
        use pdf_engine::fontembed::{FontFamily, FontStyle};
        FontStyle::new(
            FontFamily::from_key(&self.font_family).unwrap_or(FontFamily::DejaVuSans),
            self.font_bold,
            self.font_italic,
        )
    }

    /// Remember `s` as the default.
    pub fn set_font_style(&mut self, s: pdf_engine::fontembed::FontStyle) {
        self.font_family = s.family.key().to_string();
        self.font_bold = s.bold;
        self.font_italic = s.italic;
    }
}

impl Default for ToolDefaults {
    fn default() -> Self {
        Self {
            highlight: [1.0, 0.92, 0.0],
            stroke: [0.85, 0.1, 0.1],
            stroke_width: 1.5,
            opacity: 1.0,
            font_size: 12.0,
            font_family: "DejaVuSans".to_string(),
            font_bold: false,
            font_italic: false,
        }
    }
}

/// Which AI service PDF Copilot talks to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AiProvider {
    /// OpenAI chat completions (or any service speaking that protocol).
    #[default]
    OpenAi,
    /// Anthropic messages API.
    Anthropic,
    /// A server of your choice that speaks the OpenAI chat protocol (for example a local one).
    Custom,
}

impl AiProvider {
    /// Display name.
    pub fn title(self) -> &'static str {
        match self {
            AiProvider::OpenAi => "OpenAI",
            AiProvider::Anthropic => "Anthropic (Claude)",
            AiProvider::Custom => "Custom (OpenAI-compatible)",
        }
    }

    /// Default endpoint root.
    pub fn default_base_url(self) -> &'static str {
        match self {
            AiProvider::OpenAi => "https://api.openai.com/v1",
            AiProvider::Anthropic => "https://api.anthropic.com/v1",
            AiProvider::Custom => "http://localhost:11434/v1",
        }
    }

    /// Default model name: a relatively cheap one (the user can change it).
    pub fn default_model(self) -> &'static str {
        match self {
            AiProvider::OpenAi => "gpt-4.1-mini",
            AiProvider::Anthropic => "claude-haiku-4-5-20251001",
            AiProvider::Custom => "llama3.1",
        }
    }

    /// Models offered in the dropdown: (id, short description). Any other id can be typed in.
    pub fn models(self) -> &'static [(&'static str, &'static str)] {
        match self {
            AiProvider::OpenAi => &[
                ("gpt-4.1-nano", "lowest cost"),
                ("gpt-4.1-mini", "low cost"),
                ("gpt-4o-mini", "low cost"),
                ("gpt-4.1", "more capable"),
                ("gpt-5-mini", "reasoning, slower"),
                ("gpt-5", "reasoning, most capable, slower"),
            ],
            AiProvider::Anthropic => &[
                ("claude-haiku-4-5-20251001", "Haiku 4.5 - fast, low cost"),
                ("claude-sonnet-5-5", "Sonnet 5.5 - balanced"),
                ("claude-opus-5-5", "Opus 5.5 - most capable"),
                ("claude-fable-5-1", "Fable 5.1"),
            ],
            AiProvider::Custom => &[],
        }
    }
}

/// PDF Copilot settings. The API key itself is **not** here: it lives in its own file
/// (`platform::secrets`) so this file can be shared or backed up without leaking it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    pub provider: AiProvider,
    /// Model name; empty means the provider default.
    pub model: String,
    /// Endpoint root; empty means the provider default.
    pub base_url: String,
    /// Language Copilot answers in; empty means "the language of the question".
    pub answer_language: String,
    /// Language the Translate command targets by default.
    pub translate_to: String,
    /// Characters of document text sent per request (bounds cost and what leaves the machine).
    pub max_chars: u32,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            provider: AiProvider::OpenAi,
            model: String::new(),
            base_url: String::new(),
            answer_language: String::new(),
            translate_to: "English".to_string(),
            max_chars: 60_000,
        }
    }
}

impl AiSettings {
    /// Effective model name.
    pub fn model(&self) -> &str {
        if self.model.trim().is_empty() {
            self.provider.default_model()
        } else {
            self.model.trim()
        }
    }

    /// Effective endpoint root without a trailing slash.
    pub fn base_url(&self) -> String {
        let b = if self.base_url.trim().is_empty() {
            self.provider.default_base_url()
        } else {
            self.base_url.trim()
        };
        b.trim_end_matches('/').to_string()
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
    /// Zoom used when opening a document.
    pub default_zoom: DefaultZoom,
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
    /// Graphics API and frame presentation.
    pub graphics: GraphicsPrefs,
    /// Interface language.
    pub language: Language,
    /// Snap measurement points to line ends, corners, intersections and midpoints of the page.
    pub snap_to_geometry: bool,
    /// Whether the PDF Copilot panel is open.
    pub show_copilot: bool,
    /// PDF Copilot / translation settings (the key is stored separately).
    pub ai: AiSettings,
    /// The provider the user has agreed to send document text to; asked again when it changes.
    pub ai_consent_provider: Option<AiProvider>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            density: Density::Comfortable,
            ui_scale: 1.0,
            workspace: Workspace::Professional,
            default_zoom: DefaultZoom::FitPage,
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
            right_sidebar_width: 300.0,
            render_cache_mb: 384,
            graphics: GraphicsPrefs::default(),
            language: Language::default(),
            snap_to_geometry: true,
            show_copilot: false,
            ai: AiSettings::default(),
            ai_consent_provider: None,
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
        key: "default_font",
        title: "Default font for new text",
        description: "Font, bold and italic used when you add text or a text box.",
        keywords: "font typeface family serif sans mono bold italic text box liberation dejavu arial times courier",
    },
    SettingInfo {
        key: "snap_to_geometry",
        title: "Snap to drawing geometry",
        description: "Measurements snap to line ends, corners, intersections and midpoints.",
        keywords: "snap magnet cad corner endpoint intersection midpoint measure",
    },
    SettingInfo {
        key: "language",
        title: "Language",
        description: "Interface language: English, Nederlands or Deutsch (or follow the system).",
        keywords: "language taal sprache dutch nederlands german deutsch english translate interface",
    },
    SettingInfo {
        key: "graphics",
        title: "Graphics (drawing backend, frame pacing)",
        description: "Shows which graphics adapter is used and lets you try another API or uncapped frames if resizing or zooming feels slow.",
        keywords: "graphics gpu backend directx dx12 vulkan opengl vsync slow resize lag performance adapter",
    },
    SettingInfo {
        key: "ai",
        title: "PDF Copilot (AI provider and key)",
        description: "Choose OpenAI, Anthropic or your own server and store the API key on this computer.",
        keywords: "ai api key openai claude anthropic copilot chat summarize translate model",
    },
    SettingInfo {
        key: "default_zoom",
        title: "Zoom when opening a document",
        description: "Fit page, fit width or actual size (100 %).",
        keywords: "zoom open fit page width actual size 100",
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
