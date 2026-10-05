use futures::executor::block_on;
use std::sync::Arc;
use wgpu::util::DeviceExt;
use wgpu::*;
use tempo_media::VideoFrame;
use tempo_timeline::types::TransitionKind;
use crate::error::{RenderError, Result};

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LayerUniforms {
    pub transform: [f32; 16],
    pub lift: [f32; 4],
    pub gamma: [f32; 4],
    pub gain: [f32; 4],
    pub params: [f32; 4],
}

impl Default for LayerUniforms {
    fn default() -> Self {
        let mut transform = [0.0f32; 16];
        transform[0] = 1.0;
        transform[5] = 1.0;
        transform[10] = 1.0;
        transform[15] = 1.0;
        Self {
            transform,
            lift: [0.0, 0.0, 0.0, 0.0],
            gamma: [1.0, 1.0, 1.0, 0.0],
            gain: [1.0, 1.0, 1.0, 0.0],
            params: [1.0, 0.0, 0.0, 0.0],
        }
    }
}

impl LayerUniforms {
    pub fn new_transform(
        offset: [f32; 2],
        scale: [f32; 2],
        rotation_deg: f32,
        opacity: f32,
        lift: [f32; 3],
        gamma: [f32; 3],
        gain: [f32; 3],
    ) -> Self {
        let rad = rotation_deg.to_radians();
        let cos = rad.cos();
        let sin = rad.sin();

        let mut transform = [0.0f32; 16];
        transform[0] = scale[0] * cos;
        transform[1] = scale[0] * sin;
        transform[4] = -scale[1] * sin;
        transform[5] = scale[1] * cos;
        transform[10] = 1.0;
        transform[12] = offset[0];
        transform[13] = offset[1];
        transform[15] = 1.0;

        Self {
            transform,
            lift: [lift[0], lift[1], lift[2], 0.0],
            gamma: [gamma[0], gamma[1], gamma[2], 0.0],
            gain: [gain[0], gain[1], gain[2], 0.0],
            params: [opacity, 0.0, 0.0, 0.0],
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TransitionUniforms {
    pub progress: f32,
    pub kind: u32,
    pub _pad: [f32; 2],
}

#[derive(Clone, Debug)]
pub struct LayerDesc<'a> {
    pub texture_view: &'a TextureView,
    pub opacity: f32,
    pub scale: [f32; 2],
    pub offset: [f32; 2],
    pub rotation: f32,
    pub lift: [f32; 3],
    pub gamma: [f32; 3],
    pub gain: [f32; 3],
}

impl<'a> LayerDesc<'a> {
    pub fn new(texture_view: &'a TextureView) -> Self {
        Self {
            texture_view,
            opacity: 1.0,
            scale: [1.0, 1.0],
            offset: [0.0, 0.0],
            rotation: 0.0,
            lift: [0.0, 0.0, 0.0],
            gamma: [1.0, 1.0, 1.0],
            gain: [1.0, 1.0, 1.0],
        }
    }

    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }

    pub fn with_scale(mut self, scale_x: f32, scale_y: f32) -> Self {
        self.scale = [scale_x, scale_y];
        self
    }

    pub fn with_offset(mut self, offset_x: f32, offset_y: f32) -> Self {
        self.offset = [offset_x, offset_y];
        self
    }

    pub fn with_rotation(mut self, rotation_deg: f32) -> Self {
        self.rotation = rotation_deg;
        self
    }

    pub fn with_color_correction(mut self, lift: [f32; 3], gamma: [f32; 3], gain: [f32; 3]) -> Self {
        self.lift = lift;
        self.gamma = gamma;
        self.gain = gain;
        self
    }

    pub fn to_uniforms(&self) -> LayerUniforms {
        LayerUniforms::new_transform(
            self.offset,
            self.scale,
            self.rotation,
            self.opacity,
            self.lift,
            self.gamma,
            self.gain,
        )
    }
}

pub struct WgpuRenderer {
    pub instance: Instance,
    pub adapter: Adapter,
    pub device: Arc<Device>,
    pub queue: Arc<Queue>,
    pipeline: RenderPipeline,
    bind_group_layout: BindGroupLayout,
    layer_pipeline: RenderPipeline,
    layer_bind_group_layout: BindGroupLayout,
    transition_pipeline: RenderPipeline,
    transition_bind_group_layout: BindGroupLayout,
    sampler: Sampler,
}

impl WgpuRenderer {
    pub fn new() -> Result<Self> {
        let instance = Instance::default();

        let adapter = block_on(instance.request_adapter(&RequestAdapterOptions {
            power_preference: PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok_or(RenderError::AdapterRequestFailed)?;

        let (device, queue) = block_on(adapter.request_device(
            &DeviceDescriptor {
                label: Some("Tempo wgpu Device"),
                required_features: Features::empty(),
                required_limits: Limits::downlevel_webgl2_defaults(),
                memory_hints: MemoryHints::Performance,
            },
            None,
        ))?;

        let device = Arc::new(device);
        let queue = Arc::new(queue);

        // 1. Compile fullscreen quad WGSL shader
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("Fullscreen Video Quad Shader"),
            source: ShaderSource::Wgsl(
                r#"
                struct VertexOutput {
                    @builtin(position) position: vec4<f32>,
                    @location(0) uv: vec2<f32>,
                };

                @vertex
                fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
                    var out: VertexOutput;
                    let x = f32(i32(in_vertex_index & 1u) * 4 - 1);
                    let y = f32(i32(in_vertex_index & 2u) * 2 - 1);
                    out.position = vec4<f32>(x, y, 0.0, 1.0);
                    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
                    return out;
                }

                @group(0) @binding(0) var t_diffuse: texture_2d<f32>;
                @group(0) @binding(1) var s_diffuse: sampler;

                @fragment
                fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
                    return textureSample(t_diffuse, s_diffuse, in.uv);
                }
                "#
                .into(),
            ),
        });

        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Video Texture Bind Group Layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Fullscreen Video Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("Fullscreen Video Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: PipelineCompilationOptions::default(),
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(ColorTargetState {
                    format: TextureFormat::Rgba8Unorm,
                    blend: Some(BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: PipelineCompilationOptions::default(),
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // 2. Layer Compositing Pipeline (alpha blending + matrix transform + color correction)
        let layer_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("MultiTrack Layer Quad Shader"),
            source: ShaderSource::Wgsl(
                r#"
                struct LayerUniforms {
                    transform: mat4x4<f32>,
                    lift: vec4<f32>,
                    gamma: vec4<f32>,
                    gain: vec4<f32>,
                    params: vec4<f32>,
                };

                struct VertexOutput {
                    @builtin(position) position: vec4<f32>,
                    @location(0) uv: vec2<f32>,
                };

                @group(0) @binding(2) var<uniform> u_layer: LayerUniforms;

                @vertex
                fn vs_layer(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
                    var out: VertexOutput;
                    var pos = array<vec2<f32>, 6>(
                        vec2<f32>(-1.0, -1.0),
                        vec2<f32>( 1.0, -1.0),
                        vec2<f32>(-1.0,  1.0),
                        vec2<f32>(-1.0,  1.0),
                        vec2<f32>( 1.0, -1.0),
                        vec2<f32>( 1.0,  1.0),
                    );
                    let p = pos[in_vertex_index];
                    out.position = u_layer.transform * vec4<f32>(p, 0.0, 1.0);
                    out.uv = vec2<f32>((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5);
                    return out;
                }

                @group(0) @binding(0) var t_diffuse: texture_2d<f32>;
                @group(0) @binding(1) var s_diffuse: sampler;

                @fragment
                fn fs_layer(in: VertexOutput) -> @location(0) vec4<f32> {
                    let color = textureSample(t_diffuse, s_diffuse, in.uv);
                    let opacity = u_layer.params.x;
                    let c_lift = color.rgb * (vec3<f32>(1.0, 1.0, 1.0) - u_layer.lift.rgb) + u_layer.lift.rgb;
                    let inv_gamma = 1.0 / max(u_layer.gamma.rgb, vec3<f32>(0.001, 0.001, 0.001));
                    let c_gamma = pow(max(c_lift, vec3<f32>(0.0, 0.0, 0.0)), inv_gamma);
                    let c_gain = c_gamma * u_layer.gain.rgb;
                    let graded = clamp(c_gain, vec3<f32>(0.0, 0.0, 0.0), vec3<f32>(1.0, 1.0, 1.0));
                    return vec4<f32>(graded, color.a * opacity);
                }
                "#
                .into(),
            ),
        });

        let layer_bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Layer Texture Bind Group Layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let layer_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Layer Pipeline Layout"),
            bind_group_layouts: &[&layer_bind_group_layout],
            push_constant_ranges: &[],
        });

        let layer_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("Layer Compositing Pipeline"),
            layout: Some(&layer_pipeline_layout),
            vertex: VertexState {
                module: &layer_shader,
                entry_point: "vs_layer",
                buffers: &[],
                compilation_options: PipelineCompilationOptions::default(),
            },
            fragment: Some(FragmentState {
                module: &layer_shader,
                entry_point: "fs_layer",
                targets: &[Some(ColorTargetState {
                    format: TextureFormat::Rgba8Unorm,
                    blend: Some(BlendState {
                        color: BlendComponent {
                            src_factor: BlendFactor::SrcAlpha,
                            dst_factor: BlendFactor::OneMinusSrcAlpha,
                            operation: BlendOperation::Add,
                        },
                        alpha: BlendComponent {
                            src_factor: BlendFactor::One,
                            dst_factor: BlendFactor::OneMinusSrcAlpha,
                            operation: BlendOperation::Add,
                        },
                    }),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: PipelineCompilationOptions::default(),
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // 3. Transition Pipeline
        let transition_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("Transition Shader Module"),
            source: ShaderSource::Wgsl(
                r#"
                struct TransitionUniforms {
                    progress: f32,
                    kind: u32,
                    _pad0: f32,
                    _pad1: f32,
                };

                struct VertexOutput {
                    @builtin(position) position: vec4<f32>,
                    @location(0) uv: vec2<f32>,
                };

                @vertex
                fn vs_trans(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
                    var out: VertexOutput;
                    let x = f32(i32(in_vertex_index & 1u) * 4 - 1);
                    let y = f32(i32(in_vertex_index & 2u) * 2 - 1);
                    out.position = vec4<f32>(x, y, 0.0, 1.0);
                    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
                    return out;
                }

                @group(0) @binding(0) var t_a: texture_2d<f32>;
                @group(0) @binding(1) var t_b: texture_2d<f32>;
                @group(0) @binding(2) var s_sampler: sampler;
                @group(0) @binding(3) var<uniform> u_trans: TransitionUniforms;

                @fragment
                fn fs_trans(in: VertexOutput) -> @location(0) vec4<f32> {
                    let col_a = textureSample(t_a, s_sampler, in.uv);
                    let col_b = textureSample(t_b, s_sampler, in.uv);
                    let p = clamp(u_trans.progress, 0.0, 1.0);

                    // 0: CrossDissolve
                    if (u_trans.kind == 0u) {
                        return mix(col_a, col_b, p);
                    }
                    // 1: DipToBlack
                    if (u_trans.kind == 1u) {
                        if (p < 0.5) {
                            let t = p * 2.0;
                            return mix(col_a, vec4<f32>(0.0, 0.0, 0.0, 1.0), t);
                        } else {
                            let t = (p - 0.5) * 2.0;
                            return mix(vec4<f32>(0.0, 0.0, 0.0, 1.0), col_b, t);
                        }
                    }
                    // 2: DipToWhite
                    if (u_trans.kind == 2u) {
                        if (p < 0.5) {
                            let t = p * 2.0;
                            return mix(col_a, vec4<f32>(1.0, 1.0, 1.0, 1.0), t);
                        } else {
                            let t = (p - 0.5) * 2.0;
                            return mix(vec4<f32>(1.0, 1.0, 1.0, 1.0), col_b, t);
                        }
                    }
                    // 3: FadeIn / FadeFromBlack (from black into B)
                    if (u_trans.kind == 3u) {
                        return mix(vec4<f32>(0.0, 0.0, 0.0, 1.0), col_b, p);
                    }
                    // 4: FadeOut / FadeToBlack (A into black)
                    if (u_trans.kind == 4u) {
                        return mix(col_a, vec4<f32>(0.0, 0.0, 0.0, 1.0), p);
                    }
                    // 5: FadeFromWhite (from white into B)
                    if (u_trans.kind == 5u) {
                        return mix(vec4<f32>(1.0, 1.0, 1.0, 1.0), col_b, p);
                    }
                    // 6: FadeToWhite (A into white)
                    if (u_trans.kind == 6u) {
                        return mix(col_a, vec4<f32>(1.0, 1.0, 1.0, 1.0), p);
                    }

                    return mix(col_a, col_b, p);
                }
                "#
                .into(),
            ),
        });

        let transition_bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Transition Bind Group Layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 3,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let transition_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Transition Pipeline Layout"),
            bind_group_layouts: &[&transition_bind_group_layout],
            push_constant_ranges: &[],
        });

        let transition_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("Transition Render Pipeline"),
            layout: Some(&transition_pipeline_layout),
            vertex: VertexState {
                module: &transition_shader,
                entry_point: "vs_trans",
                buffers: &[],
                compilation_options: PipelineCompilationOptions::default(),
            },
            fragment: Some(FragmentState {
                module: &transition_shader,
                entry_point: "fs_trans",
                targets: &[Some(ColorTargetState {
                    format: TextureFormat::Rgba8Unorm,
                    blend: Some(BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: PipelineCompilationOptions::default(),
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("Video Texture Sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: FilterMode::Nearest,
            ..Default::default()
        });

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            pipeline,
            bind_group_layout,
            layer_pipeline,
            layer_bind_group_layout,
            transition_pipeline,
            transition_bind_group_layout,
            sampler,
        })
    }

    pub fn render_solid_frame(
        &self,
        width: u32,
        height: u32,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
    ) -> Result<Vec<u8>> {
        let width = width.max(1);
        let height = height.max(1);

        let texture_desc = TextureDescriptor {
            label: Some("Offscreen Target Texture"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let texture = self.device.create_texture(&texture_desc);
        let texture_view = texture.create_view(&TextureViewDescriptor::default());

        let mut encoder = self.device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("Solid Frame Encoder"),
        });

        {
            let _render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("Solid Clear Pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &texture_view,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Color { r, g, b, a }),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }

        self.read_texture_to_rgba(&texture, encoder, width, height)
    }

    pub fn render_texture_view_to_memory_texture(
        &self,
        source_view: &TextureView,
        target_width: u32,
        target_height: u32,
    ) -> Result<gdk4::MemoryTexture> {
        let target_width = target_width.max(1);
        let target_height = target_height.max(1);

        let target_desc = TextureDescriptor {
            label: Some("Composite Target Texture"),
            size: Extent3d {
                width: target_width,
                height: target_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let target_texture = self.device.create_texture(&target_desc);
        let target_view = target_texture.create_view(&TextureViewDescriptor::default());

        let bind_group = self.device.create_bind_group(&BindGroupDescriptor {
            label: Some("Video Quad Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(source_view),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        let mut encoder = self.device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("Composite Quad Encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("Composite Quad Pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Color::BLACK),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.draw(0..3, 0..1);
        }

        let rgba_bytes = self.read_texture_to_rgba(&target_texture, encoder, target_width, target_height)?;
        let glib_bytes = glib::Bytes::from_owned(rgba_bytes);
        let memory_texture = gdk4::MemoryTexture::new(
            target_width as i32,
            target_height as i32,
            gdk4::MemoryFormat::R8g8b8a8,
            &glib_bytes,
            (target_width * 4) as usize,
        );

        Ok(memory_texture)
    }

    pub fn render_video_frame_to_memory_texture(
        &self,
        frame: &VideoFrame,
        target_width: u32,
        target_height: u32,
    ) -> Result<gdk4::MemoryTexture> {
        let frame_texture = self.device.create_texture(&TextureDescriptor {
            label: Some("Source Video Frame Texture"),
            size: Extent3d {
                width: frame.width,
                height: frame.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });

        self.queue.write_texture(
            ImageCopyTexture {
                texture: &frame_texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            &frame.data,
            ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(frame.width * 4),
                rows_per_image: Some(frame.height),
            },
            Extent3d {
                width: frame.width,
                height: frame.height,
                depth_or_array_layers: 1,
            },
        );

        let view = frame_texture.create_view(&TextureViewDescriptor::default());
        self.render_texture_view_to_memory_texture(&view, target_width, target_height)
    }

    pub fn render_to_memory_texture(
        &self,
        width: u32,
        height: u32,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
    ) -> Result<gdk4::MemoryTexture> {
        let rgba_bytes = self.render_solid_frame(width, height, r, g, b, a)?;
        let glib_bytes = glib::Bytes::from_owned(rgba_bytes);

        let texture = gdk4::MemoryTexture::new(
            width as i32,
            height as i32,
            gdk4::MemoryFormat::R8g8b8a8,
            &glib_bytes,
            (width * 4) as usize,
        );

        Ok(texture)
    }

    pub fn create_solid_texture(
        &self,
        width: u32,
        height: u32,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
    ) -> (Texture, TextureView) {
        let width = width.max(1);
        let height = height.max(1);
        let texture = self.device.create_texture(&TextureDescriptor {
            label: Some("Solid Color Texture"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());

        let mut encoder = self.device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("Solid Clear Encoder"),
        });

        {
            let _pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("Solid Clear Pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Color { r, g, b, a }),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        (texture, view)
    }

    pub fn composite_layers(
        &self,
        target_width: u32,
        target_height: u32,
        layers: &[LayerDesc],
    ) -> Result<Vec<u8>> {
        let target_width = target_width.max(1);
        let target_height = target_height.max(1);

        let target_texture = self.device.create_texture(&TextureDescriptor {
            label: Some("MultiTrack Composite Target"),
            size: Extent3d {
                width: target_width,
                height: target_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target_texture.create_view(&TextureViewDescriptor::default());

        let mut encoder = self.device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("MultiTrack Composite Encoder"),
        });

        let mut bind_groups = Vec::with_capacity(layers.len());
        for layer in layers {
            let uniforms = layer.to_uniforms();
            let uniform_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Layer Uniform Buffer"),
                contents: bytemuck::cast_slice(&[uniforms]),
                usage: BufferUsages::UNIFORM,
            });
            let bg = self.device.create_bind_group(&BindGroupDescriptor {
                label: Some("Layer Bind Group"),
                layout: &self.layer_bind_group_layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(layer.texture_view),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::Sampler(&self.sampler),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: uniform_buffer.as_entire_binding(),
                    },
                ],
            });
            bind_groups.push(bg);
        }

        {
            let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("MultiTrack Composite Pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Color::BLACK),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&self.layer_pipeline);
            for bg in &bind_groups {
                render_pass.set_bind_group(0, bg, &[]);
                render_pass.draw(0..6, 0..1);
            }
        }

        self.read_texture_to_rgba(&target_texture, encoder, target_width, target_height)
    }

    pub fn composite_layers_to_memory_texture(
        &self,
        target_width: u32,
        target_height: u32,
        layers: &[LayerDesc],
    ) -> Result<gdk4::MemoryTexture> {
        let rgba_bytes = self.composite_layers(target_width, target_height, layers)?;
        let glib_bytes = glib::Bytes::from_owned(rgba_bytes);
        let memory_texture = gdk4::MemoryTexture::new(
            target_width as i32,
            target_height as i32,
            gdk4::MemoryFormat::R8g8b8a8,
            &glib_bytes,
            (target_width * 4) as usize,
        );

        Ok(memory_texture)
    }

    pub fn render_transition(
        &self,
        view_a: &TextureView,
        view_b: &TextureView,
        target_width: u32,
        target_height: u32,
        kind: &TransitionKind,
        progress: f32,
    ) -> Result<Vec<u8>> {
        let target_width = target_width.max(1);
        let target_height = target_height.max(1);

        let target_texture = self.device.create_texture(&TextureDescriptor {
            label: Some("Transition Target Texture"),
            size: Extent3d {
                width: target_width,
                height: target_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target_texture.create_view(&TextureViewDescriptor::default());

        let kind_u32 = match kind {
            TransitionKind::CrossDissolve | TransitionKind::Crossfade => 0,
            TransitionKind::DipToBlack => 1,
            TransitionKind::DipToWhite => 2,
            TransitionKind::FadeIn | TransitionKind::FadeFromBlack => 3,
            TransitionKind::FadeOut | TransitionKind::FadeToBlack => 4,
            TransitionKind::FadeFromWhite => 5,
            TransitionKind::FadeToWhite => 6,
            _ => 0,
        };

        let uniforms = TransitionUniforms {
            progress,
            kind: kind_u32,
            _pad: [0.0; 2],
        };

        let uniform_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Transition Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniforms]),
            usage: BufferUsages::UNIFORM,
        });

        let bind_group = self.device.create_bind_group(&BindGroupDescriptor {
            label: Some("Transition Bind Group"),
            layout: &self.transition_bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(view_a),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(view_b),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Sampler(&self.sampler),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });

        let mut encoder = self.device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("Transition Encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("Transition Pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Color::BLACK),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&self.transition_pipeline);
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.draw(0..3, 0..1);
        }

        self.read_texture_to_rgba(&target_texture, encoder, target_width, target_height)
    }

    fn read_texture_to_rgba(
        &self,
        texture: &Texture,
        mut encoder: CommandEncoder,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>> {
        let bytes_per_pixel = 4;
        let unpadded_bytes_per_row = width * bytes_per_pixel;
        let align = COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = ((unpadded_bytes_per_row + align - 1) / align) * align;
        let buffer_size = (padded_bytes_per_row * height) as BufferAddress;

        let output_buffer = self.device.create_buffer(&BufferDescriptor {
            label: Some("Readback Staging Buffer"),
            size: buffer_size,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        encoder.copy_texture_to_buffer(
            ImageCopyTexture {
                texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            ImageCopyBuffer {
                buffer: &output_buffer,
                layout: ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(std::iter::once(encoder.finish()));

        let buffer_slice = output_buffer.slice(..);
        let (tx, rx) = futures::channel::oneshot::channel();
        buffer_slice.map_async(MapMode::Read, move |res| {
            let _ = tx.send(res);
        });

        self.device.poll(Maintain::Wait);

        match block_on(rx) {
            Ok(Ok(())) => {
                let mapped_range = buffer_slice.get_mapped_range();
                let mut unpadded_data = Vec::with_capacity((unpadded_bytes_per_row * height) as usize);

                for chunk in mapped_range.chunks(padded_bytes_per_row as usize) {
                    unpadded_data.extend_from_slice(&chunk[..unpadded_bytes_per_row as usize]);
                }

                drop(mapped_range);
                output_buffer.unmap();
                Ok(unpadded_data)
            }
            _ => Err(RenderError::BufferMapFailed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gdk4::prelude::TextureExt;

    #[test]
    fn test_wgpu_render_solid_frame() {
        let renderer = WgpuRenderer::new().expect("Failed to initialize WgpuRenderer");
        let width = 64;
        let height = 64;
        let pixels = renderer
            .render_solid_frame(width, height, 1.0, 0.0, 0.0, 1.0)
            .expect("Failed to render solid frame");

        assert_eq!(pixels.len(), (width * height * 4) as usize);
        assert_eq!(pixels[0], 255);
        assert_eq!(pixels[1], 0);
        assert_eq!(pixels[2], 0);
        assert_eq!(pixels[3], 255);
    }

    #[test]
    fn test_wgpu_render_video_frame() {
        let renderer = WgpuRenderer::new().expect("Failed to initialize WgpuRenderer");
        let frame = VideoFrame {
            width: 32,
            height: 32,
            format: tempo_media::PixelFormat::Rgba,
            data: vec![128u8; 32 * 32 * 4],
            pts_us: 0,
        };

        let mem_tex = renderer
            .render_video_frame_to_memory_texture(&frame, 64, 64)
            .expect("Failed to render video frame to memory texture");

        assert_eq!(mem_tex.width(), 64);
        assert_eq!(mem_tex.height(), 64);
    }

    #[test]
    fn test_multi_track_compositor() {
        let renderer = WgpuRenderer::new().expect("Failed to initialize WgpuRenderer");
        let width = 64;
        let height = 64;

        // V1: Solid Red (1.0, 0.0, 0.0, 1.0)
        let (_tex_v1, view_v1) = renderer.create_solid_texture(width, height, 1.0, 0.0, 0.0, 1.0);
        // V2: Solid Green (0.0, 1.0, 0.0, 1.0)
        let (_tex_v2, view_v2) = renderer.create_solid_texture(width, height, 0.0, 1.0, 0.0, 1.0);

        // V1 full screen, V2 50% scale, 50% opacity, centered offset (0.0, 0.0)
        let layers = vec![
            LayerDesc::new(&view_v1),
            LayerDesc::new(&view_v2).with_scale(0.5, 0.5).with_opacity(0.5),
        ];

        let pixels = renderer.composite_layers(width, height, &layers).expect("Failed to composite layers");
        assert_eq!(pixels.len(), (width * height * 4) as usize);

        // Center pixel (32, 32) is in overlap region:
        // Expected blend: Red 50%, Green 50% -> R ≈ 127-128, G ≈ 127-128, B = 0, A = 255
        let center_idx = ((32 * width + 32) * 4) as usize;
        let r = pixels[center_idx];
        let g = pixels[center_idx + 1];
        let b = pixels[center_idx + 2];
        let a = pixels[center_idx + 3];

        assert!((r as i32 - 128).abs() <= 2, "Expected R around 128, got {}", r);
        assert!((g as i32 - 128).abs() <= 2, "Expected G around 128, got {}", g);
        assert_eq!(b, 0, "Expected B to be 0");
        assert_eq!(a, 255, "Expected A to be 255");

        // Corner pixel (2, 2) is outside V2 scaled quad, so only V1 (pure red):
        let corner_idx = ((2 * width + 2) * 4) as usize;
        assert_eq!(pixels[corner_idx], 255, "Expected pure red outside overlap");
        assert_eq!(pixels[corner_idx + 1], 0, "Expected no green outside overlap");
        assert_eq!(pixels[corner_idx + 2], 0);
        assert_eq!(pixels[corner_idx + 3], 255);
    }

    #[test]
    fn test_transition_shader() {
        let renderer = WgpuRenderer::new().expect("Failed to initialize WgpuRenderer");
        let width = 32;
        let height = 32;

        // Clip A: Solid Red (1.0, 0.0, 0.0, 1.0)
        let (_tex_a, view_a) = renderer.create_solid_texture(width, height, 1.0, 0.0, 0.0, 1.0);
        // Clip B: Solid Blue (0.0, 0.0, 1.0, 1.0)
        let (_tex_b, view_b) = renderer.create_solid_texture(width, height, 0.0, 0.0, 1.0, 1.0);

        let kind = TransitionKind::CrossDissolve;

        // Progress = 0.0 (100% A -> Red)
        let pix_0 = renderer.render_transition(&view_a, &view_b, width, height, &kind, 0.0).unwrap();
        assert_eq!(pix_0[0], 255);
        assert_eq!(pix_0[1], 0);
        assert_eq!(pix_0[2], 0);
        assert_eq!(pix_0[3], 255);

        // Progress = 0.5 (50% A, 50% B -> 50% Red, 50% Blue)
        let pix_half = renderer.render_transition(&view_a, &view_b, width, height, &kind, 0.5).unwrap();
        assert!((pix_half[0] as i32 - 128).abs() <= 2, "Expected R around 128, got {}", pix_half[0]);
        assert_eq!(pix_half[1], 0);
        assert!((pix_half[2] as i32 - 128).abs() <= 2, "Expected B around 128, got {}", pix_half[2]);
        assert_eq!(pix_half[3], 255);

        // Progress = 1.0 (100% B -> Blue)
        let pix_1 = renderer.render_transition(&view_a, &view_b, width, height, &kind, 1.0).unwrap();
        assert_eq!(pix_1[0], 0);
        assert_eq!(pix_1[1], 0);
        assert_eq!(pix_1[2], 255);
        assert_eq!(pix_1[3], 255);

        // Test FadeToBlack: starts at Red (view_a), ends at Black (0, 0, 0)
        let kind_f2b = TransitionKind::FadeToBlack;
        let pix_f2b_0 = renderer.render_transition(&view_a, &view_b, width, height, &kind_f2b, 0.0).unwrap();
        assert_eq!(pix_f2b_0[0], 255);
        assert_eq!(pix_f2b_0[2], 0);
        let pix_f2b_1 = renderer.render_transition(&view_a, &view_b, width, height, &kind_f2b, 1.0).unwrap();
        assert_eq!(pix_f2b_1[0], 0);
        assert_eq!(pix_f2b_1[1], 0);
        assert_eq!(pix_f2b_1[2], 0);

        // Test FadeToWhite: starts at Red (view_a), ends at White (255, 255, 255)
        let kind_f2w = TransitionKind::FadeToWhite;
        let pix_f2w_0 = renderer.render_transition(&view_a, &view_b, width, height, &kind_f2w, 0.0).unwrap();
        assert_eq!(pix_f2w_0[0], 255);
        assert_eq!(pix_f2w_0[2], 0);
        let pix_f2w_1 = renderer.render_transition(&view_a, &view_b, width, height, &kind_f2w, 1.0).unwrap();
        assert_eq!(pix_f2w_1[0], 255);
        assert_eq!(pix_f2w_1[1], 255);
        assert_eq!(pix_f2w_1[2], 255);

        // Test FadeFromBlack: starts at Black (0, 0, 0), ends at Blue (view_b)
        let kind_f_from_b = TransitionKind::FadeFromBlack;
        let pix_ffb_0 = renderer.render_transition(&view_a, &view_b, width, height, &kind_f_from_b, 0.0).unwrap();
        assert_eq!(pix_ffb_0[0], 0);
        assert_eq!(pix_ffb_0[1], 0);
        assert_eq!(pix_ffb_0[2], 0);
        let pix_ffb_1 = renderer.render_transition(&view_a, &view_b, width, height, &kind_f_from_b, 1.0).unwrap();
        assert_eq!(pix_ffb_1[0], 0);
        assert_eq!(pix_ffb_1[1], 0);
        assert_eq!(pix_ffb_1[2], 255);
    }
}
