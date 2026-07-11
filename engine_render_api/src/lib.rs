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

impl ViewportReadback {
    /// Returns the byte size of a `width x height` RGBA8 viewport buffer, or
    /// `None` if the dimensions would overflow `usize`.
    ///
    /// Backends use this helper to avoid panicking on maliciously large
    /// viewport configurations during lazy readback synthesis.
    pub fn buffer_size(width: u32, height: u32) -> Option<usize> {
        (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)
    }
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
pub struct ShaderBinding {
    pub set: u32,
    pub binding: u32,
    pub resource: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub shader_asset: String,
    pub shader_entry: String,
    pub shader_profile: String,
    pub bindings: Vec<ShaderBinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComputeDispatchNode {
    pub label: String,
    pub material: Material,
    pub dispatch: [u32; 3],
    pub reads: Vec<String>,
    pub writes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderPassNode {
    pub label: String,
    pub camera: Camera2d,
    pub target: Option<RenderTargetHandle>,
    pub material: Option<Material>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrameCaptureFormat {
    /// Standard RGBA8 uncompressed image data.
    Rgba8,
    /// Backend-specific raw payload (e.g., D3D11 staging texture bytes).
    Raw,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameCaptureRequest {
    pub label: String,
    pub format: FrameCaptureFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameCaptureHandle(pub u64);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameCaptureResult {
    pub handle: FrameCaptureHandle,
    pub label: String,
    pub completed: bool,
    pub data: Option<Vec<u8>>,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuInstrumentationConfig {
    /// Master switch for GPU timing queries.
    pub enabled: bool,
    /// Number of frames between timestamp query resets. Zero means "never reset".
    pub timestamp_query_period: u32,
}

/// Shared instrumentation state that backends can embed to implement the
/// [`GraphicsBackend`] GPU-timer and frame-capture hooks consistently.
#[derive(Debug, Clone, Default)]
pub struct BackendInstrumentationState {
    /// Whether GPU timestamp queries are currently enabled.
    pub gpu_timestamps_enabled: bool,
    /// Active instrumentation configuration.
    pub config: GpuInstrumentationConfig,
    /// Monotonic handle generator for frame-capture requests.
    pub next_capture_handle: u64,
    /// Captures that have been requested but not yet completed.
    pub pending_captures: Vec<(FrameCaptureHandle, FrameCaptureRequest)>,
    /// Captures that have finished and are waiting to be polled.
    pub completed_captures: Vec<FrameCaptureResult>,
}

impl BackendInstrumentationState {
    pub fn set_gpu_timestamps_enabled(&mut self, enabled: bool) {
        self.gpu_timestamps_enabled = enabled;
    }

    pub fn configure(&mut self, config: GpuInstrumentationConfig) {
        self.config = config;
        self.gpu_timestamps_enabled = config.enabled;
    }

    pub fn request_capture(
        &mut self,
        request: FrameCaptureRequest,
    ) -> FrameCaptureHandle {
        let handle = FrameCaptureHandle(self.next_capture_handle);
        self.next_capture_handle += 1;
        self.pending_captures.push((handle, request));
        handle
    }

    /// Marks the oldest pending capture as completed with the supplied payload.
    /// If no capture is pending, the payload is discarded.
    pub fn complete_oldest_capture(
        &mut self,
        width: u32,
        height: u32,
        data: Vec<u8>,
    ) {
        if let Some((handle, request)) = self.pending_captures.pop() {
            self.completed_captures.push(FrameCaptureResult {
                handle,
                label: request.label,
                completed: true,
                data: Some(data),
                width,
                height,
            });
        }
    }

    /// Polls a capture handle. Returns the completed result if available, or an
    /// in-flight marker otherwise.
    pub fn poll_capture(
        &mut self,
        handle: FrameCaptureHandle,
    ) -> Option<FrameCaptureResult> {
        if let Some(index) = self
            .completed_captures
            .iter()
            .position(|result| result.handle == handle)
        {
            return Some(self.completed_captures.remove(index));
        }

        if self.pending_captures.iter().any(|(h, _)| *h == handle) {
            return Some(FrameCaptureResult {
                handle,
                label: String::new(),
                completed: false,
                data: None,
                width: 0,
                height: 0,
            });
        }

        None
    }
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
    /// True when the backend has GPU timestamp queries enabled.
    pub gpu_timestamps_enabled: bool,
    /// Number of frame-capture requests that have not yet completed.
    pub frame_captures_pending: u64,
    /// Number of frame-capture requests that have completed.
    pub frame_captures_completed: u64,
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
            gpu_timestamps_enabled: false,
            frame_captures_pending: 0,
            frame_captures_completed: 0,
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

    /// Optionally preload a shader bytecode blob that the backend can use for
    /// subsequent render/compute passes. Backends that do not support runtime
    /// shader loading can ignore this. The default implementation is a no-op.
    fn preload_shader_bytecode(
        &mut self,
        _name: &str,
        _entry_point: &str,
        _bytecode: &[u8],
    ) -> Result<(), BackendError> {
        Ok(())
    }

    fn destroy(&mut self) -> Result<(), BackendError>;

    // GPU instrumentation and frame capture hooks. Backends may override these
    // to expose hardware timers, timestamp queries, and frame-capture payloads.
    // The default implementations are no-ops that report "not supported" so
    // callers can use them without branching on backend kind.

    /// Enables or disables GPU timestamp/instrumentation collection.
    fn set_gpu_timestamps_enabled(&mut self, _enabled: bool) {}

    /// Configures GPU instrumentation parameters.
    fn configure_gpu_instrumentation(&mut self, _config: GpuInstrumentationConfig) {}

    /// Requests a frame capture. The returned handle can be polled with
    /// [`GraphicsBackend::poll_frame_capture`].
    fn request_frame_capture(
        &mut self,
        _request: FrameCaptureRequest,
    ) -> Result<FrameCaptureHandle, BackendError> {
        Err(BackendError::Runtime(
            "frame capture not supported by this backend".into(),
        ))
    }

    /// Polls a previously requested frame capture. Returns `completed: false`
    /// while the capture is still in flight.
    fn poll_frame_capture(
        &mut self,
        _handle: FrameCaptureHandle,
    ) -> Result<FrameCaptureResult, BackendError> {
        Err(BackendError::Runtime(
            "frame capture not supported by this backend".into(),
        ))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_error_recoverable_surface() {
        let err = BackendError::SurfaceOutOfDate("out of date".into());
        assert!(err.is_recoverable_surface());
        assert!(!err.is_recoverable_device());
    }

    #[test]
    fn backend_error_recoverable_device() {
        let lost = BackendError::DeviceLost("lost".into());
        let removed = BackendError::DeviceRemoved("removed".into());
        assert!(lost.is_recoverable_device());
        assert!(removed.is_recoverable_device());
        assert!(!lost.is_recoverable_surface());
    }

    #[test]
    fn backend_diagnostics_event_ring_buffer() {
        let mut diag = BackendDiagnostics::new(BackendKind::Vulkan);
        for i in 0..300 {
            diag.push_event(BackendDiagnosticEvent {
                level: BackendDiagnosticLevel::Info,
                frame: Some(i as u64),
                pass: None,
                message: format!("event {i}"),
            });
        }
        assert_eq!(diag.events.len(), 256);
        assert_eq!(diag.events.first().unwrap().frame, Some(44));
        assert_eq!(diag.events.last().unwrap().frame, Some(299));
    }

    #[test]
    fn backend_diagnostics_pass_timing_ring_buffer() {
        let mut diag = BackendDiagnostics::new(BackendKind::Dx12);
        for i in 0..600 {
            diag.push_pass_timing(BackendPassTiming {
                frame: i as u64,
                pass: "pass".into(),
                cpu_ms: 1.0,
                gpu_ms: None,
            });
        }
        assert_eq!(diag.pass_timings.len(), 512);
        assert_eq!(diag.pass_timings.first().unwrap().frame, 88);
        assert_eq!(diag.pass_timings.last().unwrap().frame, 599);
    }

    #[test]
    fn backend_capabilities_supports_required_2d() {
        let caps = BackendCapabilities {
            textured_sprites: true,
            batching: true,
            camera_transforms: true,
            blend_modes: true,
            offscreen_targets: true,
            texture_atlas: true,
            gpu_nodes: false,
            hybrid_nodes: false,
            compute_nodes: false,
            viewport_readback: false,
        };
        assert!(caps.supports_required_2d());

        let mut missing = caps;
        missing.batching = false;
        assert!(!missing.supports_required_2d());
    }

    #[test]
    fn surface_config_from_engine_config_preserves_window_and_vsync() {
        let config = EngineConfig::default();
        let surface = SurfaceConfig::from_engine_config(&config);
        assert_eq!(surface.width, config.window.width);
        assert_eq!(surface.height, config.window.height);
        assert_eq!(surface.vsync, config.vsync);
        assert!(!surface.headless);
    }

    #[test]
    fn viewport_buffer_size_checked() {
        assert_eq!(ViewportReadback::buffer_size(2, 3), Some(24));
        assert_eq!(ViewportReadback::buffer_size(0, 100), Some(0));
        assert!(ViewportReadback::buffer_size(u32::MAX, u32::MAX).is_none());
    }

    #[test]
    fn render_graph_empty_is_empty() {
        let graph = RenderGraph::empty();
        assert!(graph.resources.is_empty());
        assert!(graph.passes.is_empty());
    }
}
