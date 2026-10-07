//! Coordinator-only single-sample hooks. Standalone Renderer3D keeps its original
//! MSAA/shadow/LOD path. Composed frames use the original full-detail meshes.
use super::*;
use crate::point_light::PointLightSettings;

/// Combined procedural meshes and debug lines admitted across a composed frame.
pub const MAX_PROCEDURAL_INSTANCES: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProceduralSceneError {
    Multisampling,
    InstanceLimit,
    InvalidInstance,
}
impl std::fmt::Display for ProceduralSceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Multisampling => "composed procedural renderer requires one sample",
            Self::InstanceLimit => "composed procedural instance/line limit exceeded",
            Self::InvalidInstance => "procedural instances require bounded finite data, unit rotations, positive scale, and nonnegative materials/line widths",
        })
    }
}
impl std::error::Error for ProceduralSceneError {}

pub(crate) struct PreparedProcedural {
    globals: Globals,
    ranges: [std::ops::Range<u32>; 4],
    lines: u32,
    stats: FrameStats,
}
fn procedural_draw_count(list: &RenderList3D) -> Result<usize, ProceduralSceneError> {
    let mut total = list.lines.len();
    for kind in MeshKind::ALL {
        total = total
            .checked_add(list.instances(kind).len())
            .ok_or(ProceduralSceneError::InstanceLimit)?;
    }
    if total > MAX_PROCEDURAL_INSTANCES {
        return Err(ProceduralSceneError::InstanceLimit);
    }
    Ok(MeshKind::ALL
        .iter()
        .filter(|&&kind| !list.instances(kind).is_empty())
        .count()
        + usize::from(!list.lines.is_empty()))
}

pub(crate) fn validate_procedural_list(list: &RenderList3D) -> Result<usize, ProceduralSceneError> {
    let draws = procedural_draw_count(list)?;
    for kind in MeshKind::ALL {
        let instances = list.instances(kind);
        for instance in instances {
            let norm = dot(
                [instance.rot[0], instance.rot[1], instance.rot[2]],
                [instance.rot[0], instance.rot[1], instance.rot[2]],
            ) + instance.rot[3] * instance.rot[3];
            if !bytemuck::cast_slice::<Instance3D, f32>(std::slice::from_ref(instance))
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1e6)
                || !instance.scale.iter().all(|&v| v >= 1e-6)
                || (norm - 1.0).abs() > 1e-3
                || instance.half_length < 0.0
                || !instance
                    .color
                    .iter()
                    .chain(&instance.material[..3])
                    .all(|&v| (0.0..=1e4).contains(&v))
            {
                return Err(ProceduralSceneError::InvalidInstance);
            }
        }
    }
    for line in &list.lines {
        if !bytemuck::cast_slice::<LineInstance3D, f32>(std::slice::from_ref(line))
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 1e6)
            || !(0.0..=4096.0).contains(&line.width)
            || !line.color.iter().all(|&v| (0.0..=1.0).contains(&v))
        {
            return Err(ProceduralSceneError::InvalidInstance);
        }
    }
    Ok(draws)
}

impl<B: Rhi> Renderer3D<B> {
    pub(crate) fn composed_draw_count(
        &self,
        list: &RenderList3D,
    ) -> Result<usize, ProceduralSceneError> {
        if self.samples != 1 {
            return Err(ProceduralSceneError::Multisampling);
        }
        procedural_draw_count(list)
    }

    /// CPU-only validation and preparation; no retained cache or statistics changes.
    pub(crate) fn prepare_composed(
        &self,
        list: &RenderList3D,
        camera: &Camera3D,
        size: (u32, u32),
        lighting: &Lighting,
        point: &PointLightSettings,
        shadow: Option<&crate::shared_shadow::PreparedShadow>,
    ) -> Result<PreparedProcedural, ProceduralSceneError> {
        self.composed_draw_count(list)?;
        validate_procedural_list(list)?;
        let mut first = 0;
        let ranges = std::array::from_fn(|index| {
            let count = list.instances(MeshKind::ALL[index]).len() as u32;
            let range = first..first + count;
            first += count;
            range
        });
        // The coordinator validates common camera/light data first. Its lighting
        // is authoritative; the standalone list's light/clear settings are ignored.
        let mut globals = self.globals(camera, size, lighting, &Mat4::IDENTITY);
        globals.shadow = [0.0; 4];
        globals.params[0] = 0.0;
        if let Some(ready) = shadow {
            globals.light_vp = ready.matrix.0;
            globals.shadow = ready.params;
            globals.params[0] = 1.0;
        }
        if let Some(light) = point.point_light {
            globals.point_position_range = [
                light.position[0],
                light.position[1],
                light.position[2],
                light.range,
            ];
            globals.point_color_intensity = [
                light.color[0],
                light.color[1],
                light.color[2],
                light.intensity,
            ];
        }
        Ok(PreparedProcedural {
            globals,
            ranges,
            lines: list.lines.len() as u32,
            stats: FrameStats {
                mesh_instances: u64::from(first),
                line_instances: list.lines.len() as u64,
                msaa_samples: 1,
                ..FrameStats::default()
            },
        })
    }

    pub(crate) fn write_composed(
        &mut self,
        list: &RenderList3D,
        prepared: &mut PreparedProcedural,
    ) {
        let clock = CpuClock::start();
        let rhi = &self.rhi;
        let stats = &mut prepared.stats;
        stats.upload(rhi, &self.globals, 0, bytemuck::bytes_of(&prepared.globals));
        if stats.mesh_instances > 0 {
            stats.buffer_reallocations +=
                u32::from(self.instances.reserve(rhi, stats.mesh_instances as usize));
            for kind in MeshKind::ALL {
                let instances = list.instances(kind);
                if !instances.is_empty() {
                    stats.upload(
                        rhi,
                        &self.instances.buffer,
                        u64::from(prepared.ranges[kind as usize].start)
                            * std::mem::size_of::<Instance3D>() as u64,
                        bytemuck::cast_slice(instances),
                    );
                }
            }
        }
        if prepared.lines > 0 {
            stats.buffer_reallocations += u32::from(self.lines.reserve(rhi, list.lines.len()));
            stats.upload(
                rhi,
                &self.lines.buffer,
                0,
                bytemuck::cast_slice(&list.lines),
            );
        }
        stats.cpu_prepare_time = clock.elapsed();
    }

    pub(crate) fn encode_composed(
        &self,
        prepared: &mut PreparedProcedural,
        encoder: &mut B::Encoder,
        color: &ColorAttachment<'_, B>,
        depth: &DepthAttachment<'_, B>,
    ) {
        let clock = CpuClock::start();
        let mut commands = Vec::with_capacity(16);
        if prepared.stats.mesh_instances > 0 {
            commands.extend([
                Command::SetPipeline(&self.main_pipeline),
                Command::SetBindGroup(
                    0,
                    if prepared.globals.params[0] > 0.5 {
                        &self
                            .composed_main_bind
                            .as_ref()
                            .expect("shared map bound")
                            .1
                    } else {
                        &self.main_bind
                    },
                ),
                Command::SetVertexBuffer(0, &self.mesh_vertices),
                Command::SetVertexBuffer(1, &self.instances.buffer),
                Command::SetIndexBuffer(&self.mesh_indices),
            ]);
            for kind in MeshKind::ALL {
                let instances = prepared.ranges[kind as usize].clone();
                if !instances.is_empty() {
                    let (indices, base_vertex) = self.mesh_ranges[kind as usize].clone();
                    prepared.stats.main.draw(instances.end - instances.start);
                    commands.push(Command::DrawIndexed {
                        indices,
                        base_vertex,
                        instances,
                    });
                }
            }
        }
        if prepared.lines > 0 {
            commands.extend([
                Command::SetPipeline(&self.line_pipeline),
                Command::SetBindGroup(0, &self.line_bind),
                Command::SetVertexBuffer(0, &self.lines.buffer),
                Command::Draw {
                    vertices: 0..6,
                    instances: 0..prepared.lines,
                },
            ]);
            prepared.stats.main.draw(prepared.lines);
        }
        self.rhi.encode_pass(
            encoder,
            "composed procedural geometry",
            Some(color),
            Some(depth),
            &commands,
        );
        prepared.stats.main.passes = 1;
        prepared.stats.cpu_encode_time = clock.elapsed();
    }

    pub(crate) fn bind_shared_shadow(&mut self, map: &crate::shared_shadow::SharedShadow<B>) {
        if self
            .composed_main_bind
            .as_ref()
            .is_none_or(|(id, _)| *id != map.id)
        {
            self.composed_main_bind = Some((
                map.id,
                self.rhi.create_bind_group(
                    &self.main_pipeline,
                    0,
                    &[
                        Binding::Uniform {
                            binding: 0,
                            buffer: &self.globals,
                        },
                        Binding::Texture {
                            binding: 1,
                            view: &map.view,
                        },
                        Binding::Sampler {
                            binding: 2,
                            sampler: &map.sampler,
                        },
                    ],
                ),
            ));
        }
    }
    pub(crate) fn composed_shadow_commands<'a>(
        &'a self,
        ready: &mut PreparedProcedural,
        commands: &mut Vec<Command<'a, B>>,
    ) {
        commands.extend([
            Command::SetPipeline(&self.shadow_pipeline),
            Command::SetBindGroup(0, &self.composed_shadow_bind),
            Command::SetVertexBuffer(0, &self.mesh_vertices),
            Command::SetVertexBuffer(1, &self.instances.buffer),
            Command::SetIndexBuffer(&self.mesh_indices),
        ]);
        for kind in [MeshKind::Sphere, MeshKind::Box, MeshKind::Capsule] {
            let instances = ready.ranges[kind as usize].clone();
            if !instances.is_empty() {
                let (indices, base_vertex) = self.mesh_ranges[kind as usize].clone();
                ready.stats.shadow.draw(instances.end - instances.start);
                commands.push(Command::DrawIndexed {
                    indices,
                    base_vertex,
                    instances,
                });
            }
        }
        ready.stats.shadow.passes = 1;
    }

    pub(crate) fn commit_composed(&mut self, prepared: PreparedProcedural) {
        self.last_frame_stats = prepared.stats;
        let spheres = &prepared.ranges[MeshKind::Sphere as usize];
        self.last_sphere_lod_stats = SphereLodStats3D {
            near_instances: spheres.end - spheres.start,
            main_draw_calls: u32::from(!spheres.is_empty()),
            shadow_draw_calls: u32::from(prepared.globals.params[0] > 0.5 && !spheres.is_empty()),
            shadow_index_invocations: if prepared.globals.params[0] > 0.5 {
                u64::from(spheres.end - spheres.start)
                    * self.mesh_ranges[MeshKind::Sphere as usize].0.len() as u64
            } else {
                0
            },
            main_index_invocations: u64::from(spheres.end - spheres.start)
                * self.mesh_ranges[MeshKind::Sphere as usize].0.len() as u64,
            upload_calls: u32::from(!spheres.is_empty()),
            upload_bytes: u64::from(spheres.end - spheres.start)
                * std::mem::size_of::<Instance3D>() as u64,
            ..Default::default()
        };
    }
}
