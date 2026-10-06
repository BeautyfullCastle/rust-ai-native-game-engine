//! Bounded, single-sample composition of procedural, static and animated models.
//!
//! A frame is validated in full before GPU allocation, uniform writes, command
//! encoding, or submitted-bound changes. The coordinator owns the only depth
//! target used by this path, clears color/depth once, and submits one encoder.
//! Standalone renderer draws retain their independent depth targets and behavior.
#[cfg(feature = "animation")]
use crate::skinned::{PreparedSkinned, SkinnedInstance, SkinnedModelRenderer, SkinnedRenderError};
use crate::{
    model_renderer::{
        prepare_globals, static_instance_draw_count, validate_static_instances, ModelRenderError,
        ModelRenderer, PreparedStatic, StaticInstance, StaticInstanceError,
    },
    point_light::{PointLightError, PointLightSettings},
    renderer3d::{validate_procedural_list, PreparedProcedural, ProceduralSceneError},
    Camera3D, Lighting, RenderList3D, Renderer3D,
};
use orr_rhi::{ColorAttachment, DepthAttachment, Rhi, TextureDesc, TextureFormat, TextureUsage};

/// Bound CPU frame preparation and the number of render passes.
pub const MAX_IMPORTED_BATCHES: usize = 256;
/// Bound combined procedural mesh/line and imported instance/primitive draws.
pub const MAX_IMPORTED_DRAWS: usize = 4096;

#[derive(Debug)]
pub enum ImportedSceneError {
    InvalidTarget,
    FormatMismatch,
    BatchLimit,
    DrawLimit,
    Model(ModelRenderError),
    StaticInstance(StaticInstanceError),
    Procedural(ProceduralSceneError),
    #[cfg(feature = "animation")]
    Skinned(SkinnedRenderError),
    PointLight(PointLightError),
}
impl std::fmt::Display for ImportedSceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTarget => f.write_str(
                "imported scene requires a single-sample color target, finite clear, and dimensions in 1..=8192",
            ),
            Self::FormatMismatch => f.write_str("imported scene target and renderer formats must match"),
            Self::BatchLimit => f.write_str("imported scene batch limit exceeded"),
            Self::DrawLimit => f.write_str("imported scene draw limit exceeded"),
            Self::Model(error) => write!(f, "imported scene: {error}"),
            Self::StaticInstance(error) => write!(f, "imported scene: {error}"),
            Self::Procedural(error) => write!(f, "imported scene: {error}"),
            #[cfg(feature = "animation")]
            Self::Skinned(error) => write!(f, "imported scene: {error}"),
            Self::PointLight(error) => write!(f, "imported scene: {error}"),
        }
    }
}
impl std::error::Error for ImportedSceneError {}

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
    /// used; list lighting, renderer clear, standalone shadows and LOD are ignored.
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
/// No shadows, multisampling, PBR, HDR, or cross-device composition is supported.
pub struct ImportedSceneRenderer<B: Rhi> {
    rhi: B,
    format: TextureFormat,
    depth: Option<Depth<B>>,
    depth_generation: u64,
    pub clear: [f64; 4],
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
            clear: crate::DEFAULT_CLEAR_3D,
        })
    }
    pub fn format(&self) -> TextureFormat {
        self.format
    }
    pub fn depth_size(&self) -> Option<(u32, u32)> {
        self.depth.as_ref().map(|depth| depth.size)
    }
    /// Zero before the first accepted frame; advances on accepted size changes.
    pub fn depth_generation(&self) -> u64 {
        self.depth_generation
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
        Ok(())
    }

    /// Pure authoring-frame validation before target resizing or GPU asset-cache creation.
    /// Admits exactly one procedural batch and the supplied static-instance batches,
    /// using this coordinator's format/clear and a single-sample target. Renderers
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
        self.validate_target(size, self.format, 1)?;
        prepare_globals(self.format, size, camera, lighting).map_err(ImportedSceneError::Model)?;
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
        draws = draws
            .checked_add(extra_draw_count)
            .ok_or(ImportedSceneError::DrawLimit)?;
        draws = draws
            .checked_add(
                validate_procedural_list(procedural).map_err(ImportedSceneError::Procedural)?,
            )
            .ok_or(ImportedSceneError::DrawLimit)?;
        if draws > MAX_IMPORTED_DRAWS {
            return Err(ImportedSceneError::DrawLimit);
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
        self.validate_target(target.size, target.format, target.sample_count)?;
        if batches.len() > MAX_IMPORTED_BATCHES {
            return Err(ImportedSceneError::BatchLimit);
        }
        let mut globals = prepare_globals(self.format, target.size, camera, lighting)
            .map_err(ImportedSceneError::Model)?;
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
        let mut draw_count = 0usize;
        let mut procedural_instances = 0usize;
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
            if format != self.format {
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
                    .prepare_composed(list, camera, target.size, lighting, point)
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

        // All fallible validation is finished. This path never allocates a
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
        let mut encoder = self.rhi.create_encoder("imported scene frame");
        let depth = &self.depth.as_ref().expect("accepted frame owns depth").view;
        if batches.is_empty() {
            self.rhi.encode_pass(
                &mut encoder,
                "empty imported scene",
                Some(&ColorAttachment {
                    view: target.view,
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
                view: target.view,
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
                    renderer.encode_frame(&mut encoder, &color, &depth)
                }
                (
                    ImportedBatch::StaticInstances { renderer, .. },
                    PreparedBatch::StaticInstances(ready),
                ) => renderer.encode_instances(ready, &mut encoder, &color, &depth),
                (ImportedBatch::Procedural { renderer, .. }, PreparedBatch::Procedural(ready)) => {
                    renderer.encode_composed(ready, &mut encoder, &color, &depth)
                }
                #[cfg(feature = "animation")]
                (ImportedBatch::Skinned { renderer, .. }, PreparedBatch::Skinned(ready)) => {
                    renderer.encode_frame(ready, &mut encoder, &color, &depth)
                }
                _ => unreachable!("prepared batches preserve order and kind"),
            }
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
        writes: usize,
        encoders: usize,
        submits: usize,
        textures: Vec<(TextureFormat, u32, (u32, u32))>,
        passes: Vec<(usize, bool, bool, bool, usize)>,
    }
    /// Records every potentially mutating RHI call; no adapter is required.
    #[derive(Clone, Default)]
    struct Mock(Rc<RefCell<Effects>>);
    impl Mock {
        fn allocate(&self) -> usize {
            let mut effects = self.0.borrow_mut();
            effects.allocations += 1;
            effects.allocations
        }
        fn take(&self) -> Effects {
            std::mem::take(&mut *self.0.borrow_mut())
        }
    }
    impl Rhi for Mock {
        type Buffer = usize;
        type Texture = usize;
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
        fn create_buffer(&self, _: &BufferDesc<'_>) -> usize {
            self.allocate()
        }
        fn write_buffer(&self, _: &usize, _: u64, _: &[u8]) {
            self.0.borrow_mut().writes += 1;
        }
        fn create_texture(&self, desc: &TextureDesc<'_>) -> usize {
            self.0.borrow_mut().textures.push((
                desc.format,
                desc.sample_count,
                (desc.width, desc.height),
            ));
            self.allocate()
        }
        fn write_texture_rgba8(
            &self,
            _: &usize,
            _: &TextureUpload<'_>,
        ) -> Result<(), TextureUploadError> {
            self.0.borrow_mut().writes += 1;
            Ok(())
        }
        fn create_texture_view(&self, texture: &usize, _: Option<TextureFormat>) -> usize {
            self.allocate();
            *texture
        }
        fn create_sampler(&self, _: &SamplerDesc) -> usize {
            self.allocate()
        }
        fn sample_count_supported(&self, _: TextureFormat, samples: u32) -> bool {
            samples == 1
        }
        fn create_shader(&self, _: &str, _: &str) -> usize {
            self.allocate()
        }
        fn create_pipeline(&self, _: &PipelineDesc<'_, Self>) -> usize {
            self.allocate()
        }
        fn create_bind_group(&self, _: &usize, _: u32, _: &[Binding<'_, Self>]) -> usize {
            self.allocate()
        }
        fn create_encoder(&self, _: &str) -> usize {
            self.0.borrow_mut().encoders += 1;
            0
        }
        fn encode_pass(
            &self,
            _: &mut usize,
            _: &str,
            color: Option<&ColorAttachment<'_, Self>>,
            depth: Option<&DepthAttachment<'_, Self>>,
            commands: &[Command<'_, Self>],
        ) {
            let color = color.expect("imported scene has a color attachment");
            let depth = depth.expect("imported scene has a depth attachment");
            let draws = commands
                .iter()
                .filter(|command| matches!(command, Command::DrawIndexed { .. }))
                .count();
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
        fn wait_idle(&self) {}
        fn read_texture(&self, _: &usize) -> Vec<u8> {
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
        use crate::{Material, Settings3D, StaticInstance, IDENTITY_ROT};
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
}
