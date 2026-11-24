//! Schema URI / YAML load, save, and source inspection drawer modal.

use std::path::PathBuf;

use freya::prelude::*;
use petro_chunked_schema::source::SchemaRef;

use crate::gamedata::save_schema_to_path;
use crate::ui::context::EditorState;
use crate::ui::theme;

use super::ModalFrame;

/// Modal drawer for loading schemas via [`SchemaRef`] (`builtin:...`, `file:...`, or path),
/// saving the active schema to disk, and viewing the live YAML serialization.
#[component]
pub fn YamlSchemaModal() -> Element {
    let mut state = use_context::<EditorState>();
    let initial_ref = use_hook(|| {
        state
            .persisted
            .read()
            .schema_ref
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "custom_schema.yaml".to_owned())
    });
    let mut ref_input = use_signal(|| initial_ref);
    let yaml_text = state
        .schema
        .read()
        .to_yaml_string()
        .unwrap_or_else(|e| format!("# YAML serialization error: {e}"));

    rsx! {
        ModalFrame {
            title: "Load / Save Schema (URI `builtin:...` or YAML File Path)".to_owned(),
            height: "310".to_owned(),

            rect {
                direction: "horizontal",
                spacing: "8",
                cross_align: "center",
                Input {
                    value: ref_input.read().clone(),
                    placeholder: "builtin:Alo or path/to/schema.yaml",
                    onchange: move |txt| ref_input.set(txt),
                }
                Button {
                    onpress: move |_| {
                        let raw = ref_input.read().trim().to_owned();
                        match SchemaRef::parse(&raw) {
                            Ok(sref) => state.load_schema_ref(sref),
                            Err(e) => state.set_status(format!("Invalid schema reference: {e}")),
                        }
                    },
                    label { "Load Ref / YAML" }
                }
                Button {
                    onpress: move |_| {
                        let raw = ref_input.read().trim().to_owned();
                        let path = match SchemaRef::parse(&raw) {
                            Ok(SchemaRef::File(p)) => p,
                            Ok(SchemaRef::Builtin(_)) => {
                                state.set_status("Cannot overwrite a built-in schema; provide a file path to save.".to_owned());
                                return;
                            }
                            Err(_) => PathBuf::from(raw),
                        };
                        let cur_s = state.schema.read().clone();
                        match save_schema_to_path(&cur_s, &path) {
                            Ok(()) => {
                                let sref = SchemaRef::File(path.clone());
                                ref_input.set(sref.to_string());
                                {
                                    let mut p = state.persisted.write();
                                    p.schema_ref = Some(sref);
                                    p.save();
                                }
                                state.set_status(format!("Saved schema YAML to {}", path.display()));
                            }
                            Err(e) => state.set_status(format!("Error saving YAML schema: {e}")),
                        }
                    },
                    label { "Save YAML File" }
                }
            }

            ScrollView {
                width: "fill",
                height: "fill",
                rect {
                    padding: "8",
                    background: theme::BG_INSET,
                    label {
                        font_size: "12",
                        font_family: "monospace",
                        "{yaml_text}"
                    }
                }
            }
        }
    }
}
