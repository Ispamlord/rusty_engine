# rusty_engine

`rusty_engine` is a native Rust 2D engine workspace targeting:

- Vulkan (Linux + Windows)
- DirectX 12 (Windows)
- DirectX 11 compatibility (Windows)

## Current implementation status

### Implemented now (M1-M4 scoped runtime implementation)
- Backend lifecycle contract in `engine_render_api`:
  - `initialize`, `create_surface`, `resize`, `acquire_frame`, `record_render_graph`, `submit`, `present`, `destroy`
- Backend diagnostics and recovery semantics:
  - event stream, pass timings, fallback counters, swapchain/device-loss counters
  - recoverable error helpers (`SurfaceOutOfDate`, `DeviceLost`, `DeviceRemoved`)
- Vulkan runtime path:
  - instance/device/queue bring-up
  - native surface + swapchain creation from window/display handles
  - acquire/submit/present with semaphore/fence sync
  - resize-triggered swapchain recreation and out-of-date mapping
- DX12 and DX11 compatibility runtime paths:
  - native device bring-up and frame command submission flow
  - Win32-handle-aware surface attach contract and diagnostics
  - compatibility render/compute pass traversal with fallback diagnostics
- Node compiler and graph runtime:
  - deterministic topo compile
  - executable GPU pass plan with dependencies/resource access metadata
  - backend-aware fallback behavior and strict-GPU mode
  - shader entry/profile metadata on nodes
- Asset pipeline hardening:
  - deterministic shader compile keys from source/include hashes + compiler signature + flags/profiles
  - include dependency tracking and cache invalidation of dependents
  - external shader compiler invocation with strict/fallback behavior
- Engine runtime contract:
  - deterministic frame phases: input -> fixed-step physics -> gameplay -> audio -> render
  - fixed-step clamp policy to prevent spiral-of-death
  - frame pacing controls and recovery policy controls from config
  - hot-reload application for graph/shader changes with runtime-safe fallback
- Performance and CI:
  - perf smoke and perf regression harness examples
  - CI matrix with CPU jobs, optional GPU jobs, and advisory Proton/Wine DX checks

## Remaining limitations
- DX12/DX11 native swapchain/backbuffer binding is scaffolded by surface contract and diagnostics; full production-grade swapchain/backbuffer RTV lifetime management still needs native Windows runtime validation.
- Real shader/pipeline GPU draw payloads are still backend scaffolds; command traversal and diagnostics are in place, but full material/shader binding production depth is not finalized.

## Workspace crates
- `engine_core`: config schema (window/backend/fixed-step/pacing/perf/shader-toolchain/recovery)
- `engine_render_api`: backend lifecycle trait, render graph IR, diagnostics/recovery types
- `backend_vulkan`: Vulkan backend implementation
- `backend_dx12`: DX12 backend implementation
- `backend_dx11`: DX11 compatibility backend implementation
- `engine_platform`: backend selection policy (`DX12 -> Vulkan -> DX11` on Windows, `Vulkan` on Linux)
- `engine_nodes`: graph schema + compile pipeline to ECS jobs and executable GPU plan
- `engine_assets`: RON graph load/save, shader build cache, hot-reload scanning
- `engine_audio`: Kira audio runtime wrapper + ECS sync system
- `engine_physics`: Rapier2D world resource + ECS sync system
- `engine_editor`: overlay/profiler/diagnostics panel model
- `engine_editor_app`: dedicated visual-programming editor app (graph canvas, inspector, diagnostics, embedded play mode)
- `engine_app`: runtime orchestration, phase scheduling, backend execution, recovery/hot-reload

## Quick start

1. Run checks:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

2. Smoke demo:

```bash
cargo run -p engine_app --example bootstrap
```

3. Performance harnesses:

```bash
cargo run -p engine_app --example perf_smoke
cargo run -p engine_app --example perf_regression
```

Optional custom paths:

```bash
cargo run -p engine_app --example bootstrap config/default.ron assets/sample_scene.ron
```

## Editor usage

Primary command:

```bash
cargo run -p engine_editor_app
```

Optional arguments:

```bash
cargo run -p engine_editor_app -- [project_path]
cargo run -p engine_editor_app -- --project <path> --scene <path>
```

Headless startup smoke mode (for CI):

```bash
cargo run -p engine_editor_app -- --project . --scene assets/sample_scene.ron --smoke
```

First-scene workflow:

1. Launch editor from project root.
2. Use graph canvas context menu to add nodes and connect compatible pins.
3. Select a node to edit execution target/fallback in Inspector.
4. Click `Hot Recompile` and use `Play` / `Stop` / `Step` controls.
5. Save with `Save` or `Save As autosave_scene.ron`.

## CI

Workflow: `.github/workflows/ci.yml`

- CPU fallback jobs (Linux + Windows): fmt, clippy, tests, bootstrap smoke, perf regression
- Optional GPU jobs (`ENABLE_GPU_CI=true`): Linux Vulkan smoke/perf, Windows DX smoke/perf
- Optional Proton advisory (`ENABLE_PROTON_CI=true`): Windows-target DX build + advisory Wine smoke on Linux
