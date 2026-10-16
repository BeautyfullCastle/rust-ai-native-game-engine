//! Yard3D's main viewport. All transforms come from the host snapshot.
//! Selection uses nearest oriented collider bounds, not imported mesh picking.
use crate::model::{EntityRow, Target};
use orr_bridge::FrameView;
use orr_ecs::Entity;
use orr_render::orr_rhi::{Rhi, TextureFormat, Wgpu};
use orr_render::{Camera3D, OffscreenTarget, RenderList3D, Renderer3D, Settings3D};
use orr_view::{Extractor3, RenderItem3, Shape3, Transform3};

pub const TARGET_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

#[derive(Default)]
pub struct YardFrame {
    pub items: Vec<RenderItem3>,
    pub poses: Vec<(Entity, Transform3)>,
}
impl YardFrame {
    pub fn extract(frame: FrameView<'_>) -> Self {
        let mut extracted = Vec::new();
        orr_sample::yard3d_view::YardExtractor.extract(frame, &mut extracted);
        Self {
            items: extracted
                .into_iter()
                .map(|item| RenderItem3 {
                    entity: item.entity,
                    transform: item.transform,
                    style: item.style,
                })
                .collect(),
            poses: orr_sample::yard3d_view::editor_body_poses(frame),
        }
    }
    pub fn list(&self, hidden: &[Entity], selected: Option<Entity>) -> RenderList3D {
        let mut list = RenderList3D {
            lighting: orr_sample::yard3d_view::yard_lighting(),
            ..RenderList3D::default()
        };
        // Keep the Yard sun's shadow policy. The composed path admits all
        // procedural, static and sampled skinned casters before drawing one
        // shared shadow map for this immutable host frame.
        let visible: Vec<_> = self
            .items
            .iter()
            .copied()
            .filter(|item| !hidden.contains(&item.entity))
            .collect();
        orr_sample::yard3d_view::fill_list(&visible, &mut list);
        if let Some(item) = self.items.iter().find(|item| Some(item.entity) == selected) {
            let p = item.transform.pos.to_array();
            let r = half_extents(item.style.shape)
                .into_iter()
                .fold(0.0_f32, f32::max)
                * 1.08;
            list.aabb(
                p.map(|v| v - r),
                p.map(|v| v + r),
                2.0,
                [1.0, 0.85, 0.2, 1.0],
            );
        }
        list
    }
    pub fn pick(
        &self,
        camera: &Camera3D,
        pixel: [f32; 2],
        size: (u32, u32),
        rows: &[EntityRow],
    ) -> Option<Target> {
        let (origin, direction) = camera.screen_ray(pixel, size);
        self.items
            .iter()
            .filter_map(|item| {
                let pos = item.transform.pos.to_array();
                let q = item.transform.rot.to_array();
                let inv = [-q[0], -q[1], -q[2], q[3]];
                let o = orr_render::math3::quat_rotate(
                    inv,
                    std::array::from_fn(|i| origin[i] - pos[i]),
                );
                let d = orr_render::math3::quat_rotate(inv, direction);
                let t = ray_box(o, d, half_extents(item.style.shape))?;
                let target = rows.iter().find(|row| row.entity == item.entity)?.target();
                Some((t, target))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, target)| target)
    }
}
fn half_extents(shape: Shape3) -> [f32; 3] {
    match shape {
        Shape3::Box { half } => half,
        Shape3::Sphere { radius } => [radius; 3],
        Shape3::Capsule {
            half_length,
            radius,
        } => [radius, half_length + radius, radius],
        Shape3::Plane { half_x, half_z } => [half_x, 0.025, half_z],
    }
}
fn ray_box(o: [f32; 3], d: [f32; 3], half: [f32; 3]) -> Option<f32> {
    let (mut near, mut far) = (0.0_f32, f32::INFINITY);
    for i in 0..3 {
        if d[i].abs() < 1e-7 {
            if o[i].abs() > half[i] {
                return None;
            }
        } else {
            let a = (-half[i] - o[i]) / d[i];
            let b = (half[i] - o[i]) / d[i];
            near = near.max(a.min(b));
            far = far.min(a.max(b));
        }
        if near > far {
            return None;
        }
    }
    near.is_finite().then_some(near)
}

#[cfg(feature = "models")]
pub struct ModelPlacement {
    pub entity: Entity,
    pub model: std::sync::Arc<orr_model::StaticModel>,
    pub instance: orr_render::StaticInstance,
}
/// Scene-owned geometry has no ECS identity and must never hide a collider.
#[cfg(feature = "models")]
pub struct SceneModel {
    pub model: std::sync::Arc<orr_model::StaticModel>,
    pub instance: orr_render::StaticInstance,
}
#[cfg(feature = "animated-models")]
pub struct AnimatedPlacement {
    pub entity: Entity,
    pub model: std::sync::Arc<orr_model::animation::AnimatedModel>,
    /// External body-to-world × binding-local TRS. Animated mesh-node
    /// transforms are carried only in `pose` and must not be multiplied here.
    pub instance: orr_model::animation::Matrix4,
    /// Owned sample for this entity at the coherent Yard snapshot time.
    pub pose: orr_model::animation::Pose,
}
#[cfg(feature = "models")]
struct CachedModel {
    model: std::sync::Arc<orr_model::StaticModel>,
    renderer: orr_render::ModelRenderer<Wgpu>,
    instances: Vec<orr_render::StaticInstance>,
}
#[cfg(feature = "animated-models")]
struct CachedAnimatedModel {
    model: std::sync::Arc<orr_model::animation::AnimatedModel>,
    renderer: orr_render::SkinnedModelRenderer<Wgpu>,
}
#[cfg(feature = "animated-models")]
type AnimatedGroup = (
    std::sync::Arc<orr_model::animation::AnimatedModel>,
    Vec<(orr_model::animation::Pose, orr_model::animation::Matrix4)>,
);
/// Production offscreen renderer, also used verbatim by mandatory GPU tests.
pub struct Viewport3dGpu {
    target: OffscreenTarget<Wgpu>,
    renderer: Renderer3D<Wgpu>,
    /// Advances only when an accepted frame changes every scene pipeline format.
    pipeline_generation: u64,
    #[cfg(feature = "models")]
    imported: orr_render::ImportedSceneRenderer<Wgpu>,
    #[cfg(feature = "models")]
    models: Vec<CachedModel>,
    #[cfg(feature = "animated-models")]
    animated_models: Vec<CachedAnimatedModel>,
}
impl Viewport3dGpu {
    pub fn new(rhi: &Wgpu, size: (u32, u32)) -> Self {
        Self {
            target: OffscreenTarget::new(rhi, size.0, size.1, TARGET_FORMAT),
            renderer: Renderer3D::with_settings(rhi.clone(), TARGET_FORMAT, Settings3D::LOW),
            pipeline_generation: 0,
            #[cfg(feature = "models")]
            imported: orr_render::ImportedSceneRenderer::new(rhi.clone(), TARGET_FORMAT)
                .expect("color format"),
            #[cfg(feature = "models")]
            models: Vec::new(),
            #[cfg(feature = "animated-models")]
            animated_models: Vec::new(),
        }
    }
    pub fn target(&self) -> &OffscreenTarget<Wgpu> {
        &self.target
    }
    pub fn read_rgba8(&self) -> Vec<u8> {
        self.target.read_rgba8()
    }
    pub fn adapter_name(&self) -> String {
        self.renderer.rhi().adapter_name()
    }
    /// Format of all current scene pipelines. The presented texture remains UNORM.
    pub fn scene_format(&self) -> TextureFormat {
        self.renderer.format()
    }
    pub fn pipeline_generation(&self) -> u64 {
        self.pipeline_generation
    }
    #[cfg(feature = "models")]
    pub fn post_process_settings(&self) -> orr_render::PostProcessSettings {
        self.imported.post_process
    }
    #[cfg(feature = "models")]
    pub fn read_hdr_rgba(&self) -> Option<Vec<[f32; 4]>> {
        self.imported.read_hdr_rgba()
    }
    /// Tightly packed little-endian RGBA16F scene bytes for portable diagnostics.
    #[cfg(feature = "models")]
    pub fn read_hdr_rgba16f(&self) -> Option<Vec<u8>> {
        self.imported
            .hdr_scene_texture()
            .map(|texture| self.renderer.rhi().read_texture(texture))
    }
    #[cfg(feature = "models")]
    pub fn post_process_size(&self) -> Option<(u32, u32)> {
        self.imported.post_process_size()
    }
    #[cfg(feature = "models")]
    pub fn post_process_generation(&self) -> u64 {
        self.imported.post_process_generation()
    }
    #[cfg(feature = "models")]
    pub fn post_process_allocated_bytes(&self) -> u64 {
        self.imported.post_process_allocated_bytes()
    }
    /// Asset counts expose the admitted cache, never a rejected partial frame.
    #[cfg(feature = "models")]
    pub fn model_cache_counts(&self) -> (usize, usize) {
        #[cfg(feature = "animated-models")]
        return (self.models.len(), self.animated_models.len());
        #[cfg(not(feature = "animated-models"))]
        (self.models.len(), 0)
    }

    #[cfg(feature = "animated-models")]
    pub fn animated_bounds(&self) -> Vec<Vec<orr_render::SkinnedBounds>> {
        self.animated_models
            .iter()
            .map(|entry| entry.renderer.bounds().to_vec())
            .collect()
    }

    #[cfg(not(feature = "models"))]
    pub fn render(
        &mut self,
        size: (u32, u32),
        list: &RenderList3D,
        camera: &Camera3D,
    ) -> Result<(), String> {
        self.target.resize(size.0, size.1);
        self.renderer
            .draw(self.target.render_view(), size, list, camera);
        Ok(())
    }
    #[cfg(feature = "models")]
    pub fn render(
        &mut self,
        size: (u32, u32),
        list: &RenderList3D,
        camera: &Camera3D,
        placements: &[ModelPlacement],
    ) -> Result<(), String> {
        self.render_post_processed(size, list, camera, placements, self.post_process_settings())
    }

    /// Admit the complete next frame and its output policy before changing any cache.
    #[cfg(feature = "models")]
    pub fn render_post_processed(
        &mut self,
        size: (u32, u32),
        list: &RenderList3D,
        camera: &Camera3D,
        placements: &[ModelPlacement],
        settings: orr_render::PostProcessSettings,
    ) -> Result<(), String> {
        self.render_composed(
            size,
            list,
            camera,
            placements,
            &[],
            #[cfg(feature = "animated-models")]
            &[],
            settings,
            #[cfg(feature = "irradiance-probes")]
            None,
        )
    }

    #[cfg(feature = "animated-models")]
    pub fn render_mixed(
        &mut self,
        size: (u32, u32),
        list: &RenderList3D,
        camera: &Camera3D,
        placements: &[ModelPlacement],
        animated_placements: &[AnimatedPlacement],
    ) -> Result<(), String> {
        self.render_mixed_post_processed(
            size,
            list,
            camera,
            placements,
            animated_placements,
            self.post_process_settings(),
        )
    }

    #[cfg(feature = "animated-models")]
    pub fn render_mixed_post_processed(
        &mut self,
        size: (u32, u32),
        list: &RenderList3D,
        camera: &Camera3D,
        placements: &[ModelPlacement],
        animated_placements: &[AnimatedPlacement],
        settings: orr_render::PostProcessSettings,
    ) -> Result<(), String> {
        self.render_composed(
            size,
            list,
            camera,
            placements,
            &[],
            animated_placements,
            settings,
            #[cfg(feature = "irradiance-probes")]
            None,
        )
    }

    /// Render one fully admitted frame with an optional view-only irradiance grid.
    /// Grid validation precedes resizing, cache creation and all GPU writes.
    #[cfg(feature = "irradiance-probes")]
    #[allow(clippy::too_many_arguments)] // One coherent composed frame plus its view-only policy.
    pub fn render_irradiance(
        &mut self,
        size: (u32, u32),
        list: &RenderList3D,
        camera: &Camera3D,
        placements: &[ModelPlacement],
        #[cfg(feature = "animated-models")] animated_placements: &[AnimatedPlacement],
        settings: orr_render::PostProcessSettings,
        grid: Option<&orr_render::IrradianceGrid>,
    ) -> Result<(), String> {
        self.render_composed(size, list, camera, placements, &[],
            #[cfg(feature = "animated-models")] animated_placements,
            settings, grid)
    }

    #[cfg(feature = "irradiance-probes")]
    pub fn irradiance(&self) -> Option<&orr_render::IrradianceGrid> {
        self.imported.irradiance.as_ref()
    }

    /// Persistent probe payload, excluding existing renderer uniforms and driver overhead.
    #[cfg(feature = "irradiance-probes")]
    pub fn irradiance_uniform_bytes(&self) -> usize {
        let (static_count, animated_count) = self.model_cache_counts();
        (2 + static_count + animated_count) * std::mem::size_of::<orr_render::IrradianceUniform>()
    }

    /// Compose scene-owned static geometry with entity models in the same depth/shadow pass.
    #[cfg(feature = "terrain")]
    #[allow(clippy::too_many_arguments)]
    pub fn render_scene(
        &mut self,
        size: (u32, u32),
        list: &RenderList3D,
        camera: &Camera3D,
        placements: &[ModelPlacement],
        scene_models: &[SceneModel],
        #[cfg(feature = "animated-models")] animated_placements: &[AnimatedPlacement],
        settings: orr_render::PostProcessSettings,
        #[cfg(feature = "irradiance-probes")] grid: Option<&orr_render::IrradianceGrid>,
    ) -> Result<(), String> {
        self.render_composed(size, list, camera, placements, scene_models,
            #[cfg(feature = "animated-models")] animated_placements,
            settings,
            #[cfg(feature = "irradiance-probes")] grid)
    }

    #[cfg(feature = "models")]
    #[allow(clippy::too_many_arguments)] // Keep all admission inputs in one transaction.
    fn render_composed(
        &mut self,
        size: (u32, u32),
        list: &RenderList3D,
        camera: &Camera3D,
        placements: &[ModelPlacement],
        scene_models: &[SceneModel],
        #[cfg(feature = "animated-models")] animated_placements: &[AnimatedPlacement],
        settings: orr_render::PostProcessSettings,
        #[cfg(feature = "irradiance-probes")] grid: Option<&orr_render::IrradianceGrid>,
    ) -> Result<(), String> {
        #[cfg(feature = "irradiance-probes")]
        if let Some(grid) = grid { grid.validate()?; }
        #[cfg(feature = "animated-models")]
        let entity_count = placements
            .len()
            .checked_add(animated_placements.len())
            .ok_or("Viewport model entity limit exceeded")?;
        #[cfg(not(feature = "animated-models"))]
        let entity_count = placements.len();
        let entity_count = entity_count.checked_add(scene_models.len()).ok_or("Viewport static input limit exceeded")?;
        if entity_count > 256 {
            return Err("Viewport supports at most 256 model-bound entities".into());
        }

        // Group by immutable asset identity and own all sampled poses before
        // building SkinnedInstance slices that borrow those poses.
        let mut groups: Vec<(
            std::sync::Arc<orr_model::StaticModel>,
            Vec<orr_render::StaticInstance>,
        )> = Vec::new();
        for (model, instance) in placements.iter().map(|p| (&p.model, p.instance))
            .chain(scene_models.iter().map(|p| (&p.model, p.instance))) {
            if let Some((_, instances)) = groups
                .iter_mut()
                .find(|(cached, _)| std::sync::Arc::ptr_eq(cached, model))
            {
                instances.push(instance);
            } else {
                if groups.len() >= 8 {
                    return Err("Viewport supports at most eight distinct model assets".into());
                }
                groups.push((model.clone(), vec![instance]));
            }
        }

        #[cfg(feature = "animated-models")]
        let mut animated_groups: Vec<AnimatedGroup> = Vec::new();
        #[cfg(feature = "animated-models")]
        for placement in animated_placements {
            if let Some((_, instances)) = animated_groups
                .iter_mut()
                .find(|(model, _)| std::sync::Arc::ptr_eq(model, &placement.model))
            {
                instances.push((placement.pose.clone(), placement.instance));
            } else {
                if groups.len() + animated_groups.len() >= 8 {
                    return Err("Viewport supports at most eight distinct model assets".into());
                }
                animated_groups.push((
                    placement.model.clone(),
                    vec![(placement.pose.clone(), placement.instance)],
                ));
            }
        }
        #[cfg(feature = "animated-models")]
        if groups.len() + animated_groups.len() > 8 {
            return Err("Viewport supports at most eight distinct model assets".into());
        }
        #[cfg(not(feature = "animated-models"))]
        if groups.len() > 8 {
            return Err("Viewport supports at most eight distinct model assets".into());
        }

        let prepared: Vec<_> = groups
            .iter()
            .map(|(model, instances)| (model.as_ref(), instances.as_slice()))
            .collect();
        #[cfg(feature = "animated-models")]
        let skinned_instances: Vec<Vec<orr_render::SkinnedInstance<'_>>> = animated_groups
            .iter()
            .map(|(_, instances)| {
                instances
                    .iter()
                    .map(|(pose, transform)| orr_render::SkinnedInstance {
                        pose,
                        transform: *transform,
                    })
                    .collect()
            })
            .collect();
        #[cfg(feature = "animated-models")]
        let prepared_skinned: Vec<_> = animated_groups
            .iter()
            .zip(&skinned_instances)
            .map(|((model, _), instances)| (model.as_ref(), instances.as_slice()))
            .collect();
        // Preflight must see retained HDR resources to account for old + new
        // allocation budgets on resize. Restore the policy even on rejection.
        #[cfg(feature = "irradiance-probes")]
        let previous_grid = std::mem::replace(&mut self.imported.irradiance, grid.cloned());
        let previous_settings = self.imported.post_process;
        self.imported.post_process = settings;
        #[cfg(feature = "animated-models")]
        let admission = self
            .imported
            .preflight_mixed(
                size,
                camera,
                &list.lighting,
                &orr_render::PointLightSettings::default(),
                list,
                &prepared,
                &prepared_skinned,
            )
            .map_err(|e| e.to_string());
        #[cfg(not(feature = "animated-models"))]
        let admission = self
            .imported
            .preflight(
                size,
                camera,
                &list.lighting,
                &orr_render::PointLightSettings::default(),
                list,
                &prepared,
            )
            .map_err(|e| e.to_string());
        self.imported.post_process = previous_settings;
        #[cfg(feature = "irradiance-probes")]
        { self.imported.irradiance = previous_grid; }
        admission?;

        let format = if settings.enabled {
            TextureFormat::Rgba16Float
        } else {
            TARGET_FORMAT
        };
        let format_changed = self.renderer.format() != format;
        let rhi = self.renderer.rhi().clone();
        // Construct all missing/format-specific renderers off to the side. If an
        // upload fails, no admitted model cache, target, bounds or policy changed.
        let replacement_renderer =
            format_changed.then(|| Renderer3D::with_settings(rhi.clone(), format, Settings3D::LOW));
        let mut new_models = Vec::new();
        for (model, _) in &groups {
            if format_changed
                || !self
                    .models
                    .iter()
                    .any(|entry| std::sync::Arc::ptr_eq(&entry.model, model))
            {
                new_models.push(CachedModel {
                    model: model.clone(),
                    renderer: orr_render::ModelRenderer::new(
                        rhi.clone(),
                        format,
                        (**model).clone(),
                    )
                    .map_err(|error| error.to_string())?,
                    instances: Vec::new(),
                });
            }
        }
        #[cfg(feature = "animated-models")]
        let mut new_animated = Vec::new();
        #[cfg(feature = "animated-models")]
        for (model, _) in &animated_groups {
            if format_changed
                || !self
                    .animated_models
                    .iter()
                    .any(|entry| std::sync::Arc::ptr_eq(&entry.model, model))
            {
                new_animated.push(CachedAnimatedModel {
                    model: model.clone(),
                    renderer: orr_render::SkinnedModelRenderer::new(
                        rhi.clone(),
                        format,
                        (**model).clone(),
                    )
                    .map_err(|error| error.to_string())?,
                });
            }
        }
        if let Some(renderer) = replacement_renderer {
            self.renderer = renderer;
            self.models.clear();
            #[cfg(feature = "animated-models")]
            self.animated_models.clear();
            self.pipeline_generation += 1;
        }
        self.models.extend(new_models);
        let mut ordered = Vec::with_capacity(groups.len());
        for (model, instances) in &groups {
            let index = self
                .models
                .iter()
                .position(|entry| std::sync::Arc::ptr_eq(&entry.model, model))
                .expect("admitted static cache");
            let mut entry = self.models.swap_remove(index);
            entry.instances.clone_from(instances);
            ordered.push(entry);
        }
        self.models = ordered;
        #[cfg(feature = "animated-models")]
        {
            self.animated_models.extend(new_animated);
            let mut ordered = Vec::with_capacity(animated_groups.len());
            for (model, _) in &animated_groups {
                let index = self
                    .animated_models
                    .iter()
                    .position(|entry| std::sync::Arc::ptr_eq(&entry.model, model))
                    .expect("admitted animated cache");
                ordered.push(self.animated_models.swap_remove(index));
            }
            self.animated_models = ordered;
        }
        self.imported.post_process = settings;
        #[cfg(feature = "irradiance-probes")]
        { self.imported.irradiance = grid.cloned(); }
        self.target.resize(size.0, size.1);
        let mut batches = vec![orr_render::ImportedBatch::Procedural {
            renderer: &mut self.renderer,
            list,
        }];
        for entry in &mut self.models {
            batches.push(orr_render::ImportedBatch::StaticInstances {
                renderer: &mut entry.renderer,
                instances: &entry.instances,
            });
        }
        #[cfg(feature = "animated-models")]
        for (entry, instances) in self.animated_models.iter_mut().zip(&skinned_instances) {
            batches.push(orr_render::ImportedBatch::Skinned {
                renderer: &mut entry.renderer,
                instances,
            });
        }
        self.imported
            .draw(
                orr_render::ImportedSceneTarget {
                    view: self.target.render_view(),
                    size,
                    format: TARGET_FORMAT,
                    sample_count: 1,
                },
                camera,
                &list.lighting,
                &orr_render::PointLightSettings::default(),
                &mut batches,
            )
            .map_err(|e| e.to_string())
    }
}
pub struct GpuViewport3d {
    gpu: Viewport3dGpu,
    state: egui_wgpu::RenderState,
    id: egui::TextureId,
    generation: u64,
    has_frame: bool,
}
impl GpuViewport3d {
    pub fn new(state: &egui_wgpu::RenderState, size: (u32, u32)) -> Self {
        Self::with_hdr_support(state, size, true)
    }
    /// `false` deliberately narrows the viewport capability; it cannot enable
    /// HDR on an unsupported adapter or change the underlying device.
    pub fn with_hdr_support(
        state: &egui_wgpu::RenderState,
        size: (u32, u32),
        allow_hdr: bool,
    ) -> Self {
        let rhi = Wgpu::from_parts(
            state.instance.clone(),
            state.adapter.clone(),
            state.device.clone(),
            state.queue.clone(),
        );
        let rhi = if allow_hdr {
            rhi
        } else {
            rhi.without_hdr_support()
        };
        let gpu = Viewport3dGpu::new(&rhi, size);
        let id = state.renderer.write().register_native_texture(
            &state.device,
            gpu.target().sample_view(),
            egui_wgpu::wgpu::FilterMode::Linear,
        );
        Self {
            generation: gpu.target().generation(),
            has_frame: false,
            gpu,
            state: state.clone(),
            id,
        }
    }
    pub fn last_texture(&self) -> Option<egui::TextureId> {
        self.has_frame.then_some(self.id)
    }

    pub fn gpu(&self) -> &Viewport3dGpu {
        &self.gpu
    }
    #[cfg(feature = "models")]
    pub fn render_post_processed(
        &mut self,
        size: (u32, u32),
        list: &RenderList3D,
        camera: &Camera3D,
        placements: &[ModelPlacement],
        settings: orr_render::PostProcessSettings,
    ) -> Result<egui::TextureId, String> {
        self.gpu
            .render_post_processed(size, list, camera, placements, settings)?;
        Ok(self.publish_texture())
    }
    #[cfg(feature = "animated-models")]
    pub fn render_mixed_post_processed(
        &mut self,
        size: (u32, u32),
        list: &RenderList3D,
        camera: &Camera3D,
        placements: &[ModelPlacement],
        animated_placements: &[AnimatedPlacement],
        settings: orr_render::PostProcessSettings,
    ) -> Result<egui::TextureId, String> {
        self.gpu.render_mixed_post_processed(
            size,
            list,
            camera,
            placements,
            animated_placements,
            settings,
        )?;
        Ok(self.publish_texture())
    }
    #[cfg(feature = "irradiance-probes")]
    #[allow(clippy::too_many_arguments)] // One coherent composed frame plus its view-only policy.
    pub fn render_irradiance(
        &mut self,
        size: (u32, u32),
        list: &RenderList3D,
        camera: &Camera3D,
        placements: &[ModelPlacement],
        #[cfg(feature = "animated-models")] animated_placements: &[AnimatedPlacement],
        settings: orr_render::PostProcessSettings,
        grid: Option<&orr_render::IrradianceGrid>,
    ) -> Result<egui::TextureId, String> {
        self.gpu.render_irradiance(size, list, camera, placements,
            #[cfg(feature = "animated-models")] animated_placements,
            settings, grid)?;
        Ok(self.publish_texture())
    }
    #[cfg(feature = "terrain")]
    #[allow(clippy::too_many_arguments)]
    pub fn render_scene(
        &mut self,
        size: (u32, u32),
        list: &RenderList3D,
        camera: &Camera3D,
        placements: &[ModelPlacement],
        scene_models: &[SceneModel],
        #[cfg(feature = "animated-models")] animated_placements: &[AnimatedPlacement],
        settings: orr_render::PostProcessSettings,
        #[cfg(feature = "irradiance-probes")] grid: Option<&orr_render::IrradianceGrid>,
    ) -> Result<egui::TextureId, String> {
        self.gpu.render_scene(size, list, camera, placements, scene_models,
            #[cfg(feature = "animated-models")] animated_placements,
            settings,
            #[cfg(feature = "irradiance-probes")] grid)?;
        Ok(self.publish_texture())
    }
    fn publish_texture(&mut self) -> egui::TextureId {
        if self.generation != self.gpu.target().generation() {
            self.generation = self.gpu.target().generation();
            self.state
                .renderer
                .write()
                .update_egui_texture_from_wgpu_texture(
                    &self.state.device,
                    self.gpu.target().sample_view(),
                    egui_wgpu::wgpu::FilterMode::Linear,
                    self.id,
                );
        }
        self.has_frame = true;
        self.id
    }

    pub fn render(
        &mut self,
        size: (u32, u32),
        list: &RenderList3D,
        camera: &Camera3D,
        #[cfg(feature = "models")] placements: &[ModelPlacement],
    ) -> Result<egui::TextureId, String> {
        self.gpu.render(
            size,
            list,
            camera,
            #[cfg(feature = "models")]
            placements,
        )?;
        Ok(self.publish_texture())
    }

    #[cfg(feature = "animated-models")]
    pub fn render_mixed(
        &mut self,
        size: (u32, u32),
        list: &RenderList3D,
        camera: &Camera3D,
        placements: &[ModelPlacement],
        animated_placements: &[AnimatedPlacement],
    ) -> Result<egui::TextureId, String> {
        self.gpu
            .render_mixed(size, list, camera, placements, animated_placements)?;
        Ok(self.publish_texture())
    }
}
impl Drop for GpuViewport3d {
    fn drop(&mut self) {
        self.state.renderer.write().free_texture(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yard_main_viewport_preserves_the_sample_sun_shadow_policy() {
        let list = YardFrame::default().list(&[], None);
        assert_eq!(list.lighting, orr_sample::yard3d_view::yard_lighting());
        assert!(list.lighting.shadows);
    }
}
