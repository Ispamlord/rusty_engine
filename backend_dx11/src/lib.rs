#[cfg(target_os = "windows")]
use std::collections::HashMap;
use std::time::Instant;

use engine_core::EngineConfig;
use engine_render_api::{
    BackendCapabilities, BackendDiagnosticEvent, BackendDiagnosticLevel, BackendDiagnostics,
    BackendError, BackendInstrumentationState, BackendKind, BackendPassTiming, FrameCaptureHandle,
    FrameCaptureRequest, FrameCaptureResult, FrameToken, GpuInstrumentationConfig, GraphicsBackend,
    RenderGraph, RenderGraphPass, RenderTargetDescriptor, RenderTargetHandle, SurfaceConfig,
    SurfaceHandle, SurfaceWindowHandles, TextureDescriptor, TextureHandle, ViewportReadback,
};

#[cfg(test)]
use engine_render_api::FrameCaptureFormat;
#[cfg(target_os = "windows")]
use raw_window_handle::RawWindowHandle;

#[cfg(target_os = "windows")]
use windows::Win32::Foundation::{HMODULE, HWND};
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0,
};
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, D3D11CreateDeviceAndSwapChain, ID3D11Buffer, ID3D11Device,
    ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11SamplerState, ID3D11ShaderResourceView,
    ID3D11Texture2D, ID3D11UnorderedAccessView, D3D11_BIND_FLAG, D3D11_BUFFER_DESC,
    D3D11_CREATE_DEVICE_FLAG, D3D11_CPU_ACCESS_FLAG, D3D11_SDK_VERSION, D3D11_SAMPLER_DESC,
    D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE,
};
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Dxgi::{
    Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_MODE_DESC, DXGI_SAMPLE_DESC},
    IDXGIAdapter, IDXGISwapChain, DXGI_PRESENT, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_EFFECT_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT,
};

#[cfg(target_os = "windows")]
mod renderer;

#[cfg(target_os = "windows")]
struct Dx11NativeState {
    _device: ID3D11Device,
    context: ID3D11DeviceContext,
    _feature_level: D3D_FEATURE_LEVEL,
    renderer: Option<renderer::Dx11Renderer>,
}

#[cfg(target_os = "windows")]
struct Dx11SurfaceState {
    swapchain: IDXGISwapChain,
    render_target_view: ID3D11RenderTargetView,
    width: u32,
    height: u32,
    vsync: bool,
}

#[cfg(target_os = "windows")]
pub(crate) struct Dx11Texture {
    pub _texture: ID3D11Texture2D,
    pub srv: ID3D11ShaderResourceView,
    pub sampler: ID3D11SamplerState,
}

#[cfg(target_os = "windows")]
pub(crate) struct Dx11RenderTarget {
    pub _texture: ID3D11Texture2D,
    pub rtv: ID3D11RenderTargetView,
    pub srv: ID3D11ShaderResourceView,
    pub width: u32,
    pub height: u32,
}

#[cfg(target_os = "windows")]
pub(crate) struct Dx11StorageBuffer {
    pub _buffer: ID3D11Buffer,
    pub uav: ID3D11UnorderedAccessView,
    pub size: u32,
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
    instrumentation: BackendInstrumentationState,
    last_recorded_graph: Option<RenderGraph>,
    last_viewport_readback: Option<ViewportReadback>,

    #[cfg(target_os = "windows")]
    native: Option<Dx11NativeState>,
    #[cfg(target_os = "windows")]
    surface_state: Option<Dx11SurfaceState>,
    #[cfg(target_os = "windows")]
    textures: HashMap<TextureHandle, Dx11Texture>,
    #[cfg(target_os = "windows")]
    render_targets: HashMap<RenderTargetHandle, Dx11RenderTarget>,
    #[cfg(target_os = "windows")]
    storage_buffers: HashMap<String, Dx11StorageBuffer>,
    #[cfg(target_os = "windows")]
    pending_shader_bytecodes: Vec<(String, String, Vec<u8>)>,
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
            instrumentation: BackendInstrumentationState::default(),
            last_recorded_graph: None,
            last_viewport_readback: None,
            #[cfg(target_os = "windows")]
            native: None,
            #[cfg(target_os = "windows")]
            surface_state: None,
            #[cfg(target_os = "windows")]
            textures: HashMap::new(),
            #[cfg(target_os = "windows")]
            render_targets: HashMap::new(),
            #[cfg(target_os = "windows")]
            storage_buffers: HashMap::new(),
            #[cfg(target_os = "windows")]
            pending_shader_bytecodes: Vec::new(),
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

    fn ensure_graph_resources(&mut self,
        _graph: &RenderGraph,
    ) -> Result<(), BackendError> {
        #[cfg(target_os = "windows")]
        {
            if let Some(native) = self.native.as_ref() {
                for resource in &graph.resources {
                    let engine_render_api::GraphResourceKind::StorageBuffer { size_bytes } =
                        resource.kind
                    else {
                        continue;
                    };
                    if self.storage_buffers.contains_key(&resource.name) {
                        continue;
                    }
                    let size = (*size_bytes).max(4) as u32;
                    let buffer = Self::create_dx11_storage_buffer(&native._device,
                        size,
                    )?;
                    self.storage_buffers
                        .insert(resource.name.clone(), buffer);
                }
            }
        }
        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn destroy_surface_state(&mut self) {
        if let Some(state) = self.surface_state.take() {
            // Flush the immediate context so the GPU finishes with the backbuffer
            // and render-target view before they are released.
            if let Some(native) = self.native.as_ref() {
                unsafe {
                    native.context.Flush();
                }
            }
            let _ = state;
        }
    }

    #[cfg(target_os = "windows")]
    fn create_dx11_device_and_surface(
        hwnd: HWND,
        width: u32,
        height: u32,
        vsync: bool,
    ) -> Result<(Dx11NativeState, Dx11SurfaceState), BackendError> {
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        let mut feature_level = D3D_FEATURE_LEVEL_11_0;
        let mut swapchain: Option<IDXGISwapChain> = None;

        let swapchain_desc = DXGI_SWAP_CHAIN_DESC {
            BufferDesc: DXGI_MODE_DESC {
                Width: width,
                Height: height,
                RefreshRate: windows::Win32::Graphics::Dxgi::Common::DXGI_RATIONAL {
                    Numerator: 60,
                    Denominator: 1,
                },
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                ScanlineOrdering:
                    windows::Win32::Graphics::Dxgi::Common::DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED,
                Scaling: windows::Win32::Graphics::Dxgi::Common::DXGI_MODE_SCALING_UNSPECIFIED,
            },
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 1,
            OutputWindow: hwnd,
            Windowed: true.into(),
            SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
            Flags: 0,
        };

        unsafe {
            D3D11CreateDeviceAndSwapChain(
                None::<&IDXGIAdapter>,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_FLAG(0),
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&swapchain_desc),
                Some(&mut swapchain),
                Some(&mut device),
                Some(&mut feature_level),
                Some(&mut context),
            )
            .map_err(|err| {
                BackendError::Init(format!("D3D11CreateDeviceAndSwapChain failed: {err}"))
            })?;
        }

        let device = device.ok_or_else(|| {
            BackendError::Init("D3D11CreateDeviceAndSwapChain returned no device".to_string())
        })?;
        let swapchain = swapchain.ok_or_else(|| {
            BackendError::Init("D3D11CreateDeviceAndSwapChain returned no swapchain".to_string())
        })?;
        let context = context.ok_or_else(|| {
            BackendError::Init("D3D11CreateDeviceAndSwapChain returned no context".to_string())
        })?;

        let render_target_view = Self::create_dx11_render_target_view(&device, &swapchain)?;

        Ok((
            Dx11NativeState {
                _device: device,
                context,
                _feature_level: feature_level,
                renderer: None,
            },
            Dx11SurfaceState {
                swapchain,
                render_target_view,
                width,
                height,
                vsync,
            },
        ))
    }

    #[cfg(target_os = "windows")]
    fn create_dx11_render_target_view(
        device: &ID3D11Device,
        swapchain: &IDXGISwapChain,
    ) -> Result<ID3D11RenderTargetView, BackendError> {
        unsafe {
            let backbuffer: ID3D11Texture2D = swapchain
                .GetBuffer(0)
                .map_err(|err| BackendError::Surface(format!("GetBuffer failed: {err}")))?;
            let mut rtv: Option<ID3D11RenderTargetView> = None;
            device
                .CreateRenderTargetView(&backbuffer, None, Some(&mut rtv))
                .map_err(|err| {
                    BackendError::Surface(format!("CreateRenderTargetView failed: {err}"))
                })?;
            rtv.ok_or_else(|| {
                BackendError::Surface("CreateRenderTargetView returned no view".to_string())
            })
        }
    }

    #[cfg(target_os = "windows")]
    fn resize_dx11_surface(
        device: &ID3D11Device,
        state: &mut Dx11SurfaceState,
        width: u32,
        height: u32,
    ) -> Result<(), BackendError> {
        unsafe {
            state
                .swapchain
                .ResizeBuffers(
                    1,
                    width,
                    height,
                    DXGI_FORMAT_R8G8B8A8_UNORM,
                    windows::Win32::Graphics::Dxgi::DXGI_SWAP_CHAIN_FLAG(0),
                )
                .map_err(|err| BackendError::Surface(format!("ResizeBuffers failed: {err}")))?;
        }
        state.render_target_view = Self::create_dx11_render_target_view(device, &state.swapchain)?;
        state.width = width;
        state.height = height;
        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn create_dx11_texture(
        device: &ID3D11Device,
        width: u32,
        height: u32,
        pixels: Option<&[u8]>,
    ) -> Result<Dx11Texture, BackendError> {
        let pixel_count = (width as usize).checked_mul(height as usize).unwrap_or(0);
        let expected = pixel_count.checked_mul(4).unwrap_or(0);
        let white: Vec<u8> = vec![0xff; expected];
        let data = pixels.unwrap_or(&white);

        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };

        let subresource = D3D11_SUBRESOURCE_DATA {
            pSysMem: data.as_ptr() as *const core::ffi::c_void,
            SysMemPitch: width * 4,
            SysMemSlicePitch: 0,
        };

        unsafe {
            let mut texture: Option<ID3D11Texture2D> = None;
            device
                .CreateTexture2D(&desc,
                    Some(std::slice::from_ref(&subresource).as_ptr()),
                    Some(&mut texture),
                )
                .map_err(|err| BackendError::Runtime(format!("CreateTexture2D failed: {err}")))?;
            let texture = texture.ok_or_else(|| {
                BackendError::Runtime("CreateTexture2D returned no texture".to_string())
            })?;

            let mut srv: Option<ID3D11ShaderResourceView> = None;
            device
                .CreateShaderResourceView(&texture,
                    None,
                    Some(&mut srv),
                )
                .map_err(|err| {
                    BackendError::Runtime(format!("CreateShaderResourceView failed: {err}"))
                })?;
            let srv = srv.ok_or_else(|| {
                BackendError::Runtime("CreateShaderResourceView returned no view".to_string())
            })?;

            let sampler_desc = D3D11_SAMPLER_DESC {
                Filter: windows::Win32::Graphics::Direct3D11::D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: windows::Win32::Graphics::Direct3D11::D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: windows::Win32::Graphics::Direct3D11::D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: windows::Win32::Graphics::Direct3D11::D3D11_TEXTURE_ADDRESS_CLAMP,
                ComparisonFunc: windows::Win32::Graphics::Direct3D11::D3D11_COMPARISON_NEVER,
                MinLOD: 0.0,
                MaxLOD: f32::MAX,
                MipLODBias: 0.0,
                MaxAnisotropy: 1,
                BorderColor: [0.0; 4],
            };
            let mut sampler: Option<ID3D11SamplerState> = None;
            device
                .CreateSamplerState(&sampler_desc, Some(&mut sampler))
                .map_err(|err| BackendError::Runtime(format!("CreateSamplerState failed: {err}")))?;
            let sampler = sampler.ok_or_else(|| {
                BackendError::Runtime("CreateSamplerState returned no state".to_string())
            })?;

            Ok(Dx11Texture {
                _texture: texture,
                srv,
                sampler,
            })
        }
    }

    #[cfg(target_os = "windows")]
    fn create_dx11_render_target(
        device: &ID3D11Device,
        width: u32,
        height: u32,
    ) -> Result<Dx11RenderTarget, BackendError> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE).0,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };

        unsafe {
            let mut texture: Option<ID3D11Texture2D> = None;
            device
                .CreateTexture2D(&desc,
                    None,
                    Some(&mut texture),
                )
                .map_err(|err| BackendError::Runtime(format!("CreateTexture2D(rt) failed: {err}")))?;
            let texture = texture.ok_or_else(|| {
                BackendError::Runtime("CreateTexture2D(rt) returned no texture".to_string())
            })?;

            let mut rtv: Option<ID3D11RenderTargetView> = None;
            device
                .CreateRenderTargetView(&texture, None, Some(&mut rtv))
                .map_err(|err| BackendError::Runtime(format!("CreateRenderTargetView failed: {err}")))?;
            let rtv = rtv.ok_or_else(|| {
                BackendError::Runtime("CreateRenderTargetView returned no view".to_string())
            })?;

            let mut srv: Option<ID3D11ShaderResourceView> = None;
            device
                .CreateShaderResourceView(&texture, None, Some(&mut srv))
                .map_err(|err| {
                    BackendError::Runtime(format!("CreateShaderResourceView(rt) failed: {err}"))
                })?;
            let srv = srv.ok_or_else(|| {
                BackendError::Runtime("CreateShaderResourceView(rt) returned no view".to_string())
            })?;

            Ok(Dx11RenderTarget {
                _texture: texture,
                rtv,
                srv,
                width,
                height,
            })
        }
    }

    #[cfg(target_os = "windows")]
    fn create_dx11_storage_buffer(
        device: &ID3D11Device,
        size: u32,
    ) -> Result<Dx11StorageBuffer, BackendError> {
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: size,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_UNORDERED_ACCESS.0,
            CPUAccessFlags: 0,
            MiscFlags: windows::Win32::Graphics::Direct3D11::D3D11_RESOURCE_MISC_BUFFER_STRUCTURED.0,
            StructureByteStride: 4,
        };

        unsafe {
            let mut buffer: Option<ID3D11Buffer> = None;
            device
                .CreateBuffer(&desc,
                    None,
                    Some(&mut buffer),
                )
                .map_err(|err| BackendError::Runtime(format!("CreateBuffer(storage) failed: {err}")))?;
            let buffer = buffer.ok_or_else(|| {
                BackendError::Runtime("CreateBuffer(storage) returned no buffer".to_string())
            })?;

            let uav_desc = windows::Win32::Graphics::Direct3D11::D3D11_UNORDERED_ACCESS_VIEW_DESC {
                Format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN,
                ViewDimension: windows::Win32::Graphics::Direct3D11::D3D11_UAV_DIMENSION_BUFFER,
                Anonymous: windows::Win32::Graphics::Direct3D11::D3D11_UNORDERED_ACCESS_VIEW_DESC_0 {
                    Buffer: windows::Win32::Graphics::Direct3D11::D3D11_BUFFER_UAV {
                        FirstElement: 0,
                        NumElements: size / 4,
                        Flags: 0,
                    },
                },
            };
            let mut uav: Option<ID3D11UnorderedAccessView> = None;
            device
                .CreateUnorderedAccessView(&buffer,
                    Some(&uav_desc),
                    Some(&mut uav),
                )
                .map_err(|err| BackendError::Runtime(format!("CreateUnorderedAccessView failed: {err}")))?;
            let uav = uav.ok_or_else(|| {
                BackendError::Runtime("CreateUnorderedAccessView returned no view".to_string())
            })?;

            Ok(Dx11StorageBuffer {
                _buffer: buffer,
                uav,
                size,
            })
        }
    }

    fn destroy_internal(&mut self) -> Result<(), BackendError> {
        #[cfg(target_os = "windows")]
        {
            self.destroy_surface_state();
            if let Some(native) = self.native.as_mut() {
                if let Some(renderer) = native.renderer.as_mut() {
                    renderer.destroy();
                }
            }
            self.native = None;
            self.textures.clear();
            self.render_targets.clear();
            self.storage_buffers.clear();
            self.pending_shader_bytecodes.clear();
        }

        self.initialized = false;
        self.frame_in_flight = None;
        self.frame_start = None;
        self.active_surface = None;
        self.surface_config = None;
        self.last_recorded_graph = None;
        self.last_viewport_readback = None;

        Ok(())
    }

    fn synthesize_viewport(&self, graph: &RenderGraph) -> Option<ViewportReadback> {
        let config = self.surface_config?;
        let width = config.width.max(1);
        let height = config.height.max(1);
        let buffer_size = ViewportReadback::buffer_size(width, height)?;
        let mut rgba = vec![0_u8; buffer_size];
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
                let zoom = render.camera.zoom.max(0.05);
                for batch in &render.batches {
                    for sprite in &batch.sprites {
                        let cx = (width as f32 * 0.5 + (sprite.x + render.camera.x) * zoom).round()
                            as i32;
                        let cy = (height as f32 * 0.5 + (sprite.y + render.camera.y) * zoom).round()
                            as i32;
                        let hw = (sprite.width * 0.5 * zoom).round().max(1.0) as i32;
                        let hh = (sprite.height * 0.5 * zoom).round().max(1.0) as i32;
                        let color = [
                            (sprite.tint[0].clamp(0.0, 1.0) * 255.0) as u8,
                            (sprite.tint[1].clamp(0.0, 1.0) * 255.0) as u8,
                            (sprite.tint[2].clamp(0.0, 1.0) * 255.0) as u8,
                        ];
                        let min_x = (cx - hw).max(0);
                        let max_x = (cx + hw).min(width as i32 - 1);
                        let min_y = (cy - hh).max(0);
                        let max_y = (cy + hh).min(height as i32 - 1);
                        let shape = sprite.texture.0 % 3;
                        let hwf = hw.max(1) as f32;
                        let hhf = hh.max(1) as f32;
                        for py in min_y..=max_y {
                            for px in min_x..=max_x {
                                let nx = (px - cx) as f32 / hwf;
                                let ny = (py - cy) as f32 / hhf;
                                let inside = match shape {
                                    1 => nx * nx + ny * ny <= 1.0,
                                    2 => {
                                        let t = ((ny + 1.0) * 0.5).clamp(0.0, 1.0);
                                        nx.abs() <= t
                                    }
                                    _ => true,
                                };
                                if !inside {
                                    continue;
                                }
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
                renderer: None,
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

        #[cfg(target_os = "windows")]
        self.destroy_surface_state();

        let handle = SurfaceHandle(self.allocate_handle());
        self.active_surface = Some(handle);
        self.surface_config = Some(config);
        self.diagnostics.supports_surface = false;
        self.last_recorded_graph = None;
        self.last_viewport_readback = None;

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
                if let RawWindowHandle::Win32(win32) = handles.window_handle {
                    let hwnd = HWND(win32.hwnd.get() as *mut core::ffi::c_void);
                    match Self::create_dx11_device_and_surface(
                        hwnd,
                        config.width.max(1),
                        config.height.max(1),
                        config.vsync,
                    ) {
                        Ok((native, surface)) => {
                            self.native = Some(native);
                            self.surface_state = Some(surface);
                            self.diagnostics.supports_surface = true;
                            self.record_event(
                                BackendDiagnosticLevel::Info,
                                None,
                                None,
                                "DX11 native Win32 swapchain and RTV created",
                            );
                        }
                        Err(err) => {
                            self.record_event(
                                BackendDiagnosticLevel::Error,
                                None,
                                None,
                                format!("DX11 swapchain creation failed: {err}"),
                            );
                            return Err(err);
                        }
                    }
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
            #[cfg(target_os = "windows")]
            {
                if let (Some(native), Some(surface_state)) =
                    (self.native.as_ref(), self.surface_state.as_mut())
                {
                    if let Err(err) = Self::resize_dx11_surface(
                        &native._device,
                        surface_state,
                        width.max(1),
                        height.max(1),
                    ) {
                        self.record_event(
                            BackendDiagnosticLevel::Error,
                            None,
                            None,
                            format!("DX11 resize failed: {err}"),
                        );
                        return Err(err);
                    }
                }
            }
            self.diagnostics.mark_swapchain_recreate();
            self.record_event(
                BackendDiagnosticLevel::Info,
                None,
                None,
                format!("DX11 resized swapchain/RTV to {width}x{height}"),
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

        let handle = TextureHandle(self.allocate_handle());

        #[cfg(target_os = "windows")]
        {
            if let Some(native) = self.native.as_ref() {
                let texture = Self::create_dx11_texture(
                    &native._device,
                    _descriptor.width,
                    _descriptor.height,
                    None,
                )?;
                self.textures.insert(handle, texture);
            }
        }

        Ok(handle)
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

        let handle = RenderTargetHandle(self.allocate_handle());

        #[cfg(target_os = "windows")]
        {
            if let Some(native) = self.native.as_ref() {
                let target = Self::create_dx11_render_target(
                    &native._device,
                    _descriptor.width,
                    _descriptor.height,
                )?;
                self.render_targets.insert(handle, target);
            }
        }

        Ok(handle)
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

        self.ensure_graph_resources(graph)?;

        for pass in &graph.passes {
            let pass_start = Instant::now();
            let (label, detail) = match pass {
                RenderGraphPass::Render(node) => {
                    let sprite_count: usize =
                        node.batches.iter().map(|batch| batch.sprites.len()).sum();
                    let shader = node
                        .material
                        .as_ref()
                        .map(|m| m.shader_asset.as_str())
                        .unwrap_or("builtin");
                    (
                        node.label.clone(),
                        format!(
                            "compat render pass with {} batches and {} sprites (shader {})",
                            node.batches.len(),
                            sprite_count,
                            shader
                        ),
                    )
                }
                RenderGraphPass::Compute(node) => (
                    node.label.clone(),
                    format!(
                        "compute fallback dispatch {}x{}x{} (compat path, shader {})",
                        node.dispatch[0],
                        node.dispatch[1],
                        node.dispatch[2],
                        node.material.shader_asset
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

        #[cfg(target_os = "windows")]
        {
            if let Some(surface_state) = self.surface_state.as_ref() {
                let native = self.native.as_mut().ok_or_else(|| {
                    BackendError::Runtime("dx11 native state missing".to_string())
                })?;
                if native.renderer.is_none() {
                    let mut renderer = renderer::Dx11Renderer::new(&native._device)?;
                    let pending = std::mem::take(&mut self.pending_shader_bytecodes);
                    for (name, entry_point, bytecode) in &pending {
                        renderer.preload_shader_bytecode(name, entry_point, bytecode);
                    }
                    native.renderer = Some(renderer);
                }
                let context = native.context.clone();
                let renderer = native.renderer.as_mut().unwrap();
                renderer.record_render_graph(
                    &context,
                    graph,
                    &self.textures,
                    &self.render_targets,
                    &self.storage_buffers,
                    &surface_state.render_target_view,
                    surface_state.width,
                    surface_state.height,
                )?;
            }
        }

        self.last_recorded_graph = Some(graph.clone());
        self.last_viewport_readback = None;

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

        #[cfg(target_os = "windows")]
        {
            if let Some(surface_state) = self.surface_state.as_ref() {
                let interval = if surface_state.vsync { 1 } else { 0 };
                unsafe {
                    surface_state.swapchain.Present(interval, DXGI_PRESENT(0));
                }
            }
        }

        self.frame_in_flight = None;
        Ok(())
    }

    fn readback_viewport(&mut self) -> Result<Option<ViewportReadback>, BackendError> {
        if self.last_viewport_readback.is_none() {
            if let Some(graph) = self.last_recorded_graph.as_ref() {
                self.last_viewport_readback = self.synthesize_viewport(graph);
            }
        }
        Ok(self.last_viewport_readback.clone())
    }

    fn preload_shader_bytecode(
        &mut self,
        name: &str,
        entry_point: &str,
        bytecode: &[u8],
    ) -> Result<(), BackendError> {
        #[cfg(target_os = "windows")]
        {
            if let Some(native) = self.native.as_mut() {
                if let Some(renderer) = native.renderer.as_mut() {
                    renderer.preload_shader_bytecode(name, entry_point, bytecode);
                    return Ok(());
                }
            }
            self.pending_shader_bytecodes.push((
                name.to_string(),
                entry_point.to_string(),
                bytecode.to_vec(),
            ));
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (name, entry_point, bytecode);
        }
        Ok(())
    }

    fn destroy(&mut self) -> Result<(), BackendError> {
        self.destroy_internal()
    }

    fn set_gpu_timestamps_enabled(&mut self, enabled: bool) {
        self.instrumentation.set_gpu_timestamps_enabled(enabled);
        self.diagnostics.gpu_timestamps_enabled = enabled;
    }

    fn configure_gpu_instrumentation(&mut self, config: GpuInstrumentationConfig) {
        self.instrumentation.configure(config);
        self.diagnostics.gpu_timestamps_enabled = config.enabled;
    }

    fn request_frame_capture(
        &mut self,
        request: FrameCaptureRequest,
    ) -> Result<FrameCaptureHandle, BackendError> {
        let handle = self.instrumentation.request_capture(request);
        self.diagnostics.frame_captures_pending =
            self.instrumentation.pending_captures.len() as u64;
        Ok(handle)
    }

    fn poll_frame_capture(
        &mut self,
        handle: FrameCaptureHandle,
    ) -> Result<FrameCaptureResult, BackendError> {
        if let Some(result) = self.instrumentation.poll_capture(handle) {
            self.diagnostics.frame_captures_pending =
                self.instrumentation.pending_captures.len() as u64;
            self.diagnostics.frame_captures_completed =
                self.instrumentation.completed_captures.len() as u64;
            Ok(result)
        } else {
            Err(BackendError::Runtime(format!(
                "frame capture handle {:?} not found",
                handle
            )))
        }
    }
}

impl Drop for Dx11Backend {
    fn drop(&mut self) {
        if let Err(err) = self.destroy_internal() {
            tracing::warn!("dx11 backend drop cleanup failed: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_dx11() {
        let backend = Dx11Backend::new();
        assert_eq!(backend.kind(), BackendKind::Dx11);
    }

    #[test]
    fn capabilities_compat_path_disables_gpu_nodes() {
        let backend = Dx11Backend::new();
        assert!(backend.capabilities().viewport_readback);
        assert!(!backend.capabilities().compute_nodes);
        assert!(!backend.capabilities().gpu_nodes);
        assert!(!backend.capabilities().hybrid_nodes);
    }

    #[test]
    fn diagnostics_new_has_zero_counts() {
        let backend = Dx11Backend::new();
        let diag = backend.diagnostics();
        assert_eq!(diag.backend, BackendKind::Dx11);
        assert!(!diag.supports_surface);
        assert_eq!(diag.fallback_events, 0);
        assert_eq!(diag.swapchain_recreates, 0);
        assert_eq!(diag.device_loss_events, 0);
        assert!(!diag.gpu_timestamps_enabled);
        assert_eq!(diag.frame_captures_pending, 0);
        assert_eq!(diag.frame_captures_completed, 0);
    }

    #[test]
    fn frame_capture_request_and_poll() {
        let mut backend = Dx11Backend::new();
        let handle = backend
            .request_frame_capture(FrameCaptureRequest {
                label: "test".into(),
                format: FrameCaptureFormat::Rgba8,
            })
            .expect("request should succeed");
        assert_eq!(backend.diagnostics().frame_captures_pending, 1);

        let in_flight = backend
            .poll_frame_capture(handle)
            .expect("poll should return in-flight marker");
        assert!(!in_flight.completed);

        backend
            .instrumentation
            .complete_oldest_capture(2, 2, vec![0; 16]);
        let completed = backend
            .poll_frame_capture(handle)
            .expect("poll should return completed capture");
        assert!(completed.completed);
        assert_eq!(completed.data, Some(vec![0; 16]));
    }

    #[test]
    fn gpu_timestamps_enable_updates_diagnostics() {
        let mut backend = Dx11Backend::new();
        backend.set_gpu_timestamps_enabled(true);
        assert!(backend.diagnostics().gpu_timestamps_enabled);
    }
}
