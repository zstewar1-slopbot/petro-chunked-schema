//! Shared strongly-typed Freya 0.5 UI primitives for `petro-chunked-schema-editor`.

use freya::prelude::*;

use super::theme;

/// Visual emphasis variant for [`action_button`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    /// Highlighted affirmative action (e.g. `Save*`, `Run Validate Now`).
    Primary,
    /// Neutral secondary action (e.g. `Close`, `Up`, `Cancel`).
    Secondary,
    /// Destructive action (e.g. `Delete Field`, `Discard Changes`, `Remove`).
    Danger,
    /// Toggleable action currently in an active/open state.
    Active,
}

/// Consistently styled dark action button with crisp borders and uniform typography.
pub fn action_button(
    text: impl Into<String>,
    style: ButtonStyle,
    compact: bool,
    mut on_press: impl FnMut() + 'static,
) -> Element {
    let text = text.into();
    let pad = if compact { (2.0, 8.0) } else { (4.0, 10.0) };
    let font_size = if compact { 11.0 } else { 12.0 };

    let (bg, border, fg, weight) = match style {
        ButtonStyle::Primary => (
            theme::BG_BTN_PRIMARY,
            theme::BORDER_BTN_PRIMARY,
            theme::TEXT_PRIMARY,
            FontWeight::BOLD,
        ),
        ButtonStyle::Secondary => (
            theme::BG_CHIP,
            theme::BORDER_CHIP,
            theme::TEXT_PRIMARY,
            FontWeight::NORMAL,
        ),
        ButtonStyle::Danger => (
            theme::BG_BTN_DANGER,
            theme::BORDER_BTN_DANGER,
            theme::ACCENT_RED,
            FontWeight::NORMAL,
        ),
        ButtonStyle::Active => (
            theme::BG_CHIP_ACTIVE,
            theme::BORDER_CHIP_ACTIVE,
            theme::TEXT_PRIMARY,
            FontWeight::BOLD,
        ),
    };

    rect()
        .padding(pad)
        .corner_radius(4.0)
        .background(bg)
        .border(theme::border_all(border))
        .main_align(Alignment::Center)
        .cross_align(Alignment::Center)
        .on_pointer_down(move |e: Event<PointerEventData>| {
            e.stop_propagation();
            on_press();
        })
        .child(
            label()
                .font_size(font_size)
                .font_weight(weight)
                .color(fg)
                .max_lines(1)
                .text(text),
        )
        .into()
}

/// Radio-style selectable chip that visually highlights whichever option is currently selected.
pub fn segmented_chip(
    text: impl Into<String>,
    selected: bool,
    compact: bool,
    mut on_press: impl FnMut() + 'static,
) -> Element {
    let text = text.into();
    let pad = if compact { (2.0, 7.0) } else { (3.0, 9.0) };
    let font_size = if compact { 11.0 } else { 12.0 };

    let (bg, border, fg, weight) = if selected {
        (
            theme::BG_CHIP_ACTIVE,
            theme::BORDER_CHIP_ACTIVE,
            theme::TEXT_PRIMARY,
            FontWeight::BOLD,
        )
    } else {
        (
            theme::BG_CHIP,
            theme::BORDER_CHIP,
            theme::TEXT_MUTED,
            FontWeight::NORMAL,
        )
    };

    rect()
        .padding(pad)
        .corner_radius(4.0)
        .background(bg)
        .border(theme::border_all(border))
        .main_align(Alignment::Center)
        .cross_align(Alignment::Center)
        .on_pointer_down(move |e: Event<PointerEventData>| {
            e.stop_propagation();
            on_press();
        })
        .child(
            label()
                .font_size(font_size)
                .font_weight(weight)
                .color(fg)
                .max_lines(1)
                .text(text),
        )
        .into()
}

/// Thin vertical divider separating logical command groups in a horizontal bar.
pub fn toolbar_divider() -> Element {
    rect()
        .width(Size::px(1.0))
        .height(Size::px(20.0))
        .background(theme::BORDER_SUBTLE)
        .into()
}

/// Small uppercase caption prefixing a group of related toolbar or card controls.
pub fn group_caption(text: impl Into<String>) -> Element {
    label()
        .font_size(10.0)
        .font_weight(FontWeight::BOLD)
        .color(theme::TEXT_DIM)
        .max_lines(1)
        .text(text.into())
        .into()
}

/// Compact dark text input bound directly to a `State<String>`.
pub fn styled_input(
    width: Size,
    value: State<String>,
    placeholder: &'static str,
) -> Element {
    Input::new(value)
        .compact()
        .width(width)
        .placeholder(placeholder)
        .background(theme::BG_INSET)
        .focus_background(theme::BG_ROW)
        .border_fill(theme::BORDER_SUBTLE)
        .focus_border_fill(theme::ACCENT_BLUE)
        .color(theme::TEXT_PRIMARY)
        .placeholder_color(theme::TEXT_DIM)
        .corner_radius(4.0)
        .into()
}

/// Compact dark text input backed by a `State<String>` that also fires `on_change` immediately on
/// every edit for live in-memory auto-apply.
pub fn styled_input_live(
    width: Size,
    mut value: State<String>,
    placeholder: &'static str,
    mut on_change: impl FnMut(String) + 'static,
) -> Element {
    Input::new(WritableSetter::new(value, move |txt: String| {
        value.set(txt.clone());
        on_change(txt);
    }))
    .compact()
    .width(width)
    .placeholder(placeholder)
    .background(theme::BG_INSET)
    .focus_background(theme::BG_ROW)
    .border_fill(theme::BORDER_SUBTLE)
    .focus_border_fill(theme::ACCENT_BLUE)
    .color(theme::TEXT_PRIMARY)
    .placeholder_color(theme::TEXT_DIM)
    .corner_radius(4.0)
    .into()
}
