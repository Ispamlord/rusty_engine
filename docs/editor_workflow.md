# Editor Workflow

`engine_editor_app` is the project-facing authoring tool for `rusty_engine`. It combines project management, graph editing, asset browsing, and a live runtime preview.

## Start Here

Run the editor from the workspace root:

```bash
cargo run -p engine_editor_app
```

For a headless startup smoke check:

```bash
cargo run -p engine_editor_app -- --project . --scene assets/sample_scene.ron --smoke
```

## Project Manager

The editor opens with a Project Manager dialog first. Use it to:

- create a new project in a chosen folder,
- open an existing project root,
- select an optional scene file,
- reuse recent projects.

The dialog can be closed without switching projects, but the editor is designed to make a project choice explicit at startup.

## Workspace Modes

The editor now exposes two authoring workspaces:

- `Gameplay / Script` for gameplay flow, object setup, and asset references,
- `Render Pipeline` for render and compute logic.

The node palette and right-click canvas menu filter to the active workspace, so you can keep gameplay logic and render logic organized without hiding the underlying graph data.

## Asset Workflow

The Assets panel lists discovered project assets and provides a lightweight preview for the selected item.

Common actions:

- click an asset to inspect it,
- drag an asset into the graph canvas to create an `AssetReference` node,
- use `Load Graph As Scene` on graph assets,
- refresh the asset index after adding files on disk.

## Graph Editing

Right-click the graph canvas to add nodes for the active workspace. Nodes can be connected with compatible pins, then edited through the Inspector.

The current authoring set includes gameplay-oriented nodes such as:

- `GameplayEvent`
- `GameplayFlow`
- `MathState`
- `ScriptBehavior`
- `ObjectInitializer`

and render-oriented nodes such as:

- `RenderPass`
- `ComputePass`
- `BuildExport`

## Runtime Preview

The right side of the editor shows the live runtime preview and simulation state.

- `Play` starts runtime simulation.
- `Stop` pauses it.
- `Step` advances one simulation tick while paused.
- `Hot Recompile` pushes the current graph into the runtime without restarting the editor.

The runtime tick counter only advances while the runtime is actively playing or stepped manually.

## Default Project

Creating a new project seeds basic shape assets and a starter scene so the editor opens with visible content instead of an empty preview.

## Typical Loop

1. Open or create a project.
2. Pick the workspace that matches the kind of logic you want to edit.
3. Add nodes or drag assets into the graph.
4. Adjust node settings in the Inspector.
5. Recompile and preview the result.
6. Save the scene when the graph is ready.