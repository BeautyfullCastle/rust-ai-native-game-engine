//! Optional bounded GPU skinning for imported animated models.
//!
//! Immutable original vertices are uploaded once. The GPU blends four joint
//! matrices; CPU deformation is used only to validate normals and compute exact
//! current bounds. Each instance/primitive owns a separate 64-matrix uniform.
//! All instances share one fresh depth/color pass. This is not an overlay into
//! the procedural renderer's depth buffer; shadows, PBR, and HDR are not supported.
use crate::{
    math3::Mat4,
    model_renderer::{prepare_globals, Globals, ModelRenderError},
    Camera3D, Lighting,
};
use bytemuck::{Pod, Zeroable};
use orr_model::{
    animation::{AnimatedModel, Matrix4, Pose},
    Wrap, IDENTITY,
};
use orr_rhi::{
    Binding, Blend, BufferDesc, BufferUsage, ColorAttachment, Command, Cull, DepthAttachment,
    DepthState, PipelineDesc, Rhi, TextureDesc, TextureFormat, TextureUpload, TextureUploadError,
    TextureUsage, Topology, VertexAttr, VertexFormat, VertexLayout, VertexStep,
};

/// Submission bounds keep per-frame palette storage and CPU validation bounded.
pub const MAX_SKINNED_INSTANCES: usize = 256;
pub const MAX_SKINNED_DRAWS: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkinnedRenderError {
    Upload(TextureUploadError),
    InvalidTarget,
    InvalidCamera,
    InvalidLighting,
    InvalidPlacement,
    InvalidPose(orr_model::Error),
    InstanceLimit,
}
impl std::fmt::Display for SkinnedRenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Upload(e) => write!(f, "skinned texture: {e}"),
            Self::InvalidTarget => f.write_str("skinned target must be a nonempty color target, at most 8192 per axis"),
            Self::InvalidCamera => f.write_str("skinned camera requires valid finite view and projection"),
            Self::InvalidLighting => f.write_str("skinned lighting must be finite, nonnegative, and shadows disabled"),
            Self::InvalidPlacement => f.write_str("instance placement must be finite, affine, nonmirrored, invertible, and within range"),
            Self::InvalidPose(e) => write!(f, "skinned pose: {e}"),
            Self::InstanceLimit => f.write_str("skinned instance/draw limit exceeded"),
        }
    }
}
impl std::error::Error for SkinnedRenderError {}
impl From<TextureUploadError> for SkinnedRenderError {
    fn from(e: TextureUploadError) -> Self {
        Self::Upload(e)
    }
}

/// An independently sampled pose and optional external model-to-world placement.
/// A skinned mesh node's transform is deliberately not used as placement.
#[derive(Clone, Copy, Debug)]
pub struct SkinnedInstance<'a> {
    pub pose: &'a Pose,
    pub transform: Matrix4,
}
impl<'a> SkinnedInstance<'a> {
    pub fn new(pose: &'a Pose) -> Self {
        Self {
            pose,
            transform: IDENTITY,
        }
    }
}

/// Exact world-space bounds of the currently deformed vertices, including the
/// external placement. Bind-pose bounds are never used for animation culling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkinnedBounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    joints: [u32; 4],
    weights: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Object {
    world: Matrix4,
    normal: Matrix4,
    color: [f32; 4],
    sampling: [u32; 4],
    joints: [Matrix4; 64],
}
struct Geometry<B: Rhi> {
    vertices: B::Buffer,
    indices: B::Buffer,
    count: u32,
}
struct InstanceDraw<B: Rhi> {
    uniform: B::Buffer,
    bind: B::BindGroup,
}
/// Fully validated CPU state. Kept separate until the entire scene is admitted.
pub(crate) struct PreparedSkinned {
    objects: Vec<Object>,
    bounds: Vec<SkinnedBounds>,
}

struct Depth<B: Rhi> {
    _texture: B::Texture,
    view: B::TextureView,
    size: (u32, u32),
}

/// One immutable asset with independent per-submission instance poses.
/// `draw` validates every instance before any GPU write or allocation, then
/// updates only uniforms. Its CPU oracle is O(instances × vertices), a deliberate
/// correctness-first bound for this initial slice. No CPU-deformed vertex upload.
pub struct SkinnedModelRenderer<B: Rhi> {
    rhi: B,
    format: TextureFormat,
    model: AnimatedModel,
    pipeline: B::Pipeline,
    globals: B::Buffer,
    views: Vec<B::TextureView>,
    geometry: Vec<Geometry<B>>,
    draws: Vec<InstanceDraw<B>>,
    depth: Option<Depth<B>>,
    bounds: Vec<SkinnedBounds>,
    pub clear: [f64; 4],
}
impl<B: Rhi> SkinnedModelRenderer<B> {
    pub fn new(
        rhi: B,
        format: TextureFormat,
        model: AnimatedModel,
    ) -> Result<Self, SkinnedRenderError> {
        if format.is_depth() {
            return Err(SkinnedRenderError::InvalidTarget);
        }
        let shader = rhi.create_shader(
            "skinned textured model",
            include_str!("shader_skinned.wgsl"),
        );
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
            // Existing RHI scalar integer formats suffice, including WebGPU.
            VertexAttr {
                location: 3,
                format: VertexFormat::Uint32,
                offset: 32,
            },
            VertexAttr {
                location: 4,
                format: VertexFormat::Uint32,
                offset: 36,
            },
            VertexAttr {
                location: 5,
                format: VertexFormat::Uint32,
                offset: 40,
            },
            VertexAttr {
                location: 6,
                format: VertexFormat::Uint32,
                offset: 44,
            },
            VertexAttr {
                location: 7,
                format: VertexFormat::Float32x4,
                offset: 48,
            },
        ];
        let pipeline = rhi.create_pipeline(&PipelineDesc::<B> {
            label: "skinned textured model",
            shader: &shader,
            vs_entry: "vs_main",
            fs_entry: "fs_main",
            vertex_buffers: &[VertexLayout {
                stride: 64,
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
            label: "skinned globals",
            size: std::mem::size_of::<Globals>() as u64,
            usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST,
        });
        let mut views = Vec::new();
        for image in &model.source().images {
            let texture = rhi.create_texture(&TextureDesc {
                label: "skinned base color sRGB",
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
        let geometry = model
            .source()
            .primitives
            .iter()
            .map(|p| {
                let vertices: Vec<_> = p
                    .vertices
                    .iter()
                    .map(|v| Vertex {
                        position: v.vertex.position,
                        normal: v.vertex.normal,
                        uv: v.vertex.uv,
                        joints: if p.skin.is_some() { v.joints } else { [0; 4] },
                        weights: if p.skin.is_some() {
                            v.weights
                        } else {
                            [1.0, 0.0, 0.0, 0.0]
                        },
                    })
                    .collect();
                Geometry {
                    vertices: upload(
                        "original skinned vertices",
                        BufferUsage::VERTEX,
                        bytemuck::cast_slice(&vertices),
                    ),
                    indices: upload(
                        "skinned indices",
                        BufferUsage::INDEX,
                        bytemuck::cast_slice(&p.indices),
                    ),
                    count: p.indices.len() as u32,
                }
            })
            .collect();
        Ok(Self {
            rhi,
            format,
            model,
            pipeline,
            globals,
            views,
            geometry,
            draws: Vec::new(),
            depth: None,
            bounds: Vec::new(),
            clear: crate::DEFAULT_CLEAR_3D,
        })
    }
    pub fn model(&self) -> &AnimatedModel {
        &self.model
    }
    pub fn rhi(&self) -> &B {
        &self.rhi
    }
    pub fn format(&self) -> TextureFormat {
        self.format
    }
    /// World-space bounds of the last successfully submitted instances, in order.
    pub fn bounds(&self) -> &[SkinnedBounds] {
        &self.bounds
    }

    /// Validate a pose and calculate exact current bounds without touching GPU state.
    pub fn instance_bounds(
        &self,
        instance: &SkinnedInstance<'_>,
    ) -> Result<SkinnedBounds, SkinnedRenderError> {
        validate_placement(instance.transform)?;
        let deformed = self
            .model
            .deform(instance.pose)
            .map_err(SkinnedRenderError::InvalidPose)?;
        let mut bounds = SkinnedBounds {
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
        };
        for primitive in deformed {
            for vertex in primitive.vertices {
                let world = Mat4(instance.transform).transform_point4(vertex.position);
                for (r, value) in world[..3].iter().copied().enumerate() {
                    if !value.is_finite() || value.abs() > 1.0e9 {
                        return Err(SkinnedRenderError::InvalidPlacement);
                    }
                    bounds.min[r] = bounds.min[r].min(value);
                    bounds.max[r] = bounds.max[r].max(value);
                }
            }
        }
        Ok(bounds)
    }
    pub fn draw(
        &mut self,
        view: &B::TextureView,
        size: (u32, u32),
        camera: &Camera3D,
        lighting: &Lighting,
        instances: &[SkinnedInstance<'_>],
    ) -> Result<(), SkinnedRenderError> {
        if size.0 == 0
            || size.1 == 0
            || size.0 > 8192
            || size.1 > 8192
            || !self.clear.iter().all(|v| v.is_finite())
        {
            return Err(SkinnedRenderError::InvalidTarget);
        }
        self.draw_count(instances.len())?;
        let globals =
            prepare_globals(self.format, size, camera, lighting).map_err(|error| match error {
                ModelRenderError::InvalidCamera => SkinnedRenderError::InvalidCamera,
                ModelRenderError::InvalidLighting => SkinnedRenderError::InvalidLighting,
                ModelRenderError::InvalidTarget => SkinnedRenderError::InvalidTarget,
                ModelRenderError::Upload(error) => SkinnedRenderError::Upload(error),
            })?;
        let prepared = self.prepare_instances(instances)?;
        if self.depth.as_ref().is_none_or(|d| d.size != size) {
            let texture = self.rhi.create_texture(&TextureDesc {
                label: "skinned depth",
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
        self.write_frame(&globals, &prepared);
        let mut encoder = self.rhi.create_encoder("skinned frame");
        // Exactly one pass/clear for all instances, including an empty scene.
        self.encode_frame(
            &prepared,
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
        self.commit_frame(prepared);
        Ok(())
    }

    pub(crate) fn draw_count(&self, instance_count: usize) -> Result<usize, SkinnedRenderError> {
        let count = instance_count
            .checked_mul(self.geometry.len())
            .ok_or(SkinnedRenderError::InstanceLimit)?;
        if instance_count > MAX_SKINNED_INSTANCES || count > MAX_SKINNED_DRAWS {
            return Err(SkinnedRenderError::InstanceLimit);
        }
        Ok(count)
    }

    /// Pure CPU validation; no depth/cache allocation, writes, or bound mutation.
    pub(crate) fn prepare_instances(
        &self,
        instances: &[SkinnedInstance<'_>],
    ) -> Result<PreparedSkinned, SkinnedRenderError> {
        let count = self.draw_count(instances.len())?;
        // Every pose (including singular blended normals) and placement is checked
        // before the first GPU write. Failed submission keeps old pixels/bounds.
        let bounds: Vec<_> = instances
            .iter()
            .map(|i| self.instance_bounds(i))
            .collect::<Result<_, _>>()?;
        let mut objects = Vec::with_capacity(count);
        for instance in instances {
            let normal = validate_placement(instance.transform)?;
            for p in &self.model.source().primitives {
                let m = &self.model.source().materials[p.material as usize];
                let wrap = |v| match v {
                    Wrap::Clamp => 0,
                    Wrap::Repeat => 1,
                    Wrap::Mirror => 2,
                };
                let mut joints = [IDENTITY; 64];
                if let Some(skin) = p.skin {
                    let palette = &instance.pose.skin_matrices()[skin as usize];
                    joints[..palette.len()].copy_from_slice(palette);
                } else {
                    joints[0] = instance.pose.global()[p.node as usize];
                }
                objects.push(Object {
                    world: instance.transform,
                    normal,
                    color: m.base_color,
                    sampling: [
                        wrap(m.wrap_s),
                        wrap(m.wrap_t),
                        u32::from(m.linear_filter),
                        0,
                    ],
                    joints,
                });
            }
        }
        Ok(PreparedSkinned { objects, bounds })
    }

    /// Allocate only uniform slots, then upload the already validated frame.
    pub(crate) fn write_frame(&mut self, globals: &Globals, prepared: &PreparedSkinned) {
        let count = prepared.objects.len();
        // Cache one slot for every instance/primitive pair. Rewriting one shared
        // buffer between recorded draws would make all instances see the last pose.
        while self.draws.len() < count {
            let p = &self.model.source().primitives[self.draws.len() % self.geometry.len()];
            let m = &self.model.source().materials[p.material as usize];
            let uniform = self.rhi.create_buffer(&BufferDesc {
                label: "independent skinned instance palette",
                size: std::mem::size_of::<Object>() as u64,
                usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST,
            });
            let bind = self.rhi.create_bind_group(
                &self.pipeline,
                0,
                &[
                    Binding::Uniform {
                        binding: 0,
                        buffer: &self.globals,
                    },
                    Binding::Texture {
                        binding: 1,
                        view: &self.views[m.image as usize],
                    },
                    Binding::Uniform {
                        binding: 2,
                        buffer: &uniform,
                    },
                ],
            );
            self.draws.push(InstanceDraw { uniform, bind });
        }
        self.rhi
            .write_buffer(&self.globals, 0, bytemuck::bytes_of(globals));
        for (draw, object) in self.draws.iter().zip(&prepared.objects) {
            self.rhi
                .write_buffer(&draw.uniform, 0, bytemuck::bytes_of(object));
        }
    }

    /// Records into caller-owned attachments without allocating private depth.
    pub(crate) fn encode_frame(
        &self,
        prepared: &PreparedSkinned,
        encoder: &mut B::Encoder,
        color: &ColorAttachment<'_, B>,
        depth: &DepthAttachment<'_, B>,
    ) {
        let mut commands = vec![Command::SetPipeline(&self.pipeline)];
        for i in 0..prepared.objects.len() {
            let draw = &self.draws[i];
            let geometry = &self.geometry[i % self.geometry.len()];
            commands.extend([
                Command::SetBindGroup(0, &draw.bind),
                Command::SetVertexBuffer(0, &geometry.vertices),
                Command::SetIndexBuffer(&geometry.indices),
                Command::DrawIndexed {
                    indices: 0..geometry.count,
                    base_vertex: 0,
                    instances: 0..1,
                },
            ]);
        }
        self.rhi.encode_pass(
            encoder,
            "skinned models",
            Some(color),
            Some(depth),
            &commands,
        );
    }

    /// Publish bounds only after the complete encoder has been submitted.
    pub(crate) fn commit_frame(&mut self, prepared: PreparedSkinned) {
        self.bounds = prepared.bounds;
    }
}

fn validate_placement(transform: Matrix4) -> Result<Matrix4, SkinnedRenderError> {
    if orr_model::determinant(transform) <= 0.0 {
        return Err(SkinnedRenderError::InvalidPlacement);
    }
    orr_model::normal_matrix(transform).map_err(|_| SkinnedRenderError::InvalidPlacement)
}
