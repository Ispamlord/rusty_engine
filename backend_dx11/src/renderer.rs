//! Real GPU sprite rendering for the DX11 backend.
#![cfg(target_os = "windows")]

use std::collections::HashMap;

use engine_render_api::{
    BackendError, Camera2d, ComputeDispatchNode, Material, RenderGraph, RenderGraphPass,
    RenderPassNode, RenderTargetHandle, SpriteInstance, TextureHandle,
};
use windows::core::PCSTR;
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCompile, D3DCOMPILE_ENABLE_STRICTNESS};
use windows::Win32::Graphics::Direct3D::{
    ID3DBlob, ID3DInclude, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11BlendState, ID3D11Buffer, ID3D11ClassLinkage, ID3D11ComputeShader, ID3D11Device,
    ID3D11DeviceContext, ID3D11InputLayout, ID3D11PixelShader, ID3D11RenderTargetView,
    ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11UnorderedAccessView,
    ID3D11VertexShader, D3D11_BIND_SHADER_RESOURCE, D3D11_BIND_UNORDERED_ACCESS,
    D3D11_BIND_VERTEX_BUFFER, D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_ONE,
    D3D11_BLEND_OP_ADD, D3D11_BLEND_SRC_ALPHA, D3D11_BUFFER_DESC, D3D11_COLOR_WRITE_ENABLE_ALL,
    D3D11_CPU_ACCESS_WRITE, D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_INPUT_ELEMENT_DESC,
    D3D11_INPUT_PER_VERTEX_DATA, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_WRITE_DISCARD,
    D3D11_RENDER_TARGET_BLEND_DESC, D3D11_RESOURCE_MISC_BUFFER_STRUCTURED,
    D3D11_SAMPLER_DESC, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC,
    D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_USAGE_DEFAULT, D3D11_USAGE_DYNAMIC,
    D3D11_USAGE_IMMUTABLE, D3D11_VIEWPORT,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R8G8B8A8_UNORM,
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

const MAX_SPRITES: usize = 4096;
const MAX_VERTICES: usize = MAX_SPRITES * 6;
const VERTEX_BUFFER_SIZE: usize = MAX_VERTICES * std::mem::size_of::<SpriteVertex>();

const CLEAR_COLOR: [f32; 4] = [0.04, 0.09, 0.08, 1.0];

const VERTEX_SHADER_SOURCE: &str = r#"
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

const PIXEL_SHADER_SOURCE: &str = r#"
Texture2D sprite_texture : register(t0);
SamplerState sprite_sampler : register(s0);
struct PSInput { float4 position : SV_POSITION; float4 color : COLOR; float2 uv : TEXCOORD; };
float4 main(PSInput input) : SV_TARGET {
    return sprite_texture.Sample(sprite_sampler, input.uv) * input.color;
}
"#;

/// GPU-side state needed to record real sprite draw work.
pub struct Dx11Renderer {
    device: ID3D11Device,
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    input_layout: ID3D11InputLayout,
    vertex_buffer: ID3D11Buffer,
    blend_state: ID3D11BlendState,
    default_texture: crate::Dx11Texture,
    default_storage_buffer: ID3D11Buffer,
    default_uav: ID3D11UnorderedAccessView,
    shader_bytecodes: HashMap<String, Vec<u8>>,
    pixel_shaders: HashMap<String, ID3D11PixelShader>,
    compute_shaders: HashMap<String, ID3D11ComputeShader>,
}

impl Dx11Renderer {
    pub fn new(device: &ID3D11Device) -> Result<Self, BackendError> {
        unsafe {
            let (vertex_shader, pixel_shader, input_layout) = compile_shaders(device)?;
            let vertex_buffer = create_dynamic_vertex_buffer(device)?;
            let blend_state = create_blend_state(device)?;
            let default_texture = crate::Dx11Backend::create_dx11_texture(device, 1, 1, None)?;
            let (default_storage_buffer, default_uav) =
                create_default_storage_buffer(device, 256)?;

            Ok(Self {
                device: device.clone(),
                vertex_shader,
                pixel_shader,
                input_layout,
                vertex_buffer,
                blend_state,
                default_texture,
                default_storage_buffer,
                default_uav,
                shader_bytecodes: HashMap::new(),
                pixel_shaders: HashMap::new(),
                compute_shaders: HashMap::new(),
            })
        }
    }

    /// Stores a shader bytecode blob for later use by render/compute passes.
    pub fn preload_shader_bytecode(&mut self, name: &str, _entry_point: &str, bytecode: &[u8]) {
        self.shader_bytecodes
            .insert(name.to_string(), bytecode.to_vec());
    }

    /// Releases cached shaders and bytecode. Safe to call multiple times.
    pub fn destroy(&mut self) {
        self.shader_bytecodes.clear();
        self.pixel_shaders.clear();
        self.compute_shaders.clear();
    }

    /// Records actual GPU commands for the render graph into the immediate context.
    pub fn record_render_graph(
        &mut self,
        context: &ID3D11DeviceContext,
        graph: &RenderGraph,
        textures: &HashMap<TextureHandle, crate::Dx11Texture>,
        render_targets: &HashMap<RenderTargetHandle, crate::Dx11RenderTarget>,
        storage_buffers: &HashMap<String, crate::Dx11StorageBuffer>,
        default_render_target_view: &ID3D11RenderTargetView,
        default_width: u32,
        default_height: u32,
    ) -> Result<(), BackendError> {
        unsafe {
            context.IASetInputLayout(&self.input_layout);
            context.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            context.VSSetShader(&self.vertex_shader, None);
            context.OMSetBlendState(&self.blend_state, Some(&[1.0f32; 4]), 0xffffffff);

            for pass in &graph.passes {
                match pass {
                    RenderGraphPass::Render(render) => {
                        let (rtv, width, height) = match render.target {
                            Some(handle) => {
                                let target = render_targets.get(&handle).ok_or_else(|| {
                                    BackendError::Runtime(format!(
                                        "render target {handle:?} not found"
                                    ))
                                })?;
                                (&target.rtv,
                                    target.width,
                                    target.height,
                                )
                            }
                            None => (default_render_target_view, default_width, default_height),
                        };
                        self.record_render_pass(
                            context,
                            render,
                            textures,
                            rtv,
                            width,
                            height,
                        )?;
                    }
                    RenderGraphPass::Compute(compute) => {
                        self.record_compute_pass(context, compute, storage_buffers)?;
                    }
                }
            }
        }

        Ok(())
    }

    unsafe fn record_render_pass(
        &mut self,
        context: &ID3D11DeviceContext,
        render: &RenderPassNode,
        textures: &HashMap<TextureHandle, crate::Dx11Texture>,
        render_target_view: &ID3D11RenderTargetView,
        width: u32,
        height: u32,
    ) -> Result<(), BackendError> {
        context.ClearRenderTargetView(render_target_view, &CLEAR_COLOR);

        let render_targets = [Some(render_target_view.clone())];
        context.OMSetRenderTargets(Some(&render_targets), None::<&ID3D11DepthStencilView>);

        let viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: width as f32,
            Height: height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        context.RSSetViewports(Some(&[viewport]));

        // Group sprites by texture so each group can be drawn with one
        // shader-resource binding.
        struct DrawGroup {
            texture: TextureHandle,
            vertex_offset: u32,
            vertex_count: u32,
        }

        let mut all_vertices: Vec<SpriteVertex> = Vec::new();
        let mut draw_groups: Vec<DrawGroup> = Vec::new();
        let half_width = width as f32 * 0.5;
        let half_height = height as f32 * 0.5;

        for batch in &render.batches {
            let mut groups: HashMap<TextureHandle, Vec<SpriteVertex>> = HashMap::new();
            for sprite in &batch.sprites {
                let vertices = build_sprite_vertices(sprite, &render.camera, half_width, half_height);
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
                    texture,
                    vertex_offset: offset,
                    vertex_count: count,
                });
            }
        }

        let total_vertices = all_vertices.len().min(MAX_VERTICES) as u32;
        if total_vertices > 0 {
            context.IASetVertexBuffers(0, 1, None, None, None);

            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context
                .Map(
                    &self.vertex_buffer,
                    0,
                    D3D11_MAP_WRITE_DISCARD,
                    0,
                    Some(&mut mapped),
                )
                .map_err(|err| BackendError::Runtime(format!("Map vertex buffer failed: {err}")))?;

            let bytes = bytemuck::cast_slice(&all_vertices[..total_vertices as usize]);
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.pData as *mut u8, bytes.len());
            context.Unmap(&self.vertex_buffer, 0);
        }

        let pixel_shader = self.resolve_pixel_shader(&render.material)?;
        context.PSSetShader(&pixel_shader, None);

        let stride = std::mem::size_of::<SpriteVertex>() as u32;
        let offset = 0u32;
        let vertex_buffers = [Some(self.vertex_buffer.clone())];
        context.IASetVertexBuffers(
            0,
            1,
            Some(vertex_buffers.as_ptr()),
            Some(&stride),
            Some(&offset),
        );

        for group in &draw_groups {
            let (srv, sampler) = textures
                .get(&group.texture)
                .map(|t| (&t.srv,
                    &t.sampler,
                ))
                .unwrap_or((&self.default_texture.srv,
                    &self.default_texture.sampler,
                ));
            context.PSSetShaderResources(0, Some(&[Some(srv.clone())]));
            context.PSSetSamplers(0, Some(&[Some(sampler.clone())]));
            context.Draw(group.vertex_count, group.vertex_offset);
        }

        Ok(())
    }

    unsafe fn record_compute_pass(
        &mut self,
        context: &ID3D11DeviceContext,
        compute: &ComputeDispatchNode,
        storage_buffers: &HashMap<String, crate::Dx11StorageBuffer>,
    ) -> Result<(), BackendError> {
        let uav = compute
            .material
            .bindings
            .first()
            .and_then(|binding| storage_buffers.get(&binding.resource))
            .map(|sb| &sb.uav)
            .unwrap_or(&self.default_uav);

        if compute.material.shader_asset.is_empty() {
            tracing::info!(
                "DX11 compute pass '{}' has no shader asset; skipping dispatch",
                compute.label
            );
            return Ok(());
        }

        let bytecode = match self.shader_bytecodes.get(&compute.material.shader_asset) {
            Some(bytecode) => bytecode,
            None => {
                tracing::info!(
                    "DX11 compute pass '{}' references unloaded shader '{}'; skipping dispatch",
                    compute.label,
                    compute.material.shader_asset
                );
                return Ok(());
            }
        };

        let shader = if let Some(shader) = self.compute_shaders.get(&compute.material.shader_asset)
        {
            shader.clone()
        } else {
            let shader = create_compute_shader_from_bytecode(&self.device, bytecode)?;
            self.compute_shaders
                .insert(compute.material.shader_asset.clone(), shader.clone());
            shader
        };

        context.CSSetShader(&shader, None);
        context.CSSetUnorderedAccessViews(0, 1, Some(&[Some(uav.clone())].as_ptr()), None);
        context.Dispatch(
            compute.dispatch[0],
            compute.dispatch[1],
            compute.dispatch[2],
        );

        Ok(())
    }

    fn resolve_pixel_shader(
        &mut self,
        material: &Option<Material>,
    ) -> Result<ID3D11PixelShader, BackendError> {
        let shader_asset = match material {
            Some(material) if !material.shader_asset.is_empty() => &material.shader_asset,
            _ => return Ok(self.pixel_shader.clone()),
        };

        if let Some(shader) = self.pixel_shaders.get(shader_asset) {
            return Ok(shader.clone());
        }

        let bytecode = match self.shader_bytecodes.get(shader_asset) {
            Some(bytecode) => bytecode,
            None => return Ok(self.pixel_shader.clone()),
        };

        let shader = unsafe { create_pixel_shader_from_bytecode(&self.device, bytecode)? };
        self.pixel_shaders
            .insert(shader_asset.clone(), shader.clone());
        Ok(shader)
    }
}

unsafe fn compile_shaders(
    device: &ID3D11Device,
) -> Result<(ID3D11VertexShader, ID3D11PixelShader, ID3D11InputLayout), BackendError> {
    let vs_blob = compile_shader(
        VERTEX_SHADER_SOURCE.as_bytes(),
        PCSTR::from_raw(b"main\0".as_ptr()),
        PCSTR::from_raw(b"vs_5_0\0".as_ptr()),
    )?;
    let ps_blob = compile_shader(
        PIXEL_SHADER_SOURCE.as_bytes(),
        PCSTR::from_raw(b"main\0".as_ptr()),
        PCSTR::from_raw(b"ps_5_0\0".as_ptr()),
    )?;

    let vertex_shader = create_vertex_shader(device, &vs_blob)?;
    let pixel_shader = create_pixel_shader(device, &ps_blob)?;
    let input_layout = create_input_layout(device, &vs_blob)?;

    Ok((vertex_shader, pixel_shader, input_layout))
}

unsafe fn compile_shader(
    source: &[u8],
    entry: PCSTR,
    target: PCSTR,
) -> Result<ID3DBlob, BackendError> {
    let mut code: Option<ID3DBlob> = None;
    let mut error: Option<ID3DBlob> = None;

    D3DCompile(
        source.as_ptr() as *const core::ffi::c_void,
        source.len(),
        PCSTR::null(),
        None,
        None::<&ID3DInclude>,
        entry,
        target,
        D3DCOMPILE_ENABLE_STRICTNESS,
        0,
        &mut code,
        Some(&mut error),
    )
    .map_err(|err| {
        let message = error_message(error.as_ref());
        BackendError::Runtime(format!("HLSL compile failed ({err}): {message}"))
    })?;

    code.ok_or_else(|| BackendError::Runtime("D3DCompile returned no shader blob".to_string()))
}

unsafe fn error_message(error: Option<&ID3DBlob>) -> String {
    let Some(blob) = error else {
        return String::new();
    };
    let ptr = blob.GetBufferPointer();
    let len = blob.GetBufferSize();
    if ptr.is_null() || len == 0 {
        return String::new();
    }
    let bytes = std::slice::from_raw_parts(ptr as *const u8, len);
    String::from_utf8_lossy(bytes).to_string()
}

unsafe fn shader_bytecode(blob: &ID3DBlob) -> &[u8] {
    let ptr = blob.GetBufferPointer() as *const u8;
    let len = blob.GetBufferSize();
    std::slice::from_raw_parts(ptr, len)
}

unsafe fn create_vertex_shader(
    device: &ID3D11Device,
    blob: &ID3DBlob,
) -> Result<ID3D11VertexShader, BackendError> {
    let mut shader: Option<ID3D11VertexShader> = None;
    device
        .CreateVertexShader(
            shader_bytecode(blob),
            None::<&ID3D11ClassLinkage>,
            Some(&mut shader),
        )
        .map_err(|err| BackendError::Runtime(format!("CreateVertexShader failed: {err}")))?;
    shader.ok_or_else(|| BackendError::Runtime("CreateVertexShader returned no shader".to_string()))
}

unsafe fn create_pixel_shader(
    device: &ID3D11Device,
    blob: &ID3DBlob,
) -> Result<ID3D11PixelShader, BackendError> {
    create_pixel_shader_from_bytecode(device, shader_bytecode(blob))
}

unsafe fn create_pixel_shader_from_bytecode(
    device: &ID3D11Device,
    bytecode: &[u8],
) -> Result<ID3D11PixelShader, BackendError> {
    let mut shader: Option<ID3D11PixelShader> = None;
    device
        .CreatePixelShader(bytecode, None::<&ID3D11ClassLinkage>, Some(&mut shader))
        .map_err(|err| {
            BackendError::Runtime(format!("CreatePixelShader from bytecode failed: {err}"))
        })?;
    shader.ok_or_else(|| BackendError::Runtime("CreatePixelShader returned no shader".to_string()))
}

unsafe fn create_compute_shader_from_bytecode(
    device: &ID3D11Device,
    bytecode: &[u8],
) -> Result<ID3D11ComputeShader, BackendError> {
    let mut shader: Option<ID3D11ComputeShader> = None;
    device
        .CreateComputeShader(bytecode, None::<&ID3D11ClassLinkage>, Some(&mut shader))
        .map_err(|err| {
            BackendError::Runtime(format!("CreateComputeShader from bytecode failed: {err}"))
        })?;
    shader
        .ok_or_else(|| BackendError::Runtime("CreateComputeShader returned no shader".to_string()))
}

unsafe fn create_input_layout(
    device: &ID3D11Device,
    vs_blob: &ID3DBlob,
) -> Result<ID3D11InputLayout, BackendError> {
    let elements = [
        D3D11_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR::from_raw(b"POSITION\0".as_ptr()),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: 0,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
        D3D11_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR::from_raw(b"COLOR\0".as_ptr()),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: std::mem::size_of::<[f32; 2]>() as u32,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
        D3D11_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR::from_raw(b"TEXCOORD\0".as_ptr()),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: std::mem::size_of::<[f32; 6]>() as u32,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
    ];

    let mut layout: Option<ID3D11InputLayout> = None;
    device
        .CreateInputLayout(&elements, shader_bytecode(vs_blob), Some(&mut layout))
        .map_err(|err| BackendError::Runtime(format!("CreateInputLayout failed: {err}")))?;
    layout.ok_or_else(|| BackendError::Runtime("CreateInputLayout returned no layout".to_string()))
}

unsafe fn create_dynamic_vertex_buffer(
    device: &ID3D11Device,
) -> Result<ID3D11Buffer, BackendError> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: VERTEX_BUFFER_SIZE as u32,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_VERTEX_BUFFER.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        MiscFlags: 0,
        StructureByteStride: 0,
    };

    let mut buffer: Option<ID3D11Buffer> = None;
    device
        .CreateBuffer(&desc, None, Some(&mut buffer))
        .map_err(|err| BackendError::Runtime(format!("CreateBuffer(vertex) failed: {err}")))?;
    buffer.ok_or_else(|| BackendError::Runtime("CreateBuffer returned no buffer".to_string()))
}

unsafe fn create_blend_state(device: &ID3D11Device) -> Result<ID3D11BlendState, BackendError> {
    let render_target = D3D11_RENDER_TARGET_BLEND_DESC {
        BlendEnable: true.into(),
        SrcBlend: D3D11_BLEND_SRC_ALPHA,
        DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
        BlendOp: D3D11_BLEND_OP_ADD,
        SrcBlendAlpha: D3D11_BLEND_ONE,
        DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
        BlendOpAlpha: D3D11_BLEND_OP_ADD,
        RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
    };

    let mut desc = D3D11_BLEND_DESC::default();
    desc.AlphaToCoverageEnable = false.into();
    desc.IndependentBlendEnable = false.into();
    desc.RenderTarget[0] = render_target;

    let mut state: Option<ID3D11BlendState> = None;
    device
        .CreateBlendState(&desc, Some(&mut state))
        .map_err(|err| BackendError::Runtime(format!("CreateBlendState failed: {err}")))?;
    state.ok_or_else(|| BackendError::Runtime("CreateBlendState returned no state".to_string()))
}

fn build_sprite_vertices_for_pass(
    render: &RenderPassNode,
    width: u32,
    height: u32,
) -> Vec<SpriteVertex> {
    let half_width = width as f32 * 0.5;
    let half_height = height as f32 * 0.5;
    let camera = &render.camera;

    render
        .batches
        .iter()
        .flat_map(|batch| &batch.sprites)
        .flat_map(|sprite| build_sprite_vertices(sprite, camera, half_width, half_height))
        .collect()
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

unsafe fn create_default_storage_buffer(
    device: &ID3D11Device,
    size: u32,
) -> Result<(ID3D11Buffer, ID3D11UnorderedAccessView), BackendError> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: size,
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_UNORDERED_ACCESS.0,
        CPUAccessFlags: 0,
        MiscFlags: D3D11_RESOURCE_MISC_BUFFER_STRUCTURED.0,
        StructureByteStride: 4,
    };

    let mut buffer: Option<ID3D11Buffer> = None;
    device
        .CreateBuffer(&desc,
            None,
            Some(&mut buffer),
        )
        .map_err(|err| BackendError::Runtime(format!("CreateBuffer(default storage) failed: {err}")))?;
    let buffer = buffer.ok_or_else(|| {
        BackendError::Runtime("CreateBuffer(default storage) returned no buffer".to_string())
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
        .CreateUnorderedAccessView(
            &buffer,
            Some(&uav_desc),
            Some(&mut uav),
        )
        .map_err(|err| BackendError::Runtime(format!("CreateUnorderedAccessView(default) failed: {err}")))?;
    let uav = uav.ok_or_else(|| {
        BackendError::Runtime("CreateUnorderedAccessView(default) returned no view".to_string())
    })?;

    Ok((buffer, uav))
}
