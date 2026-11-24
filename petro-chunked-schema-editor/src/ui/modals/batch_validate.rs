//! Batch validation centered modal scanning loose files and `.meg` archives.

use freya::prelude::*;

use crate::gamedata::{BatchValidationReport, run_batch_validation};
use crate::ui::context::EditorState;
use crate::ui::theme;
use crate::ui::widgets::{ActionButton, ButtonStyle, StyledInput};

use super::ModalDialog;

/// Centered dialog running batch validation of the active
/// [`ResolvedSchema`](petro_chunked_schema::ResolvedSchema) across all configured `GameData`
/// directories for a given file extension.
#[component]
pub fn BatchValidateModal() -> Element {
    let mut state = use_context::<EditorState>();
    let initial_ext = use_hook(|| state.persisted.read().validate_extension.clone());
    let mut ext_input = use_signal(|| initial_ext);
    let mut report = use_signal(|| None::<BatchValidationReport>);
    let schema_name = state.schema.read().name.clone();
    let dir_count = state.persisted.read().game_data_dirs.len();

    rsx! {
        ModalDialog {
            title: format!("Batch Validate `{schema_name}` Across GameData Directories & .meg Archives"),
            width: "900".to_owned(),
            height: "560".to_owned(),

            rect {
                direction: "horizontal",
                spacing: "8",
                cross_align: "center",
                label { font_size: "12", color: theme::TEXT_MUTED, "Extension:" }
                StyledInput {
                    width: Some("100".to_owned()),
                    value: ext_input.read().clone(),
                    placeholder: Some("alo".to_owned()),
                    onchange: move |txt| ext_input.set(txt),
                }
                ActionButton {
                    label: "Run Validate Now".to_owned(),
                    style: ButtonStyle::Primary,
                    onpress: move |_| {
                        let ext = ext_input.read().trim().to_owned();
                        let dirs = {
                            let mut p = state.persisted.write();
                            p.validate_extension = ext.clone();
                            p.save();
                            p.game_data_dirs.clone()
                        };
                        let resolved = state.resolved_schema.read().clone();
                        let rep = run_batch_validation(&resolved, &dirs, &ext);
                        state.set_status(format!(
                            "Batch validation on .{}: {}/{} passed ({} failed, {} skipped)",
                            rep.extension, rep.passed_count, rep.total_checked, rep.failed_count, rep.skipped_count
                        ));
                        report.set(Some(rep));
                    },
                }
                label {
                    font_size: "11",
                    color: theme::TEXT_DIM,
                    "Scanning {dir_count} configured GameData directory(s)"
                }
            }

            if let Some(rep) = report.read().as_ref() {
                rect {
                    width: "fill",
                    padding: "8 10",
                    corner_radius: "5",
                    background: theme::BG_SUB_CARD,
                    border: "1 solid rgb(48, 53, 68)",
                    label {
                        font_size: "12",
                        font_weight: "bold",
                        color: if rep.failed_count == 0 { theme::ACCENT_GREEN } else { theme::ACCENT_RED },
                        "Schema `{rep.schema_name}` on `.{rep.extension}` -> Checked: {rep.total_checked} | Passed: {rep.passed_count} | Failed: {rep.failed_count} | Skipped: {rep.skipped_count}"
                    }
                }
                ScrollView {
                    width: "fill",
                    height: "flex(1)",
                    rect {
                        width: "fill",
                        direction: "vertical",
                        spacing: "3",

                        if rep.failures.is_empty() {
                            rect {
                                width: "fill",
                                padding: "14",
                                corner_radius: "4",
                                background: theme::BG_INSET,
                                label {
                                    font_size: "12",
                                    color: theme::ACCENT_GREEN,
                                    "All {rep.passed_count} files passed validation with zero diagnostics."
                                }
                            }
                        }

                        for fail in &rep.failures {
                            {
                                let f_src = fail.source.clone();
                                let f_disp = fail.display_name.clone();
                                let first_diag = fail
                                    .diagnostics
                                    .first()
                                    .map(|d| format!("[{} @ 0x{:X}] {}", d.tree_path, d.file_offset, d.message))
                                    .unwrap_or_default();
                                let diag_len = fail.diagnostics.len();
                                rsx! {
                                    rect {
                                        width: "fill",
                                        height: "30",
                                        padding: "0 8",
                                        corner_radius: "4",
                                        background: theme::BG_INSET,
                                        direction: "horizontal",
                                        spacing: "8",
                                        cross_align: "center",
                                        overflow: "clip",
                                        ActionButton {
                                            label: "Open in Editor".to_owned(),
                                            style: ButtonStyle::Secondary,
                                            compact: Some(true),
                                            onpress: move |_| {
                                                state.open_file(f_src.clone());
                                                state.close_modal();
                                            },
                                        }
                                        label {
                                            font_size: "12",
                                            color: theme::ACCENT_RED,
                                            max_lines: "1",
                                            "{f_disp} ({diag_len} error(s)) — {first_diag}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                rect {
                    width: "fill",
                    height: "flex(1)",
                    padding: "16",
                    corner_radius: "5",
                    background: theme::BG_INSET,
                    border: "1 solid rgb(46, 51, 66)",
                    label {
                        font_size: "12",
                        color: theme::TEXT_MUTED,
                        "Enter an extension (such as `alo` or `ala`) and press `Run Validate Now` to validate every matching loose file and .meg entry in memory."
                    }
                }
            }
        }
    }
}
