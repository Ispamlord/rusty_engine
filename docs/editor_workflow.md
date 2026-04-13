# Editor Workflow

`engine_editor_app` is the visual-programming frontend for `rusty_engine`.
It now edits unified scene documents (`.scene.ron`) that include hierarchy/layers/components plus embedded logic graph.

For a full implementation and feature status snapshot, see [docs/code_status.md](docs/code_status.md).

## Start

```bash
cargo run -p engine_editor_app
```

Headless startup smoke:

```bash
cargo run -p engine_editor_app -- --project . --scene assets/sample_scene.scene.ron --smoke
```

## Scene Authoring

The left panel includes:

- **Layers**: add layer, rename, toggle visibility/lock.
- **Hierarchy**: add/duplicate/delete objects and select active object.
- **Assets**: inspect assets and drag assets into the graph canvas.

Object editing is in **Inspector -> Object Inspector**:

- name, parent, layer assignment,
- transform,
- optional sprite/collider/camera components,
- custom properties.

## Logic Authoring

Graph editing uses `egui-snarl` with workspace split:

- `Gameplay / Script`
- `Render Pipeline`

Node authoring supports:

- right-click node creation,
- compatible pin linking,
- inspector edits for target/fallback/shader metadata,
- script node metadata (`script_asset`, `script_entry`, `script_phase`),
- hot recompile into embedded runtime.

## Runtime Viewport

Viewport panel now uses backend viewport readback and displays actual runtime frame content.

Controls:

- `Play` / `Stop` / `Restart` / `Step`
- `Hot Recompile`
- viewport pan/zoom and quick object focus.

## Safety and Recovery

- Transaction-backed history graph with branch checkout.
- Undo/redo from toolbar or shortcuts.
- Dirty-state guard before project switch.
- Autosave recovery modal if `.rusty_engine/editor_autosave.scene.ron` exists.
- Legacy graph-only `.ron` files are rejected in scene editor flow.

## Shortcuts

- `Ctrl/Cmd+S`: save scene
- `Ctrl/Cmd+Z`: undo
- `Ctrl/Cmd+Shift+Z` or `Ctrl/Cmd+Y`: redo
- `Ctrl/Cmd+C` / `Ctrl/Cmd+V`: copy/paste selected node/object
- `Ctrl/Cmd+D`: duplicate selected node/object
- `Delete`: delete selected node/object
- `Space`: play/stop toggle
- `F`: focus selected object in viewport
