//! The wgpu renderer.
//!
//! Two primitives, one path: every rectangle is a textured quad sampled from a
//! texture, and solid rectangles sample a 1x1 white texture. That avoids a
//! branch in the shader and a second pipeline, at the cost of one bind group per
//! rectangle — acceptable when a frame holds tens of them, not thousands.
//!
//! Text goes through `glyphon`, which shapes with `cosmic-text` and caches glyphs
//! in an atlas. Nothing about text layout is hand-rolled: CJK wrapping and font
//! fallback are exactly the kind of problem that should be solved once, upstream.

use bytemuck::{Pod, Zeroable};
use glyphon::{
    Attrs, Buffer, Cache, Color as GlyphColor, Family, FontSystem, Metrics, PrepareError,
    Resolution, Shaping, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingResource, BindingType, BlendState, BufferBindingType,
    BufferDescriptor, BufferUsages, ColorTargetState, ColorWrites, DeviceDescriptor, FragmentState,
    LoadOp, MultisampleState, Operations, PipelineLayoutDescriptor, PrimitiveState,
    PrimitiveTopology, RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor,
    RequestAdapterOptions, SamplerBindingType, SamplerDescriptor, ShaderModuleDescriptor,
    ShaderSource, ShaderStages, StoreOp, TextureDescriptor, TextureDimension, TextureFormat,
    TextureSampleType, TextureUsages, TextureView, TextureViewDescriptor, TextureViewDimension,
    VertexAttribute, VertexBufferLayout, VertexFormat, VertexState,
};

use crate::frame::{Frame, Rgba, TextureId};
use crate::image::Rgba8Image;

/// Format used for every render target this renderer draws into.
///
/// Windows surfaces are almost always BGRA8 sRGB, and an offscreen target in the
/// same format lets one pipeline serve both. Using a different format for
/// offscreen would need a second pipeline for no benefit.
pub const TARGET_FORMAT: TextureFormat = TextureFormat::Bgra8UnormSrgb;

const SHADER: &str = r#"
struct Globals {
    resolution: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct Quad {
    @location(0) rect: vec4<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv: vec4<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, q: Quad) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let corner = corners[vi];
    let pixel = q.rect.xy + corner * q.rect.zw;
    let ndc = vec2<f32>(
        pixel.x / globals.resolution.x * 2.0 - 1.0,
        1.0 - pixel.y / globals.resolution.y * 2.0,
    );

    var out: VsOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = mix(q.uv.xy, q.uv.zw, corner);
    out.color = q.color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, in.uv) * in.color;
}
"#;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct InstanceRaw {
    rect: [f32; 4],
    color: [f32; 4],
    uv: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Globals {
    resolution: [f32; 2],
    _pad: [f32; 2],
}

/// Why the renderer could not start or draw.
#[derive(Debug)]
pub enum RenderError {
    /// No adapter accepted the request, or the device could not be created.
    NoAdapter(String),
    /// Text shaping or atlas preparation failed.
    Text(String),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoAdapter(message) => write!(f, "no usable GPU: {message}"),
            Self::Text(message) => write!(f, "text preparation failed: {message}"),
        }
    }
}

impl std::error::Error for RenderError {}

impl From<PrepareError> for RenderError {
    fn from(err: PrepareError) -> Self {
        Self::Text(err.to_string())
    }
}

impl From<glyphon::RenderError> for RenderError {
    fn from(err: glyphon::RenderError) -> Self {
        Self::Text(err.to_string())
    }
}

/// Draws [`Frame`]s.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    size: (u32, u32),

    quad_pipeline: wgpu::RenderPipeline,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    globals: wgpu::Buffer,

    instances: wgpu::Buffer,
    instance_capacity: usize,

    /// Index 0 is the 1x1 white texture used by solid quads.
    bind_groups: Vec<BindGroup>,

    font_system: FontSystem,
    swash_cache: SwashCache,
    /// Held for the lifetime of the atlas: glyphon's atlas keeps borrowing its
    /// backing cache, so dropping this would leave the atlas pointing at freed
    /// bookkeeping even though no method here reads the field directly.
    #[allow(dead_code)]
    atlas_cache: Cache,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    viewport: Viewport,
}

impl Renderer {
    /// Create a renderer with an offscreen target of the given size.
    pub fn new_headless(width: u32, height: u32) -> Result<Self, RenderError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .map_err(|err| RenderError::NoAdapter(err.to_string()))?;

        let (device, queue) = pollster::block_on(adapter.request_device(&DeviceDescriptor {
            label: Some("rustory device"),
            ..Default::default()
        }))
        .map_err(|err| RenderError::NoAdapter(err.to_string()))?;

        Self::from_device(device, queue, width, height)
    }

    /// Build the renderer around an existing device.
    ///
    /// The application uses this when it already owns a device for a window
    /// surface, so GPU state is not created twice.
    pub fn from_device(
        device: wgpu::Device,
        queue: wgpu::Queue,
        width: u32,
        height: u32,
    ) -> Result<Self, RenderError> {
        let width = width.max(1);
        let height = height.max(1);

        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("rustory quads"),
            source: ShaderSource::Wgsl(SHADER.into()),
        });

        let texture_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("rustory texture"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::VERTEX,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
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
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("rustory quad layout"),
            bind_group_layouts: &[Some(&texture_layout)],
            immediate_size: 0,
        });

        let attributes = [
            VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: VertexFormat::Float32x4,
            },
            VertexAttribute {
                offset: 16,
                shader_location: 1,
                format: VertexFormat::Float32x4,
            },
            VertexAttribute {
                offset: 32,
                shader_location: 2,
                format: VertexFormat::Float32x4,
            },
        ];

        let quad_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("rustory quad pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(VertexBufferLayout {
                    array_stride: std::mem::size_of::<InstanceRaw>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attributes,
                })],
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(ColorTargetState {
                    format: TARGET_FORMAT,
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("rustory sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let globals = device.create_buffer(&BufferDescriptor {
            label: Some("rustory globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let instance_capacity = 256;
        let instances = device.create_buffer(&BufferDescriptor {
            label: Some("rustory quad instances"),
            size: (instance_capacity * std::mem::size_of::<InstanceRaw>()) as u64,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Solid quads sample this. One texel is enough; the sampler clamps.
        let white = Rgba8Image::solid(1, 1, Rgba::rgb(255, 255, 255));
        let white_view = upload_rgba(&device, &queue, &white, "rustory white");
        let bind_groups = vec![make_bind_group(
            &device,
            &texture_layout,
            &globals,
            &white_view,
            &sampler,
        )];

        let font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let atlas_cache = Cache::new(&device);
        let mut atlas = TextAtlas::new(&device, &queue, &atlas_cache, TARGET_FORMAT);
        let text_renderer =
            TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
        let mut viewport = Viewport::new(&device, &atlas_cache);
        viewport.update(&queue, Resolution { width, height });
        // Touching the font system here surfaces a broken font database at
        // startup rather than inside the first draw call.
        let _ = font_system.db().len();

        Ok(Self {
            device,
            queue,
            size: (width, height),
            quad_pipeline,
            texture_layout,
            sampler,
            globals,
            instances,
            instance_capacity,
            bind_groups,
            font_system,
            swash_cache,
            atlas_cache,
            atlas,
            text_renderer,
            viewport,
        })
    }

    #[must_use]
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    #[must_use]
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Upload a texture and get a handle to draw it with.
    pub fn upload_texture(&mut self, image: &Rgba8Image, label: &str) -> TextureId {
        let view = upload_rgba(&self.device, &self.queue, image, label);
        let bind_group = make_bind_group(
            &self.device,
            &self.texture_layout,
            &self.globals,
            &view,
            &self.sampler,
        );
        self.bind_groups.push(bind_group);
        // Index 0 is the white texture, so texture ids start at 1.
        TextureId((self.bind_groups.len() - 1) as u32)
    }

    /// Resize the coordinate space. Text wrapping and the projection both use it.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        self.viewport.update(
            &self.queue,
            Resolution {
                width: self.size.0,
                height: self.size.1,
            },
        );
    }

    /// Draw a frame into `target`.
    ///
    /// `target` must have [`TARGET_FORMAT`] and match [`Renderer::size`].
    pub fn render_to(&mut self, frame: &Frame, target: &TextureView) -> Result<(), RenderError> {
        self.queue.write_buffer(
            &self.globals,
            0,
            bytemuck::bytes_of(&Globals {
                resolution: [frame.width as f32, frame.height as f32],
                _pad: [0.0, 0.0],
            }),
        );

        self.upload_instances(frame);

        // Glyphon needs owned buffers that outlive the text areas, so they are
        // built here and dropped after the pass.
        let mut buffers: Vec<Buffer> = Vec::with_capacity(frame.texts.len());
        for item in &frame.texts {
            let mut buffer = Buffer::new(
                &mut self.font_system,
                Metrics::new(item.font_size, item.line_height),
            );
            buffer.set_size(Some(item.max_width), Some(frame.height as f32));
            buffer.set_text(
                &item.text,
                &Attrs::new().family(Family::SansSerif),
                Shaping::Advanced,
                None,
            );
            buffer.shape_until_scroll(&mut self.font_system, false);
            buffers.push(buffer);
        }

        let areas: Vec<TextArea<'_>> = frame
            .texts
            .iter()
            .zip(buffers.iter())
            .map(|(item, buffer)| TextArea {
                buffer,
                left: item.x,
                top: item.y,
                scale: 1.0,
                bounds: TextBounds {
                    left: 0,
                    top: 0,
                    right: frame.width as i32,
                    bottom: frame.height as i32,
                },
                default_color: GlyphColor::rgba(
                    item.color.r,
                    item.color.g,
                    item.color.b,
                    item.color.a,
                ),
                custom_glyphs: &[],
            })
            .collect();

        self.text_renderer.prepare(
            &self.device,
            &self.queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            areas,
            &mut self.swash_cache,
        )?;

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("rustory frame"),
            });

        {
            let clear = frame.clear.to_array();
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("rustory pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(wgpu::Color {
                            r: clear[0] as f64,
                            g: clear[1] as f64,
                            b: clear[2] as f64,
                            a: clear[3] as f64,
                        }),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            if !frame.quads.is_empty() {
                pass.set_pipeline(&self.quad_pipeline);
                pass.set_vertex_buffer(0, self.instances.slice(..));
                for (index, quad) in frame.quads.iter().enumerate() {
                    let bind_index = quad.texture.map_or(0, |TextureId(id)| id as usize);
                    let Some(bind_group) = self.bind_groups.get(bind_index) else {
                        // A stale texture id would otherwise draw the wrong image
                        // silently. Skipping keeps the frame honest.
                        continue;
                    };
                    pass.set_bind_group(0, bind_group, &[]);
                    pass.draw(0..6, index as u32..index as u32 + 1);
                }
            }

            self.text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)?;
        }

        self.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    /// Render a frame and read the pixels back.
    ///
    /// This is how rendering gets verified without a window: the caller gets an
    /// image it can inspect, compare or write to disk.
    pub fn render_offscreen(&mut self, frame: &Frame) -> Result<Rgba8Image, RenderError> {
        if (frame.width, frame.height) != self.size {
            self.resize(frame.width, frame.height);
        }

        let texture = self.device.create_texture(&TextureDescriptor {
            label: Some("rustory offscreen"),
            size: wgpu::Extent3d {
                width: self.size.0,
                height: self.size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());
        self.render_to(frame, &view)?;

        read_back(&self.device, &self.queue, &texture, self.size)
    }
}

fn make_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    globals: &wgpu::Buffer,
    texture: &TextureView,
    sampler: &wgpu::Sampler,
) -> BindGroup {
    device.create_bind_group(&BindGroupDescriptor {
        label: Some("rustory texture bind group"),
        layout,
        entries: &[
            BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::TextureView(texture),
            },
            BindGroupEntry {
                binding: 2,
                resource: BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn upload_rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &Rgba8Image,
    label: &str,
) -> TextureView {
    let size = wgpu::Extent3d {
        width: image.width(),
        height: image.height(),
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&TextureDescriptor {
        label: Some(label),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8UnormSrgb,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        image.data(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * image.width()),
            rows_per_image: Some(image.height()),
        },
        size,
    );

    texture.create_view(&TextureViewDescriptor::default())
}

impl Renderer {
    fn upload_instances(&mut self, frame: &Frame) {
        if frame.quads.is_empty() {
            return;
        }

        let needed = frame.quads.len();
        if needed > self.instance_capacity {
            self.instance_capacity = needed.next_power_of_two();
            self.instances = self.device.create_buffer(&BufferDescriptor {
                label: Some("rustory quad instances"),
                size: (self.instance_capacity * std::mem::size_of::<InstanceRaw>()) as u64,
                usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }

        let raw: Vec<InstanceRaw> = frame
            .quads
            .iter()
            .map(|quad| InstanceRaw {
                rect: [quad.rect.x, quad.rect.y, quad.rect.width, quad.rect.height],
                color: quad.color.to_array(),
                uv: quad.uv.bounds,
            })
            .collect();

        self.queue
            .write_buffer(&self.instances, 0, bytemuck::cast_slice(&raw));
    }
}

/// Copy a rendered texture back to the CPU, undoing the row padding wgpu requires.
fn read_back(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    size: (u32, u32),
) -> Result<Rgba8Image, RenderError> {
    let unpadded_bytes_per_row = size.0 * 4;
    // copy_texture_to_buffer requires each row to start on a 256-byte boundary.
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;

    let buffer = device.create_buffer(&BufferDescriptor {
        label: Some("rustory readback"),
        size: (padded_bytes_per_row as u64) * (size.1 as u64),
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("rustory readback encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(size.1),
            },
        },
        wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map_err(|err| RenderError::NoAdapter(format!("failed to wait for readback: {err}")))?;

    let mapped = slice
        .get_mapped_range()
        .map_err(|err| RenderError::NoAdapter(format!("readback buffer unavailable: {err}")))?;
    let mut data = Vec::with_capacity((size.0 * size.1 * 4) as usize);
    for row in 0..size.1 {
        let start = (row * padded_bytes_per_row) as usize;
        let end = start + unpadded_bytes_per_row as usize;
        data.extend_from_slice(&mapped[start..end]);
    }
    drop(mapped);
    buffer.unmap();

    // The target is BGRA; the CPU image is RGBA.
    for pixel in data.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }

    Rgba8Image::from_rgba(size.0, size.1, data)
        .ok_or_else(|| RenderError::NoAdapter("readback produced a malformed buffer".into()))
}
