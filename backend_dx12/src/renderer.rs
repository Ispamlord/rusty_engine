#[cfg(target_os = "windows")]
use std::cell::Cell;
#[cfg(target_os = "windows")]
use std::collections::HashMap;

#[cfg(target_os = "windows")]
use engine_render_api::{
    BackendError, RenderGraph, RenderGraphPass, RenderTargetHandle, SpriteInstance, TextureHandle,
};

#[cfg(target_os = "windows")]
use windows::core::PCSTR;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::RECT;
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Direct3D::{ID3DBlob, ID3DInclude};
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Direct3D12::{
    D3D12_BLEND_DESC, D3D12_BLEND_OP_ADD, D3D12_BLEND_INV_SRC_ALPHA, D3D12_BLEND_ONE,
    D3D12_BLEND_SRC_ALPHA, D3D12_COLOR_WRITE_ENABLE_ALL, D3D12_COMPARISON_FUNC_ALWAYS,
    D3D12_COMPARISON_FUNC_LESS, D3D12_COMPARISON_FUNC_NEVER, D3D12_COMPUTE_PIPELINE_STATE_DESC,
    D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF, D3D12_CPU_DESCRIPTOR_HANDLE,
    D3D12_CPU_PAGE_PROPERTY_UNKNOWN, D3D12_CULL_MODE_NONE, D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
    D3D12_DEPTH_STENCIL_DESC, D3D12_DEPTH_STENCILOP_DESC, D3D12_DEPTH_WRITE_MASK_ZERO,
    D3D12_DESCRIPTOR_HEAP_DESC, D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
    D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
    D3D12_DESCRIPTOR_HEAP_TYPE_RTV, D3D12_DESCRIPTOR_RANGE, D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
    D3D12_DESCRIPTOR_RANGE_TYPE_UAV, D3D12_FILL_MODE_SOLID, D3D12_FILTER_MIN_MAG_MIP_LINEAR,
    D3D12_GPU_DESCRIPTOR_HANDLE, D3D12_GRAPHICS_PIPELINE_STATE_DESC, D3D12_HEAP_FLAG_NONE,
    D3D12_HEAP_PROPERTIES, D3D12_HEAP_TYPE_DEFAULT, D3D12_HEAP_TYPE_UPLOAD,
    D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA, D3D12_INPUT_ELEMENT_DESC, D3D12_INPUT_LAYOUT_DESC,
    D3D12_LOGIC_OP_NOOP, D3D12_MEMORY_POOL_UNKNOWN, D3D12_PIPELINE_STATE_FLAG_NONE,
    D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE, D3D12_RASTERIZER_DESC, D3D12_RENDER_TARGET_BLEND_DESC,
    D3D12_RESOURCE_BARRIER, D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
    D3D12_RESOURCE_BARRIER_FLAG_NONE, D3D12_RESOURCE_BARRIER_TYPE_TRANSITION, D3D12_RESOURCE_DESC,
    D3D12_RESOURCE_DIMENSION_BUFFER, D3D12_RESOURCE_DIMENSION_TEXTURE2D,
    D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET, D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
    D3D12_RESOURCE_FLAG_NONE, D3D12_RESOURCE_STATE_GENERIC_READ,
    D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_PRESENT,
    D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
    D3D12_RESOURCE_TRANSITION_BARRIER, D3D12_ROOT_DESCRIPTOR_TABLE, D3D12_ROOT_PARAMETER,
    D3D12_ROOT_PARAMETER_0, D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE, D3D12_ROOT_SIGNATURE_DESC,
    D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT, D3D12_ROOT_SIGNATURE_FLAGS,
    D3D12_SHADER_BYTECODE, D3D12_SHADER_RESOURCE_VIEW_DESC, D3D12_SHADER_RESOURCE_VIEW_DESC_0,
    D3D12_SHADER_VISIBILITY_ALL, D3D12_SHADER_VISIBILITY_PIXEL, D3D12_SRV_DIMENSION_TEXTURE2D,
    D3D12_STATIC_BORDER_COLOR_TRANSPARENT_BLACK, D3D12_STATIC_SAMPLER_DESC,
    D3D12_STENCIL_OP_KEEP, D3D12_TEX2D_SRV, D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
    D3D12_TEXTURE_LAYOUT_ROW_MAJOR, D3D12_TEXTURE_LAYOUT_UNKNOWN, D3D12_UNORDERED_ACCESS_VIEW_DESC,
    D3D12_UNORDERED_ACCESS_VIEW_DESC_0, D3D12_UAV_DIMENSION_BUFFER, D3D12_VERTEX_BUFFER_VIEW,
    D3D12_VIEWPORT, D3D_ROOT_SIGNATURE_VERSION_1, D3D12SerializeRootSignature, ID3D12Device,
    ID3D12GraphicsCommandList, ID3D12PipelineState, ID3D12Resource, ID3D12RootSignature,
};
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_FORMAT_R8G8B8A8_UNORM,
    DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC,
};

/// GPU-side state cached by the DX12 backend to record real sprite draws.
#[cfg(target_os = "windows")]
pub struct Dx12Renderer {
    device: ID3D12Device,
    root_signature: ID3D12RootSignature,
    compute_root_signature: ID3D12RootSignature,
    pipeline_state: ID3D12PipelineState,
    vertex_buffer: ID3D12Resource,
    vertex_buffer_capacity: usize,
    #[allow(dead_code)]
    vertex_blob: ID3DBlob,
    #[allow(dead_code)]
    pixel_blob: ID3DBlob,
    /// Shader bytecodes preloaded by the runtime, keyed by asset name.
    pub(crate) shader_bytecodes: HashMap<String, Vec<u8>>,
    /// Cached graphics PSOs for render passes that reference preloaded pixel shader bytecode.
    material_psos: HashMap<String, ID3D12PipelineState>,
    /// Cached compute PSOs for compute passes that reference preloaded compute shader bytecode.
    compute_psos: HashMap<String, ID3D12PipelineState>,
    /// CPU/GPU descriptor heap for SRVs and UAVs.
    descriptor_heap: ID3D12DescriptorHeap,
    descriptor_heap_capacity: u32,
    descriptor_cpu_start: D3D12_CPU_DESCRIPTOR_HANDLE,
    descriptor_gpu_start: D3D12_GPU_DESCRIPTOR_HANDLE,
    descriptor_increment_size: u32,
    next_descriptor_index: Cell<u32>,
    /// Dedicated heap for offscreen render-target RTVs.
    offscreen_rtv_heap: ID3D12DescriptorHeap,
    offscreen_rtv_cpu_start: D3D12_CPU_DESCRIPTOR_HANDLE,
    offscreen_rtv_increment_size: u32,
    next_offscreen_rtv_index: Cell<u32>,
    /// Default white 1x1 texture used when a sprite has no valid texture handle.
    default_texture: ID3D12Resource,
    default_texture_srv_cpu: D3D12_CPU_DESCRIPTOR_HANDLE,
    default_texture_srv_gpu: D3D12_GPU_DESCRIPTOR_HANDLE,
    /// Default UAV bound for compute dispatches that do not reference a storage buffer.
    default_uav: D3D12_CPU_DESCRIPTOR_HANDLE,
    default_uav_gpu: D3D12_GPU_DESCRIPTOR_HANDLE,
}

#[cfg(target_os = "windows")]
impl Dx12Renderer {
    pub const MAX_SPRITES: usize = 4096;

    pub fn new(device: &ID3D12Device) -> Result<Self, BackendError> {
        unsafe {
            let vertex_blob = compile_shader(BUILTIN_VERTEX_SHADER, b"main\0", b"vs_5_1\0")?;
            let pixel_blob = compile_shader(BUILTIN_PIXEL_SHADER, b"main\0", b"ps_5_1\0")?;

            let root_signature = create_root_signature(device)?;
            let compute_root_signature = create_compute_root_signature(device)?;
            let pipeline_state = create_graphics_pipeline_state(
                device,
                &root_signature,
                shader_bytecode(&vertex_blob),
                shader_bytecode(&pixel_blob),
            )?;

            let vertex_buffer_capacity =
                Self::MAX_SPRITES * 6 * std::mem::size_of::<SpriteVertex>();
            let vertex_buffer = create_upload_buffer(device, vertex_buffer_capacity)?;

            const DESCRIPTOR_HEAP_SIZE: u32 = 256;
            let descriptor_heap = create_descriptor_heap(
                device,
                D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                DESCRIPTOR_HEAP_SIZE,
                D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
            )?;
            let descriptor_cpu_start = descriptor_heap.GetCPUDescriptorHandleForHeapStart();
            let descriptor_gpu_start = descriptor_heap.GetGPUDescriptorHandleForHeapStart();
            let descriptor_increment_size =
                device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV);

            let (default_texture, default_texture_srv_cpu, default_texture_srv_gpu) =
                create_default_texture(
                    device,
                    &descriptor_heap,
                    descriptor_cpu_start,
                    descriptor_gpu_start,
                    descriptor_increment_size,
                )?;

            let (default_uav_buffer, default_uav, default_uav_gpu) = create_default_uav_buffer(
                device,
                &descriptor_heap,
                descriptor_cpu_start,
                descriptor_gpu_start,
                descriptor_increment_size,
                1,
            )?;

            const OFFSCREEN_RTV_HEAP_SIZE: u32 = 64;
            let offscreen_rtv_heap = create_descriptor_heap(
                device,
                D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                OFFSCREEN_RTV_HEAP_SIZE,
                D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
            )?;
            let offscreen_rtv_cpu_start = offscreen_rtv_heap.GetCPUDescriptorHandleForHeapStart();
            let offscreen_rtv_increment_size =
                device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV);

            Ok(Self {
                device: device.clone(),
                root_signature,
                compute_root_signature,
                pipeline_state,
                vertex_buffer,
                vertex_buffer_capacity,
                vertex_blob,
                pixel_blob,
                shader_bytecodes: HashMap::new(),
                material_psos: HashMap::new(),
                compute_psos: HashMap::new(),
                descriptor_heap,
                descriptor_heap_capacity: DESCRIPTOR_HEAP_SIZE,
                descriptor_cpu_start,
                descriptor_gpu_start,
                descriptor_increment_size,
                next_descriptor_index: Cell::new(2),
                offscreen_rtv_heap,
                offscreen_rtv_cpu_start,
                offscreen_rtv_increment_size,
                next_offscreen_rtv_index: Cell::new(0),
                default_texture,
                default_texture_srv_cpu,
                default_texture_srv_gpu,
                default_uav,
                default_uav_gpu,
            })
        }
    }

    /// Stores preloaded shader bytecode for later use by render/compute passes.
    pub fn preload_shader_bytecode(&mut self, name: &str, _entry_point: &str, bytecode: &[u8]) {
        self.shader_bytecodes
            .insert(name.to_string(), bytecode.to_vec());
    }

    /// Releases cached pipeline states and shader bytecode.
    pub fn destroy(&mut self) {
        self.material_psos.clear();
        self.compute_psos.clear();
        self.shader_bytecodes.clear();
    }

    /// Allocates a CPU descriptor index from the CBV/SRV/UAV heap.
    pub fn allocate_descriptor_index(&self,
    ) -> Option<u32> {
        let index = self.next_descriptor_index.get();
        if index >= self.descriptor_heap_capacity {
            return None;
        }
        self.next_descriptor_index.set(index + 1);
        Some(index)
    }

    /// Returns the CPU handle for a descriptor index in the CBV/SRV/UAV heap.
    pub fn cpu_handle(&self,
        index: u32,
    ) -> D3D12_CPU_DESCRIPTOR_HANDLE {
        D3D12_CPU_DESCRIPTOR_HANDLE {
            ptr: self.descriptor_cpu_start.ptr
                + (index as usize * self.descriptor_increment_size as usize),
        }
    }

    /// Returns the GPU handle for a descriptor index in the CBV/SRV/UAV heap.
    pub fn gpu_handle(&self,
        index: u32,
    ) -> D3D12_GPU_DESCRIPTOR_HANDLE {
        D3D12_GPU_DESCRIPTOR_HANDLE {
            ptr: self.descriptor_gpu_start.ptr
                + (index as u64 * self.descriptor_increment_size as u64),
        }
    }

    /// Allocates a CPU descriptor index from the offscreen RTV heap.
    pub fn allocate_rtv_index(&self,
    ) -> Option<u32> {
        let index = self.next_offscreen_rtv_index.get();
        if index >= 64 {
            return None;
        }
        self.next_offscreen_rtv_index.set(index + 1);
        Some(index)
    }

    /// Returns the CPU handle for an offscreen RTV descriptor index.
    pub fn rtv_cpu_handle(&self,
        index: u32,
    ) -> D3D12_CPU_DESCRIPTOR_HANDLE {
        D3D12_CPU_DESCRIPTOR_HANDLE {
            ptr: self.offscreen_rtv_cpu_start.ptr
                + (index as usize * self.offscreen_rtv_increment_size as usize),
        }
    }

    /// Records a full frame of sprite rendering into `command_list`.
    pub fn record_frame(
        &mut self,
        command_list: &ID3D12GraphicsCommandList,
        surface_state: &super::Dx12SurfaceState,
        graph: &RenderGraph,
        textures: &HashMap<TextureHandle, super::Dx12Texture>,
        render_targets: &HashMap<RenderTargetHandle, super::Dx12RenderTarget>,
        storage_buffers: &HashMap<String, super::Dx12StorageBuffer>,
    ) -> Result<(), BackendError> {
        let swapchain_width = surface_state.width.max(1);
        let swapchain_height = surface_state.height.max(1);
        let clear_color = [0.04_f32, 0.09, 0.08, 1.0];

        unsafe {
            command_list.SetDescriptorHeaps(&[Some(self.descriptor_heap.clone())]);

            let (vertices, pass_texture_groups) =
                build_sprite_vertices_by_pass_and_texture(graph, swapchain_width, swapchain_height);
            let total_vertex_count = vertices.len();

            if total_vertex_count > 0 {
                let bytes = bytemuck::cast_slice(&vertices);
                if bytes.len() > self.vertex_buffer_capacity {
                    return Err(BackendError::Runtime(
                        "sprite vertex data exceeds upload buffer capacity".to_string(),
                    ));
                }

                let mut mapped = std::ptr::null_mut::<core::ffi::c_void>();
                self.vertex_buffer
                    .Map(0, None, Some(&mut mapped))
                    .map_err(|err| BackendError::Runtime(format!("map vertex buffer failed: {err}")))?;
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped as *mut u8, bytes.len());
                self.vertex_buffer.Unmap(0, None);

                let view = D3D12_VERTEX_BUFFER_VIEW {
                    BufferLocation: self.vertex_buffer.GetGPUVirtualAddress(),
                    SizeInBytes: bytes.len() as u32,
                    StrideInBytes: std::mem::size_of::<SpriteVertex>() as u32,
                };
                command_list.IASetVertexBuffers(0, Some(&[view]));
                command_list.IASetPrimitiveTopology(
                    windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
                );
            }

            for (pass_index, pass) in graph.passes.iter().enumerate() {
                match pass {
                    RenderGraphPass::Render(render) => {
                        let (render_target_resource, rtv_handle, target_width, target_height, is_swapchain) =
                            if let Some(target_handle) = render.target {
                                if let Some(rt) = render_targets.get(&target_handle) {
                                    (&rt.texture,
                                     rt.rtv,
                                     rt.width,
                                     rt.height,
                                     false)
                                } else {
                                    let idx = surface_state.swapchain.GetCurrentBackBufferIndex() as usize;
                                    (
                                        &surface_state.render_targets[idx],
                                        swapchain_rtv_handle(surface_state, idx),
                                        swapchain_width,
                                        swapchain_height,
                                        true,
                                    )
                                }
                            } else {
                                let idx = surface_state.swapchain.GetCurrentBackBufferIndex() as usize;
                                (
                                    &surface_state.render_targets[idx],
                                    swapchain_rtv_handle(surface_state, idx),
                                    swapchain_width,
                                    swapchain_height,
                                    true,
                                )
                            };

                        let before_state = if is_swapchain {
                            D3D12_RESOURCE_STATE_PRESENT
                        } else {
                            D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE
                        };
                        command_list.ResourceBarrier(&[transition_barrier(
                            render_target_resource,
                            before_state,
                            D3D12_RESOURCE_STATE_RENDER_TARGET,
                        )]);

                        command_list.ClearRenderTargetView(rtv_handle, &clear_color, None);

                        command_list.SetGraphicsRootSignature(&self.root_signature);

                        let viewport = D3D12_VIEWPORT {
                            TopLeftX: 0.0,
                            TopLeftY: 0.0,
                            Width: target_width as f32,
                            Height: target_height as f32,
                            MinDepth: 0.0,
                            MaxDepth: 1.0,
                        };
                        command_list.RSSetViewports(&[viewport]);

                        let scissor = RECT {
                            left: 0,
                            top: 0,
                            right: target_width as i32,
                            bottom: target_height as i32,
                        };
                        command_list.RSSetScissorRects(&[scissor]);

                        command_list.OMSetRenderTargets(1, Some(&rtv_handle), false, None);

                        let shader_asset = render
                            .material
                            .as_ref()
                            .map(|m| m.shader_asset.as_str())
                            .unwrap_or("");
                        let pso = if !shader_asset.is_empty()
                            && self.shader_bytecodes.contains_key(shader_asset)
                        {
                            if !self.material_psos.contains_key(shader_asset) {
                                let bytecode = self.shader_bytecodes.get(shader_asset).unwrap().clone();
                                let pso = create_graphics_pipeline_state(
                                    &self.device,
                                    &self.root_signature,
                                    shader_bytecode(&self.vertex_blob),
                                    raw_bytecode(&bytecode),
                                )?;
                                self.material_psos.insert(shader_asset.to_string(), pso);
                            }
                            self.material_psos.get(shader_asset).unwrap()
                        } else {
                            &self.pipeline_state
                        };
                        command_list.SetPipelineState(pso);

                        for (texture_handle, start, count) in &pass_texture_groups[pass_index] {
                            let (srv_cpu, srv_gpu) = textures
                                .get(texture_handle)
                                .map(|t| (t.srv, self.gpu_handle_from_cpu(t.srv)))
                                .unwrap_or((
                                    self.default_texture_srv_cpu,
                                    self.default_texture_srv_gpu,
                                ));
                            command_list.SetGraphicsRootDescriptorTable(0, srv_gpu);
                            if *count > 0 {
                                command_list.DrawInstanced(*count, 1, *start, 0);
                            }
                        }

                        let after_state = if is_swapchain {
                            D3D12_RESOURCE_STATE_PRESENT
                        } else {
                            D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE
                        };
                        command_list.ResourceBarrier(&[transition_barrier(
                            render_target_resource,
                            D3D12_RESOURCE_STATE_RENDER_TARGET,
                            after_state,
                        )]);
                    }
                    RenderGraphPass::Compute(compute) => {
                        let shader_asset = &compute.material.shader_asset;
                        if shader_asset.is_empty()
                            || !self.shader_bytecodes.contains_key(shader_asset)
                        {
                            continue;
                        }

                        command_list.SetComputeRootSignature(&self.compute_root_signature);

                        if !self.compute_psos.contains_key(shader_asset) {
                            let bytecode = self.shader_bytecodes.get(shader_asset).unwrap().clone();
                            let pso = create_compute_pipeline_state(
                                &self.device,
                                &self.compute_root_signature,
                                &bytecode,
                            )?;
                            self.compute_psos.insert(shader_asset.clone(), pso);
                        }
                        let pso = self.compute_psos.get(shader_asset).unwrap();
                        command_list.SetPipelineState(pso);

                        let uav_cpu = compute
                            .material
                            .bindings
                            .first()
                            .and_then(|b| storage_buffers.get(&b.resource))
                            .map(|sb| sb.uav)
                            .unwrap_or(self.default_uav);
                        let uav_gpu = self.gpu_handle_from_cpu(uav_cpu);
                        command_list.SetComputeRootDescriptorTable(0, uav_gpu);

                        command_list.Dispatch(
                            compute.dispatch[0],
                            compute.dispatch[1],
                            compute.dispatch[2],
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Computes the GPU descriptor handle corresponding to a CPU handle in the
    /// CBV/SRV/UAV heap.
    fn gpu_handle_from_cpu(
        &self,
        cpu: D3D12_CPU_DESCRIPTOR_HANDLE,
    ) -> D3D12_GPU_DESCRIPTOR_HANDLE {
        let offset = cpu.ptr - self.descriptor_cpu_start.ptr;
        D3D12_GPU_DESCRIPTOR_HANDLE {
            ptr: self.descriptor_gpu_start.ptr + offset as u64,
        }
    }
}

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

#[cfg(target_os = "windows")]
fn swapchain_rtv_handle(
    surface_state: &super::Dx12SurfaceState,
    index: usize,
) -> D3D12_CPU_DESCRIPTOR_HANDLE {
    let rtv_start = unsafe { surface_state.rtv_heap.GetCPUDescriptorHandleForHeapStart() };
    D3D12_CPU_DESCRIPTOR_HANDLE {
        ptr: rtv_start.ptr + (index * surface_state.rtv_descriptor_size as usize),
    }
}

#[cfg(target_os = "windows")]
unsafe fn create_descriptor_heap(
    device: &ID3D12Device,
    heap_type: windows::Win32::Graphics::Direct3D12::D3D12_DESCRIPTOR_HEAP_TYPE,
    num_descriptors: u32,
    flags: windows::Win32::Graphics::Direct3D12::D3D12_DESCRIPTOR_HEAP_FLAGS,
) -> Result<ID3D12DescriptorHeap, BackendError> {
    let heap_desc = D3D12_DESCRIPTOR_HEAP_DESC {
        Type: heap_type,
        NumDescriptors: num_descriptors,
        Flags: flags,
        NodeMask: 0,
    };
    device
        .CreateDescriptorHeap(&heap_desc)
        .map_err(|err| BackendError::Runtime(format!("CreateDescriptorHeap failed: {err}")))
}

#[cfg(target_os = "windows")]
unsafe fn create_default_texture(
    device: &ID3D12Device,
    _descriptor_heap: &ID3D12DescriptorHeap,
    cpu_start: D3D12_CPU_DESCRIPTOR_HANDLE,
    gpu_start: D3D12_GPU_DESCRIPTOR_HANDLE,
    _increment_size: u32,
) -> Result<(ID3D12Resource, D3D12_CPU_DESCRIPTOR_HANDLE, D3D12_GPU_DESCRIPTOR_HANDLE), BackendError>
{
    const WIDTH: u32 = 1;
    const HEIGHT: u32 = 1;

    let heap_properties = D3D12_HEAP_PROPERTIES {
        Type: D3D12_HEAP_TYPE_UPLOAD,
        CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
        MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
        CreationNodeMask: 0,
        VisibleNodeMask: 0,
    };
    let resource_desc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        Alignment: 0,
        Width: WIDTH as u64,
        Height: HEIGHT,
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        Flags: D3D12_RESOURCE_FLAG_NONE,
    };

    let texture: ID3D12Resource = device
        .CreateCommittedResource(
            &heap_properties,
            D3D12_HEAP_FLAG_NONE,
            &resource_desc,
            D3D12_RESOURCE_STATE_GENERIC_READ,
            None,
        )
        .map_err(|err| BackendError::Runtime(format!("create default texture failed: {err}")))?;

    let white: [u8; 4] = [255, 255, 255, 255];
    let mut mapped = std::ptr::null_mut::<core::ffi::c_void>();
    texture
        .Map(0, None, Some(&mut mapped))
        .map_err(|err| BackendError::Runtime(format!("map default texture failed: {err}")))?;
    std::ptr::copy_nonoverlapping(white.as_ptr(), mapped as *mut u8, white.len());
    texture.Unmap(0, None);

    let srv_cpu = D3D12_CPU_DESCRIPTOR_HANDLE {
        ptr: cpu_start.ptr,
    };
    let srv_gpu = D3D12_GPU_DESCRIPTOR_HANDLE {
        ptr: gpu_start.ptr,
    };

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
    device.CreateShaderResourceView(&texture, Some(&srv_desc), srv_cpu);

    Ok((texture, srv_cpu, srv_gpu))
}

#[cfg(target_os = "windows")]
unsafe fn create_default_uav_buffer(
    device: &ID3D12Device,
    descriptor_heap: &ID3D12DescriptorHeap,
    cpu_start: D3D12_CPU_DESCRIPTOR_HANDLE,
    gpu_start: D3D12_GPU_DESCRIPTOR_HANDLE,
    increment_size: u32,
    index: u32,
) -> Result<(ID3D12Resource, D3D12_CPU_DESCRIPTOR_HANDLE, D3D12_GPU_DESCRIPTOR_HANDLE), BackendError>
{
    let size = 256u32;
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
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        Flags: D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
    };

    let buffer: ID3D12Resource = device
        .CreateCommittedResource(
            &heap_properties,
            D3D12_HEAP_FLAG_NONE,
            &resource_desc,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            None,
        )
        .map_err(|err| BackendError::Runtime(format!("create default uav buffer failed: {err}")))?;

    let uav_cpu = D3D12_CPU_DESCRIPTOR_HANDLE {
        ptr: cpu_start.ptr + (index as usize * increment_size as usize),
    };
    let uav_gpu = D3D12_GPU_DESCRIPTOR_HANDLE {
        ptr: gpu_start.ptr + (index as u64 * increment_size as u64),
    };

    let uav_desc = D3D12_UNORDERED_ACCESS_VIEW_DESC {
        Format: DXGI_FORMAT_UNKNOWN,
        ViewDimension: D3D12_UAV_DIMENSION_BUFFER,
        Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
            Buffer: D3D12_BUFFER_UAV {
                FirstElement: 0,
                NumElements: size / 4,
                StructureByteStride: 0,
                CounterOffsetInBytes: 0,
                Flags: D3D12_BUFFER_UAV_FLAG_NONE,
            },
        },
    };
    device.CreateUnorderedAccessView(&buffer, None, Some(&uav_desc), uav_cpu);

    Ok((buffer, uav_cpu, uav_gpu))
}

#[cfg(target_os = "windows")]
unsafe fn compile_shader(source: &str, entry: &[u8], target: &[u8]) -> Result<ID3DBlob, BackendError> {
    let mut blob: Option<ID3DBlob> = None;
    let mut error: Option<ID3DBlob> = None;

    D3DCompile(
        source.as_ptr() as *const core::ffi::c_void,
        source.len(),
        PCSTR::null(),
        None,
        None::<&ID3DInclude>,
        PCSTR::from_raw(entry.as_ptr()),
        PCSTR::from_raw(target.as_ptr()),
        0,
        0,
        &mut blob,
        Some(&mut error),
    )
    .map_err(|err| {
        let message = error
            .as_ref()
            .map(|e| {
                let slice = std::slice::from_raw_parts(
                    e.GetBufferPointer() as *const u8,
                    e.GetBufferSize(),
                );
                String::from_utf8_lossy(slice).to_string()
            })
            .unwrap_or_else(|| format!("D3DCompile failed: {err}"));
        BackendError::Runtime(message)
    })?;

    blob.ok_or_else(|| BackendError::Runtime("D3DCompile returned no shader blob".to_string()))
}

#[cfg(target_os = "windows")]
unsafe fn create_root_signature(device: &ID3D12Device) -> Result<ID3D12RootSignature, BackendError> {
    let range = D3D12_DESCRIPTOR_RANGE {
        RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
        NumDescriptors: 1,
        BaseShaderRegister: 0,
        RegisterSpace: 0,
        OffsetInDescriptorsFromTableStart: 0,
    };
    let table = D3D12_ROOT_DESCRIPTOR_TABLE {
        NumDescriptorRanges: 1,
        pDescriptorRanges: &range,
    };
    let param = D3D12_ROOT_PARAMETER {
        ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
        ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
        Anonymous: D3D12_ROOT_PARAMETER_0 { DescriptorTable: table },
    };
    let sampler = D3D12_STATIC_SAMPLER_DESC {
        Filter: D3D12_FILTER_MIN_MAG_MIP_LINEAR,
        AddressU: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
        AddressV: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
        AddressW: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
        MipLODBias: 0.0,
        MaxAnisotropy: 0,
        ComparisonFunc: D3D12_COMPARISON_FUNC_NEVER,
        BorderColor: D3D12_STATIC_BORDER_COLOR_TRANSPARENT_BLACK,
        MinLOD: 0.0,
        MaxLOD: f32::MAX,
        ShaderRegister: 0,
        RegisterSpace: 0,
        ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
    };
    let desc = D3D12_ROOT_SIGNATURE_DESC {
        NumParameters: 1,
        pParameters: &param,
        NumStaticSamplers: 1,
        pStaticSamplers: &sampler,
        Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
    };
    serialize_and_create_root_signature(device, &desc)
}

#[cfg(target_os = "windows")]
unsafe fn create_compute_root_signature(device: &ID3D12Device) -> Result<ID3D12RootSignature, BackendError> {
    let range = D3D12_DESCRIPTOR_RANGE {
        RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_UAV,
        NumDescriptors: 1,
        BaseShaderRegister: 0,
        RegisterSpace: 0,
        OffsetInDescriptorsFromTableStart: 0,
    };
    let table = D3D12_ROOT_DESCRIPTOR_TABLE {
        NumDescriptorRanges: 1,
        pDescriptorRanges: &range,
    };
    let param = D3D12_ROOT_PARAMETER {
        ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
        ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
        Anonymous: D3D12_ROOT_PARAMETER_0 { DescriptorTable: table },
    };
    let desc = D3D12_ROOT_SIGNATURE_DESC {
        NumParameters: 1,
        pParameters: &param,
        NumStaticSamplers: 0,
        pStaticSamplers: std::ptr::null(),
        Flags: D3D12_ROOT_SIGNATURE_FLAGS(0),
    };
    serialize_and_create_root_signature(device, &desc)
}

#[cfg(target_os = "windows")]
unsafe fn serialize_and_create_root_signature(
    device: &ID3D12Device,
    desc: &D3D12_ROOT_SIGNATURE_DESC,
) -> Result<ID3D12RootSignature, BackendError> {
    let mut blob: Option<ID3DBlob> = None;
    let mut error: Option<ID3DBlob> = None;
    D3D12SerializeRootSignature(desc, D3D_ROOT_SIGNATURE_VERSION_1, &mut blob, Some(&mut error))
        .map_err(|err| {
            let message = error
                .as_ref()
                .map(|e| {
                    let slice = std::slice::from_raw_parts(
                        e.GetBufferPointer() as *const u8,
                        e.GetBufferSize(),
                    );
                    String::from_utf8_lossy(slice).to_string()
                })
                .unwrap_or_else(|| format!("D3D12SerializeRootSignature failed: {err}"));
            BackendError::Runtime(message)
        })?;

    let signature_blob = blob.ok_or_else(|| {
        BackendError::Runtime("D3D12SerializeRootSignature returned no blob".to_string())
    })?;
    let bytes = std::slice::from_raw_parts(
        signature_blob.GetBufferPointer() as *const u8,
        signature_blob.GetBufferSize(),
    );

    device
        .CreateRootSignature(0, bytes)
        .map_err(|err| BackendError::Runtime(format!("CreateRootSignature failed: {err}")))
}

#[cfg(target_os = "windows")]
unsafe fn create_graphics_pipeline_state(
    device: &ID3D12Device,
    root_signature: &ID3D12RootSignature,
    vertex_bytecode: D3D12_SHADER_BYTECODE,
    pixel_bytecode: D3D12_SHADER_BYTECODE,
) -> Result<ID3D12PipelineState, BackendError> {
    let input_elements = [
        D3D12_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR::from_raw(b"POSITION\0".as_ptr()),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: 0,
            InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
        D3D12_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR::from_raw(b"COLOR\0".as_ptr()),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: std::mem::size_of::<[f32; 2]>() as u32,
            InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
        D3D12_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR::from_raw(b"TEXCOORD\0".as_ptr()),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: std::mem::size_of::<[f32; 6]>() as u32,
            InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
    ];

    let mut pso_desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC::default();
    pso_desc.pRootSignature =
        std::mem::ManuallyDrop::new(Some(root_signature.clone()));
    pso_desc.VS = vertex_bytecode;
    pso_desc.PS = pixel_bytecode;
    pso_desc.BlendState = blend_desc();
    pso_desc.SampleMask = u32::MAX;
    pso_desc.RasterizerState = rasterizer_desc();
    pso_desc.DepthStencilState = depth_stencil_desc();
    pso_desc.InputLayout = D3D12_INPUT_LAYOUT_DESC {
        pInputElementDescs: input_elements.as_ptr(),
        NumElements: input_elements.len() as u32,
    };
    pso_desc.PrimitiveTopologyType = D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE;
    pso_desc.NumRenderTargets = 1;
    pso_desc.RTVFormats[0] = DXGI_FORMAT_R8G8B8A8_UNORM;
    pso_desc.SampleDesc = DXGI_SAMPLE_DESC {
        Count: 1,
        Quality: 0,
    };
    pso_desc.Flags = D3D12_PIPELINE_STATE_FLAG_NONE;

    device
        .CreateGraphicsPipelineState(&pso_desc)
        .map_err(|err| BackendError::Runtime(format!("CreateGraphicsPipelineState failed: {err}")))
}

#[cfg(target_os = "windows")]
unsafe fn create_compute_pipeline_state(
    device: &ID3D12Device,
    root_signature: &ID3D12RootSignature,
    bytecode: &[u8],
) -> Result<ID3D12PipelineState, BackendError> {
    let mut pso_desc = D3D12_COMPUTE_PIPELINE_STATE_DESC::default();
    pso_desc.pRootSignature = std::mem::ManuallyDrop::new(Some(root_signature.clone()));
    pso_desc.CS = D3D12_SHADER_BYTECODE {
        pShaderBytecode: bytecode.as_ptr() as *const core::ffi::c_void,
        BytecodeLength: bytecode.len(),
    };
    pso_desc.Flags = D3D12_PIPELINE_STATE_FLAG_NONE;

    device
        .CreateComputePipelineState(&pso_desc)
        .map_err(|err| BackendError::Runtime(format!("CreateComputePipelineState failed: {err}")))
}

#[cfg(target_os = "windows")]
unsafe fn shader_bytecode(blob: &ID3DBlob) -> D3D12_SHADER_BYTECODE {
    D3D12_SHADER_BYTECODE {
        pShaderBytecode: blob.GetBufferPointer(),
        BytecodeLength: blob.GetBufferSize(),
    }
}

#[cfg(target_os = "windows")]
fn raw_bytecode(bytes: &[u8]) -> D3D12_SHADER_BYTECODE {
    D3D12_SHADER_BYTECODE {
        pShaderBytecode: bytes.as_ptr() as *const core::ffi::c_void,
        BytecodeLength: bytes.len(),
    }
}

#[cfg(target_os = "windows")]
fn blend_desc() -> D3D12_BLEND_DESC {
    D3D12_BLEND_DESC {
        AlphaToCoverageEnable: false.into(),
        IndependentBlendEnable: false.into(),
        RenderTarget: [D3D12_RENDER_TARGET_BLEND_DESC {
            BlendEnable: true.into(),
            LogicOpEnable: false.into(),
            SrcBlend: D3D12_BLEND_SRC_ALPHA,
            DestBlend: D3D12_BLEND_INV_SRC_ALPHA,
            BlendOp: D3D12_BLEND_OP_ADD,
            SrcBlendAlpha: D3D12_BLEND_ONE,
            DestBlendAlpha: D3D12_BLEND_INV_SRC_ALPHA,
            BlendOpAlpha: D3D12_BLEND_OP_ADD,
            LogicOp: D3D12_LOGIC_OP_NOOP,
            RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
        }; 8],
    }
}

#[cfg(target_os = "windows")]
fn rasterizer_desc() -> D3D12_RASTERIZER_DESC {
    D3D12_RASTERIZER_DESC {
        FillMode: D3D12_FILL_MODE_SOLID,
        CullMode: D3D12_CULL_MODE_NONE,
        FrontCounterClockwise: false.into(),
        DepthBias: 0,
        DepthBiasClamp: 0.0,
        SlopeScaledDepthBias: 0.0,
        DepthClipEnable: true.into(),
        MultisampleEnable: false.into(),
        AntialiasedLineEnable: false.into(),
        ForcedSampleCount: 0,
        ConservativeRaster: D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF,
    }
}

#[cfg(target_os = "windows")]
fn depth_stencil_desc() -> D3D12_DEPTH_STENCIL_DESC {
    let op = D3D12_DEPTH_STENCILOP_DESC {
        StencilFailOp: D3D12_STENCIL_OP_KEEP,
        StencilDepthFailOp: D3D12_STENCIL_OP_KEEP,
        StencilPassOp: D3D12_STENCIL_OP_KEEP,
        StencilFunc: D3D12_COMPARISON_FUNC_ALWAYS,
    };
    D3D12_DEPTH_STENCIL_DESC {
        DepthEnable: false.into(),
        DepthWriteMask: D3D12_DEPTH_WRITE_MASK_ZERO,
        DepthFunc: D3D12_COMPARISON_FUNC_LESS,
        StencilEnable: false.into(),
        StencilReadMask: 0,
        StencilWriteMask: 0,
        FrontFace: op,
        BackFace: op,
    }
}

#[cfg(target_os = "windows")]
unsafe fn create_upload_buffer(
    device: &ID3D12Device,
    size: usize,
) -> Result<ID3D12Resource, BackendError> {
    let heap_properties = D3D12_HEAP_PROPERTIES {
        Type: D3D12_HEAP_TYPE_UPLOAD,
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
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        Flags: D3D12_RESOURCE_FLAG_NONE,
    };

    device
        .CreateCommittedResource(
            &heap_properties,
            D3D12_HEAP_FLAG_NONE,
            &resource_desc,
            D3D12_RESOURCE_STATE_GENERIC_READ,
            None,
        )
        .map_err(|err| BackendError::Runtime(format!("create upload buffer failed: {err}")))
}

#[cfg(target_os = "windows")]
unsafe fn transition_barrier(
    resource: &ID3D12Resource,
    before: D3D12_RESOURCE_STATES,
    after: D3D12_RESOURCE_STATES,
) -> D3D12_RESOURCE_BARRIER {
    let mut barrier = D3D12_RESOURCE_BARRIER::default();
    barrier.Type = D3D12_RESOURCE_BARRIER_TYPE_TRANSITION;
    barrier.Flags = D3D12_RESOURCE_BARRIER_FLAG_NONE;
    barrier.Anonymous.Transition = std::mem::ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
        pResource: std::mem::ManuallyDrop::new(Some(resource.clone())),
        Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
        StateBefore: before,
        StateAfter: after,
    });
    barrier
}

fn build_sprite_vertices_by_pass_and_texture(
    graph: &RenderGraph,
    width: u32,
    height: u32,
) -> (Vec<SpriteVertex>, Vec<Vec<(TextureHandle, u32, u32)>>) {
    let mut vertices = Vec::new();
    let mut pass_groups = Vec::with_capacity(graph.passes.len());
    let half_width = width as f32 * 0.5;
    let half_height = height as f32 * 0.5;

    for pass in &graph.passes {
        let RenderGraphPass::Render(render) = pass else {
            pass_groups.push(Vec::new());
            continue;
        };
        let pass_start = vertices.len() as u32;
        let camera = &render.camera;
        let zoom = camera.zoom.max(0.05);

        let mut groups: Vec<(TextureHandle, u32, u32)> = Vec::new();
        let mut current_texture: Option<TextureHandle> = None;
        let mut current_start = pass_start;

        for batch in &render.batches {
            for sprite in &batch.sprites {
                if current_texture != Some(sprite.texture) {
                    if let Some(tex) = current_texture {
                        let count = vertices.len() as u32 - current_start;
                        if count > 0 {
                            groups.push((tex, current_start, count));
                        }
                    }
                    current_texture = Some(sprite.texture);
                    current_start = vertices.len() as u32;
                }

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

                vertices.extend_from_slice(&[
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
                ]);
            }
        }

        if let Some(tex) = current_texture {
            let count = vertices.len() as u32 - current_start;
            if count > 0 {
                groups.push((tex, current_start, count));
            }
        }

        pass_groups.push(groups);
    }

    (vertices, pass_groups)
}

const BUILTIN_VERTEX_SHADER: &str = r#"
struct VSInput { float2 position : POSITION; float4 color : COLOR; float2 uv : TEXCOORD; };
struct VSOutput { float4 position : SV_POSITION; float4 color : COLOR; float2 uv : TEXCOORD; };
VSOutput main(VSInput input) {
    VSOutput output;
    output.position = float4(input.position, 0.0, 1.0);
    output.color = input.color;
    output.uv = input.uv;
    return output;
}
"#;

const BUILTIN_PIXEL_SHADER: &str = r#"
struct PSInput { float4 position : SV_POSITION; float4 color : COLOR; float2 uv : TEXCOORD; };
Texture2D sprite_texture : register(t0);
SamplerState sprite_sampler : register(s0);
float4 main(PSInput input) : SV_TARGET { return sprite_texture.Sample(sprite_sampler, input.uv) * input.color; }
"#;
