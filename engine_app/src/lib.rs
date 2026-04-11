use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use backend_dx11::Dx11Backend;
use backend_dx12::Dx12Backend;
use backend_vulkan::VulkanBackend;
use bevy_ecs::prelude::*;
use engine_assets::{
    load_node_graph, AssetBuildCache, AssetChange, AssetError, AssetHotReload, AssetKind,
    ShaderCompileOptions, ShaderSourceKind, ShaderTarget,
};
use engine_audio::{audio_sync_system, AudioRuntime, AudioState};
use engine_core::{load_config_from_ron, BackendPreference, EngineConfig, EngineCoreError};
use engine_editor::{draw_overlay, EditorState, FrameTimings};
use engine_nodes::{
    compile_graph, CompileDiagnostic, CompiledGraphArtifact, EcsJobDescriptor, NodeCompileError,
    NodeCompileOptions, NodeGraph,
};
use engine_physics::{physics_sync_system, PhysicsWorld};
use engine_platform::{
    available_backends_for_platform, choose_backend, PlatformError, RuntimePlatform,
};
use engine_render_api::{
    BackendCapabilities, BackendDiagnosticEvent, BackendDiagnosticLevel, BackendDiagnostics,
    BackendError, BackendKind, GraphicsBackend, RenderGraph, RenderGraphPass, SurfaceConfig,
    SurfaceHandle, SurfaceWindowHandles,
};
use thiserror::Error;
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::Window;
use winit::window::WindowAttributes;

#[derive(Debug, Clone, PartialEq)]
pub struct ViewportFrame {
    pub frame_index: u64,
    pub width: u32,
    pub height: u32,
    pub texture_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeDiagnosticsSnapshot {
    pub compile_diagnostics: Vec<CompileDiagnostic>,
    pub backend_diagnostics: BackendDiagnostics,
    pub frame_timings: FrameTimings,
    pub telemetry: FallbackTelemetry,
    pub active_backend: BackendKind,
}

#[derive(Resource, Default)]
struct GraphRuntimeState {
    jobs: Vec<EcsJobDescriptor>,
    executed_frames: u64,
    executed_jobs: u64,
    executed_passes: u64,
}

#[derive(Resource, Default)]
struct FrameCounter(pub u64);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FallbackTelemetry {
    pub compile_fallback_events: u64,
    pub compile_failures: u64,
    pub shader_rebuild_errors: u64,
    pub recovery_events: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HotReloadReport {
    pub changed_assets: usize,
    pub shaders_rebuilt: usize,
    pub scene_recompiled: bool,
    pub had_errors: bool,
}

fn frame_counter_system(mut counter: ResMut<FrameCounter>) {
    counter.0 += 1;
}

fn graph_runtime_system(mut state: ResMut<GraphRuntimeState>) {
    state.executed_frames += 1;
    state.executed_jobs += state.jobs.len() as u64;
}

#[derive(Debug, Error)]
pub enum EngineAppError {
    #[error(transparent)]
    Backend(#[from] BackendError),

    #[error(transparent)]
    Platform(#[from] PlatformError),

    #[error(transparent)]
    Asset(#[from] AssetError),

    #[error(transparent)]
    NodeCompile(#[from] NodeCompileError),

    #[error(transparent)]
    Core(#[from] EngineCoreError),

    #[error("audio runtime init failed: {0}")]
    Audio(String),

    #[error("no scene loaded")]
    NoSceneLoaded,

    #[error("backend recovery failed after {0} attempts")]
    RecoveryAttemptsExceeded(u32),
}

pub struct EngineApp {
    config: EngineConfig,
    backend_override: Option<BackendPreference>,
    platform: RuntimePlatform,
    active_backend: BackendKind,
    backend: Box<dyn GraphicsBackend>,
    surface: SurfaceHandle,

    fixed_schedule: Schedule,
    gameplay_schedule: Schedule,
    audio_schedule: Schedule,
    world: World,

    assets: AssetHotReload,
    build_cache: AssetBuildCache,
    scene_path: Option<PathBuf>,
    scene_graph: Option<NodeGraph>,
    compiled_graph: Option<CompiledGraphArtifact>,

    editor_state: EditorState,
    frame_timings: FrameTimings,
    backend_diagnostics: BackendDiagnostics,
    egui_context: egui::Context,
    surface_window: Option<SurfaceWindowHandles>,

    last_frame_instant: Instant,
    fixed_step_accumulator: f32,
    recovery_attempts: u32,
    telemetry: FallbackTelemetry,
    is_play_mode: bool,

    #[allow(dead_code)]
    audio_runtime: AudioRuntime,
}

impl EngineApp {
    pub fn from_config_path(path: impl AsRef<Path>) -> Result<Self, EngineAppError> {
        let config = load_config_from_ron(path)?;
        Self::new(config)
    }

    pub fn new(config: EngineConfig) -> Result<Self, EngineAppError> {
        let platform = RuntimePlatform::current();
        let available = available_backends_for_platform(platform);
        let active_backend = choose_backend(config.backend_preference, &available, platform)?;

        let mut backend = create_backend(active_backend);
        backend.initialize(&config)?;
        let surface = backend.create_surface(SurfaceConfig::from_engine_config(&config), None)?;

        let audio_runtime = match AudioRuntime::new() {
            Ok(runtime) => runtime,
            Err(err) => {
                tracing::warn!("falling back to silent audio runtime: {err}");
                AudioRuntime::silent()
            }
        };

        let mut world = World::new();
        world.insert_resource(PhysicsWorld::default());
        world.insert_resource(AudioState::default());
        world.insert_resource(GraphRuntimeState::default());
        world.insert_resource(FrameCounter::default());

        let mut fixed_schedule = Schedule::default();
        fixed_schedule.add_systems(physics_sync_system);

        let mut gameplay_schedule = Schedule::default();
        gameplay_schedule.add_systems((graph_runtime_system, frame_counter_system));

        let mut audio_schedule = Schedule::default();
        audio_schedule.add_systems(audio_sync_system);

        let backend_diagnostics = backend.diagnostics();

        Ok(Self {
            config,
            backend_override: None,
            platform,
            active_backend,
            backend,
            surface,
            fixed_schedule,
            gameplay_schedule,
            audio_schedule,
            world,
            assets: AssetHotReload::new(),
            build_cache: AssetBuildCache::new(),
            scene_path: None,
            scene_graph: None,
            compiled_graph: None,
            editor_state: EditorState::default(),
            frame_timings: FrameTimings::default(),
            backend_diagnostics,
            egui_context: egui::Context::default(),
            surface_window: None,
            last_frame_instant: Instant::now(),
            fixed_step_accumulator: 0.0,
            recovery_attempts: 0,
            telemetry: FallbackTelemetry::default(),
            is_play_mode: false,
            audio_runtime,
        })
    }

    pub fn start_play_mode(&mut self) {
        self.is_play_mode = true;
        self.editor_state.is_playing = true;
    }

    pub fn stop_play_mode(&mut self) {
        self.is_play_mode = false;
        self.editor_state.is_playing = false;
    }

    pub fn is_play_mode(&self) -> bool {
        self.is_play_mode
    }

    pub fn step_play_frame(&mut self) -> Result<(), EngineAppError> {
        let was_playing = self.is_play_mode;
        self.is_play_mode = true;
        let result = self.run_for_frames(1);
        self.is_play_mode = was_playing;
        self.editor_state.is_playing = self.is_play_mode;
        result
    }

    pub fn set_active_scene_graph(&mut self, graph: NodeGraph) -> Result<bool, EngineAppError> {
        self.compile_and_apply_graph(graph)
    }

    pub fn viewport_frame(&self) -> ViewportFrame {
        let frame_index = self
            .world
            .get_resource::<FrameCounter>()
            .map(|counter| counter.0)
            .unwrap_or(0);

        ViewportFrame {
            frame_index,
            width: self.config.window.width,
            height: self.config.window.height,
            texture_id: None,
        }
    }

    pub fn diagnostics_snapshot(&self) -> RuntimeDiagnosticsSnapshot {
        let compile_diagnostics = self
            .compiled_graph
            .as_ref()
            .map(|artifact| artifact.diagnostics.clone())
            .unwrap_or_default();

        RuntimeDiagnosticsSnapshot {
            compile_diagnostics,
            backend_diagnostics: self.backend_diagnostics.clone(),
            frame_timings: self.frame_timings.clone(),
            telemetry: self.telemetry.clone(),
            active_backend: self.active_backend,
        }
    }

    pub fn attach_window(&mut self, window: &Window) -> Result<(), EngineAppError> {
        let window_handle = window
            .window_handle()
            .map_err(|err| BackendError::Surface(format!("window handle error: {err}")))?;
        let display_handle = window
            .display_handle()
            .map_err(|err| BackendError::Surface(format!("display handle error: {err}")))?;

        let handles = SurfaceWindowHandles {
            window_handle: window_handle.as_raw(),
            display_handle: display_handle.as_raw(),
        };

        let config = SurfaceConfig::from_engine_config(&self.config);
        let surface = self.backend.create_surface(config, Some(handles))?;
        self.surface = surface;
        self.surface_window = Some(handles);

        Ok(())
    }

    pub fn load_scene(&mut self, path: impl AsRef<Path>) -> Result<(), EngineAppError> {
        let scene_path = path.as_ref().to_path_buf();
        let graph = load_node_graph(&scene_path)?;
        let compiled = self.compile_and_apply_graph(graph)?;
        if !compiled {
            self.push_backend_event(
                BackendDiagnosticLevel::Warning,
                None,
                None,
                format!(
                    "scene {} failed to compile; keeping previous valid runtime graph",
                    scene_path.display()
                ),
            );
        }
        self.scene_path = Some(scene_path);
        Ok(())
    }

    pub fn set_backend_override(
        &mut self,
        backend_preference: BackendPreference,
    ) -> Result<(), EngineAppError> {
        let effective_preference = if backend_preference == BackendPreference::Auto {
            self.config.backend_preference
        } else {
            self.backend_override = Some(backend_preference);
            backend_preference
        };

        let available = available_backends_for_platform(self.platform);
        let selected = choose_backend(effective_preference, &available, self.platform)?;

        if selected != self.active_backend {
            self.backend.destroy()?;
            let mut backend = create_backend(selected);
            backend.initialize(&self.config)?;
            let surface = backend.create_surface(
                SurfaceConfig::from_engine_config(&self.config),
                self.surface_window,
            )?;

            self.backend = backend;
            self.active_backend = selected;
            self.surface = surface;

            if let Some(scene_graph) = self.scene_graph.clone() {
                let _ = self.compile_and_apply_graph(scene_graph)?;
            }
        }

        Ok(())
    }

    pub fn resize_surface(&mut self, width: u32, height: u32) -> Result<(), EngineAppError> {
        self.backend.resize(self.surface, width, height)?;
        Ok(())
    }

    pub fn run(&mut self) -> Result<(), EngineAppError> {
        self.run_for_frames(1)
    }

    pub fn run_for_frames(&mut self, frame_count: u32) -> Result<(), EngineAppError> {
        for _ in 0..frame_count {
            if let Err(err) = self.run_single_frame() {
                if self.try_recover(&err)? {
                    continue;
                }
                return Err(err);
            }
        }

        Ok(())
    }

    fn run_single_frame(&mut self) -> Result<(), EngineAppError> {
        let frame_start = Instant::now();
        let delta = self.consume_delta_time();

        self.run_input_phase();
        self.run_fixed_update_phase(delta);
        self.run_gameplay_phase();
        self.run_audio_phase();
        self.run_render_phase()?;

        self.frame_timings.cpu_frame_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
        self.backend_diagnostics = self.backend.diagnostics();
        self.frame_timings.gpu_frame_ms = self.backend_diagnostics.last_gpu_frame_ms;

        self.draw_editor_overlay();
        self.apply_frame_pacing(frame_start);

        Ok(())
    }

    fn consume_delta_time(&mut self) -> f32 {
        let now = Instant::now();
        let delta = now
            .saturating_duration_since(self.last_frame_instant)
            .as_secs_f32();
        self.last_frame_instant = now;
        delta
    }

    fn run_input_phase(&mut self) {
        // Placeholder for input collection; deterministic ordering starts here.
    }

    fn run_fixed_update_phase(&mut self, delta_seconds: f32) {
        let fixed = &self.config.fixed_step;
        let fixed_dt = 1.0_f32 / fixed.hz.max(1.0);
        self.fixed_step_accumulator =
            (self.fixed_step_accumulator + delta_seconds).min(fixed.max_catch_up_seconds);

        let mut executed_steps = 0_u32;
        while self.fixed_step_accumulator >= fixed_dt && executed_steps < fixed.max_steps_per_frame
        {
            self.fixed_schedule.run(&mut self.world);
            self.fixed_step_accumulator -= fixed_dt;
            executed_steps += 1;
        }

        if self.fixed_step_accumulator >= fixed_dt {
            self.fixed_step_accumulator = 0.0;
            self.push_backend_event(
                BackendDiagnosticLevel::Warning,
                None,
                None,
                "fixed-step clamp triggered; accumulator reset to prevent spiral-of-death",
            );
        }
    }

    fn run_gameplay_phase(&mut self) {
        if self.is_play_mode {
            self.gameplay_schedule.run(&mut self.world);
        }
    }

    fn run_audio_phase(&mut self) {
        self.audio_schedule.run(&mut self.world);
    }

    fn run_render_phase(&mut self) -> Result<(), EngineAppError> {
        let frame = self.backend.acquire_frame(self.surface)?;

        if let Some(artifact) = &self.compiled_graph {
            let submission_graph = optimize_submission_graph(&artifact.render_graph);
            self.backend.record_render_graph(frame, &submission_graph)?;

            if let Some(mut runtime_state) = self.world.get_resource_mut::<GraphRuntimeState>() {
                runtime_state.executed_passes += submission_graph.passes.len() as u64;
            }
        } else {
            self.backend
                .record_render_graph(frame, &RenderGraph::empty())?;
        }

        self.backend.submit(frame)?;
        self.backend.present(frame)?;
        Ok(())
    }

    fn apply_frame_pacing(&self, frame_start: Instant) {
        if !self.config.frame_pacing.sleep_enabled {
            return;
        }

        if self.config.frame_pacing.target_fps == 0 {
            return;
        }

        let target = Duration::from_secs_f32(1.0 / self.config.frame_pacing.target_fps as f32);
        let elapsed = frame_start.elapsed();
        if target > elapsed {
            thread::sleep(target - elapsed);
        }
    }

    fn try_recover(&mut self, err: &EngineAppError) -> Result<bool, EngineAppError> {
        let policy = self.config.recovery_policy;

        match err {
            EngineAppError::Backend(backend_err) if backend_err.is_recoverable_surface() => {
                if !policy.recover_surface_out_of_date {
                    return Ok(false);
                }
                self.recover_surface_out_of_date()?;
                Ok(true)
            }
            EngineAppError::Backend(backend_err) if backend_err.is_recoverable_device() => {
                if !policy.recover_device_loss {
                    return Ok(false);
                }
                self.recover_backend_loss()?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn recover_surface_out_of_date(&mut self) -> Result<(), EngineAppError> {
        self.recovery_attempts += 1;
        if self.recovery_attempts > self.config.recovery_policy.max_recovery_attempts {
            return Err(EngineAppError::RecoveryAttemptsExceeded(
                self.recovery_attempts,
            ));
        }

        let surface = self.backend.create_surface(
            SurfaceConfig::from_engine_config(&self.config),
            self.surface_window,
        )?;
        self.surface = surface;
        self.telemetry.recovery_events += 1;
        self.backend_diagnostics.mark_swapchain_recreate();
        self.push_backend_event(
            BackendDiagnosticLevel::Warning,
            None,
            None,
            "surface out-of-date recovered by recreating surface/swapchain",
        );

        Ok(())
    }

    fn recover_backend_loss(&mut self) -> Result<(), EngineAppError> {
        self.recovery_attempts += 1;
        if self.recovery_attempts > self.config.recovery_policy.max_recovery_attempts {
            return Err(EngineAppError::RecoveryAttemptsExceeded(
                self.recovery_attempts,
            ));
        }

        let mut backend = create_backend(self.active_backend);
        backend.initialize(&self.config)?;
        let surface = backend.create_surface(
            SurfaceConfig::from_engine_config(&self.config),
            self.surface_window,
        )?;
        self.backend = backend;
        self.surface = surface;

        if let Some(scene_graph) = self.scene_graph.clone() {
            let _ = self.compile_and_apply_graph(scene_graph)?;
        }

        self.telemetry.recovery_events += 1;
        self.backend_diagnostics.mark_device_loss();
        self.push_backend_event(
            BackendDiagnosticLevel::Error,
            None,
            None,
            "device loss recovered by backend reinitialization",
        );

        Ok(())
    }

    fn compile_and_apply_graph(&mut self, graph: NodeGraph) -> Result<bool, EngineAppError> {
        let compile_options = NodeCompileOptions {
            strict_gpu: self.config.shader_toolchain.strict,
            ..NodeCompileOptions::default()
        };

        let compile_start = Instant::now();
        let compile_result = compile_graph(
            &graph,
            &compile_options,
            self.active_backend,
            self.backend.capabilities(),
        );
        self.frame_timings.node_compile_ms = compile_start.elapsed().as_secs_f32() * 1000.0;

        let artifact = match compile_result {
            Ok(artifact) => artifact,
            Err(err) => {
                self.telemetry.compile_failures += 1;
                self.push_backend_event(
                    BackendDiagnosticLevel::Error,
                    None,
                    None,
                    format!("graph compile failed: {err}"),
                );
                if self.compiled_graph.is_some() {
                    return Ok(false);
                }
                return Err(EngineAppError::NodeCompile(err));
            }
        };

        let fallback_count = artifact.diagnostics.len() as u64;
        if fallback_count > 0 {
            self.telemetry.compile_fallback_events += fallback_count;
            for _ in 0..fallback_count {
                self.backend_diagnostics.mark_fallback();
            }
        }

        if let Some(mut runtime_state) = self.world.get_resource_mut::<GraphRuntimeState>() {
            runtime_state.jobs = artifact.ecs_jobs.clone();
        }

        self.scene_graph = Some(graph);
        self.compiled_graph = Some(artifact);
        self.editor_state
            .mark_graph_compiled(self.telemetry.compile_fallback_events as usize);

        Ok(true)
    }

    pub fn mark_graph_dirty(&mut self) {
        self.editor_state.mark_graph_dirty();
    }

    pub fn hot_recompile_if_needed(&mut self) -> Result<bool, EngineAppError> {
        if !self.editor_state.graph_dirty {
            return Ok(false);
        }

        let path = self
            .scene_path
            .clone()
            .ok_or(EngineAppError::NoSceneLoaded)?;
        let graph = load_node_graph(path)?;
        self.compile_and_apply_graph(graph)
    }

    pub fn poll_asset_changes(&mut self, root: impl AsRef<Path>) -> Result<usize, EngineAppError> {
        let changes = self.assets.scan_changes(root)?;
        Ok(changes.len())
    }

    pub fn apply_hot_reload(
        &mut self,
        root: impl AsRef<Path>,
    ) -> Result<HotReloadReport, EngineAppError> {
        let changes = self.assets.scan_changes(root)?;
        let mut report = HotReloadReport {
            changed_assets: changes.len(),
            ..HotReloadReport::default()
        };

        for change in &changes {
            self.process_asset_change(change, &mut report)?;
        }

        Ok(report)
    }

    fn process_asset_change(
        &mut self,
        change: &AssetChange,
        report: &mut HotReloadReport,
    ) -> Result<(), EngineAppError> {
        if matches!(change.kind, AssetKind::Shader) {
            self.build_cache.invalidate(&change.path);
            let source_kind = shader_source_kind(&change.path).unwrap_or(ShaderSourceKind::Hlsl);
            let target = shader_target_for_backend(self.active_backend);
            let options = ShaderCompileOptions {
                toolchain: self.config.shader_toolchain.clone(),
                optimization: "O2".to_string(),
                include_dirs: vec![change.path.parent().unwrap_or(Path::new(".")).to_path_buf()],
            };

            match self.build_cache.build_or_reuse_shader(
                &change.path,
                source_kind,
                target,
                &options,
            ) {
                Ok((_artifact, _reused)) => {
                    report.shaders_rebuilt += 1;
                }
                Err(err) => {
                    report.had_errors = true;
                    self.telemetry.shader_rebuild_errors += 1;
                    self.push_backend_event(
                        BackendDiagnosticLevel::Error,
                        None,
                        None,
                        format!("shader rebuild failed for {}: {err}", change.path.display()),
                    );
                }
            }
        }

        if matches!(change.kind, AssetKind::Graph) {
            if let Some(scene_path) = &self.scene_path {
                if *scene_path == change.path {
                    let graph = load_node_graph(scene_path)?;
                    let recompiled = self.compile_and_apply_graph(graph)?;
                    report.scene_recompiled = recompiled;
                }
            }
        }

        Ok(())
    }

    fn draw_editor_overlay(&mut self) {
        let diagnostics = self
            .compiled_graph
            .as_ref()
            .map(|artifact| artifact.diagnostics.as_slice())
            .unwrap_or(&[]);

        let raw_input = egui::RawInput::default();
        let active_backend = self.active_backend;
        let capabilities = self.backend.capabilities();
        let frame_timings = self.frame_timings.clone();
        let backend_diagnostics = self.backend_diagnostics.clone();

        let _ = self.egui_context.run(raw_input, |ctx| {
            draw_overlay(
                ctx,
                &mut self.editor_state,
                &frame_timings,
                active_backend,
                capabilities,
                diagnostics,
                &backend_diagnostics,
            );
        });
    }

    fn push_backend_event(
        &mut self,
        level: BackendDiagnosticLevel,
        frame: Option<u64>,
        pass: Option<String>,
        message: impl Into<String>,
    ) {
        self.backend_diagnostics.push_event(BackendDiagnosticEvent {
            level,
            frame,
            pass,
            message: message.into(),
        });
    }

    pub fn window_attributes(&self) -> WindowAttributes {
        WindowAttributes::default()
            .with_title(self.config.window.title.clone())
            .with_inner_size(winit::dpi::PhysicalSize::new(
                self.config.window.width,
                self.config.window.height,
            ))
            .with_resizable(self.config.window.resizable)
    }

    pub fn active_backend(&self) -> BackendKind {
        self.active_backend
    }

    pub fn compiled_graph(&self) -> Option<&CompiledGraphArtifact> {
        self.compiled_graph.as_ref()
    }

    pub fn backend_diagnostics(&self) -> &BackendDiagnostics {
        &self.backend_diagnostics
    }

    pub fn backend_capabilities(&self) -> BackendCapabilities {
        self.backend.capabilities()
    }

    pub fn editor_state(&self) -> &EditorState {
        &self.editor_state
    }

    pub fn telemetry(&self) -> &FallbackTelemetry {
        &self.telemetry
    }

    pub fn set_frame_pacing_sleep_enabled(&mut self, enabled: bool) {
        self.config.frame_pacing.sleep_enabled = enabled;
    }
}

fn create_backend(kind: BackendKind) -> Box<dyn GraphicsBackend> {
    match kind {
        BackendKind::Vulkan => Box::new(VulkanBackend::new()),
        BackendKind::Dx12 => Box::new(Dx12Backend::new()),
        BackendKind::Dx11 => Box::new(Dx11Backend::new()),
    }
}

fn optimize_submission_graph(graph: &RenderGraph) -> RenderGraph {
    let mut optimized = graph.clone();

    for pass in &mut optimized.passes {
        if let RenderGraphPass::Render(render) = pass {
            for batch in &mut render.batches {
                let blend_key = blend_sort_key(batch.blend);
                batch
                    .sprites
                    .sort_by_key(|sprite| (sprite.texture.0, blend_key));
            }
            render.batches.sort_by_key(|batch| {
                let texture = batch
                    .sprites
                    .first()
                    .map(|sprite| sprite.texture.0)
                    .unwrap_or(0);
                (texture, blend_sort_key(batch.blend))
            });
        }
    }

    optimized
}

fn blend_sort_key(blend: engine_render_api::BlendMode) -> u8 {
    match blend {
        engine_render_api::BlendMode::Alpha => 0,
        engine_render_api::BlendMode::Additive => 1,
        engine_render_api::BlendMode::Multiply => 2,
    }
}

fn shader_source_kind(path: &Path) -> Option<ShaderSourceKind> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    match ext.as_str() {
        "glsl" | "vert" | "frag" | "comp" => Some(ShaderSourceKind::Glsl),
        "hlsl" => Some(ShaderSourceKind::Hlsl),
        _ => None,
    }
}

fn shader_target_for_backend(backend: BackendKind) -> ShaderTarget {
    match backend {
        BackendKind::Vulkan => ShaderTarget::VulkanSpirv,
        BackendKind::Dx12 => ShaderTarget::Dx12Dxil,
        BackendKind::Dx11 => ShaderTarget::Dx11Dxbc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_assets::save_node_graph;
    use engine_nodes::{
        ComputeDispatchConfig, GpuResourceAccess, Node, NodeExecutionTarget, NodeFallbackPolicy,
        NodeGraph, NodeKind,
    };
    use std::collections::BTreeMap;

    fn sample_graph() -> NodeGraph {
        NodeGraph {
            version: engine_nodes::CURRENT_GRAPH_VERSION,
            nodes: vec![
                Node {
                    id: 1,
                    name: "start".to_string(),
                    kind: NodeKind::GameplayEvent,
                    target: NodeExecutionTarget::Cpu,
                    dependencies: vec![],
                    settings: BTreeMap::new(),
                    gpu_bindings: vec![],
                    compute: None,
                    fallback_policy: NodeFallbackPolicy::Cpu,
                    gpu_resource_states: vec![],
                    shader_entry: None,
                    shader_profile: None,
                },
                Node {
                    id: 2,
                    name: "compute_particles".to_string(),
                    kind: NodeKind::ComputePass,
                    target: NodeExecutionTarget::Hybrid,
                    dependencies: vec![1],
                    settings: BTreeMap::from([
                        ("shader".to_string(), "particles.hlsl".to_string()),
                        (
                            "write_resources".to_string(),
                            "particles_buffer".to_string(),
                        ),
                    ]),
                    gpu_bindings: vec![],
                    compute: Some(ComputeDispatchConfig { x: 4, y: 4, z: 1 }),
                    fallback_policy: NodeFallbackPolicy::Cpu,
                    gpu_resource_states: vec![engine_nodes::NodeGpuResourceState {
                        resource: "particles_buffer".to_string(),
                        access: GpuResourceAccess::Write,
                    }],
                    shader_entry: Some("cs_main".to_string()),
                    shader_profile: Some("cs_6_6".to_string()),
                },
                Node {
                    id: 3,
                    name: "render".to_string(),
                    kind: NodeKind::RenderPass,
                    target: NodeExecutionTarget::Gpu,
                    dependencies: vec![2],
                    settings: BTreeMap::from([
                        ("sprite_count".to_string(), "4".to_string()),
                        ("blend".to_string(), "alpha".to_string()),
                    ]),
                    gpu_bindings: vec![],
                    compute: None,
                    fallback_policy: NodeFallbackPolicy::Cpu,
                    gpu_resource_states: vec![],
                    shader_entry: Some("vs_main".to_string()),
                    shader_profile: Some("ps_6_0".to_string()),
                },
            ],
        }
    }

    #[test]
    fn backend_contract_has_required_features() {
        let backends: Vec<Box<dyn GraphicsBackend>> = vec![
            Box::new(VulkanBackend::new()),
            Box::new(Dx12Backend::new()),
            Box::new(Dx11Backend::new()),
        ];

        for backend in backends {
            assert!(
                backend.capabilities().supports_required_2d(),
                "backend {:?} must satisfy required 2D features",
                backend.kind(),
            );
        }
    }

    #[test]
    fn linux_platform_selects_vulkan_by_default() {
        let selected = choose_backend(
            BackendPreference::Auto,
            &[BackendKind::Dx11, BackendKind::Vulkan],
            RuntimePlatform::Linux,
        )
        .expect("backend should be selected");
        assert_eq!(selected, BackendKind::Vulkan);
    }

    #[test]
    fn hot_recompile_updates_compiled_artifact() {
        let temp_dir = std::env::temp_dir().join("rusty_engine_app_test");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let scene_path = temp_dir.join("scene.ron");

        let mut app = EngineApp::new(EngineConfig::default()).expect("app should initialize");

        let graph_a = sample_graph();
        save_node_graph(&scene_path, &graph_a).expect("graph should save");
        app.load_scene(&scene_path).expect("scene should load");
        let first_pass_count = app
            .compiled_graph()
            .expect("compiled graph should exist")
            .render_graph
            .passes
            .len();

        let mut graph_b = sample_graph();
        graph_b.nodes.push(Node {
            id: 4,
            name: "build".to_string(),
            kind: NodeKind::BuildExport,
            target: NodeExecutionTarget::Hybrid,
            dependencies: vec![3],
            settings: BTreeMap::new(),
            gpu_bindings: vec![],
            compute: None,
            fallback_policy: NodeFallbackPolicy::Cpu,
            gpu_resource_states: vec![],
            shader_entry: None,
            shader_profile: None,
        });

        save_node_graph(&scene_path, &graph_b).expect("graph should save");
        app.mark_graph_dirty();
        let recompiled = app
            .hot_recompile_if_needed()
            .expect("hot recompile should run");

        assert!(recompiled);
        let second_pass_count = app
            .compiled_graph()
            .expect("compiled graph should exist")
            .render_graph
            .passes
            .len();
        assert!(second_pass_count > first_pass_count);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn run_updates_backend_diagnostics() {
        let mut app = EngineApp::new(EngineConfig::default()).expect("app should initialize");
        app.run_for_frames(1).expect("frame should run");
        assert!(
            !app.backend_diagnostics().events.is_empty()
                || app.backend_diagnostics().last_cpu_frame_ms >= 0.0
        );
    }

    #[test]
    fn apply_hot_reload_reports_changes() {
        let temp_dir = std::env::temp_dir().join("rusty_engine_hot_reload");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");

        let shader_path = temp_dir.join("test.hlsl");
        std::fs::write(&shader_path, "//@entry main\n").expect("shader should write");

        let mut app = EngineApp::new(EngineConfig::default()).expect("app should initialize");
        let report = app
            .apply_hot_reload(&temp_dir)
            .expect("hot reload should succeed");

        assert!(report.changed_assets >= 1);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
