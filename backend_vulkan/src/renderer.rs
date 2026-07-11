use std::collections::HashMap;
use std::ffi::CString;

use ash::vk;
use engine_render_api::{
    BackendError, Camera2d, ComputeDispatchNode, Material, RenderGraph, RenderGraphPass,
    RenderTargetHandle, SpriteInstance, TextureHandle,
};

/// Vertex layout used by the built-in sprite pipeline.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SpriteVertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
    pub tex_coord: [f32; 2],
}

unsafe impl bytemuck::Pod for SpriteVertex {}
unsafe impl bytemuck::Zeroable for SpriteVertex {}

impl SpriteVertex {
    fn binding_description() -> vk::VertexInputBindingDescription {
        vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(std::mem::size_of::<Self>() as u32)
            .input_rate(vk::VertexInputRate::VERTEX)
    }

    fn attribute_descriptions() -> [vk::VertexInputAttributeDescription; 3] {
        [
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(0)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(0),
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(1)
                .format(vk::Format::R32G32B32A32_SFLOAT)
                .offset(std::mem::size_of::<[f32; 2]>() as u32),
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(std::mem::size_of::<[f32; 6]>() as u32),
        ]
    }
}

/// Maximum number of texture descriptor sets allocated per frame.
pub const MAX_DRAW_DESCRIPTOR_SETS: u32 = 512;

/// GPU-side state needed to record and submit real draw/dispatch work.
pub struct VulkanRenderer {
    pub render_pass: vk::RenderPass,
    pub pipeline_layout: vk::PipelineLayout,
    pub graphics_pipeline: vk::Pipeline,
    pub descriptor_set_layout: vk::DescriptorSetLayout,
    pub graphics_descriptor_pool: vk::DescriptorPool,
    pub compute_descriptor_pool: vk::DescriptorPool,
    pub compute_pipeline_layout: vk::PipelineLayout,
    pub compute_pipeline: vk::Pipeline,
    pub compute_descriptor_set_layout: vk::DescriptorSetLayout,
    pub compute_storage_buffer: vk::Buffer,
    pub compute_storage_buffer_memory: vk::DeviceMemory,
    pub shader_modules: HashMap<String, vk::ShaderModule>,
    pub shader_entry_points: HashMap<String, String>,
    pub compute_pipelines: HashMap<String, vk::Pipeline>,
    pub material_graphics_pipelines: HashMap<String, vk::Pipeline>,
    pub framebuffers: HashMap<u32, vk::Framebuffer>,
    pub image_views: HashMap<u32, vk::ImageView>,
    pub offscreen_render_pass: vk::RenderPass,
    pub offscreen_framebuffers: HashMap<engine_render_api::RenderTargetHandle, vk::Framebuffer>,
    pub vertex_buffer: vk::Buffer,
    pub vertex_buffer_memory: vk::DeviceMemory,
    pub dummy_texture: crate::VulkanTexture,
    pub frame_descriptor_sets: Vec<vk::DescriptorSet>,
}

impl VulkanRenderer {
    pub const MAX_SPRITES: usize = 4096;
    pub const VERTEX_BUFFER_SIZE: vk::DeviceSize =
        (Self::MAX_SPRITES * 6 * std::mem::size_of::<SpriteVertex>()) as vk::DeviceSize;
    pub const COMPUTE_STORAGE_BUFFER_SIZE: vk::DeviceSize = 256;

    pub fn new(
        device: &ash::Device,
        physical_device: vk::PhysicalDevice,
        instance: &ash::Instance,
        surface_format: vk::SurfaceFormatKHR,
        command_pool: vk::CommandPool,
        queue_family_index: u32,
        graphics_queue: vk::Queue,
    ) -> Result<Self, BackendError> {
        let render_pass = create_render_pass(device, surface_format.format)?;
        let offscreen_render_pass = create_offscreen_render_pass(device, vk::Format::R8G8B8A8_UNORM)?;
        let descriptor_set_layout = create_descriptor_set_layout(device)?;
        let pipeline_layout = create_pipeline_layout(device, descriptor_set_layout)?;
        let graphics_pipeline =
            create_graphics_pipeline(device, render_pass, pipeline_layout)?;

        let compute_descriptor_set_layout = create_compute_descriptor_set_layout(device)?;
        let compute_pipeline_layout =
            create_compute_pipeline_layout(device, compute_descriptor_set_layout)?;
        let compute_pipeline = create_compute_pipeline(device, compute_pipeline_layout)?;

        let compute_descriptor_pool = create_compute_descriptor_pool(device)?;
        let graphics_descriptor_pool = create_graphics_descriptor_pool(device)?;

        let (vertex_buffer, vertex_buffer_memory) = create_vertex_buffer(
            device,
            physical_device,
            instance,
            Self::VERTEX_BUFFER_SIZE,
        )?;

        let dummy_texture = create_texture_image(
            device,
            physical_device,
            instance,
            queue_family_index,
            command_pool,
            graphics_queue,
            1,
            1,
            None,
        )?;

        let (compute_storage_buffer, compute_storage_buffer_memory) = create_storage_buffer(
            device,
            physical_device,
            instance,
            Self::COMPUTE_STORAGE_BUFFER_SIZE,
        )?;

        // Zero-initialize the compute storage buffer so the shader reads defined values.
        unsafe {
            let ptr = device
                .map_memory(
                    compute_storage_buffer_memory,
                    0,
                    Self::COMPUTE_STORAGE_BUFFER_SIZE,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(|err| BackendError::Runtime(format!("map compute buffer failed: {err}")))?;
            std::ptr::write_bytes(ptr as *mut u8, 0, Self::COMPUTE_STORAGE_BUFFER_SIZE as usize);
            device.unmap_memory(compute_storage_buffer_memory);
        }

        Ok(Self {
            render_pass,
            pipeline_layout,
            graphics_pipeline,
            descriptor_set_layout,
            graphics_descriptor_pool,
            compute_descriptor_pool,
            compute_pipeline_layout,
            compute_pipeline,
            compute_descriptor_set_layout,
            compute_storage_buffer,
            compute_storage_buffer_memory,
            shader_modules: HashMap::new(),
            shader_entry_points: HashMap::new(),
            compute_pipelines: HashMap::new(),
            material_graphics_pipelines: HashMap::new(),
            framebuffers: HashMap::new(),
            image_views: HashMap::new(),
            offscreen_render_pass,
            offscreen_framebuffers: HashMap::new(),
            vertex_buffer,
            vertex_buffer_memory,
            dummy_texture,
            frame_descriptor_sets: Vec::new(),
        })
    }

    /// Ensures image views and framebuffers exist for every swapchain image.
    pub fn ensure_swapchain_views(
        &mut self,
        device: &ash::Device,
        images: &[vk::Image],
        extent: vk::Extent2D,
    ) -> Result<(), BackendError> {
        // Destroy old views/framebuffers first.
        for (_index, framebuffer) in self.framebuffers.drain() {
            unsafe {
                device.destroy_framebuffer(framebuffer, None);
            }
        }
        for (_index, view) in self.image_views.drain() {
            unsafe {
                device.destroy_image_view(view, None);
            }
        }

        for (index, &image) in images.iter().enumerate() {
            let view = create_image_view(device, image, vk::Format::B8G8R8A8_UNORM)?;
            let framebuffer = create_framebuffer(device, self.render_pass, view, extent)?;
            self.image_views.insert(index as u32, view);
            self.framebuffers.insert(index as u32, framebuffer);
        }

        Ok(())
    }

    fn get_or_create_offscreen_framebuffer(
        &mut self,
        device: &ash::Device,
        handle: engine_render_api::RenderTargetHandle,
        target: &crate::VulkanRenderTarget,
    ) -> Result<vk::Framebuffer, BackendError> {
        if let Some(&framebuffer) = self.offscreen_framebuffers.get(&handle) {
            return Ok(framebuffer);
        }
        let extent = vk::Extent2D {
            width: target.width,
            height: target.height,
        };
        let framebuffer = create_framebuffer(
            device,
            self.offscreen_render_pass,
            target.view,
            extent,
        )?;
        self.offscreen_framebuffers.insert(handle, framebuffer);
        Ok(framebuffer)
    }

    /// Loads a SPIR-V shader module from raw bytecode and stores it under `name`.
    pub fn load_shader_module(
        &mut self,
        device: &ash::Device,
        name: &str,
        bytecode: &[u8],
    ) -> Result<(), BackendError> {
        let code: Vec<u32> = bytecode
            .chunks_exact(4)
            .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect();
        let shader_info = vk::ShaderModuleCreateInfo::default().code(&code);
        let module = unsafe {
            device
                .create_shader_module(&shader_info, None)
                .map_err(|err| {
                    BackendError::Runtime(format!(
                        "create shader module '{name}' failed: {err}"
                    ))
                })?
        };
        self.shader_modules.insert(name.to_string(), module);
        Ok(())
    }

    /// Records actual GPU commands for the render graph into the supplied
    /// command buffer. The command buffer must already be begun.
    #[allow(clippy::too_many_arguments)]
    pub fn record_render_graph(
        &mut self,
        device: &ash::Device,
        command_buffer: vk::CommandBuffer,
        graph: &RenderGraph,
        textures: &HashMap<TextureHandle, crate::VulkanTexture>,
        render_targets: &HashMap<RenderTargetHandle, crate::VulkanRenderTarget>,
        storage_buffers: &HashMap<String, crate::VulkanStorageBuffer>,
        acquired_image_index: u32,
        extent: vk::Extent2D,
        clear_color: [f32; 4],
    ) -> Result<(), BackendError> {
        let framebuffer = *self.framebuffers.get(&acquired_image_index).ok_or_else(|| {
            BackendError::Runtime(format!(
                "no framebuffer for swapchain image {}",
                acquired_image_index
            ))
        })?;

        // The frame fence was waited on in acquire_frame, so descriptor sets
        // from the previous frame are no longer in flight and can be reset.
        unsafe {
            device
                .reset_descriptor_pool(
                    self.graphics_descriptor_pool,
                    vk::DescriptorPoolResetFlags::empty(),
                )
                .map_err(|err| {
                    BackendError::Runtime(format!(
                        "reset graphics descriptor pool failed: {err}"
                    ))
                })?;
            device
                .reset_descriptor_pool(
                    self.compute_descriptor_pool,
                    vk::DescriptorPoolResetFlags::empty(),
                )
                .map_err(|err| {
                    BackendError::Runtime(format!(
                        "reset compute descriptor pool failed: {err}"
                    ))
                })?;
        }
        self.frame_descriptor_sets.clear();

        // Each draw group shares a pipeline and texture so it can be issued
        // with a single descriptor-set binding.
        struct DrawGroup {
            pipeline: vk::Pipeline,
            texture: TextureHandle,
            vertex_offset: u32,
            vertex_count: u32,
        }

        struct PassDraw {
            target: Option<RenderTargetHandle>,
            group_start: usize,
            group_end: usize,
        }

        let mut all_vertices: Vec<SpriteVertex> = Vec::new();
        let mut draw_groups: Vec<DrawGroup> = Vec::new();
        let mut pass_draws: Vec<PassDraw> = Vec::new();
        let half_width = extent.width as f32 * 0.5;
        let half_height = extent.height as f32 * 0.5;

        for pass in &graph.passes {
            let RenderGraphPass::Render(render) = pass else {
                continue;
            };

            let pipeline = if let Some(material) = render
                .material
                .as_ref()
                .filter(|m| !m.shader_asset.is_empty())
            {
                if let Some(&module) = self.shader_modules.get(&material.shader_asset) {
                    get_or_create_material_graphics_pipeline(self, device, material, module)?
                } else {
                    self.graphics_pipeline
                }
            } else {
                self.graphics_pipeline
            };

            let group_start = draw_groups.len();
            for batch in &render.batches {
                let mut groups: HashMap<TextureHandle, Vec<SpriteVertex>> = HashMap::new();
                for sprite in &batch.sprites {
                    let vertices =
                        build_sprite_vertices(sprite, &render.camera, half_width, half_height);
                    groups
                        .entry(sprite.texture)
                        .or_default()
                        .extend_from_slice(&vertices);
                }

                let mut sorted: Vec<_> = groups.into_iter().collect();
                sorted.sort_by_key(|(handle, _)| handle.0);
                for (texture, vertices) in sorted {
                    if vertices.is_empty() {
                        continue;
                    }
                    let offset = all_vertices.len() as u32;
                    let count = vertices.len() as u32;
                    all_vertices.extend(vertices);
                    draw_groups.push(DrawGroup {
                        pipeline,
                        texture,
                        vertex_offset: offset,
                        vertex_count: count,
                    });
                }
            }
            pass_draws.push(PassDraw {
                target: render.target,
                group_start,
                group_end: draw_groups.len(),
            });
        }

        if !all_vertices.is_empty() {
            upload_vertices(
                device,
                self.vertex_buffer,
                self.vertex_buffer_memory,
                0,
                &all_vertices,
            )?;
        }

        if !draw_groups.is_empty() {
            let layouts = vec![self.descriptor_set_layout; draw_groups.len()];
            self.frame_descriptor_sets =
                allocate_descriptor_sets(device, self.graphics_descriptor_pool, &layouts)?;
        }

        let vertex_stride = std::mem::size_of::<SpriteVertex>() as vk::DeviceSize;
        let mut pass_index = 0usize;

        for pass in &graph.passes {
            match pass {
                RenderGraphPass::Render(_) => {
                    let pass_draw = &pass_draws[pass_index];
                    pass_index += 1;

                    let (pass_render_pass, pass_framebuffer, pass_extent) =
                        if let Some(handle) = pass_draw.target {
                            let target = render_targets.get(&handle).ok_or_else(|| {
                                BackendError::Runtime(format!(
                                    "render target {handle:?} not found"
                                ))
                            })?;
                            let fb = self.get_or_create_offscreen_framebuffer(
                                device,
                                handle,
                                target,
                            )?;
                            (
                                self.offscreen_render_pass,
                                fb,
                                vk::Extent2D {
                                    width: target.width,
                                    height: target.height,
                                },
                            )
                        } else {
                            (self.render_pass, framebuffer, extent)
                        };

                    let clear_values = [vk::ClearValue {
                        color: vk::ClearColorValue {
                            float32: clear_color,
                        },
                    }];
                    let render_pass_begin = vk::RenderPassBeginInfo::default()
                        .render_pass(pass_render_pass)
                        .framebuffer(pass_framebuffer)
                        .render_area(vk::Rect2D {
                            offset: vk::Offset2D { x: 0, y: 0 },
                            extent: pass_extent,
                        })
                        .clear_values(&clear_values);

                    unsafe {
                        device.cmd_begin_render_pass(
                            command_buffer,
                            &render_pass_begin,
                            vk::SubpassContents::INLINE,
                        );
                        device.cmd_set_viewport(
                            command_buffer,
                            0,
                            &[vk::Viewport {
                                x: 0.0,
                                y: 0.0,
                                width: pass_extent.width as f32,
                                height: pass_extent.height as f32,
                                min_depth: 0.0,
                                max_depth: 1.0,
                            }],
                        );
                        device.cmd_set_scissor(
                            command_buffer,
                            0,
                            &[render_pass_begin.render_area],
                        );

                        for (index, group) in draw_groups
                            .iter()
                            .enumerate()
                            .take(pass_draw.group_end)
                            .skip(pass_draw.group_start)
                        {
                            let descriptor_set = self.frame_descriptor_sets[index];
                            let (view, sampler) = textures
                                .get(&group.texture)
                                .map(|t| (t.view, t.sampler))
                                .unwrap_or((self.dummy_texture.view, self.dummy_texture.sampler));
                            update_graphics_descriptor_set(device, descriptor_set, view, sampler);
                            device.cmd_bind_descriptor_sets(
                                command_buffer,
                                vk::PipelineBindPoint::GRAPHICS,
                                self.pipeline_layout,
                                0,
                                &[descriptor_set],
                                &[],
                            );
                            device.cmd_bind_pipeline(
                                command_buffer,
                                vk::PipelineBindPoint::GRAPHICS,
                                group.pipeline,
                            );
                            let offset_bytes = group.vertex_offset as vk::DeviceSize * vertex_stride;
                            device.cmd_bind_vertex_buffers(
                                command_buffer,
                                0,
                                &[self.vertex_buffer],
                                &[offset_bytes],
                            );
                            device.cmd_draw(command_buffer, group.vertex_count, 1, 0, 0);
                        }

                        device.cmd_end_render_pass(command_buffer);
                    }
                }
                RenderGraphPass::Compute(compute) => {
                    record_compute_pass(
                        self,
                        device,
                        command_buffer,
                        compute,
                        storage_buffers,
                    )?;
                }
            }
        }

        Ok(())
    }

    pub fn destroy(
        &mut self, device: &ash::Device
    ) {
        unsafe {
            for (_name, pipeline) in self.material_graphics_pipelines.drain() {
                device.destroy_pipeline(pipeline, None);
            }
            for (_name, pipeline) in self.compute_pipelines.drain() {
                device.destroy_pipeline(pipeline, None);
            }
            for (_name, module) in self.shader_modules.drain() {
                device.destroy_shader_module(module, None);
            }
            for (_index, framebuffer) in self.framebuffers.drain() {
                device.destroy_framebuffer(framebuffer, None);
            }
            for (_index, view) in self.image_views.drain() {
                device.destroy_image_view(view, None);
            }
            for (_handle, framebuffer) in self.offscreen_framebuffers.drain() {
                device.destroy_framebuffer(framebuffer, None);
            }
            device.destroy_render_pass(self.offscreen_render_pass, None);
            device.destroy_pipeline(self.compute_pipeline, None);
            device.destroy_pipeline_layout(self.compute_pipeline_layout, None);
            device.destroy_descriptor_set_layout(self.compute_descriptor_set_layout, None);
            device.destroy_buffer(self.compute_storage_buffer, None);
            device.free_memory(self.compute_storage_buffer_memory, None);
            self.dummy_texture.destroy(device);
            device.destroy_buffer(self.vertex_buffer, None);
            device.free_memory(self.vertex_buffer_memory, None);
            device.destroy_descriptor_pool(self.graphics_descriptor_pool, None);
            device.destroy_descriptor_pool(self.compute_descriptor_pool, None);
            device.destroy_pipeline(self.graphics_pipeline, None);
            device.destroy_pipeline_layout(self.pipeline_layout, None);
            device.destroy_descriptor_set_layout(self.descriptor_set_layout, None);
            device.destroy_render_pass(self.render_pass, None);
        }
    }
}

fn record_compute_pass(
    renderer: &mut VulkanRenderer,
    device: &ash::Device,
    command_buffer: vk::CommandBuffer,
    compute: &ComputeDispatchNode,
    storage_buffers: &HashMap<String, crate::VulkanStorageBuffer>,
) -> Result<(), BackendError> {
    let pipeline = if compute.material.shader_asset.is_empty() {
        renderer.compute_pipeline
    } else if let Some(&module) = renderer.shader_modules.get(&compute.material.shader_asset) {
        if let Some(&pipeline) = renderer.compute_pipelines.get(&compute.material.shader_asset) {
            pipeline
        } else {
            let entry_point = renderer
                .shader_entry_points
                .get(&compute.material.shader_asset)
                .map(String::as_str)
                .unwrap_or("main");
            let entry_cstr = CString::new(entry_point).map_err(|_| {
                BackendError::Runtime(format!(
                    "invalid compute shader entry point '{entry_point}'"
                ))
            })?;
            let pipeline = create_user_compute_pipeline(
                device,
                renderer.compute_pipeline_layout,
                module,
                &entry_cstr,
            )?;
            renderer
                .compute_pipelines
                .insert(compute.material.shader_asset.clone(), pipeline);
            pipeline
        }
    } else {
        renderer.compute_pipeline
    };

    let (storage_buffer, storage_buffer_size) = compute
        .material
        .bindings
        .first()
        .and_then(|binding| storage_buffers.get(&binding.resource))
        .map(|sb| (sb.buffer, sb.size))
        .unwrap_or((
            renderer.compute_storage_buffer,
            VulkanRenderer::COMPUTE_STORAGE_BUFFER_SIZE,
        ));

    let descriptor_set = allocate_descriptor_set(
        device,
        renderer.compute_descriptor_pool,
        renderer.compute_descriptor_set_layout,
    )?;
    update_compute_descriptor_set(device, descriptor_set, storage_buffer, storage_buffer_size);

    unsafe {
        device.cmd_bind_pipeline(
            command_buffer,
            vk::PipelineBindPoint::COMPUTE,
            pipeline,
        );
        device.cmd_bind_descriptor_sets(
            command_buffer,
            vk::PipelineBindPoint::COMPUTE,
            renderer.compute_pipeline_layout,
            0,
            &[descriptor_set],
            &[],
        );
        device.cmd_dispatch(
            command_buffer,
            compute.dispatch[0],
            compute.dispatch[1],
            compute.dispatch[2],
        );
    }
    Ok(())
}

fn get_or_create_material_graphics_pipeline(
    renderer: &mut VulkanRenderer,
    device: &ash::Device,
    material: &Material,
    frag_module: vk::ShaderModule,
) -> Result<vk::Pipeline, BackendError> {
    if let Some(&pipeline) = renderer.material_graphics_pipelines.get(&material.shader_asset) {
        return Ok(pipeline);
    }

    let entry_point = renderer
        .shader_entry_points
        .get(&material.shader_asset)
        .map(String::as_str)
        .unwrap_or("main");
    let entry_cstr = CString::new(entry_point).map_err(|_| {
        BackendError::Runtime(format!(
            "invalid fragment shader entry point '{entry_point}'"
        ))
    })?;

    let pipeline = create_material_graphics_pipeline(
        device,
        renderer.render_pass,
        renderer.pipeline_layout,
        frag_module,
        &entry_cstr,
    )?;
    renderer
        .material_graphics_pipelines
        .insert(material.shader_asset.clone(), pipeline);
    Ok(pipeline)
}

fn build_sprite_vertices(
    sprite: &SpriteInstance,
    camera: &Camera2d,
    half_width: f32,
    half_height: f32,
) -> [SpriteVertex; 6] {
    let zoom = camera.zoom.max(0.05);
    let cx = (sprite.x + camera.x) * zoom;
    let cy = (sprite.y + camera.y) * zoom;
    let hw = sprite.width * 0.5 * zoom;
    let hh = sprite.height * 0.5 * zoom;

    let left = (cx - hw + half_width) / half_width - 1.0;
    let right = (cx + hw + half_width) / half_width - 1.0;
    let top = 1.0 - (cy - hh + half_height) / half_height;
    let bottom = 1.0 - (cy + hh + half_height) / half_height;

    let color = [
        sprite.tint[0].clamp(0.0, 1.0),
        sprite.tint[1].clamp(0.0, 1.0),
        sprite.tint[2].clamp(0.0, 1.0),
        sprite.tint[3].clamp(0.0, 1.0),
    ];

    [
        SpriteVertex {
            position: [left, top],
            color,
            tex_coord: [0.0, 0.0],
        },
        SpriteVertex {
            position: [right, top],
            color,
            tex_coord: [1.0, 0.0],
        },
        SpriteVertex {
            position: [left, bottom],
            color,
            tex_coord: [0.0, 1.0],
        },
        SpriteVertex {
            position: [right, top],
            color,
            tex_coord: [1.0, 0.0],
        },
        SpriteVertex {
            position: [right, bottom],
            color,
            tex_coord: [1.0, 1.0],
        },
        SpriteVertex {
            position: [left, bottom],
            color,
            tex_coord: [0.0, 1.0],
        },
    ]
}

fn upload_vertices(
    device: &ash::Device,
    _buffer: vk::Buffer,
    buffer_memory: vk::DeviceMemory,
    offset: vk::DeviceSize,
    vertices: &[SpriteVertex],
) -> Result<(), BackendError> {
    let bytes = bytemuck::cast_slice(vertices);
    let size = bytes.len() as vk::DeviceSize;

    unsafe {
        let ptr = device
            .map_memory(buffer_memory, offset, size, vk::MemoryMapFlags::empty())
            .map_err(|err| BackendError::Runtime(format!("map vertex buffer failed: {err}")))?;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
        device.unmap_memory(buffer_memory);
    }

    Ok(())
}

fn create_render_pass(
    device: &ash::Device,
    color_format: vk::Format,
) -> Result<vk::RenderPass, BackendError> {
    let color_attachment = vk::AttachmentDescription::default()
        .format(color_format)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::STORE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);

    let color_attachment_ref = vk::AttachmentReference::default()
        .attachment(0)
        .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);

    let subpass = vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(std::slice::from_ref(&color_attachment_ref));

    let dependency = vk::SubpassDependency::default()
        .src_subpass(vk::SUBPASS_EXTERNAL)
        .dst_subpass(0)
        .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .src_access_mask(vk::AccessFlags::empty())
        .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE);

    let render_pass_info = vk::RenderPassCreateInfo::default()
        .attachments(std::slice::from_ref(&color_attachment))
        .subpasses(std::slice::from_ref(&subpass))
        .dependencies(std::slice::from_ref(&dependency));

    unsafe {
        device
            .create_render_pass(&render_pass_info, None)
            .map_err(|err| BackendError::Runtime(format!("create render pass failed: {err}")))
    }
}

fn create_offscreen_render_pass(
    device: &ash::Device,
    color_format: vk::Format,
) -> Result<vk::RenderPass, BackendError> {
    let color_attachment = vk::AttachmentDescription::default()
        .format(color_format)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::STORE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .final_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);

    let color_attachment_ref = vk::AttachmentReference::default()
        .attachment(0)
        .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);

    let subpass = vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(std::slice::from_ref(&color_attachment_ref));

    let dependency = vk::SubpassDependency::default()
        .src_subpass(vk::SUBPASS_EXTERNAL)
        .dst_subpass(0)
        .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .src_access_mask(vk::AccessFlags::empty())
        .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE);

    let render_pass_info = vk::RenderPassCreateInfo::default()
        .attachments(std::slice::from_ref(&color_attachment))
        .subpasses(std::slice::from_ref(&subpass))
        .dependencies(std::slice::from_ref(&dependency));

    unsafe {
        device
            .create_render_pass(&render_pass_info, None)
            .map_err(|err| BackendError::Runtime(format!("create offscreen render pass failed: {err}")))
    }
}

fn create_descriptor_set_layout(
    device: &ash::Device,
) -> Result<vk::DescriptorSetLayout, BackendError> {
    let bindings = [
        vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        vk::DescriptorSetLayoutBinding::default()
            .binding(1)
            .descriptor_type(vk::DescriptorType::SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT),
    ];

    let layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);

    unsafe {
        device
            .create_descriptor_set_layout(&layout_info, None)
            .map_err(|err| BackendError::Runtime(format!("create descriptor layout failed: {err}")))
    }
}

fn create_compute_descriptor_set_layout(
    device: &ash::Device,
) -> Result<vk::DescriptorSetLayout, BackendError> {
    let bindings = [
        vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE),
    ];

    let layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);

    unsafe {
        device
            .create_descriptor_set_layout(&layout_info, None)
            .map_err(|err| {
                BackendError::Runtime(format!("create compute descriptor layout failed: {err}"))
            })
    }
}

fn create_pipeline_layout(
    device: &ash::Device,
    descriptor_set_layout: vk::DescriptorSetLayout,
) -> Result<vk::PipelineLayout, BackendError> {
    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));

    unsafe {
        device
            .create_pipeline_layout(&layout_info, None)
            .map_err(|err| BackendError::Runtime(format!("create pipeline layout failed: {err}")))
    }
}

fn create_compute_pipeline_layout(
    device: &ash::Device,
    descriptor_set_layout: vk::DescriptorSetLayout,
) -> Result<vk::PipelineLayout, BackendError> {
    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));

    unsafe {
        device
            .create_pipeline_layout(&layout_info, None)
            .map_err(|err| {
                BackendError::Runtime(format!("create compute pipeline layout failed: {err}"))
            })
    }
}

fn create_graphics_pipeline(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    pipeline_layout: vk::PipelineLayout,
) -> Result<vk::Pipeline, BackendError> {
    let vert_module = compile_wgsl_to_spirv_module(device, BUILTIN_VERTEX_SHADER)?;
    let frag_module = compile_wgsl_to_spirv_module(device, BUILTIN_FRAGMENT_SHADER)?;

    let vert_stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::VERTEX)
        .module(vert_module)
        .name(c"main");
    let frag_stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::FRAGMENT)
        .module(frag_module)
        .name(c"main");

    let binding = SpriteVertex::binding_description();
    let attributes = SpriteVertex::attribute_descriptions();
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(std::slice::from_ref(&binding))
        .vertex_attribute_descriptions(&attributes);

    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST);

    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);

    let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0);

    let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);

    let color_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
        .blend_enable(true)
        .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
        .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .color_blend_op(vk::BlendOp::ADD)
        .src_alpha_blend_factor(vk::BlendFactor::ONE)
        .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .alpha_blend_op(vk::BlendOp::ADD)
        .color_write_mask(vk::ColorComponentFlags::RGBA);

    let color_blending = vk::PipelineColorBlendStateCreateInfo::default()
        .attachments(std::slice::from_ref(&color_blend_attachment));

    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic_state_info =
        vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

    let stages = [vert_stage, frag_stage];
    let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport_state)
        .rasterization_state(&rasterizer)
        .multisample_state(&multisampling)
        .color_blend_state(&color_blending)
        .dynamic_state(&dynamic_state_info)
        .layout(pipeline_layout)
        .render_pass(render_pass)
        .subpass(0);

    let result = unsafe {
        device
            .create_graphics_pipelines(
                vk::PipelineCache::null(),
                std::slice::from_ref(&pipeline_info),
                None,
            )
            .map_err(|err| BackendError::Runtime(format!("create graphics pipeline failed: {err:?}")))
    };

    unsafe {
        device.destroy_shader_module(vert_module, None);
        device.destroy_shader_module(frag_module, None);
    }

    result.map(|pipelines| pipelines[0])
}

fn create_compute_pipeline(
    device: &ash::Device,
    pipeline_layout: vk::PipelineLayout,
) -> Result<vk::Pipeline, BackendError> {
    let module = compile_wgsl_to_spirv_module(device, BUILTIN_COMPUTE_SHADER)?;

    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(module)
        .name(c"main");

    let pipeline_info = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(pipeline_layout);

    let result = unsafe {
        device
            .create_compute_pipelines(
                vk::PipelineCache::null(),
                std::slice::from_ref(&pipeline_info),
                None,
            )
            .map_err(|err| BackendError::Runtime(format!("create compute pipeline failed: {err:?}")))
    };

    unsafe {
        device.destroy_shader_module(module, None);
    }

    result.map(|pipelines| pipelines[0])
}

fn create_material_graphics_pipeline(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    pipeline_layout: vk::PipelineLayout,
    frag_module: vk::ShaderModule,
    frag_entry_point: &std::ffi::CStr,
) -> Result<vk::Pipeline, BackendError> {
    let vert_module = compile_wgsl_to_spirv_module(device, BUILTIN_VERTEX_SHADER)?;

    let vert_stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::VERTEX)
        .module(vert_module)
        .name(c"main");
    let frag_stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::FRAGMENT)
        .module(frag_module)
        .name(frag_entry_point);

    let binding = SpriteVertex::binding_description();
    let attributes = SpriteVertex::attribute_descriptions();
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(std::slice::from_ref(&binding))
        .vertex_attribute_descriptions(&attributes);

    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST);

    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);

    let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0);

    let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);

    let color_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
        .blend_enable(true)
        .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
        .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .color_blend_op(vk::BlendOp::ADD)
        .src_alpha_blend_factor(vk::BlendFactor::ONE)
        .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .alpha_blend_op(vk::BlendOp::ADD)
        .color_write_mask(vk::ColorComponentFlags::RGBA);

    let color_blending = vk::PipelineColorBlendStateCreateInfo::default()
        .attachments(std::slice::from_ref(&color_blend_attachment));

    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic_state_info =
        vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

    let stages = [vert_stage, frag_stage];
    let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport_state)
        .rasterization_state(&rasterizer)
        .multisample_state(&multisampling)
        .color_blend_state(&color_blending)
        .dynamic_state(&dynamic_state_info)
        .layout(pipeline_layout)
        .render_pass(render_pass)
        .subpass(0);

    let result = unsafe {
        device
            .create_graphics_pipelines(
                vk::PipelineCache::null(),
                std::slice::from_ref(&pipeline_info),
                None,
            )
            .map_err(|err| {
                BackendError::Runtime(format!(
                    "create material graphics pipeline failed: {err:?}"
                ))
            })
    };

    unsafe {
        device.destroy_shader_module(vert_module, None);
    }

    result.map(|pipelines| pipelines[0])
}

fn create_user_compute_pipeline(
    device: &ash::Device,
    pipeline_layout: vk::PipelineLayout,
    module: vk::ShaderModule,
    entry_point: &std::ffi::CStr,
) -> Result<vk::Pipeline, BackendError> {
    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(module)
        .name(entry_point);

    let pipeline_info = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(pipeline_layout);

    unsafe {
        device
            .create_compute_pipelines(
                vk::PipelineCache::null(),
                std::slice::from_ref(&pipeline_info),
                None,
            )
            .map_err(|err| {
                BackendError::Runtime(format!(
                    "create user compute pipeline failed: {err:?}"
                ))
            })
            .map(|pipelines| pipelines[0])
    }
}

fn create_framebuffer(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    image_view: vk::ImageView,
    extent: vk::Extent2D,
) -> Result<vk::Framebuffer, BackendError> {
    let framebuffer_info = vk::FramebufferCreateInfo::default()
        .render_pass(render_pass)
        .attachments(std::slice::from_ref(&image_view))
        .width(extent.width)
        .height(extent.height)
        .layers(1);

    unsafe {
        device
            .create_framebuffer(&framebuffer_info, None)
            .map_err(|err| BackendError::Runtime(format!("create framebuffer failed: {err}")))
    }
}

pub(crate) fn create_image_view(
    device: &ash::Device,
    image: vk::Image,
    format: vk::Format,
) -> Result<vk::ImageView, BackendError> {
    let view_info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(format)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });

    unsafe {
        device
            .create_image_view(&view_info, None)
            .map_err(|err| BackendError::Runtime(format!("create image view failed: {err}")))
    }
}

fn create_compute_descriptor_pool(
    device: &ash::Device,
) -> Result<vk::DescriptorPool, BackendError> {
    let max_sets = 64u32;
    let pool_size = vk::DescriptorPoolSize {
        ty: vk::DescriptorType::STORAGE_BUFFER,
        descriptor_count: max_sets,
    };
    let pool_info = vk::DescriptorPoolCreateInfo::default()
        .max_sets(max_sets)
        .pool_sizes(std::slice::from_ref(&pool_size));
    unsafe {
        device
            .create_descriptor_pool(&pool_info, None)
            .map_err(|err| BackendError::Runtime(format!("create compute descriptor pool failed: {err}")))
    }
}

fn create_graphics_descriptor_pool(
    device: &ash::Device,
) -> Result<vk::DescriptorPool, BackendError> {
    let max_sets = MAX_DRAW_DESCRIPTOR_SETS;
    let pool_sizes = [
        vk::DescriptorPoolSize {
            ty: vk::DescriptorType::SAMPLED_IMAGE,
            descriptor_count: max_sets,
        },
        vk::DescriptorPoolSize {
            ty: vk::DescriptorType::SAMPLER,
            descriptor_count: max_sets,
        },
    ];
    let pool_info = vk::DescriptorPoolCreateInfo::default()
        .max_sets(max_sets)
        .pool_sizes(&pool_sizes);
    unsafe {
        device
            .create_descriptor_pool(&pool_info, None)
            .map_err(|err| BackendError::Runtime(format!("create graphics descriptor pool failed: {err}")))
    }
}

fn allocate_descriptor_set(
    device: &ash::Device,
    descriptor_pool: vk::DescriptorPool,
    descriptor_set_layout: vk::DescriptorSetLayout,
) -> Result<vk::DescriptorSet, BackendError> {
    let alloc_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(descriptor_pool)
        .set_layouts(std::slice::from_ref(&descriptor_set_layout));

    unsafe {
        device
            .allocate_descriptor_sets(&alloc_info)
            .map_err(|err| BackendError::Runtime(format!("allocate descriptor set failed: {err}")))
            .map(|sets| sets[0])
    }
}

fn allocate_descriptor_sets(
    device: &ash::Device,
    descriptor_pool: vk::DescriptorPool,
    layouts: &[vk::DescriptorSetLayout],
) -> Result<Vec<vk::DescriptorSet>, BackendError> {
    let alloc_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(descriptor_pool)
        .set_layouts(layouts);

    unsafe {
        device
            .allocate_descriptor_sets(&alloc_info)
            .map_err(|err| BackendError::Runtime(format!("allocate descriptor sets failed: {err}")))
    }
}

fn update_graphics_descriptor_set(
    device: &ash::Device,
    descriptor_set: vk::DescriptorSet,
    image_view: vk::ImageView,
    sampler: vk::Sampler,
) {
    let image_info = vk::DescriptorImageInfo::default()
        .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .image_view(image_view);
    let sampler_info = vk::DescriptorImageInfo::default().sampler(sampler);

    let writes = [
        vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(0)
            .dst_array_element(0)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .image_info(std::slice::from_ref(&image_info)),
        vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(1)
            .dst_array_element(0)
            .descriptor_type(vk::DescriptorType::SAMPLER)
            .image_info(std::slice::from_ref(&sampler_info)),
    ];

    unsafe {
        device.update_descriptor_sets(&writes, &[]);
    }
}

fn update_compute_descriptor_set(
    device: &ash::Device,
    descriptor_set: vk::DescriptorSet,
    buffer: vk::Buffer,
    range: vk::DeviceSize,
) {
    let buffer_info = vk::DescriptorBufferInfo::default()
        .buffer(buffer)
        .offset(0)
        .range(range);

    let write = vk::WriteDescriptorSet::default()
        .dst_set(descriptor_set)
        .dst_binding(0)
        .dst_array_element(0)
        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
        .buffer_info(std::slice::from_ref(&buffer_info));

    unsafe {
        device.update_descriptor_sets(std::slice::from_ref(&write), &[]);
    }
}

pub(crate) fn find_memory_type(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    type_filter: u32,
    properties: vk::MemoryPropertyFlags,
) -> Result<u32, BackendError> {
    let mem_properties = unsafe { instance.get_physical_device_memory_properties(physical_device) };
    for i in 0..mem_properties.memory_type_count {
        if (type_filter & (1 << i)) != 0
            && mem_properties.memory_types[i as usize]
                .property_flags
                .contains(properties)
        {
            return Ok(i);
        }
    }
    Err(BackendError::Runtime(
        "no suitable Vulkan memory type found".to_string(),
    ))
}

fn create_vertex_buffer(
    device: &ash::Device,
    physical_device: vk::PhysicalDevice,
    instance: &ash::Instance,
    size: vk::DeviceSize,
) -> Result<(vk::Buffer, vk::DeviceMemory), BackendError> {
    let buffer_info = vk::BufferCreateInfo::default()
        .size(size)
        .usage(vk::BufferUsageFlags::VERTEX_BUFFER)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);

    let buffer = unsafe {
        device
            .create_buffer(&buffer_info, None)
            .map_err(|err| BackendError::Runtime(format!("create vertex buffer failed: {err}")))?
    };

    let mem_requirements = unsafe { device.get_buffer_memory_requirements(buffer) };
    let memory_type = find_memory_type(
        instance,
        physical_device,
        mem_requirements.memory_type_bits,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    )?;

    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(mem_requirements.size)
        .memory_type_index(memory_type);

    let memory = unsafe {
        device
            .allocate_memory(&alloc_info, None)
            .map_err(|err| BackendError::Runtime(format!("allocate vertex buffer memory failed: {err}")))?
    };

    unsafe {
        device
            .bind_buffer_memory(buffer, memory, 0)
            .map_err(|err| BackendError::Runtime(format!("bind vertex buffer memory failed: {err}")))?;
    }

    Ok((buffer, memory))
}

pub(crate) fn create_storage_buffer(
    device: &ash::Device,
    physical_device: vk::PhysicalDevice,
    instance: &ash::Instance,
    size: vk::DeviceSize,
) -> Result<(vk::Buffer, vk::DeviceMemory), BackendError> {
    let buffer_info = vk::BufferCreateInfo::default()
        .size(size)
        .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);

    let buffer = unsafe {
        device
            .create_buffer(&buffer_info, None)
            .map_err(|err| BackendError::Runtime(format!("create storage buffer failed: {err}")))?
    };

    let mem_requirements = unsafe { device.get_buffer_memory_requirements(buffer) };
    let memory_type = find_memory_type(
        instance,
        physical_device,
        mem_requirements.memory_type_bits,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    )?;

    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(mem_requirements.size)
        .memory_type_index(memory_type);

    let memory = unsafe {
        device
            .allocate_memory(&alloc_info, None)
            .map_err(|err| BackendError::Runtime(format!("allocate storage buffer memory failed: {err}")))?
    };

    unsafe {
        device
            .bind_buffer_memory(buffer, memory, 0)
            .map_err(|err| BackendError::Runtime(format!("bind storage buffer memory failed: {err}")))?;
    }

    Ok((buffer, memory))
}

pub(crate) fn create_sampler(device: &ash::Device) -> Result<vk::Sampler, BackendError> {
    let sampler_info = vk::SamplerCreateInfo::default()
        .mag_filter(vk::Filter::LINEAR)
        .min_filter(vk::Filter::LINEAR)
        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE);
    unsafe {
        device
            .create_sampler(&sampler_info, None)
            .map_err(|err| BackendError::Runtime(format!("create sampler failed: {err}")))
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn create_texture_image(
    device: &ash::Device,
    physical_device: vk::PhysicalDevice,
    instance: &ash::Instance,
    queue_family_index: u32,
    command_pool: vk::CommandPool,
    queue: vk::Queue,
    width: u32,
    height: u32,
    data: Option<&[u8]>,
) -> Result<crate::VulkanTexture, BackendError> {
    if width == 0 || height == 0 {
        return Err(BackendError::Runtime(
            "texture dimensions must be non-zero".to_string(),
        ));
    }

    let extent = vk::Extent3D {
        width,
        height,
        depth: 1,
    };
    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::R8G8B8A8_UNORM)
        .extent(extent)
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .queue_family_indices(std::slice::from_ref(&queue_family_index))
        .initial_layout(vk::ImageLayout::UNDEFINED);

    let image = unsafe {
        device
            .create_image(&image_info, None)
            .map_err(|err| BackendError::Runtime(format!("create texture image failed: {err}")))?
    };

    let mem_requirements = unsafe { device.get_image_memory_requirements(image) };
    let memory_type = find_memory_type(
        instance,
        physical_device,
        mem_requirements.memory_type_bits,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )?;

    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(mem_requirements.size)
        .memory_type_index(memory_type);
    let memory = unsafe {
        device
            .allocate_memory(&alloc_info, None)
            .map_err(|err| BackendError::Runtime(format!("allocate texture memory failed: {err}")))?
    };

    unsafe {
        device
            .bind_image_memory(image, memory, 0)
            .map_err(|err| BackendError::Runtime(format!("bind texture memory failed: {err}")))?;
    }

    let view = create_image_view(device, image, vk::Format::R8G8B8A8_UNORM)?;
    let sampler = create_sampler(device)?;

    let pixel_count = (width as usize).checked_mul(height as usize).ok_or_else(|| {
        BackendError::Runtime("texture dimensions too large".to_string())
    })?;
    let expected_bytes = pixel_count.checked_mul(4).ok_or_else(|| {
        BackendError::Runtime("texture size too large".to_string())
    })?;
    let pixels: Vec<u8> = if let Some(data) = data {
        if data.len() == expected_bytes {
            data.to_vec()
        } else {
            vec![0xff; expected_bytes]
        }
    } else {
        vec![0xff; expected_bytes]
    };

    // Staging buffer for the upload.
    let staging_size = pixels.len() as vk::DeviceSize;
    let staging_buffer_info = vk::BufferCreateInfo::default()
        .size(staging_size)
        .usage(vk::BufferUsageFlags::TRANSFER_SRC)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let staging_buffer = unsafe {
        device
            .create_buffer(&staging_buffer_info, None)
            .map_err(|err| BackendError::Runtime(format!("create texture staging buffer failed: {err}")))?
    };
    let staging_mem_req = unsafe { device.get_buffer_memory_requirements(staging_buffer) };
    let staging_mem_type = find_memory_type(
        instance,
        physical_device,
        staging_mem_req.memory_type_bits,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    )?;
    let staging_alloc = vk::MemoryAllocateInfo::default()
        .allocation_size(staging_mem_req.size)
        .memory_type_index(staging_mem_type);
    let staging_memory = unsafe {
        device
            .allocate_memory(&staging_alloc, None)
            .map_err(|err| {
                BackendError::Runtime(format!(
                    "allocate texture staging memory failed: {err}"
                ))
            })?
    };
    unsafe {
        device
            .bind_buffer_memory(staging_buffer, staging_memory, 0)
            .map_err(|err| BackendError::Runtime(format!("bind texture staging memory failed: {err}")))?;
        let ptr = device
            .map_memory(
                staging_memory,
                0,
                staging_size,
                vk::MemoryMapFlags::empty(),
            )
            .map_err(|err| BackendError::Runtime(format!("map texture staging memory failed: {err}")))?;
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), ptr as *mut u8, pixels.len());
        device.unmap_memory(staging_memory);
    }

    // Record layout transitions and copy.
    let allocate_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let cmd = unsafe {
        device
            .allocate_command_buffers(&allocate_info)
            .map_err(|err| BackendError::Runtime(format!("allocate texture upload cb failed: {err}")))?[0]
    };
    unsafe {
        device
            .begin_command_buffer(
                cmd,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
            .map_err(|err| BackendError::Runtime(format!("begin texture upload cb failed: {err}")))?;

        let barrier_to_transfer = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            })
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE);
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            std::slice::from_ref(&barrier_to_transfer),
        );

        let region = vk::BufferImageCopy::default()
            .buffer_offset(0)
            .buffer_row_length(0)
            .buffer_image_height(0)
            .image_subresource(vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            })
            .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
            .image_extent(extent);
        device.cmd_copy_buffer_to_image(
            cmd,
            staging_buffer,
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            std::slice::from_ref(&region),
        );

        let barrier_to_shader = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            })
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ);
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            std::slice::from_ref(&barrier_to_shader),
        );

        device
            .end_command_buffer(cmd)
            .map_err(|err| BackendError::Runtime(format!("end texture upload cb failed: {err}")))?;
    }

    let fence = unsafe {
        device
            .create_fence(
                &vk::FenceCreateInfo::default(),
                None,
            )
            .map_err(|err| BackendError::Runtime(format!("create texture upload fence failed: {err}")))?
    };
    let submit_info = vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&cmd));
    unsafe {
        device
            .queue_submit(queue, std::slice::from_ref(&submit_info), fence)
            .map_err(|err| BackendError::Runtime(format!("submit texture upload failed: {err}")))?;
        device
            .wait_for_fences(std::slice::from_ref(&fence), true, u64::MAX)
            .map_err(|err| BackendError::Runtime(format!("wait for texture upload failed: {err}")))?;
        device.destroy_fence(fence, None);
        device.free_command_buffers(command_pool, std::slice::from_ref(&cmd));
        device.destroy_buffer(staging_buffer, None);
        device.free_memory(staging_memory, None);
    }

    Ok(crate::VulkanTexture {
        image,
        memory,
        view,
        sampler,
    })
}

pub(crate) fn create_render_target_image(
    device: &ash::Device,
    physical_device: vk::PhysicalDevice,
    instance: &ash::Instance,
    queue_family_index: u32,
    width: u32,
    height: u32,
) -> Result<crate::VulkanRenderTarget, BackendError> {
    if width == 0 || height == 0 {
        return Err(BackendError::Runtime(
            "render target dimensions must be non-zero".to_string(),
        ));
    }

    let extent = vk::Extent3D {
        width,
        height,
        depth: 1,
    };
    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::R8G8B8A8_UNORM)
        .extent(extent)
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .queue_family_indices(std::slice::from_ref(&queue_family_index))
        .initial_layout(vk::ImageLayout::UNDEFINED);

    let image = unsafe {
        device
            .create_image(&image_info, None)
            .map_err(|err| BackendError::Runtime(format!("create render target image failed: {err}")))?
    };

    let mem_requirements = unsafe { device.get_image_memory_requirements(image) };
    let memory_type = find_memory_type(
        instance,
        physical_device,
        mem_requirements.memory_type_bits,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )?;

    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(mem_requirements.size)
        .memory_type_index(memory_type);
    let memory = unsafe {
        device
            .allocate_memory(&alloc_info, None)
            .map_err(|err| {
                BackendError::Runtime(format!(
                    "allocate render target memory failed: {err}"
                ))
            })?
    };

    unsafe {
        device
            .bind_image_memory(image, memory, 0)
            .map_err(|err| BackendError::Runtime(format!("bind render target memory failed: {err}")))?;
    }

    let view = create_image_view(device, image, vk::Format::R8G8B8A8_UNORM)?;

    Ok(crate::VulkanRenderTarget {
        image,
        memory,
        view,
        width,
        height,
    })
}

fn compile_wgsl_to_spirv_module(
    device: &ash::Device,
    wgsl_source: &str,
) -> Result<vk::ShaderModule, BackendError> {
    let module = naga::front::wgsl::parse_str(wgsl_source)
        .map_err(|err| BackendError::Runtime(format!("WGSL parse failed: {err}")))?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|err| BackendError::Runtime(format!("WGSL validation failed: {err}")))?;

    let entry = module.entry_points.first().ok_or_else(|| {
        BackendError::Runtime("WGSL shader has no entry point".to_string())
    })?;
    let pipeline_options = naga::back::spv::PipelineOptions {
        shader_stage: entry.stage,
        entry_point: entry.name.clone(),
    };

    let spv = naga::back::spv::write_vec(
        &module,
        &info,
        &naga::back::spv::Options::default(),
        Some(&pipeline_options),
    )
    .map_err(|err| BackendError::Runtime(format!("SPIR-V generation failed: {err}")))?;

    let shader_info = vk::ShaderModuleCreateInfo::default().code(&spv);

    unsafe {
        device
            .create_shader_module(&shader_info, None)
            .map_err(|err| BackendError::Runtime(format!("create shader module failed: {err}")))
    }
}

const BUILTIN_VERTEX_SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn main(
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv: vec2<f32>,
) -> VertexOutput {
    var out: VertexOutput;
    out.position = vec4<f32>(position, 0.0, 1.0);
    out.color = color;
    out.uv = uv;
    return out;
}
"#;

const BUILTIN_FRAGMENT_SHADER: &str = r#"
@group(0) @binding(0)
var sprite_texture: texture_2d<f32>;
@group(0) @binding(1)
var sprite_sampler: sampler;

@fragment
fn main(
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
) -> @location(0) vec4<f32> {
    return textureSample(sprite_texture, sprite_sampler, uv) * color;
}
"#;

const BUILTIN_COMPUTE_SHADER: &str = r#"
@group(0) @binding(0)
var<storage, read_write> data: array<u32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i < arrayLength(&data)) {
        data[i] = data[i] + 1u;
    }
}
"#;
