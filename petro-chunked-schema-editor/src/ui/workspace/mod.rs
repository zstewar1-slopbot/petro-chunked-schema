//! Three-pane main workspace shell combining the chunk tree, hex inspector, and schema stack.

mod chunk_tree;
mod hex_inspector;
mod schema_editor;

use freya::prelude::*;

use crate::editor_ops::resolve_selected_node;
use crate::ui::context::EditorState;

use chunk_tree::ChunkTreePane;
use hex_inspector::HexInspectorPane;
use schema_editor::SchemaEditorPane;

/// Main horizontal 3-pane workspace: chunk/mini-chunk tree (left), color-coded hex inspector
/// (center), and non-indenting interactive schema stack (right).
#[component]
pub fn Workspace() -> Element {
    let state = use_context::<EditorState>();
    let resolved = state.resolved_schema.read();
    let df_guard = state.dynamic_file.read();
    let tree_path = state.selected_tree_path.read();

    let selected_node = df_guard
        .as_ref()
        .and_then(|df| resolve_selected_node(df, &resolved, &tree_path));

    rsx! {
        rect {
            width: "fill",
            height: "flex(1)",
            content: "flex",
            direction: "horizontal",

            ChunkTreePane {}
            HexInspectorPane { node: selected_node }
            SchemaEditorPane {}
        }
    }
}
