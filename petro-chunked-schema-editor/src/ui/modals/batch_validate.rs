//! Freestanding batch validation window scanning loose files and `.meg` archives.

use freya::prelude::*;

use crate::gamedata::{BatchValidationReport, run_batch_validation};
use crate::ui::context::EditorState;
use crate::ui::theme;
use crate::ui::widgets::{ButtonStyle, action_button, styled_input};

/// Freestanding window running batch validation of the active
/// [`ResolvedSchema`](petro_chunked_schema::ResolvedSchema) across all configured `GameData`
/// directories for a given file extension.
#[derive(PartialEq)]
pub struct BatchValidateWindow;

impl Component for BatchValidateWindow {
    fn render(&self) -> impl IntoElement {
        let state = EditorState::current();
        let initial_ext = use_hook(|| state.persisted.read().validate_extension.clone());
        let ext_input = use_state(|| initial_ext);
        let mut report = use_state(|| None::<BatchValidationReport>);
        let dir_count = state.persisted.read().game_data_dirs.len();
        let mut st_run = state;
        let mut st_close = state;

        let report_body: Element = if let Some(rep) = report.read().as_ref() {
            let failure_rows: Vec<Element> = rep
                .failures
                .iter()
                .map(|fail| {
                    let f_src = fail.source.clone();
                    let f_disp = fail.display_name.clone();
                    let first_diag = fail
                        .diagnostics
                        .first()
                        .map(|d| format!("[{} @ 0x{:X}] {}", d.tree_path, d.file_offset, d.message))
                        .unwrap_or_default();
                    let diag_len = fail.diagnostics.len();
                    let mut st_open = state;

                    rect()
                        .width(Size::fill())
                        .height(Size::px(30.0))
                        .padding((0.0, 8.0))
                        .corner_radius(4.0)
                        .background(theme::BG_INSET)
                        .horizontal()
                        .spacing(8.0)
                        .cross_align(Alignment::Center)
                        .overflow(Overflow::Clip)
                        .child(action_button(
                            "Open in Editor",
                            ButtonStyle::Secondary,
                            true,
                            move || {
                                st_open.open_file(f_src.clone());
                                st_open.close_current_modal_window();
                            },
                        ))
                        .child(
                            label()
                                .font_size(12.0)
                                .color(theme::ACCENT_RED)
                                .max_lines(1)
                                .text(format!("{f_disp} ({diag_len} error(s)) — {first_diag}")),
                        )
                        .into()
                })
                .collect();

            rect()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .content(Content::Flex)
                .vertical()
                .spacing(8.0)
                .child(
                    rect()
                        .width(Size::fill())
                        .padding((8.0, 10.0))
                        .corner_radius(5.0)
                        .background(theme::BG_SUB_CARD)
                        .border(theme::border_all(theme::BORDER_CARD))
                        .child(
                            label()
                                .font_size(12.0)
                                .font_weight(FontWeight::BOLD)
                                .color(if rep.failed_count == 0 {
                                    theme::ACCENT_GREEN
                                } else {
                                    theme::ACCENT_RED
                                })
                                .text(format!(
                                    "Schema `{}` on `.{}` -> Checked: {} | Passed: {} | Failed: {} | Skipped: {}",
                                    rep.schema_name,
                                    rep.extension,
                                    rep.total_checked,
                                    rep.passed_count,
                                    rep.failed_count,
                                    rep.skipped_count
                                )),
                        ),
                )
                .child(
                    ScrollView::new()
                        .width(Size::fill())
                        .height(Size::flex(1.0))
                        .child(
                            rect()
                                .width(Size::fill())
                                .vertical()
                                .spacing(3.0)
                                .maybe_child(rep.failures.is_empty().then(|| {
                                    rect()
                                        .width(Size::fill())
                                        .padding(14.0)
                                        .corner_radius(4.0)
                                        .background(theme::BG_INSET)
                                        .child(
                                            label()
                                                .font_size(12.0)
                                                .color(theme::ACCENT_GREEN)
                                                .text(format!(
                                                    "All {} files passed validation with zero diagnostics.",
                                                    rep.passed_count
                                                )),
                                        )
                                }))
                                .children(failure_rows),
                        ),
                )
                .into()
        } else {
            rect()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .padding(16.0)
                .corner_radius(5.0)
                .background(theme::BG_INSET)
                .border(theme::border_all(theme::BORDER_CARD))
                .child(
                    label()
                        .font_size(12.0)
                        .color(theme::TEXT_MUTED)
                        .text("Enter an extension (such as `alo` or `ala`) and press `Run Validate Now` to validate every matching loose file and .meg entry in memory."),
                )
                .into()
        };

        rect()
            .width(Size::fill())
            .height(Size::fill())
            .content(Content::Flex)
            .vertical()
            .spacing(10.0)
            .child(
                rect()
                    .width(Size::fill())
                    .horizontal()
                    .main_align(Alignment::SpaceBetween)
                    .cross_align(Alignment::Center)
                    .child(
                        rect()
                            .horizontal()
                            .spacing(8.0)
                            .cross_align(Alignment::Center)
                            .child(
                                label()
                                    .font_size(12.0)
                                    .color(theme::TEXT_MUTED)
                                    .text("Extension:"),
                            )
                            .child(styled_input(Size::px(100.0), ext_input, "alo"))
                            .child(action_button(
                                "Run Validate Now",
                                ButtonStyle::Primary,
                                false,
                                move || {
                                    let ext = ext_input.read().trim().to_owned();
                                    let dirs = {
                                        let mut p = st_run.persisted.write();
                                        p.validate_extension = ext.clone();
                                        p.save();
                                        p.game_data_dirs.clone()
                                    };
                                    let resolved = st_run.resolved_schema.read().clone();
                                    let rep = run_batch_validation(&resolved, &dirs, &ext);
                                    st_run.set_status(format!(
                                        "Batch validation on .{}: {}/{} passed ({} failed, {} skipped)",
                                        rep.extension,
                                        rep.passed_count,
                                        rep.total_checked,
                                        rep.failed_count,
                                        rep.skipped_count
                                    ));
                                    report.set(Some(rep));
                                },
                            ))
                            .child(
                                label()
                                    .font_size(11.0)
                                    .color(theme::TEXT_DIM)
                                    .text(format!(
                                        "Scanning {dir_count} configured GameData directory(s)"
                                    )),
                            ),
                    )
                    .child(action_button("Close", ButtonStyle::Secondary, false, move || {
                        st_close.close_current_modal_window();
                    })),
            )
            .child(report_body)
    }
}
