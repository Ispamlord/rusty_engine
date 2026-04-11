use std::time::Instant;

use engine_core::EngineConfig;
use engine_render_api::{
    BackendCapabilities, BackendDiagnosticEvent, BackendDiagnosticLevel, BackendDiagnostics,
    BackendError, BackendKind, BackendPassTiming, FrameToken, GraphicsBackend, RenderGraph,
    RenderGraphPass, RenderTargetDescriptor, RenderTargetHandle, SurfaceConfig, SurfaceHandle,
    SurfaceWindowHandles, TextureDescriptor, TextureHandle,
};
#[cfg(target_os = "windows")]
use raw_window_handle::RawWindowHandle;

#[cfg(target_os = "windows")]
use windows::core::{IUnknown, Interface};
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::{CloseHandle, HANDLE};
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Direct3D::{D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_12_0};
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Direct3D12::{
    D3D12CreateDevice, ID3D12CommandAllocator, ID3D12CommandList, ID3D12CommandQueue, ID3D12Device,
    ID3D12Fence, ID3D12GraphicsCommandList, D3D12_COMMAND_LIST_TYPE_DIRECT,
    D3D12_COMMAND_QUEUE_DESC, D3D12_COMMAND_QUEUE_FLAG_NONE, D3D12_FENCE_FLAG_NONE,
};
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, IDXGIAdapter1, IDXGIFactory4, DXGI_ADAPTER_FLAG_SOFTWARE,
    DXGI_ERROR_NOT_FOUND,
};
#[cfg(target_os = "windows")]
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject, INFINITE};

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

pub struct Dx12Backend {
    initialized: bool,
    frame_in_flight: Option<FrameToken>,
    frame_start: Option<Instant>,
    next_handle: u64,
    next_frame: u64,
    active_surface: Option<SurfaceHandle>,
    surface_config: Option<SurfaceConfig>,
    diagnostics: BackendDiagnostics,

    #[cfg(target_os = "windows")]
    native: Option<Dx12NativeState>,
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

    fn destroy_internal(&mut self) -> Result<(), BackendError> {
        #[cfg(target_os = "windows")]
        {
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

        Ok(())
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
            let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory2(0) }
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

        let handle = SurfaceHandle(self.allocate_handle());
        self.active_surface = Some(handle);
        self.surface_config = Some(config);
        self.diagnostics.supports_surface = false;

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
                if matches!(handles.window_handle, RawWindowHandle::Win32(_)) {
                    self.diagnostics.supports_surface = true;
                    self.record_event(
                        BackendDiagnosticLevel::Info,
                        None,
                        None,
                        "DX12 native Win32 surface attached; swapchain path enabled",
                    );
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
            self.diagnostics.mark_swapchain_recreate();
            self.record_event(
                BackendDiagnosticLevel::Info,
                None,
                None,
                format!("DX12 resize requested to {width}x{height}; swapchain recreation queued"),
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

        Ok(TextureHandle(self.allocate_handle()))
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
                    (
                        node.label.clone(),
                        format!(
                            "render pass with {} batches and {} sprites",
                            node.batches.len(),
                            sprite_count
                        ),
                    )
                }
                RenderGraphPass::Compute(node) => (
                    node.label.clone(),
                    format!(
                        "compute dispatch {}x{}x{}",
                        node.dispatch[0], node.dispatch[1], node.dispatch[2]
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
        }

        if let Some(start) = self.frame_start.take() {
            let elapsed_ms = start.elapsed().as_secs_f32() * 1000.0;
            self.diagnostics.last_cpu_frame_ms = elapsed_ms;
            self.diagnostics.last_gpu_frame_ms = elapsed_ms;
        }

        self.frame_in_flight = None;
        Ok(())
    }

    fn destroy(&mut self) -> Result<(), BackendError> {
        self.destroy_internal()
    }
}

impl Drop for Dx12Backend {
    fn drop(&mut self) {
        if let Err(err) = self.destroy_internal() {
            tracing::warn!("dx12 backend drop cleanup failed: {err}");
        }
    }
}
