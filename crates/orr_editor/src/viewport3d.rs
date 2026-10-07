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
        list.lighting.shadows = false;
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
        #[cfg(feature = "animated-models")]
        return self.render_composed(size, list, camera, placements, &[]);
        #[cfg(not(feature = "animated-models"))]
        self.render_composed(size, list, camera, placements)
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
        self.render_composed(size, list, camera, placements, animated_placements)
    }

    #[cfg(feature = "models")]
    fn render_composed(
        &mut self,
        size: (u32, u32),
        list: &RenderList3D,
        camera: &Camera3D,
        placements: &[ModelPlacement],
        #[cfg(feature = "animated-models")] animated_placements: &[AnimatedPlacement],
    ) -> Result<(), String> {
        #[cfg(feature = "animated-models")]
        let entity_count = placements
            .len()
            .checked_add(animated_placements.len())
            .ok_or("Viewport model entity limit exceeded")?;
        #[cfg(not(feature = "animated-models"))]
        let entity_count = placements.len();
        if entity_count > 256 {
            return Err("Viewport supports at most 256 model-bound entities".into());
        }

        // Group by immutable asset identity and own all sampled poses before
        // building SkinnedInstance slices that borrow those poses.
        let mut groups: Vec<(
            std::sync::Arc<orr_model::StaticModel>,
            Vec<orr_render::StaticInstance>,
        )> = Vec::new();
        for placement in placements {
            if let Some((_, instances)) = groups
                .iter_mut()
                .find(|(model, _)| std::sync::Arc::ptr_eq(model, &placement.model))
            {
                instances.push(placement.instance);
            } else {
                if groups.len() >= 8 {
                    return Err("Viewport supports at most eight distinct model assets".into());
                }
                groups.push((placement.model.clone(), vec![placement.instance]));
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
        #[cfg(feature = "animated-models")]
        self.imported
            .preflight_mixed(
                size,
                camera,
                &list.lighting,
                &orr_render::PointLightSettings::default(),
                list,
                &prepared,
                &prepared_skinned,
            )
            .map_err(|e| e.to_string())?;
        #[cfg(not(feature = "animated-models"))]
        self.imported
            .preflight(
                size,
                camera,
                &list.lighting,
                &orr_render::PointLightSettings::default(),
                list,
                &prepared,
            )
            .map_err(|e| e.to_string())?;

        // The complete mixed CPU frame has been admitted. GPU cache mutations
        // and target resizing below can no longer expose an invalid partial frame.
        for model in &mut self.models {
            model.instances.clear();
        }
        for (model, instances) in &groups {
            let index = if let Some(index) = self
                .models
                .iter()
                .position(|entry| std::sync::Arc::ptr_eq(&entry.model, model))
            {
                index
            } else {
                let renderer = orr_render::ModelRenderer::new(
                    self.renderer.rhi().clone(),
                    TARGET_FORMAT,
                    (**model).clone(),
                )
                .map_err(|e| e.to_string())?;
                self.models.push(CachedModel {
                    model: model.clone(),
                    renderer,
                    instances: Vec::new(),
                });
                self.models.len() - 1
            };
            self.models[index]
                .instances
                .extend(instances.iter().copied());
        }
        self.models.retain(|entry| {
            groups
                .iter()
                .any(|(model, _)| std::sync::Arc::ptr_eq(&entry.model, model))
        });

        #[cfg(feature = "animated-models")]
        {
            let mut ordered = Vec::with_capacity(animated_groups.len());
            for (model, _) in &animated_groups {
                if let Some(index) = self
                    .animated_models
                    .iter()
                    .position(|entry| std::sync::Arc::ptr_eq(&entry.model, model))
                {
                    ordered.push(self.animated_models.swap_remove(index));
                } else {
                    let renderer = orr_render::SkinnedModelRenderer::new(
                        self.renderer.rhi().clone(),
                        TARGET_FORMAT,
                        (**model).clone(),
                    )
                    .map_err(|e| e.to_string())?;
                    ordered.push(CachedAnimatedModel {
                        model: model.clone(),
                        renderer,
                    });
                }
            }
            self.animated_models = ordered;
        }
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
        let rhi = Wgpu::from_parts(
            state.instance.clone(),
            state.adapter.clone(),
            state.device.clone(),
            state.queue.clone(),
        );
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
        Ok(self.id)
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
        Ok(self.id)
    }
}
impl Drop for GpuViewport3d {
    fn drop(&mut self) {
        self.state.renderer.write().free_texture(&self.id);
    }
}
