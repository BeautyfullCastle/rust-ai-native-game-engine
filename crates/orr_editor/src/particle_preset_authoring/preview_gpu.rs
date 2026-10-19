//! Isolated, fixed-size particle preset preview on the editor's existing device.
//!
//! This uses the production particle renderer, not a CPU image or another GPU.
//! The camera looks down -Z at the origin with +Y up. Its orthographic view is
//! 13 1/3 world units wide and 10 high; particles outside that view are clipped.
//! Preview positions are already relative to an emitter at the origin.

use egui::TextureId;
use orr_render::orr_rhi::{
    ColorAttachment, DepthAttachment, Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu,
};
use orr_render::particle_renderer::{ParticleRenderError, ParticleRenderer, ParticleTarget};
use orr_render::{Camera3D, OffscreenTarget, Projection};
use orr_view::particles::ParticleDraw;

pub(super) const PREVIEW_SIZE: (u32, u32) = (320, 240);
const FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;
const CLEAR_COLOR: [f64; 4] = [0.025, 0.025, 0.035, 1.0];

/// Shared implementation for the native UI and headless real-GPU acceptance.
/// All targets and buffers are allocated once; rendering never resizes them.
pub(super) struct PreviewTarget {
    rhi: Wgpu,
    renderer: ParticleRenderer<Wgpu>,
    color: OffscreenTarget<Wgpu>,
    _depth: <Wgpu as Rhi>::Texture,
    depth_view: <Wgpu as Rhi>::TextureView,
}

impl PreviewTarget {
    pub(super) fn new(rhi: &Wgpu) -> Result<Self, ParticleRenderError> {
        // Check the additional sample/copy usage required by the UI before
        // creating targets. The production renderer checks blending and depth.
        let capabilities = rhi.texture_format_capabilities(FORMAT);
        let usage = TextureUsage::RENDER_ATTACHMENT
            | TextureUsage::TEXTURE_BINDING
            | TextureUsage::COPY_SRC;
        if !capabilities.usages.contains(usage)
            || capabilities.max_dimension_2d < PREVIEW_SIZE.0
            || rhi
                .texture_format_capabilities(TextureFormat::Depth32Float)
                .max_dimension_2d
                < PREVIEW_SIZE.0
        {
            return Err(ParticleRenderError::UnsupportedTarget);
        }
        let renderer = ParticleRenderer::new(rhi.clone(), FORMAT)?;
        let color = OffscreenTarget::new(rhi, PREVIEW_SIZE.0, PREVIEW_SIZE.1, FORMAT);
        let depth = rhi.create_texture(&TextureDesc {
            label: "particle preset preview depth",
            width: PREVIEW_SIZE.0,
            height: PREVIEW_SIZE.1,
            format: TextureFormat::Depth32Float,
            usage: TextureUsage::RENDER_ATTACHMENT,
            sample_count: 1,
            view_formats: &[],
        });
        let depth_view = rhi.create_texture_view(&depth, None);
        Ok(Self {
            rhi: rhi.clone(),
            renderer,
            color,
            _depth: depth,
            depth_view,
        })
    }

    pub(super) fn render(&mut self, draws: &[ParticleDraw]) -> Result<(), ParticleRenderError> {
        let camera = Camera3D {
            eye: [0.0, 0.0, 20.0],
            target: [0.0; 3],
            up: [0.0, 1.0, 0.0],
            projection: Projection::Orthographic {
                half_height: 5.0,
                near: 0.1,
                far: 100.0,
            },
        };
        // Invalid input is rejected without touching the previous image or GPU
        // staging buffers. A valid empty batch still clears both attachments.
        let prepared = self.renderer.prepare(&camera, PREVIEW_SIZE, draws)?;
        let mut encoder = self.rhi.create_encoder("particle preset preview");
        self.rhi.encode_pass(
            &mut encoder,
            "clear particle preset preview",
            Some(&ColorAttachment {
                view: self.color.render_view(),
                clear: Some(CLEAR_COLOR),
                resolve: None,
            }),
            Some(&DepthAttachment {
                view: &self.depth_view,
                clear: Some(1.0),
                store: true,
            }),
            &[],
        );
        self.renderer.upload_and_encode(
            &mut encoder,
            ParticleTarget {
                color: self.color.render_view(),
                depth: &self.depth_view,
                size: PREVIEW_SIZE,
                color_format: FORMAT,
                depth_format: TextureFormat::Depth32Float,
                sample_count: 1,
                depth_size: PREVIEW_SIZE,
                depth_sample_count: 1,
            },
            &prepared,
        )?;
        // Submit before egui samples this view, or a later render overwrites
        // the renderer's shared uniform/instance staging. Exactly one submission
        // contains both the clear and particle passes, including an empty frame.
        self.rhi.submit(encoder);
        Ok(())
    }

    /// Synchronous diagnostic readback of the actual production render target.
    /// Kept out of the normal frame path; no CPU texture upload is performed.
    #[cfg(test)]
    pub(super) fn read_rgba8(&self) -> Vec<u8> {
        self.color.read_rgba8()
    }
}

/// Owns one native egui registration for the lifetime of the preview pane.
pub struct PreviewGpu {
    target: PreviewTarget,
    state: egui_wgpu::RenderState,
    id: TextureId,
}

impl PreviewGpu {
    pub fn new(state: &egui_wgpu::RenderState) -> Result<Self, ParticleRenderError> {
        let rhi = Wgpu::from_parts(
            state.instance.clone(),
            state.adapter.clone(),
            state.device.clone(),
            state.queue.clone(),
        );
        let mut target = PreviewTarget::new(&rhi)?;
        // Do not expose an uninitialized target to the first egui frame.
        target.render(&[])?;
        let id = state.renderer.write().register_native_texture(
            &state.device,
            target.color.sample_view(),
            egui_wgpu::wgpu::FilterMode::Linear,
        );
        Ok(Self {
            target,
            state: state.clone(),
            id,
        })
    }

    pub fn render(&mut self, draws: &[ParticleDraw]) -> Result<(), ParticleRenderError> {
        self.target.render(draws)
    }

    pub fn texture_id(&self) -> TextureId {
        self.id
    }

    pub fn size(&self) -> (u32, u32) {
        PREVIEW_SIZE
    }

    #[cfg(test)]
    pub fn read_rgba8(&self) -> Vec<u8> {
        self.target.read_rgba8()
    }
}

impl Drop for PreviewGpu {
    fn drop(&mut self) {
        self.state.renderer.write().free_texture(&self.id);
    }
}
