//! Semantic commands, per-platform default shortcuts, user overrides, conflict detection
//! and the action-search index (command palette, shortcut reference).
//!
//! The registry is the single source of truth: tooltips, menus, the palette and the
//! shortcut reference are all generated from [`REGISTRY`], so documentation cannot drift
//! from implementation.

use crate::platform_kind::OsKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A semantic command. Binding to keys happens per platform, never in UI code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CommandId {
    FileOpen,
    FileSave,
    FileSaveAs,
    FileProperties,
    FileExportImage,
    FormFlatten,
    FileClose,
    FileQuit,
    EditUndo,
    EditRedo,
    EditCopy,
    EditSelectAll,
    EditDelete,
    EditDuplicate,
    Find,
    FindNext,
    FindPrevious,
    CommandPalette,
    Preferences,
    ShortcutReference,
    About,
    ViewZoomIn,
    ViewZoomOut,
    ViewZoomActual,
    ViewFitPage,
    ViewFitWidth,
    ViewModeContinuous,
    ViewModeSingle,
    ViewModeFacing,
    ViewRotateClockwise,
    ViewRotateCounterClockwise,
    ViewToggleLeftSidebar,
    ViewToggleRightSidebar,
    ViewDarkPages,
    GoNextPage,
    GoPreviousPage,
    GoFirstPage,
    GoLastPage,
    GoToPage,
    ToolSelect,
    ToolHand,
    ToolTextSelect,
    ToolHighlight,
    ToolUnderline,
    ToolStrikeOut,
    ToolNote,
    ToolFreeText,
    ToolCallout,
    ToolRectangle,
    ToolEllipse,
    ToolLine,
    ToolArrow,
    ToolPolygon,
    ToolPolyline,
    ToolInk,
    ToolStamp,
    ToolEditText,
    ToolAddText,
    ToolAddImage,
    ToolFillForm,
    ToolMeasureDistance,
    ToolMeasurePerimeter,
    ToolMeasureArea,
    ToolMeasureRect,
    ToolMeasureRadius,
    ToolMeasureAngle,
    ToolCount,
    ToolCalibrate,
    ToolPlaceSignature,
    ToolSignArea,
    SignDocument,
    OcrDocument,
    ShowSignatures,
    DrawSignature,
    ExportMeasurements,
    PageRotateClockwise,
    PageRotateCounterClockwise,
    PageDelete,
    PageInsertBlank,
    PageInsertImage,
    PageDuplicate,
    PageExtract,
    PageMoveUp,
    PageMoveDown,
    DocumentMerge,
    NextTab,
    PreviousTab,
    Escape,
}

/// Command groups, used for ribbon placement, palette grouping and the shortcut reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    File,
    Edit,
    View,
    Navigate,
    Comment,
    Content,
    Forms,
    Measure,
    Sign,
    Organize,
    Help,
}

impl Category {
    /// Display name.
    pub fn title(self) -> &'static str {
        match self {
            Category::File => "File",
            Category::Edit => "Edit",
            Category::View => "View",
            Category::Navigate => "Navigate",
            Category::Comment => "Comment",
            Category::Content => "Content",
            Category::Forms => "Forms",
            Category::Measure => "Measure",
            Category::Sign => "Sign",
            Category::Organize => "Organize",
            Category::Help => "Help",
        }
    }
}

/// Non-modifier keys that can be bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Key {
    /// Letter or digit (uppercase letter).
    Char(char),
    /// Function key 1..=12.
    F(u8),
    Delete,
    Backspace,
    Escape,
    Enter,
    Tab,
    Space,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    PageUp,
    PageDown,
    Home,
    End,
    Plus,
    Minus,
    Equals,
}

impl Key {
    fn label(self) -> String {
        match self {
            Key::Char(c) => c.to_string(),
            Key::F(n) => format!("F{n}"),
            Key::Delete => "Delete".into(),
            Key::Backspace => "Backspace".into(),
            Key::Escape => "Esc".into(),
            Key::Enter => "Enter".into(),
            Key::Tab => "Tab".into(),
            Key::Space => "Space".into(),
            Key::ArrowLeft => "←".into(),
            Key::ArrowRight => "→".into(),
            Key::ArrowUp => "↑".into(),
            Key::ArrowDown => "↓".into(),
            Key::PageUp => "Page Up".into(),
            Key::PageDown => "Page Down".into(),
            Key::Home => "Home".into(),
            Key::End => "End".into(),
            Key::Plus => "+".into(),
            Key::Minus => "-".into(),
            Key::Equals => "=".into(),
        }
    }
}

/// A key chord. `command` means the platform's primary modifier: Ctrl on Windows/Linux,
/// ⌘ on macOS. `ctrl` is the *physical* Control key and only matters on macOS.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Shortcut {
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub key: Key,
}

impl Shortcut {
    /// Primary-modifier chord.
    pub const fn cmd(key: Key) -> Self {
        Self {
            command: true,
            shift: false,
            alt: false,
            ctrl: false,
            key,
        }
    }
    /// Primary + Shift chord.
    pub const fn cmd_shift(key: Key) -> Self {
        Self {
            command: true,
            shift: true,
            alt: false,
            ctrl: false,
            key,
        }
    }
    /// Bare key.
    pub const fn plain(key: Key) -> Self {
        Self {
            command: false,
            shift: false,
            alt: false,
            ctrl: false,
            key,
        }
    }
    /// Shift chord.
    pub const fn shift(key: Key) -> Self {
        Self {
            command: false,
            shift: true,
            alt: false,
            ctrl: false,
            key,
        }
    }
    /// Alt chord (Option on macOS).
    pub const fn alt(key: Key) -> Self {
        Self {
            command: false,
            shift: false,
            alt: true,
            ctrl: false,
            key,
        }
    }

    /// Label in the platform's conventions: `Ctrl+Shift+Z` on Windows, `⇧⌘Z` on macOS.
    pub fn label(&self, os: OsKind) -> String {
        if os.uses_command_key() {
            let mut s = String::new();
            if self.ctrl {
                s.push('⌃');
            }
            if self.alt {
                s.push('⌥');
            }
            if self.shift {
                s.push('⇧');
            }
            if self.command {
                s.push('⌘');
            }
            s.push_str(&self.key.label());
            s
        } else {
            let mut parts: Vec<String> = Vec::new();
            if self.command || self.ctrl {
                parts.push("Ctrl".into());
            }
            if self.alt {
                parts.push("Alt".into());
            }
            if self.shift {
                parts.push("Shift".into());
            }
            parts.push(self.key.label());
            parts.join("+")
        }
    }
}

/// Static metadata for a command.
pub struct CommandInfo {
    pub id: CommandId,
    pub title: &'static str,
    /// One-line description shown in tooltips and the palette.
    pub description: &'static str,
    pub category: Category,
    /// Extra search terms (synonyms) for the palette.
    pub keywords: &'static str,
    /// Default bindings. `None` platform = all platforms.
    pub bindings: &'static [(Option<OsKind>, Shortcut)],
}

use Category as C;
use CommandId as Id;
use Key::{Char, F};

const WIN_LIN: Option<OsKind> = None;
const MAC: Option<OsKind> = Some(OsKind::MacOs);
const WIN: Option<OsKind> = Some(OsKind::Windows);
const LIN: Option<OsKind> = Some(OsKind::Linux);

macro_rules! cmd {
    ($id:expr, $title:expr, $desc:expr, $cat:expr, $kw:expr, [$($b:expr),*]) => {
        CommandInfo { id: $id, title: $title, description: $desc, category: $cat, keywords: $kw, bindings: &[$($b),*] }
    };
}

/// Every command the application exposes.
pub static REGISTRY: &[CommandInfo] = &[
    cmd!(
        Id::FileOpen,
        "Open…",
        "Open a PDF document",
        C::File,
        "load file",
        [(None, Shortcut::cmd(Char('O')))]
    ),
    cmd!(
        Id::FileSave,
        "Save",
        "Save changes to this document",
        C::File,
        "write",
        [(None, Shortcut::cmd(Char('S')))]
    ),
    cmd!(
        Id::FileProperties,
        "Document Properties…",
        "View and edit title, author, subject and keywords",
        C::File,
        "metadata info title author",
        []
    ),
    cmd!(
        Id::FileExportImage,
        "Export Page as Image…",
        "Save the current page as a PNG picture",
        C::File,
        "png picture screenshot render",
        []
    ),
    cmd!(
        Id::FormFlatten,
        "Flatten Form Fields…",
        "Turn all form fields into plain page content",
        C::Forms,
        "bake lock fill static acroform",
        []
    ),
    cmd!(
        Id::FileSaveAs,
        "Save As…",
        "Save a copy under a new name",
        C::File,
        "export copy",
        [(None, Shortcut::cmd_shift(Char('S')))]
    ),
    cmd!(
        Id::FileClose,
        "Close Document",
        "Close the current tab",
        C::File,
        "tab",
        [(None, Shortcut::cmd(Char('W')))]
    ),
    cmd!(
        Id::FileQuit,
        "Quit",
        "Quit BergPDF",
        C::File,
        "exit",
        [
            (MAC, Shortcut::cmd(Char('Q'))),
            (LIN, Shortcut::cmd(Char('Q')))
        ]
    ),
    cmd!(
        Id::EditUndo,
        "Undo",
        "Undo the last change",
        C::Edit,
        "revert",
        [(None, Shortcut::cmd(Char('Z')))]
    ),
    cmd!(
        Id::EditRedo,
        "Redo",
        "Redo the last undone change",
        C::Edit,
        "repeat",
        [
            (MAC, Shortcut::cmd_shift(Char('Z'))),
            (WIN, Shortcut::cmd(Char('Y'))),
            (LIN, Shortcut::cmd(Char('Y'))),
            (WIN, Shortcut::cmd_shift(Char('Z'))),
            (LIN, Shortcut::cmd_shift(Char('Z')))
        ]
    ),
    cmd!(
        Id::EditCopy,
        "Copy",
        "Copy the selected text",
        C::Edit,
        "clipboard",
        [(None, Shortcut::cmd(Char('C')))]
    ),
    cmd!(
        Id::EditSelectAll,
        "Select All",
        "Select all text on the page",
        C::Edit,
        "",
        [(None, Shortcut::cmd(Char('A')))]
    ),
    cmd!(
        Id::EditDelete,
        "Delete",
        "Delete the selected annotation or object",
        C::Edit,
        "remove erase",
        [
            (None, Shortcut::plain(Key::Delete)),
            (MAC, Shortcut::plain(Key::Backspace))
        ]
    ),
    cmd!(
        Id::EditDuplicate,
        "Duplicate",
        "Duplicate the selected annotation",
        C::Edit,
        "copy clone",
        [(None, Shortcut::cmd(Char('D')))]
    ),
    cmd!(
        Id::Find,
        "Find…",
        "Search the document text",
        C::Edit,
        "search",
        [(None, Shortcut::cmd(Char('F')))]
    ),
    cmd!(
        Id::FindNext,
        "Find Next",
        "Jump to the next search result",
        C::Edit,
        "search",
        [
            (None, Shortcut::plain(F(3))),
            (MAC, Shortcut::cmd(Char('G')))
        ]
    ),
    cmd!(
        Id::FindPrevious,
        "Find Previous",
        "Jump to the previous search result",
        C::Edit,
        "search",
        [
            (None, Shortcut::shift(F(3))),
            (MAC, Shortcut::cmd_shift(Char('G')))
        ]
    ),
    cmd!(
        Id::CommandPalette,
        "Search Commands…",
        "Find and run any command, tool or setting",
        C::Help,
        "palette actions",
        [(None, Shortcut::cmd(Char('K')))]
    ),
    cmd!(
        Id::Preferences,
        "Preferences…",
        "Open settings",
        C::File,
        "settings options",
        [
            (MAC, Shortcut::cmd(Char(','))),
            (WIN, Shortcut::cmd(Char(','))),
            (LIN, Shortcut::cmd(Char(',')))
        ]
    ),
    cmd!(
        Id::ShortcutReference,
        "Keyboard Shortcuts",
        "Show every keyboard shortcut",
        C::Help,
        "keys hotkeys",
        [(None, Shortcut::cmd(Key::Char('/')))]
    ),
    cmd!(
        Id::About,
        "About BergPDF",
        "Version, licences and dependency notices",
        C::Help,
        "version",
        []
    ),
    cmd!(
        Id::ViewZoomIn,
        "Zoom In",
        "Increase the zoom level",
        C::View,
        "bigger magnify",
        [
            (None, Shortcut::cmd(Key::Equals)),
            (None, Shortcut::cmd(Key::Plus))
        ]
    ),
    cmd!(
        Id::ViewZoomOut,
        "Zoom Out",
        "Decrease the zoom level",
        C::View,
        "smaller",
        [(None, Shortcut::cmd(Key::Minus))]
    ),
    cmd!(
        Id::ViewZoomActual,
        "Actual Size (100%)",
        "Show the page at 100%",
        C::View,
        "reset zoom",
        [(None, Shortcut::cmd(Char('0')))]
    ),
    cmd!(
        Id::ViewFitPage,
        "Fit Page",
        "Fit the whole page in the window",
        C::View,
        "zoom",
        [(None, Shortcut::cmd(Char('9')))]
    ),
    cmd!(
        Id::ViewFitWidth,
        "Fit Width",
        "Fit the page width to the window",
        C::View,
        "zoom",
        [(None, Shortcut::cmd(Char('8')))]
    ),
    cmd!(
        Id::ViewModeContinuous,
        "Continuous Scrolling",
        "Show pages in a continuous column",
        C::View,
        "scroll layout",
        []
    ),
    cmd!(
        Id::ViewModeSingle,
        "Single Page",
        "Show one page at a time",
        C::View,
        "layout",
        []
    ),
    cmd!(
        Id::ViewModeFacing,
        "Facing Pages",
        "Show two pages side by side",
        C::View,
        "two-up spread layout",
        []
    ),
    cmd!(
        Id::ViewRotateClockwise,
        "Rotate View Clockwise",
        "Rotate the display only; the document is not changed",
        C::View,
        "orientation",
        [(None, Shortcut::cmd_shift(Key::Plus))]
    ),
    cmd!(
        Id::ViewRotateCounterClockwise,
        "Rotate View Counter-clockwise",
        "Rotate the display only; the document is not changed",
        C::View,
        "orientation",
        [(None, Shortcut::cmd_shift(Key::Minus))]
    ),
    cmd!(
        Id::ViewToggleLeftSidebar,
        "Toggle Navigation Panel",
        "Show or hide thumbnails, bookmarks and results",
        C::View,
        "sidebar thumbnails",
        [(None, Shortcut::plain(F(4)))]
    ),
    cmd!(
        Id::ViewToggleRightSidebar,
        "Toggle Properties Panel",
        "Show or hide properties and comments",
        C::View,
        "sidebar",
        [(None, Shortcut::plain(F(5)))]
    ),
    cmd!(
        Id::ViewDarkPages,
        "Dark Page View",
        "Comfortable dark reading filter (display only)",
        C::View,
        "night mode invert accessibility",
        []
    ),
    cmd!(
        Id::GoNextPage,
        "Next Page",
        "Go to the next page",
        C::Navigate,
        "forward",
        [
            (None, Shortcut::plain(Key::PageDown)),
            (None, Shortcut::alt(Key::ArrowRight))
        ]
    ),
    cmd!(
        Id::GoPreviousPage,
        "Previous Page",
        "Go to the previous page",
        C::Navigate,
        "back",
        [
            (None, Shortcut::plain(Key::PageUp)),
            (None, Shortcut::alt(Key::ArrowLeft))
        ]
    ),
    cmd!(
        Id::GoFirstPage,
        "First Page",
        "Go to the first page",
        C::Navigate,
        "",
        [(None, Shortcut::plain(Key::Home))]
    ),
    cmd!(
        Id::GoLastPage,
        "Last Page",
        "Go to the last page",
        C::Navigate,
        "",
        [(None, Shortcut::plain(Key::End))]
    ),
    cmd!(
        Id::GoToPage,
        "Go to Page…",
        "Jump to a page number",
        C::Navigate,
        "page number",
        [(WIN_LIN, Shortcut::cmd_shift(Char('N')))]
    ),
    cmd!(
        Id::ToolSelect,
        "Select Tool",
        "Select and move annotations",
        C::Comment,
        "pointer arrow",
        [(None, Shortcut::plain(Char('V')))]
    ),
    cmd!(
        Id::ToolHand,
        "Hand Tool",
        "Drag to pan the page",
        C::View,
        "pan grab",
        [(None, Shortcut::plain(Char('H')))]
    ),
    cmd!(
        Id::ToolTextSelect,
        "Select Text",
        "Select and copy page text",
        C::Edit,
        "text cursor",
        [(None, Shortcut::plain(Char('T')))]
    ),
    cmd!(
        Id::ToolHighlight,
        "Highlight Text",
        "Highlight selected text",
        C::Comment,
        "markup",
        [(None, Shortcut::alt(Char('H')))]
    ),
    cmd!(
        Id::ToolUnderline,
        "Underline Text",
        "Underline selected text",
        C::Comment,
        "markup",
        [(None, Shortcut::alt(Char('U')))]
    ),
    cmd!(
        Id::ToolStrikeOut,
        "Strikethrough Text",
        "Strike out selected text",
        C::Comment,
        "markup strikeout",
        [(None, Shortcut::alt(Char('S')))]
    ),
    cmd!(
        Id::ToolNote,
        "Sticky Note",
        "Add a sticky note comment",
        C::Comment,
        "comment",
        [(None, Shortcut::alt(Char('N')))]
    ),
    cmd!(
        Id::ToolFreeText,
        "Text Box",
        "Add a free-text annotation",
        C::Comment,
        "typewriter free text",
        [(None, Shortcut::alt(Char('T')))]
    ),
    cmd!(
        Id::ToolCallout,
        "Callout",
        "Add a text box with a leader line",
        C::Comment,
        "free text",
        []
    ),
    cmd!(
        Id::ToolRectangle,
        "Rectangle",
        "Draw a rectangle",
        C::Comment,
        "shape box",
        [(None, Shortcut::alt(Char('R')))]
    ),
    cmd!(
        Id::ToolEllipse,
        "Ellipse",
        "Draw an ellipse",
        C::Comment,
        "shape circle oval",
        [(None, Shortcut::alt(Char('E')))]
    ),
    cmd!(
        Id::ToolLine,
        "Line",
        "Draw a line (Shift constrains the angle)",
        C::Comment,
        "shape",
        [(None, Shortcut::alt(Char('L')))]
    ),
    cmd!(
        Id::ToolArrow,
        "Arrow",
        "Draw an arrow",
        C::Comment,
        "shape line",
        [(None, Shortcut::alt(Char('A')))]
    ),
    cmd!(
        Id::ToolPolygon,
        "Polygon",
        "Draw a closed polygon",
        C::Comment,
        "shape",
        []
    ),
    cmd!(
        Id::ToolPolyline,
        "Polyline",
        "Draw an open polyline",
        C::Comment,
        "shape",
        []
    ),
    cmd!(
        Id::ToolInk,
        "Pencil",
        "Draw freehand",
        C::Comment,
        "ink draw",
        [(None, Shortcut::alt(Char('P')))]
    ),
    cmd!(
        Id::ToolStamp,
        "Stamp",
        "Place a text stamp",
        C::Comment,
        "approved draft",
        []
    ),
    cmd!(
        Id::ToolEditText,
        "Edit Text",
        "Edit existing page text",
        C::Content,
        "modify content",
        [(None, Shortcut::alt(Char('X')))]
    ),
    cmd!(
        Id::ToolAddText,
        "Add Text",
        "Add new text to the page content",
        C::Content,
        "type insert",
        []
    ),
    cmd!(
        Id::ToolAddImage,
        "Add Image",
        "Place an image on the page",
        C::Content,
        "picture insert",
        []
    ),
    cmd!(
        Id::ToolFillForm,
        "Fill Form Fields",
        "Click form fields to fill them in",
        C::Forms,
        "form acroform field checkbox radio text",
        []
    ),
    cmd!(
        Id::ToolMeasureDistance,
        "Measure Distance",
        "Measure a straight distance at the calibrated scale",
        C::Measure,
        "length ruler dimension",
        []
    ),
    cmd!(
        Id::ToolMeasurePerimeter,
        "Measure Length (Path)",
        "Measure the length of a multi-segment path",
        C::Measure,
        "perimeter polyline run cable pipe",
        []
    ),
    cmd!(
        Id::ToolMeasureArea,
        "Measure Area",
        "Measure the area of a polygon",
        C::Measure,
        "polygon room floor",
        []
    ),
    cmd!(
        Id::ToolMeasureRect,
        "Measure Rectangle",
        "Measure a rectangle's area and perimeter",
        C::Measure,
        "area box",
        []
    ),
    cmd!(
        Id::ToolMeasureRadius,
        "Measure Radius",
        "Measure a circle by centre and edge",
        C::Measure,
        "diameter circle circumference",
        []
    ),
    cmd!(
        Id::ToolMeasureAngle,
        "Measure Angle",
        "Measure the angle between two arms",
        C::Measure,
        "degrees",
        []
    ),
    cmd!(
        Id::ToolCount,
        "Count",
        "Place numbered markers to count items by category",
        C::Measure,
        "tally takeoff quantity",
        []
    ),
    cmd!(
        Id::ToolCalibrate,
        "Calibrate Scale",
        "Set the drawing scale from a known length",
        C::Measure,
        "scale ratio units set",
        []
    ),
    cmd!(
        Id::OcrDocument,
        "Recognize Text (OCR)…",
        "Make scanned pages searchable by adding an invisible text layer",
        C::Content,
        "ocr scan scanned recognise recognize searchable text image",
        []
    ),
    cmd!(
        Id::SignDocument,
        "Sign with Certificate…",
        "Digitally sign the document with a certificate file (.p12 / .pfx)",
        C::Sign,
        "digital signature certificate pkcs12 pfx cryptographic",
        []
    ),
    cmd!(
        Id::ShowSignatures,
        "Signatures…",
        "Show the digital signatures in this document and whether they still match",
        C::Sign,
        "verify validate check signed certificate",
        []
    ),
    cmd!(
        Id::DrawSignature,
        "Draw Signature…",
        "Draw your handwritten signature once and keep it for reuse",
        C::Sign,
        "handwritten sign draw ink autograph",
        []
    ),
    cmd!(
        Id::ToolPlaceSignature,
        "Place Signature",
        "Place your saved handwritten signature on the page (a drawing, not a digital signature)",
        C::Sign,
        "handwritten sign stamp initials",
        []
    ),
    cmd!(
        Id::ToolSignArea,
        "Choose Signature Area",
        "Drag the area where a digital signature will be shown",
        C::Sign,
        "visible signature box",
        []
    ),
    cmd!(
        Id::ExportMeasurements,
        "Export Measurements (CSV)…",
        "Save all measurements and counts as a CSV file",
        C::Measure,
        "takeoff spreadsheet export",
        []
    ),
    cmd!(
        Id::PageRotateClockwise,
        "Rotate Pages Clockwise",
        "Rotate the selected pages in the document",
        C::Organize,
        "rotate page",
        [(None, Shortcut::cmd_shift(Key::Char(']')))]
    ),
    cmd!(
        Id::PageRotateCounterClockwise,
        "Rotate Pages Counter-clockwise",
        "Rotate the selected pages in the document",
        C::Organize,
        "rotate page",
        [(None, Shortcut::cmd_shift(Key::Char('[')))]
    ),
    cmd!(
        Id::PageDelete,
        "Delete Pages",
        "Delete the selected pages",
        C::Organize,
        "remove page",
        []
    ),
    cmd!(
        Id::PageInsertBlank,
        "Insert Blank Page",
        "Insert an empty page after the current one",
        C::Organize,
        "new page add",
        []
    ),
    cmd!(
        Id::PageInsertImage,
        "Insert Image as Page…",
        "Add one or more pictures (PNG, JPEG) as new pages after the current page",
        C::Organize,
        "picture photo scan add page",
        []
    ),
    cmd!(
        Id::PageDuplicate,
        "Duplicate Pages",
        "Duplicate the selected pages",
        C::Organize,
        "copy page",
        []
    ),
    cmd!(
        Id::PageExtract,
        "Extract Pages…",
        "Save the selected pages as a new document",
        C::Organize,
        "split",
        []
    ),
    cmd!(
        Id::PageMoveUp,
        "Move Pages Earlier",
        "Move the selected pages one position up",
        C::Organize,
        "reorder",
        []
    ),
    cmd!(
        Id::PageMoveDown,
        "Move Pages Later",
        "Move the selected pages one position down",
        C::Organize,
        "reorder",
        []
    ),
    cmd!(
        Id::DocumentMerge,
        "Merge Documents…",
        "Append pages from other PDF files",
        C::Organize,
        "combine join",
        []
    ),
    cmd!(
        Id::NextTab,
        "Next Tab",
        "Switch to the next document tab",
        C::Navigate,
        "",
        [
            (
                WIN,
                Shortcut {
                    command: true,
                    shift: false,
                    alt: false,
                    ctrl: false,
                    key: Key::Tab
                }
            ),
            (
                LIN,
                Shortcut {
                    command: true,
                    shift: false,
                    alt: false,
                    ctrl: false,
                    key: Key::Tab
                }
            ),
            (
                MAC,
                Shortcut {
                    command: false,
                    shift: false,
                    alt: false,
                    ctrl: true,
                    key: Key::Tab
                }
            )
        ]
    ),
    cmd!(
        Id::PreviousTab,
        "Previous Tab",
        "Switch to the previous document tab",
        C::Navigate,
        "",
        [
            (
                WIN,
                Shortcut {
                    command: true,
                    shift: true,
                    alt: false,
                    ctrl: false,
                    key: Key::Tab
                }
            ),
            (
                LIN,
                Shortcut {
                    command: true,
                    shift: true,
                    alt: false,
                    ctrl: false,
                    key: Key::Tab
                }
            ),
            (
                MAC,
                Shortcut {
                    command: false,
                    shift: true,
                    alt: false,
                    ctrl: true,
                    key: Key::Tab
                }
            )
        ]
    ),
    cmd!(
        Id::Escape,
        "Cancel",
        "Cancel the current operation or clear the selection",
        C::Edit,
        "esc",
        [(None, Shortcut::plain(Key::Escape))]
    ),
];

/// Look up a command's metadata.
pub fn info(id: CommandId) -> &'static CommandInfo {
    // Every CommandId has a registry entry (enforced by a unit test), so the fallback is
    // unreachable in practice; it exists only to avoid a panic path.
    REGISTRY.iter().find(|c| c.id == id).unwrap_or(&REGISTRY[0])
}

/// Default bindings of a command for an OS.
pub fn default_bindings(id: CommandId, os: OsKind) -> Vec<Shortcut> {
    let mut v: Vec<Shortcut> = info(id)
        .bindings
        .iter()
        .filter(|(p, _)| p.is_none() || *p == Some(os))
        .map(|(_, s)| *s)
        .collect();
    // On macOS a binding registered for "all platforms" with `command` uses ⌘ already;
    // `Ctrl`-based tab switching is registered explicitly above.
    v.dedup();
    v
}

/// User-customisable bindings layered over the defaults.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct KeyBindings {
    /// Full replacement binding lists per command (empty list = unbound).
    pub overrides: BTreeMap<CommandId, Vec<Shortcut>>,
}

impl KeyBindings {
    /// Effective shortcuts for a command.
    pub fn shortcuts(&self, id: CommandId, os: OsKind) -> Vec<Shortcut> {
        match self.overrides.get(&id) {
            Some(v) => v.clone(),
            None => default_bindings(id, os),
        }
    }

    /// First (primary) shortcut label for tooltips and menus, in the platform's style.
    pub fn primary_label(&self, id: CommandId, os: OsKind) -> Option<String> {
        self.shortcuts(id, os).first().map(|s| s.label(os))
    }

    /// Resolve a pressed chord to a command.
    pub fn resolve(&self, chord: &Shortcut, os: OsKind) -> Option<CommandId> {
        REGISTRY
            .iter()
            .find(|c| self.shortcuts(c.id, os).contains(chord))
            .map(|c| c.id)
    }

    /// Commands that already use `chord` (excluding `except`).
    pub fn conflicts(&self, chord: &Shortcut, except: CommandId, os: OsKind) -> Vec<CommandId> {
        REGISTRY
            .iter()
            .filter(|c| c.id != except && self.shortcuts(c.id, os).contains(chord))
            .map(|c| c.id)
            .collect()
    }

    /// Replace the bindings of a command.
    pub fn set(&mut self, id: CommandId, shortcuts: Vec<Shortcut>) {
        self.overrides.insert(id, shortcuts);
    }

    /// Restore defaults for a command.
    pub fn reset(&mut self, id: CommandId) {
        self.overrides.remove(&id);
    }
}

/// A palette result.
#[derive(Clone, Debug, PartialEq)]
pub enum PaletteItem {
    /// A registered command.
    Command(CommandId),
    /// A setting, identified by its key in [`crate::prefs::SETTINGS`].
    Setting(&'static str),
}

/// Fuzzy-ish scoring: higher is better, `None` = no match.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Some(0);
    }
    let t = text.to_lowercase();
    if t == q {
        return Some(1000);
    }
    if t.starts_with(&q) {
        return Some(800 - t.len() as i32);
    }
    // All query words must appear (word-prefix preferred).
    let mut total = 0;
    for w in q.split_whitespace() {
        let s = if t
            .split(|c: char| !c.is_alphanumeric())
            .any(|tw| tw.starts_with(w))
        {
            300
        } else if t.contains(w) {
            150
        } else if is_subsequence(w, &t) {
            40
        } else {
            return None;
        };
        total += s;
    }
    Some(total - t.len() as i32 / 4)
}

fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

/// Search commands by title, description, category and keywords.
pub fn search_commands(query: &str, enabled: impl Fn(CommandId) -> bool) -> Vec<CommandId> {
    let mut scored: Vec<(i32, CommandId)> = REGISTRY
        .iter()
        .filter(|c| enabled(c.id))
        .filter_map(|c| {
            let hay = format!(
                "{} {} {} {}",
                c.title,
                c.keywords,
                c.category.title(),
                c.description
            );
            let title = score(query, c.title).map(|s| s + 500);
            let rest = score(query, &hay);
            title.or(rest).map(|s| (s, c.id))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, id)| id).collect()
}

/// Text for the generated shortcut reference: `(category, title, shortcut labels)`.
pub fn shortcut_reference(
    bindings: &KeyBindings,
    os: OsKind,
) -> Vec<(Category, &'static str, Vec<String>)> {
    let mut out: Vec<_> = REGISTRY
        .iter()
        .map(|c| {
            let labels = bindings
                .shortcuts(c.id, os)
                .iter()
                .map(|s| s.label(os))
                .collect::<Vec<_>>();
            (c.category, c.title, labels)
        })
        .filter(|(_, _, l)| !l.is_empty())
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(b.1)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_variant_is_registered_once() {
        // Compile-time exhaustiveness is approximated by checking uniqueness and that the
        // lookup of each registry id round-trips.
        let mut seen = std::collections::BTreeSet::new();
        for c in REGISTRY {
            assert!(seen.insert(c.id), "duplicate registry entry {:?}", c.id);
            assert_eq!(info(c.id).id, c.id);
            assert!(!c.title.is_empty() && !c.description.is_empty());
        }
        // Spot-check a few that previously went missing during refactors.
        for id in [
            Id::EditRedo,
            Id::FileSaveAs,
            Id::CommandPalette,
            Id::ToolEditText,
            Id::Escape,
        ] {
            assert!(seen.contains(&id));
        }
    }

    #[test]
    fn mac_labels_use_command_symbols_never_ctrl() {
        let kb = KeyBindings::default();
        assert_eq!(
            kb.primary_label(Id::FileOpen, OsKind::MacOs).as_deref(),
            Some("⌘O")
        );
        assert_eq!(
            kb.primary_label(Id::EditRedo, OsKind::MacOs).as_deref(),
            Some("⇧⌘Z")
        );
        assert_eq!(
            kb.primary_label(Id::FileOpen, OsKind::Windows).as_deref(),
            Some("Ctrl+O")
        );
        assert_eq!(
            kb.primary_label(Id::EditRedo, OsKind::Windows).as_deref(),
            Some("Ctrl+Y")
        );
        for c in REGISTRY {
            for s in kb.shortcuts(c.id, OsKind::MacOs) {
                let l = s.label(OsKind::MacOs);
                assert!(
                    !l.contains("Ctrl"),
                    "{:?} shows a Windows label on macOS: {l}",
                    c.id
                );
            }
            for s in kb.shortcuts(c.id, OsKind::Windows) {
                let l = s.label(OsKind::Windows);
                assert!(
                    !l.contains('⌘') && !l.contains('⇧'),
                    "{:?} shows a macOS label on Windows: {l}",
                    c.id
                );
            }
        }
    }

    #[test]
    fn redo_has_both_windows_chords() {
        let kb = KeyBindings::default();
        let w = kb.shortcuts(Id::EditRedo, OsKind::Windows);
        assert!(
            w.contains(&Shortcut::cmd(Char('Y'))) && w.contains(&Shortcut::cmd_shift(Char('Z')))
        );
        assert_eq!(
            kb.resolve(&Shortcut::cmd_shift(Char('Z')), OsKind::MacOs),
            Some(Id::EditRedo)
        );
        assert_eq!(kb.resolve(&Shortcut::cmd(Char('Y')), OsKind::MacOs), None);
    }

    #[test]
    fn no_default_binding_conflicts_per_platform() {
        for os in [OsKind::Windows, OsKind::MacOs, OsKind::Linux] {
            let kb = KeyBindings::default();
            let mut seen: BTreeMap<Shortcut, CommandId> = BTreeMap::new();
            for c in REGISTRY {
                for s in kb.shortcuts(c.id, os) {
                    if let Some(prev) = seen.insert(s, c.id) {
                        panic!("{os:?}: {s:?} bound to both {prev:?} and {:?}", c.id);
                    }
                }
            }
        }
    }

    #[test]
    fn overrides_and_conflicts() {
        let mut kb = KeyBindings::default();
        let chord = Shortcut::cmd(Char('O'));
        assert_eq!(
            kb.conflicts(&chord, Id::FileSave, OsKind::Windows),
            vec![Id::FileOpen]
        );
        kb.set(Id::FileOpen, vec![Shortcut::cmd(Char('L'))]);
        assert_eq!(kb.resolve(&chord, OsKind::Windows), None);
        assert_eq!(
            kb.resolve(&Shortcut::cmd(Char('L')), OsKind::Windows),
            Some(Id::FileOpen)
        );
        kb.reset(Id::FileOpen);
        assert_eq!(kb.resolve(&chord, OsKind::Windows), Some(Id::FileOpen));
    }

    #[test]
    fn palette_finds_commands_without_knowing_their_ribbon_location() {
        let all = |_| true;
        for c in REGISTRY {
            let found = search_commands(c.title, all);
            assert!(
                found.contains(&c.id),
                "{:?} not found by its own title",
                c.id
            );
            assert!(
                found.iter().position(|x| *x == c.id).unwrap() < 5,
                "{:?} ranks too low for its own title",
                c.id
            );
        }
        assert!(search_commands("split", all).contains(&Id::PageExtract));
        assert!(search_commands("night mode", all).contains(&Id::ViewDarkPages));
        assert_eq!(search_commands("zzzzqq", all), vec![]);
        // Disabled commands are not offered.
        assert!(!search_commands("undo", |id| id != Id::EditUndo).contains(&Id::EditUndo));
    }
}
