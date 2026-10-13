//! Bounded, single-sample composition of procedural, static and animated models.
//!
//! A frame is validated in full before GPU allocation, uniform writes, command
//! encoding, or submitted-bound changes. The coordinator owns shared main depth
//! and one bounded directional map, clears each once, and submits one encoder.
//! Standalone renderer draws retain their independent depth targets and behavior.
use crate::shared_shadow::{
    MAX_IMPORTED_CASTERS, SHARED_SHADOW_MAP_SIZE, SharedShadow, prepare_shared_globals,
};
#[cfg(feature = "animation")]
use crate::skinned::{PreparedSkinned, SkinnedInstance, SkinnedModelRenderer, SkinnedRenderError};
use crate::{
    Camera3D, Lighting, RenderList3D, Renderer3D,
    model_renderer::{
        ModelRenderError, ModelRenderer, PreparedStatic, StaticInstance, StaticInstanceError,
        static_instance_draw_count, validate_static_instances,
    },
    point_light::{PointLightError, PointLightSettings},
    renderer3d::{PreparedProcedural, ProceduralSceneError, validate_procedural_list},
};
use orr_rhi::{ColorAttachment, DepthAttachment, Rhi, TextureDesc, TextureFormat, TextureUsage};

/// Bound CPU frame preparation and the number of render passes.
pub const MAX_IMPORTED_BATCHES: usize = 256;
/// Bound combined procedural mesh/line and imported instance/primitive draws.
pub const MAX_IMPORTED_DRAWS: usize = 4096;

#[derive(Debug)]
pub enum ImportedSceneError {
    InvalidTarget,
    UnsupportedHdr,
    InvalidPostProcess,
    PostProcessBudget,
    FormatMismatch,
    BatchLimit,
    DrawLimit,
    CasterLimit,
    Model(ModelRenderError),
    StaticInstance(StaticInstanceError),
    Procedural(ProceduralSceneError),
    #[cfg(feature = "animation")]
    Skinned(SkinnedRenderError),
    PointLight(PointLightError),
    #[cfg(feature = "particles")]
    Particles(crate::particle_renderer::ParticleRenderError),
    #[cfg(feature = "irradiance-probes")]
    Irradiance(String),
}
impl std::fmt::Display for ImportedSceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTarget => f.write_str(
                "imported scene requires a single-sample color target, finite clear, and dimensions in 1..=8192",
            ),
            Self::UnsupportedHdr => f.write_str("RGBA16F render/sample/filter/blend/readback or requested dimensions unsupported"),
            Self::InvalidPostProcess => f.write_str("invalid bounded HDR/bloom settings"),
            Self::PostProcessBudget => f.write_str("HDR color targets exceed the 128 MiB retained/transient resize budget"),
            Self::FormatMismatch => f.write_str("imported scene target and renderer formats must match"),
            Self::BatchLimit => f.write_str("imported scene batch limit exceeded"),
            Self::CasterLimit => f.write_str("imported scene shadow caster limit exceeded"),
            Self::DrawLimit => f.write_str("imported scene draw limit exceeded"),
            Self::Model(error) => write!(f, "imported scene: {error}"),
            Self::StaticInstance(error) => write!(f, "imported scene: {error}"),
            Self::Procedural(error) => write!(f, "imported scene: {error}"),
            #[cfg(feature = "animation")]
            Self::Skinned(error) => write!(f, "imported scene: {error}"),
            Self::PointLight(error) => write!(f, "imported scene: {error}"),
            #[cfg(feature = "particles")]
            Self::Particles(error) => write!(f, "{error}"),
            #[cfg(feature = "irradiance-probes")]
            Self::Irradiance(error) => write!(f, "irradiance grid: {error}"),
        }
    }
}
impl std::error::Error for ImportedSceneError {}

/// Immutable, device-independent camera admission for one exact viewport.
///
/// The stored column-major main matrix is computed once by the renderer's camera
/// validation and uploaded unchanged by [`ImportedSceneRenderer::draw_prepared`].
/// This token does not certify terrain clearance or the target view's metadata.
/// Existing preparation still performs redundant camera calculations for admission;
/// successful prepared draws replace their main matrices before any frame effects.
#[derive(Clone, Copy, Debug)]
pub struct PreparedSceneCamera {
    camera: Camera3D,
    viewport: (u32, u32),
    view_proj: [[f32; 4]; 4],
}
impl PreparedSceneCamera {
    /// CPU-only, fixed-size preparation. Dimensions are checked before division.
    pub fn new(camera: Camera3D, viewport: (u32, u32)) -> Result<Self, ImportedSceneError> {
        if viewport.0 == 0 || viewport.1 == 0 || viewport.0 > 8192 || viewport.1 > 8192 {
            return Err(ImportedSceneError::InvalidTarget);
        }
        let view_proj = crate::model_renderer::validate_camera(&camera, viewport)
            .map_err(ImportedSceneError::Model)?;
        Ok(Self { camera, viewport, view_proj })
    }

    /// Camera snapshot used for all eye/projection/forward metadata.
    pub fn camera(&self) -> Camera3D {
        self.camera
    }

    pub fn viewport(&self) -> (u32, u32) {
        self.viewport
    }

    /// Exact stored column-major f32 coefficients; modifying this copy cannot
    /// alter the token. Certify these coefficients, then draw this same token.
    pub fn view_proj(&self) -> [[f32; 4]; 4] {
        self.view_proj
    }
}

/// Color attachment metadata, checked before any frame effects.
///
/// The RHI deliberately exposes no view introspection. The caller must supply
/// the view's actual size, format, and sample count, and use the same underlying
/// device for the target, coordinator, and all renderers, just as for RHI calls.
pub struct ImportedSceneTarget<'a, B: Rhi> {
    pub view: &'a B::TextureView,
    pub size: (u32, u32),
    pub format: TextureFormat,
    pub sample_count: u32,
}

/// Optional bounded particle batch, sharing the scene color and stored depth.
#[cfg(feature = "particles")]
pub struct ParticleBatch<'a, B: Rhi> {
    pub renderer: &'a mut crate::particle_renderer::ParticleRenderer<B>,
    pub items: &'a [orr_view::particles::ParticleDraw],
}

/// A renderer can occur only once in a frame: exclusive borrows prevent repeated
/// uses from overwriting uniforms referenced by an earlier batch. Put all poses
/// for one animated asset into the same `Skinned` batch instead.
///
/// A renderer's standalone `clear` is ignored; the coordinator clears the frame.
pub enum ImportedBatch<'a, B: Rhi> {
    Static(&'a mut ModelRenderer<B>),
    /// Independent external TRS placements of one immutable imported asset.
    StaticInstances {
        renderer: &'a mut ModelRenderer<B>,
        instances: &'a [StaticInstance],
    },
    /// Single-sample, full-detail procedural geometry. Coordinator lighting is
    /// used; list lighting, renderer clear, private shadow map and LOD are ignored.
    Procedural {
        renderer: &'a mut Renderer3D<B>,
        list: &'a RenderList3D,
    },
    #[cfg(feature = "animation")]
    Skinned {
        renderer: &'a mut SkinnedModelRenderer<B>,
        instances: &'a [SkinnedInstance<'a>],
    },
}

struct Depth<B: Rhi> {
    _texture: B::Texture,
    view: B::TextureView,
    size: (u32, u32),
}

enum PreparedBatch {
    Static,
    StaticInstances(PreparedStatic),
    Procedural(Box<PreparedProcedural>),
    #[cfg(feature = "animation")]
    Skinned(PreparedSkinned),
}

/// Shared scene depth and frame submission for procedural and imported geometry.
/// One fixed-size directional shadow map and opt-in bounded HDR/bloom. No cascades, multisampling, PBR,
/// or cross-device composition. Off-coverage receivers are fully lit.
pub struct ImportedSceneRenderer<B: Rhi> {
    rhi: B,
    format: TextureFormat,
    depth: Option<Depth<B>>,
    depth_generation: u64,
    shadow: Option<SharedShadow<B>>,
    /// HDR clear is scene-linear and receives the final display transform.
    /// Legacy clear semantics are unchanged when HDR is disabled.
    pub clear: [f64; 4],
    pub post_process: crate::PostProcessSettings,
    /// Optional diffuse irradiance. Validated before any frame resource/cache mutation.
    /// No coefficient storage is replicated per instance: each asset renderer has
    /// one 9280-byte globals extension (procedural renderers have two). Existing
    /// viewport asset limits bound retained storage; toggles allocate no variants.
    #[cfg(feature = "irradiance-probes")]
    pub irradiance: Option<crate::IrradianceGrid>,
    post_processor: Option<crate::PostProcessor<B>>,
    post_process_generation: u64,
}
impl<B: Rhi> ImportedSceneRenderer<B> {
    /// Create a coordinator without allocating GPU resources.
    pub fn new(rhi: B, format: TextureFormat) -> Result<Self, ImportedSceneError> {
        if format.is_depth() {
            return Err(ImportedSceneError::InvalidTarget);
        }
        Ok(Self {
            rhi,
            format,
            depth: None,
            depth_generation: 0,
            shadow: None,
            clear: crate::DEFAULT_CLEAR_3D,
            post_process: crate::PostProcessSettings::default(),
            #[cfg(feature = "irradiance-probes")]
            irradiance: None,
            post_processor: None,
            post_process_generation: 0,
        })
    }
    pub fn format(&self) -> TextureFormat {
        self.format
    }
    /// Required format of every procedural/static/skinned scene pipeline.
    /// Enabled postprocessing requires an RGBA8/BGRA8 final display format.
    /// `format()` remains the caller's final display target format.
    pub fn scene_format(&self) -> TextureFormat {
        if self.post_process.enabled {
            TextureFormat::Rgba16Float
        } else {
            self.format
        }
    }
    pub fn post_process_size(&self) -> Option<(u32, u32)> {
        self.post_processor.as_ref().map(|p| p.size())
    }
    /// Advances only when an accepted HDR frame creates/replaces color targets.
    pub fn post_process_generation(&self) -> u64 {
        self.post_process_generation
    }
    pub fn post_process_allocated_bytes(&self) -> u64 {
        self.post_processor.as_ref().map_or(0, |p| p.bytes())
    }
    /// Read-only pre-display scene texture for diagnostics and rendering oracles.
    pub fn hdr_scene_texture(&self) -> Option<&B::Texture> {
        self.post_processor.as_ref().map(|p| p.scene_texture())
    }
    /// Synchronous diagnostic readback, independent of final display encoding.
    pub fn read_hdr_rgba(&self) -> Option<Vec<[f32; 4]>> {
        self.hdr_scene_texture().map(|texture| {
            self.rhi
                .read_texture(texture)
                .chunks_exact(8)
                .map(|pixel| {
                    orr_rhi::decode_rgba16f_texel(pixel.try_into().expect("RGBA16F texel"))
                })
                .collect()
        })
    }
    pub fn depth_size(&self) -> Option<(u32, u32)> {
        self.depth.as_ref().map(|depth| depth.size)
    }
    /// Zero before the first accepted frame; advances on accepted size changes.
    pub fn depth_generation(&self) -> u64 {
        self.depth_generation
    }

    /// Fixed map allocation is independent of main viewport resizing.
    pub fn shadow_map_size(&self) -> Option<u32> {
        self.shadow.as_ref().map(|_| SHARED_SHADOW_MAP_SIZE)
    }
    /// Zero before the first accepted shadow-on frame, then one for its fixed map.
    pub fn shadow_map_generation(&self) -> u64 {
        u64::from(self.shadow.is_some())
    }

    fn validate_target(
        &self,
        size: (u32, u32),
        format: TextureFormat,
        samples: u32,
    ) -> Result<(), ImportedSceneError> {
        if size.0 == 0
            || size.1 == 0
            || size.0 > 8192
            || size.1 > 8192
            || format.is_depth()
            || samples != 1
            || !self.clear.iter().all(|value| value.is_finite())
        {
            return Err(ImportedSceneError::InvalidTarget);
        }
        if format != self.format {
            return Err(ImportedSceneError::FormatMismatch);
        }
        crate::post_process::validate(
            &self.rhi,
            self.post_process,
            size,
            self.post_processor.as_ref(),
        )?;
        if self.post_process.enabled
            && (self.format == TextureFormat::Rgba16Float
                || !self.clear[..3].iter().all(|v| (0.0..=65504.0).contains(v))
                || !(0.0..=1.0).contains(&self.clear[3]))
        {
            return Err(ImportedSceneError::InvalidTarget);
        }
        Ok(())
    }

    /// Pure authoring-frame validation before target resizing or GPU asset-cache creation.
    /// Admits exactly one procedural batch and the supplied static-instance batches,
    /// using this coordinator's scene format/clear and a single-sample target. Renderers
    /// constructed afterwards must use that same format, device, and one sample.
    /// The accepted data must remain unchanged until `draw`, which validates again.
    /// No GPU calls or retained state changes occur, including on success.
    pub fn preflight(
        &self,
        size: (u32, u32),
        camera: &Camera3D,
        lighting: &Lighting,
        point: &PointLightSettings,
        procedural: &RenderList3D,
        models: &[(&orr_model::StaticModel, &[StaticInstance])],
    ) -> Result<(), ImportedSceneError> {
        self.preflight_cpu(size, camera, lighting, point, procedural, models, 0, 0)
    }

    /// Mixed static/skinned authoring-frame validation before target resizing or
    /// GPU cache creation. The static-only `preflight` API remains unchanged.
    #[cfg(feature = "animation")]
    #[allow(clippy::too_many_arguments)] // Additive counterpart to the static preflight API.
    pub fn preflight_mixed(
        &self,
        size: (u32, u32),
        camera: &Camera3D,
        lighting: &Lighting,
        point: &PointLightSettings,
        procedural: &RenderList3D,
        models: &[(&orr_model::StaticModel, &[StaticInstance])],
        skinned: &[(&orr_model::animation::AnimatedModel, &[SkinnedInstance<'_>])],
    ) -> Result<(), ImportedSceneError> {
        let skinned_draws = skinned
            .iter()
            .try_fold(0usize, |count, (model, instances)| {
                count
                    .checked_add(
                        crate::skinned::skinned_draw_count(model, instances.len())
                            .map_err(ImportedSceneError::Skinned)?,
                    )
                    .ok_or(ImportedSceneError::DrawLimit)
            })?;
        self.preflight_cpu(
            size,
            camera,
            lighting,
            point,
            procedural,
            models,
            skinned.len(),
            skinned_draws,
        )?;
        // Admit budgets and static data first, then run potentially expensive
        // current-pose deformation checks. No GPU state has changed yet.
        for (model, instances) in skinned {
            crate::skinned::validate_skinned_instances(model, instances)
                .map_err(ImportedSceneError::Skinned)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // Shared static/mixed admission without API churn.
    fn preflight_cpu(
        &self,
        size: (u32, u32),
        camera: &Camera3D,
        lighting: &Lighting,
        point: &PointLightSettings,
        procedural: &RenderList3D,
        models: &[(&orr_model::StaticModel, &[StaticInstance])],
        extra_batch_count: usize,
        extra_draw_count: usize,
    ) -> Result<(), ImportedSceneError> {
        #[cfg(feature = "irradiance-probes")]
        if let Some(grid) = &self.irradiance {
            grid.validate().map_err(ImportedSceneError::Irradiance)?;
        }
        self.validate_target(size, self.format, 1)?;
        prepare_shared_globals(self.scene_format(), size, camera, lighting)
            .map_err(ImportedSceneError::Model)?;
        point.validate().map_err(ImportedSceneError::PointLight)?;
        let model_batch_count = models
            .len()
            .checked_add(extra_batch_count)
            .ok_or(ImportedSceneError::BatchLimit)?;
        // The procedural list is one additional batch at submission time.
        if model_batch_count >= MAX_IMPORTED_BATCHES {
            return Err(ImportedSceneError::BatchLimit);
        }
        let mut draws = 0usize;
        for (model, instances) in models {
            draws = draws
                .checked_add(
                    static_instance_draw_count(model, instances.len())
                        .map_err(ImportedSceneError::StaticInstance)?,
                )
                .ok_or(ImportedSceneError::DrawLimit)?;
            if draws > MAX_IMPORTED_DRAWS {
                return Err(ImportedSceneError::DrawLimit);
            }
        }
        let imported_draws = draws
            .checked_add(extra_draw_count)
            .ok_or(ImportedSceneError::DrawLimit)?;
        draws = imported_draws
            .checked_add(
                validate_procedural_list(
                    procedural,
                    self.scene_format() == TextureFormat::Rgba16Float,
                )
                .map_err(ImportedSceneError::Procedural)?,
            )
            .ok_or(ImportedSceneError::DrawLimit)?;
        if draws > MAX_IMPORTED_DRAWS {
            return Err(ImportedSceneError::DrawLimit);
        }
        // Keep the existing main-budget error precedence, including when shadows are off.
        let casters = imported_draws
            .checked_add(procedural_casters(procedural))
            .ok_or(ImportedSceneError::CasterLimit)?;
        if casters > MAX_IMPORTED_CASTERS {
            return Err(ImportedSceneError::CasterLimit);
        }
        for (model, instances) in models {
            validate_static_instances(model, instances)
                .map_err(ImportedSceneError::StaticInstance)?;
        }
        Ok(())
    }

    /// Validate the whole frame, upload it, clear once, and submit once.
    ///
    /// Invalid frames preserve color/depth, depth size/generation, renderer
    /// uniform caches, and previously submitted animated bounds. An empty frame
    /// still clears both attachments. Depth is loaded/stored between batches,
    /// so unequal-depth opaque overlap is independent of submission order.
    pub fn draw(
        &mut self,
        target: ImportedSceneTarget<'_, B>,
        camera: &Camera3D,
        lighting: &Lighting,
        point: &PointLightSettings,
        batches: &mut [ImportedBatch<'_, B>],
    ) -> Result<(), ImportedSceneError> {
        self.draw_inner(target, camera, lighting, point, batches, None, #[cfg(feature = "particles")] None)
    }

    /// Draw using the token's exact stored main matrix for all scene batches.
    ///
    /// A viewport mismatch rejects before frame effects, even for equal aspect
    /// ratios. All existing target, lighting, batch and postprocess admission
    /// remains in force. The target metadata must truthfully describe its view.
    /// This entry accepts no particles; their independent camera path is unchanged.
    pub fn draw_prepared(
        &mut self,
        target: ImportedSceneTarget<'_, B>,
        camera: &PreparedSceneCamera,
        lighting: &Lighting,
        point: &PointLightSettings,
        batches: &mut [ImportedBatch<'_, B>],
    ) -> Result<(), ImportedSceneError> {
        if target.size != camera.viewport {
            return Err(ImportedSceneError::InvalidTarget);
        }
        self.draw_inner(target, &camera.camera, lighting, point, batches, Some(camera), #[cfg(feature = "particles")] None)
    }

    /// Adds a preflighted visual-only particle pass before HDR postprocessing.
    #[cfg(feature = "particles")]
    #[allow(clippy::too_many_arguments)] // Keep the existing draw arguments plus one optional pass.
    pub fn draw_with_particles(
        &mut self, target: ImportedSceneTarget<'_, B>, camera: &Camera3D,
        lighting: &Lighting, point: &PointLightSettings, batches: &mut [ImportedBatch<'_, B>],
        particles: Option<ParticleBatch<'_, B>>,
    ) -> Result<(), ImportedSceneError> {
        self.draw_inner(target, camera, lighting, point, batches, None, particles)
    }

    #[allow(clippy::too_many_arguments)] // Shared implementation preserves the existing draw API.
    fn draw_inner(
        &mut self, target: ImportedSceneTarget<'_, B>, camera: &Camera3D,
        lighting: &Lighting, point: &PointLightSettings, batches: &mut [ImportedBatch<'_, B>],
        prepared_camera: Option<&PreparedSceneCamera>,
        #[cfg(feature = "particles")] mut particles: Option<ParticleBatch<'_, B>>,
    ) -> Result<(), ImportedSceneError> {
        // CPU-only probe admission comes before all allocation, cache and upload effects.
        #[cfg(feature = "irradiance-probes")]
        let irradiance = match &self.irradiance {
            Some(grid) => grid.packed_uniform().map_err(ImportedSceneError::Irradiance)?,
            None => <crate::IrradianceUniform as bytemuck::Zeroable>::zeroed(),
        };
        self.validate_target(target.size, target.format, target.sample_count)?;
        #[cfg(feature = "particles")]
        let particle_ready = if let Some(batch) = &particles {
            if batch.renderer.format() != self.scene_format() { return Err(ImportedSceneError::FormatMismatch); }
            Some(batch.renderer.prepare(camera, target.size, batch.items).map_err(ImportedSceneError::Particles)?)
        } else { None };
        #[cfg(feature = "particles")]
        let particle_passes = usize::from(particle_ready.as_ref().is_some_and(|p| p.particle_count() > 0));
        #[cfg(not(feature = "particles"))]
        let particle_passes = 0usize;
        if batches.len().checked_add(particle_passes).is_none_or(|n| n > MAX_IMPORTED_BATCHES) {
            return Err(ImportedSceneError::BatchLimit);
        }
        let (mut globals, shadow_ready) =
            prepare_shared_globals(self.scene_format(), target.size, camera, lighting)
                .map_err(ImportedSceneError::Model)?;
        #[cfg(feature = "irradiance-probes")]
        {
            globals.irradiance = irradiance;
        }
        point.validate().map_err(ImportedSceneError::PointLight)?;
        if let Some(light) = &point.point_light {
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

        // Admit all formats and budgets before potentially expensive pose checks.
        #[cfg(feature = "particles")]
        let mut draw_count = particle_ready.as_ref().map_or(0, |p| p.draw_count());
        #[cfg(not(feature = "particles"))]
        let mut draw_count = 0usize;
        let mut procedural_instances = 0usize;
        let mut caster_count = 0usize;
        for batch in batches.iter() {
            let (format, count) = match batch {
                ImportedBatch::Static(renderer) => (renderer.format(), renderer.draw_count()),
                ImportedBatch::StaticInstances {
                    renderer,
                    instances,
                } => (
                    renderer.format(),
                    renderer
                        .instance_draw_count(instances.len())
                        .map_err(ImportedSceneError::StaticInstance)?,
                ),
                ImportedBatch::Procedural { renderer, list } => (
                    renderer.format(),
                    renderer
                        .composed_draw_count(list)
                        .map_err(ImportedSceneError::Procedural)?,
                ),
                #[cfg(feature = "animation")]
                ImportedBatch::Skinned {
                    renderer,
                    instances,
                } => (
                    renderer.format(),
                    renderer
                        .draw_count(instances.len())
                        .map_err(ImportedSceneError::Skinned)?,
                ),
            };
            if format != self.scene_format() {
                return Err(ImportedSceneError::FormatMismatch);
            }
            if let ImportedBatch::Procedural { list, .. } = batch {
                procedural_instances += list.instance_count() + list.lines.len();
                if procedural_instances > crate::renderer3d::MAX_PROCEDURAL_INSTANCES {
                    return Err(ImportedSceneError::Procedural(
                        ProceduralSceneError::InstanceLimit,
                    ));
                }
            }
            draw_count = draw_count
                .checked_add(count)
                .ok_or(ImportedSceneError::DrawLimit)?;
            if draw_count > MAX_IMPORTED_DRAWS {
                return Err(ImportedSceneError::DrawLimit);
            }
            let casters = match batch {
                ImportedBatch::Procedural { list, .. } => procedural_casters(list),
                _ => count,
            };
            caster_count = caster_count
                .checked_add(casters)
                .ok_or(ImportedSceneError::CasterLimit)?;
            if caster_count > MAX_IMPORTED_CASTERS {
                return Err(ImportedSceneError::CasterLimit);
            }
        }
        let mut prepared = batches
            .iter()
            .map(|batch| match batch {
                ImportedBatch::Static(_) => Ok(PreparedBatch::Static),
                ImportedBatch::StaticInstances {
                    renderer,
                    instances,
                } => renderer
                    .prepare_instances(instances)
                    .map(PreparedBatch::StaticInstances)
                    .map_err(ImportedSceneError::StaticInstance),
                ImportedBatch::Procedural { renderer, list } => renderer
                    .prepare_composed(
                        list,
                        camera,
                        target.size,
                        lighting,
                        point,
                        shadow_ready.as_ref(),
                    )
                    .map(|ready| PreparedBatch::Procedural(Box::new(ready)))
                    .map_err(ImportedSceneError::Procedural),
                #[cfg(feature = "animation")]
                ImportedBatch::Skinned {
                    renderer,
                    instances,
                } => renderer
                    .prepare_instances(instances)
                    .map(PreparedBatch::Skinned)
                    .map_err(ImportedSceneError::Skinned),
            })
            .collect::<Result<Vec<_>, ImportedSceneError>>()?;

        #[cfg(feature = "irradiance-probes")]
        for ready in &mut prepared {
            if let PreparedBatch::Procedural(ready) = ready {
                ready.set_irradiance(irradiance);
            }
        }

        if let Some(camera) = prepared_camera {
            globals.set_view_proj(camera.view_proj);
            for ready in &mut prepared {
                if let PreparedBatch::Procedural(ready) = ready {
                    ready.set_view_proj(camera.view_proj);
                }
            }
        }

        // All fallible validation is finished, including the final batch/pose.
        if self.post_process.enabled {
            if self
                .post_processor
                .as_ref()
                .is_none_or(|p| p.size() != target.size)
            {
                if let Some(p) = self.post_processor.as_mut() {
                    // Retire older in-flight generations before allocating the
                    // admitted old + new peak. Only resource transitions stall.
                    self.rhi.wait_idle();
                    p.resize(&self.rhi, target.size);
                } else {
                    self.post_processor = Some(crate::PostProcessor::new(
                        &self.rhi,
                        target.size,
                        self.format,
                    ));
                }
                self.post_process_generation = self.post_process_generation.saturating_add(1);
            }
        } else {
            // Retire in-flight HDR references before releasing this ownership;
            // a rapid off/on sequence cannot hide an older color generation.
            if self.post_processor.is_some() {
                self.rhi.wait_idle();
            }
            self.post_processor = None;
        }
        if shadow_ready.is_some() && self.shadow.is_none() {
            self.shadow = Some(SharedShadow::new(&self.rhi));
        }
        // This path never allocates a
        // renderer's standalone depth target, including on the first frame.
        if self
            .depth
            .as_ref()
            .is_none_or(|depth| depth.size != target.size)
        {
            let texture = self.rhi.create_texture(&TextureDesc {
                label: "imported scene shared depth",
                width: target.size.0,
                height: target.size.1,
                format: TextureFormat::Depth32Float,
                usage: TextureUsage::RENDER_ATTACHMENT,
                sample_count: 1,
                view_formats: &[],
            });
            let view = self.rhi.create_texture_view(&texture, None);
            self.depth = Some(Depth {
                _texture: texture,
                view,
                size: target.size,
            });
            self.depth_generation = self.depth_generation.saturating_add(1);
        }
        for (batch, ready) in batches.iter_mut().zip(&mut prepared) {
            match (batch, ready) {
                (ImportedBatch::Static(renderer), PreparedBatch::Static) => {
                    renderer.write_frame(&globals)
                }
                (
                    ImportedBatch::StaticInstances { renderer, .. },
                    PreparedBatch::StaticInstances(ready),
                ) => renderer.write_instances(&globals, ready),
                (
                    ImportedBatch::Procedural { renderer, list },
                    PreparedBatch::Procedural(ready),
                ) => renderer.write_composed(list, ready),
                #[cfg(feature = "animation")]
                (ImportedBatch::Skinned { renderer, .. }, PreparedBatch::Skinned(ready)) => {
                    renderer.write_frame(&globals, ready)
                }
                _ => unreachable!("prepared batches preserve order and kind"),
            }
        }
        if shadow_ready.is_some() {
            let map = self.shadow.as_ref().expect("validated shadow map");
            for batch in batches.iter_mut() {
                match batch {
                    ImportedBatch::Static(r)
                    | ImportedBatch::StaticInstances { renderer: r, .. } => {
                        r.bind_shared_shadow(map)
                    }
                    ImportedBatch::Procedural { renderer, .. } => renderer.bind_shared_shadow(map),
                    #[cfg(feature = "animation")]
                    ImportedBatch::Skinned { renderer, .. } => renderer.bind_shared_shadow(map),
                }
            }
        }
        let mut encoder = self.rhi.create_encoder("imported scene frame");
        if shadow_ready.is_some() {
            let mut commands = Vec::new();
            for (batch, ready) in batches.iter().zip(&mut prepared) {
                match (batch, ready) {
                    (ImportedBatch::Static(r), PreparedBatch::Static) => {
                        r.shared_commands(None, true, &mut commands)
                    }
                    (
                        ImportedBatch::StaticInstances { renderer, .. },
                        PreparedBatch::StaticInstances(ready),
                    ) => renderer.shared_commands(Some(ready), true, &mut commands),
                    (
                        ImportedBatch::Procedural { renderer, .. },
                        PreparedBatch::Procedural(ready),
                    ) => renderer.composed_shadow_commands(ready, &mut commands),
                    #[cfg(feature = "animation")]
                    (ImportedBatch::Skinned { renderer, .. }, PreparedBatch::Skinned(ready)) => {
                        renderer.shared_commands(ready, true, &mut commands)
                    }
                    _ => unreachable!("validated prepared batch kind"),
                }
            }
            self.rhi.encode_pass(
                &mut encoder,
                "shared directional shadow",
                None,
                Some(&DepthAttachment {
                    view: &self.shadow.as_ref().expect("validated shadow map").view,
                    clear: Some(1.0),
                    store: true,
                }),
                &commands,
            );
        }
        let depth = &self.depth.as_ref().expect("accepted frame owns depth").view;
        let scene_view = self
            .post_processor
            .as_ref()
            .map_or(target.view, |p| p.scene_view());
        if batches.is_empty() {
            self.rhi.encode_pass(
                &mut encoder,
                "empty imported scene",
                Some(&ColorAttachment {
                    view: scene_view,
                    clear: Some(self.clear),
                    resolve: None,
                }),
                Some(&DepthAttachment {
                    view: depth,
                    clear: Some(1.0),
                    store: true,
                }),
                &[],
            );
        }
        for (index, (batch, ready)) in batches.iter().zip(&mut prepared).enumerate() {
            let color = ColorAttachment {
                view: scene_view,
                clear: (index == 0).then_some(self.clear),
                resolve: None,
            };
            let depth = DepthAttachment {
                view: depth,
                clear: (index == 0).then_some(1.0),
                store: true,
            };
            match (batch, ready) {
                (ImportedBatch::Static(renderer), PreparedBatch::Static) => {
                    if shadow_ready.is_some() {
                        let mut commands = Vec::new();
                        renderer.shared_commands(None, false, &mut commands);
                        self.rhi.encode_pass(
                            &mut encoder,
                            "shared static receiver",
                            Some(&color),
                            Some(&depth),
                            &commands,
                        );
                    } else {
                        renderer.encode_frame(&mut encoder, &color, &depth)
                    }
                }
                (
                    ImportedBatch::StaticInstances { renderer, .. },
                    PreparedBatch::StaticInstances(ready),
                ) => {
                    if shadow_ready.is_some() {
                        let mut commands = Vec::new();
                        renderer.shared_commands(Some(ready), false, &mut commands);
                        self.rhi.encode_pass(
                            &mut encoder,
                            "shared static instance receiver",
                            Some(&color),
                            Some(&depth),
                            &commands,
                        );
                    } else {
                        renderer.encode_instances(ready, &mut encoder, &color, &depth);
                    }
                }
                (ImportedBatch::Procedural { renderer, .. }, PreparedBatch::Procedural(ready)) => {
                    renderer.encode_composed(ready, &mut encoder, &color, &depth)
                }
                #[cfg(feature = "animation")]
                (ImportedBatch::Skinned { renderer, .. }, PreparedBatch::Skinned(ready)) => {
                    if shadow_ready.is_some() {
                        let mut commands = Vec::new();
                        renderer.shared_commands(ready, false, &mut commands);
                        self.rhi.encode_pass(
                            &mut encoder,
                            "shared skinned receiver",
                            Some(&color),
                            Some(&depth),
                            &commands,
                        );
                    } else {
                        renderer.encode_frame(ready, &mut encoder, &color, &depth);
                    }
                }
                _ => unreachable!("prepared batches preserve order and kind"),
            }
        }
        #[cfg(feature = "particles")]
        if let (Some(batch), Some(ready)) = (&mut particles, &particle_ready) {
            // Metadata is entirely determined by the completed preflight and
            // the scene-owned attachments. No further fallible frame admission.
            batch.renderer.upload_and_encode(&mut encoder, crate::particle_renderer::ParticleTarget {
                color: scene_view, depth, size: target.size, color_format: self.scene_format(),
                depth_format: TextureFormat::Depth32Float, sample_count: 1,
                depth_size: target.size, depth_sample_count: 1,
            }, ready).expect("preflighted scene particle target");
        }
        if let Some(post) = &self.post_processor {
            post.encode(
                &self.rhi,
                &mut encoder,
                target.view,
                self.format,
                self.post_process,
                lighting,
            );
        }
        self.rhi.submit(encoder);
        for (batch, ready) in batches.iter_mut().zip(prepared) {
            match (batch, ready) {
                (ImportedBatch::Procedural { renderer, .. }, PreparedBatch::Procedural(ready)) => {
                    renderer.commit_composed(*ready)
                }
                #[cfg(feature = "animation")]
                (ImportedBatch::Skinned { renderer, .. }, PreparedBatch::Skinned(ready)) => {
                    renderer.commit_frame(ready)
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn procedural_casters(list: &RenderList3D) -> usize {
    usize::from(!list.spheres.is_empty())
        + usize::from(!list.boxes.is_empty())
        + usize::from(!list.capsules.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_rhi::{
        Acquire, Binding, BufferDesc, Command, PipelineDesc, SamplerDesc, TextureUpload,
        TextureUploadError,
    };
    use std::{cell::RefCell, rc::Rc};

    #[derive(Clone, Debug, Default, PartialEq)]
    struct Effects {
        allocations: usize,
        allocation_kinds: Vec<&'static str>,
        buffers: Vec<(String, u64, String)>,
        upload_lengths: Vec<usize>,
        writes: usize,
        encoders: usize,
        submits: usize,
        idle_waits: usize,
        dropped_textures: Vec<(usize, TextureFormat)>,
        texture_lifecycle: Vec<&'static str>,
        textures: Vec<(TextureFormat, u32, (u32, u32))>,
        passes: Vec<(usize, bool, bool, bool, usize)>,
        shadow_passes: Vec<(usize, bool, bool, usize)>,
        pass_buffers: Vec<Vec<usize>>,
        written_buffers: Vec<usize>,
        pass_uniforms: Vec<Vec<usize>>,
        pass_textures: Vec<Vec<usize>>,
        post_passes: Vec<(String, usize, Vec<usize>)>,
    }
    /// Records ownership release separately from texture/view creation. The
    /// production backend may defer physical release until the recorded wait.
    struct RecordedTexture {
        id: usize,
        format: TextureFormat,
        effects: Rc<RefCell<Effects>>,
    }
    impl Drop for RecordedTexture {
        fn drop(&mut self) {
            self.effects
                .borrow_mut()
                .texture_lifecycle
                .push("drop_texture");
            self.effects
                .borrow_mut()
                .dropped_textures
                .push((self.id, self.format));
        }
    }
    /// Records every potentially mutating RHI call; no adapter is required.
    type BoundResources = (Vec<usize>, Vec<usize>);
    type BindRecords = Rc<RefCell<std::collections::BTreeMap<usize, BoundResources>>>;
    // Kept separate from Effects: existing material resource-trace equality
    // intentionally compares resource work, not material-dependent uniform bytes.
    type UploadRecords = Rc<RefCell<Vec<(usize, u64, Vec<u8>)>>>;
    #[derive(Clone, Default)]
    struct Mock(
        Rc<RefCell<Effects>>,
        Rc<RefCell<usize>>,
        BindRecords,
        Rc<RefCell<Option<orr_rhi::TextureFormatCapabilities>>>,
        UploadRecords,
    );
    impl Mock {
        fn allocate(&self, kind: &'static str) -> usize {
            let mut effects = self.0.borrow_mut();
            effects.allocations += 1;
            effects.allocation_kinds.push(kind);
            *self.1.borrow_mut() += 1;
            *self.1.borrow()
        }
        fn take(&self) -> Effects {
            std::mem::take(&mut *self.0.borrow_mut())
        }
    }
    impl Rhi for Mock {
        type Buffer = usize;
        type Texture = RecordedTexture;
        type TextureView = usize;
        type Shader = usize;
        type Pipeline = usize;
        type BindGroup = usize;
        type Sampler = usize;
        type Encoder = usize;
        type Surface = ();
        type Frame = usize;
        fn adapter_name(&self) -> String {
            "recording mock".into()
        }
        fn create_buffer(&self, desc: &BufferDesc<'_>) -> usize {
            self.0.borrow_mut().buffers.push((
                desc.label.to_owned(), desc.size, format!("{:?}", desc.usage),
            ));
            self.allocate("buffer")
        }
        fn write_buffer(&self, buffer: &usize, offset: u64, bytes: &[u8]) {
            self.0.borrow_mut().upload_lengths.push(bytes.len());
            self.0.borrow_mut().written_buffers.push(*buffer);
            self.4.borrow_mut().push((*buffer, offset, bytes.to_vec()));
            self.0.borrow_mut().writes += 1;
        }
        fn texture_view_formats_supported(&self) -> bool {
            true
        }
        fn copy_texture(&self, _: &RecordedTexture, _: &RecordedTexture) {
            panic!("imported renderer does not copy textures");
        }
        fn create_texture(&self, desc: &TextureDesc<'_>) -> RecordedTexture {
            self.0.borrow_mut().texture_lifecycle.push("create_texture");
            self.0.borrow_mut().textures.push((
                desc.format,
                desc.sample_count,
                (desc.width, desc.height),
            ));
            RecordedTexture {
                id: self.allocate("texture"),
                format: desc.format,
                effects: self.0.clone(),
            }
        }
        fn write_texture_rgba8(
            &self,
            _: &RecordedTexture,
            upload: &TextureUpload<'_>,
        ) -> Result<(), TextureUploadError> {
            self.0.borrow_mut().upload_lengths.push(upload.data.len());
            self.0.borrow_mut().writes += 1;
            Ok(())
        }
        fn create_texture_view(
            &self,
            texture: &RecordedTexture,
            _: Option<TextureFormat>,
        ) -> usize {
            self.allocate("texture_view");
            texture.id
        }
        fn create_sampler(&self, _: &SamplerDesc) -> usize {
            self.allocate("sampler")
        }
        fn sample_count_supported(&self, _: TextureFormat, samples: u32) -> bool {
            samples == 1
        }
        fn texture_format_capabilities(
            &self,
            _: TextureFormat,
        ) -> orr_rhi::TextureFormatCapabilities {
            self.3
                .borrow()
                .unwrap_or(orr_rhi::TextureFormatCapabilities {
                    usages: TextureUsage::RENDER_ATTACHMENT
                        | TextureUsage::TEXTURE_BINDING
                        | TextureUsage::COPY_SRC,
                    filterable: true,
                    blendable: true,
                    max_dimension_2d: 8192,
                    supports_hdr_postprocessing: true,
                    max_readback_buffer_size: 256 * 1024 * 1024,
                })
        }
        fn create_shader(&self, _: &str, _: &str) -> usize {
            self.allocate("shader")
        }
        fn create_pipeline(&self, _: &PipelineDesc<'_, Self>) -> usize {
            self.allocate("pipeline")
        }
        fn create_bind_group(&self, _: &usize, _: u32, entries: &[Binding<'_, Self>]) -> usize {
            let id = self.allocate("bind_group");
            self.2.borrow_mut().insert(
                id,
                (
                    entries
                        .iter()
                        .filter_map(|entry| match entry {
                            Binding::Uniform { buffer, .. } => Some(**buffer),
                            _ => None,
                        })
                        .collect(),
                    entries
                        .iter()
                        .filter_map(|entry| match entry {
                            Binding::Texture { view, .. } => Some(**view),
                            _ => None,
                        })
                        .collect(),
                ),
            );
            id
        }
        fn create_encoder(&self, _: &str) -> usize {
            self.0.borrow_mut().encoders += 1;
            0
        }
        fn encode_pass(
            &self,
            _: &mut usize,
            label: &str,
            color: Option<&ColorAttachment<'_, Self>>,
            depth: Option<&DepthAttachment<'_, Self>>,
            commands: &[Command<'_, Self>],
        ) {
            let draws = commands
                .iter()
                .filter(|command| matches!(command, Command::DrawIndexed { .. }))
                .count();
            let mut buffers = Vec::new();
            let mut textures = Vec::new();
            let mut uniforms = Vec::new();
            for command in commands {
                match command {
                    Command::SetVertexBuffer(_, b) | Command::SetIndexBuffer(b) => {
                        buffers.push(**b)
                    }
                    Command::SetBindGroup(_, b) => {
                        let binds = self.2.borrow();
                        let (bound_buffers, bound_textures) = binds.get(b).expect("recorded bind");
                        buffers.extend(bound_buffers.iter().copied());
                        uniforms.extend(bound_buffers.iter().copied());
                        textures.extend(bound_textures.iter().copied());
                    }
                    _ => {}
                }
            }
            let Some(depth) = depth else {
                let color = color.expect("postprocess has a color target");
                assert!(
                    !textures.contains(color.view),
                    "a pass must never sample its own output"
                );
                self.0
                    .borrow_mut()
                    .post_passes
                    .push((label.to_owned(), *color.view, textures));
                return;
            };
            self.0.borrow_mut().pass_buffers.push(buffers);
            self.0.borrow_mut().pass_uniforms.push(uniforms);
            self.0.borrow_mut().pass_textures.push(textures);
            let Some(color) = color else {
                self.0.borrow_mut().shadow_passes.push((
                    *depth.view,
                    depth.clear.is_some(),
                    depth.store,
                    draws,
                ));
                return;
            };
            self.0.borrow_mut().passes.push((
                *depth.view,
                color.clear.is_some(),
                depth.clear.is_some(),
                depth.store,
                draws,
            ));
        }
        fn submit(&self, _: usize) {
            self.0.borrow_mut().submits += 1;
        }
        fn wait_idle(&self) {
            self.0.borrow_mut().idle_waits += 1;
            self.0.borrow_mut().texture_lifecycle.push("wait_idle");
        }
        fn read_texture(&self, _: &RecordedTexture) -> Vec<u8> {
            unimplemented!()
        }
        fn surface_format(&self, _: &()) -> TextureFormat {
            unimplemented!()
        }
        fn resize_surface(&self, _: &mut (), _: u32, _: u32) {
            unimplemented!()
        }
        fn acquire_frame(&self, _: &mut ()) -> Acquire<usize> {
            unimplemented!()
        }
        fn read_frame(&self, _: &usize) -> Option<Vec<u8>> {
            unimplemented!()
        }
        fn frame_view<'a>(&self, frame: &'a usize) -> &'a usize {
            frame
        }
        fn present(&self, _: usize) {
            unimplemented!()
        }
    }
    #[cfg(feature = "metallic-roughness")]
    mod material_resource_tests {
        use super::*;
        include!("imported_scene/material_resource_tests.rs");
    }
    const FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;
    fn model(rhi: &Mock, format: TextureFormat) -> ModelRenderer<Mock> {
        let model = orr_model::import::import_with_resolver(
            "fixtures/model.glb",
            include_bytes!("../../orr_model/tests/fixtures/model.glb"),
            |_| panic!("embedded fixture"),
        )
        .unwrap();
        ModelRenderer::new(rhi.clone(), format, model).unwrap()
    }
    fn draw(
        scene: &mut ImportedSceneRenderer<Mock>,
        size: (u32, u32),
        batches: &mut [ImportedBatch<'_, Mock>],
    ) -> Result<(), ImportedSceneError> {
        scene.draw(
            ImportedSceneTarget {
                view: &0,
                size,
                format: FORMAT,
                sample_count: 1,
            },
            &Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 2.0),
            &Lighting {
                shadows: false,
                ..Lighting::default()
            },
            &PointLightSettings::default(),
            batches,
        )
    }
    fn assert_passes(effects: &Effects, count: usize) {
        assert_eq!(effects.encoders, 1);
        assert_eq!(effects.submits, 1);
        assert_eq!(effects.passes.len(), count);
        let shared_depth = effects.passes[0].0;
        for (index, &(depth, color_clear, depth_clear, store, _)) in
            effects.passes.iter().enumerate()
        {
            assert_eq!(depth, shared_depth);
            assert_eq!(color_clear, index == 0);
            assert_eq!(depth_clear, index == 0);
            assert!(store);
        }
    }

    #[cfg(feature = "irradiance-probes")]
    #[test]
    fn probe_toggle_reuses_all_gpu_state_and_bad_grid_resize_is_atomic() {
        let rhi = Mock::default();
        let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
        scene.post_process.enabled = true;
        let mut model = model(&rhi, TextureFormat::Rgba16Float);
        let mut procedural = Renderer3D::with_settings(
            rhi.clone(),
            TextureFormat::Rgba16Float,
            crate::Settings3D::LOW,
        );
        let mut list = RenderList3D::new();
        list.cuboid(
            [0.0; 3],
            crate::IDENTITY_ROT,
            [0.5; 3],
            &crate::Material::new([0.3; 3]),
        );
        let instances = [StaticInstance::default()];
        let frame = |scene: &mut ImportedSceneRenderer<Mock>,
                     model: &mut ModelRenderer<Mock>,
                     procedural: &mut Renderer3D<Mock>,
                     size,
                     instances: &[StaticInstance]| {
            draw(
                scene,
                size,
                &mut [
                    ImportedBatch::Procedural {
                        renderer: procedural,
                        list: &list,
                    },
                    ImportedBatch::StaticInstances {
                        renderer: model,
                        instances,
                    },
                ],
            )
        };
        frame(
            &mut scene,
            &mut model,
            &mut procedural,
            (32, 32),
            &instances,
        )
        .unwrap();
        rhi.take();
        for cycle in 0..40 {
            scene.irradiance = (cycle % 3 != 0).then(|| crate::IrradianceGrid {
                enabled: cycle % 3 == 1,
                ..Default::default()
            });
            frame(
                &mut scene,
                &mut model,
                &mut procedural,
                (32, 32),
                &instances,
            )
            .unwrap();
            let effects = rhi.take();
            assert_eq!(
                effects.allocations, 0,
                "probe toggles cannot cache new pipelines/binds/buffers"
            );
            assert!(effects.textures.is_empty());
            assert_eq!(effects.submits, 1);
            assert_eq!(effects.passes.len(), 2);
        }
        let previous_stats = procedural.last_frame_stats();
        let previous_depth = (scene.depth_size(), scene.depth_generation());
        let previous_hdr = (
            scene.post_process_size(),
            scene.post_process_generation(),
            scene.post_process_allocated_bytes(),
        );
        let mut bad = crate::IrradianceGrid::default();
        bad.spacing[1] = f32::NAN;
        scene.irradiance = Some(bad);
        assert!(matches!(
            frame(
                &mut scene,
                &mut model,
                &mut procedural,
                (64, 64),
                &[StaticInstance::default(); 4]
            ),
            Err(ImportedSceneError::Irradiance(_))
        ));
        assert_eq!(rhi.take(), Effects::default());
        assert_eq!(previous_stats, procedural.last_frame_stats());
        assert_eq!(
            previous_depth,
            (scene.depth_size(), scene.depth_generation())
        );
        assert_eq!(
            previous_hdr,
            (
                scene.post_process_size(),
                scene.post_process_generation(),
                scene.post_process_allocated_bytes()
            )
        );
        assert!(matches!(
            scene.preflight(
                (64, 64),
                &Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 2.0),
                &Lighting {
                    shadows: false,
                    ..Default::default()
                },
                &PointLightSettings::default(),
                &list,
                &[(model.model(), &instances)],
            ),
            Err(ImportedSceneError::Irradiance(_))
        ));
        assert_eq!(rhi.take(), Effects::default());
        // Re-admit the old shape after rejection: the rejected 4-instance request
        // did not grow any slot cache, nor did it replace the old target.
        scene.irradiance = None;
        frame(
            &mut scene,
            &mut model,
            &mut procedural,
            (32, 32),
            &instances,
        )
        .unwrap();
        assert_eq!(rhi.take().allocations, 0);
    }

    #[cfg(feature = "irradiance-probes")]
    #[test]
    fn probe_globals_remain_below_portable_uniform_limit() {
        assert_eq!(std::mem::size_of::<crate::IrradianceUniform>(), 9280);
        let material_globals = if cfg!(feature = "metallic-roughness") { 32 } else { 0 };
        assert_eq!(std::mem::size_of::<crate::model_renderer::Globals>(), 9536 + material_globals);
        assert!(std::mem::size_of::<crate::model_renderer::Globals>() <= 16 * 1024);
    }

    #[test]
    fn hdr_targets_passes_reuse_resize_and_off_are_recorded() {
        let rhi = Mock::default();
        let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
        assert_eq!(scene.scene_format(), FORMAT);
        draw(&mut scene, (3, 5), &mut []).unwrap();
        let off = rhi.take();
        assert!(off.post_passes.is_empty());
        assert_eq!(off.textures, vec![(TextureFormat::Depth32Float, 1, (3, 5))]);
        assert_eq!(scene.post_process_allocated_bytes(), 0);
        scene.post_process.enabled = true;
        assert_eq!(scene.scene_format(), TextureFormat::Rgba16Float);
        draw(&mut scene, (3, 5), &mut []).unwrap();
        let first = rhi.take();
        assert_eq!(
            first.textures,
            vec![
                (TextureFormat::Rgba16Float, 1, (3, 5)),
                (TextureFormat::Rgba16Float, 1, (2, 3)),
                (TextureFormat::Rgba16Float, 1, (2, 3))
            ]
        );
        assert_eq!(first.post_passes.len(), 6);
        assert_eq!(first.submits, 1);
        assert_eq!(first.idle_waits, 0);
        assert_eq!(scene.post_process_generation(), 1);
        assert_eq!(scene.post_process_allocated_bytes(), 216);
        draw(&mut scene, (3, 5), &mut []).unwrap();
        let reused = rhi.take();
        assert_eq!(reused.allocations, 0);
        assert_eq!(reused.idle_waits, 0);
        assert!(reused.dropped_textures.is_empty());
        assert_eq!(reused.post_passes, first.post_passes);
        scene.post_process.bloom = false;
        draw(&mut scene, (3, 5), &mut []).unwrap();
        let no_bloom = rhi.take();
        assert_eq!(no_bloom.allocations, 0);
        assert_eq!(no_bloom.idle_waits, 0);
        assert!(no_bloom.dropped_textures.is_empty());
        assert_eq!(no_bloom.post_passes.len(), 1);
        assert_eq!(no_bloom.post_passes[0].0, "HDR final display");
        scene.post_process.bloom = true;
        scene.post_process.strength = 0.0;
        draw(&mut scene, (3, 5), &mut []).unwrap();
        assert_eq!(rhi.take().post_passes.len(), 1);
        draw(&mut scene, (7, 9), &mut []).unwrap();
        let resized = rhi.take();
        assert_eq!(resized.idle_waits, 1);
        assert_eq!(
            resized.texture_lifecycle.first(),
            Some(&"wait_idle"),
            "retire in-flight work before allocating or releasing a generation"
        );
        assert_eq!(
            resized
                .dropped_textures
                .iter()
                .filter(|(_, format)| *format == TextureFormat::Rgba16Float)
                .count(),
            3
        );
        assert_eq!(
            resized
                .dropped_textures
                .iter()
                .filter(|(_, format)| *format == TextureFormat::Depth32Float)
                .count(),
            1
        );
        assert_eq!(
            resized.allocations, 12,
            "reuse pipelines/uniforms/sampler; replace 3 colors, 4 bindings and shared depth"
        );
        assert_eq!(scene.post_process_generation(), 2);
        assert_eq!(scene.post_process_size(), Some((7, 9)));
        assert_eq!(scene.post_process_allocated_bytes(), 824);
        scene.post_process.enabled = false;
        draw(&mut scene, (7, 9), &mut []).unwrap();
        let off = rhi.take();
        assert_eq!(off.allocations, 0);
        assert_eq!(off.idle_waits, 1);
        assert_eq!(off.dropped_textures.len(), 3);
        assert_eq!(
            off.texture_lifecycle,
            vec!["wait_idle", "drop_texture", "drop_texture", "drop_texture"]
        );
        assert!(
            off.dropped_textures
                .iter()
                .all(|(_, format)| *format == TextureFormat::Rgba16Float)
        );
        assert!(off.post_passes.is_empty());
        assert!(scene.hdr_scene_texture().is_none());
        assert_eq!(scene.post_process_allocated_bytes(), 0);
        assert_eq!(scene.post_process_generation(), 2);
        scene.post_process.enabled = true;
        draw(&mut scene, (7, 9), &mut []).unwrap();
        assert_eq!(scene.post_process_generation(), 3);
    }

    #[test]
    fn hdr_final_target_must_be_display_format() {
        let rhi = Mock::default();
        let mut scene =
            ImportedSceneRenderer::new(rhi.clone(), TextureFormat::Rgba16Float).unwrap();
        // Direct scene-linear rendering into a float target remains deliberate.
        assert!(
            scene
                .validate_target((32, 32), TextureFormat::Rgba16Float, 1)
                .is_ok()
        );
        scene.post_process.enabled = true;
        assert!(matches!(
            scene.validate_target((32, 32), TextureFormat::Rgba16Float, 1),
            Err(ImportedSceneError::InvalidTarget)
        ));
        assert_eq!(rhi.take(), Effects::default());
        assert_eq!(scene.post_process_generation(), 0);
    }

    #[test]
    fn hdr_every_required_capability_is_checked_before_effects() {
        let rhi = Mock::default();
        let base = rhi.texture_format_capabilities(TextureFormat::Rgba16Float);
        let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
        scene.post_process.enabled = true;
        let mut failures = Vec::new();
        let mut caps = base;
        caps.usages = TextureUsage::RENDER_ATTACHMENT;
        failures.push(caps);
        let mut caps = base;
        caps.filterable = false;
        failures.push(caps);
        let mut caps = base;
        caps.blendable = false;
        failures.push(caps);
        let mut caps = base;
        caps.max_dimension_2d = 32;
        failures.push(caps);
        let mut caps = base;
        caps.supports_hdr_postprocessing = false;
        failures.push(caps);
        let mut caps = base;
        caps.max_readback_buffer_size = 511;
        failures.push(caps);
        for caps in failures {
            *rhi.3.borrow_mut() = Some(caps);
            assert!(matches!(
                draw(&mut scene, (33, 1), &mut []),
                Err(ImportedSceneError::UnsupportedHdr)
            ));
            assert_eq!(rhi.take(), Effects::default());
            assert_eq!(scene.post_process_generation(), 0);
            assert_eq!(scene.depth_generation(), 0);
        }
        // Exact row padding matters: 33 FP16 texels require a 512-byte row.
        let mut caps = base;
        caps.max_readback_buffer_size = 512;
        *rhi.3.borrow_mut() = Some(caps);
        draw(&mut scene, (33, 1), &mut []).unwrap();
        assert_eq!(scene.post_process_size(), Some((33, 1)));
    }

    #[test]
    fn hdr_capability_settings_peak_budget_and_late_batches_fail_closed() {
        let rhi = Mock::default();
        let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
        scene.post_process.enabled = true;
        *rhi.3.borrow_mut() = Some(orr_rhi::TextureFormatCapabilities::default());
        assert!(matches!(
            draw(&mut scene, (32, 32), &mut []),
            Err(ImportedSceneError::UnsupportedHdr)
        ));
        assert_eq!(rhi.take(), Effects::default());
        assert_eq!(scene.post_process_generation(), 0);
        *rhi.3.borrow_mut() = None;
        draw(&mut scene, (3000, 3000), &mut []).unwrap();
        rhi.take();
        let generation = scene.post_process_generation();
        let bytes = scene.post_process_allocated_bytes();
        // Each fits alone, but both during replacement exceed 128 MiB.
        assert!(matches!(
            draw(&mut scene, (3100, 3100), &mut []),
            Err(ImportedSceneError::PostProcessBudget)
        ));
        assert_eq!(rhi.take(), Effects::default());
        assert_eq!(scene.post_process_generation(), generation);
        assert_eq!(scene.post_process_allocated_bytes(), bytes);
        assert_eq!(scene.depth_size(), Some((3000, 3000)));
        assert!(matches!(
            draw(&mut scene, (4000, 4000), &mut []),
            Err(ImportedSceneError::PostProcessBudget)
        ));
        assert_eq!(rhi.take(), Effects::default());
        scene.post_process.threshold = f32::NAN;
        assert!(matches!(
            draw(&mut scene, (32, 32), &mut []),
            Err(ImportedSceneError::InvalidPostProcess)
        ));
        assert_eq!(rhi.take(), Effects::default());
        scene.post_process.threshold = 1.0;
        scene.clear[0] = 65505.0;
        assert!(matches!(
            draw(&mut scene, (32, 32), &mut []),
            Err(ImportedSceneError::InvalidTarget)
        ));
        assert_eq!(rhi.take(), Effects::default());
        scene.clear = crate::DEFAULT_CLEAR_3D;
        let mut renderer = model(&rhi, TextureFormat::Rgba16Float);
        let mut procedural = Renderer3D::with_settings(
            rhi.clone(),
            TextureFormat::Rgba16Float,
            crate::Settings3D::LOW,
        );
        let list = RenderList3D::default();
        let mut invalid = StaticInstance::default();
        invalid.translation[0] = f32::NAN;
        rhi.take();
        assert!(
            draw(
                &mut scene,
                (32, 32),
                &mut [
                    ImportedBatch::Procedural {
                        renderer: &mut procedural,
                        list: &list
                    },
                    ImportedBatch::StaticInstances {
                        renderer: &mut renderer,
                        instances: &[invalid]
                    }
                ]
            )
            .is_err()
        );
        assert_eq!(rhi.take(), Effects::default());
        assert_eq!(scene.post_process_generation(), generation);
        // Off still validates settings, and rejecting it retains every HDR target.
        scene.post_process.enabled = false;
        scene.post_process.radius = 0;
        assert!(matches!(
            draw(&mut scene, (32, 32), &mut []),
            Err(ImportedSceneError::InvalidPostProcess)
        ));
        assert_eq!(rhi.take(), Effects::default());
        assert_eq!(scene.post_process_allocated_bytes(), bytes);
    }

    #[test]
    fn mock_preflight_before_cache_or_resize_matches_frame_validation_without_gpu_effects() {
        let rhi = Mock::default();
        let model = model(&rhi, FORMAT);
        let scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
        let mut list = RenderList3D::new();
        list.sphere(
            [0.0; 3],
            crate::IDENTITY_ROT,
            0.5,
            &crate::Material::new([0.5; 3]),
        );
        let camera = Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 2.0);
        let lighting = Lighting {
            shadows: false,
            ..Default::default()
        };
        let point = PointLightSettings::default();
        let instances = [StaticInstance::default()];
        rhi.take();
        scene
            .preflight(
                (32, 32),
                &camera,
                &lighting,
                &point,
                &list,
                &[(model.model(), &instances)],
            )
            .unwrap();
        instances[0].validate_for(model.model()).unwrap();
        assert_eq!(rhi.take(), Effects::default());
        let invalid = [StaticInstance {
            scale: [0.0; 3],
            ..Default::default()
        }];
        assert!(matches!(
            scene.preflight(
                (64, 64),
                &camera,
                &lighting,
                &point,
                &list,
                &[(model.model(), &invalid)]
            ),
            Err(ImportedSceneError::StaticInstance(
                StaticInstanceError::InvalidPlacement
            ))
        ));
        list.spheres[0].pos[0] = f32::NAN;
        assert!(matches!(
            scene.preflight(
                (64, 64),
                &camera,
                &lighting,
                &point,
                &list,
                &[(model.model(), &instances)]
            ),
            Err(ImportedSceneError::Procedural(
                ProceduralSceneError::InvalidInstance
            ))
        ));
        assert_eq!(rhi.take(), Effects::default());
        assert_eq!(scene.depth_generation(), 0);
    }

    #[test]
    fn mock_procedural_and_static_instances_share_depth_and_late_failures_have_no_effects() {
        use crate::{IDENTITY_ROT, Material, Settings3D, StaticInstance};
        let rhi = Mock::default();
        let mut model = model(&rhi, FORMAT);
        let mut procedural = Renderer3D::with_settings(rhi.clone(), FORMAT, Settings3D::LOW);
        let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
        let mut list = RenderList3D::new();
        list.cuboid([0.0; 3], IDENTITY_ROT, [0.5; 3], &Material::new([0.3; 3]));
        let instances = [StaticInstance::default(); 2];
        rhi.take();
        draw(
            &mut scene,
            (32, 32),
            &mut [
                ImportedBatch::Procedural {
                    renderer: &mut procedural,
                    list: &list,
                },
                ImportedBatch::StaticInstances {
                    renderer: &mut model,
                    instances: &instances,
                },
            ],
        )
        .unwrap();
        let effects = rhi.take();
        assert_passes(&effects, 2);
        assert_eq!(
            effects.textures,
            vec![(TextureFormat::Depth32Float, 1, (32, 32))]
        );
        assert_eq!(procedural.last_frame_stats().main.draw_calls, 1);
        let stats = procedural.last_frame_stats();
        let bad = [StaticInstance {
            scale: [0.0; 3],
            ..Default::default()
        }];
        assert!(matches!(
            draw(
                &mut scene,
                (64, 64),
                &mut [
                    ImportedBatch::Procedural {
                        renderer: &mut procedural,
                        list: &list
                    },
                    ImportedBatch::StaticInstances {
                        renderer: &mut model,
                        instances: &bad
                    },
                ]
            ),
            Err(ImportedSceneError::StaticInstance(
                StaticInstanceError::InvalidPlacement
            ))
        ));
        assert_eq!(rhi.take(), Effects::default());
        assert_eq!(scene.depth_size(), Some((32, 32)));
        assert_eq!(scene.depth_generation(), 1);
        assert_eq!(procedural.last_frame_stats(), stats);
        // Earlier static slots must not grow/upload when a later procedural item is bad.
        let more_instances = [StaticInstance::default(); 3];
        list.boxes[0].rot = [0.0; 4];
        assert!(matches!(
            draw(
                &mut scene,
                (64, 64),
                &mut [
                    ImportedBatch::StaticInstances {
                        renderer: &mut model,
                        instances: &more_instances
                    },
                    ImportedBatch::Procedural {
                        renderer: &mut procedural,
                        list: &list
                    },
                ]
            ),
            Err(ImportedSceneError::Procedural(
                ProceduralSceneError::InvalidInstance
            ))
        ));
        assert_eq!(rhi.take(), Effects::default());
        assert_eq!(procedural.last_frame_stats(), stats);
    }

    #[test]
    fn mock_static_instances_validate_composed_nodes_and_capacity_before_gpu_effects() {
        let rhi = Mock::default();
        let mut renderer = model(&rhi, FORMAT);
        let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
        let too_many =
            vec![StaticInstance::default(); crate::model_renderer::MAX_STATIC_INSTANCES + 1];
        rhi.take();
        assert!(matches!(
            draw(
                &mut scene,
                (32, 32),
                &mut [ImportedBatch::StaticInstances {
                    renderer: &mut renderer,
                    instances: &too_many
                },]
            ),
            Err(ImportedSceneError::StaticInstance(
                StaticInstanceError::InstanceLimit
            ))
        ));
        assert_eq!(rhi.take(), Effects::default());
        let mut source = renderer.model().source().clone();
        source.primitives[0].transform[3][0] = 1000.0;
        let mut renderer = ModelRenderer::new(
            rhi.clone(),
            FORMAT,
            orr_model::StaticModel::new(source).unwrap(),
        )
        .unwrap();
        // External TRS is valid alone, but external * node exceeds matrix range.
        let instances = [StaticInstance {
            scale: [1e5; 3],
            ..Default::default()
        }];
        rhi.take();
        assert!(matches!(
            draw(
                &mut scene,
                (32, 32),
                &mut [ImportedBatch::StaticInstances {
                    renderer: &mut renderer,
                    instances: &instances
                },]
            ),
            Err(ImportedSceneError::StaticInstance(
                StaticInstanceError::InvalidPlacement
            ))
        ));
        assert_eq!(rhi.take(), Effects::default());
        assert_eq!(scene.depth_generation(), 0);
    }

    #[test]
    fn mock_static_shared_depth_clear_submit_reuse_and_empty_frame() {
        let rhi = Mock::default();
        let mut a = model(&rhi, FORMAT);
        let mut b = model(&rhi, FORMAT);
        rhi.take();
        let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
        assert_eq!(
            rhi.take(),
            Effects::default(),
            "coordinator construction allocates nothing"
        );
        draw(
            &mut scene,
            (32, 32),
            &mut [ImportedBatch::Static(&mut a), ImportedBatch::Static(&mut b)],
        )
        .unwrap();
        let effects = rhi.take();
        assert_eq!(
            effects.textures,
            vec![(TextureFormat::Depth32Float, 1, (32, 32))]
        );
        assert_eq!(
            effects.allocations, 2,
            "only one shared texture and its view"
        );
        assert_eq!(effects.writes, 2, "one globals write per renderer");
        assert_passes(&effects, 2);
        assert!(effects.passes.iter().all(|pass| pass.4 > 0));
        draw(
            &mut scene,
            (32, 32),
            &mut [ImportedBatch::Static(&mut b), ImportedBatch::Static(&mut a)],
        )
        .unwrap();
        let effects = rhi.take();
        assert_eq!(effects.allocations, 0);
        assert_passes(&effects, 2);
        assert_eq!(scene.depth_generation(), 1);
        draw(&mut scene, (32, 32), &mut []).unwrap();
        let effects = rhi.take();
        assert_eq!(effects.allocations, 0);
        assert_eq!(effects.writes, 0);
        assert_passes(&effects, 1);
        assert_eq!(effects.passes[0].4, 0);
        draw(&mut scene, (64, 64), &mut []).unwrap();
        let effects = rhi.take();
        assert_eq!(
            effects.textures,
            vec![(TextureFormat::Depth32Float, 1, (64, 64))]
        );
        assert_passes(&effects, 1);
        assert_eq!(scene.depth_size(), Some((64, 64)));
        assert_eq!(scene.depth_generation(), 2);
    }

    #[test]
    fn mock_late_format_rejection_has_zero_gpu_effects_even_on_resize() {
        let rhi = Mock::default();
        let mut good = model(&rhi, FORMAT);
        let mut bad = model(&rhi, TextureFormat::Bgra8Unorm);
        let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
        rhi.take();
        assert!(matches!(
            draw(
                &mut scene,
                (32, 32),
                &mut [
                    ImportedBatch::Static(&mut good),
                    ImportedBatch::Static(&mut bad)
                ]
            ),
            Err(ImportedSceneError::FormatMismatch)
        ));
        assert_eq!(rhi.take(), Effects::default());
        assert_eq!(scene.depth_size(), None);
        assert_eq!(scene.depth_generation(), 0);
        draw(
            &mut scene,
            (32, 32),
            &mut [ImportedBatch::Static(&mut good)],
        )
        .unwrap();
        rhi.take();
        assert!(matches!(
            draw(
                &mut scene,
                (64, 64),
                &mut [
                    ImportedBatch::Static(&mut good),
                    ImportedBatch::Static(&mut bad)
                ]
            ),
            Err(ImportedSceneError::FormatMismatch)
        ));
        assert_eq!(rhi.take(), Effects::default());
        assert_eq!(scene.depth_size(), Some((32, 32)));
        assert_eq!(scene.depth_generation(), 1);
    }

    #[cfg(feature = "animation")]
    #[test]
    fn mock_late_pose_rejection_keeps_uniform_cache_depth_and_submitted_bounds() {
        let rhi = Mock::default();
        let animated = || {
            orr_model::animation_import::import_with_resolver(
                "fixtures/animated_strip.glb",
                include_bytes!("../../orr_model/tests/fixtures/animated_strip.glb"),
                |_| panic!("embedded fixture"),
            )
            .unwrap()
        };
        let model = animated();
        let pose = model.rest_pose().unwrap();
        let foreign_pose = animated().rest_pose().unwrap();
        let mut a = SkinnedModelRenderer::new(rhi.clone(), FORMAT, model.clone()).unwrap();
        let mut b = SkinnedModelRenderer::new(rhi.clone(), FORMAT, model).unwrap();
        let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
        let one = [SkinnedInstance::new(&pose)];
        rhi.take();
        draw(
            &mut scene,
            (32, 32),
            &mut [
                ImportedBatch::Skinned {
                    renderer: &mut a,
                    instances: &one,
                },
                ImportedBatch::Skinned {
                    renderer: &mut b,
                    instances: &one,
                },
            ],
        )
        .unwrap();
        let effects = rhi.take();
        assert_eq!(
            effects.textures,
            vec![(TextureFormat::Depth32Float, 1, (32, 32))]
        );
        assert_passes(&effects, 2);
        let (a_bounds, b_bounds) = (a.bounds().to_vec(), b.bounds().to_vec());
        let mut placement = orr_model::IDENTITY;
        placement[3][0] = 2.0;
        // A would need new cache entries and new bounds, but B's later foreign
        // pose must reject everything before any of those effects can happen.
        let two = [SkinnedInstance {
            pose: &pose,
            transform: placement,
        }; 2];
        let invalid = [SkinnedInstance::new(&foreign_pose)];
        assert!(matches!(
            draw(
                &mut scene,
                (64, 64),
                &mut [
                    ImportedBatch::Skinned {
                        renderer: &mut a,
                        instances: &two
                    },
                    ImportedBatch::Skinned {
                        renderer: &mut b,
                        instances: &invalid
                    },
                ]
            ),
            Err(ImportedSceneError::Skinned(
                SkinnedRenderError::InvalidPose(_)
            ))
        ));
        assert_eq!(rhi.take(), Effects::default());
        assert_eq!(a.bounds(), a_bounds);
        assert_eq!(b.bounds(), b_bounds);
        assert_eq!(scene.depth_size(), Some((32, 32)));
        assert_eq!(scene.depth_generation(), 1);
    }

    #[cfg(feature = "animation")]
    #[test]
    fn mock_mixed_preflight_validates_skinned_pose_before_any_gpu_effect() {
        let rhi = Mock::default();
        let animated = || {
            orr_model::animation_import::import_with_resolver(
                "fixtures/animated_strip.glb",
                include_bytes!("../../orr_model/tests/fixtures/animated_strip.glb"),
                |_| panic!("embedded fixture"),
            )
            .unwrap()
        };
        let model = animated();
        let pose = model.rest_pose().unwrap();
        let foreign_pose = animated().rest_pose().unwrap();
        let instances = [SkinnedInstance::new(&pose)];
        let foreign = [SkinnedInstance::new(&foreign_pose)];
        let scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
        let camera = Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 2.0);
        let lighting = crate::Lighting {
            shadows: false,
            ..Default::default()
        };
        let point = PointLightSettings::default();
        let procedural = RenderList3D::default();

        scene
            .preflight_mixed(
                (32, 32),
                &camera,
                &lighting,
                &point,
                &procedural,
                &[],
                &[(&model, &instances)],
            )
            .unwrap();
        assert_eq!(rhi.take(), Effects::default());

        assert!(matches!(
            scene.preflight_mixed(
                (64, 64),
                &camera,
                &lighting,
                &point,
                &procedural,
                &[],
                &[(&model, &foreign)],
            ),
            Err(ImportedSceneError::Skinned(
                SkinnedRenderError::InvalidPose(_)
            ))
        ));
        assert_eq!(rhi.take(), Effects::default());
        assert_eq!(scene.depth_size(), None);
        assert_eq!(scene.depth_generation(), 0);
    }
    #[test]
    fn mock_shared_shadow_preflight_retains_main_draw_limit_precedence() {
        let rhi = Mock::default();
        let scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
        let camera = Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 2.0);
        for shadows in [false, true] {
            let lighting = Lighting {
                shadows,
                ..Default::default()
            };
            assert!(matches!(
                scene.preflight_cpu(
                    (32, 32),
                    &camera,
                    &lighting,
                    &PointLightSettings::default(),
                    &RenderList3D::new(),
                    &[],
                    1,
                    MAX_IMPORTED_DRAWS + 1
                ),
                Err(ImportedSceneError::DrawLimit)
            ));
            assert_eq!(rhi.take(), Effects::default());
        }
    }

    #[test]
    fn mock_shared_shadow_resources_upload_once_and_reject_before_any_mutation() {
        let rhi = Mock::default();
        let mut renderer = model(&rhi, FORMAT);
        let mut procedural = Renderer3D::with_settings(rhi.clone(), FORMAT, crate::Settings3D::LOW);
        let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
        let camera = Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 2.0);
        let mut light = Lighting {
            shadows: true,
            shadow_radius: 4.0,
            ..Default::default()
        };
        let mut list = RenderList3D::new();
        list.cuboid(
            [0.0; 3],
            crate::IDENTITY_ROT,
            [0.5; 3],
            &crate::Material::new([0.5; 3]),
        );
        let instances = [StaticInstance::default(); 2];
        let run = |scene: &mut ImportedSceneRenderer<Mock>,
                   light: &Lighting,
                   size,
                   renderer: &mut ModelRenderer<Mock>,
                   procedural: &mut Renderer3D<Mock>| {
            scene.draw(
                ImportedSceneTarget {
                    view: &0,
                    size,
                    format: FORMAT,
                    sample_count: 1,
                },
                &camera,
                light,
                &PointLightSettings::default(),
                &mut [
                    ImportedBatch::StaticInstances {
                        renderer,
                        instances: &instances,
                    },
                    ImportedBatch::Procedural {
                        renderer: procedural,
                        list: &list,
                    },
                ],
            )
        };
        rhi.take();
        run(&mut scene, &light, (32, 32), &mut renderer, &mut procedural).unwrap();
        let first = rhi.take();
        assert_eq!(first.submits, 1);
        assert_eq!(first.encoders, 1);
        let map = scene.shadow.as_ref().unwrap().view;
        assert!(first.pass_textures[0].is_empty());
        assert_eq!(
            first.pass_textures[2],
            vec![map],
            "procedural main samples coordinator map only"
        );
        assert!(first.pass_textures[1].contains(&map));
        assert_eq!(first.shadow_passes.len(), 1);
        assert!(first.shadow_passes[0].1);
        assert!(first.shadow_passes[0].2);
        assert_eq!(first.shadow_passes[0].3, renderer.draw_count() * 2 + 1);
        assert_eq!(first.textures.len(), 2);
        assert!(
            first
                .textures
                .contains(&(TextureFormat::Depth32Float, 1, (1024, 1024)))
        );
        // Main and depth use exactly the same uploaded object/instance/geometry buffers.
        let mut shadow_buffers = first.pass_buffers[0].clone();
        shadow_buffers.sort_unstable();
        shadow_buffers.dedup();
        let mut main_buffers = first.pass_buffers[1..]
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        main_buffers.sort_unstable();
        main_buffers.dedup();
        assert_eq!(shadow_buffers, main_buffers);
        let mut written = first.written_buffers.clone();
        written.sort_unstable();
        written.dedup();
        assert_eq!(
            written.len(),
            first.written_buffers.len(),
            "no main/shadow duplicate writes"
        );
        for _ in 0..3 {
            run(&mut scene, &light, (32, 32), &mut renderer, &mut procedural).unwrap();
            let effects = rhi.take();
            assert_eq!(effects.allocations, 0);
            assert_eq!(effects.shadow_passes.len(), 1);
            assert_eq!(effects.submits, 1);
        }
        let bad = [StaticInstance {
            scale: [0.0; 3],
            ..Default::default()
        }];
        let previous_stats = procedural.last_frame_stats();
        assert!(
            scene
                .draw(
                    ImportedSceneTarget {
                        view: &0,
                        size: (64, 64),
                        format: FORMAT,
                        sample_count: 1
                    },
                    &camera,
                    &light,
                    &PointLightSettings::default(),
                    &mut [
                        ImportedBatch::Procedural {
                            renderer: &mut procedural,
                            list: &list
                        },
                        ImportedBatch::StaticInstances {
                            renderer: &mut renderer,
                            instances: &bad
                        }
                    ]
                )
                .is_err()
        );
        assert_eq!(rhi.take(), Effects::default());
        assert_eq!(procedural.last_frame_stats(), previous_stats);
        let budget_instances = vec![StaticInstance::default(); 256];
        let oversized = vec![(renderer.model(), budget_instances.as_slice()); 9];
        assert!(
            scene
                .preflight(
                    (64, 64),
                    &camera,
                    &light,
                    &PointLightSettings::default(),
                    &list,
                    &oversized
                )
                .is_err()
        );
        assert_eq!(rhi.take(), Effects::default());
        for bad in [f32::NAN, f32::INFINITY, 0.0, 1e6] {
            light.shadow_radius = bad;
            assert!(run(&mut scene, &light, (64, 64), &mut renderer, &mut procedural).is_err());
            assert_eq!(rhi.take(), Effects::default());
            assert_eq!(scene.depth_size(), Some((32, 32)));
            assert_eq!(scene.shadow_map_generation(), 1);
        }
        light.shadow_radius = 4.0;
        light.shadow_center[0] = f32::NAN;
        assert!(run(&mut scene, &light, (64, 64), &mut renderer, &mut procedural).is_err());
        assert_eq!(rhi.take(), Effects::default());
        light.shadow_center = [0.0; 3];
        run(&mut scene, &light, (64, 64), &mut renderer, &mut procedural).unwrap();
        let resized = rhi.take();
        assert_eq!(
            resized.textures,
            vec![(TextureFormat::Depth32Float, 1, (64, 64))]
        );
        assert_eq!(scene.shadow_map_generation(), 1);
        light.shadows = false;
        run(&mut scene, &light, (64, 64), &mut renderer, &mut procedural).unwrap();
        assert!(rhi.take().shadow_passes.is_empty());
        light.shadows = true;
        scene
            .draw(
                ImportedSceneTarget {
                    view: &0,
                    size: (64, 64),
                    format: FORMAT,
                    sample_count: 1,
                },
                &camera,
                &light,
                &PointLightSettings::default(),
                &mut [],
            )
            .unwrap();
        let empty = rhi.take();
        assert_eq!(empty.shadow_passes.len(), 1);
        assert_eq!(empty.shadow_passes[0].3, 0);
        assert!(empty.shadow_passes[0].1);
        assert_eq!(empty.submits, 1);
        let mut other = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
        run(&mut other, &light, (64, 64), &mut renderer, &mut procedural).unwrap();
        let other_map = other.shadow.as_ref().unwrap().view;
        assert_ne!(map, other_map);
        let changed = rhi.take();
        assert_eq!(changed.pass_textures[2], vec![other_map]);
        run(&mut other, &light, (64, 64), &mut renderer, &mut procedural).unwrap();
        assert_eq!(
            rhi.take().allocations,
            0,
            "one cached receiver binding per renderer"
        );
        run(&mut scene, &light, (64, 64), &mut renderer, &mut procedural).unwrap();
        let rebound = rhi.take();
        assert!(rebound.textures.is_empty());
        assert_eq!(
            rebound.allocations, 2,
            "replace just the two receiver binding generations"
        );
        assert_eq!(rebound.pass_textures[2], vec![map]);
    }
    #[cfg(feature = "particles")]
    mod particle_composition {
        use super::*;
        include!("imported_scene/particle_composition_tests.rs");
    }

    mod prepared_camera_tests {
        use super::*;

        fn camera() -> Camera3D {
            Camera3D::perspective([3.0, 2.0, 7.0], [0.5, -0.25, 0.0], 53.0)
        }

        fn light() -> Lighting {
            Lighting { shadows: false, ..Lighting::default() }
        }

        fn target(size: (u32, u32)) -> ImportedSceneTarget<'static, Mock> {
            ImportedSceneTarget { view: &0, size, format: FORMAT, sample_count: 1 }
        }

        fn bits(matrix: [[f32; 4]; 4]) -> [[u32; 4]; 4] {
            matrix.map(|column| column.map(f32::to_bits))
        }

        fn upload(uploads: &[(usize, u64, Vec<u8>)], id: usize) -> &[u8] {
            let matches: Vec<_> = uploads.iter()
                .filter(|(buffer, _, _)| *buffer == id).collect();
            assert_eq!(matches.len(), 1, "expected exactly one upload to bound buffer {id}");
            assert_eq!(matches[0].1, 0, "globals must start at byte zero");
            &matches[0].2
        }

        #[test]
        fn prepared_camera_constructor_bounds_and_snapshot_are_cpu_only() {
            let mut original = camera();
            let token = PreparedSceneCamera::new(original, (63, 37)).unwrap();
            let snapshot = token.camera();
            let matrix = bits(token.view_proj());
            original.eye = [f32::NAN; 3];
            let mut copy = token.camera();
            copy.target = [99.0; 3];
            let mut matrix_copy = token.view_proj();
            matrix_copy[0][0] = f32::NAN;
            assert_ne!(copy, snapshot);
            assert!(original.eye[0].is_nan() && matrix_copy[0][0].is_nan());
            assert_eq!(token.camera(), snapshot);
            assert_eq!(token.viewport(), (63, 37));
            assert_eq!(bits(token.view_proj()), matrix);
            assert!(token.view_proj().iter().flatten().all(|v| v.is_finite()));
            for size in [(0, 1), (1, 0), (8193, 1), (1, 8193), (u32::MAX, u32::MAX)] {
                // Target precedence is observable even with a bad camera.
                assert!(matches!(PreparedSceneCamera::new(original, size), Err(ImportedSceneError::InvalidTarget)));
            }
            for size in [(1, 1), (8192, 1), (1, 8192), (8192, 8192)] {
                for c in [camera(), Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 2.0)] {
                    assert_eq!(PreparedSceneCamera::new(c, size).unwrap().viewport(), size);
                }
            }
        }

        #[test]
        fn prepared_camera_reuses_camera_admission_for_invalid_inputs() {
            use crate::Projection;
            let valid = camera();
            let mut invalid = Vec::new();
            for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                invalid.push(Camera3D { eye: [value, 2.0, 3.0], ..valid });
                invalid.push(Camera3D { target: [value, 0.0, 0.0], ..valid });
                invalid.push(Camera3D { up: [0.0, value, 0.0], ..valid });
            }
            invalid.push(Camera3D { target: valid.eye, ..valid });
            invalid.push(Camera3D { up: [0.0; 3], ..valid });
            invalid.push(Camera3D { up: crate::math3::sub(valid.target, valid.eye), ..valid });
            invalid.push(Camera3D { eye: [f32::MAX; 3], ..valid });
            for projection in [
                Projection::Perspective { fov_y: 0.0, near: 0.1, far: 10.0 },
                Projection::Perspective { fov_y: std::f32::consts::PI, near: 0.1, far: 10.0 },
                Projection::Perspective { fov_y: 1.0, near: 0.0, far: 10.0 },
                Projection::Perspective { fov_y: 1.0, near: 1.0, far: 1.0 },
                Projection::Perspective { fov_y: 1.0, near: 0.1, far: f32::INFINITY },
                Projection::Perspective { fov_y: f32::NAN, near: 0.1, far: 10.0 },
                Projection::Orthographic { half_height: 0.0, near: -1.0, far: 1.0 },
                Projection::Orthographic { half_height: 1.0, near: 1.0, far: 1.0 },
                Projection::Orthographic { half_height: 1.0, near: f32::NAN, far: 1.0 },
            ] {
                invalid.push(Camera3D { projection, ..valid });
            }
            for c in invalid {
                assert!(matches!(PreparedSceneCamera::new(c, (64, 48)), Err(ImportedSceneError::Model(ModelRenderError::InvalidCamera))));
            }
        }

        #[test]
        fn prepared_camera_nonsymmetric_orthographic_profile_has_exact_coefficients() {
            // Axis-aligned dyadic profile, derived independently: translation is
            // (-8,4,-2), x/y scale is 1/4 and 1/2, z maps [-2,-6] to [0,1].
            let c = Camera3D {
                eye: [8.0, -4.0, 2.0], target: [8.0, -4.0, 1.0], up: [0.0, 1.0, 0.0],
                projection: crate::Projection::Orthographic { half_height: 2.0, near: 2.0, far: 6.0 },
            };
            let matrix = PreparedSceneCamera::new(c, (64, 32)).unwrap().view_proj();
            let expected = [[0.25, 0.0, 0.0, 0.0], [0.0, 0.5, 0.0, 0.0], [0.0, 0.0, -0.25, 0.0], [-2.0, 2.0, 0.0, 1.0]];
            assert_eq!(matrix, expected);
            let transpose = std::array::from_fn(|i| std::array::from_fn(|j| expected[j][i]));
            assert_ne!(matrix, transpose, "profile must detect row/column confusion");
        }

        #[test]
        fn prepared_camera_exact_uploads_cover_all_batches_orders_and_metadata() {
            for c in [camera(), Camera3D::orthographic([8.0, -4.0, 2.0], [8.0, -4.0, 0.0], 2.0)] {
                for reverse in [false, true] {
                    for shadows in [false, true] {
                      for sentinel in [false, true] {
                        let rhi = Mock::default();
                        let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
                        let mut a = model(&rhi, FORMAT);
                        let mut b = model(&rhi, FORMAT);
                        let mut p = Renderer3D::with_settings(rhi.clone(), FORMAT, crate::Settings3D::LOW);
                        let mut q = Renderer3D::with_settings(rhi.clone(), FORMAT, crate::Settings3D::LOW);
                        let mut list = RenderList3D::new();
                        list.cuboid([0.0; 3], crate::IDENTITY_ROT, [0.5; 3], &crate::Material::new([0.3; 3]));
                        let instances = [StaticInstance::default(); 2];
                        #[cfg(feature = "animation")]
                        let animated = orr_model::animation_import::import_with_resolver(
                            "fixtures/animated_strip.glb", include_bytes!("../../orr_model/tests/fixtures/animated_strip.glb"), |_| panic!("embedded fixture"),
                        ).unwrap();
                        #[cfg(feature = "animation")]
                        let pose = animated.rest_pose().unwrap();
                        #[cfg(feature = "animation")]
                        let skinned_instances = [SkinnedInstance::new(&pose)];
                        #[cfg(feature = "animation")]
                        let mut skin = SkinnedModelRenderer::new(rhi.clone(), FORMAT, animated).unwrap();
                        let mut batches = vec![
                            ImportedBatch::Static(&mut a),
                            ImportedBatch::Procedural { renderer: &mut p, list: &list },
                            ImportedBatch::StaticInstances { renderer: &mut b, instances: &instances },
                            ImportedBatch::Procedural { renderer: &mut q, list: &list },
                        ];
                        #[cfg(feature = "animation")]
                        batches.push(ImportedBatch::Skinned { renderer: &mut skin, instances: &skinned_instances });
                        if reverse { batches.reverse(); }
                        let lighting = Lighting { shadows, ..light() };
                        let mut token = PreparedSceneCamera::new(c, (63, 37)).unwrap();
                        // Private test-only sentinel: differing nonsymmetric bits,
                        // including both signed zeros, detect recomputation or transpose.
                        // No production arbitrary-matrix constructor exists.
                        if sentinel {
                            token.view_proj = [[1.0, -0.0, 3.0, 4.0], [5.0, 6.0, 0.0, 8.0], [9.0, 10.0, 11.0, 12.0], [13.0, 14.0, 15.0, 16.0]];
                        }
                        rhi.take();
                        rhi.4.borrow_mut().clear();
                        scene.draw(target(token.viewport()), &c, &lighting, &PointLightSettings::default(), &mut batches).unwrap();
                        let raw = rhi.take();
                        let raw_uploads = std::mem::take(&mut *rhi.4.borrow_mut());
                        scene.draw_prepared(target(token.viewport()), &token, &lighting, &PointLightSettings::default(), &mut batches).unwrap();
                        let prepared = rhi.take();
                        let prepared_uploads = std::mem::take(&mut *rhi.4.borrow_mut());
                        let count = batches.len();
                        assert_eq!(prepared.submits, 1);
                        assert_eq!(prepared.passes.len(), count);
                        assert_eq!(prepared.shadow_passes.len(), usize::from(shadows));
                        let main = &prepared.pass_uniforms[prepared.pass_uniforms.len() - count..];
                        let raw_main = &raw.pass_uniforms[raw.pass_uniforms.len() - count..];
                        let mut ids = std::collections::BTreeSet::new();
                        for ((batch, bound), raw_bound) in batches.iter().zip(main).zip(raw_main) {
                            let id = bound[0];
                            assert!(ids.insert(id), "each batch owns a distinct globals buffer");
                            assert_eq!(id, raw_bound[0]);
                            let actual = upload(&prepared_uploads, id);
                            let old = upload(&raw_uploads, id);
                            let actual_bits: Vec<_> = actual[..64].chunks_exact(4).map(|bytes| u32::from_ne_bytes(bytes.try_into().unwrap())).collect();
                            assert_eq!(actual_bits, bits(token.view_proj()).into_iter().flatten().collect::<Vec<_>>());
                            if sentinel {
                                assert_ne!(&actual[..64], &old[..64], "sentinel must defeat recomputation");
                            }
                            assert_eq!(&actual[64..], &old[64..], "shadow/light/material/viewport metadata changed");
                            #[cfg(feature = "metallic-roughness")]
                            if !matches!(batch, ImportedBatch::Procedural { .. }) {
                                let eye = [c.eye[0], c.eye[1], c.eye[2], if matches!(c.projection, crate::Projection::Orthographic { .. }) { 1.0 } else { 0.0 }];
                                let forward = c.forward();
                                assert_eq!(&actual[256..272], bytemuck::bytes_of(&eye));
                                assert_eq!(&actual[272..288], bytemuck::bytes_of(&[forward[0], forward[1], forward[2], 0.0]));
                            }
                            #[cfg(not(feature = "metallic-roughness"))]
                            let _ = batch;
                        }
                        // Object and instance uploads must remain unchanged. Shared
                        // shadow shaders consume the preserved light_vp fields above.
                        for (id, offset, bytes) in &prepared_uploads {
                            if !ids.contains(id) {
                                assert!(raw_uploads.iter().any(|(old_id, old_offset, old)| old_id == id && old_offset == offset && old == bytes));
                            }
                        }
                      }
                    }
                }
            }
        }

        #[test]
        fn prepared_camera_viewport_mismatch_preserves_initial_and_existing_state() {
            for existing in [false, true] {
                let rhi = Mock::default();
                let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
                scene.post_process.enabled = true;
                let token = PreparedSceneCamera::new(camera(), (32, 32)).unwrap();
                if existing { scene.draw_prepared(target((32, 32)), &token, &light(), &PointLightSettings::default(), &mut []).unwrap(); }
                let before = (scene.depth_generation(), scene.depth_size(), scene.post_process_generation(), scene.post_process_allocated_bytes(), scene.shadow_map_generation());
                rhi.take();
                for size in [(64, 64), (32, 31), (0, 0)] {
                    assert!(matches!(scene.draw_prepared(target(size), &token, &light(), &PointLightSettings::default(), &mut []), Err(ImportedSceneError::InvalidTarget)));
                    assert_eq!(rhi.take(), Effects::default());
                    assert_eq!((scene.depth_generation(), scene.depth_size(), scene.post_process_generation(), scene.post_process_allocated_bytes(), scene.shadow_map_generation()), before);
                }
            }
        }

        #[test]
        fn prepared_camera_late_rejection_preserves_caches_stats_and_generations() {
            for existing in [false, true] {
                let rhi = Mock::default();
                let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
                scene.post_process.enabled = true;
                let mut a = model(&rhi, TextureFormat::Rgba16Float);
                let mut p = Renderer3D::with_settings(rhi.clone(), TextureFormat::Rgba16Float, crate::Settings3D::LOW);
                let mut good = RenderList3D::new();
                good.cuboid([0.0; 3], crate::IDENTITY_ROT, [0.5; 3], &crate::Material::new([0.3; 3]));
                let mut bad = RenderList3D::new();
                bad.cuboid([0.0; 3], [0.0; 4], [0.5; 3], &crate::Material::new([0.3; 3]));
                let token = PreparedSceneCamera::new(camera(), (32, 32)).unwrap();
                if existing {
                    scene.draw_prepared(target((32, 32)), &token, &light(), &PointLightSettings::default(), &mut [ImportedBatch::Procedural { renderer: &mut p, list: &good }, ImportedBatch::Static(&mut a)]).unwrap();
                }
                let before = (scene.depth_generation(), scene.depth_size(), scene.post_process_generation(), scene.post_process_allocated_bytes(), scene.shadow_map_generation());
                let stats = p.last_frame_stats();
                let resized = PreparedSceneCamera::new(camera(), (64, 64)).unwrap();
                let instances = [StaticInstance::default(); 3];
                rhi.take();
                assert!(matches!(scene.draw_prepared(target((64, 64)), &resized, &light(), &PointLightSettings::default(), &mut [ImportedBatch::StaticInstances { renderer: &mut a, instances: &instances }, ImportedBatch::Procedural { renderer: &mut p, list: &bad }]), Err(ImportedSceneError::Procedural(ProceduralSceneError::InvalidInstance))));
                assert_eq!(rhi.take(), Effects::default());
                assert_eq!(p.last_frame_stats(), stats);
                assert_eq!((scene.depth_generation(), scene.depth_size(), scene.post_process_generation(), scene.post_process_allocated_bytes(), scene.shadow_map_generation()), before);
            }
        }

        #[test]
        fn prepared_camera_target_and_hdr_admission_still_precede_uploads() {
            let rhi = Mock::default();
            let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
            let token = PreparedSceneCamera::new(camera(), (32, 32)).unwrap();
            for bad in [ImportedSceneTarget { sample_count: 4, ..target((32, 32)) }, ImportedSceneTarget { format: TextureFormat::Depth32Float, ..target((32, 32)) }] {
                assert!(matches!(scene.draw_prepared(bad, &token, &light(), &PointLightSettings::default(), &mut []), Err(ImportedSceneError::InvalidTarget)));
                assert_eq!(rhi.take(), Effects::default());
            }
            assert!(matches!(scene.draw_prepared(ImportedSceneTarget { format: TextureFormat::Bgra8Unorm, ..target((32, 32)) }, &token, &light(), &PointLightSettings::default(), &mut []), Err(ImportedSceneError::FormatMismatch)));
            scene.post_process.enabled = true;
            *rhi.3.borrow_mut() = Some(orr_rhi::TextureFormatCapabilities::default());
            assert!(matches!(scene.draw_prepared(target((32, 32)), &token, &light(), &PointLightSettings::default(), &mut []), Err(ImportedSceneError::UnsupportedHdr)));
            assert_eq!(rhi.take(), Effects::default());
            *rhi.3.borrow_mut() = None;
            scene.post_process.threshold = f32::NAN;
            assert!(matches!(scene.draw_prepared(target((32, 32)), &token, &light(), &PointLightSettings::default(), &mut []), Err(ImportedSceneError::InvalidPostProcess)));
            assert_eq!(rhi.take(), Effects::default());
            scene.post_process.threshold = 1.0;
            let large = PreparedSceneCamera::new(camera(), (4000, 4000)).unwrap();
            assert!(matches!(scene.draw_prepared(target(large.viewport()), &large, &light(), &PointLightSettings::default(), &mut []), Err(ImportedSceneError::PostProcessBudget)));
            assert_eq!(rhi.take(), Effects::default());
            assert_eq!(scene.depth_generation(), 0);
        }

        #[cfg(feature = "animation")]
        #[test]
        fn prepared_camera_late_pose_rejection_preserves_submitted_bounds() {
            let rhi = Mock::default();
            let animated = || orr_model::animation_import::import_with_resolver(
                "fixtures/animated_strip.glb", include_bytes!("../../orr_model/tests/fixtures/animated_strip.glb"), |_| panic!("embedded fixture"),
            ).unwrap();
            let source = animated();
            let pose = source.rest_pose().unwrap();
            let foreign = animated().rest_pose().unwrap();
            for existing in [false, true] {
                let mut skin = SkinnedModelRenderer::new(rhi.clone(), TextureFormat::Rgba16Float, source.clone()).unwrap();
                let mut a = model(&rhi, TextureFormat::Rgba16Float);
                let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
                scene.post_process.enabled = true;
                let token = PreparedSceneCamera::new(camera(), (32, 32)).unwrap();
                if existing {
                    scene.draw_prepared(target((32, 32)), &token, &light(), &PointLightSettings::default(), &mut [ImportedBatch::Skinned { renderer: &mut skin, instances: &[SkinnedInstance::new(&pose)] }]).unwrap();
                }
                let bounds = skin.bounds().to_vec();
                let before = (scene.depth_generation(), scene.depth_size(), scene.post_process_generation(), scene.post_process_allocated_bytes());
                let resized = PreparedSceneCamera::new(camera(), (64, 64)).unwrap();
                rhi.take();
                assert!(matches!(scene.draw_prepared(target((64, 64)), &resized, &light(), &PointLightSettings::default(), &mut [ImportedBatch::StaticInstances { renderer: &mut a, instances: &[StaticInstance::default(); 3] }, ImportedBatch::Skinned { renderer: &mut skin, instances: &[SkinnedInstance::new(&foreign)] }]), Err(ImportedSceneError::Skinned(SkinnedRenderError::InvalidPose(_)))));
                assert_eq!(rhi.take(), Effects::default());
                assert_eq!(skin.bounds(), bounds);
                assert_eq!((scene.depth_generation(), scene.depth_size(), scene.post_process_generation(), scene.post_process_allocated_bytes()), before);
            }
        }

        #[test]
        fn prepared_camera_raw_error_order_and_particle_entry_remain_compatible() {
            let rhi = Mock::default();
            let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
            let invalid = Camera3D { eye: [f32::NAN; 3], ..camera() };
            let lighting = Lighting { intensity: f32::NAN, ..light() };
            assert!(matches!(scene.draw(target((0, 0)), &invalid, &lighting, &PointLightSettings::default(), &mut []), Err(ImportedSceneError::InvalidTarget)));
            assert!(matches!(scene.draw(target((32, 32)), &invalid, &lighting, &PointLightSettings::default(), &mut []), Err(ImportedSceneError::Model(ModelRenderError::InvalidCamera))));
            assert!(matches!(scene.draw(target((32, 32)), &camera(), &lighting, &PointLightSettings::default(), &mut []), Err(ImportedSceneError::Model(ModelRenderError::InvalidLighting))));
            #[cfg(feature = "particles")]
            assert!(matches!(scene.draw_with_particles(target((32, 32)), &invalid, &lighting, &PointLightSettings::default(), &mut [], None), Err(ImportedSceneError::Model(ModelRenderError::InvalidCamera))));
            assert_eq!(rhi.take(), Effects::default());
        }
    }

}

#[cfg(all(test, feature = "animation"))]
#[path = "imported_scene/shadow_gpu_tests.rs"]
mod shadow_gpu_tests;

#[cfg(test)]
mod prepared_camera_gpu_tests {
    use super::*;
    use orr_rhi::{Wgpu, WgpuOptions};
    use std::io::Write;

    const FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

    fn capture(name: &str, pixels: &[u8], token: &PreparedSceneCamera, gpu: &Wgpu) {
        if let Some(root) = std::env::var_os("ORR_PREPARED_CAMERA_CAPTURE_DIR") {
            let root = std::path::PathBuf::from(root);
            assert!(root.is_dir(), "runner must provide the empty capture directory");
            let write = |suffix: &str, data: &[u8]| {
                let mut file = std::fs::OpenOptions::new().write(true).create_new(true)
                    .open(root.join(format!("{name}.{suffix}"))).unwrap();
                file.write_all(data).unwrap();
            };
            let size = token.viewport();
            let mut ppm = format!("P6\n{} {}\n255\n", size.0, size.1).into_bytes();
            for pixel in pixels.chunks_exact(4) { ppm.extend_from_slice(&pixel[..3]); }
            write("ppm", &ppm);
            write("rgba", pixels);
            let metadata = serde_json::json!({
                "fixture": name, "size": [size.0, size.1],
                "matrix_bits": token.view_proj().map(|column| column.map(f32::to_bits)),
                "eye": token.camera().eye, "target": token.camera().target,
                "adapter": gpu.adapter_name(), "software": gpu.is_software(),
                "metallic_roughness": cfg!(feature = "metallic-roughness"),
                "animation": cfg!(feature = "animation"),
            });
            write("json", &serde_json::to_vec_pretty(&metadata).unwrap());
        }
    }

    // Independent axis-aligned projection of the frozen world-space landmarks.
    // This uses neither Camera3D::world_to_screen nor the uploaded matrix.
    fn landmark(pixels: &[u8], size: (u32, u32), perspective: bool, point: [f64; 3], kind: u8) {
        let half_h = if perspective {
            (8.0 - point[2]) * (53.0_f64.to_radians() * 0.5).tan()
        } else { 2.4 };
        let half_w = half_h * f64::from(size.0) / f64::from(size.1);
        let x = (f64::from(size.0) * (0.5 + (point[0] - 0.125) / (2.0 * half_w))).floor() as u32;
        let y = (f64::from(size.1) * (0.5 - (point[1] - 0.2) / (2.0 * half_h))).floor() as u32;
        assert!(x > 0 && x + 1 < size.0 && y > 0 && y + 1 < size.1);
        let mut count = 0;
        for row in y - 1..=y + 1 {
            for column in x - 1..=x + 1 {
                let at = ((row * size.0 + column) * 4) as usize;
                let rgb = &pixels[at..at + 3];
                let [r, g, b] = [u16::from(rgb[0]), u16::from(rgb[1]), u16::from(rgb[2])];
                count += usize::from(match kind {
                    0 => r > g + 20 && r > b + 20,
                    1 => b > r + 20 && b > g + 20,
                    2 => g > r + 20 && g > b + 20,
                    _ => r.max(g).max(b) > 10,
                });
            }
        }
        assert!(count >= if kind == 3 { 1 } else { 3 }, "landmark {point:?}, kind {kind}, coverage {count}");
    }

    #[allow(clippy::too_many_arguments)]
    fn batches<'a>(
        model: &'a mut ModelRenderer<Wgpu>, instances: &'a mut ModelRenderer<Wgpu>,
        placements: &'a [StaticInstance],
        a: &'a mut Renderer3D<Wgpu>, a_list: &'a RenderList3D,
        b: &'a mut Renderer3D<Wgpu>, b_list: &'a RenderList3D,
        #[cfg(feature = "animation")] skin: &'a mut SkinnedModelRenderer<Wgpu>,
        #[cfg(feature = "animation")] poses: &'a [SkinnedInstance<'a>],
    ) -> Vec<ImportedBatch<'a, Wgpu>> {
        let mut result = vec![
            ImportedBatch::Static(model),
            ImportedBatch::StaticInstances { renderer: instances, instances: placements },
            ImportedBatch::Procedural { renderer: a, list: a_list },
        ];
        #[cfg(feature = "animation")]
        result.push(ImportedBatch::Skinned { renderer: skin, instances: poses });
        result.push(ImportedBatch::Procedural { renderer: b, list: b_list });
        result
    }

    #[test]
    #[ignore = "opt-in actual software GPU acceptance; run explicitly with ORR_REQUIRE_GPU=1"]
    fn prepared_camera_matches_raw_pixels_and_rejects_without_target_changes() {
        let gpu = Wgpu::headless(WgpuOptions { force_software: true, ..WgpuOptions::default() })
            .expect("prepared-camera acceptance requires a software GPU");
        assert!(gpu.is_software());
        let info = gpu.adapter().get_info();
        eprintln!("prepared-camera adapter: name={:?} backend={:?} device_type={:?} vendor={} device={} driver={:?} driver_info={:?} software={}",
            info.name, info.backend, info.device_type, info.vendor, info.device, info.driver, info.driver_info, gpu.is_software());
        let imported = orr_model::import::import_with_resolver(
            "fixtures/model.glb", include_bytes!("../../orr_model/tests/fixtures/model.glb"), |_| panic!("embedded fixture"),
        ).unwrap();
        let mut source = imported.source().clone();
        source.primitives.truncate(1);
        source.primitives[0].transform = orr_model::IDENTITY;
        source.primitives[0].transform[3][0] = -1.0;
        for vertex in &mut source.primitives[0].vertices {
            vertex.position = vertex.position.map(|v| v * 0.5);
        }
        for image in &mut source.images {
            for pixel in image.rgba8.chunks_exact_mut(4) { pixel.copy_from_slice(&[220, 35, 35, 255]); }
        }
        for material in &mut source.materials { material.base_color = [1.0; 4]; }
        let source = orr_model::StaticModel::new(source).unwrap();
        let make_model = || {
            #[cfg(feature = "metallic-roughness")]
            { ModelRenderer::new_with_materials(gpu.clone(), FORMAT, source.clone(),
                &vec![crate::MaterialShading::MetallicRoughness(crate::ScalarMaterial::new(0.65, 0.35).unwrap()); source.source().materials.len()]).unwrap() }
            #[cfg(not(feature = "metallic-roughness"))]
            { ModelRenderer::new(gpu.clone(), FORMAT, source.clone()).unwrap() }
        };
        let mut model = make_model();
        let mut instanced = make_model();
        let placements = [StaticInstance { translation: [2.0, 0.0, 0.0], ..StaticInstance::default() }];
        let mut a = Renderer3D::with_settings(gpu.clone(), FORMAT, crate::Settings3D::LOW);
        let mut b = Renderer3D::with_settings(gpu.clone(), FORMAT, crate::Settings3D::LOW);
        let mut a_list = RenderList3D::new();
        a_list.cuboid([-1.0, 0.0, 1.0], crate::IDENTITY_ROT, [0.2; 3], &crate::Material::new([0.05, 0.1, 0.9]));
        let mut b_list = RenderList3D::new();
        b_list.cuboid([0.0, -0.9, 0.5], crate::IDENTITY_ROT, [0.25; 3], &crate::Material::new([0.05, 0.8, 0.1]));
        let mut bad = RenderList3D::new();
        bad.cuboid([0.0; 3], [0.0; 4], [0.5; 3], &crate::Material::new([0.3; 3]));
        #[cfg(feature = "animation")]
        let animated = orr_model::animation_import::import_with_resolver(
            "fixtures/animated_strip.glb", include_bytes!("../../orr_model/tests/fixtures/animated_strip.glb"), |_| panic!("embedded fixture"),
        ).unwrap();
        #[cfg(feature = "animation")]
        let pose = animated.rest_pose().unwrap();
        #[cfg(feature = "animation")]
        let mut skin = SkinnedModelRenderer::new(gpu.clone(), FORMAT, animated).unwrap();
        #[cfg(feature = "animation")]
        let poses = [SkinnedInstance { pose: &pose, transform: [
            [0.4, 0.0, 0.0, 0.0], [0.0, 0.4, 0.0, 0.0], [0.0, 0.0, 0.4, 0.0], [-0.12, 0.6, 0.1, 1.0],
        ] }];
        let mut scene = ImportedSceneRenderer::new(gpu.clone(), FORMAT).unwrap();
        scene.clear = [0.0, 0.0, 0.0, 1.0];
        // Raw reference rendering must not allocate or resize the prepared coordinator.
        let mut raw_scene = ImportedSceneRenderer::new(gpu.clone(), FORMAT).unwrap();
        raw_scene.clear = scene.clear;
        let lighting = Lighting {
            shadows: cfg!(feature = "metallic-roughness"), shadow_center: [0.0, 0.5, 0.5], shadow_radius: 3.0,
            sky: [1.0; 3], ground: [1.0; 3], ambient: 0.6, intensity: 0.8, direction: [0.3, -0.6, -1.0],
            tonemap: false, exposure: 1.0, ..Lighting::default()
        };
        let points = PointLightSettings::default();
        let mut captures = 0;
        for (size_index, size) in [(128, 96), (96, 128)].into_iter().enumerate() {
            for (profile, perspective) in [("perspective", true), ("orthographic", false)] {
                let camera = if perspective {
                    Camera3D::perspective([0.125, 0.2, 8.0], [0.125, 0.2, 0.0], 53.0)
                } else { Camera3D::orthographic([0.125, 0.2, 8.0], [0.125, 0.2, 0.0], 2.4) };
                let token = PreparedSceneCamera::new(camera, size).unwrap();
                let matrix = token.view_proj();
                let transposed = std::array::from_fn(|i| std::array::from_fn(|j| matrix[j][i]));
                assert_ne!(matrix, transposed, "fixture must use a nonsymmetric matrix");
                let texture = gpu.create_texture(&TextureDesc {
                    label: "prepared-camera parity and rejection", width: size.0, height: size.1, format: FORMAT,
                    usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC, sample_count: 1, view_formats: &[],
                });
                let view = gpu.create_texture_view(&texture, None);
                let target = || ImportedSceneTarget { view: &view, size, format: FORMAT, sample_count: 1 };
                let prefix = format!("{profile}-{}x{}", size.0, size.1);
                let initial = scene.depth_generation() == 0;
                let mut before = Vec::new();
                if initial {
                    let mut encoder = gpu.create_encoder("known initial target");
                    gpu.encode_render_pass(&mut encoder, "known magenta seed", &ColorAttachment {
                        view: &view, clear: Some([1.0, 0.0, 1.0, 1.0]), resolve: None,
                    }, &[]);
                    gpu.submit(encoder);
                    before = gpu.read_texture(&texture);
                    assert!(before.chunks_exact(4).all(|pixel| pixel == [255, 0, 255, 255]));
                    capture("initial-seed", &before, &token, &gpu);
                    captures += 1;
                }
                for phase in ["initial", "accepted"] {
                    if phase == "initial" && !initial { continue; }
                    if phase == "accepted" {
                        let mut raw = Vec::new();
                        for kind in ["raw", "prepared", "reversed"] {
                            let mut submitted = batches(&mut model, &mut instanced, &placements, &mut a, &a_list, &mut b, &b_list,
                                #[cfg(feature = "animation")] &mut skin,
                                #[cfg(feature = "animation")] &poses);
                            if kind == "reversed" { submitted.reverse(); }
                            if kind == "raw" {
                                let depth_before_reference = (scene.depth_size(), scene.depth_generation());
                                let shadow_before_reference = (scene.shadow_map_size(), scene.shadow_map_generation());
                                raw_scene.draw(target(), &token.camera(), &lighting, &points, &mut submitted).unwrap();
                                assert_eq!((scene.depth_size(), scene.depth_generation()), depth_before_reference);
                                assert_eq!((scene.shadow_map_size(), scene.shadow_map_generation()), shadow_before_reference);
                            } else {
                                if kind == "prepared" {
                                    let previous_generation = size_index as u64 + u64::from(!perspective);
                                    assert_eq!(scene.depth_generation(), previous_generation);
                                }
                                scene.draw_prepared(target(), &token, &lighting, &points, &mut submitted).unwrap();
                                assert_eq!(scene.depth_size(), Some(size));
                                assert_eq!(scene.depth_generation(), size_index as u64 + 1);
                                assert_eq!(scene.shadow_map_generation(), u64::from(lighting.shadows));
                            }
                            let pixels = gpu.read_texture(&texture);
                            assert_eq!(pixels.len(), (size.0 * size.1 * 4) as usize);
                            assert!(pixels.chunks_exact(4).all(|pixel| pixel[3] == 255));
                            landmark(&pixels, size, perspective, [-1.35, 0.35, 0.0], 0);
                            landmark(&pixels, size, perspective, [1.0, 0.25, 0.0], 0);
                            // Blue front face is nearer than the red panel at z=0.
                            landmark(&pixels, size, perspective, [-1.0, 0.0, 1.2], 1);
                            landmark(&pixels, size, perspective, [0.0, -0.9, 0.75], 2);
                            #[cfg(feature = "animation")]
                            landmark(&pixels, size, perspective, [-0.12, 1.2, 0.1], 3);
                            if kind == "raw" { raw = pixels.clone(); }
                            else { assert_eq!(pixels, raw, "{prefix}/{kind} changed actual pixels"); }
                            capture(&format!("{prefix}-{kind}"), &pixels, &token, &gpu);
                            captures += 1;
                            before = pixels;
                        }
                    }
                    let depth = (scene.depth_size(), scene.depth_generation());
                    let hdr = (scene.post_process_size(), scene.post_process_generation(), scene.post_process_allocated_bytes());
                    let stats = (a.last_frame_stats(), b.last_frame_stats());
                    let shadow = (scene.shadow_map_size(), scene.shadow_map_generation());
                    #[cfg(feature = "animation")]
                    let bounds = skin.bounds().to_vec();
                    let mismatch = PreparedSceneCamera::new(camera, (size.0 / 2, size.1 / 2)).unwrap();
                    assert!(matches!(scene.draw_prepared(target(), &mismatch, &lighting, &points, &mut []), Err(ImportedSceneError::InvalidTarget)));
                    let after = gpu.read_texture(&texture);
                    assert_eq!(after, before);
                    capture(&format!("{phase}-{prefix}-mismatch"), &after, &token, &gpu);
                    captures += 1;
                    assert!(matches!(scene.draw_prepared(target(), &token, &lighting, &points,
                        &mut batches(&mut model, &mut instanced, &placements, &mut a, &a_list, &mut b, &bad,
                            #[cfg(feature = "animation")] &mut skin,
                            #[cfg(feature = "animation")] &poses)), Err(ImportedSceneError::Procedural(ProceduralSceneError::InvalidInstance))));
                    let after = gpu.read_texture(&texture);
                    assert_eq!(after, before);
                    assert_eq!((scene.depth_size(), scene.depth_generation()), depth);
                    assert_eq!((scene.post_process_size(), scene.post_process_generation(), scene.post_process_allocated_bytes()), hdr);
                    assert_eq!((a.last_frame_stats(), b.last_frame_stats()), stats);
                    assert_eq!((scene.shadow_map_size(), scene.shadow_map_generation()), shadow);
                    #[cfg(feature = "animation")]
                    assert_eq!(skin.bounds(), bounds);
                    capture(&format!("{phase}-{prefix}-late"), &after, &token, &gpu);
                    captures += 1;
                }
                assert_eq!(scene.depth_generation(), size_index as u64 + 1);
                assert_eq!(scene.shadow_map_generation(), u64::from(lighting.shadows));
            }
        }
        assert_eq!(captures, 23);
        eprintln!("prepared-camera actual-device acceptance: 12 successful frames, 23 captures; raw/prepared/reversed parity, separate coverage, nearer overlap, resize and rejected-target preservation passed; native interaction untested");
    }
}
