//! The 3D renderer: shadow pass, then one multisampled color+depth pass.
//!
//! # Frame
//!
//! 1. The instances of every mesh kind are written to one instance buffer
//!    (four `write_buffer` calls, no copy on the CPU), the debug lines to
//!    another.
//! 2. Shadow pass (when [`Lighting::shadows`]): all spheres, boxes and
//!    capsules are drawn depth only from the sun into a 2048 x 2048
//!    `Depth32Float` map. The map is an orthographic box around
//!    `shadow_center` with `shadow_radius`; its center snaps to whole texels
//!    so the shadows do not shimmer when the center moves.
//! 3. Main pass into an MSAA (4x when the adapter has it, else 1x) color
//!    texture with a depth buffer, resolved into the target view: meshes,
//!    then debug lines (depth tested, not written).
//!
//! # Shading
//!
//! A metal/roughness Blinn-Phong: see [`crate::Material`] and
//! `shader3d.wgsl`. Linear lighting, ACES tone mapping (optional), and the
//! sRGB encode is done by the hardware when the target format is sRGB and by
//! the shader otherwise, so the picture looks the same either way.
//!
//! # Shadows
//!
//! Hardware compare sampler (`textureSampleCompareLevel` on a
//! `Depth32Float` map with linear filtering, which is a 2x2 bilinear compare)
//! taken on a 3x3 grid: a smooth PCF edge about 4 texels wide. Acne is
//! controlled by a slope scaled depth bias in the shadow pipeline, front
//! face culling there, a normal offset of 1.5 texels and a small constant
//! depth bias.

use bytemuck::{Pod, Zeroable};
use orr_rhi::{
    Binding, Blend, BufferDesc, BufferUsage, ColorAttachment, Command, Compare, Cull, DepthAttachment, DepthState,
    PipelineDesc, Rhi, SamplerDesc, TextureDesc, TextureFormat, TextureUsage, VertexAttr, VertexFormat, VertexLayout,
    VertexStep,
};

use crate::camera3d::Camera3D;
use crate::list3d::{Instance3D, Lighting, LineInstance3D, RenderList3D};
use crate::math3::{cross, dot, normalize, scale, Mat4, Vec3};
use crate::mesh::{MeshKind, MeshSet, Vertex3};
use crate::renderer::Growing;
use crate::stats::{CpuClock, FrameStats, PassStats};

const SHADER: &str = include_str!("shader3d.wgsl");
const DEPTH: TextureFormat = TextureFormat::Depth32Float;

/// Background color of [`Renderer3D`] (linear).
pub const DEFAULT_CLEAR_3D: [f64; 4] = [0.30, 0.42, 0.62, 1.0];

/// Quality settings fixed when the renderer is created.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Settings3D {
    /// Requested MSAA sample count (1, 2, 4, 8). The renderer uses the
    /// largest the adapter supports that is not above it.
    pub msaa: u32,
    /// Side of the square shadow map in texels.
    pub shadow_map_size: u32,
    /// Longitude segments of the sphere and capsule meshes (3 to 64; the latitude rings follow).
    pub mesh_segments: u32,
}

impl Default for Settings3D {
    fn default() -> Self {
        Self { msaa: 4, shadow_map_size: 2048, mesh_segments: crate::mesh::DEFAULT_SEGMENTS }
    }
}

impl Settings3D {
    /// The mobile / weak-GPU preset: no MSAA (the biggest saving on tile and software renderers),
    /// a 512 texel shadow map and 12-segment round meshes (a sphere is 192 triangles instead of 1,536).
    pub const LOW: Settings3D = Settings3D { msaa: 1, shadow_map_size: 512, mesh_segments: 12 };
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    light_vp: [[f32; 4]; 4],
    cam_pos: [f32; 4],
    light_dir: [f32; 4],
    light_color: [f32; 4],
    sky: [f32; 4],
    ground: [f32; 4],
    params: [f32; 4],
    shadow: [f32; 4],
    viewport: [f32; 4],
}

const MESH_ATTRS: [VertexAttr; 3] = [
    VertexAttr { location: 0, format: VertexFormat::Float32x3, offset: 0 },
    VertexAttr { location: 1, format: VertexFormat::Float32x3, offset: 12 },
    VertexAttr { location: 2, format: VertexFormat::Float32, offset: 24 },
];

const INSTANCE_ATTRS: [VertexAttr; 5] = [
    VertexAttr { location: 3, format: VertexFormat::Float32x4, offset: 0 },
    VertexAttr { location: 4, format: VertexFormat::Float32x4, offset: 16 },
    VertexAttr { location: 5, format: VertexFormat::Float32x4, offset: 32 },
    VertexAttr { location: 6, format: VertexFormat::Float32x4, offset: 48 },
    VertexAttr { location: 7, format: VertexFormat::Float32x4, offset: 64 },
];

const LINE_ATTRS: [VertexAttr; 3] = [
    VertexAttr { location: 0, format: VertexFormat::Float32x4, offset: 0 },
    VertexAttr { location: 1, format: VertexFormat::Float32x4, offset: 16 },
    VertexAttr { location: 2, format: VertexFormat::Float32x4, offset: 32 },
];

/// Size dependent textures: the multisampled color buffer and the depth buffer.
struct Attachments<B: Rhi> {
    size: (u32, u32),
    // Kept alive: the views borrow nothing but the textures own the memory.
    _color: Option<B::Texture>,
    color_view: Option<B::TextureView>,
    _depth: B::Texture,
    depth_view: B::TextureView,
}

/// The matrix of the sun's orthographic shadow camera.
///
/// The box is `2 * radius` wide and deep and `6 * radius` long (the eye sits
/// `3 * radius` from the center). Its center is snapped to whole shadow
/// texels along all three light axes.
pub fn light_view_proj(lighting: &Lighting, shadow_map_size: u32) -> Mat4 {
    let dir = normalize(lighting.direction);
    let dir = if dot(dir, dir) < 0.5 { [0.0, -1.0, 0.0] } else { dir };
    let r = lighting.shadow_radius.max(0.5);
    let helper = if dir[1].abs() > 0.95 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let s = normalize(cross(dir, helper));
    let u = cross(s, dir);
    let texel = 2.0 * r / shadow_map_size.max(1) as f32;
    let snap = |v: f32| (v / texel).round() * texel;
    let c = lighting.shadow_center;
    let center: Vec3 = {
        let (cs, cu, cf) = (snap(dot(c, s)), snap(dot(c, u)), snap(dot(c, dir)));
        [
            s[0] * cs + u[0] * cu + dir[0] * cf,
            s[1] * cs + u[1] * cu + dir[1] * cf,
            s[2] * cs + u[2] * cu + dir[2] * cf,
        ]
    };
    let eye = [center[0] - dir[0] * 3.0 * r, center[1] - dir[1] * 3.0 * r, center[2] - dir[2] * 3.0 * r];
    let view = Mat4::look_at(eye, center, u);
    Mat4::orthographic(-r, r, -r, r, 0.1, 6.0 * r).mul(&view)
}

/// Draws [`RenderList3D`]s. Create one per target format.
pub struct Renderer3D<B: Rhi> {
    rhi: B,
    format: TextureFormat,
    settings: Settings3D,
    samples: u32,
    /// Background color, linear RGBA.
    pub clear: [f64; 4],
    shadow_pipeline: B::Pipeline,
    main_pipeline: B::Pipeline,
    line_pipeline: B::Pipeline,
    globals: B::Buffer,
    shadow_globals: B::Buffer,
    main_bind: B::BindGroup,
    shadow_bind: B::BindGroup,
    line_bind: B::BindGroup,
    _shadow_texture: B::Texture,
    shadow_view: B::TextureView,
    _sampler: B::Sampler,
    mesh_vertices: B::Buffer,
    mesh_indices: B::Buffer,
    mesh_ranges: [(std::ops::Range<u32>, i32); 4],
    instances: Growing<B>,
    lines: Growing<B>,
    attachments: Option<Attachments<B>>,
    last_frame_stats: FrameStats,
}

impl<B: Rhi> Renderer3D<B> {
    pub fn new(rhi: B, format: TextureFormat) -> Self {
        Self::with_settings(rhi, format, Settings3D::default())
    }

    pub fn with_settings(rhi: B, format: TextureFormat, settings: Settings3D) -> Self {
        let samples = rhi.supported_samples(format, settings.msaa);
        let shader = rhi.create_shader("orr_render 3d", SHADER);

        let mesh_layout = VertexLayout {
            stride: std::mem::size_of::<Vertex3>() as u64,
            step: VertexStep::Vertex,
            attrs: &MESH_ATTRS,
        };
        let instance_layout = VertexLayout {
            stride: std::mem::size_of::<Instance3D>() as u64,
            step: VertexStep::Instance,
            attrs: &INSTANCE_ATTRS,
        };
        let line_layout = VertexLayout {
            stride: std::mem::size_of::<LineInstance3D>() as u64,
            step: VertexStep::Instance,
            attrs: &LINE_ATTRS,
        };

        let shadow_pipeline = rhi.create_pipeline(&PipelineDesc::<B> {
            label: "3d shadow",
            shader: &shader,
            vs_entry: "vs_shadow",
            fs_entry: "",
            vertex_buffers: &[mesh_layout, instance_layout],
            color_format: None,
            blend: Blend::Opaque,
            topology: orr_rhi::Topology::TriangleList,
            cull: Cull::Front,
            depth: Some(DepthState { format: DEPTH, write: true, compare: Compare::Less, bias: 2, slope_bias: 2.0 }),
            samples: 1,
        });
        let main_pipeline = rhi.create_pipeline(&PipelineDesc::<B> {
            label: "3d main",
            shader: &shader,
            vs_entry: "vs_main",
            fs_entry: "fs_main",
            vertex_buffers: &[mesh_layout, instance_layout],
            color_format: Some(format),
            blend: Blend::Opaque,
            topology: orr_rhi::Topology::TriangleList,
            cull: Cull::Back,
            depth: Some(DepthState::opaque(DEPTH)),
            samples,
        });
        let line_pipeline = rhi.create_pipeline(&PipelineDesc::<B> {
            label: "3d lines",
            shader: &shader,
            vs_entry: "vs_line",
            fs_entry: "fs_line",
            vertex_buffers: &[line_layout],
            color_format: Some(format),
            blend: Blend::Alpha,
            topology: orr_rhi::Topology::TriangleList,
            cull: Cull::None,
            depth: Some(DepthState { format: DEPTH, write: false, compare: Compare::LessEqual, bias: -2, slope_bias: -1.0 }),
            samples,
        });

        let uniform = |label: &str| {
            rhi.create_buffer(&BufferDesc {
                label,
                size: std::mem::size_of::<Globals>() as u64,
                usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST,
            })
        };
        let globals = uniform("3d globals");
        let shadow_globals = uniform("3d shadow globals");

        let shadow_texture = rhi.create_texture(&TextureDesc {
            label: "shadow map",
            width: settings.shadow_map_size,
            height: settings.shadow_map_size,
            format: DEPTH,
            usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::TEXTURE_BINDING,
            sample_count: 1,
            view_formats: &[],
        });
        let shadow_view = rhi.create_texture_view(&shadow_texture, None);
        let sampler = rhi.create_sampler(&SamplerDesc { linear: true, compare: true });

        let main_bind = rhi.create_bind_group(
            &main_pipeline,
            0,
            &[
                Binding::Uniform { binding: 0, buffer: &globals },
                Binding::Texture { binding: 1, view: &shadow_view },
                Binding::Sampler { binding: 2, sampler: &sampler },
            ],
        );
        let shadow_bind = rhi.create_bind_group(&shadow_pipeline, 0, &[Binding::Uniform { binding: 0, buffer: &shadow_globals }]);
        let line_bind = rhi.create_bind_group(&line_pipeline, 0, &[Binding::Uniform { binding: 0, buffer: &globals }]);

        let meshes = MeshSet::build_with(settings.mesh_segments);
        let mesh_vertices = rhi.create_buffer(&BufferDesc {
            label: "mesh vertices",
            size: (meshes.vertices.len() * std::mem::size_of::<Vertex3>()) as u64,
            usage: BufferUsage::VERTEX | BufferUsage::COPY_DST,
        });
        rhi.write_buffer(&mesh_vertices, 0, bytemuck::cast_slice(&meshes.vertices));
        let mesh_indices = rhi.create_buffer(&BufferDesc {
            label: "mesh indices",
            size: (meshes.indices.len() * 4) as u64,
            usage: BufferUsage::INDEX | BufferUsage::COPY_DST,
        });
        rhi.write_buffer(&mesh_indices, 0, bytemuck::cast_slice(&meshes.indices));

        let instances = Growing::new(&rhi, "3d instances", std::mem::size_of::<Instance3D>());
        let lines = Growing::new(&rhi, "3d lines", std::mem::size_of::<LineInstance3D>());
        Self {
            rhi,
            format,
            settings,
            samples,
            clear: DEFAULT_CLEAR_3D,
            shadow_pipeline,
            main_pipeline,
            line_pipeline,
            globals,
            shadow_globals,
            main_bind,
            shadow_bind,
            line_bind,
            _shadow_texture: shadow_texture,
            shadow_view,
            _sampler: sampler,
            mesh_vertices,
            mesh_indices,
            mesh_ranges: meshes.ranges,
            instances,
            lines,
            attachments: None,
            last_frame_stats: FrameStats::default(),
        }
    }

    pub fn format(&self) -> TextureFormat {
        self.format
    }

    pub fn rhi(&self) -> &B {
        &self.rhi
    }

    /// The MSAA sample count in use (the request, or less when unsupported).
    pub fn samples(&self) -> u32 {
        self.samples
    }

    pub fn settings(&self) -> Settings3D {
        self.settings
    }

    /// Work submitted by the last completed draw, including shadow-map clear passes.
    pub fn last_frame_stats(&self) -> FrameStats {
        self.last_frame_stats
    }

    fn ensure_attachments(&mut self, size: (u32, u32)) -> u32 {
        if self.attachments.as_ref().is_some_and(|a| a.size == size) {
            return 0;
        }
        let rhi = &self.rhi;
        let (color, color_view) = if self.samples > 1 {
            let t = rhi.create_texture(&TextureDesc {
                label: "3d msaa color",
                width: size.0,
                height: size.1,
                format: self.format,
                usage: TextureUsage::RENDER_ATTACHMENT,
                sample_count: self.samples,
                view_formats: &[],
            });
            let v = rhi.create_texture_view(&t, None);
            (Some(t), Some(v))
        } else {
            (None, None)
        };
        let depth = rhi.create_texture(&TextureDesc {
            label: "3d depth",
            width: size.0,
            height: size.1,
            format: DEPTH,
            usage: TextureUsage::RENDER_ATTACHMENT,
            sample_count: self.samples,
            view_formats: &[],
        });
        let depth_view = rhi.create_texture_view(&depth, None);
        self.attachments = Some(Attachments { size, _color: color, color_view, _depth: depth, depth_view });
        1 + u32::from(self.samples > 1)
    }

    fn globals(&self, camera: &Camera3D, size: (u32, u32), lighting: &Lighting, light_vp: &Mat4) -> Globals {
        let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
        let to_light = normalize(scale(lighting.direction, -1.0));
        let texel_world = 2.0 * lighting.shadow_radius.max(0.5) / self.settings.shadow_map_size.max(1) as f32;
        Globals {
            view_proj: camera.view_proj(w / h).0,
            light_vp: light_vp.0,
            cam_pos: [camera.eye[0], camera.eye[1], camera.eye[2], 1.0],
            light_dir: [to_light[0], to_light[1], to_light[2], lighting.intensity],
            light_color: [lighting.color[0], lighting.color[1], lighting.color[2], 1.0],
            sky: [lighting.sky[0], lighting.sky[1], lighting.sky[2], lighting.ambient],
            ground: [lighting.ground[0], lighting.ground[1], lighting.ground[2], 1.0],
            params: [
                if lighting.shadows { 1.0 } else { 0.0 },
                if lighting.tonemap { 1.0 } else { 0.0 },
                if self.format.is_srgb() { 0.0 } else { 1.0 },
                lighting.exposure,
            ],
            shadow: [1.0 / self.settings.shadow_map_size.max(1) as f32, texel_world * 1.5, 0.0004, 0.0],
            viewport: [w, h, 0.0, 0.0],
        }
    }

    /// The clear color as the target stores it: a non sRGB target is written
    /// without hardware encoding, so the value is encoded here too.
    fn clear_value(&self) -> [f64; 4] {
        if self.format.is_srgb() {
            return self.clear;
        }
        let enc = |c: f64| if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
        [enc(self.clear[0]), enc(self.clear[1]), enc(self.clear[2]), self.clear[3]]
    }

    /// Draws `list` into `view` (`size` pixels, multisampled internally and
    /// resolved into `view`) seen through `camera`, and submits it.
    pub fn draw(&mut self, view: &B::TextureView, size: (u32, u32), list: &RenderList3D, camera: &Camera3D) {
        let prepare_clock = CpuClock::start();
        let replacing_attachments = self.attachments.is_some();
        let mut stats = FrameStats {
            mesh_instances: list.instance_count() as u64,
            line_instances: list.lines.len() as u64,
            msaa_samples: self.samples,
            attachment_allocations: self.ensure_attachments(size),
            ..FrameStats::default()
        };
        if replacing_attachments {
            stats.attachment_reallocations = stats.attachment_allocations;
        }
        let light_vp = light_view_proj(&list.lighting, self.settings.shadow_map_size);
        let globals = self.globals(camera, size, &list.lighting, &light_vp);
        let shadow_globals = Globals { view_proj: light_vp.0, ..globals };
        let rhi = &self.rhi;
        stats.upload(rhi, &self.globals, 0, bytemuck::bytes_of(&globals));
        stats.upload(rhi, &self.shadow_globals, 0, bytemuck::bytes_of(&shadow_globals));

        // One instance buffer, the kinds back to back in `MeshKind` order.
        let total = list.instance_count();
        let mut ranges = [0u32..0u32, 0..0, 0..0, 0..0];
        if total > 0 {
            stats.buffer_reallocations += u32::from(self.instances.reserve(rhi, total));
            let item = std::mem::size_of::<Instance3D>() as u64;
            let mut first = 0u32;
            for kind in MeshKind::ALL {
                let part = list.instances(kind);
                if !part.is_empty() {
                    stats.upload(rhi, &self.instances.buffer, u64::from(first) * item, bytemuck::cast_slice(part));
                }
                ranges[kind as usize] = first..first + part.len() as u32;
                first += part.len() as u32;
            }
        }
        if !list.lines.is_empty() {
            stats.buffer_reallocations += u32::from(self.lines.reserve(rhi, list.lines.len()));
            stats.upload(rhi, &self.lines.buffer, 0, bytemuck::cast_slice(&list.lines));
        }

        stats.cpu_prepare_time = prepare_clock.elapsed();
        let encode_clock = CpuClock::start();
        let draw_meshes = |commands: &mut Vec<Command<B>>, with_planes: bool, pass: &mut PassStats| {
            for kind in MeshKind::ALL {
                if kind == MeshKind::Plane && !with_planes {
                    continue;
                }
                let instances = ranges[kind as usize].clone();
                if instances.is_empty() {
                    continue;
                }
                let (indices, base_vertex) = self.mesh_ranges[kind as usize].clone();
                pass.draw(instances.end - instances.start);
                commands.push(Command::DrawIndexed { indices, base_vertex, instances });
            }
        };

        let mut encoder = rhi.create_encoder("3d frame");
        if list.lighting.shadows && total > 0 {
            let mut commands: Vec<Command<B>> = vec![
                Command::SetPipeline(&self.shadow_pipeline),
                Command::SetBindGroup(0, &self.shadow_bind),
                Command::SetVertexBuffer(0, &self.mesh_vertices),
                Command::SetVertexBuffer(1, &self.instances.buffer),
                Command::SetIndexBuffer(&self.mesh_indices),
            ];
            draw_meshes(&mut commands, false, &mut stats.shadow);
            rhi.encode_pass(
                &mut encoder,
                "3d shadow",
                None,
                Some(&DepthAttachment { view: &self.shadow_view, clear: Some(1.0), store: true }),
                &commands,
            );
        } else {
            // Nothing was drawn into the map: clear it so stale shadows never show.
            rhi.encode_pass(
                &mut encoder,
                "3d shadow clear",
                None,
                Some(&DepthAttachment { view: &self.shadow_view, clear: Some(1.0), store: true }),
                &[],
            );
        }

        stats.shadow.passes += 1;

        let mut commands: Vec<Command<B>> = Vec::with_capacity(24);
        if total > 0 {
            commands.push(Command::SetPipeline(&self.main_pipeline));
            commands.push(Command::SetBindGroup(0, &self.main_bind));
            commands.push(Command::SetVertexBuffer(0, &self.mesh_vertices));
            commands.push(Command::SetVertexBuffer(1, &self.instances.buffer));
            commands.push(Command::SetIndexBuffer(&self.mesh_indices));
            draw_meshes(&mut commands, true, &mut stats.main);
        }
        if !list.lines.is_empty() {
            commands.push(Command::SetPipeline(&self.line_pipeline));
            commands.push(Command::SetBindGroup(0, &self.line_bind));
            commands.push(Command::SetVertexBuffer(0, &self.lines.buffer));
            commands.push(Command::Draw { vertices: 0..6, instances: 0..list.lines.len() as u32 });
            stats.main.draw(list.lines.len() as u32);
        }
        let att = self.attachments.as_ref().expect("attachments created above");
        let color = match &att.color_view {
            Some(msaa) => ColorAttachment { view: msaa, clear: Some(self.clear_value()), resolve: Some(view) },
            None => ColorAttachment { view, clear: Some(self.clear_value()), resolve: None },
        };
        rhi.encode_pass(
            &mut encoder,
            "3d main",
            Some(&color),
            Some(&DepthAttachment { view: &att.depth_view, clear: Some(1.0), store: false }),
            &commands,
        );
        stats.main.passes += 1;
        stats.cpu_encode_time = encode_clock.elapsed();
        let submit_clock = CpuClock::start();
        rhi.submit(encoder);
        stats.cpu_submit_time = submit_clock.elapsed();
        self.last_frame_stats = stats;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_matrix_maps_the_center_to_the_middle_of_the_box() {
        let l = Lighting { shadow_center: [3.0, 0.5, -2.0], shadow_radius: 20.0, ..Lighting::default() };
        let m = light_view_proj(&l, 2048);
        let c = m.transform_point4(l.shadow_center);
        // Snapping moves the center by less than one texel (2 * 20 / 2048 world units, 1 / 1024 clip).
        assert!(c[0].abs() < 2.0 / 1024.0 && c[1].abs() < 2.0 / 1024.0, "{c:?}");
        assert!(c[2] > 0.0 && c[2] < 1.0, "{c:?}");
    }

    #[test]
    fn light_matrix_is_stable_while_the_center_moves_less_than_a_texel() {
        let a = Lighting { shadow_center: [0.0, 0.0, 0.0], shadow_radius: 20.0, ..Lighting::default() };
        let b = Lighting { shadow_center: [0.003, 0.0, 0.0], ..a };
        assert_eq!(light_view_proj(&a, 2048), light_view_proj(&b, 2048));
    }

    #[test]
    fn straight_down_light_does_not_break_the_basis() {
        let l = Lighting { direction: [0.0, -1.0, 0.0], ..Lighting::default() };
        let m = light_view_proj(&l, 1024);
        assert!(m.0.iter().flatten().all(|v| v.is_finite()));
    }
}
