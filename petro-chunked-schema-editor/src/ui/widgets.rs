//! Shared styled UI primitives for `petro-chunked-schema-editor`.
//!
//! Provides cohesive dark-themed action buttons, radio-style segmented selector chips, toolbar
//! dividers, section captions, and compact text-input themes so every pane shares uniform
//! typography and visual hierarchy.

use freya::prelude::*;

use super::theme;

/// Visual emphasis variant for [`ActionButton`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    /// Highlighted affirmative action (e.g. `Save Field`, `Run Validate Now`).
    Primary,
    /// Neutral secondary action (e.g. `Close [X]`, `Set Pad`).
    Secondary,
    /// Destructive action (e.g. `Delete Field`, `Remove`).
    Danger,
    /// Toggleable action currently in an active/open state.
    Active,
}

/// Consistently styled dark action button with crisp borders and uniform typography.
#[component]
pub fn ActionButton(
    /// Visible button label.
    label: String,
    /// Visual emphasis style.
    style: ButtonStyle,
    /// When `Some(true)`, uses tighter padding and `11px` text for inline row actions.
    compact: Option<bool>,
    /// Callback fired on pointer press.
    onpress: EventHandler<()>,
) -> Element {
    let is_compact = compact.unwrap_or(false);
    let padding = if is_compact { "2 8" } else { "4 10" };
    let font_size = if is_compact { "11" } else { "12" };

    let (bg, border, fg, weight) = match style {
        ButtonStyle::Primary => (
            theme::BG_BTN_PRIMARY,
            theme::BORDER_BTN_PRIMARY,
            theme::TEXT_PRIMARY,
            "bold",
        ),
        ButtonStyle::Secondary => (theme::BG_CHIP, theme::BORDER_CHIP, theme::TEXT_PRIMARY, "normal"),
        ButtonStyle::Danger => (
            theme::BG_BTN_DANGER,
            theme::BORDER_BTN_DANGER,
            theme::ACCENT_RED,
            "normal",
        ),
        ButtonStyle::Active => (
            theme::BG_CHIP_ACTIVE,
            theme::BORDER_CHIP_ACTIVE,
            theme::TEXT_PRIMARY,
            "bold",
        ),
    };

    rsx! {
        rect {
            padding: "{padding}",
            corner_radius: "4",
            background: "{bg}",
            border: "{border}",
            main_align: "center",
            cross_align: "center",
            onpointerdown: move |_| onpress.call(()),
            label {
                font_size: "{font_size}",
                font_weight: "{weight}",
                color: "{fg}",
                max_lines: "1",
                "{label}"
            }
        }
    }
}

/// Radio-style selectable chip that visually highlights whichever option is currently selected.
#[component]
pub fn SegmentedChip(
    /// Visible option label.
    label: String,
    /// `true` when this option represents the active selection.
    selected: bool,
    /// When `Some(true)`, uses tighter padding and `11px` font size.
    compact: Option<bool>,
    /// Callback fired on pointer press.
    onpress: EventHandler<()>,
) -> Element {
    let is_compact = compact.unwrap_or(false);
    let padding = if is_compact { "2 7" } else { "3 9" };
    let font_size = if is_compact { "11" } else { "12" };

    let (bg, border, fg, weight) = if selected {
        (
            theme::BG_CHIP_ACTIVE,
            theme::BORDER_CHIP_ACTIVE,
            theme::TEXT_PRIMARY,
            "bold",
        )
    } else {
        (theme::BG_CHIP, theme::BORDER_CHIP, theme::TEXT_MUTED, "normal")
    };

    rsx! {
        rect {
            padding: "{padding}",
            corner_radius: "4",
            background: "{bg}",
            border: "{border}",
            main_align: "center",
            cross_align: "center",
            onpointerdown: move |_| onpress.call(()),
            label {
                font_size: "{font_size}",
                font_weight: "{weight}",
                color: "{fg}",
                max_lines: "1",
                "{label}"
            }
        }
    }
}

/// Thin vertical divider separating logical command groups in a horizontal bar.
#[component]
pub fn ToolbarDivider() -> Element {
    rsx! {
        rect {
            width: "1",
            height: "20",
            background: theme::BORDER_SUBTLE,
        }
    }
}

/// Small uppercase caption prefixing a group of related toolbar or strip controls.
#[component]
pub fn GroupCaption(
    /// Group caption label (e.g. `"ASSETS"`, `"SCHEMA"`, `"TOOLS"`, `"BUILT-IN"`).
    text: String,
) -> Element {
    rsx! {
        label {
            font_size: "10",
            font_weight: "bold",
            color: theme::TEXT_DIM,
            max_lines: "1",
            "{text}"
        }
    }
}

/// Builds a compact dark [`InputThemeWith`] matching the editor's typography and dark surface palette.
pub fn compact_input_theme() -> InputThemeWith {
    theme_with!(InputTheme {
        margin: "0".into(),
        background: theme::BG_INSET.into(),
        hover_background: theme::BG_ROW.into(),
        border_fill: theme::BORDER_SUBTLE.into(),
        focus_border_fill: theme::ACCENT_BLUE.into(),
        font_theme: theme_with!(FontTheme {
            color: theme::TEXT_PRIMARY.into(),
        }),
    })
}

/// Width-controlled dark [`Input`] wrapper using [`compact_input_theme`] and uniform padding.
#[component]
pub fn StyledInput(
    /// Outer width constraint (e.g. `"fill"`, `"260"`, `"120"`).
    width: Option<String>,
    /// Controlled input text value.
    value: String,
    /// Optional placeholder string shown when empty.
    placeholder: Option<String>,
    /// Callback emitted whenever the text changes.
    onchange: EventHandler<String>,
) -> Element {
    let w = width.unwrap_or_else(|| "fill".to_owned());
    rsx! {
        rect {
            width: "{w}",
            overflow: "clip",
            Input {
                width: "fill",
                theme: compact_input_theme(),
                value: value,
                placeholder: placeholder,
                onchange: move |txt| onchange.call(txt),
            }
        }
    }
}
