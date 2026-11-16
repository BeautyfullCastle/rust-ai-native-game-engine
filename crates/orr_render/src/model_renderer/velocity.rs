//! Motion vectors of instanced static models: the placement of every draw now and in the last frame.
use super::*;
use crate::velocity;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Placement {
    world: [[f32; 4]; 4],
    before: [[f32; 4]; 4],
}

/// What the renderer keeps to draw motion vectors, made when a frame first asks for them.
pub(crate) struct InstanceVelocity<B: Rhi> {
    gpu: velocity::Gpu<B>,
    /// The placement uniform of each draw with the bind group that reads it.
    draws: Vec<(B::Buffer, B::BindGroup)>,
    trail: velocity::Trail<[[f32; 4]; 4]>,
}

impl<B: Rhi> InstanceVelocity<B> {
    fn new(rhi: &B) -> Self {
        let layout = velocity::position_layout(std::mem::size_of::<Vertex>() as u64);
        Self { gpu: velocity::Gpu::new(rhi, "static model motion vectors", "vs_static", &[layout], Cull::Back), draws: Vec::new(), trail: Default::default() }
    }
}

impl<B: Rhi> ModelRenderer<B> {
    /// Writes each draw's placement with its place in the last frame.
    pub(crate) fn write_velocity(&mut self, prepared: &PreparedStatic, frame: &velocity::Frame) {
        let rhi = &self.rhi;
        let state = self.velocity.get_or_insert_with(|| InstanceVelocity::new(rhi));
        let worlds: Vec<[[f32; 4]; 4]> = prepared.objects.iter().map(|object| object.world).collect();
        let before = state.trail.advance(frame.number, &worlds);
        while state.draws.len() < worlds.len() {
            let buffer = rhi.create_buffer(&BufferDesc { label: "static model placement", size: std::mem::size_of::<Placement>() as u64, usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST });
            let bind = rhi.create_bind_group(
                &state.gpu.pipeline,
                0,
                &[Binding::Uniform { binding: 0, buffer: &state.gpu.camera }, Binding::Uniform { binding: 1, buffer: &buffer }],
            );
            state.draws.push((buffer, bind));
        }
        state.gpu.write_camera(rhi, frame);
        for ((buffer, _), (&world, &before)) in state.draws.iter().zip(worlds.iter().zip(&before)) {
            rhi.write_buffer(buffer, 0, bytemuck::bytes_of(&Placement { world, before }));
        }
    }

    /// Draws the instances the main pass shows into the motion vector target.
    pub(crate) fn velocity_commands<'a>(&'a self, prepared: &PreparedStatic, commands: &mut Vec<Command<'a, B>>) {
        let Some(state) = &self.velocity else { return };
        commands.push(Command::SetPipeline(&state.gpu.pipeline));
        for (index, (_, bind)) in state.draws.iter().take(prepared.objects.len()).enumerate().filter(|&(index, _)| prepared.shows(index)) {
            let geometry = &self.draws[index % self.draws.len()];
            commands.extend([
                Command::SetBindGroup(0, bind),
                Command::SetVertexBuffer(0, &geometry.vertices),
                Command::SetIndexBuffer(&geometry.indices),
                Command::DrawIndexed { indices: 0..geometry.count, base_vertex: 0, instances: 0..1 },
            ]);
        }
    }
}
