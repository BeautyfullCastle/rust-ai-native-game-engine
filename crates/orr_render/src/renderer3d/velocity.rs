//! Motion vectors of the built-in meshes of a composed frame. Surface shader
//! instances and debug lines are not drawn; the effects reproject them with the camera alone.
use super::*;
use crate::velocity;

/// Where the previous frame's instance sits, read next to the current one: position, rotation and scale.
const BEFORE_ATTRS: [VertexAttr; 3] = [
    VertexAttr { location: 8, format: VertexFormat::Float32x4, offset: 0 },
    VertexAttr { location: 9, format: VertexFormat::Float32x4, offset: 16 },
    VertexAttr { location: 10, format: VertexFormat::Float32x4, offset: 32 },
];

/// What the renderer keeps to draw motion vectors, made when a frame first asks for them.
pub(crate) struct MeshVelocity<B: Rhi> {
    gpu: velocity::Gpu<B>,
    bind: B::BindGroup,
    /// The frame's instances and the last frame's, in list order, kind after kind.
    current: Growing<B>,
    before: Growing<B>,
    trails: [velocity::Trail<Instance3D>; 4],
    ranges: [std::ops::Range<u32>; 4],
}

impl<B: Rhi> MeshVelocity<B> {
    fn new(rhi: &B) -> Self {
        let mesh = VertexLayout { stride: std::mem::size_of::<Vertex3>() as u64, step: VertexStep::Vertex, attrs: &MESH_ATTRS };
        let instance = VertexLayout { stride: std::mem::size_of::<Instance3D>() as u64, step: VertexStep::Instance, attrs: &INSTANCE_ATTRS };
        let before = VertexLayout { stride: std::mem::size_of::<Instance3D>() as u64, step: VertexStep::Instance, attrs: &BEFORE_ATTRS };
        let gpu = velocity::Gpu::new(rhi, "3d motion vectors", "vs_mesh", &[mesh, instance, before], Cull::Back);
        let bind = rhi.create_bind_group(&gpu.pipeline, 0, &[Binding::Uniform { binding: 0, buffer: &gpu.camera }]);
        Self {
            gpu,
            bind,
            current: Growing::new(rhi, "3d motion vector instances", std::mem::size_of::<Instance3D>()),
            before: Growing::new(rhi, "3d motion vector previous instances", std::mem::size_of::<Instance3D>()),
            trails: Default::default(),
            ranges: Default::default(),
        }
    }
}

impl<B: Rhi> Renderer3D<B> {
    /// Uploads the frame's instances with their places in the last frame. Call after the list is validated.
    pub(crate) fn write_velocity(&mut self, list: &RenderList3D, frame: &velocity::Frame) {
        let rhi = &self.rhi;
        let state = self.velocity.get_or_insert_with(|| MeshVelocity::new(rhi));
        let total: usize = MeshKind::ALL.iter().map(|&kind| list.instances(kind).len()).sum();
        let (mut current, mut before) = (Vec::with_capacity(total), Vec::with_capacity(total));
        for kind in MeshKind::ALL {
            let instances = list.instances(kind);
            let start = current.len() as u32;
            let earlier = state.trails[kind as usize].advance(frame.number, instances);
            current.extend_from_slice(instances);
            before.extend_from_slice(&earlier);
            state.ranges[kind as usize] = start..current.len() as u32;
        }
        state.gpu.write_camera(rhi, frame);
        if total > 0 {
            state.current.reserve(rhi, total);
            state.before.reserve(rhi, total);
            rhi.write_buffer(&state.current.buffer, 0, bytemuck::cast_slice(&current));
            rhi.write_buffer(&state.before.buffer, 0, bytemuck::cast_slice(&before));
        }
    }

    /// Draws the written instances into the motion vector target.
    pub(crate) fn velocity_commands<'a>(&'a self, commands: &mut Vec<Command<'a, B>>) {
        let Some(state) = &self.velocity else { return };
        if state.ranges.iter().all(|range| range.is_empty()) {
            return;
        }
        commands.extend([
            Command::SetPipeline(&state.gpu.pipeline),
            Command::SetBindGroup(0, &state.bind),
            Command::SetVertexBuffer(0, &self.mesh_vertices),
            Command::SetVertexBuffer(1, &state.current.buffer),
            Command::SetVertexBuffer(2, &state.before.buffer),
            Command::SetIndexBuffer(&self.mesh_indices),
        ]);
        for kind in MeshKind::ALL {
            let instances = state.ranges[kind as usize].clone();
            if !instances.is_empty() {
                let (indices, base_vertex) = self.mesh_ranges[kind as usize].clone();
                commands.push(Command::DrawIndexed { indices, base_vertex, instances });
            }
        }
    }
}
