//! Centralized strongly-typed Freya 0.5 color palette, border helpers, and byte-span styling.

use freya::prelude::*;
use petro_chunked_schema::dynamic::ByteSpanAnnotation;

/// Main window / center-pane background.
pub const BG_ROOT: Color = Color::from_rgb(18, 19, 24);
/// Top toolbar surface background.
pub const BG_TOOLBAR: Color = Color::from_rgb(28, 30, 38);
/// Status bar surface background.
pub const BG_STATUS: Color = Color::from_rgb(23, 25, 32);
/// Left tree pane background.
pub const BG_PANE_LEFT: Color = Color::from_rgb(22, 24, 31);
/// Right schema stack pane background.
pub const BG_PANE_RIGHT: Color = Color::from_rgb(23, 25, 33);
/// Freestanding modal window surface background.
pub const BG_MODAL: Color = Color::from_rgb(26, 29, 39);
/// Section card background in right pane / inspector footer.
pub const BG_CARD: Color = Color::from_rgb(30, 33, 44);
/// Collapsed ancestor card background in the right-rail schema stack.
pub const BG_STACK_PARENT: Color = Color::from_rgb(26, 29, 39);
/// Sub-surface card background for code/YAML/legend boxes.
pub const BG_SUB_CARD: Color = Color::from_rgb(24, 26, 34);
/// Deep recessed surface for hex grid, file lists, and diagnostic scroll area.
pub const BG_INSET: Color = Color::from_rgb(14, 15, 20);
/// Default row / unassigned hex cell background.
pub const BG_ROW: Color = Color::from_rgb(28, 31, 40);
/// Selected tree or list row background.
pub const BG_ROW_SELECTED: Color = Color::from_rgb(48, 72, 110);
/// Hover-synchronized match highlight on left-rail chunk tree rows.
pub const BG_ROW_HOVER_MATCH: Color = Color::from_rgb(52, 64, 96);
/// Error-highlighted tree row background.
pub const BG_ROW_ERROR: Color = Color::from_rgb(68, 32, 36);
/// Cursor-selected hex byte cell background.
pub const BG_CURSOR: Color = Color::from_rgb(225, 165, 40);
/// Padding byte-span background.
pub const BG_PADDING: Color = Color::from_rgb(65, 65, 72);
/// Warm amber badge surface for `.meg` archive tags.
pub const BG_MEG_BADGE: Color = Color::from_rgb(72, 54, 24);

/// Subtle horizontal/vertical hairline border and divider color.
pub const BORDER_SUBTLE: Color = Color::from_rgb(52, 57, 74);
/// Softer card border color.
pub const BORDER_CARD: Color = Color::from_rgb(48, 53, 68);
/// Inactive radio/chip selector background.
pub const BG_CHIP: Color = Color::from_rgb(35, 39, 52);
/// Selected radio/chip selector background.
pub const BG_CHIP_ACTIVE: Color = Color::from_rgb(40, 92, 162);
/// Inactive chip border color.
pub const BORDER_CHIP: Color = Color::from_rgb(58, 64, 84);
/// Selected chip border color.
pub const BORDER_CHIP_ACTIVE: Color = Color::from_rgb(105, 175, 250);
/// Highlighted border color for the active bottom card in the schema stack.
pub const BORDER_STACK_ACTIVE: Color = Color::from_rgb(92, 160, 240);
/// Primary affirmative action button background.
pub const BG_BTN_PRIMARY: Color = Color::from_rgb(38, 96, 175);
/// Primary action button border color.
pub const BORDER_BTN_PRIMARY: Color = Color::from_rgb(92, 160, 240);
/// Destructive action button background.
pub const BG_BTN_DANGER: Color = Color::from_rgb(82, 34, 40);
/// Destructive action button border color.
pub const BORDER_BTN_DANGER: Color = Color::from_rgb(155, 68, 76);
/// Amber warning border color.
pub const BORDER_AMBER: Color = Color::from_rgb(165, 125, 55);

/// Primary light foreground text color.
pub const TEXT_PRIMARY: Color = Color::from_rgb(235, 237, 245);
/// Secondary muted foreground text color.
pub const TEXT_MUTED: Color = Color::from_rgb(175, 180, 195);
/// Subtle dim foreground color for tree guides / offsets.
pub const TEXT_DIM: Color = Color::from_rgb(130, 140, 165);
/// Dark foreground used on bright cursor highlight cells.
pub const TEXT_DARK: Color = Color::from_rgb(15, 15, 20);

/// Blue accent for titles, chunk IDs, and schema indicators.
pub const ACCENT_BLUE: Color = Color::from_rgb(120, 195, 255);
/// Green accent for matched fields and passing states.
pub const ACCENT_GREEN: Color = Color::from_rgb(140, 235, 165);
/// Amber accent for mini-chunk IDs, byte offsets, and warnings.
pub const ACCENT_AMBER: Color = Color::from_rgb(245, 200, 120);
/// Red/coral accent for errors and diagnostics.
pub const ACCENT_RED: Color = Color::from_rgb(255, 165, 145);
/// Muted mint foreground for the hex ASCII column.
pub const ACCENT_ASCII: Color = Color::from_rgb(165, 210, 180);

/// Rotating 8-color palette used to highlight structural [`ByteSpanAnnotation`]s in the hex grid.
pub const SPAN_BG_COLORS: [Color; 8] = [
    Color::from_rgb(30, 88, 135),
    Color::from_rgb(38, 110, 68),
    Color::from_rgb(125, 78, 28),
    Color::from_rgb(102, 52, 130),
    Color::from_rgb(28, 112, 115),
    Color::from_rgb(130, 48, 78),
    Color::from_rgb(85, 105, 32),
    Color::from_rgb(70, 75, 135),
];

/// Returns the background color token for a [`ByteSpanAnnotation`].
pub fn span_bg_color(span: &ByteSpanAnnotation) -> Color {
    if span.is_padding {
        BG_PADDING
    } else {
        SPAN_BG_COLORS[span.color_index % SPAN_BG_COLORS.len()]
    }
}

/// Builds a uniform `1px` inner border with `color`.
pub fn border_all(color: Color) -> Border {
    Border::new().fill(color).width(1.0)
}

/// Builds a `1px` bottom-only border with `color`.
pub fn border_bottom(color: Color) -> Border {
    Border::new().fill(color).width(BorderWidth {
        top: 0.0,
        right: 0.0,
        bottom: 1.0,
        left: 0.0,
    })
}

/// Builds a `1px` top-only border with `color`.
pub fn border_top(color: Color) -> Border {
    Border::new().fill(color).width(BorderWidth {
        top: 1.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    })
}

/// Builds a `1px` left-only border with `color`.
pub fn border_left(color: Color) -> Border {
    Border::new().fill(color).width(BorderWidth {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 1.0,
    })
}
