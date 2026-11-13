//! Render passes that a game or a tool adds to [`crate::ImportedSceneRenderer`].
//!
//! A frame of the imported scene runs in fixed stages. The engine's own
//! batches draw in them, and every added pass draws in the stages it names:
//!
//! 1. [`PassStage::Shadow`]: into the shared sun shadow map, after the
//!    engine's casters, with the light's matrix ([`PassShadow::light_vp`]).
//! 2. [`PassStage::Opaque`]: into the scene color and depth, after the batches.
//! 3. [`PassStage::Sky`]: after everything opaque, so a sky drawn at the far
//!    plane fills only what nothing covers.
//! 4. [`PassStage::Transparent`]: last before particles and post processing;
//!    depth is kept, so blended surfaces sort against the scene.
//!
//! Passes are added once and keep their GPU objects. Each frame the renderer
//! calls [`RenderPass::prepare`] on every pass, then [`RenderPass::encode`]
//! for each stage the pass draws in. Exposure, tone mapping and display
//! encoding stay with the post chain; passes write scene-linear color.
use crate::{Camera3D, Lighting};
use orr_rhi::{ColorAttachment, DepthAttachment, Rhi, TextureFormat};
use std::any::Any;

/// Most passes one renderer takes.
pub const MAX_RENDER_PASSES: usize = 16;

/// Where in the frame a pass draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PassStage {
    Shadow,
    Opaque,
    Sky,
    Transparent,
}

/// The shared sun shadow map of a frame with sun shadows.
pub struct PassShadow<'a, B: Rhi> {
    /// The depth map; casters write it in [`PassStage::Shadow`], receivers sample it.
    pub view: &'a B::TextureView,
    /// A comparison sampler for the map.
    pub sampler: &'a B::Sampler,
    /// World to the map's clip space. Texture coordinates are `(x * 0.5 + 0.5, 0.5 - y * 0.5)`.
    pub light_vp: [[f32; 4]; 4],
    /// x: one texel in texture coordinates, y: normal offset in meters, z: depth bias.
    pub params: [f32; 4],
    /// Changes when the map is made again, so a pass knows to rebuild bind groups that hold it.
    pub map_id: u64,
}

/// What every pass knows about the frame.
pub struct PassFrame<'a, B: Rhi> {
    /// World to clip space of the camera.
    pub view_proj: [[f32; 4]; 4],
    pub camera: &'a Camera3D,
    pub lighting: &'a Lighting,
    /// Size of the scene targets in pixels.
    pub size: (u32, u32),
    /// Format of the scene color target: RGBA16F while post processing is on.
    pub color_format: TextureFormat,
    /// Format of the scene depth target and the shadow map.
    pub depth_format: TextureFormat,
    /// `None` when the frame has no sun shadows.
    pub shadow: Option<PassShadow<'a, B>>,
}

/// A pass added to the imported scene renderer.
pub trait RenderPass<B: Rhi>: Any {
    /// A short name for errors and debugging.
    fn name(&self) -> &str;
    /// The stages the pass draws in. The renderer reads it every frame.
    fn stages(&self) -> &[PassStage];
    /// Writes buffers and builds bind groups for this frame, before any stage is encoded.
    fn prepare(&mut self, rhi: &B, frame: &PassFrame<'_, B>);
    /// Records the pass's draws for `stage`. In [`PassStage::Shadow`] `color`
    /// is `None` and `depth` is the shadow map; in the other stages they are
    /// the scene targets. Both are loaded and stored: the pass never clears them.
    fn encode(&self, rhi: &B, stage: PassStage, frame: &PassFrame<'_, B>, encoder: &mut B::Encoder, color: Option<&ColorAttachment<'_, B>>, depth: &DepthAttachment<'_, B>);
    /// The pass as `Any`, so its owner can reach it again by type.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
