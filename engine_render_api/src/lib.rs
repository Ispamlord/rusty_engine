use engine_core::EngineConfig;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BackendKind {
    Vulkan,
    Dx12,
    Dx11,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlendMode {
    Alpha,
    Additive,
    Multiply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TextureHandle(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RenderTargetHandle(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SurfaceHandle(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameToken(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewportReadback {
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureDescriptor {
    pub width: u32,
    pub height: u32,
    pub mip_levels: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderTargetDescriptor {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceConfig {
    pub width: u32,
    pub height: u32,
    pub vsync: bool,
    pub headless: bool,
}

impl SurfaceConfig {
    pub fn from_engine_config(config: &EngineConfig) -> Self {
        Self {
            width: config.window.width,
            height: config.window.height,
            vsync: config.vsync,
            headless: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SurfaceWindowHandles {
    pub window_handle: RawWindowHandle,
    pub display_handle: RawDisplayHandle,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Camera2d {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

impl Default for Camera2d {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SpriteInstance {
    pub texture: TextureHandle,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub rotation_radians: f32,
    pub tint: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphResourceLifetime {
    Transient,
    Persistent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphResourceKind {
    Texture(TextureDescriptor),
    RenderTarget(RenderTargetDescriptor),
    StorageBuffer { size_bytes: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphResourceDescriptor {
    pub name: String,
    pub kind: GraphResourceKind,
    pub lifetime: GraphResourceLifetime,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpriteBatchCommand {
    pub label: String,
    pub blend: BlendMode,
    pub target: Option<RenderTargetHandle>,
    pub sprites: Vec<SpriteInstance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputeDispatchNode {
    pub label: String,
    pub shader: String,
    pub dispatch: [u32; 3],
    pub reads: Vec<String>,
    pub writes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderPassNode {
    pub label: String,
    pub camera: Camera2d,
    pub target: Option<RenderTargetHandle>,
    pub batches: Vec<SpriteBatchCommand>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RenderGraphPass {
    Render(RenderPassNode),
    Compute(ComputeDispatchNode),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderGraph {
    pub resources: Vec<GraphResourceDescriptor>,
    pub passes: Vec<RenderGraphPass>,
}

impl RenderGraph {
    pub fn empty() -> Self {
        Self {
            resources: Vec::new(),
            passes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackendDiagnosticLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendDiagnosticEvent {
    pub level: BackendDiagnosticLevel,
    pub frame: Option<u64>,
    pub pass: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendPassTiming {
    pub frame: u64,
    pub pass: String,
    pub cpu_ms: f32,
    pub gpu_ms: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendDiagnostics {
    pub backend: BackendKind,
    pub supports_surface: bool,
    pub last_cpu_frame_ms: f32,
    pub last_gpu_frame_ms: f32,
    pub fallback_events: u64,
    pub swapchain_recreates: u64,
    pub device_loss_events: u64,
    pub events: Vec<BackendDiagnosticEvent>,
    pub pass_timings: Vec<BackendPassTiming>,
}

impl BackendDiagnostics {
    pub fn new(backend: BackendKind) -> Self {
        Self {
            backend,
            supports_surface: false,
            last_cpu_frame_ms: 0.0,
            last_gpu_frame_ms: 0.0,
            fallback_events: 0,
            swapchain_recreates: 0,
            device_loss_events: 0,
            events: Vec::new(),
            pass_timings: Vec::new(),
        }
    }

    pub fn push_event(&mut self, event: BackendDiagnosticEvent) {
        self.events.push(event);
        if self.events.len() > 256 {
            let drain_until = self.events.len() - 256;
            self.events.drain(0..drain_until);
        }
    }

    pub fn push_pass_timing(&mut self, timing: BackendPassTiming) {
        self.pass_timings.push(timing);
        if self.pass_timings.len() > 512 {
            let drain_until = self.pass_timings.len() - 512;
            self.pass_timings.drain(0..drain_until);
        }
    }

    pub fn mark_fallback(&mut self) {
        self.fallback_events += 1;
    }

    pub fn mark_swapchain_recreate(&mut self) {
        self.swapchain_recreates += 1;
    }

    pub fn mark_device_loss(&mut self) {
        self.device_loss_events += 1;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendCapabilities {
    pub textured_sprites: bool,
    pub batching: bool,
    pub camera_transforms: bool,
    pub blend_modes: bool,
    pub offscreen_targets: bool,
    pub texture_atlas: bool,
    pub gpu_nodes: bool,
    pub hybrid_nodes: bool,
    pub compute_nodes: bool,
    pub viewport_readback: bool,
}

impl BackendCapabilities {
    pub fn supports_required_2d(&self) -> bool {
        self.textured_sprites
            && self.batching
            && self.camera_transforms
            && self.blend_modes
            && self.offscreen_targets
            && self.texture_atlas
    }
}

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("backend {0:?} is unavailable on this platform")]
    Unavailable(BackendKind),

    #[error("backend initialization failed: {0}")]
    Init(String),

    #[error("surface error: {0}")]
    Surface(String),

    #[error("surface out of date: {0}")]
    SurfaceOutOfDate(String),

    #[error("render command rejected: {0}")]
    Command(String),

    #[error("device lost: {0}")]
    DeviceLost(String),

    #[error("device removed: {0}")]
    DeviceRemoved(String),

    #[error("runtime backend failure: {0}")]
    Runtime(String),
}

impl BackendError {
    pub fn is_recoverable_surface(&self) -> bool {
        matches!(self, Self::SurfaceOutOfDate(_))
    }

    pub fn is_recoverable_device(&self) -> bool {
        matches!(self, Self::DeviceLost(_) | Self::DeviceRemoved(_))
    }
}

pub trait GraphicsBackend {
    fn kind(&self) -> BackendKind;

    fn initialize(&mut self, config: &EngineConfig) -> Result<(), BackendError>;

    fn create_surface(
        &mut self,
        config: SurfaceConfig,
        window: Option<SurfaceWindowHandles>,
    ) -> Result<SurfaceHandle, BackendError>;

    fn resize(
        &mut self,
        surface: SurfaceHandle,
        width: u32,
        height: u32,
    ) -> Result<(), BackendError>;

    fn capabilities(&self) -> BackendCapabilities;

    fn diagnostics(&self) -> BackendDiagnostics;

    fn create_texture(
        &mut self,
        descriptor: TextureDescriptor,
    ) -> Result<TextureHandle, BackendError>;

    fn create_render_target(
        &mut self,
        descriptor: RenderTargetDescriptor,
    ) -> Result<RenderTargetHandle, BackendError>;

    fn acquire_frame(&mut self, surface: SurfaceHandle) -> Result<FrameToken, BackendError>;

    fn record_render_graph(
        &mut self,
        frame: FrameToken,
        graph: &RenderGraph,
    ) -> Result<(), BackendError>;

    fn submit(&mut self, frame: FrameToken) -> Result<(), BackendError>;

    fn present(&mut self, frame: FrameToken) -> Result<(), BackendError>;

    fn readback_viewport(&mut self) -> Result<Option<ViewportReadback>, BackendError>;

    fn destroy(&mut self) -> Result<(), BackendError>;
}

pub fn required_backend_features() -> [&'static str; 6] {
    [
        "textured_sprites",
        "batching",
        "camera_transforms",
        "blend_modes",
        "offscreen_targets",
        "texture_atlas",
    ]
}
