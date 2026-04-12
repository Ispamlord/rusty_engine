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

## Prerequisites

### Windows

Install these first:

1. Rust toolchain (stable, MSVC target)
   - Install `rustup` and run:

```bash
rustup default stable
rustup target add x86_64-pc-windows-msvc
```

2. Visual C++ toolchain + Windows SDK
   - Install Visual Studio 2022 Build Tools.
   - Include workload: `Desktop development with C++`.
   - Make sure a recent Windows SDK is selected.

3. Git

Optional, but recommended for full shader toolchain behavior:

1. Vulkan SDK (for `glslc`)
2. DirectX Shader Compiler (`dxc`)
3. FXC (`fxc`, usually provided by Windows SDK)

Windows notes:

- Default config expects `glslc`, `dxc`, and `fxc` on `PATH`.
- You can set explicit paths in `config/default.ron` under `shader_toolchain`.
- Shader toolchain strict mode is disabled by default (`strict: false`), so missing compilers can fall back instead of failing hard.

### Linux

Install these first:

1. Rust toolchain (stable)

```bash
rustup default stable
```

2. Git

3. System dependencies (Ubuntu/Debian):

```bash
sudo apt-get update
sudo apt-get install -y \
  libasound2-dev \
  libudev-dev \
  libx11-dev \
  libxrandr-dev \
  libxinerama-dev \
  libxcursor-dev \
  libxi-dev \
  libwayland-dev \
  libxkbcommon-dev \
  libvulkan-dev
```

Optional for full shader toolchain behavior:

1. Vulkan SDK / `glslc`

Linux notes:

- Linux runtime backend is Vulkan.
- If `glslc` is not found and strict mode remains disabled, shader compile can fall back instead of failing hard.

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

Editor guide:

- See [docs/editor_workflow.md](docs/editor_workflow.md) for the current project manager, workspace split, asset drag-and-drop, and node authoring flow.

First-scene workflow:

1. Launch editor from project root.
2. Use the Project Manager to open or create a project before entering the editor.
3. Switch between `Gameplay / Script` and `Render Pipeline` workspaces depending on the graph you want to edit.
4. Drag assets from the Assets panel into the graph to create asset reference nodes, or right-click the canvas to add nodes manually.
5. Select a node to edit execution target, fallback, and other node settings in Inspector.
6. Click `Hot Recompile` and use `Play` / `Stop` / `Step` controls.
7. Save with `Save` or `Save As autosave_scene.ron`.

## CI

Workflow: `.github/workflows/ci.yml`

- CPU fallback jobs (Linux + Windows): fmt, clippy, tests, bootstrap smoke, perf regression
- Optional GPU jobs (`ENABLE_GPU_CI=true`): Linux Vulkan smoke/perf, Windows DX smoke/perf
- Optional Proton advisory (`ENABLE_PROTON_CI=true`): Windows-target DX build + advisory Wine smoke on Linux
