//! Decals: colors projected onto the opaque scene from boxes.
//!
//! A decal multiplies the lit color of every opaque surface inside its box, so
//! it darkens or tints (dirt, scorch marks, paint on a light wall) and keeps
//! the light and shadow under it. It cannot make a surface lighter. Surfaces
//! turned more than about 70° away from the projection direction fade out.
//! One frame holds up to [`MAX_DECALS`]; later ones are not drawn.
//!
//! A decal is a round or square spot with a soft edge, or a rectangle of the
//! renderer's decal atlas (`ImportedSceneRenderer::set_decal_atlas`): the
//! atlas color is multiplied by the decal color and its alpha is the cover.
// Only the imported scene draws decals; without it the types stay for render lists.
#![cfg_attr(not(feature = "imported-scene"), allow(dead_code))]

use bytemuck::{Pod, Zeroable};

/// Decals one frame draws.
pub const MAX_DECALS: usize = 32;
/// Widest and tallest decal atlas.
pub const MAX_DECAL_ATLAS_SIZE: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DecalShape {
    /// An ellipse filling the box's width and height.
    #[default]
    Round,
    /// The whole width and height of the box.
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decal {
    /// Center of the box.
    pub center: [f32; 3],
    /// The direction the decal is projected along, toward the surface. It need not be a unit vector.
    pub direction: [f32; 3],
    /// The top of the decal on the surface. Only its part across `direction` is used.
    pub up: [f32; 3],
    /// Width, height and depth of the box in meters. Depth runs along `direction`.
    pub size: [f32; 3],
    /// Linear color the surface is multiplied by, and opacity.
    pub color: [f32; 4],
    pub shape: DecalShape,
    /// Share of the edge that fades, 0 (hard) to 1 (fades from the center).
    pub softness: f32,
    /// Rectangle of the decal atlas as `[u0, v0, u1, v1]`, with v down. `None` draws `shape`.
    pub region: Option<[f32; 4]>,
}

impl Default for Decal {
    /// A dark round spot one meter wide, projected straight down.
    fn default() -> Self {
        Self {
            center: [0.0; 3],
            direction: [0.0, -1.0, 0.0],
            up: [0.0, 0.0, -1.0],
            size: [1.0; 3],
            color: [0.1, 0.1, 0.1, 1.0],
            shape: DecalShape::Round,
            softness: 0.3,
            region: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecalAtlasError {
    /// Width or height is zero or above [`MAX_DECAL_ATLAS_SIZE`].
    Size,
    /// The pixels are not `width * height * 4` bytes.
    DataLength,
    /// The render backend cannot upload textures.
    Unsupported,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuDecal {
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    color: [f32; 4],
    region: [f32; 4],
    style: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DecalUniform {
    inv_view_proj: [[f32; 4]; 4],
    info: [f32; 4],
    list: [GpuDecal; MAX_DECALS],
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn unit(v: [f32; 3]) -> Option<[f32; 3]> {
    let length = dot(v, v).sqrt();
    (length.is_finite() && length > 1e-6).then(|| v.map(|c| c / length))
}

/// Unit forward, up and right of a decal, or `None` without a direction.
fn basis(direction: [f32; 3], up: [f32; 3]) -> Option<[[f32; 3]; 3]> {
    let forward = unit(direction)?;
    // An `up` along the direction falls back to any axis across it.
    let up = [up, [0.0, 0.0, -1.0], [1.0, 0.0, 0.0]].into_iter().find_map(|up| {
        let along = dot(up, forward);
        unit([up[0] - forward[0] * along, up[1] - forward[1] * along, up[2] - forward[2] * along])
    })?;
    Some([forward, up, cross(forward, up)])
}

impl Decal {
    /// The decal as the shader reads it, or `None` when it cannot show.
    fn gpu(&self) -> Option<GpuDecal> {
        let finite = |v: &[f32]| v.iter().all(|c| c.is_finite());
        if !finite(&self.center) || !finite(&self.size) || !finite(&self.color) || !self.softness.is_finite() {
            return None;
        }
        if self.size.iter().any(|s| *s <= 0.0) || self.color[3] <= 0.0 {
            return None;
        }
        let [forward, up, right] = basis(self.direction, self.up)?;
        let row = |axis: [f32; 3], size: f32| {
            let a = axis.map(|c| c / size);
            [a[0], a[1], a[2], -dot(a, self.center)]
        };
        let region = self.region.filter(|r| r.iter().all(|c| c.is_finite()));
        Some(GpuDecal {
            right: row(right, self.size[0]),
            up: row(up, self.size[1]),
            forward: row(forward, self.size[2]),
            color: [self.color[0].max(0.0), self.color[1].max(0.0), self.color[2].max(0.0), self.color[3].min(1.0)],
            region: region.unwrap_or([0.0, 0.0, 1.0, 1.0]),
            style: [
                if self.shape == DecalShape::Square { 1.0 } else { 0.0 },
                self.softness.clamp(0.0, 1.0),
                if region.is_some() { 1.0 } else { 0.0 },
                0.0,
            ],
        })
    }
}

fn uniform(decals: &[Decal], view_proj: [[f32; 4]; 4]) -> (DecalUniform, usize) {
    let mut u = DecalUniform::zeroed();
    u.inv_view_proj = crate::post_effects::inverse(view_proj);
    let mut count = 0;
    for gpu in decals.iter().filter_map(Decal::gpu).take(MAX_DECALS) {
        u.list[count] = gpu;
        count += 1;
    }
    u.info[0] = count as f32;
    (u, count)
}

#[cfg(feature = "imported-scene")]
pub(crate) use pass::DecalPass;

#[cfg(feature = "imported-scene")]
mod pass {
    use super::*;
    use orr_rhi::*;

    /// The pipeline, uniform and atlas of the decals of one renderer.
    pub(crate) struct DecalPass<B: Rhi> {
        shader: B::Shader,
        pipeline: Option<(TextureFormat, B::Pipeline)>,
        uniform: B::Buffer,
        sampler: B::Sampler,
        _atlas: B::Texture,
        atlas_view: B::TextureView,
    }

    fn atlas_texture<B: Rhi>(rhi: &B, width: u32, height: u32) -> B::Texture {
        rhi.create_texture(&TextureDesc {
            label: "decal atlas",
            width,
            height,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsage::TEXTURE_BINDING.or(TextureUsage::COPY_DST),
            sample_count: 1,
            view_formats: &[],
        })
    }

    impl<B: Rhi> DecalPass<B> {
        pub(crate) fn new(rhi: &B) -> Self {
            // Until an atlas is set, a white pixel: a region then draws the decal color as a square.
            let atlas = atlas_texture(rhi, 1, 1);
            let _ = rhi.write_texture_rgba8(&atlas, &TextureUpload { origin: [0, 0], width: 1, height: 1, bytes_per_row: 4, data: &[255; 4] });
            Self {
                shader: rhi.create_shader("decals", include_str!("decals.wgsl")),
                pipeline: None,
                uniform: rhi.create_buffer(&BufferDesc { label: "decals", size: std::mem::size_of::<DecalUniform>() as u64, usage: BufferUsage::UNIFORM.or(BufferUsage::COPY_DST) }),
                sampler: rhi.create_sampler(&SamplerDesc::LINEAR),
                atlas_view: rhi.create_texture_view(&atlas, None),
                _atlas: atlas,
            }
        }

        pub(crate) fn set_atlas(&mut self, rhi: &B, width: u32, height: u32, rgba: &[u8]) -> Result<(), DecalAtlasError> {
            if width == 0 || height == 0 || width > MAX_DECAL_ATLAS_SIZE || height > MAX_DECAL_ATLAS_SIZE {
                return Err(DecalAtlasError::Size);
            }
            if rgba.len() as u64 != u64::from(width) * u64::from(height) * 4 {
                return Err(DecalAtlasError::DataLength);
            }
            let atlas = atlas_texture(rhi, width, height);
            rhi.write_texture_rgba8(&atlas, &TextureUpload { origin: [0, 0], width, height, bytes_per_row: width * 4, data: rgba })
                .map_err(|_| DecalAtlasError::Unsupported)?;
            self.atlas_view = rhi.create_texture_view(&atlas, None);
            self._atlas = atlas;
            Ok(())
        }

        /// Multiplies `color` by the decals around the surfaces of `depth`.
        #[allow(clippy::too_many_arguments)] // One frame's attachments and camera.
        pub(crate) fn encode(&mut self, rhi: &B, encoder: &mut B::Encoder, color: &B::TextureView, format: TextureFormat, depth: &B::TextureView, decals: &[Decal], view_proj: [[f32; 4]; 4]) {
            let (u, count) = uniform(decals, view_proj);
            if count == 0 {
                return;
            }
            if self.pipeline.as_ref().is_none_or(|(f, _)| *f != format) {
                let pipeline = rhi.create_pipeline(&PipelineDesc::color("decals", &self.shader, ("vs_fullscreen", "fs_decals"), &[], format, Blend::Multiply));
                self.pipeline = Some((format, pipeline));
            }
            let pipeline = &self.pipeline.as_ref().expect("made above").1;
            rhi.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
            let bind = rhi.create_bind_group(pipeline, 0, &[
                Binding::Uniform { binding: 0, buffer: &self.uniform },
                Binding::Texture { binding: 1, view: depth },
                Binding::Texture { binding: 2, view: &self.atlas_view },
                Binding::Sampler { binding: 3, sampler: &self.sampler },
            ]);
            rhi.encode_pass(
                encoder,
                "decals",
                Some(&ColorAttachment { view: color, clear: None, resolve: None }),
                None,
                &[Command::SetPipeline(pipeline), Command::SetBindGroup(0, &bind), Command::Draw { vertices: 0..3, instances: 0..1 }],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: [[f32; 4]; 4] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

    fn local(d: &GpuDecal, p: [f32; 3]) -> [f32; 3] {
        [d.right, d.up, d.forward].map(|r| r[0] * p[0] + r[1] * p[1] + r[2] * p[2] + r[3])
    }

    #[test]
    fn the_box_maps_its_corners_to_half_units() {
        let decal = Decal { center: [1.0, 2.0, 3.0], size: [2.0, 4.0, 1.0], ..Default::default() };
        let d = decal.gpu().unwrap();
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5);
        assert!(close(local(&d, [1.0, 2.0, 3.0]), [0.0; 3]));
        // Projected down with its top toward -z, its right is +x; half a meter down is half its depth.
        assert!(close(local(&d, [0.0, 2.0, 3.0]), [-0.5, 0.0, 0.0]));
        assert!(close(local(&d, [1.0, 2.0, 1.0]), [0.0, 0.5, 0.0]));
        assert!(close(local(&d, [1.0, 1.5, 3.0]), [0.0, 0.0, 0.5]));
    }

    #[test]
    fn an_up_along_the_direction_still_gives_a_box() {
        let decal = Decal { up: [0.0, 3.0, 0.0], ..Default::default() };
        assert!(decal.gpu().is_some());
    }

    #[test]
    fn decals_that_cannot_show_are_skipped_and_the_rest_are_capped() {
        let bad = [
            Decal { size: [0.0, 1.0, 1.0], ..Default::default() },
            Decal { color: [1.0, 1.0, 1.0, 0.0], ..Default::default() },
            Decal { direction: [0.0; 3], ..Default::default() },
            Decal { center: [f32::NAN, 0.0, 0.0], ..Default::default() },
        ];
        assert!(bad.iter().all(|d| d.gpu().is_none()));
        let many = vec![Decal::default(); MAX_DECALS + 5];
        assert_eq!(uniform(&bad, IDENTITY).1, 0);
        assert_eq!(uniform(&many, IDENTITY).1, MAX_DECALS);
    }
}
