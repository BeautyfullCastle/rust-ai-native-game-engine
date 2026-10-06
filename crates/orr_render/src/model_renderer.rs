//! Optional static imported model renderer. Real indexed POS/NORMAL/UV geometry,
//! opaque sRGB base-color diffuse lighting, depth and mirrored-node winding.
//! Standalone: single sample, no shadows/PBR/animation. Shared directional shadows
//! are admitted only by ImportedSceneRenderer. External TRS instances can be composed
//! with procedural and animated geometry by the imported-scene coordinator.
use crate::{
    Camera3D, Lighting, Projection,
    math3::{cross, dot, normalize, sub},
};
use bytemuck::{Pod, Zeroable};
use orr_model::{StaticModel, Wrap};
use orr_rhi::{
    Binding, Blend, BufferDesc, BufferUsage, ColorAttachment, Command, Cull, DepthAttachment,
    DepthState, PipelineDesc, Rhi, TextureDesc, TextureFormat, TextureUpload, TextureUploadError,
    TextureUsage, Topology, VertexAttr, VertexFormat, VertexLayout, VertexStep,
};

/// External entity placement: world = external TRS × imported node transform.
/// Rotation is a unit XYZW quaternion; scale must be positive and nonzero.
/// Accepted placements and composed normal matrices are finite and bounded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StaticInstance {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}
impl Default for StaticInstance {
    fn default() -> Self {
        Self {
            translation: [0.0; 3],
            rotation: crate::IDENTITY_ROT,
            scale: [1.0; 3],
        }
    }
}
/// Bounds per asset per frame, in addition to the coordinator's aggregate limit.
pub const MAX_STATIC_INSTANCES: usize = 256;
pub const MAX_STATIC_INSTANCE_DRAWS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticInstanceError {
    InvalidPlacement,
    InstanceLimit,
}
impl std::fmt::Display for StaticInstanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidPlacement => "static placement requires bounded finite TRS, unit rotation, positive scale, and invertible composed node transforms",
            Self::InstanceLimit => "static instance/draw limit exceeded",
        })
    }
}
impl std::error::Error for StaticInstanceError {}

#[cfg(feature = "imported-scene")]
impl StaticInstance {
    /// Validate this placement against every primitive before constructing GPU caches.
    /// This uses exactly the same composed matrices and conservative world bounds as draw.
    pub fn validate_for(&self, model: &StaticModel) -> Result<(), StaticInstanceError> {
        validate_static_instances(model, std::slice::from_ref(self))
    }

    fn matrix(&self) -> Result<crate::math3::Mat4, StaticInstanceError> {
        let norm = self.rotation.iter().map(|v| v * v).sum::<f32>();
        if !self
            .translation
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 1e6)
            || !self
                .scale
                .iter()
                .all(|v| v.is_finite() && *v > 0.0 && *v <= 1e6)
            || !norm.is_finite()
            || (norm - 1.0).abs() > 1e-3
        {
            return Err(StaticInstanceError::InvalidPlacement);
        }
        // Normalize tolerated authoring roundoff so positive scale cannot change winding.
        let q = self.rotation.map(|v| v / norm.sqrt());
        let mut matrix = crate::math3::Mat4::IDENTITY;
        for axis in 0..3 {
            let mut basis = [0.0; 3];
            basis[axis] = self.scale[axis];
            let rotated = crate::math3::quat_rotate(q, basis);
            matrix.0[axis][..3].copy_from_slice(&rotated);
        }
        matrix.0[3][..3].copy_from_slice(&self.translation);
        orr_model::normal_matrix(matrix.0).map_err(|_| StaticInstanceError::InvalidPlacement)?;
        Ok(matrix)
    }
}

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
    pub(crate) light_vp: [[f32; 4]; 4],
    pub(crate) shadow: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Object {
    world: [[f32; 4]; 4],
    normal: [[f32; 4]; 4],
    color: [f32; 4],
    sampling: [u32; 4],
}
#[cfg(feature = "imported-scene")]
struct InstanceDraw<B: Rhi> {
    uniform: B::Buffer,
    bind: B::BindGroup,
    #[cfg(feature = "imported-scene")]
    shadow: crate::shared_shadow::ObjectBindings<B>,
}
#[cfg(feature = "imported-scene")]
pub(crate) struct PreparedStatic {
    objects: Vec<Object>,
}
struct Draw<B: Rhi> {
    #[cfg(feature = "imported-scene")]
    _uniform: B::Buffer,
    vertices: B::Buffer,
    indices: B::Buffer,
    bind: B::BindGroup,
    #[cfg(feature = "imported-scene")]
    shadow: crate::shared_shadow::ObjectBindings<B>,
    count: u32,
}
struct Depth<B: Rhi> {
    _texture: B::Texture,
    view: B::TextureView,
    size: (u32, u32),
}
/// Immutable GPU geometry/textures; upload once, then update frame uniforms.
/// Per-primitive stable IDs/material slots remain available in `model()`.
/// `draw` owns a fresh depth pass; it is not an overlay into Renderer3D's depth.
pub struct ModelRenderer<B: Rhi> {
    rhi: B,
    format: TextureFormat,
    model: StaticModel,
    pipeline: B::Pipeline,
    #[cfg(feature = "imported-scene")]
    shadow: crate::shared_shadow::ShadowPipelines<B>,
    globals: B::Buffer,
    draws: Vec<Draw<B>>,
    #[cfg(feature = "imported-scene")]
    views: Vec<B::TextureView>,
    #[cfg(feature = "imported-scene")]
    instance_draws: Vec<InstanceDraw<B>>,
    #[cfg(feature = "imported-scene")]
    local_bounds: Vec<([f32; 3], [f32; 3])>,
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
        #[cfg(feature = "imported-scene")]
        let shadow = crate::shared_shadow::ShadowPipelines::new(
            &rhi,
            &shader,
            format,
            &[VertexLayout {
                stride: 32,
                step: VertexStep::Vertex,
                attrs: &attrs,
            }],
        );
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
                #[cfg(feature = "imported-scene")]
                shadow: shadow.object(&rhi, &globals, &uniform, &views[m.image as usize]),
                count: indices.len() as u32,
                #[cfg(feature = "imported-scene")]
                _uniform: uniform,
            });
        }
        #[cfg(feature = "imported-scene")]
        let local_bounds = model_local_bounds(&model);
        Ok(Self {
            rhi,
            format,
            model,
            pipeline,
            #[cfg(feature = "imported-scene")]
            shadow,
            globals,
            draws,
            #[cfg(feature = "imported-scene")]
            views,
            #[cfg(feature = "imported-scene")]
            instance_draws: Vec::new(),
            #[cfg(feature = "imported-scene")]
            local_bounds,
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

    #[cfg(feature = "imported-scene")]
    pub(crate) fn instance_draw_count(&self, count: usize) -> Result<usize, StaticInstanceError> {
        static_instance_draw_count(&self.model, count)
    }

    /// CPU-only preparation, shared with public preflight validation.
    #[cfg(feature = "imported-scene")]
    pub(crate) fn prepare_instances(
        &self,
        instances: &[StaticInstance],
    ) -> Result<PreparedStatic, StaticInstanceError> {
        prepare_static_instances(&self.model, &self.local_bounds, instances)
    }

    #[cfg(feature = "imported-scene")]
    pub(crate) fn write_instances(&mut self, globals: &Globals, prepared: &PreparedStatic) {
        while self.instance_draws.len() < prepared.objects.len() {
            let p = &self.model.source().primitives[self.instance_draws.len() % self.draws.len()];
            let material = &self.model.source().materials[p.material as usize];
            let uniform = self.rhi.create_buffer(&BufferDesc {
                label: "static instance object",
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
                        view: &self.views[material.image as usize],
                    },
                    Binding::Uniform {
                        binding: 2,
                        buffer: &uniform,
                    },
                ],
            );
            let shadow = self.shadow.object(
                &self.rhi,
                &self.globals,
                &uniform,
                &self.views[material.image as usize],
            );
            self.instance_draws.push(InstanceDraw {
                uniform,
                bind,
                shadow,
            });
        }
        self.write_frame(globals);
        for (draw, object) in self.instance_draws.iter().zip(&prepared.objects) {
            self.rhi
                .write_buffer(&draw.uniform, 0, bytemuck::bytes_of(object));
        }
    }

    #[cfg(feature = "imported-scene")]
    pub(crate) fn encode_instances(
        &self,
        prepared: &PreparedStatic,
        encoder: &mut B::Encoder,
        color: &ColorAttachment<'_, B>,
        depth: &DepthAttachment<'_, B>,
    ) {
        let mut commands = vec![Command::SetPipeline(&self.pipeline)];
        for (index, draw) in self
            .instance_draws
            .iter()
            .take(prepared.objects.len())
            .enumerate()
        {
            let geometry = &self.draws[index % self.draws.len()];
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
            "static model instances",
            Some(color),
            Some(depth),
            &commands,
        );
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
    #[cfg(feature = "imported-scene")]
    pub(crate) fn bind_shared_shadow(&mut self, map: &crate::shared_shadow::SharedShadow<B>) {
        self.shadow.bind_map(&self.rhi, map);
    }
    #[cfg(feature = "imported-scene")]
    pub(crate) fn shared_commands<'a>(
        &'a self,
        ready: Option<&PreparedStatic>,
        depth: bool,
        commands: &mut Vec<Command<'a, B>>,
    ) {
        commands.push(Command::SetPipeline(if depth {
            &self.shadow.depth
        } else {
            &self.shadow.main
        }));
        if !depth {
            commands.push(Command::SetBindGroup(1, self.shadow.receiver()));
        }
        let count = ready.map_or(self.draws.len(), |r| r.objects.len());
        for i in 0..count {
            let geometry = &self.draws[i % self.draws.len()];
            let binds = if ready.is_some() {
                &self.instance_draws[i].shadow
            } else {
                &geometry.shadow
            };
            commands.extend([
                Command::SetBindGroup(0, if depth { &binds.depth } else { &binds.main }),
                Command::SetVertexBuffer(0, &geometry.vertices),
                Command::SetIndexBuffer(&geometry.indices),
                Command::DrawIndexed {
                    indices: 0..geometry.count,
                    base_vertex: 0,
                    instances: 0..1,
                },
            ]);
        }
    }
}
#[cfg(feature = "imported-scene")]
fn model_local_bounds(model: &StaticModel) -> Vec<([f32; 3], [f32; 3])> {
    model
        .source()
        .primitives
        .iter()
        .map(|p| {
            let mut min = [f32::INFINITY; 3];
            let mut max = [f32::NEG_INFINITY; 3];
            for vertex in &p.vertices {
                for axis in 0..3 {
                    min[axis] = min[axis].min(vertex.position[axis]);
                    max[axis] = max[axis].max(vertex.position[axis]);
                }
            }
            (min, max)
        })
        .collect()
}

#[cfg(feature = "imported-scene")]
pub(crate) fn static_instance_draw_count(
    model: &StaticModel,
    count: usize,
) -> Result<usize, StaticInstanceError> {
    let draws = count
        .checked_mul(model.source().primitives.len())
        .ok_or(StaticInstanceError::InstanceLimit)?;
    if count > MAX_STATIC_INSTANCES || draws > MAX_STATIC_INSTANCE_DRAWS {
        return Err(StaticInstanceError::InstanceLimit);
    }
    Ok(draws)
}

#[cfg(feature = "imported-scene")]
pub(crate) fn validate_static_instances(
    model: &StaticModel,
    instances: &[StaticInstance],
) -> Result<(), StaticInstanceError> {
    static_instance_draw_count(model, instances.len())?;
    prepare_static_instances(model, &model_local_bounds(model), instances).map(|_| ())
}

#[cfg(feature = "imported-scene")]
fn prepare_static_instances(
    model: &StaticModel,
    bounds: &[([f32; 3], [f32; 3])],
    instances: &[StaticInstance],
) -> Result<PreparedStatic, StaticInstanceError> {
    let mut objects = Vec::with_capacity(static_instance_draw_count(model, instances.len())?);
    for instance in instances {
        let placement = instance.matrix()?;
        for (p, &(min, max)) in model.source().primitives.iter().zip(bounds) {
            let world = placement.mul(&crate::math3::Mat4(p.transform));
            let normal = orr_model::normal_matrix(world.0)
                .map_err(|_| StaticInstanceError::InvalidPlacement)?;
            if orr_model::determinant(world.0).is_sign_negative()
                != orr_model::determinant(p.transform).is_sign_negative()
            {
                return Err(StaticInstanceError::InvalidPlacement);
            }
            // A transformed AABB bounds every vertex, including rotated/sheared nodes.
            for corner in 0..8 {
                let local = std::array::from_fn(|axis| {
                    if corner & (1 << axis) == 0 {
                        min[axis]
                    } else {
                        max[axis]
                    }
                });
                if !world
                    .transform_point4(local)
                    .iter()
                    .all(|v| v.is_finite() && v.abs() <= 1e9)
                {
                    return Err(StaticInstanceError::InvalidPlacement);
                }
            }
            let material = &model.source().materials[p.material as usize];
            let wrap = |value| match value {
                Wrap::Clamp => 0,
                Wrap::Repeat => 1,
                Wrap::Mirror => 2,
            };
            objects.push(Object {
                world: world.0,
                normal,
                color: material.base_color,
                sampling: [
                    wrap(material.wrap_s),
                    wrap(material.wrap_t),
                    u32::from(material.linear_filter),
                    0,
                ],
            });
        }
    }
    Ok(PreparedStatic { objects })
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
            // HDR output is scene-linear before exposure, tone mapping and encoding.
            if format == TextureFormat::Rgba16Float { 1.0 } else { 0.0 },
        ],
        point_position_range: [0.0; 4],
        point_color_intensity: [0.0; 4],
        light_vp: orr_model::IDENTITY,
        shadow: [0.0; 4],
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

#[cfg(all(test, feature = "imported-scene"))]
mod instance_tests {
    use super::*;

    #[test]
    fn static_trs_applies_scale_then_rotation_then_translation() {
        let half = std::f32::consts::FRAC_PI_4;
        let instance = StaticInstance {
            translation: [3.0, 4.0, 5.0],
            rotation: [0.0, 0.0, half.sin(), half.cos()],
            scale: [2.0, 3.0, 4.0],
        };
        let point = instance.matrix().unwrap().transform_point4([1.0, 2.0, 3.0]);
        for (actual, expected) in point.into_iter().zip([-3.0, 6.0, 17.0, 1.0]) {
            assert!((actual - expected).abs() < 1e-5);
        }
    }

    #[test]
    fn static_trs_rejects_nonfinite_nonunit_mirrored_and_singular_placements() {
        for instance in [
            StaticInstance {
                translation: [f32::NAN, 0.0, 0.0],
                ..Default::default()
            },
            StaticInstance {
                translation: [1e7, 0.0, 0.0],
                ..Default::default()
            },
            StaticInstance {
                rotation: [0.0; 4],
                ..Default::default()
            },
            StaticInstance {
                rotation: [0.0, 0.0, 0.0, 2.0],
                ..Default::default()
            },
            StaticInstance {
                rotation: [f32::INFINITY, 0.0, 0.0, 1.0],
                ..Default::default()
            },
            StaticInstance {
                scale: [0.0, 1.0, 1.0],
                ..Default::default()
            },
            StaticInstance {
                scale: [-1.0, 1.0, 1.0],
                ..Default::default()
            },
            StaticInstance {
                scale: [1e-8; 3],
                ..Default::default()
            },
        ] {
            assert_eq!(
                instance.matrix(),
                Err(StaticInstanceError::InvalidPlacement)
            );
        }
    }
}
