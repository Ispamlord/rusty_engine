use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::time::Instant;

mod renderer;

use ash::vk;
use ash_window::create_surface as create_vulkan_surface;
use engine_core::EngineConfig;
use engine_render_api::{
    BackendCapabilities, BackendDiagnosticEvent, BackendDiagnosticLevel, BackendDiagnostics,
    BackendError, BackendInstrumentationState, BackendKind, BackendPassTiming, FrameCaptureHandle,
    FrameCaptureRequest, FrameCaptureResult, FrameToken, GraphResourceKind,
    GpuInstrumentationConfig, GraphicsBackend, RenderGraph, RenderGraphPass, RenderTargetDescriptor,
    RenderTargetHandle, SurfaceConfig, SurfaceHandle, SurfaceWindowHandles, TextureDescriptor,
    TextureHandle, ViewportReadback,
};
use renderer::VulkanRenderer;

#[cfg(test)]
use engine_render_api::FrameCaptureFormat;

pub(crate) struct VulkanTexture {
    pub image: vk::Image,
    pub memory: vk::DeviceMemory,
    pub view: vk::ImageView,
    pub sampler: vk::Sampler,
}

impl VulkanTexture {
    pub(crate) unsafe fn destroy(&self, device: &ash::Device) {
        device.destroy_sampler(self.sampler, None);
        device.destroy_image_view(self.view, None);
        device.destroy_image(self.image, None);
        device.free_memory(self.memory, None);
    }
}

pub(crate) struct VulkanRenderTarget {
    pub image: vk::Image,
    pub memory: vk::DeviceMemory,
    pub view: vk::ImageView,
    pub width: u32,
    pub height: u32,
}

impl VulkanRenderTarget {
    pub(crate) unsafe fn destroy(&self, device: &ash::Device) {
        device.destroy_image_view(self.view, None);
        device.destroy_image(self.image, None);
        device.free_memory(self.memory, None);
    }
}

pub(crate) struct VulkanStorageBuffer {
    pub buffer: vk::Buffer,
    pub memory: vk::DeviceMemory,
    pub size: vk::DeviceSize,
}

impl VulkanStorageBuffer {
    pub(crate) unsafe fn destroy(&self, device: &ash::Device) {
        device.destroy_buffer(self.buffer, None);
        device.free_memory(self.memory, None);
    }
}

struct VulkanSurfaceState {
    surface_loader: ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
    swapchain_loader: ash::khr::swapchain::Device,
    swapchain: vk::SwapchainKHR,
    images: Vec<vk::Image>,
    image_available: vk::Semaphore,
    render_finished: vk::Semaphore,
    acquired_image_index: Option<u32>,
    format: vk::SurfaceFormatKHR,
    extent: vk::Extent2D,
}

pub struct VulkanBackend {
    initialized: bool,
    next_handle: u64,
    next_frame: u64,
    active_surface: Option<SurfaceHandle>,
    surface_config: Option<SurfaceConfig>,
    frame_in_flight: Option<FrameToken>,
    frame_start: Option<Instant>,
    diagnostics: BackendDiagnostics,
    instrumentation: BackendInstrumentationState,

    entry: Option<ash::Entry>,
    instance: Option<ash::Instance>,
    physical_device: Option<vk::PhysicalDevice>,
    device: Option<ash::Device>,
    queue_family_index: Option<u32>,
    graphics_queue: Option<vk::Queue>,
    command_pool: Option<vk::CommandPool>,
    command_buffer: Option<vk::CommandBuffer>,
    submit_fence: Option<vk::Fence>,
    surface_state: Option<VulkanSurfaceState>,
    surface_window_handles: Option<SurfaceWindowHandles>,
    renderer: Option<VulkanRenderer>,
    textures: HashMap<TextureHandle, VulkanTexture>,
    render_targets: HashMap<RenderTargetHandle, VulkanRenderTarget>,
    storage_buffers: HashMap<String, VulkanStorageBuffer>,
    last_recorded_graph: Option<RenderGraph>,
    last_viewport_readback: Option<ViewportReadback>,
}

impl Default for VulkanBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl VulkanBackend {
    pub fn new() -> Self {
        Self {
            initialized: false,
            next_handle: 1,
            next_frame: 1,
            active_surface: None,
            surface_config: None,
            frame_in_flight: None,
            frame_start: None,
            diagnostics: BackendDiagnostics::new(BackendKind::Vulkan),
            instrumentation: BackendInstrumentationState::default(),
            entry: None,
            instance: None,
            physical_device: None,
            device: None,
            queue_family_index: None,
            graphics_queue: None,
            command_pool: None,
            command_buffer: None,
            submit_fence: None,
            surface_state: None,
            surface_window_handles: None,
            renderer: None,
            textures: HashMap::new(),
            render_targets: HashMap::new(),
            storage_buffers: HashMap::new(),
            last_recorded_graph: None,
            last_viewport_readback: None,
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

    fn ensure_graph_resources(
        &mut self,
        graph: &RenderGraph,
    ) -> Result<(), BackendError> {
        let device = self.device_ref()?.clone();
        let physical_device = self.physical_device.ok_or_else(|| {
            BackendError::Runtime("vulkan physical device missing".to_string())
        })?;
        let instance = self.instance.as_ref().ok_or_else(|| {
            BackendError::Runtime("vulkan instance missing".to_string())
        })?;

        for resource in &graph.resources {
            let GraphResourceKind::StorageBuffer { size_bytes } = resource.kind else {
                continue;
            };
            if self.storage_buffers.contains_key(&resource.name) {
                continue;
            }
            let size = size_bytes as vk::DeviceSize;
            let (buffer, memory) =
                renderer::create_storage_buffer(&device, physical_device, instance, size)?;
            self.storage_buffers.insert(
                resource.name.clone(),
                VulkanStorageBuffer {
                    buffer,
                    memory,
                    size,
                },
            );
        }

        Ok(())
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

    fn destroy_surface_state(&mut self) -> Result<(), BackendError> {
        let Some(surface_state) = self.surface_state.take() else {
            return Ok(());
        };

        let device = self.device_ref()?;

        unsafe {
            device
                .device_wait_idle()
                .map_err(|err| BackendError::Runtime(format!("vkDeviceWaitIdle failed: {err}")))?;
            device.destroy_semaphore(surface_state.image_available, None);
            device.destroy_semaphore(surface_state.render_finished, None);
            surface_state
                .swapchain_loader
                .destroy_swapchain(surface_state.swapchain, None);
            surface_state
                .surface_loader
                .destroy_surface(surface_state.surface, None);
        }

        Ok(())
    }

    fn destroy_internal(&mut self) -> Result<(), BackendError> {
        if !self.initialized {
            return Ok(());
        }

        self.destroy_surface_state()?;

        let device = self.device.as_ref().ok_or_else(|| {
            BackendError::Runtime("vulkan device missing during destroy".to_string())
        })?;

        unsafe {
            device
                .device_wait_idle()
                .map_err(|err| BackendError::Runtime(format!("vkDeviceWaitIdle failed: {err}")))?;
        }

        if let Some(device) = self.device.clone() {
            if let Some(mut renderer) = self.renderer.take() {
                renderer.destroy(&device);
            }
        }

        for (_handle, texture) in self.textures.drain() {
            unsafe {
                texture.destroy(device);
            }
        }

        for (_handle, target) in self.render_targets.drain() {
            unsafe {
                target.destroy(device);
            }
        }

        for (_name, buffer) in self.storage_buffers.drain() {
            unsafe {
                buffer.destroy(device);
            }
        }

        if let Some(fence) = self.submit_fence.take() {
            unsafe {
                device.destroy_fence(fence, None);
            }
        }

        if let Some(pool) = self.command_pool.take() {
            unsafe {
                device.destroy_command_pool(pool, None);
            }
        }

        if let Some(device) = self.device.take() {
            unsafe {
                device.destroy_device(None);
            }
        }

        if let Some(instance) = self.instance.take() {
            unsafe {
                instance.destroy_instance(None);
            }
        }

        self.entry = None;
        self.initialized = false;
        self.active_surface = None;
        self.surface_config = None;
        self.frame_in_flight = None;
        self.frame_start = None;
        self.graphics_queue = None;
        self.queue_family_index = None;
        self.command_buffer = None;
        self.submit_fence = None;
        self.physical_device = None;
        self.surface_window_handles = None;
        self.last_recorded_graph = None;
        self.last_viewport_readback = None;

        Ok(())
    }

    fn device_ref(&self) -> Result<&ash::Device, BackendError> {
        self.device
            .as_ref()
            .ok_or_else(|| BackendError::Runtime("vulkan device is not initialized".to_string()))
    }

    fn queue_ref(&self) -> Result<vk::Queue, BackendError> {
        self.graphics_queue.ok_or_else(|| {
            BackendError::Runtime("vulkan graphics queue is not initialized".to_string())
        })
    }

    fn command_buffer_ref(&self) -> Result<vk::CommandBuffer, BackendError> {
        self.command_buffer.ok_or_else(|| {
            BackendError::Runtime("vulkan command buffer is not initialized".to_string())
        })
    }

    fn pick_surface_format(
        formats: &[vk::SurfaceFormatKHR],
    ) -> Result<vk::SurfaceFormatKHR, BackendError> {
        formats
            .iter()
            .copied()
            .find(|format| {
                format.format == vk::Format::B8G8R8A8_UNORM
                    && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
            })
            .or_else(|| formats.first().copied())
            .ok_or_else(|| BackendError::Surface("no supported surface format".to_string()))
    }

    fn pick_present_mode(vsync: bool, modes: &[vk::PresentModeKHR]) -> vk::PresentModeKHR {
        if vsync {
            return vk::PresentModeKHR::FIFO;
        }

        if modes.contains(&vk::PresentModeKHR::MAILBOX) {
            vk::PresentModeKHR::MAILBOX
        } else if modes.contains(&vk::PresentModeKHR::IMMEDIATE) {
            vk::PresentModeKHR::IMMEDIATE
        } else {
            vk::PresentModeKHR::FIFO
        }
    }

    fn choose_extent(
        capabilities: &vk::SurfaceCapabilitiesKHR,
        width: u32,
        height: u32,
    ) -> vk::Extent2D {
        if capabilities.current_extent.width != u32::MAX {
            capabilities.current_extent
        } else {
            vk::Extent2D {
                width: width
                    .max(capabilities.min_image_extent.width)
                    .min(capabilities.max_image_extent.width)
                    .max(1),
                height: height
                    .max(capabilities.min_image_extent.height)
                    .min(capabilities.max_image_extent.height)
                    .max(1),
            }
        }
    }

    fn create_surface_state(
        &mut self,
        config: SurfaceConfig,
        window: SurfaceWindowHandles,
    ) -> Result<VulkanSurfaceState, BackendError> {
        let entry = self
            .entry
            .as_ref()
            .ok_or_else(|| BackendError::Runtime("vulkan entry missing".to_string()))?;
        let instance = self
            .instance
            .clone()
            .ok_or_else(|| BackendError::Runtime("vulkan instance missing".to_string()))?;
        let device = self
            .device
            .clone()
            .ok_or_else(|| BackendError::Runtime("vulkan device missing".to_string()))?;
        let physical_device = self
            .physical_device
            .ok_or_else(|| BackendError::Runtime("physical device missing".to_string()))?;
        let queue_family_index = self
            .queue_family_index
            .ok_or_else(|| BackendError::Runtime("queue family index missing".to_string()))?;

        let surface_loader = ash::khr::surface::Instance::new(entry, &instance);
        let surface = unsafe {
            create_vulkan_surface(
                entry,
                &instance,
                window.display_handle,
                window.window_handle,
                None,
            )
            .map_err(|err| BackendError::Surface(format!("create Vulkan surface failed: {err}")))?
        };

        let present_supported = unsafe {
            surface_loader
                .get_physical_device_surface_support(physical_device, queue_family_index, surface)
                .map_err(|err| {
                    BackendError::Surface(format!(
                        "query physical device surface support failed: {err}"
                    ))
                })?
        };

        if !present_supported {
            unsafe {
                surface_loader.destroy_surface(surface, None);
            }
            return Err(BackendError::Surface(
                "selected Vulkan queue family does not support present for this surface"
                    .to_string(),
            ));
        }

        let capabilities = unsafe {
            surface_loader
                .get_physical_device_surface_capabilities(physical_device, surface)
                .map_err(|err| {
                    BackendError::Surface(format!(
                        "query physical device surface capabilities failed: {err}"
                    ))
                })?
        };

        let formats = unsafe {
            surface_loader
                .get_physical_device_surface_formats(physical_device, surface)
                .map_err(|err| {
                    BackendError::Surface(format!(
                        "query physical device surface formats failed: {err}"
                    ))
                })?
        };

        let present_modes = unsafe {
            surface_loader
                .get_physical_device_surface_present_modes(physical_device, surface)
                .map_err(|err| {
                    BackendError::Surface(format!(
                        "query physical device present modes failed: {err}"
                    ))
                })?
        };

        let format = Self::pick_surface_format(&formats)?;
        let present_mode = Self::pick_present_mode(config.vsync, &present_modes);
        let extent = Self::choose_extent(&capabilities, config.width, config.height);

        let mut image_count = capabilities.min_image_count.saturating_add(1);
        if capabilities.max_image_count > 0 {
            image_count = image_count.min(capabilities.max_image_count);
        }

        let swapchain_loader = ash::khr::swapchain::Device::new(&instance, &device);
        let swapchain_info = vk::SwapchainCreateInfoKHR::default()
            .surface(surface)
            .min_image_count(image_count)
            .image_color_space(format.color_space)
            .image_format(format.format)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(capabilities.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true);

        let swapchain = unsafe {
            swapchain_loader
                .create_swapchain(&swapchain_info, None)
                .map_err(|err| {
                    BackendError::Surface(format!("vkCreateSwapchainKHR failed: {err}"))
                })?
        };

        let images = unsafe {
            swapchain_loader
                .get_swapchain_images(swapchain)
                .map_err(|err| {
                    BackendError::Surface(format!("get swapchain images failed: {err}"))
                })?
        };

        let image_available = unsafe {
            device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                .map_err(|err| {
                    BackendError::Surface(format!("create image semaphore failed: {err}"))
                })?
        };

        let render_finished = unsafe {
            device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                .map_err(|err| {
                    BackendError::Surface(format!("create render semaphore failed: {err}"))
                })?
        };

        let command_pool = self.command_pool.ok_or_else(|| {
            BackendError::Runtime("command pool missing during surface creation".to_string())
        })?;
        let queue = self.queue_ref()?;

        if self.renderer.is_none() {
            self.renderer = Some(VulkanRenderer::new(
                &device,
                physical_device,
                &instance,
                format,
                command_pool,
                queue_family_index,
                queue,
            )?);
        }

        if let Some(renderer) = self.renderer.as_mut() {
            renderer.ensure_swapchain_views(&device, &images, extent)?;
        }

        Ok(VulkanSurfaceState {
            surface_loader,
            surface,
            swapchain_loader,
            swapchain,
            images,
            image_available,
            render_finished,
            acquired_image_index: None,
            format,
            extent,
        })
    }

    fn update_frame_time(&mut self) {
        if let Some(start) = self.frame_start.take() {
            let elapsed_ms = start.elapsed().as_secs_f32() * 1000.0;
            self.diagnostics.last_cpu_frame_ms = elapsed_ms;
            self.diagnostics.last_gpu_frame_ms = elapsed_ms;
        }
    }

    fn collect_instance_extensions(entry: &ash::Entry) -> Result<Vec<*const i8>, BackendError> {
        let available = unsafe {
            entry
                .enumerate_instance_extension_properties(None)
                .map_err(|err| {
                    BackendError::Init(format!("enumerate instance extensions failed: {err}"))
                })?
        };

        let mut available_names = std::collections::HashSet::new();
        for ext in available {
            let name = unsafe { CStr::from_ptr(ext.extension_name.as_ptr()) }
                .to_string_lossy()
                .to_string();
            available_names.insert(name);
        }

        let mut enabled = Vec::new();
        let mut maybe_enable = |name: &CStr| {
            if available_names.contains(name.to_string_lossy().as_ref()) {
                enabled.push(name.as_ptr());
            }
        };

        maybe_enable(ash::khr::surface::NAME);

        #[cfg(target_os = "linux")]
        {
            maybe_enable(ash::khr::xlib_surface::NAME);
            maybe_enable(ash::khr::xcb_surface::NAME);
            maybe_enable(ash::khr::wayland_surface::NAME);
        }

        #[cfg(target_os = "windows")]
        {
            maybe_enable(ash::khr::win32_surface::NAME);
        }

        Ok(enabled)
    }

    fn collect_device_extensions(
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
    ) -> Result<Vec<*const i8>, BackendError> {
        let available = unsafe {
            instance
                .enumerate_device_extension_properties(physical_device)
                .map_err(|err| {
                    BackendError::Init(format!("enumerate device extensions failed: {err}"))
                })?
        };

        let mut has_swapchain = false;
        for ext in available {
            let name = unsafe { CStr::from_ptr(ext.extension_name.as_ptr()) }.to_string_lossy();
            if name == ash::khr::swapchain::NAME.to_string_lossy() {
                has_swapchain = true;
                break;
            }
        }

        if has_swapchain {
            Ok(vec![ash::khr::swapchain::NAME.as_ptr()])
        } else {
            Ok(Vec::new())
        }
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
                rgba[idx] = 10;
                rgba[idx + 1] = 24;
                rgba[idx + 2] = 20;
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

impl GraphicsBackend for VulkanBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Vulkan
    }

    fn initialize(&mut self, _config: &EngineConfig) -> Result<(), BackendError> {
        if self.initialized {
            return Ok(());
        }

        let app_name = CString::new("rusty_engine")
            .map_err(|err| BackendError::Init(format!("invalid app name: {err}")))?;
        let engine_name = CString::new("rusty_engine")
            .map_err(|err| BackendError::Init(format!("invalid engine name: {err}")))?;

        let entry = unsafe {
            ash::Entry::load()
                .map_err(|err| BackendError::Init(format!("failed to load Vulkan entry: {err}")))?
        };

        let instance_extensions = Self::collect_instance_extensions(&entry)?;

        let app_info = vk::ApplicationInfo::default()
            .application_name(app_name.as_c_str())
            .application_version(0)
            .engine_name(engine_name.as_c_str())
            .engine_version(0)
            .api_version(vk::make_api_version(0, 1, 0, 0));

        let instance_create_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(&instance_extensions);

        let instance = unsafe {
            entry
                .create_instance(&instance_create_info, None)
                .map_err(|err| BackendError::Init(format!("vkCreateInstance failed: {err}")))?
        };

        let physical_devices = unsafe {
            instance.enumerate_physical_devices().map_err(|err| {
                BackendError::Init(format!("enumerate physical devices failed: {err}"))
            })?
        };

        let Some((physical_device, queue_family_index)) =
            physical_devices.iter().find_map(|device| {
                let families =
                    unsafe { instance.get_physical_device_queue_family_properties(*device) };
                families
                    .iter()
                    .enumerate()
                    .find(|(_, family)| family.queue_flags.contains(vk::QueueFlags::GRAPHICS))
                    .map(|(index, _)| (*device, index as u32))
            })
        else {
            return Err(BackendError::Init(
                "no Vulkan physical device with graphics queue was found".to_string(),
            ));
        };

        let device_extensions = Self::collect_device_extensions(&instance, physical_device)?;

        let priorities = [1.0_f32];
        let queue_create_info = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family_index)
            .queue_priorities(&priorities)];
        let device_create_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_create_info)
            .enabled_extension_names(&device_extensions);

        let device = unsafe {
            instance
                .create_device(physical_device, &device_create_info, None)
                .map_err(|err| BackendError::Init(format!("vkCreateDevice failed: {err}")))?
        };

        let graphics_queue = unsafe { device.get_device_queue(queue_family_index, 0) };

        let command_pool = unsafe {
            device
                .create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(queue_family_index)
                        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                    None,
                )
                .map_err(|err| BackendError::Init(format!("vkCreateCommandPool failed: {err}")))?
        };

        let command_buffer = unsafe {
            device
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(command_pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
                .map_err(|err| {
                    BackendError::Init(format!("vkAllocateCommandBuffers failed: {err}"))
                })?
                .into_iter()
                .next()
                .ok_or_else(|| {
                    BackendError::Init("vkAllocateCommandBuffers returned no buffers".to_string())
                })?
        };

        let submit_fence = unsafe {
            device
                .create_fence(
                    &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                    None,
                )
                .map_err(|err| BackendError::Init(format!("vkCreateFence failed: {err}")))?
        };

        self.entry = Some(entry);
        self.instance = Some(instance);
        self.physical_device = Some(physical_device);
        self.queue_family_index = Some(queue_family_index);
        self.graphics_queue = Some(graphics_queue);
        self.command_pool = Some(command_pool);
        self.command_buffer = Some(command_buffer);
        self.submit_fence = Some(submit_fence);
        self.device = Some(device);
        self.initialized = true;

        self.record_event(
            BackendDiagnosticLevel::Info,
            None,
            None,
            "Vulkan initialized with graphics queue and command submission objects",
        );

        Ok(())
    }

    fn preload_shader_bytecode(
        &mut self,
        name: &str,
        entry_point: &str,
        bytecode: &[u8],
    ) -> Result<(), BackendError> {
        if !self.initialized {
            return Err(BackendError::Runtime(
                "vulkan backend not initialized".to_string(),
            ));
        }

        if self.renderer.is_none() {
            let device = self.device_ref()?.clone();
            let physical_device = self.physical_device.ok_or_else(|| {
                BackendError::Runtime("physical device missing".to_string())
            })?;
            let instance = self
                .instance
                .as_ref()
                .ok_or_else(|| BackendError::Runtime("vulkan instance missing".to_string()))?
                .clone();
            let command_pool = self.command_pool.ok_or_else(|| {
                BackendError::Runtime("command pool missing".to_string())
            })?;
            let queue_family_index = self.queue_family_index.ok_or_else(|| {
                BackendError::Runtime("queue family index missing".to_string())
            })?;
            let queue = self.queue_ref()?;

            // No surface format is known yet; use a sensible default.  The
            // renderer will be reused when the surface is created.
            let default_format = vk::SurfaceFormatKHR {
                format: vk::Format::B8G8R8A8_UNORM,
                color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
            };

            self.renderer = Some(VulkanRenderer::new(
                &device,
                physical_device,
                &instance,
                default_format,
                command_pool,
                queue_family_index,
                queue,
            )?);
        }

        let device = self.device_ref()?.clone();

        if let Some(renderer) = self.renderer.as_mut() {
            renderer.load_shader_module(&device, name, bytecode)?;
            renderer
                .shader_entry_points
                .insert(name.to_string(), entry_point.to_string());
        }

        Ok(())
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

        self.destroy_surface_state()?;

        let handle = SurfaceHandle(self.allocate_handle());
        self.active_surface = Some(handle);
        self.surface_config = Some(config);
        self.last_recorded_graph = None;
        self.last_viewport_readback = None;

        if config.headless {
            self.surface_window_handles = None;
            self.diagnostics.supports_surface = false;
            self.record_event(
                BackendDiagnosticLevel::Warning,
                None,
                None,
                "Headless Vulkan surface mode enabled; present is a no-op",
            );
            return Ok(handle);
        }

        if let Some(window) = window {
            self.surface_window_handles = Some(window);
            let surface_state = self.create_surface_state(config, window)?;
            self.record_event(
                BackendDiagnosticLevel::Info,
                None,
                None,
                format!(
                    "Vulkan surface/swapchain created: {} images, format {:?}, extent {}x{}",
                    surface_state.images.len(),
                    surface_state.format.format,
                    surface_state.extent.width,
                    surface_state.extent.height
                ),
            );
            self.diagnostics.supports_surface = true;
            self.surface_state = Some(surface_state);
        } else {
            self.surface_window_handles = None;
            self.diagnostics.supports_surface = false;
            self.record_event(
                BackendDiagnosticLevel::Warning,
                None,
                None,
                "No native window handles provided; Vulkan will run in headless submit mode",
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

        if width == 0 || height == 0 {
            return Err(BackendError::Surface(
                "resize dimensions must be greater than zero".to_string(),
            ));
        }

        if let Some(config) = &mut self.surface_config {
            config.width = width;
            config.height = height;
        }

        if let Some(config) = self.surface_config {
            if let Some(window) = self.surface_window_handles {
                self.destroy_surface_state()?;
                let surface_state = self.create_surface_state(config, window)?;
                self.surface_state = Some(surface_state);
                self.diagnostics.mark_swapchain_recreate();
                self.record_event(
                    BackendDiagnosticLevel::Info,
                    None,
                    None,
                    format!("Surface resized to {width}x{height}; swapchain recreated immediately"),
                );
            } else {
                self.record_event(
                    BackendDiagnosticLevel::Info,
                    None,
                    None,
                    format!(
                        "Surface resized to {width}x{height}; no window handle available for recreation"
                    ),
                );
            }
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
        descriptor: TextureDescriptor,
    ) -> Result<TextureHandle, BackendError> {
        if !self.initialized {
            return Err(BackendError::Runtime(
                "vulkan backend not initialized".to_string(),
            ));
        }

        let handle = TextureHandle(self.allocate_handle());

        let device = self.device_ref()?;
        let physical_device = self.physical_device.ok_or_else(|| {
            BackendError::Runtime("vulkan physical device missing".to_string())
        })?;
        let instance = self.instance.as_ref().ok_or_else(|| {
            BackendError::Runtime("vulkan instance missing".to_string())
        })?;
        let queue_family_index = self.queue_family_index.ok_or_else(|| {
            BackendError::Runtime("vulkan queue family missing".to_string())
        })?;
        let command_pool = self.command_pool.ok_or_else(|| {
            BackendError::Runtime("vulkan command pool missing".to_string())
        })?;
        let queue = self.queue_ref()?;

        let texture = renderer::create_texture_image(
            device,
            physical_device,
            instance,
            queue_family_index,
            command_pool,
            queue,
            descriptor.width,
            descriptor.height,
            None,
        )?;
        self.textures.insert(handle, texture);
        Ok(handle)
    }

    fn create_render_target(
        &mut self,
        descriptor: RenderTargetDescriptor,
    ) -> Result<RenderTargetHandle, BackendError> {
        if !self.initialized {
            return Err(BackendError::Runtime(
                "vulkan backend not initialized".to_string(),
            ));
        }

        let handle = RenderTargetHandle(self.allocate_handle());

        let device = self.device_ref()?;
        let physical_device = self.physical_device.ok_or_else(|| {
            BackendError::Runtime("vulkan physical device missing".to_string())
        })?;
        let instance = self.instance.as_ref().ok_or_else(|| {
            BackendError::Runtime("vulkan instance missing".to_string())
        })?;
        let queue_family_index = self.queue_family_index.ok_or_else(|| {
            BackendError::Runtime("vulkan queue family missing".to_string())
        })?;

        let target = renderer::create_render_target_image(
            device,
            physical_device,
            instance,
            queue_family_index,
            descriptor.width,
            descriptor.height,
        )?;
        self.render_targets.insert(handle, target);
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

        let command_pool = self.command_pool.ok_or_else(|| {
            BackendError::Runtime("vulkan command pool is not initialized".to_string())
        })?;
        let command_buffer = self.command_buffer_ref()?;
        let fence = self.submit_fence.ok_or_else(|| {
            BackendError::Runtime("vulkan submit fence is not initialized".to_string())
        })?;

        {
            let device = self.device_ref()?;
            const FRAME_FENCE_TIMEOUT_NS: u64 = 5_000_000_000;
            unsafe {
                match device.wait_for_fences(&[fence], true, FRAME_FENCE_TIMEOUT_NS) {
                    Ok(()) => {}
                    Err(vk::Result::TIMEOUT) => {
                        return Err(BackendError::DeviceLost(
                            "waiting for frame fence timed out".to_string(),
                        ));
                    }
                    Err(err) => {
                        return Err(BackendError::Runtime(format!(
                            "vkWaitForFences failed: {err}"
                        )));
                    }
                }
                device
                    .reset_fences(&[fence])
                    .map_err(|err| BackendError::Runtime(format!("vkResetFences failed: {err}")))?;
                device
                    .reset_command_pool(command_pool, vk::CommandPoolResetFlags::empty())
                    .map_err(|err| {
                        BackendError::Runtime(format!("vkResetCommandPool failed: {err}"))
                    })?;
            }
        }

        let mut acquire_suboptimal = false;
        if let Some(surface_state) = &mut self.surface_state {
            let acquire_result = unsafe {
                surface_state.swapchain_loader.acquire_next_image(
                    surface_state.swapchain,
                    u64::MAX,
                    surface_state.image_available,
                    vk::Fence::null(),
                )
            };

            match acquire_result {
                Ok((image_index, suboptimal)) => {
                    surface_state.acquired_image_index = Some(image_index);
                    acquire_suboptimal = suboptimal;
                }
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                    return Err(BackendError::SurfaceOutOfDate(
                        "Vulkan swapchain out of date during acquire".to_string(),
                    ));
                }
                Err(err) => {
                    return Err(BackendError::Runtime(format!(
                        "vkAcquireNextImageKHR failed: {err}"
                    )));
                }
            }
        }

        if acquire_suboptimal {
            self.record_event(
                BackendDiagnosticLevel::Warning,
                None,
                None,
                "Vulkan swapchain returned SUBOPTIMAL on acquire",
            );
        }

        {
            let device = self.device_ref()?;
            unsafe {
                device
                    .begin_command_buffer(
                        command_buffer,
                        &vk::CommandBufferBeginInfo::default()
                            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                    )
                    .map_err(|err| {
                        BackendError::Runtime(format!("vkBeginCommandBuffer failed: {err}"))
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

            let cpu_ms = pass_start.elapsed().as_secs_f32() * 1000.0;
            self.record_event(
                BackendDiagnosticLevel::Info,
                Some(frame.0),
                Some(label.clone()),
                format!("Recorded {detail}"),
            );
            self.diagnostics.push_pass_timing(BackendPassTiming {
                frame: frame.0,
                pass: label,
                cpu_ms,
                gpu_ms: None,
            });
        }

        let command_buffer = self.command_buffer_ref()?;
        let device = self
            .device
            .clone()
            .ok_or_else(|| BackendError::Runtime("vulkan device missing".to_string()))?;
        let surface_info = self
            .surface_state
            .as_ref()
            .and_then(|s| s.acquired_image_index.map(|i| (i, s.extent)));

        if let Some(renderer) = self.renderer.as_mut() {
            if let Some((image_index, extent)) = surface_info {
                renderer.record_render_graph(
                    &device,
                    command_buffer,
                    graph,
                    &self.textures,
                    &self.render_targets,
                    &self.storage_buffers,
                    image_index,
                    extent,
                    [0.04, 0.09, 0.08, 1.0],
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

        let device = self.device_ref()?;
        let queue = self.queue_ref()?;
        let command_buffer = self.command_buffer_ref()?;
        let fence = self.submit_fence.ok_or_else(|| {
            BackendError::Runtime("vulkan submit fence is not initialized".to_string())
        })?;

        unsafe {
            device.end_command_buffer(command_buffer).map_err(|err| {
                BackendError::Runtime(format!("vkEndCommandBuffer failed: {err}"))
            })?;
        }

        let command_buffers = [command_buffer];
        let mut wait_semaphores = Vec::new();
        let mut wait_stages = Vec::new();
        let mut signal_semaphores = Vec::new();

        if let Some(surface_state) = &self.surface_state {
            wait_semaphores.push(surface_state.image_available);
            wait_stages.push(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT);
            signal_semaphores.push(surface_state.render_finished);
        }

        let submit_info = [vk::SubmitInfo::default()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&command_buffers)
            .signal_semaphores(&signal_semaphores)];

        unsafe {
            device
                .queue_submit(queue, &submit_info, fence)
                .map_err(|err| BackendError::Runtime(format!("vkQueueSubmit failed: {err}")))?;
        }

        Ok(())
    }

    fn present(&mut self, frame: FrameToken) -> Result<(), BackendError> {
        if self.frame_in_flight != Some(frame) {
            return Err(BackendError::Command(
                "present called with stale frame token".to_string(),
            ));
        }

        let queue = self.queue_ref()?;
        let mut present_suboptimal = false;
        if let Some(surface_state) = &mut self.surface_state {
            let image_index = surface_state.acquired_image_index.take().ok_or_else(|| {
                BackendError::Runtime(
                    "present called without an acquired swapchain image".to_string(),
                )
            })?;

            let wait_semaphores = [surface_state.render_finished];
            let swapchains = [surface_state.swapchain];
            let image_indices = [image_index];
            let present_info = vk::PresentInfoKHR::default()
                .wait_semaphores(&wait_semaphores)
                .swapchains(&swapchains)
                .image_indices(&image_indices);

            let present_result = unsafe {
                surface_state
                    .swapchain_loader
                    .queue_present(queue, &present_info)
            };

            match present_result {
                Ok(suboptimal) => {
                    present_suboptimal = suboptimal;
                }
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                    return Err(BackendError::SurfaceOutOfDate(
                        "Vulkan swapchain out of date during present".to_string(),
                    ));
                }
                Err(err) => {
                    return Err(BackendError::Runtime(format!(
                        "vkQueuePresentKHR failed: {err}"
                    )));
                }
            }
        }

        if present_suboptimal {
            self.record_event(
                BackendDiagnosticLevel::Warning,
                Some(frame.0),
                None,
                "Vulkan swapchain returned SUBOPTIMAL on present",
            );
        }

        self.update_frame_time();
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

impl Drop for VulkanBackend {
    fn drop(&mut self) {
        if let Err(err) = self.destroy_internal() {
            tracing::warn!("vulkan backend drop cleanup failed: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_vulkan() {
        let backend = VulkanBackend::new();
        assert_eq!(backend.kind(), BackendKind::Vulkan);
    }

    #[test]
    fn capabilities_claim_viewport_readback() {
        let backend = VulkanBackend::new();
        assert!(backend.capabilities().viewport_readback);
        assert!(backend.capabilities().compute_nodes);
    }

    #[test]
    fn diagnostics_new_has_zero_counts() {
        let backend = VulkanBackend::new();
        let diag = backend.diagnostics();
        assert_eq!(diag.backend, BackendKind::Vulkan);
        assert!(!diag.supports_surface);
        assert_eq!(diag.fallback_events, 0);
        assert_eq!(diag.swapchain_recreates, 0);
        assert_eq!(diag.device_loss_events, 0);
        assert!(!diag.gpu_timestamps_enabled);
        assert_eq!(diag.frame_captures_pending, 0);
        assert_eq!(diag.frame_captures_completed, 0);
        assert!(diag.events.is_empty());
        assert!(diag.pass_timings.is_empty());
    }

    #[test]
    fn frame_capture_request_and_poll() {
        let mut backend = VulkanBackend::new();
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
        assert_eq!(completed.width, 2);
        assert_eq!(completed.height, 2);
    }

    #[test]
    fn gpu_timestamps_enable_updates_diagnostics() {
        let mut backend = VulkanBackend::new();
        backend.set_gpu_timestamps_enabled(true);
        assert!(backend.diagnostics().gpu_timestamps_enabled);
        backend.configure_gpu_instrumentation(GpuInstrumentationConfig {
            enabled: false,
            timestamp_query_period: 0,
        });
        assert!(!backend.diagnostics().gpu_timestamps_enabled);
    }
}
