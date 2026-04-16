# Code Status

Last updated: 2026-04-16

This document summarizes what is currently implemented in code, what is usable with caveats, and what is still scaffold/in progress.

Status legend:
- DONE: Implemented and in active use.
- PARTIAL: Implemented but with known limitations.
- TODO: Planned or scaffold-level only.

## What Is In The Codebase

- DONE: Workspace split by crate responsibilities (core, runtime, editor, nodes, assets, backends).
- DONE: Unified scene document format (.scene.ron) with hierarchy, layers, components, and embedded node graph.
- DONE: Editor workflow for scene and graph authoring with undo/redo, autosave recovery, and diagnostics.
- DONE: Script runtime integration through Rhai for ScriptBehavior and Custom node logic.
- DONE: Runtime frame pipeline (input -> fixed update -> gameplay -> audio -> render).

## Runtime And Pipeline Status

- DONE: Backend lifecycle contract and diagnostics API in engine_render_api.
- DONE: Graph compile pipeline with deterministic ordering and execution target lowering (CPU/GPU/Hybrid).
- DONE: Compile-time fallback and diagnostics for unsupported GPU targets.
- DONE: Gameplay script execution path, input bridge (keyboard), and scene mutation host APIs.
- DONE: Viewport camera controls and runtime-to-editor viewport readback.
- DONE: Adaptive viewport readback cadence balancing (1..4 frames) based on frame-time pressure.
- DONE: Lazy viewport synthesis in Vulkan/DX11/DX12 compatibility paths (deferred until readback request).
- PARTIAL: GPU pass metadata/planning is present, but full production-grade material/shader pipeline binding is not finished.
- PARTIAL: DX11/DX12 compatibility paths are usable but still not final production-depth render backend implementations.

## Editor Feature Status

- DONE: Scene hierarchy/layers/object inspector editing for non-custom and custom nodes.
- DONE: Node inspector staging workflow with apply semantics for stable editing.
- DONE: ScriptBehavior direct script editing and one-click conversion to Custom node assets.
- DONE: Viewport controls (pan/zoom/focus), runtime tick display, and diagnostics panel.
- DONE: FPS counter in toolbar and diagnostics (derived from CPU frame time).
- DONE: Readback cadence visibility in viewport panel.

## Assets And Custom Node Authoring

- DONE: Asset kind inference for shape presets and node config assets (.node.yml/.node.yaml).
- DONE: Project-local custom node templates and node registry seed files.
- DONE: Custom node config parsing, registry parsing, and type descriptor support in engine_nodes.
- PARTIAL: Custom node execution currently routes through script implementation path and does not yet include a fully separate native execution backend.

## Performance And Threading Status

- DONE: Perf smoke and perf regression examples, plus CI integration hooks.
- DONE: Runtime frame pacing controls and fixed-step spiral-of-death protection.
- DONE: Perf regression harness now measures full app-frame wall timing and records runtime/backend CPU timing breakdowns.
- DONE: Core-topology aware scheduler tuning now models high-clock vs many-core balancing through configurable scheduler bias and worker limits.
- DONE: Runtime script jobs execute in dependency waves with conflict-safe parallel dispatch for independent jobs.

## Recommended Next Milestones

- TODO: Implement full GPU draw/dispatch translation path with production shader/material binding.
- TODO: Add richer script conflict analysis (read/write sets) so more jobs can be auto-promoted to safe parallel execution without manual hints.
- TODO: Extend perf regression thresholds to gate app/runtime/backend timing channels independently per backend profile.
- TODO: Add backend-specific GPU instrumentation and frame capture hooks for deeper profiling.
