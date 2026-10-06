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
        let mut list = RenderList3D {lighting:orr_sample::yard3d_view::yard_lighting(),..RenderList3D::default()};
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
#[cfg(feature = "models")]
struct CachedModel {
    model: std::sync::Arc<orr_model::StaticModel>,
    renderer: orr_render::ModelRenderer<Wgpu>,
    instances: Vec<orr_render::StaticInstance>,
}
/// Production offscreen renderer, also used verbatim by mandatory GPU tests.
pub struct Viewport3dGpu {
    target: OffscreenTarget<Wgpu>,
    renderer: Renderer3D<Wgpu>,
    #[cfg(feature = "models")]
    imported: orr_render::ImportedSceneRenderer<Wgpu>,
    #[cfg(feature = "models")]
    models: Vec<CachedModel>,
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
        // Preflight the complete CPU frame before target allocation or GPU cache changes.
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
                groups.push((placement.model.clone(), vec![placement.instance]));
            }
        }
        let prepared: Vec<_> = groups
            .iter()
            .map(|(model, instances)| (model.as_ref(), instances.as_slice()))
            .collect();
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
        for model in &mut self.models {
            model.instances.clear();
        }
        for placement in placements {
            let index = if let Some(index) = self
                .models
                .iter()
                .position(|entry| std::sync::Arc::ptr_eq(&entry.model, &placement.model))
            {
                index
            } else {
                let renderer = orr_render::ModelRenderer::new(
                    self.renderer.rhi().clone(),
                    TARGET_FORMAT,
                    (*placement.model).clone(),
                )
                .map_err(|e| e.to_string())?;
                self.models.push(CachedModel {
                    model: placement.model.clone(),
                    renderer,
                    instances: Vec::new(),
                });
                self.models.len() - 1
            };
            self.models[index].instances.push(placement.instance);
        }
        self.models.retain(|entry| !entry.instances.is_empty());
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
}
impl Drop for GpuViewport3d {
    fn drop(&mut self) {
        self.state.renderer.write().free_texture(&self.id);
    }
}
