//! Centralized color palette and byte-span styling helpers for the Freya UI.

use petro_chunked_schema::dynamic::ByteSpanAnnotation;

/// Main window / center-pane background.
pub const BG_ROOT: &str = "rgb(18, 19, 24)";
/// Top toolbar surface background.
pub const BG_TOOLBAR: &str = "rgb(28, 30, 38)";
/// Status bar and right-pane background.
pub const BG_STATUS: &str = "rgb(23, 25, 32)";
/// Left tree pane background.
pub const BG_PANE_LEFT: &str = "rgb(22, 24, 31)";
/// Right schema editor pane background.
pub const BG_PANE_RIGHT: &str = "rgb(23, 25, 33)";
/// Collapsible modal drawer background.
pub const BG_MODAL: &str = "rgb(32, 35, 46)";
/// Section card background in right pane / inspector footer.
pub const BG_CARD: &str = "rgb(30, 33, 44)";
/// Sub-surface card background for code/YAML/legend boxes.
pub const BG_SUB_CARD: &str = "rgb(24, 26, 34)";
/// Deep recessed surface for hex grid and diagnostic scroll area.
pub const BG_INSET: &str = "rgb(14, 15, 20)";
/// Default row / unassigned hex cell background.
pub const BG_ROW: &str = "rgb(28, 31, 40)";
/// Selected tree row background.
pub const BG_ROW_SELECTED: &str = "rgb(48, 72, 110)";
/// Error-highlighted tree row background.
pub const BG_ROW_ERROR: &str = "rgb(68, 32, 36)";
/// Cursor-selected hex byte cell background.
pub const BG_CURSOR: &str = "rgb(225, 165, 40)";
/// Padding byte-span background.
pub const BG_PADDING: &str = "rgb(65, 65, 72)";

/// Primary light foreground text color.
pub const TEXT_PRIMARY: &str = "rgb(235, 237, 245)";
/// Secondary muted foreground text color.
pub const TEXT_MUTED: &str = "rgb(175, 180, 195)";
/// Subtle dim foreground color for tree guides / offsets.
pub const TEXT_DIM: &str = "rgb(130, 140, 165)";
/// Dark foreground used on bright cursor highlight cells.
pub const TEXT_DARK: &str = "rgb(15, 15, 20)";

/// Blue accent for titles, chunk IDs, and schema indicators.
pub const ACCENT_BLUE: &str = "rgb(120, 195, 255)";
/// Green accent for matched fields and passing states.
pub const ACCENT_GREEN: &str = "rgb(140, 235, 165)";
/// Amber accent for mini-chunk IDs, byte offsets, and warnings.
pub const ACCENT_AMBER: &str = "rgb(245, 200, 120)";
/// Red/coral accent for errors and diagnostics.
pub const ACCENT_RED: &str = "rgb(255, 165, 145)";
/// Muted mint foreground for the hex ASCII column.
pub const ACCENT_ASCII: &str = "rgb(165, 210, 180)";

/// Rotating 8-color palette used to highlight structural [`ByteSpanAnnotation`]s in the hex grid.
pub const SPAN_BG_COLORS: [&str; 8] = [
    "rgb(30, 88, 135)",
    "rgb(38, 110, 68)",
    "rgb(125, 78, 28)",
    "rgb(102, 52, 130)",
    "rgb(28, 112, 115)",
    "rgb(130, 48, 78)",
    "rgb(85, 105, 32)",
    "rgb(70, 75, 135)",
];

/// Returns the background color token for a [`ByteSpanAnnotation`].
pub fn span_bg_color(span: &ByteSpanAnnotation) -> &'static str {
    if span.is_padding {
        BG_PADDING
    } else {
        SPAN_BG_COLORS[span.color_index % SPAN_BG_COLORS.len()]
    }
}
