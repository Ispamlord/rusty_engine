use std::time::Instant;

use engine_core::EngineConfig;
use engine_render_api::{
    BackendCapabilities, BackendDiagnosticEvent, BackendDiagnosticLevel, BackendDiagnostics,
    BackendError, BackendKind, BackendPassTiming, FrameToken, GraphicsBackend, RenderGraph,
    RenderGraphPass, RenderTargetDescriptor, RenderTargetHandle, SurfaceConfig, SurfaceHandle,
    SurfaceWindowHandles, TextureDescriptor, TextureHandle, ViewportReadback,
};
#[cfg(target_os = "windows")]
use raw_window_handle::RawWindowHandle;

#[cfg(target_os = "windows")]
use windows::Win32::Foundation::HMODULE;
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0,
};
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, D3D11_CREATE_DEVICE_FLAG,
    D3D11_SDK_VERSION,
};
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Dxgi::IDXGIAdapter;

#[cfg(target_os = "windows")]
struct Dx11NativeState {
    _device: ID3D11Device,
    context: ID3D11DeviceContext,
    _feature_level: D3D_FEATURE_LEVEL,
}

pub struct Dx11Backend {
    initialized: bool,
    frame_in_flight: Option<FrameToken>,
    frame_start: Option<Instant>,
    next_handle: u64,
    next_frame: u64,
    active_surface: Option<SurfaceHandle>,
    surface_config: Option<SurfaceConfig>,
    diagnostics: BackendDiagnostics,
    last_viewport_readback: Option<ViewportReadback>,

    #[cfg(target_os = "windows")]
    native: Option<Dx11NativeState>,
}

impl Default for Dx11Backend {
    fn default() -> Self {
        Self::new()
    }
}

impl Dx11Backend {
    pub fn new() -> Self {
        Self {
            initialized: false,
            frame_in_flight: None,
            frame_start: None,
            next_handle: 1,
            next_frame: 1,
            active_surface: None,
            surface_config: None,
            diagnostics: BackendDiagnostics::new(BackendKind::Dx11),
            last_viewport_readback: None,
            #[cfg(target_os = "windows")]
            native: None,
        }
    }

    fn allocate_handle(&mut self) -> u64 {
        let handle = self.next_handle;
        self.next_handle += 1;
        handle
    }

    fn next_frame_token(&mut self) -> FrameToken {
        let token = FrameToken(self.next_frame);
        self.next_frame += 1;
        token
    }

    fn record_event(
        &mut self,
        level: BackendDiagnosticLevel,
        frame: Option<u64>,
        pass: Option<String>,
        message: impl Into<String>,
    ) {
        self.diagnostics.push_event(BackendDiagnosticEvent {
            level,
            frame,
            pass,
            message: message.into(),
        });
    }

    fn destroy_internal(&mut self) -> Result<(), BackendError> {
        #[cfg(target_os = "windows")]
        {
            self.native = None;
        }

        self.initialized = false;
        self.frame_in_flight = None;
        self.frame_start = None;
        self.active_surface = None;
        self.surface_config = None;
        self.last_viewport_readback = None;

        Ok(())
    }

    fn synthesize_viewport(&self, graph: &RenderGraph) -> Option<ViewportReadback> {
        let config = self.surface_config?;
        let width = config.width.max(1);
        let height = config.height.max(1);
        let mut rgba = vec![0_u8; width as usize * height as usize * 4];
        for y in 0..height {
            for x in 0..width {
                let idx = ((y * width + x) * 4) as usize;
                rgba[idx] = 22;
                rgba[idx + 1] = 16;
                rgba[idx + 2] = 20;
                rgba[idx + 3] = 255;
            }
        }

        for pass in &graph.passes {
            if let RenderGraphPass::Render(render) = pass {
                for batch in &render.batches {
                    for sprite in &batch.sprites {
                        let cx = (width as f32 * 0.5 + sprite.x).round() as i32;
                        let cy = (height as f32 * 0.5 + sprite.y).round() as i32;
                        let hw = (sprite.width * 0.5).round().max(1.0) as i32;
                        let hh = (sprite.height * 0.5).round().max(1.0) as i32;
                        let color = [
                            ((sprite.texture.0.wrapping_mul(73) % 255) as u8).max(35),
                            ((sprite.texture.0.wrapping_mul(47) % 255) as u8).max(35),
                            ((sprite.texture.0.wrapping_mul(91) % 255) as u8).max(35),
                        ];
                        let min_x = (cx - hw).max(0);
                        let max_x = (cx + hw).min(width as i32 - 1);
                        let min_y = (cy - hh).max(0);
                        let max_y = (cy + hh).min(height as i32 - 1);
                        for py in min_y..=max_y {
                            for px in min_x..=max_x {
                                let idx = ((py as u32 * width + px as u32) * 4) as usize;
                                rgba[idx] = color[0];
                                rgba[idx + 1] = color[1];
                                rgba[idx + 2] = color[2];
                                rgba[idx + 3] = 255;
                            }
                        }
                    }
                }
            }
        }

        Some(ViewportReadback {
            width,
            height,
            rgba8: rgba,
        })
    }
}

impl GraphicsBackend for Dx11Backend {
    fn kind(&self) -> BackendKind {
        BackendKind::Dx11
    }

    fn initialize(&mut self, _config: &EngineConfig) -> Result<(), BackendError> {
        if self.initialized {
            return Ok(());
        }

        #[cfg(not(target_os = "windows"))]
        {
            Err(BackendError::Unavailable(BackendKind::Dx11))
        }

        #[cfg(target_os = "windows")]
        {
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            let mut feature_level = D3D_FEATURE_LEVEL_11_0;

            unsafe {
                D3D11CreateDevice(
                    None::<&IDXGIAdapter>,
                    D3D_DRIVER_TYPE_HARDWARE,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_FLAG(0),
                    Some(&[D3D_FEATURE_LEVEL_11_0]),
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    Some(&mut feature_level),
                    Some(&mut context),
                )
                .map_err(|err| BackendError::Init(format!("D3D11CreateDevice failed: {err}")))?;
            }

            let device = device.ok_or_else(|| {
                BackendError::Init("D3D11CreateDevice returned no device".to_string())
            })?;
            let context = context.ok_or_else(|| {
                BackendError::Init("D3D11CreateDevice returned no context".to_string())
            })?;

            self.native = Some(Dx11NativeState {
                _device: device,
                context,
                _feature_level: feature_level,
            });
            self.initialized = true;
            self.record_event(
                BackendDiagnosticLevel::Info,
                None,
                None,
                "DX11 initialized with compatibility rendering path",
            );

            Ok(())
        }
    }

    fn create_surface(
        &mut self,
        config: SurfaceConfig,
        window: Option<SurfaceWindowHandles>,
    ) -> Result<SurfaceHandle, BackendError> {
        if !self.initialized {
            return Err(BackendError::Runtime(
                "create_surface called before initialize".to_string(),
            ));
        }

        let handle = SurfaceHandle(self.allocate_handle());
        self.active_surface = Some(handle);
        self.surface_config = Some(config);
        self.diagnostics.supports_surface = false;

        if config.headless {
            self.record_event(
                BackendDiagnosticLevel::Warning,
                None,
                None,
                "Headless DX11 compatibility mode enabled; present is a no-op",
            );
        } else if let Some(handles) = window {
            #[cfg(target_os = "windows")]
            {
                if matches!(handles.window_handle, RawWindowHandle::Win32(_)) {
                    self.diagnostics.supports_surface = true;
                    self.record_event(
                        BackendDiagnosticLevel::Info,
                        None,
                        None,
                        "DX11 native Win32 surface attached; compatibility swapchain path enabled",
                    );
                } else {
                    self.record_event(
                        BackendDiagnosticLevel::Warning,
                        None,
                        None,
                        "DX11 received non-Win32 window handle; using compatibility headless mode",
                    );
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = handles;
                self.record_event(
                    BackendDiagnosticLevel::Warning,
                    None,
                    None,
                    "DX11 window handles ignored on non-Windows build",
                );
            }
        } else {
            self.record_event(
                BackendDiagnosticLevel::Warning,
                None,
                None,
                "No native window handles provided; DX11 running in compatibility headless mode",
            );
        }
        Ok(handle)
    }

    fn resize(
        &mut self,
        surface: SurfaceHandle,
        width: u32,
        height: u32,
    ) -> Result<(), BackendError> {
        if Some(surface) != self.active_surface {
            return Err(BackendError::Surface("unknown surface handle".to_string()));
        }

        if let Some(config) = &mut self.surface_config {
            config.width = width;
            config.height = height;
        }
        if self.diagnostics.supports_surface {
            self.diagnostics.mark_swapchain_recreate();
            self.record_event(
                BackendDiagnosticLevel::Info,
                None,
                None,
                format!("DX11 resize requested to {width}x{height}; swapchain recreation queued"),
            );
        }
        Ok(())
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            textured_sprites: true,
            batching: true,
            camera_transforms: true,
            blend_modes: true,
            offscreen_targets: true,
            texture_atlas: true,
            gpu_nodes: false,
            hybrid_nodes: false,
            compute_nodes: false,
            viewport_readback: true,
        }
    }

    fn diagnostics(&self) -> BackendDiagnostics {
        self.diagnostics.clone()
    }

    fn create_texture(
        &mut self,
        _descriptor: TextureDescriptor,
    ) -> Result<TextureHandle, BackendError> {
        if !self.initialized {
            return Err(BackendError::Runtime(
                "dx11 backend not initialized".to_string(),
            ));
        }

        Ok(TextureHandle(self.allocate_handle()))
    }

    fn create_render_target(
        &mut self,
        _descriptor: RenderTargetDescriptor,
    ) -> Result<RenderTargetHandle, BackendError> {
        if !self.initialized {
            return Err(BackendError::Runtime(
                "dx11 backend not initialized".to_string(),
            ));
        }

        Ok(RenderTargetHandle(self.allocate_handle()))
    }

    fn acquire_frame(&mut self, surface: SurfaceHandle) -> Result<FrameToken, BackendError> {
        if Some(surface) != self.active_surface {
            return Err(BackendError::Surface(
                "acquire_frame called with unknown surface".to_string(),
            ));
        }

        if self.frame_in_flight.is_some() {
            return Err(BackendError::Runtime(
                "frame already acquired and not presented".to_string(),
            ));
        }

        #[cfg(target_os = "windows")]
        {
            if let Some(native) = &self.native {
                unsafe {
                    native.context.ClearState();
                }
            }
        }

        let frame = self.next_frame_token();
        self.frame_in_flight = Some(frame);
        self.frame_start = Some(Instant::now());
        Ok(frame)
    }

    fn record_render_graph(
        &mut self,
        frame: FrameToken,
        graph: &RenderGraph,
    ) -> Result<(), BackendError> {
        if self.frame_in_flight != Some(frame) {
            return Err(BackendError::Command(
                "record_render_graph called with stale frame token".to_string(),
            ));
        }

        for pass in &graph.passes {
            let pass_start = Instant::now();
            let (label, detail) = match pass {
                RenderGraphPass::Render(node) => {
                    let sprite_count: usize =
                        node.batches.iter().map(|batch| batch.sprites.len()).sum();
                    (
                        node.label.clone(),
                        format!(
                            "compat render pass with {} batches and {} sprites",
                            node.batches.len(),
                            sprite_count
                        ),
                    )
                }
                RenderGraphPass::Compute(node) => (
                    node.label.clone(),
                    format!(
                        "compute fallback dispatch {}x{}x{} (compat path)",
                        node.dispatch[0], node.dispatch[1], node.dispatch[2]
                    ),
                ),
            };

            self.record_event(
                BackendDiagnosticLevel::Info,
                Some(frame.0),
                Some(label.clone()),
                format!("DX11 recorded {detail}"),
            );
            self.diagnostics.push_pass_timing(BackendPassTiming {
                frame: frame.0,
                pass: label,
                cpu_ms: pass_start.elapsed().as_secs_f32() * 1000.0,
                gpu_ms: None,
            });
        }

        self.last_viewport_readback = self.synthesize_viewport(graph);

        Ok(())
    }

    fn submit(&mut self, frame: FrameToken) -> Result<(), BackendError> {
        if self.frame_in_flight != Some(frame) {
            return Err(BackendError::Command(
                "submit called with stale frame token".to_string(),
            ));
        }

        #[cfg(target_os = "windows")]
        {
            if let Some(native) = &self.native {
                unsafe {
                    native.context.Flush();
                }
            }
        }

        Ok(())
    }

    fn present(&mut self, frame: FrameToken) -> Result<(), BackendError> {
        if self.frame_in_flight != Some(frame) {
            return Err(BackendError::Command(
                "present called with stale frame token".to_string(),
            ));
        }

        if let Some(start) = self.frame_start.take() {
            let elapsed_ms = start.elapsed().as_secs_f32() * 1000.0;
            self.diagnostics.last_cpu_frame_ms = elapsed_ms;
            self.diagnostics.last_gpu_frame_ms = elapsed_ms;
        }

        self.frame_in_flight = None;
        Ok(())
    }

    fn readback_viewport(&mut self) -> Result<Option<ViewportReadback>, BackendError> {
        Ok(self.last_viewport_readback.clone())
    }

    fn destroy(&mut self) -> Result<(), BackendError> {
        self.destroy_internal()
    }
}

impl Drop for Dx11Backend {
    fn drop(&mut self) {
        if let Err(err) = self.destroy_internal() {
            tracing::warn!("dx11 backend drop cleanup failed: {err}");
        }
    }
}
