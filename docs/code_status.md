# Code Status

Last updated: 2026-07-10

This document summarizes what is currently implemented in code, what is usable with caveats, and what is still scaffold/in progress.

Status legend:
- DONE: Implemented and in active use.
- PARTIAL: Implemented but with known limitations.
- TODO: Planned or scaffold-level only.

## Quality Gate Status

- `cargo test --workspace` passes.
- `cargo clippy --workspace --all-targets -- -D warnings` passes.
- All crates have at least basic unit-test coverage; config, platform, render API, backends, nodes, assets, runtime, physics, audio, and editor crates received a production-hardening pass.

## Recent Production Hardening

- **Config validation**: `engine_core` now validates every config section on load, rejecting values that would cause division-by-zero, impossible worker ranges, empty toolchain paths, or unusable window dimensions.
- **Platform docs/tests**: `engine_platform` gained rustdoc and additional tests for backend selection edge cases.
- **Render API / backends**: `engine_render_api` added overflow-safe viewport buffer sizing and diagnostics tests; Vulkan/DX11/DX12 backends use the safe sizing and have basic unit tests. Vulkan `resize` and extent selection guard against zero dimensions; fence waits use a finite timeout instead of `u64::MAX`. GPU instrumentation and frame-capture hooks are now exposed through the `GraphicsBackend` trait (`set_gpu_timestamps_enabled`, `configure_gpu_instrumentation`, `request_frame_capture`, `poll_frame_capture`) and implemented by all three backends via `BackendInstrumentationState`.
- **Node compiler**: `engine_nodes` now rejects self-dependencies with a dedicated error and no longer panics on an internal node lookup miss.
- **Asset pipeline**: `engine_assets` fails explicitly on missing includes, unsupported include syntax, and malformed shader metadata; temporary compiler output is cleaned up on both success and failure.
- **Runtime systems**: `engine_app` drains unbounded script event/log buffers each frame, hardens `spawn_object`/`despawn_object`/`set_sprite` against invalid ids and dimensions, and adds focused unit tests. Frame pacing now spin-yields for the last ~2 ms instead of relying solely on `thread::sleep`, which removes most OS sleep-granularity stutter (especially on Windows). The render-graph submission path no longer clones the whole graph just to sort sprites/batches; it sorts in place. `engine_audio` and `engine_physics` received module docs and unit tests documenting the currently stubbed integrations.
- **Editor**: `engine_editor` added undoable node renames, dependency-cycle detection in `ConnectNodes` and document validation, a depth guard for recursive asset indexing, and centralized payload syncing from settings. `engine_editor_app` was updated to use the new command signatures.

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
- **Runtime shader/material binding**: `engine_app::run_render_phase` now preloads every unique `Material::shader_asset` from the active graph through `engine_assets::AssetBuildCache` and forwards the compiled bytecode to the active backend via `GraphicsBackend::preload_shader_bytecode`. Vulkan, DX11, and DX12 cache the bytecode and lazily build user pipelines/descriptors: Vulkan creates material graphics pipelines (built-in VS + user FS) and compute pipelines from SPIR-V; DX11 creates user pixel/compute shaders from DXBC; DX12 creates user graphics/compute PSOs from DXIL. When a user shader is missing or fails to compile, each backend falls back to its built-in sprite/compute path.

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
- **Runtime script jobs execute in dependency waves with conflict-safe parallel dispatch for independent jobs.** Script jobs can now declare explicit `read_set` / `write_set` settings; the runtime detects WAW, RAW, and WAR conflicts and only parallelizes non-conflicting jobs. Legacy `script_parallel_key` / `object_id` / `object_name` / `layer_id` markers continue to work.
- DONE: Script runtime exposes mouse input (`mouse_x`, `mouse_y`, `mouse_down`) and sprite-based UI primitives (`draw_rect`, `draw_text` placeholder) for in-game HUDs.
- DONE: Editor viewport forwards mouse position/buttons into the runtime so play-mode scripts can use mouse aim and click interactions.

## Example Projects

- DONE: `rusty-roguelike-shooter` is now an editor project under `project/` with a `.scene.ron`, Rhai scripts, and a standalone binary that loads the project via `EngineApp`. It uses mouse aiming/shooting, procedural enemy spawning, scaling difficulty, a shop every 2 levels, and sprite-based UI overlays.

## Recommended Next Milestones

- PARTIAL: Full descriptor/buffer resource binding for user shaders — bytecode is now consumed, but real textures/storage buffers from `RenderGraph` resources and `ShaderBinding` are not yet bound. The next step is to allocate GPU memory for `create_texture`/`create_render_target`/graph storage buffers and bind them through descriptors/root signatures.
- DONE: Real GPU draw/dispatch recording on Vulkan (sprite draw + compute dispatch) and real sprite rendering on DX11/DX12 (code written, D3D paths not hardware-verified on this Linux host).
- DONE: Runtime shader/material binding pipeline (`preload_shader_bytecode`) across Vulkan, DX11, and DX12.
- DONE: DX11/DX12 native swapchain/backbuffer RTV creation and resize recreation on Windows paths.
- DONE: Richer script conflict analysis via explicit `read_set` / `write_set` settings on `ScriptBehavior`/`Custom` nodes; the runtime detects WAW, RAW, and WAR conflicts before parallel dispatch. Legacy `script_parallel_key` markers continue to work.
- DONE: Extend perf regression thresholds to gate app/runtime/backend timing channels independently per backend profile (implemented in `engine_app/examples/perf_regression.rs`).
- DONE: Backend-specific GPU instrumentation and frame capture hooks (`GraphicsBackend` API + backend implementations + diagnostics counters).
