//! Application state shared by all UI modules.

use crate::theme::Palette;
use editor_core::command::CommandId;
use editor_core::jobs::WorkerHub;
use editor_core::prefs::Preferences;
use editor_core::session::{DocId, DocumentSession};
use editor_core::tiles::{ByteLru, TileKey};
use editor_core::tools::Tool;
use pdf_engine::annot::{AnnotId, AnnotationInfo};
use pdf_engine::doc::PageId;
use pdf_engine::geom::{Point, Rect};
use pdf_engine::pagecontent::{EditReport, ImageInfo, ObjRef, TextRunInfo};
use pdf_engine::text::TextPage;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

/// Left sidebar sections.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LeftTab {
    Thumbnails,
    Bookmarks,
    Search,
}

/// Right sidebar sections.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RightTab {
    Properties,
    Comments,
    Measure,
}

/// Ribbon tabs. Only tabs with working content are shown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RibbonTab {
    File,
    Home,
    Edit,
    Comment,
    Forms,
    Measure,
    Sign,
    Organize,
    View,
}

/// A transient message.
pub struct Notice {
    pub text: String,
    pub error: bool,
    pub until: Instant,
}

/// Modal dialogs.
pub enum Dialog {
    /// Close-with-unsaved-changes confirmation for tab index.
    ConfirmClose { tab: usize },
    /// Quit with unsaved documents.
    ConfirmQuit,
    /// An error that needs acknowledgement (e.g. save failed).
    Error { title: String, detail: String },
    /// Confirm opening an external link.
    ConfirmLink { uri: String },
    /// Go to page.
    GoToPage { text: String },
    /// Preferences (search box text).
    Preferences { filter: String },
    /// Shortcut reference.
    Shortcuts {
        filter: String,
        capture: Option<CommandId>,
    },
    /// About / licences.
    About,
    /// First-run workspace choice.
    FirstRun,
    /// Delete-pages confirmation.
    ConfirmDeletePages { count: usize },
    /// Document warnings shown after opening (signatures, XFA…).
    OpenWarnings { title: String, lines: Vec<String> },
    /// Free-text entry for a new text box / note.
    TextEntry {
        page: PageId,
        tool: Tool,
        rect: Rect,
        text: String,
    },
    /// New page text (real content, not an annotation).
    AddText {
        page: PageId,
        at: Point,
        text: String,
        size: f64,
        bold: bool,
    },
    /// Document properties.
    Properties(Box<PropsState>),
    /// Confirm flattening all form fields.
    ConfirmFlatten { fields: usize },
    /// Recognise text (OCR) options.
    Ocr(Box<crate::ocr_ui::OcrDialogState>),
    /// OCR running.
    OcrProgress,
    /// Sign with a certificate.
    Sign(Box<SignDialogState>),
    /// Signatures found in the document.
    Signatures(Vec<pdf_engine::sign::SignatureInfo>),
    /// Drawing pad for the handwritten signature.
    DrawSignature(Box<DrawSigState>),
    /// Documents recovered after a crash.
    Recovery(Vec<platform::recovery::RecoveryEntry>),
    /// Fill a text or choice form field.
    FillField(Box<FillFieldState>),
    /// Ask how form fields are treated when pages are copied.
    FormPolicy(FormPolicyState),
    /// Set or calibrate a drawing scale.
    Scale(Box<ScaleDialog>),
}

/// Pointer interaction in progress on the canvas.
#[derive(Clone, Debug, Default)]
pub enum Interaction {
    #[default]
    None,
    Panning,
    /// Text selection drag: page, anchor glyph.
    TextSelect {
        page: PageId,
        anchor: usize,
    },
    /// Drawing a shape or markup: page index, points in PDF space.
    Draw {
        page: PageId,
        tool: Tool,
        points: Vec<Point>,
    },
    /// Moving the selected annotations; accumulated delta in PDF space.
    Move {
        delta: (f64, f64),
    },
    /// Resizing one annotation; handle index (0..8) and the preview rect in PDF space.
    Resize {
        id: AnnotId,
        page: PageId,
        handle: usize,
        rect: Rect,
    },
    /// Moving the selected page-content object; accumulated delta in PDF space.
    ContentMove {
        delta: (f64, f64),
    },
    /// Resizing the selected image; preview rect in PDF space.
    ContentResize {
        page: PageId,
        handle: usize,
        rect: Rect,
    },
}

/// A comment list row: (page index, page, annotation).
pub type CommentRow = (usize, PageId, AnnotationInfo);

/// Per-tab transient state.
#[derive(Default)]
pub struct TabState {
    pub interaction: Interaction,
    /// Annotation cache: page -> (revision, infos).
    pub annots: HashMap<PageId, (u64, Arc<Vec<AnnotationInfo>>)>,
    /// Page index requested by an explicit "go to page" (applied next frame).
    pub goto: Option<usize>,
    pub last_zoom_change: Option<Instant>,
    pub thumb_scroll_to_current: bool,
    pub polygon_points: Vec<Point>,
    /// Pending zoom change (new zoom factor, optional anchor in viewport coordinates).
    pub zoom_request: Option<(f64, Option<egui::Vec2>)>,
    /// Pages being dragged in the thumbnail organizer.
    pub thumb_drag: Option<Vec<PageId>>,
    /// Last clicked thumbnail index (for shift-range selection).
    pub thumb_anchor: Option<usize>,
    pub bookmarks: Option<(u64, Arc<Vec<pdf_engine::nav::Bookmark>>)>,
    pub comments: Option<(u64, Arc<Vec<CommentRow>>)>,
    pub comments_filter: String,
    /// Page-content objects (text runs, images) per page for the current revision.
    pub objects: HashMap<PageId, (u64, Arc<PageObjects>)>,
    /// Working copy of the text run being edited.
    pub edit_draft: Option<EditDraft>,
    /// Parsed form fields for the current revision.
    pub form: Option<(u64, Arc<pdf_engine::forms::FormInfo>)>,
    /// Scale registry for the current revision.
    pub scales: Option<(u64, Arc<pdf_engine::measure::ScaleSet>)>,
    /// Measurement report rows for the current revision.
    pub measure_rows: Option<(u64, Arc<Vec<pdf_engine::measure::MeasureRow>>)>,
    /// A region scale waiting for the user to drag its rectangle.
    pub pending_region: Option<(PageId, pdf_engine::measure::Scale)>,
    /// Category new count markers are filed under.
    pub count_category: String,
}

/// Editable page objects of one page.
pub struct PageObjects {
    pub runs: Vec<TextRunInfo>,
    pub images: Vec<ImageInfo>,
    /// Why content editing is unavailable for this page, if it is.
    pub error: Option<String>,
}

/// Draft of a text-run edit shown in the properties panel.
#[derive(Clone)]
pub struct EditDraft {
    pub page: PageId,
    pub run: ObjRef,
    pub original: String,
    pub text: String,
    pub size: f64,
    pub original_size: f64,
    pub fit_width: bool,
    /// Last outcome: (is_error, message).
    pub message: Option<(bool, String)>,
    /// Characters the font cannot show, with the font name, when an apply failed for that reason.
    pub missing: Option<(String, String)>,
    pub report: Option<EditReport>,
}

/// One tab.
pub struct Tab {
    pub session: DocumentSession,
    pub ui: TabState,
}

/// Everything the UI needs.
pub struct App {
    pub prefs: Preferences,
    pub pal: Palette,
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub hub: WorkerHub,
    pub tiles: ByteLru<TileKey, egui::TextureHandle>,
    pub in_flight: HashSet<TileKey>,
    pub failed: HashSet<TileKey>,
    pub text: HashMap<(DocId, PageId), (u64, Arc<TextPage>)>,
    pub text_pending: HashSet<(DocId, PageId, u64)>,
    pub tool: Tool,
    pub left_tab: LeftTab,
    pub right_tab: RightTab,
    pub ribbon_tab: RibbonTab,
    pub dialog: Option<Dialog>,
    pub palette_open: bool,
    pub palette_query: String,
    pub palette_sel: usize,
    pub notices: Vec<Notice>,
    pub prefs_dirty: bool,
    pub frame_counter: u64,
    pub search_focus: bool,
    pub system_dark: bool,
    pub quit_confirmed: bool,
    pub dark_filter_applied: bool,
    pub pending_open: Vec<std::path::PathBuf>,
    pub last_autosave: Instant,
    /// Dialog to show after the current one closes (e.g. Preferences -> Shortcuts).
    pub dialog_next: Option<Dialog>,
    pub last_title: String,
    /// Page exports running on background threads.
    pub exports: Vec<crate::docops_ui::ExportJob>,
    /// Sign-dialog state parked while the user drags the signature area on the page.
    pub sign_return: Option<Box<SignDialogState>>,
    /// Text recognition running on a background thread.
    pub ocr_job: Option<crate::ocr_ui::OcrJob>,
    /// The saved handwritten signature (a drawing).
    pub handwriting: editor_core::handwriting::HandwrittenSignature,
}

impl App {
    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active)
    }

    pub fn notify(&mut self, text: impl Into<String>) {
        self.notices.push(Notice {
            text: text.into(),
            error: false,
            until: Instant::now() + std::time::Duration::from_secs(4),
        });
    }

    pub fn notify_error(&mut self, text: impl Into<String>) {
        self.notices.push(Notice {
            text: text.into(),
            error: true,
            until: Instant::now() + std::time::Duration::from_secs(8),
        });
    }
}

/// State of the "fill field" dialog.
pub struct FillFieldState {
    pub field: pdf_engine::ObjectId,
    pub name: String,
    pub value: String,
    pub multiline: bool,
    pub password: bool,
    pub max_len: Option<usize>,
    /// Combo/list options as `(export value, display text)`; empty for text fields.
    pub options: Vec<(String, String)>,
    pub error: Option<String>,
}

/// A page operation that must know how form fields are handled.
#[derive(Clone, Debug, PartialEq)]
pub enum FormOp {
    Duplicate,
    Extract,
    Merge(Vec<std::path::PathBuf>),
}

/// State of the form-policy dialog.
pub struct FormPolicyState {
    pub op: FormOp,
    pub policy: pdf_engine::pageops::FormPolicy,
}

/// How far a new scale applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleScope {
    Document,
    Page,
    Region,
}

/// How the scale is specified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleMode {
    /// From a measured segment of known real length.
    Calibrate,
    /// A `1:n` ratio.
    Ratio,
    /// `paper = real` with units.
    Custom,
}

/// State of the scale dialog.
pub struct ScaleDialog {
    pub page: PageId,
    /// Length of the drawn calibration segment in points, if the dialog came from the tool.
    pub measured_pts: Option<f64>,
    pub mode: ScaleMode,
    pub scope: ScaleScope,
    pub known_len: String,
    pub known_unit: pdf_engine::measure::Unit,
    pub ratio_n: String,
    pub ratio_unit: pdf_engine::measure::Unit,
    pub paper_len: String,
    pub paper_unit: pdf_engine::measure::Unit,
    pub real_len: String,
    pub real_unit: pdf_engine::measure::Unit,
    pub apply_existing: bool,
    pub error: Option<String>,
}

/// State of the document-properties dialog.
pub struct PropsState {
    pub title: String,
    pub author: String,
    pub subject: String,
    pub keywords: String,
    pub creator: String,
    pub producer: String,
    pub created: String,
    pub modified: String,
    pub pages: usize,
    pub file_size: usize,
    pub xmp: bool,
    pub can_edit: bool,
    pub error: Option<String>,
}

/// State of the "sign with certificate" dialog.
#[derive(Clone, Default)]
pub struct SignDialogState {
    pub cert_path: Option<std::path::PathBuf>,
    pub password: String,
    pub reason: String,
    pub location: String,
    pub contact: String,
    pub visible: bool,
    pub area: Option<(PageId, Rect)>,
    pub signer_hint: Option<String>,
    pub error: Option<String>,
}

/// State of the signature drawing pad (coordinates relative to the pad).
#[derive(Default)]
pub struct DrawSigState {
    pub strokes: Vec<Vec<[f32; 2]>>,
    pub error: Option<String>,
}
