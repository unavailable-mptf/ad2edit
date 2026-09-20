//! GPU rendering of dupe geometry.
//!
//! This module depends on `wgpu` and nothing else from the UI stack, which is
//! deliberate: it means the whole renderer compiles and is checkable on its
//! own, with only the short handshake that fetches the device from eframe
//! living in the GUI binary.
//!
//! The scene is drawn into a texture this module owns, with its own depth
//! attachment, and that texture is handed to egui as an image. egui's own
//! render pass has no depth buffer, so drawing into it directly could not
//! depth-test — rendering offscreen is what makes real occlusion possible.

use std::collections::HashMap;
use std::sync::Arc;

use wgpu::util::DeviceExt;

/// One vertex as the shader wants it. `repr(C)` because it is uploaded
/// straight to the GPU.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// Baked light, multiplied into the colour unless fullbright. Map faces
    /// carry a lightmap sample here; models carry white.
    pub light: [f32; 3],
}

impl Vertex {
    pub const UNLIT: [f32; 3] = [1.0, 1.0, 1.0];
}

/// Per-draw data: the model matrix and a tint.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    tint: [f32; 4],
    /// x: 1 when a texture is bound. y: 1 for fullbright. z: clip count.
    flags: [f32; 4],
    /// Proper Clipping planes in MODEL space: xyz normal, w distance. A
    /// fragment is kept where dot(p, n) >= d — Source's PushCustomClipPlane
    /// convention, and since rotation preserves dot products the test can
    /// stay in model space instead of transforming every plane per frame.
    clips: [[f32; 4]; MAX_CLIPS],
}

pub const MAX_CLIPS: usize = 8;

const SHADER: &str = r#"
struct Uniforms {
    view_proj : mat4x4<f32>,
    model     : mat4x4<f32>,
    tint      : vec4<f32>,
    flags     : vec4<f32>,
    clips     : array<vec4<f32>, 8>,
};

@group(0) @binding(0) var<uniform> u : Uniforms;
@group(1) @binding(0) var t_diffuse  : texture_2d<f32>;
@group(1) @binding(1) var s_diffuse  : sampler;

struct VsOut {
    @builtin(position) clip   : vec4<f32>,
    @location(0)       normal : vec3<f32>,
    @location(1)       uv     : vec2<f32>,
    @location(2)       local  : vec3<f32>,
    @location(3)       light  : vec3<f32>,
};

@vertex
fn vs(
    @location(0) position : vec3<f32>,
    @location(1) normal   : vec3<f32>,
    @location(2) uv       : vec2<f32>,
    @location(3) light    : vec3<f32>,
) -> VsOut {
    var out : VsOut;
    let world = u.model * vec4<f32>(position, 1.0);
    out.clip = u.view_proj * world;
    // Rotation only: the model matrix carries no scale, so the upper 3x3
    // transforms normals correctly without an inverse transpose.
    out.normal = (u.model * vec4<f32>(normal, 0.0)).xyz;
    out.uv = uv;
    out.local = position;
    out.light = light;
    return out;
}

@fragment
fn fs(in : VsOut) -> @location(0) vec4<f32> {
    // Proper Clipping: discard on the far side of any active plane.
    let clip_count = i32(u.flags.z);
    for (var i = 0; i < 8; i = i + 1) {
        if (i < clip_count) {
            let c = u.clips[i];
            if (dot(in.local, c.xyz) < c.w) {
                discard;
            }
        }
    }
    let n = normalize(in.normal);
    let light = normalize(vec3<f32>(0.35, 0.5, 1.0));
    // Two-sided: Source models have inconsistent winding in places and a
    // one-sided term turns those faces black.
    let lambert = abs(dot(n, light));
    // Fullbright leaves albedo alone. Directional shading on a textured model
    // mostly just makes it hard to read, which is why SFM's own viewport has
    // a fullbright toggle too.
    // flags.w = 1 marks a lightmapped map batch: its vertex light already
    // holds the baked lighting, so no directional term on top.
    var shade = 1.0;
    if (u.flags.y < 0.5) {
        if (u.flags.w > 0.5) {
            shade = 1.0;
        } else {
            shade = 0.35 + 0.65 * lambert;
        }
    }
    var baked = vec3<f32>(1.0, 1.0, 1.0);
    if (u.flags.y < 0.5) {
        baked = in.light;
    }

    // The tint multiplies the texture, so an unselected textured prop is
    // given a white tint and shows its own colours untouched. Tinting it grey
    // is what made fullbright still look dark.
    var albedo = u.tint.rgb;
    var alpha = u.tint.a;
    if (u.flags.x > 0.5) {
        let texel = textureSample(t_diffuse, s_diffuse, in.uv);
        albedo = texel.rgb * u.tint.rgb;
        alpha = alpha * texel.a;
    }
    // A cut-out texel, or a part faded right out, draws nothing. Textures
    // arrive with their alpha already set to what the material means by it.
    if (alpha < 0.04) {
        discard;
    }
    return vec4<f32>(albedo * shade * baked, alpha);
}
"#;

/// A model's geometry living on the GPU.
pub struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    /// (first index, count, texture) per material section. One texture for
    /// the whole model put a body skin on a ragdoll's face; a model has as
    /// many textures as it has materials.
    sections: Vec<(u32, u32, Option<wgpu::BindGroup>, bool)>,
}

/// One thing to draw this frame.
pub struct Instance {
    pub mesh: Arc<GpuMesh>,
    pub model: [[f32; 4]; 4],
    pub tint: [f32; 4],
    /// Model-space clip planes (normal xyz, distance w). Up to MAX_CLIPS.
    pub clips: Vec<[f32; 4]>,
}

pub struct Renderer {
    pipeline: wgpu::RenderPipeline,
    blend_pipeline: wgpu::RenderPipeline,
    uniform_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// A single white pixel, bound when a mesh has no texture so the shader
    /// needs no second pipeline.
    blank: wgpu::BindGroup,

    colour: Option<wgpu::Texture>,
    colour_view: Option<wgpu::TextureView>,
    depth_view: Option<wgpu::TextureView>,
    size: (u32, u32),
    format: wgpu::TextureFormat,

    /// Uniform buffer reused across draws, offset per instance.
    uniforms: wgpu::Buffer,
    uniform_group: wgpu::BindGroup,
    capacity: usize,
}

/// Uniform buffer offsets must be a multiple of this on most backends.
const UNIFORM_STRIDE: u64 = 512;

impl Renderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Renderer {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ad2edit scene"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ad2edit uniforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(
                        std::mem::size_of::<Uniforms>() as u64
                    ),
                },
                count: None,
            }],
        });

        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ad2edit texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ad2edit layout"),
            bind_group_layouts: &[Some(&uniform_layout), Some(&texture_layout)],
            immediate_size: 0,
        });

        let build_pipeline = |label: &str, blend: Option<wgpu::BlendState>, writes_depth: bool| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                        wgpu::VertexAttribute {
                            offset: 12,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                        wgpu::VertexAttribute {
                            offset: 24,
                            shader_location: 2,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        wgpu::VertexAttribute {
                            offset: 32,
                            shader_location: 3,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // No back-face culling: Source model winding is not reliable
                // enough for it, and the shader is two-sided anyway.
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(writes_depth),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let pipeline = build_pipeline("ad2edit pipeline", None, true);
        // See-through surfaces go over the solid scene without writing depth,
        // so one pane of glass never hides another. Destination alpha is left
        // at 1: egui composites this texture, and a hole in its alpha would
        // show the panel behind the viewport.
        let see_through = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let blend_pipeline = build_pipeline("ad2edit see-through pipeline", Some(see_through), false);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ad2edit sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        // One white pixel, bound whenever a mesh has no texture. Cheaper than
        // a second pipeline and keeps the shader branch-free on the host side.
        let blank = make_texture_group(
            device,
            queue,
            &texture_layout,
            &sampler,
            1,
            1,
            &[255, 255, 255, 255],
        );

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ad2edit uniforms"),
            size: UNIFORM_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ad2edit uniform group"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniforms,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<Uniforms>() as u64),
                }),
            }],
        });

        Renderer {
            pipeline,
            blend_pipeline,
            uniform_layout,
            texture_layout,
            sampler,
            blank,
            colour: None,
            colour_view: None,
            depth_view: None,
            size: (0, 0),
            format,
            uniforms,
            uniform_group,
            capacity: 1,
        }
    }
}

/// Builds a bind group around an RGBA8 image.
pub fn make_texture_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> wgpu::BindGroup {
    let size = wgpu::Extent3d {
        width: width.max(1),
        height: height.max(1),
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ad2edit diffuse"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // NOT Srgb. egui renders to a non-sRGB framebuffer, so an sRGB texture
        // here would be decoded to linear on sample and then displayed as if
        // it were sRGB — gamma-crushed and dark. Passing values through
        // unchanged is also exactly what Source's fullbright does.
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * size.width),
            rows_per_image: Some(size.height),
        },
        size,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ad2edit texture group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

impl GpuMesh {
    /// Whether any section has a diffuse texture, so the caller can decide
    /// not to tint one that already has its own colours.
    pub fn has_texture(&self) -> bool {
        self.sections.iter().any(|(_, _, t, _)| t.is_some())
    }
}

impl Renderer {
    /// Uploads geometry, optionally with a diffuse texture.
    /// `sections` is (first index, count, texture, blended) per material;
    /// pass one covering everything for a single-material mesh. A blended
    /// section has alpha between solid and clear in its texture and is drawn
    /// in the second pass, over everything solid.
    pub fn upload_mesh(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vertices: &[Vertex],
        indices: &[u32],
        sections: &[(u32, u32, Option<(u32, u32, &[u8])>, bool)],
    ) -> Arc<GpuMesh> {
        let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ad2edit vertices"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ad2edit indices"),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let sections = sections
            .iter()
            .map(|(start, count, tex, blended)| {
                let group = tex.map(|(w, h, rgba)| {
                    make_texture_group(device, queue, &self.texture_layout, &self.sampler, w, h, rgba)
                });
                (*start, *count, group, *blended)
            })
            .collect();
        Arc::new(GpuMesh {
            vertices: vb,
            indices: ib,
            sections,
        })
    }

    /// The texture egui draws. None until the first render.
    pub fn colour_view(&self) -> Option<&wgpu::TextureView> {
        self.colour_view.as_ref()
    }

    fn ensure_target(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let (w, h) = (width.max(1), height.max(1));
        if self.size == (w, h) && self.colour.is_some() {
            return;
        }
        let colour = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ad2edit scene colour"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ad2edit scene depth"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.colour_view = Some(colour.create_view(&Default::default()));
        self.depth_view = Some(depth.create_view(&Default::default()));
        self.colour = Some(colour);
        self.size = (w, h);
    }

    /// Grows the uniform buffer so every instance gets its own slice at a
    /// correctly aligned offset.
    fn ensure_uniforms(&mut self, device: &wgpu::Device, count: usize) {
        if count <= self.capacity {
            return;
        }
        let capacity = count.next_power_of_two();
        self.uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ad2edit uniforms"),
            size: UNIFORM_STRIDE * capacity as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.uniform_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ad2edit uniform group"),
            layout: &self.uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &self.uniforms,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<Uniforms>() as u64),
                }),
            }],
        });
        self.capacity = capacity;
    }

    /// Draws the scene into this module's own target, depth-tested.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        width: u32,
        height: u32,
        view_proj: [[f32; 4]; 4],
        background: [f64; 3],
        fullbright: bool,
        // Map vertices carry baked light: skip the directional term for them.
        map_lit: bool,
        instances: &[Instance],
        map: &[MapDraw<'_>],
    ) {
        self.ensure_target(device, width, height);
        self.ensure_uniforms(device, instances.len() + 2);

        // Slot 0 is the map: identity model matrix, white tint. It's written
        // even when there's no map, so the offsets below never shift.
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        for textured in [false, true] {
            // Two map slots: 0 for textured batches, 1 for untextured ones,
            // since the texture flag lives in the uniform.
            // An untextured map face should read as concrete-ish grey, not
            // blown-out white — the white walls in the first construct load
            // were exactly this slot's tint.
            let tint = if textured { [1.0, 1.0, 1.0, 1.0] } else { [0.55, 0.55, 0.56, 1.0] };
            let u = Uniforms {
                view_proj,
                model: identity,
                tint,
                flags: [if textured { 1.0 } else { 0.0 }, if fullbright { 1.0 } else { 0.0 }, 0.0, if map_lit { 1.0 } else { 0.0 }],
                clips: [[0.0; 4]; MAX_CLIPS],
            };
            let slot = if textured { 0 } else { 1 };
            queue.write_buffer(&self.uniforms, slot * UNIFORM_STRIDE, bytemuck::bytes_of(&u));
        }

        // Then one slice per instance, after the two map slots.
        for (i, inst) in instances.iter().enumerate() {
            let i = i + 2;
            let mut clips = [[0.0f32; 4]; MAX_CLIPS];
            for (k, c) in inst.clips.iter().take(MAX_CLIPS).enumerate() {
                clips[k] = *c;
            }
            let u = Uniforms {
                view_proj,
                model: inst.model,
                tint: inst.tint,
                flags: [
                    if inst.mesh.has_texture() { 1.0 } else { 0.0 },
                    if fullbright { 1.0 } else { 0.0 },
                    inst.clips.len().min(MAX_CLIPS) as f32,
                    0.0,
                ],
                clips,
            };
            queue.write_buffer(
                &self.uniforms,
                i as u64 * UNIFORM_STRIDE,
                bytemuck::bytes_of(&u),
            );
        }

        let (Some(colour), Some(depth)) = (self.colour_view.as_ref(), self.depth_view.as_ref())
        else {
            return;
        };

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ad2edit scene pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: colour,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: background[0],
                        g: background[1],
                        b: background[2],
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        pass.set_pipeline(&self.pipeline);

        for draw in map {
            let slot = if draw.batch.texture.is_some() { 0 } else { 1 };
            pass.set_bind_group(0, &self.uniform_group, &[(slot * UNIFORM_STRIDE) as u32]);
            pass.set_bind_group(1, draw.batch.texture.as_ref().unwrap_or(&self.blank), &[]);
            pass.set_vertex_buffer(0, draw.batch.vertices.slice(..));
            pass.set_index_buffer(draw.batch.indices.slice(..), wgpu::IndexFormat::Uint32);
            for (start, count) in draw.ranges {
                pass.draw_indexed(*start..*start + *count, 0, 0..1);
            }
        }

        // A part faded by the Colour tool is see-through as a whole; otherwise
        // only the sections whose material blends are.
        let faded = |inst: &Instance| inst.tint[3] < 0.999;
        let draw = |pass: &mut wgpu::RenderPass, i: usize, inst: &Instance, see_through: bool| {
            let mut bound = false;
            for (start, count, tex, blended) in &inst.mesh.sections {
                if (faded(inst) || *blended) != see_through {
                    continue;
                }
                if !bound {
                    pass.set_bind_group(0, &self.uniform_group, &[((i + 2) as u64 * UNIFORM_STRIDE) as u32]);
                    pass.set_vertex_buffer(0, inst.mesh.vertices.slice(..));
                    pass.set_index_buffer(inst.mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    bound = true;
                }
                pass.set_bind_group(1, tex.as_ref().unwrap_or(&self.blank), &[]);
                pass.draw_indexed(*start..*start + *count, 0, 0..1);
            }
        };
        for (i, inst) in instances.iter().enumerate() {
            draw(&mut pass, i, inst, false);
        }
        // Far to near, by each part's depth in clip space, so nearer glass
        // goes over farther glass.
        let depth = |inst: &Instance| {
            let at = inst.model[3];
            view_proj[0][3] * at[0] + view_proj[1][3] * at[1] + view_proj[2][3] * at[2] + view_proj[3][3]
        };
        let mut far_to_near: Vec<(usize, f32)> = instances
            .iter()
            .enumerate()
            .filter(|(_, inst)| faded(inst) || inst.mesh.sections.iter().any(|section| section.3))
            .map(|(i, inst)| (i, depth(inst)))
            .collect();
        far_to_near.sort_by(|a, b| b.1.total_cmp(&a.1));
        if !far_to_near.is_empty() {
            pass.set_pipeline(&self.blend_pipeline);
            for (i, _) in far_to_near {
                draw(&mut pass, i, &instances[i], true);
            }
        }
    }
}

/// Column-major, because WGSL reads consecutive groups of four floats from a
/// uniform buffer as the COLUMNS of a mat4x4.
fn transpose(m: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0f32; 4]; 4];
    for r in 0..4 {
        for c in 0..4 {
            out[c][r] = m[r][c];
        }
    }
    out
}

fn mul4(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0f32; 4]; 4];
    for r in 0..4 {
        for c in 0..4 {
            out[r][c] = (0..4).map(|k| a[r][k] * b[k][c]).sum();
        }
    }
    out
}

/// World-to-clip, ready to upload.
///
/// `right`, `up` and `fwd` are the camera basis and `eye` its position. The
/// projection maps depth into 0..1 rather than -1..1, which is wgpu's
/// convention and not OpenGL's.
pub fn view_projection(
    eye: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    fwd: [f32; 3],
    fov_y: f32,
    aspect: f32,
    near: f32,
    far: f32,
) -> [[f32; 4]; 4] {
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let view = [
        [right[0], right[1], right[2], -dot(right, eye)],
        [up[0], up[1], up[2], -dot(up, eye)],
        [-fwd[0], -fwd[1], -fwd[2], dot(fwd, eye)],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let f = 1.0 / (fov_y * 0.5).tan();
    let proj = [
        [f / aspect.max(1e-6), 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), near * far / (near - far)],
        [0.0, 0.0, -1.0, 0.0],
    ];
    transpose(mul4(proj, view))
}

/// A rotation-and-translation matrix built from the entity's own rotated basis
/// vectors, so it shares the convention the editing code already uses rather
/// than deriving one here.
pub fn model_matrix(x: [f32; 3], y: [f32; 3], z: [f32; 3], pos: [f32; 3]) -> [[f32; 4]; 4] {
    [
        [x[0], x[1], x[2], 0.0],
        [y[0], y[1], y[2], 0.0],
        [z[0], z[1], z[2], 0.0],
        [pos[0], pos[1], pos[2], 1.0],
    ]
}

/// A map's material batch on the GPU. Drawn as index ranges, so culling
/// decides which slices of one buffer get issued rather than which of ten
/// thousand buffers get bound.
pub struct MapBatch {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    texture: Option<wgpu::BindGroup>,
}

impl MapBatch {
    pub fn has_texture(&self) -> bool {
        self.texture.is_some()
    }
}

/// One map batch plus the index ranges to draw from it this frame.
pub struct MapDraw<'a> {
    pub batch: &'a MapBatch,
    pub ranges: &'a [(u32, u32)],
}

impl Renderer {
    pub fn upload_map_batch(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vertices: &[Vertex],
        indices: &[u32],
        texture: Option<(u32, u32, &[u8])>,
    ) -> MapBatch {
        let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ad2edit map vertices"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ad2edit map indices"),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let texture = texture.map(|(w, h, rgba)| {
            // A map texture's alpha is a gloss or blend mask, never a hole.
            let mut solid = rgba.to_vec();
            solid.chunks_exact_mut(4).for_each(|pixel| pixel[3] = 255);
            make_texture_group(device, queue, &self.texture_layout, &self.sampler, w, h, &solid)
        });
        MapBatch {
            vertices: vb,
            indices: ib,
            texture,
        }
    }
}

pub type MeshCache = HashMap<String, Arc<GpuMesh>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(m: [[f32; 4]; 4], v: [f32; 4]) -> [f32; 4] {
        // m is column-major, so column j scaled by v[j].
        let mut out = [0.0f32; 4];
        for j in 0..4 {
            for i in 0..4 {
                out[i] += m[j][i] * v[j];
            }
        }
        out
    }

    #[test]
    fn a_point_at_the_eye_plus_forward_lands_in_the_middle() {
        let eye = [0.0, 0.0, 0.0];
        let m = view_projection(
            eye,
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, -1.0],
            60f32.to_radians(),
            1.0,
            1.0,
            1000.0,
        );
        // Ten units straight down the view direction.
        let clip = apply(m, [0.0, 0.0, -10.0, 1.0]);
        assert!(clip[3] > 0.0, "should be in front of the camera");
        let ndc = [clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]];
        assert!(ndc[0].abs() < 1e-5 && ndc[1].abs() < 1e-5, "centred: {ndc:?}");
        assert!(
            (0.0..=1.0).contains(&ndc[2]),
            "wgpu depth is 0..1, got {}",
            ndc[2]
        );
    }

    #[test]
    fn nearer_points_get_smaller_depth() {
        let m = view_projection(
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, -1.0],
            60f32.to_radians(),
            1.6,
            1.0,
            1000.0,
        );
        let near = apply(m, [0.0, 0.0, -5.0, 1.0]);
        let far = apply(m, [0.0, 0.0, -500.0, 1.0]);
        assert!(
            near[2] / near[3] < far[2] / far[3],
            "depth must increase with distance for a Less test"
        );
    }

    #[test]
    fn the_model_matrix_places_and_rotates() {
        // A quarter turn about Z: x becomes y.
        let m = model_matrix(
            [0.0, 1.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [10.0, 20.0, 30.0],
        );
        let p = apply(m, [1.0, 0.0, 0.0, 1.0]);
        assert!((p[0] - 10.0).abs() < 1e-5, "{p:?}");
        assert!((p[1] - 21.0).abs() < 1e-5, "{p:?}");
        assert!((p[2] - 30.0).abs() < 1e-5, "{p:?}");
    }
}

#[cfg(test)]
mod shader_tests {
    /// The editor only compiles this when a dupe is opened, so a mistake in
    /// it would otherwise surface as a crash on the owner's machine.
    #[test]
    fn the_shader_parses_and_validates() {
        use wgpu::naga;
        let module = naga::front::wgsl::parse_str(super::SHADER).expect("the WGSL parses");
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .expect("the WGSL validates");
    }
}
