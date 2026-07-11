use std::time::Instant;

#[cfg(target_os = "windows")]
use std::collections::HashMap;

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
use windows::core::{IUnknown, Interface};
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Direct3D::{D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_12_0};
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Direct3D12::{
    D3D12_CPU_DESCRIPTOR_HANDLE, D3D12_CPU_PAGE_PROPERTY_UNKNOWN, D3D12CreateDevice,
    D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING, D3D12_DESCRIPTOR_HEAP_DESC,
    D3D12_DESCRIPTOR_HEAP_FLAG_NONE, D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
    D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
    D3D12_GPU_DESCRIPTOR_HANDLE, D3D12_HEAP_PROPERTIES, D3D12_HEAP_TYPE_DEFAULT,
    D3D12_MEMORY_POOL_UNKNOWN, D3D12_RESOURCE_DESC, D3D12_RESOURCE_DIMENSION_BUFFER,
    D3D12_RESOURCE_DIMENSION_TEXTURE2D, D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
    D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS, D3D12_RESOURCE_FLAG_NONE,
    D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_RENDER_TARGET,
    D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
    D3D12_RENDER_TARGET_VIEW_DESC, D3D12_RENDER_TARGET_VIEW_DESC_0, D3D12_RTV_DIMENSION_TEXTURE2D,
    D3D12_TEX2D_RTV, D3D12_SHADER_RESOURCE_VIEW_DESC, D3D12_SHADER_RESOURCE_VIEW_DESC_0,
    D3D12_SRV_DIMENSION_TEXTURE2D, D3D12_TEX2D_SRV, D3D12_TEXTURE_LAYOUT_UNKNOWN,
    D3D12_UNORDERED_ACCESS_VIEW_DESC, D3D12_UNORDERED_ACCESS_VIEW_DESC_0, D3D12_UAV_DIMENSION_BUFFER,
    D3D12_BUFFER_UAV, D3D12_BUFFER_UAV_FLAG_NONE, D3D12_CLEAR_VALUE, D3D12_CLEAR_VALUE_0,
    ID3D12CommandAllocator, ID3D12CommandList, ID3D12CommandQueue, ID3D12DescriptorHeap,
    ID3D12Device, ID3D12Fence, ID3D12GraphicsCommandList, ID3D12Resource,
    D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_COMMAND_QUEUE_DESC, D3D12_COMMAND_QUEUE_FLAG_NONE,
    D3D12_FENCE_FLAG_NONE, D3D12_HEAP_FLAG_NONE,
};
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, IDXGIAdapter1, IDXGIFactory4, IDXGISwapChain1, IDXGISwapChain3,
    DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_CREATE_FACTORY_FLAGS, DXGI_ERROR_NOT_FOUND,
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_PRESENT, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
#[cfg(target_os = "windows")]
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject, INFINITE};

#[cfg(target_os = "windows")]
mod renderer;
#[cfg(target_os = "windows")]
use renderer::Dx12Renderer;

#[cfg(target_os = "windows")]
struct Dx12NativeState {
    _factory: IDXGIFactory4,
    _adapter: IDXGIAdapter1,
    device: ID3D12Device,
    queue: ID3D12CommandQueue,
    allocator: ID3D12CommandAllocator,
    command_list: ID3D12GraphicsCommandList,
    fence: ID3D12Fence,
    fence_event: HANDLE,
    fence_value: u64,
}

#[cfg(target_os = "windows")]
struct Dx12SurfaceState {
    swapchain: IDXGISwapChain3,
    rtv_heap: ID3D12DescriptorHeap,
    render_targets: Vec<ID3D12Resource>,
    rtv_descriptor_size: u32,
    current_backbuffer_index: u32,
    width: u32,
    height: u32,
    vsync: bool,
}

#[cfg(target_os = "windows")]
struct Dx12Texture {
    texture: ID3D12Resource,
    srv: D3D12_CPU_DESCRIPTOR_HANDLE,
}

#[cfg(target_os = "windows")]
struct Dx12RenderTarget {
    texture: ID3D12Resource,
    rtv: D3D12_CPU_DESCRIPTOR_HANDLE,
    srv: D3D12_CPU_DESCRIPTOR_HANDLE,
    width: u32,
    height: u32,
}

#[cfg(target_os = "windows")]
struct Dx12StorageBuffer {
    buffer: ID3D12Resource,
    uav: D3D12_CPU_DESCRIPTOR_HANDLE,
    size: u32,
}

pub struct Dx12Backend {
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
    native: Option<Dx12NativeState>,
    #[cfg(target_os = "windows")]
    surface_state: Option<Dx12SurfaceState>,
    #[cfg(target_os = "windows")]
    renderer: Option<Dx12Renderer>,
    #[cfg(target_os = "windows")]
    textures: HashMap<TextureHandle, Dx12Texture>,
    #[cfg(target_os = "windows")]
    render_targets: HashMap<RenderTargetHandle, Dx12RenderTarget>,
    #[cfg(target_os = "windows")]
    storage_buffers: HashMap<String, Dx12StorageBuffer>,
}

impl Default for Dx12Backend {
    fn default() -> Self {
        Self::new()
    }
}

impl Dx12Backend {
    pub fn new() -> Self {
        Self {
            initialized: false,
            frame_in_flight: None,
            frame_start: None,
            next_handle: 1,
            next_frame: 1,
            active_surface: None,
            surface_config: None,
            diagnostics: BackendDiagnostics::new(BackendKind::Dx12),
            instrumentation: BackendInstrumentationState::default(),
            last_recorded_graph: None,
            last_viewport_readback: None,
            #[cfg(target_os = "windows")]
            native: None,
            #[cfg(target_os = "windows")]
            surface_state: None,
            #[cfg(target_os = "windows")]
            renderer: None,
            #[cfg(target_os = "windows")]
            textures: HashMap::new(),
            #[cfg(target_os = "windows")]
            render_targets: HashMap::new(),
            #[cfg(target_os = "windows")]
            storage_buffers: HashMap::new(),
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

    #[cfg(target_os = "windows")]
    fn native_mut(&mut self) -> Result<&mut Dx12NativeState, BackendError> {
        self.native
            .as_mut()
            .ok_or_else(|| BackendError::Runtime("dx12 native state not initialized".to_string()))
    }

    #[cfg(target_os = "windows")]
    fn wait_for_fence(state: &Dx12NativeState, value: u64) -> Result<(), BackendError> {
        let completed = unsafe { state.fence.GetCompletedValue() };
        if completed >= value {
            return Ok(());
        }

        unsafe {
            state
                .fence
                .SetEventOnCompletion(value, state.fence_event)
                .map_err(|err| {
                    BackendError::Runtime(format!("SetEventOnCompletion failed: {err}"))
                })?;
            let _ = WaitForSingleObject(state.fence_event, INFINITE);
        }
        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn pick_adapter(factory: &IDXGIFactory4) -> Result<IDXGIAdapter1, BackendError> {
        let mut index = 0;
        loop {
            match unsafe { factory.EnumAdapters1(index) } {
                Ok(adapter) => {
                    let desc = unsafe { adapter.GetDesc1() }
                        .map_err(|err| BackendError::Init(format!("GetDesc1 failed: {err}")))?;
                    if (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) == 0 {
                        return Ok(adapter);
                    }
                }
                Err(err) if err.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(err) => {
                    return Err(BackendError::Init(format!(
                        "EnumAdapters1 failed while selecting adapter: {err}"
                    )))
                }
            }
            index += 1;
        }

        Err(BackendError::Init(
            "no suitable DX12 hardware adapter found".to_string(),
        ))
    }

    #[cfg(target_os = "windows")]
    fn destroy_surface_state(&mut self) {
        if let Some(state) = self.surface_state.take() {
            // Wait for the GPU to finish using these resources before releasing them.
            let _ = Self::wait_for_fence(
                self.native.as_ref().expect("native state must exist"),
                state.current_backbuffer_index as u64,
            );
            let _ = state;
        }
    }

    fn destroy_internal(&mut self) -> Result<(), BackendError> {
        #[cfg(target_os = "windows")]
        {
            self.destroy_surface_state();
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.destroy();
            }
            self.renderer = None;
            self.textures.clear();
            self.render_targets.clear();
            self.storage_buffers.clear();
            if let Some(state) = self.native.take() {
                unsafe {
                    let _ = state.queue.Signal(&state.fence, state.fence_value + 1);
                    let _ = Self::wait_for_fence(&state, state.fence_value + 1);
                    let _ = CloseHandle(state.fence_event);
                }
            }
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

    #[cfg(target_os = "windows")]
    fn create_dx12_surface_state(
        factory: &IDXGIFactory4,
        device: &ID3D12Device,
        queue: &ID3D12CommandQueue,
        hwnd: HWND,
        width: u32,
        height: u32,
        vsync: bool,
    ) -> Result<Dx12SurfaceState, BackendError> {
        let swapchain_desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            Stereo: false.into(),
            SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: windows::Win32::Graphics::Dxgi::DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: windows::Win32::Graphics::Dxgi::Common::DXGI_ALPHA_MODE_IGNORE,
            Flags: 0,
        };

        let swapchain1: IDXGISwapChain1 = unsafe {
            factory
                .CreateSwapChainForHwnd(queue, hwnd, &swapchain_desc, None, None)
                .map_err(|err| BackendError::Surface(format!("CreateSwapChainForHwnd failed: {err}")))?
        };

        let swapchain: IDXGISwapChain3 = swapchain1
            .cast()
            .map_err(|err| BackendError::Surface(format!("swapchain cast to IDXGISwapChain3 failed: {err}")))?;

        let (rtv_heap, render_targets, rtv_descriptor_size) =
            Self::create_dx12_render_target_views(device, &swapchain, 2)?;

        let current_backbuffer_index = unsafe { swapchain.GetCurrentBackBufferIndex() };

        Ok(Dx12SurfaceState {
            swapchain,
            rtv_heap,
            render_targets,
            rtv_descriptor_size,
            current_backbuffer_index,
            width,
            height,
            vsync,
        })
    }

    #[cfg(target_os = "windows")]
    fn create_dx12_render_target_views(
        device: &ID3D12Device,
        swapchain: &IDXGISwapChain3,
        buffer_count: u32,
    ) -> Result<(ID3D12DescriptorHeap, Vec<ID3D12Resource>, u32), BackendError> {
        let heap_desc = D3D12_DESCRIPTOR_HEAP_DESC {
            Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
            NumDescriptors: buffer_count,
            Flags: D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
            NodeMask: 0,
        };
        let rtv_heap: ID3D12DescriptorHeap = unsafe {
            device
                .CreateDescriptorHeap(&heap_desc)
                .map_err(|err| BackendError::Surface(format!("CreateDescriptorHeap failed: {err}")))?
        };

        let rtv_descriptor_size =
            unsafe { device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV) };
        let rtv_start = unsafe { rtv_heap.GetCPUDescriptorHandleForHeapStart() };

        let mut render_targets = Vec::with_capacity(buffer_count as usize);
        for i in 0..buffer_count {
            let resource: ID3D12Resource = unsafe {
                swapchain
                    .GetBuffer(i)
                    .map_err(|err| BackendError::Surface(format!("GetBuffer failed: {err}")))?
            };
            let handle = D3D12_CPU_DESCRIPTOR_HANDLE {
                ptr: rtv_start.ptr + (i as usize * rtv_descriptor_size as usize),
            };
            unsafe {
                device
                    .CreateRenderTargetView(&resource, None, handle)
                    .map_err(|err| BackendError::Surface(format!("CreateRenderTargetView failed: {err}")))?;
            }
            render_targets.push(resource);
        }

        Ok((rtv_heap, render_targets, rtv_descriptor_size))
    }

    #[cfg(target_os = "windows")]
    fn create_dx12_texture(
        device: &ID3D12Device,
        width: u32,
        height: u32,
    ) -> Result<ID3D12Resource, BackendError> {
        let heap_properties = D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_DEFAULT,
            CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
            MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
            CreationNodeMask: 0,
            VisibleNodeMask: 0,
        };
        let resource_desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Alignment: 0,
            Width: width as u64,
            Height: height,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
            Flags: D3D12_RESOURCE_FLAG_NONE,
        };

        unsafe {
            device
                .CreateCommittedResource(
                    &heap_properties,
                    D3D12_HEAP_FLAG_NONE,
                    &resource_desc,
                    D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                    None,
                )
                .map_err(|err| BackendError::Runtime(format!("create texture failed: {err}")))
        }
    }

    #[cfg(target_os = "windows")]
    fn create_dx12_render_target(
        device: &ID3D12Device,
        renderer: &Dx12Renderer,
        width: u32,
        height: u32,
    ) -> Result<(ID3D12Resource, D3D12_CPU_DESCRIPTOR_HANDLE, D3D12_CPU_DESCRIPTOR_HANDLE), BackendError>
    {
        let heap_properties = D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_DEFAULT,
            CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
            MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
            CreationNodeMask: 0,
            VisibleNodeMask: 0,
        };
        let resource_desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Alignment: 0,
            Width: width as u64,
            Height: height,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
            Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
        };
        let clear_value = D3D12_CLEAR_VALUE {
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            Anonymous: D3D12_CLEAR_VALUE_0 {
                Color: [0.0, 0.0, 0.0, 0.0],
            },
        };

        let texture = unsafe {
            device
                .CreateCommittedResource(
                    &heap_properties,
                    D3D12_HEAP_FLAG_NONE,
                    &resource_desc,
                    D3D12_RESOURCE_STATE_RENDER_TARGET,
                    Some(&clear_value),
                )
                .map_err(|err| BackendError::Runtime(format!("create render target failed: {err}")))?
        };

        let rtv_index = renderer
            .allocate_rtv_index()
            .ok_or_else(|| BackendError::Runtime("offscreen rtv heap exhausted".to_string()))?;
        let rtv = renderer.rtv_cpu_handle(rtv_index);

        let rtv_desc = D3D12_RENDER_TARGET_VIEW_DESC {
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            ViewDimension: D3D12_RTV_DIMENSION_TEXTURE2D,
            Anonymous: D3D12_RENDER_TARGET_VIEW_DESC_0 {
                Texture2D: D3D12_TEX2D_RTV {
                    MipSlice: 0,
                    PlaneSlice: 0,
                },
            },
        };
        unsafe {
            device.CreateRenderTargetView(&texture, Some(&rtv_desc), rtv);
        }

        let srv_index = renderer
            .allocate_descriptor_index()
            .ok_or_else(|| BackendError::Runtime("srv heap exhausted".to_string()))?;
        let srv = renderer.cpu_handle(srv_index);

        let srv_desc = D3D12_SHADER_RESOURCE_VIEW_DESC {
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
            Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
            Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                Texture2D: D3D12_TEX2D_SRV {
                    MostDetailedMip: 0,
                    MipLevels: 1,
                    PlaneSlice: 0,
                    ResourceMinLODClamp: 0.0,
                },
            },
        };
        unsafe {
            device.CreateShaderResourceView(&texture, Some(&srv_desc), srv);
        }

        Ok((texture, rtv, srv))
    }

    #[cfg(target_os = "windows")]
    fn create_dx12_storage_buffer(
        device: &ID3D12Device,
        renderer: &Dx12Renderer,
        size: u32,
    ) -> Result<(ID3D12Resource, D3D12_CPU_DESCRIPTOR_HANDLE), BackendError> {
        let heap_properties = D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_DEFAULT,
            CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
            MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
            CreationNodeMask: 0,
            VisibleNodeMask: 0,
        };
        let resource_desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
            Alignment: 0,
            Width: size as u64,
            Height: 1,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: DXGI_FORMAT_UNKNOWN,
            SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
            Flags: D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
        };

        let buffer = unsafe {
            device
                .CreateCommittedResource(
                    &heap_properties,
                    D3D12_HEAP_FLAG_NONE,
                    &resource_desc,
                    D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                    None,
                )
                .map_err(|err| BackendError::Runtime(format!("create storage buffer failed: {err}")))?
        };

        let uav_index = renderer
            .allocate_descriptor_index()
            .ok_or_else(|| BackendError::Runtime("uav heap exhausted".to_string()))?;
        let uav = renderer.cpu_handle(uav_index);

        let uav_desc = D3D12_UNORDERED_ACCESS_VIEW_DESC {
            Format: DXGI_FORMAT_UNKNOWN,
            ViewDimension: D3D12_UAV_DIMENSION_BUFFER,
            Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
                Buffer: D3D12_BUFFER_UAV {
                    FirstElement: 0,
                    NumElements: (size / 4).max(1),
                    StructureByteStride: 0,
                    CounterOffsetInBytes: 0,
                    Flags: D3D12_BUFFER_UAV_FLAG_NONE,
                },
            },
        };
        unsafe {
            device.CreateUnorderedAccessView(&buffer, None, Some(&uav_desc), uav);
        }

        Ok((buffer, uav))
    }

    #[cfg(target_os = "windows")]
    fn ensure_graph_resources(
        &mut self,
        graph: &RenderGraph,
    ) -> Result<(), BackendError> {
        let native = self.native.as_ref().ok_or_else(|| {
            BackendError::Runtime("dx12 native state missing during graph resource creation".to_string())
        })?;
        let renderer = self.renderer.as_ref().ok_or_else(|| {
            BackendError::Runtime("dx12 renderer missing during graph resource creation".to_string())
        })?;

        for resource in &graph.resources {
            if let GraphResourceKind::StorageBuffer { size_bytes } = &resource.kind {
                if !self.storage_buffers.contains_key(&resource.name) {
                    let size = (*size_bytes).max(4) as u32;
                    let (buffer, uav) =
                        Self::create_dx12_storage_buffer(&native.device, renderer, size)?;
                    self.storage_buffers.insert(
                        resource.name.clone(),
                        Dx12StorageBuffer { buffer, uav, size },
                    );
                }
            }
        }

        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn resize_dx12_surface(
        device: &ID3D12Device,
        state: &mut Dx12SurfaceState,
        width: u32,
        height: u32,
    ) -> Result<(), BackendError> {
        unsafe {
            state
                .swapchain
                .ResizeBuffers(
                    2,
                    width,
                    height,
                    DXGI_FORMAT_R8G8B8A8_UNORM,
                    DXGI_SWAP_CHAIN_FLAG(0),
                )
                .map_err(|err| BackendError::Surface(format!("ResizeBuffers failed: {err}")))?;
        }

        let (rtv_heap, render_targets, rtv_descriptor_size) =
            Self::create_dx12_render_target_views(device, &state.swapchain, 2)?;
        state.rtv_heap = rtv_heap;
        state.render_targets = render_targets;
        state.rtv_descriptor_size = rtv_descriptor_size;
        state.current_backbuffer_index = unsafe { state.swapchain.GetCurrentBackBufferIndex() };
        state.width = width;
        state.height = height;
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
                rgba[idx] = 12;
                rgba[idx + 1] = 18;
                rgba[idx + 2] = 30;
                rgba[idx + 3] = 255;
            }
        }

        for pass in &graph.passes {
            if let RenderGraphPass::Render(render) = pass {
                let zoom = render.camera.zoom.max(0.05);
                for batch in &render.batches {
                    for sprite in &batch.sprites {
                        let cx = (width as f32 * 0.5 + (sprite.x + render.camera.x) * zoom)
                            .round() as i32;
                        let cy = (height as f32 * 0.5 + (sprite.y + render.camera.y) * zoom)
                            .round() as i32;
                        let hw = (sprite.width * 0.5 * zoom).round().max(1.0) as i32;
                        let hh = (sprite.height * 0.5 * zoom).round().max(1.0) as i32;
                        let tint = [
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
                                rgba[idx] = tint[0];
                                rgba[idx + 1] = tint[1];
                                rgba[idx + 2] = tint[2];
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

impl GraphicsBackend for Dx12Backend {
    fn kind(&self) -> BackendKind {
        BackendKind::Dx12
    }

    fn initialize(&mut self, _config: &EngineConfig) -> Result<(), BackendError> {
        if self.initialized {
            return Ok(());
        }

        #[cfg(not(target_os = "windows"))]
        {
            Err(BackendError::Unavailable(BackendKind::Dx12))
        }

        #[cfg(target_os = "windows")]
        {
            let factory: IDXGIFactory4 = unsafe {
                CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))
            }
            .map_err(|err| BackendError::Init(format!("CreateDXGIFactory2 failed: {err}")))?;
            let adapter = Self::pick_adapter(&factory)?;

            let adapter_unknown: IUnknown = adapter
                .cast()
                .map_err(|err| BackendError::Init(format!("adapter cast failed: {err}")))?;

            let mut device: Option<ID3D12Device> = None;
            unsafe {
                D3D12CreateDevice(&adapter_unknown, D3D_FEATURE_LEVEL_12_0, &mut device)
                    .or_else(|_| {
                        D3D12CreateDevice(&adapter_unknown, D3D_FEATURE_LEVEL_11_0, &mut device)
                    })
                    .map_err(|err| {
                        BackendError::Init(format!("D3D12CreateDevice failed: {err}"))
                    })?;
            }
            let device = device.ok_or_else(|| {
                BackendError::Init("D3D12CreateDevice returned no device".to_string())
            })?;

            let queue_desc = D3D12_COMMAND_QUEUE_DESC {
                Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
                Priority: 0,
                Flags: D3D12_COMMAND_QUEUE_FLAG_NONE,
                NodeMask: 0,
            };

            let queue: ID3D12CommandQueue = unsafe { device.CreateCommandQueue(&queue_desc) }
                .map_err(|err| BackendError::Init(format!("CreateCommandQueue failed: {err}")))?;

            let allocator: ID3D12CommandAllocator =
                unsafe { device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }.map_err(
                    |err| BackendError::Init(format!("CreateCommandAllocator failed: {err}")),
                )?;

            let command_list: ID3D12GraphicsCommandList = unsafe {
                device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &allocator, None)
            }
            .map_err(|err| BackendError::Init(format!("CreateCommandList failed: {err}")))?;

            unsafe {
                command_list
                    .Close()
                    .map_err(|err| BackendError::Init(format!("initial Close failed: {err}")))?;
            }

            let fence: ID3D12Fence = unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }
                .map_err(|err| BackendError::Init(format!("CreateFence failed: {err}")))?;

            let fence_event = unsafe { CreateEventW(None, false, false, None) }
                .map_err(|err| BackendError::Init(format!("CreateEventW failed: {err}")))?;

            let renderer =
                Dx12Renderer::new(&device).map_err(|err| {
                    BackendError::Init(format!("dx12 renderer creation failed: {err}"))
                })?;

            self.native = Some(Dx12NativeState {
                _factory: factory,
                _adapter: adapter,
                device,
                queue,
                allocator,
                command_list,
                fence,
                fence_event,
                fence_value: 0,
            });
            self.renderer = Some(renderer);

            self.initialized = true;
            self.record_event(
                BackendDiagnosticLevel::Info,
                None,
                None,
                "DX12 initialized with device/queue/allocator/list/fence",
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
                "Headless DX12 surface mode enabled; present is a no-op",
            );
        } else if let Some(handles) = window {
            #[cfg(target_os = "windows")]
            {
                if let RawWindowHandle::Win32(win32) = handles.window_handle {
                    let hwnd = HWND(win32.hwnd.get() as *mut core::ffi::c_void);
                    let native = self.native.as_ref().ok_or_else(|| {
                        BackendError::Runtime("DX12 native state missing during surface creation".to_string())
                    })?;
                    match Self::create_dx12_surface_state(
                        &native._factory,
                        &native.device,
                        &native.queue,
                        hwnd,
                        config.width.max(1),
                        config.height.max(1),
                        config.vsync,
                    ) {
                        Ok(surface) => {
                            self.surface_state = Some(surface);
                            self.diagnostics.supports_surface = true;
                            self.record_event(
                                BackendDiagnosticLevel::Info,
                                None,
                                None,
                                "DX12 native Win32 swapchain and RTV heap created",
                            );
                        }
                        Err(err) => {
                            self.record_event(
                                BackendDiagnosticLevel::Error,
                                None,
                                None,
                                format!("DX12 swapchain creation failed: {err}"),
                            );
                            return Err(err);
                        }
                    }
                } else {
                    self.record_event(
                        BackendDiagnosticLevel::Warning,
                        None,
                        None,
                        "DX12 received non-Win32 window handle; using compatibility headless mode",
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
                    "DX12 window handles ignored on non-Windows build",
                );
            }
        } else {
            self.record_event(
                BackendDiagnosticLevel::Warning,
                None,
                None,
                "No native window handles provided; DX12 running in headless submit mode",
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
                    if let Err(err) = Self::resize_dx12_surface(
                        &native.device,
                        surface_state,
                        width.max(1),
                        height.max(1),
                    ) {
                        self.record_event(
                            BackendDiagnosticLevel::Error,
                            None,
                            None,
                            format!("DX12 resize failed: {err}"),
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
                format!("DX12 resized swapchain/RTVs to {width}x{height}"),
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
            gpu_nodes: true,
            hybrid_nodes: true,
            compute_nodes: true,
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
                "dx12 backend not initialized".to_string(),
            ));
        }

        let handle = TextureHandle(self.allocate_handle());

        #[cfg(target_os = "windows")]
        {
            let native = self.native.as_ref().ok_or_else(|| {
                BackendError::Runtime("dx12 native state missing during texture creation".to_string())
            })?;
            let renderer = self.renderer.as_ref().ok_or_else(|| {
                BackendError::Runtime("dx12 renderer missing during texture creation".to_string())
            })?;
            let texture = Self::create_dx12_texture(
                &native.device,
                _descriptor.width.max(1),
                _descriptor.height.max(1),
            )?;
            let srv_index = renderer
                .allocate_descriptor_index()
                .ok_or_else(|| BackendError::Runtime("descriptor heap exhausted".to_string()))?;
            let srv = renderer.cpu_handle(srv_index);
            unsafe {
                native.device.CreateShaderResourceView(
                    &texture,
                    None,
                    srv,
                );
            }
            self.textures.insert(handle, Dx12Texture { texture, srv });
        }

        Ok(handle)
    }

    fn create_render_target(
        &mut self,
        _descriptor: RenderTargetDescriptor,
    ) -> Result<RenderTargetHandle, BackendError> {
        if !self.initialized {
            return Err(BackendError::Runtime(
                "dx12 backend not initialized".to_string(),
            ));
        }

        let handle = RenderTargetHandle(self.allocate_handle());

        #[cfg(target_os = "windows")]
        {
            let native = self.native.as_ref().ok_or_else(|| {
                BackendError::Runtime("dx12 native state missing during render target creation".to_string())
            })?;
            let renderer = self.renderer.as_ref().ok_or_else(|| {
                BackendError::Runtime("dx12 renderer missing during render target creation".to_string())
            })?;
            let (texture, rtv, srv) = Self::create_dx12_render_target(
                &native.device,
                renderer,
                _descriptor.width.max(1),
                _descriptor.height.max(1),
            )?;
            self.render_targets.insert(
                handle,
                Dx12RenderTarget {
                    texture,
                    rtv,
                    srv,
                    width: _descriptor.width.max(1),
                    height: _descriptor.height.max(1),
                },
            );
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
            let state = self.native_mut()?;
            Self::wait_for_fence(state, state.fence_value)?;
            unsafe {
                state.allocator.Reset().map_err(|err| {
                    BackendError::Runtime(format!("allocator reset failed: {err}"))
                })?;
                state
                    .command_list
                    .Reset(&state.allocator, None)
                    .map_err(|err| {
                        BackendError::Runtime(format!("command list reset failed: {err}"))
                    })?;
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
                    let shader = node
                        .material
                        .as_ref()
                        .map(|m| m.shader_asset.as_str())
                        .unwrap_or("builtin");
                    (
                        node.label.clone(),
                        format!(
                            "render pass with {} batches and {} sprites (shader {})",
                            node.batches.len(),
                            sprite_count,
                            shader
                        ),
                    )
                }
                RenderGraphPass::Compute(node) => (
                    node.label.clone(),
                    format!(
                        "compute dispatch {}x{}x{} (shader {})",
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
                Some(label),
                format!("DX12 recorded {detail}"),
            );
            self.diagnostics.push_pass_timing(BackendPassTiming {
                frame: frame.0,
                pass: match pass {
                    RenderGraphPass::Render(node) => node.label.clone(),
                    RenderGraphPass::Compute(node) => node.label.clone(),
                },
                cpu_ms: pass_start.elapsed().as_secs_f32() * 1000.0,
                gpu_ms: None,
            });
        }

        #[cfg(target_os = "windows")]
        {
            self.ensure_graph_resources(graph)?;

            if let Some(surface_state) = self.surface_state.as_ref() {
                let native = self.native.as_ref().ok_or_else(|| {
                    BackendError::Runtime("dx12 native state missing during recording".to_string())
                })?;
                let renderer = self.renderer.as_mut().ok_or_else(|| {
                    BackendError::Runtime("dx12 renderer missing during recording".to_string())
                })?;
                let textures = &self.textures;
                let render_targets = &self.render_targets;
                let storage_buffers = &self.storage_buffers;
                renderer.record_frame(
                    &native.command_list,
                    surface_state,
                    graph,
                    textures,
                    render_targets,
                    storage_buffers,
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
            let state = self.native_mut()?;

            unsafe {
                state.command_list.Close().map_err(|err| {
                    BackendError::Runtime(format!("command list close failed: {err}"))
                })?;

                let command_list: ID3D12CommandList = state.command_list.cast().map_err(|err| {
                    BackendError::Runtime(format!("command list cast failed: {err}"))
                })?;

                state.queue.ExecuteCommandLists(&[Some(command_list)]);
                state.fence_value += 1;
                state
                    .queue
                    .Signal(&state.fence, state.fence_value)
                    .map_err(|err| BackendError::Runtime(format!("queue signal failed: {err}")))?;
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

        #[cfg(target_os = "windows")]
        {
            let state = self.native_mut()?;
            Self::wait_for_fence(state, state.fence_value)?;

            if let Some(surface_state) = self.surface_state.as_ref() {
                let interval = if surface_state.vsync { 1 } else { 0 };
                unsafe {
                    surface_state
                        .swapchain
                        .Present(interval, DXGI_PRESENT(0))
                        .map_err(|err| BackendError::Surface(format!("Present failed: {err}")))?;
                }
            }
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
        if self.last_viewport_readback.is_none() {
            if let Some(graph) = self.last_recorded_graph.as_ref() {
                self.last_viewport_readback = self.synthesize_viewport(graph);
            }
        }
        Ok(self.last_viewport_readback.clone())
    }

    fn preload_shader_bytecode(
        &mut self,
        _name: &str,
        _entry_point: &str,
        _bytecode: &[u8],
    ) -> Result<(), BackendError> {
        #[cfg(target_os = "windows")]
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.preload_shader_bytecode(_name, _entry_point, _bytecode);
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

    fn configure_gpu_instrumentation(
        &mut self,
        config: GpuInstrumentationConfig,
    ) {
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

impl Drop for Dx12Backend {
    fn drop(&mut self) {
        if let Err(err) = self.destroy_internal() {
            tracing::warn!("dx12 backend drop cleanup failed: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_dx12() {
        let backend = Dx12Backend::new();
        assert_eq!(backend.kind(), BackendKind::Dx12);
    }

    #[test]
    fn capabilities_claim_viewport_readback() {
        let backend = Dx12Backend::new();
        assert!(backend.capabilities().viewport_readback);
        assert!(backend.capabilities().compute_nodes);
    }

    #[test]
    fn diagnostics_new_has_zero_counts() {
        let backend = Dx12Backend::new();
        let diag = backend.diagnostics();
        assert_eq!(diag.backend, BackendKind::Dx12);
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
        let mut backend = Dx12Backend::new();
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

        backend.instrumentation.complete_oldest_capture(2, 2, vec![0; 16]);
        let completed = backend
            .poll_frame_capture(handle)
            .expect("poll should return completed capture");
        assert!(completed.completed);
        assert_eq!(completed.data, Some(vec![0; 16]));
    }

    #[test]
    fn gpu_timestamps_enable_updates_diagnostics() {
        let mut backend = Dx12Backend::new();
        backend.set_gpu_timestamps_enabled(true);
        assert!(backend.diagnostics().gpu_timestamps_enabled);
    }
}
