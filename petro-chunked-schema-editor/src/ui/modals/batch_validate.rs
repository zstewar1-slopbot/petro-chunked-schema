//! Batch validation drawer modal scanning loose files and `.meg` archives.

use freya::prelude::*;

use crate::gamedata::{BatchValidationReport, run_batch_validation};
use crate::ui::context::EditorState;
use crate::ui::theme;

use super::ModalFrame;

/// Modal drawer running batch validation of the active [`ResolvedSchema`](petro_chunked_schema::ResolvedSchema)
/// across all configured `GameData` directories for a given extension.
#[component]
pub fn BatchValidateModal() -> Element {
    let mut state = use_context::<EditorState>();
    let initial_ext = use_hook(|| state.persisted.read().validate_extension.clone());
    let mut ext_input = use_signal(|| initial_ext);
    let mut report = use_signal(|| None::<BatchValidationReport>);
    let schema_name = state.schema.read().name.clone();

    rsx! {
        ModalFrame {
            title: format!("Batch Validate `{schema_name}` Across GameData Directories & .meg Archives"),
            height: "305".to_owned(),

            rect {
                direction: "horizontal",
                spacing: "8",
                cross_align: "center",
                label { font_size: "12", "Extension:" }
                Input {
                    value: ext_input.read().clone(),
                    placeholder: "alo",
                    onchange: move |txt| ext_input.set(txt),
                }
                Button {
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
                    label { "Run Validate Now" }
                }
            }

            if let Some(rep) = report.read().as_ref() {
                label {
                    font_size: "12",
                    color: if rep.failed_count == 0 { theme::ACCENT_GREEN } else { theme::ACCENT_RED },
                    "Schema `{rep.schema_name}` on `.{rep.extension}` -> Checked: {rep.total_checked} | Passed: {rep.passed_count} | Failed: {rep.failed_count} | Skipped: {rep.skipped_count}"
                }
                ScrollView {
                    width: "fill",
                    height: "fill",
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
                                    padding: "3 6",
                                    direction: "horizontal",
                                    spacing: "8",
                                    cross_align: "center",
                                    Button {
                                        onpress: move |_| {
                                            state.open_file(f_src.clone());
                                            state.close_modal();
                                        },
                                        label { "Open in Editor" }
                                    }
                                    label {
                                        font_size: "12",
                                        color: theme::ACCENT_RED,
                                        "{f_disp} ({diag_len} error(s)) — {first_diag}"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
