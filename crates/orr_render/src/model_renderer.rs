//! Optional static imported model renderer. Real indexed POS/NORMAL/UV geometry,
//! opaque sRGB base-color diffuse lighting, depth and mirrored-node winding.
//! Single sample, no shadows/PBR/animation. Existing procedural renderer is unchanged.
use crate::{
    math3::{cross, dot, normalize, sub},
    Camera3D, Lighting, Projection,
};
use bytemuck::{Pod, Zeroable};
use orr_model::{StaticModel, Wrap};
use orr_rhi::{
    Binding, Blend, BufferDesc, BufferUsage, ColorAttachment, Command, Cull, DepthAttachment,
    DepthState, PipelineDesc, Rhi, TextureDesc, TextureFormat, TextureUpload, TextureUploadError,
    TextureUsage, Topology, VertexAttr, VertexFormat, VertexLayout, VertexStep,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelRenderError {
    Upload(TextureUploadError),
    InvalidTarget,
    InvalidCamera,
    InvalidLighting,
}
impl std::fmt::Display for ModelRenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Upload(e) => write!(f, "model texture: {e}"),
            Self::InvalidTarget => {
                f.write_str("model target must be a nonempty color target, at most 8192 per axis")
            }
            Self::InvalidCamera => {
                f.write_str("model camera requires valid finite view and projection")
            }
            Self::InvalidLighting => {
                f.write_str("model lighting must be finite, nonnegative, and shadows disabled")
            }
        }
    }
}
impl std::error::Error for ModelRenderError {}
impl From<TextureUploadError> for ModelRenderError {
    fn from(e: TextureUploadError) -> Self {
        Self::Upload(e)
    }
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct Globals {
    view_proj: [[f32; 4]; 4],
    direction: [f32; 4],
    sun: [f32; 4],
    sky: [f32; 4],
    ground: [f32; 4],
    params: [f32; 4],
    pub(crate) point_position_range: [f32; 4],
    pub(crate) point_color_intensity: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Object {
    world: [[f32; 4]; 4],
    normal: [[f32; 4]; 4],
    color: [f32; 4],
    sampling: [u32; 4],
}
struct Draw<B: Rhi> {
    vertices: B::Buffer,
    indices: B::Buffer,
    bind: B::BindGroup,
    count: u32,
}
struct Depth<B: Rhi> {
    _texture: B::Texture,
    view: B::TextureView,
    size: (u32, u32),
}
/// Immutable GPU model; upload once, then update only camera/light uniforms.
/// Per-primitive stable IDs/material slots remain available in `model()`.
/// `draw` owns a fresh depth pass; it is not an overlay into Renderer3D's depth.
pub struct ModelRenderer<B: Rhi> {
    rhi: B,
    format: TextureFormat,
    model: StaticModel,
    pipeline: B::Pipeline,
    globals: B::Buffer,
    draws: Vec<Draw<B>>,
    depth: Option<Depth<B>>,
    pub clear: [f64; 4],
}
impl<B: Rhi> ModelRenderer<B> {
    pub fn new(
        rhi: B,
        format: TextureFormat,
        model: StaticModel,
    ) -> Result<Self, ModelRenderError> {
        if format.is_depth() {
            return Err(ModelRenderError::InvalidTarget);
        }
        let shader = rhi.create_shader("static textured model", include_str!("shader_model.wgsl"));
        let attrs = [
            VertexAttr {
                location: 0,
                format: VertexFormat::Float32x3,
                offset: 0,
            },
            VertexAttr {
                location: 1,
                format: VertexFormat::Float32x3,
                offset: 12,
            },
            VertexAttr {
                location: 2,
                format: VertexFormat::Float32x2,
                offset: 24,
            },
        ];
        let pipeline = rhi.create_pipeline(&PipelineDesc::<B> {
            label: "static textured model",
            shader: &shader,
            vs_entry: "vs_main",
            fs_entry: "fs_main",
            vertex_buffers: &[VertexLayout {
                stride: 32,
                step: VertexStep::Vertex,
                attrs: &attrs,
            }],
            color_format: Some(format),
            blend: Blend::Opaque,
            topology: Topology::TriangleList,
            cull: Cull::Back,
            depth: Some(DepthState::opaque(TextureFormat::Depth32Float)),
            samples: 1,
        });
        let globals = rhi.create_buffer(&BufferDesc {
            label: "model globals",
            size: std::mem::size_of::<Globals>() as u64,
            usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST,
        });
        let mut views = Vec::new();
        for image in &model.source().images {
            let texture = rhi.create_texture(&TextureDesc {
                label: "model base color sRGB",
                width: image.width,
                height: image.height,
                format: TextureFormat::Rgba8UnormSrgb,
                usage: TextureUsage::TEXTURE_BINDING | TextureUsage::COPY_DST,
                sample_count: 1,
                view_formats: &[],
            });
            rhi.write_texture_rgba8(
                &texture,
                &TextureUpload {
                    origin: [0, 0],
                    width: image.width,
                    height: image.height,
                    bytes_per_row: image.width * 4,
                    data: &image.rgba8,
                },
            )?;
            views.push(rhi.create_texture_view(&texture, None));
        }
        let upload = |label, usage, bytes: &[u8]| {
            let buffer = rhi.create_buffer(&BufferDesc {
                label,
                size: bytes.len() as u64,
                usage: usage | BufferUsage::COPY_DST,
            });
            rhi.write_buffer(&buffer, 0, bytes);
            buffer
        };
        let mut draws = Vec::new();
        for p in &model.source().primitives {
            let vertices: Vec<_> = p
                .vertices
                .iter()
                .map(|v| Vertex {
                    position: v.position,
                    normal: v.normal,
                    uv: v.uv,
                })
                .collect();
            let mut indices = p.indices.clone();
            if orr_model::determinant(p.transform) < 0.0 {
                for triangle in indices.chunks_exact_mut(3) {
                    triangle.swap(1, 2);
                }
            }
            let m = &model.source().materials[p.material as usize];
            let wrap = |v| match v {
                Wrap::Clamp => 0,
                Wrap::Repeat => 1,
                Wrap::Mirror => 2,
            };
            let object = Object {
                world: p.transform,
                // StaticModel has already validated invertibility/finite normal matrix.
                normal: orr_model::normal_matrix(p.transform).expect("validated model transform"),
                color: m.base_color,
                sampling: [
                    wrap(m.wrap_s),
                    wrap(m.wrap_t),
                    u32::from(m.linear_filter),
                    0,
                ],
            };
            let uniform = upload(
                "model object",
                BufferUsage::UNIFORM,
                bytemuck::bytes_of(&object),
            );
            let bind = rhi.create_bind_group(
                &pipeline,
                0,
                &[
                    Binding::Uniform {
                        binding: 0,
                        buffer: &globals,
                    },
                    Binding::Texture {
                        binding: 1,
                        view: &views[m.image as usize],
                    },
                    Binding::Uniform {
                        binding: 2,
                        buffer: &uniform,
                    },
                ],
            );
            draws.push(Draw {
                vertices: upload(
                    "model vertices",
                    BufferUsage::VERTEX,
                    bytemuck::cast_slice(&vertices),
                ),
                indices: upload(
                    "model indices",
                    BufferUsage::INDEX,
                    bytemuck::cast_slice(&indices),
                ),
                bind,
                count: indices.len() as u32,
            });
        }
        Ok(Self {
            rhi,
            format,
            model,
            pipeline,
            globals,
            draws,
            depth: None,
            clear: crate::DEFAULT_CLEAR_3D,
        })
    }
    pub fn model(&self) -> &StaticModel {
        &self.model
    }
    pub fn rhi(&self) -> &B {
        &self.rhi
    }
    pub fn format(&self) -> TextureFormat {
        self.format
    }
    pub fn draw(
        &mut self,
        view: &B::TextureView,
        size: (u32, u32),
        camera: &Camera3D,
        lighting: &Lighting,
    ) -> Result<(), ModelRenderError> {
        // All caller-controlled state is checked before writes/allocation/submission.
        if size.0 == 0
            || size.1 == 0
            || size.0 > 8192
            || size.1 > 8192
            || !self.clear.iter().all(|v| v.is_finite())
        {
            return Err(ModelRenderError::InvalidTarget);
        }
        let globals = prepare_globals(self.format, size, camera, lighting)?;
        if self.depth.as_ref().is_none_or(|d| d.size != size) {
            let texture = self.rhi.create_texture(&TextureDesc {
                label: "model depth",
                width: size.0,
                height: size.1,
                format: TextureFormat::Depth32Float,
                usage: TextureUsage::RENDER_ATTACHMENT,
                sample_count: 1,
                view_formats: &[],
            });
            let depth_view = self.rhi.create_texture_view(&texture, None);
            self.depth = Some(Depth {
                _texture: texture,
                view: depth_view,
                size,
            });
        }
        self.write_frame(&globals);
        let mut encoder = self.rhi.create_encoder("model frame");
        self.encode_frame(
            &mut encoder,
            &ColorAttachment {
                view,
                clear: Some(self.clear),
                resolve: None,
            },
            &DepthAttachment {
                view: &self.depth.as_ref().expect("depth allocated").view,
                clear: Some(1.0),
                store: false,
            },
        );
        self.rhi.submit(encoder);
        Ok(())
    }

    #[cfg(feature = "imported-scene")]
    pub(crate) fn draw_count(&self) -> usize {
        self.draws.len()
    }

    /// GPU writes only; the caller has validated the complete frame.
    pub(crate) fn write_frame(&self, globals: &Globals) {
        self.rhi
            .write_buffer(&self.globals, 0, bytemuck::bytes_of(globals));
    }

    /// Records into caller-owned attachments without allocating private depth.
    pub(crate) fn encode_frame(
        &self,
        encoder: &mut B::Encoder,
        color: &ColorAttachment<'_, B>,
        depth: &DepthAttachment<'_, B>,
    ) {
        let mut commands = vec![Command::SetPipeline(&self.pipeline)];
        for draw in &self.draws {
            commands.extend([
                Command::SetBindGroup(0, &draw.bind),
                Command::SetVertexBuffer(0, &draw.vertices),
                Command::SetIndexBuffer(&draw.indices),
                Command::DrawIndexed {
                    indices: 0..draw.count,
                    base_vertex: 0,
                    instances: 0..1,
                },
            ]);
        }
        self.rhi
            .encode_pass(encoder, "static model", Some(color), Some(depth), &commands);
    }
}
/// Pure validation and uniform preparation, shared by standalone and composed paths.
pub(crate) fn prepare_globals(
    format: TextureFormat,
    size: (u32, u32),
    camera: &Camera3D,
    lighting: &Lighting,
) -> Result<Globals, ModelRenderError> {
    let view_proj = validate_camera(camera, size)?;
    let direction = normalize(lighting.direction);
    if lighting.shadows
        || !lighting.direction.iter().all(|v| v.is_finite())
        || dot(direction, direction) < 0.5
        || !lighting
            .color
            .iter()
            .chain(&lighting.sky)
            .chain(&lighting.ground)
            .chain([&lighting.intensity, &lighting.ambient, &lighting.exposure])
            .all(|v| v.is_finite() && *v >= 0.0 && *v <= 1e4)
    {
        return Err(ModelRenderError::InvalidLighting);
    }
    let vector = |v: [f32; 3], w| [v[0], v[1], v[2], w];
    Ok(Globals {
        view_proj,
        direction: vector(direction, 0.0),
        sun: vector(lighting.color, lighting.intensity),
        sky: vector(lighting.sky, lighting.ambient),
        ground: vector(lighting.ground, 0.0),
        params: [
            lighting.exposure,
            if lighting.tonemap { 1.0 } else { 0.0 },
            if format.is_srgb() { 0.0 } else { 1.0 },
            0.0,
        ],
        point_position_range: [0.0; 4],
        point_color_intensity: [0.0; 4],
    })
}

fn validate_camera(camera: &Camera3D, size: (u32, u32)) -> Result<[[f32; 4]; 4], ModelRenderError> {
    let f = sub(camera.target, camera.eye);
    // Match Camera3D::view / Mat4::look_at, which crosses the normalized forward.
    let right = cross(normalize(f), camera.up);
    if !camera
        .eye
        .iter()
        .chain(&camera.target)
        .chain(&camera.up)
        .all(|v| v.is_finite())
        || !dot(f, f).is_finite()
        || dot(f, f) < 1e-12
        || !dot(right, right).is_finite()
        || dot(right, right) < 1e-12
    {
        return Err(ModelRenderError::InvalidCamera);
    }
    let valid = match camera.projection {
        Projection::Perspective { fov_y, near, far } => {
            fov_y.is_finite()
                && fov_y > 0.0
                && fov_y < std::f32::consts::PI
                && near.is_finite()
                && near > 0.0
                && far.is_finite()
                && far > near
        }
        Projection::Orthographic {
            half_height,
            near,
            far,
        } => {
            half_height.is_finite()
                && half_height >= 1e-6
                && near.is_finite()
                && far.is_finite()
                && far > near
        }
    };
    let matrix = camera.view_proj(size.0 as f32 / size.1 as f32).0;
    if !valid || !matrix.iter().flatten().all(|v| v.is_finite()) {
        return Err(ModelRenderError::InvalidCamera);
    }
    Ok(matrix)
}
