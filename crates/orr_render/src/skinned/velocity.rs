//! Motion vectors of skinned models: the placement and joint palette of every draw, now and in the last frame.
//! Morph target changes between frames are not part of the move; the draw's current vertices are used for both.
use super::*;
use crate::velocity;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Pose {
    world: Matrix4,
    joints: [Matrix4; 64],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Poses {
    now: Pose,
    before: Pose,
}

/// What the renderer keeps to draw motion vectors, made when a frame first asks for them.
pub(crate) struct PoseVelocity<B: Rhi> {
    gpu: velocity::Gpu<B>,
    /// The pose uniform of each draw with the bind group that reads it.
    draws: Vec<(B::Buffer, B::BindGroup)>,
    trail: velocity::Trail<Pose>,
}

impl<B: Rhi> PoseVelocity<B> {
    fn new(rhi: &B) -> Self {
        let layout = VertexLayout { stride: std::mem::size_of::<Vertex>() as u64, step: VertexStep::Vertex, attrs: &SKINNED_ATTRS };
        Self { gpu: velocity::Gpu::new(rhi, "skinned model motion vectors", "vs_skinned", &[layout], Cull::Back), draws: Vec::new(), trail: Default::default() }
    }
}

impl<B: Rhi> SkinnedModelRenderer<B> {
    /// Writes each draw's pose with its pose in the last frame.
    pub(crate) fn write_velocity(&mut self, prepared: &PreparedSkinned, frame: &velocity::Frame) {
        let rhi = &self.rhi;
        let state = self.velocity.get_or_insert_with(|| PoseVelocity::new(rhi));
        let now: Vec<Pose> = prepared.objects.iter().map(|object| Pose { world: object.world, joints: object.joints }).collect();
        let before = state.trail.advance(frame.number, &now);
        while state.draws.len() < now.len() {
            let buffer = rhi.create_buffer(&BufferDesc { label: "skinned model poses", size: std::mem::size_of::<Poses>() as u64, usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST });
            let bind = rhi.create_bind_group(
                &state.gpu.pipeline,
                0,
                &[Binding::Uniform { binding: 0, buffer: &state.gpu.camera }, Binding::Uniform { binding: 2, buffer: &buffer }],
            );
            state.draws.push((buffer, bind));
        }
        state.gpu.write_camera(rhi, frame);
        for ((buffer, _), (&now, &before)) in state.draws.iter().zip(now.iter().zip(&before)) {
            rhi.write_buffer(buffer, 0, bytemuck::bytes_of(&Poses { now, before }));
        }
    }

    /// Draws the instances the main pass shows into the motion vector target.
    pub(crate) fn velocity_commands<'a>(&'a self, prepared: &PreparedSkinned, commands: &mut Vec<Command<'a, B>>) {
        let Some(state) = &self.velocity else { return };
        commands.push(Command::SetPipeline(&state.gpu.pipeline));
        for (index, (_, bind)) in state.draws.iter().take(prepared.objects.len()).enumerate().filter(|&(index, _)| prepared.shows(index)) {
            let geometry = &self.geometry[index % self.geometry.len()];
            commands.extend([
                Command::SetBindGroup(0, bind),
                Command::SetVertexBuffer(0, self.vertices(prepared, index)),
                Command::SetIndexBuffer(&geometry.indices),
                Command::DrawIndexed { indices: 0..geometry.count, base_vertex: 0, instances: 0..1 },
            ]);
        }
    }
}
