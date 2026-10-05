//! Tools. The three families are deliberately distinct (and surfaced as such in the UI):
//! navigating/selecting, creating *annotations* (comment layer), and editing/adding *page
//! content* (the document's own content stream).

use crate::command::CommandId;

/// Active tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Tool {
    /// Select and move annotations.
    #[default]
    Select,
    /// Pan with drag.
    Hand,
    /// Select/copy text.
    TextSelect,
    Highlight,
    Underline,
    StrikeOut,
    Note,
    FreeText,
    Callout,
    Rectangle,
    Ellipse,
    Line,
    Arrow,
    Polygon,
    Polyline,
    Ink,
    Stamp,
    /// Edit existing page content (text runs, images).
    EditText,
    /// Add new text to page content.
    AddText,
    /// Add an image to page content.
    AddImage,
    /// Fill AcroForm fields.
    FillForm,
    /// Measure a straight distance.
    MeasureDistance,
    /// Measure a polyline length.
    MeasurePerimeter,
    /// Measure a polygon area.
    MeasureArea,
    /// Measure a rectangle (area and perimeter).
    MeasureRect,
    /// Measure a circle by centre and edge.
    MeasureRadius,
    /// Measure an angle.
    MeasureAngle,
    /// Place count markers.
    Count,
    /// Place the saved handwritten signature (a drawing; not a cryptographic signature).
    PlaceSignature,
    /// Drag the area where a digital signature will be shown.
    SignArea,
    /// Calibrate the drawing scale from a known length.
    Calibrate,
}

/// Family a tool belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolFamily {
    /// Navigation / selection; modifies nothing.
    Navigate,
    /// Creates annotations (standard PDF annotation objects).
    Annotation,
    /// Changes the page's real content stream.
    PageContent,
    /// Fills form fields (field values, not page content).
    Form,
    /// Creates measurement annotations or calibrates scales.
    Measure,
    /// Chooses where a digital signature is shown.
    Sign,
}

impl Tool {
    /// The family of this tool.
    pub fn family(self) -> ToolFamily {
        match self {
            Tool::Select | Tool::Hand | Tool::TextSelect => ToolFamily::Navigate,
            Tool::EditText | Tool::AddText | Tool::AddImage => ToolFamily::PageContent,
            Tool::FillForm => ToolFamily::Form,
            Tool::MeasureDistance
            | Tool::MeasurePerimeter
            | Tool::MeasureArea
            | Tool::MeasureRect
            | Tool::MeasureRadius
            | Tool::MeasureAngle
            | Tool::Count
            | Tool::Calibrate => ToolFamily::Measure,
            Tool::PlaceSignature => ToolFamily::Annotation,
            Tool::SignArea => ToolFamily::Sign,
            _ => ToolFamily::Annotation,
        }
    }

    /// Tool activated by a command, if any.
    pub fn from_command(c: CommandId) -> Option<Tool> {
        Some(match c {
            CommandId::ToolSelect => Tool::Select,
            CommandId::ToolHand => Tool::Hand,
            CommandId::ToolTextSelect => Tool::TextSelect,
            CommandId::ToolHighlight => Tool::Highlight,
            CommandId::ToolUnderline => Tool::Underline,
            CommandId::ToolStrikeOut => Tool::StrikeOut,
            CommandId::ToolNote => Tool::Note,
            CommandId::ToolFreeText => Tool::FreeText,
            CommandId::ToolCallout => Tool::Callout,
            CommandId::ToolRectangle => Tool::Rectangle,
            CommandId::ToolEllipse => Tool::Ellipse,
            CommandId::ToolLine => Tool::Line,
            CommandId::ToolArrow => Tool::Arrow,
            CommandId::ToolPolygon => Tool::Polygon,
            CommandId::ToolPolyline => Tool::Polyline,
            CommandId::ToolInk => Tool::Ink,
            CommandId::ToolStamp => Tool::Stamp,
            CommandId::ToolEditText => Tool::EditText,
            CommandId::ToolAddText => Tool::AddText,
            CommandId::ToolAddImage => Tool::AddImage,
            CommandId::ToolFillForm => Tool::FillForm,
            CommandId::ToolMeasureDistance => Tool::MeasureDistance,
            CommandId::ToolMeasurePerimeter => Tool::MeasurePerimeter,
            CommandId::ToolMeasureArea => Tool::MeasureArea,
            CommandId::ToolMeasureRect => Tool::MeasureRect,
            CommandId::ToolMeasureRadius => Tool::MeasureRadius,
            CommandId::ToolMeasureAngle => Tool::MeasureAngle,
            CommandId::ToolCount => Tool::Count,
            CommandId::ToolCalibrate => Tool::Calibrate,
            CommandId::ToolPlaceSignature => Tool::PlaceSignature,
            CommandId::ToolSignArea => Tool::SignArea,
            _ => return None,
        })
    }

    /// Command that activates this tool.
    pub fn command(self) -> CommandId {
        match self {
            Tool::Select => CommandId::ToolSelect,
            Tool::Hand => CommandId::ToolHand,
            Tool::TextSelect => CommandId::ToolTextSelect,
            Tool::Highlight => CommandId::ToolHighlight,
            Tool::Underline => CommandId::ToolUnderline,
            Tool::StrikeOut => CommandId::ToolStrikeOut,
            Tool::Note => CommandId::ToolNote,
            Tool::FreeText => CommandId::ToolFreeText,
            Tool::Callout => CommandId::ToolCallout,
            Tool::Rectangle => CommandId::ToolRectangle,
            Tool::Ellipse => CommandId::ToolEllipse,
            Tool::Line => CommandId::ToolLine,
            Tool::Arrow => CommandId::ToolArrow,
            Tool::Polygon => CommandId::ToolPolygon,
            Tool::Polyline => CommandId::ToolPolyline,
            Tool::Ink => CommandId::ToolInk,
            Tool::Stamp => CommandId::ToolStamp,
            Tool::EditText => CommandId::ToolEditText,
            Tool::AddText => CommandId::ToolAddText,
            Tool::AddImage => CommandId::ToolAddImage,
            Tool::FillForm => CommandId::ToolFillForm,
            Tool::MeasureDistance => CommandId::ToolMeasureDistance,
            Tool::MeasurePerimeter => CommandId::ToolMeasurePerimeter,
            Tool::MeasureArea => CommandId::ToolMeasureArea,
            Tool::MeasureRect => CommandId::ToolMeasureRect,
            Tool::MeasureRadius => CommandId::ToolMeasureRadius,
            Tool::MeasureAngle => CommandId::ToolMeasureAngle,
            Tool::Count => CommandId::ToolCount,
            Tool::Calibrate => CommandId::ToolCalibrate,
            Tool::PlaceSignature => CommandId::ToolPlaceSignature,
            Tool::SignArea => CommandId::ToolSignArea,
        }
    }

    /// Short status-bar hint describing how to use the tool.
    pub fn hint(self) -> &'static str {
        match self {
            Tool::Select => "Click an annotation to select it; drag to move.",
            Tool::Hand => "Drag to pan the document.",
            Tool::TextSelect => "Drag over text to select it; Ctrl/Cmd+C copies.",
            Tool::Highlight | Tool::Underline | Tool::StrikeOut => "Drag over text to mark it up.",
            Tool::Note => "Click where the note should be placed.",
            Tool::FreeText => "Drag a box, then type your text.",
            Tool::Callout => {
                "Press where the callout should point, drag to where the text box should go."
            }
            Tool::Rectangle | Tool::Ellipse => "Drag to draw. Hold Shift for a square/circle.",
            Tool::Line | Tool::Arrow => "Drag to draw. Hold Shift to snap to 15° angles.",
            Tool::Polygon | Tool::Polyline => {
                "Click to add points; double-click or Enter to finish; Esc cancels."
            }
            Tool::Ink => "Drag to draw freehand.",
            Tool::Stamp => "Drag a box to place the stamp.",
            Tool::EditText => {
                "Click text or an image to edit it. Outlined text can be edited; grey text shows why it can't."
            }
            Tool::AddText => "Click where the new page text should start.",
            Tool::AddImage => "Drag a box, then choose an image file.",
            Tool::FillForm => "Click a field to fill it. Text fields open an editor; boxes toggle.",
            Tool::MeasureDistance => "Click the start, then the end point. Shift locks to 15°.",
            Tool::MeasurePerimeter => {
                "Click points along the path; double-click or Enter to finish."
            }
            Tool::MeasureArea => "Click the corners; double-click or Enter to finish.",
            Tool::MeasureRect => "Drag a rectangle to measure its area and perimeter.",
            Tool::MeasureRadius => "Click the centre, then a point on the edge.",
            Tool::MeasureAngle => "Click the first arm, the vertex, then the second arm.",
            Tool::Count => "Click each item to count it. Choose the category in the side panel.",
            Tool::Calibrate => "Click two points whose real distance you know.",
            Tool::PlaceSignature => {
                "Click where your handwritten signature should go (a drawing, not a digital signature)."
            }
            Tool::SignArea => "Drag the rectangle where the digital signature will be shown.",
        }
    }
}
