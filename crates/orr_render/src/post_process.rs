//! Optional bounded scene-linear HDR and bloom. Only the final pass applies
//! `Lighting::exposure`, tone mapping and display encoding. Selection/debug lines
//! are scene-linear inputs too: values at or below the threshold do not bloom.
use crate::{ImportedSceneError, Lighting};
use orr_rhi::{
    Binding, Blend, BufferDesc, BufferUsage, ColorAttachment, Command, PipelineDesc, Rhi,
    SamplerDesc, TextureDesc, TextureFormat, TextureUsage,
};

/// Includes all three RGBA16F color targets and old + replacement targets during resize.
/// Depth, asset textures, driver allocator overhead and the caller's display
/// target are outside this color budget. Accepted resize/off transitions drain
/// in-flight work before replacing/releasing targets; steady-state never waits.
pub const MAX_POST_PROCESS_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_HDR_RADIANCE: f32 = 65504.0;

/// Explicit opt-in. These bounds also cap the number of fullscreen passes/taps.
/// Settings are checked even when disabled; invalid requests never alter retained state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostProcessSettings {
    pub enabled: bool,
    pub bloom: bool,
    /// Scene-linear peak-RGB threshold, 0..=65504, applied before downsampling.
    pub threshold: f32,
    /// Additive bloom multiplier, 0..=8. Exposure remains in `Lighting` only.
    pub strength: f32,
    /// Triangle filter half-width in half-resolution texels, 1..=8.
    pub radius: u32,
    /// Horizontal + vertical pairs, 1..=4.
    pub iterations: u32,
}
impl Default for PostProcessSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            bloom: true,
            threshold: 1.0,
            strength: 0.15,
            radius: 4,
            iterations: 2,
        }
    }
}
impl PostProcessSettings {
    pub fn validate(&self) -> Result<(), ImportedSceneError> {
        if !self.threshold.is_finite()
            || !(0.0..=MAX_HDR_RADIANCE).contains(&self.threshold)
            || !self.strength.is_finite()
            || !(0.0..=8.0).contains(&self.strength)
            || !(1..=8).contains(&self.radius)
            || !(1..=4).contains(&self.iterations)
        {
            return Err(ImportedSceneError::InvalidPostProcess);
        }
        Ok(())
    }
    fn bloom_active(self) -> bool {
        self.bloom && self.strength > 0.0
    }
}

pub(crate) fn color_bytes(size: (u32, u32)) -> Option<u64> {
    let (w, h) = (u64::from(size.0), u64::from(size.1));
    w.checked_mul(h)?
        .checked_add(w.div_ceil(2).checked_mul(h.div_ceil(2))?.checked_mul(2)?)?
        .checked_mul(8)
}
pub(crate) fn validate<B: Rhi>(
    rhi: &B,
    settings: PostProcessSettings,
    size: (u32, u32),
    previous: Option<&PostProcessor<B>>,
) -> Result<(), ImportedSceneError> {
    settings.validate()?;
    if !settings.enabled {
        return Ok(());
    }
    let cap = rhi.texture_format_capabilities(TextureFormat::Rgba16Float);
    let readback_bytes = u64::from(size.0)
        .checked_mul(8)
        .and_then(|row| row.checked_add(255))
        .map(|row| row / 256 * 256)
        .and_then(|row| row.checked_mul(u64::from(size.1)))
        .ok_or(ImportedSceneError::UnsupportedHdr)?;
    if !cap.supports_hdr_postprocessing
        || readback_bytes > cap.max_readback_buffer_size
        || !cap.usages.contains(
            TextureUsage::RENDER_ATTACHMENT
                | TextureUsage::TEXTURE_BINDING
                | TextureUsage::COPY_SRC,
        )
        || !cap.filterable
        || !cap.blendable
        || size.0 > cap.max_dimension_2d
        || size.1 > cap.max_dimension_2d
    {
        return Err(ImportedSceneError::UnsupportedHdr);
    }
    let bytes = color_bytes(size).ok_or(ImportedSceneError::PostProcessBudget)?;
    let retained = previous
        .filter(|p| p.size() != size)
        .map_or(0, |p| p.bytes());
    if bytes
        .checked_add(retained)
        .is_none_or(|n| n > MAX_POST_PROCESS_BYTES)
    {
        return Err(ImportedSceneError::PostProcessBudget);
    }
    Ok(())
}

struct Color<B: Rhi> {
    texture: B::Texture,
    view: B::TextureView,
}
struct Targets<B: Rhi> {
    scene: Color<B>,
    half: [Color<B>; 2],
    size: (u32, u32),
    extract_bind: B::BindGroup,
    horizontal_bind: B::BindGroup,
    vertical_bind: B::BindGroup,
    final_bind: B::BindGroup,
}
/// Owns one full-resolution and exactly two ceil-half-resolution RGBA16F targets.
/// No targets, pipelines or passes are created until an enabled frame is admitted.
pub struct PostProcessor<B: Rhi> {
    targets: Targets<B>,
    extract: B::Pipeline,
    blur: B::Pipeline,
    final_pass: B::Pipeline,
    extract_params: B::Buffer,
    horizontal_params: B::Buffer,
    vertical_params: B::Buffer,
    final_params: B::Buffer,
    sampler: B::Sampler,
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    display: [f32; 4],
    filter: [f32; 4],
}
impl<B: Rhi> PostProcessor<B> {
    pub(crate) fn new(rhi: &B, size: (u32, u32), format: TextureFormat) -> Self {
        let shader = rhi.create_shader(
            "HDR bloom and final display",
            include_str!("shader_post_process.wgsl"),
        );
        let pipeline = |label, entry, format| {
            rhi.create_pipeline(&PipelineDesc::color(
                label,
                &shader,
                ("vs_fullscreen", entry),
                &[],
                format,
                Blend::Opaque,
            ))
        };
        let extract = pipeline(
            "HDR bright extraction",
            "fs_extract",
            TextureFormat::Rgba16Float,
        );
        let blur = pipeline("HDR separable bloom", "fs_blur", TextureFormat::Rgba16Float);
        let final_pass = pipeline("HDR final display", "fs_final", format);
        let uniform = |label| {
            rhi.create_buffer(&BufferDesc {
                label,
                size: std::mem::size_of::<Params>() as u64,
                usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST,
            })
        };
        let extract_params = uniform("HDR extract parameters");
        let horizontal_params = uniform("HDR horizontal parameters");
        let vertical_params = uniform("HDR vertical parameters");
        let final_params = uniform("HDR final display parameters");
        let sampler = rhi.create_sampler(&SamplerDesc {
            linear: true,
            compare: false,
        });
        let targets = Self::targets(
            rhi,
            size,
            &extract,
            &blur,
            &final_pass,
            &extract_params,
            &horizontal_params,
            &vertical_params,
            &final_params,
            &sampler,
        );
        Self {
            targets,
            extract,
            blur,
            final_pass,
            extract_params,
            horizontal_params,
            vertical_params,
            final_params,
            sampler,
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn targets(
        rhi: &B,
        size: (u32, u32),
        extract: &B::Pipeline,
        blur: &B::Pipeline,
        final_pass: &B::Pipeline,
        ep: &B::Buffer,
        hp: &B::Buffer,
        vp: &B::Buffer,
        fp: &B::Buffer,
        sampler: &B::Sampler,
    ) -> Targets<B> {
        let color = |label, size: (u32, u32)| {
            let texture = rhi.create_texture(&TextureDesc {
                label,
                width: size.0,
                height: size.1,
                format: TextureFormat::Rgba16Float,
                usage: TextureUsage::RENDER_ATTACHMENT
                    | TextureUsage::TEXTURE_BINDING
                    | TextureUsage::COPY_SRC,
                sample_count: 1,
                view_formats: &[],
            });
            let view = rhi.create_texture_view(&texture, None);
            Color { texture, view }
        };
        let scene = color("HDR scene linear", size);
        let half_size = (size.0.div_ceil(2), size.1.div_ceil(2));
        let half = [
            color("HDR bloom ping", half_size),
            color("HDR bloom pong", half_size),
        ];
        let bind = |pipeline, input: &B::TextureView, params: &B::Buffer| {
            rhi.create_bind_group(
                pipeline,
                0,
                &[
                    Binding::Texture {
                        binding: 0,
                        view: input,
                    },
                    Binding::Uniform {
                        binding: 3,
                        buffer: params,
                    },
                ],
            )
        };
        let extract_bind = bind(extract, &scene.view, ep);
        let horizontal_bind = bind(blur, &half[0].view, hp);
        let vertical_bind = bind(blur, &half[1].view, vp);
        let final_bind = rhi.create_bind_group(
            final_pass,
            0,
            &[
                Binding::Texture {
                    binding: 0,
                    view: &scene.view,
                },
                Binding::Texture {
                    binding: 1,
                    view: &half[0].view,
                },
                Binding::Sampler {
                    binding: 2,
                    sampler,
                },
                Binding::Uniform {
                    binding: 3,
                    buffer: fp,
                },
            ],
        );
        Targets {
            scene,
            half,
            size,
            extract_bind,
            horizontal_bind,
            vertical_bind,
            final_bind,
        }
    }
    pub(crate) fn resize(&mut self, rhi: &B, size: (u32, u32)) {
        if self.size() != size {
            self.targets = Self::targets(
                rhi,
                size,
                &self.extract,
                &self.blur,
                &self.final_pass,
                &self.extract_params,
                &self.horizontal_params,
                &self.vertical_params,
                &self.final_params,
                &self.sampler,
            );
        }
    }
    pub fn size(&self) -> (u32, u32) {
        self.targets.size
    }
    pub fn bytes(&self) -> u64 {
        color_bytes(self.size()).expect("admitted dimensions")
    }
    pub fn scene_texture(&self) -> &B::Texture {
        &self.targets.scene.texture
    }
    pub(crate) fn scene_view(&self) -> &B::TextureView {
        &self.targets.scene.view
    }
    pub(crate) fn encode(
        &self,
        rhi: &B,
        encoder: &mut B::Encoder,
        target: &B::TextureView,
        format: TextureFormat,
        settings: PostProcessSettings,
        lighting: &Lighting,
    ) {
        let write =
            |buffer, params: Params| rhi.write_buffer(buffer, 0, bytemuck::bytes_of(&params));
        let pass = |encoder: &mut B::Encoder, label, pipeline, bind, view| {
            rhi.encode_pass(
                encoder,
                label,
                Some(&ColorAttachment {
                    view,
                    clear: None,
                    resolve: None,
                }),
                None,
                &[
                    Command::SetPipeline(pipeline),
                    Command::SetBindGroup(0, bind),
                    Command::Draw {
                        vertices: 0..3,
                        instances: 0..1,
                    },
                ],
            )
        };
        if settings.bloom_active() {
            write(
                &self.extract_params,
                Params {
                    display: [0.; 4],
                    filter: [settings.threshold, 0., 0., 0.],
                },
            );
            write(
                &self.horizontal_params,
                Params {
                    display: [0.; 4],
                    filter: [0., settings.radius as f32, 1., 0.],
                },
            );
            write(
                &self.vertical_params,
                Params {
                    display: [0.; 4],
                    filter: [0., settings.radius as f32, 0., 1.],
                },
            );
            pass(
                encoder,
                "HDR bright extraction",
                &self.extract,
                &self.targets.extract_bind,
                &self.targets.half[0].view,
            );
            for _ in 0..settings.iterations {
                pass(
                    encoder,
                    "HDR bloom horizontal",
                    &self.blur,
                    &self.targets.horizontal_bind,
                    &self.targets.half[1].view,
                );
                pass(
                    encoder,
                    "HDR bloom vertical",
                    &self.blur,
                    &self.targets.vertical_bind,
                    &self.targets.half[0].view,
                );
            }
        }
        write(
            &self.final_params,
            Params {
                display: [
                    lighting.exposure,
                    if lighting.tonemap { 1. } else { 0. },
                    if format.is_srgb() { 0. } else { 1. },
                    if settings.bloom_active() {
                        settings.strength
                    } else {
                        0.
                    },
                ],
                filter: [0.; 4],
            },
        );
        pass(
            encoder,
            "HDR final display",
            &self.final_pass,
            &self.targets.final_bind,
            target,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_settings_and_odd_sizes() {
        assert_eq!(color_bytes((1, 1)), Some(24));
        assert_eq!(color_bytes((3, 5)), Some((15 + 2 * 6) * 8));
        for bad in [f32::NAN, f32::INFINITY, -1.0, 65505.0] {
            assert!(
                PostProcessSettings {
                    threshold: bad,
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            PostProcessSettings {
                radius: 9,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            PostProcessSettings {
                iterations: 0,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(color_bytes((u32::MAX, u32::MAX)).is_none());
    }
}
