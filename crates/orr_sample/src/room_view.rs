//! Read-only RoomEscapeV1 presentation shared by native play and offscreen tests.
//! The scene index retains full generation-bearing entities; no numeric-index rebinding.
use crate::{
    room_project::{PreparedModels, MAX_MODEL_ASSETS, MAX_MODEL_BINDINGS},
    yard3d_view,
};
use orr_bridge::FrameView;
use orr_ecs::Entity;
use orr_games::room_escape_game::{RoomActor, RoomRun, EXIT, FLOOR, KEY, PLAYER, WALL};
use orr_model_bindings::model_bindings::{LocalTransform, ModelKind};
use orr_physics3d::{Body, Collider};
use orr_reflect::{Guid, SceneIndex};
use orr_render::orr_rhi::{TextureFormat, Wgpu};
use orr_render::{
    Camera3D, ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget, ModelRenderer,
    OffscreenTarget, OrbitCamera, PointLightSettings, RenderList3D, Renderer3D, Settings3D,
    StaticInstance,
};
use orr_view::{Extractor3, RenderItem3, Transform3};
use std::{collections::BTreeSet, sync::Arc};

pub const TARGET_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

/// Exact current-frame collider presentation. The collected key remains in the
/// simulation and GUID index, but is absent from both presentation paths.
pub fn items(frame: FrameView<'_>) -> Vec<RenderItem3> {
    let collected = frame.singleton::<RoomRun>().key_collected != 0;
    let mut extracted = Vec::new();
    yard3d_view::YardExtractor.extract(frame, &mut extracted);
    extracted
        .into_iter()
        .filter_map(|item| {
            let actor = frame.get::<RoomActor>(item.entity)?;
            if actor.kind == KEY && collected {
                return None;
            }
            let mut style = item.style;
            style.color = match actor.kind {
                PLAYER => [0.10, 0.65, 0.95, 1.0],
                KEY => [1.0, 0.72, 0.04, 1.0],
                EXIT => [0.12, 0.85, 0.30, 1.0],
                WALL => [0.37, 0.43, 0.55, 1.0],
                FLOOR => [0.21, 0.26, 0.30, 1.0],
                _ => return None,
            };
            Some(RenderItem3 {
                entity: item.entity,
                transform: item.transform,
                style,
            })
        })
        .collect()
}

/// Fixed oblique overview centered at the room origin, from positive X/Z.
/// Orthographic framing keeps the admitted ±16-unit room visible in either
/// landscape or portrait targets; there is no simulation camera or write-back.
pub fn camera(size: (u32, u32)) -> Camera3D {
    let view = OrbitCamera::new([0.0, 0.5, 0.0], 0.65, 1.02, 50.0).camera();
    let aspect = size.0.max(1) as f32 / size.1.max(1) as f32;
    Camera3D::orthographic(view.eye, view.target, 24.0 / aspect.min(1.0))
}

pub struct ModelPlacement {
    pub entity: Entity,
    pub model: Arc<orr_model::StaticModel>,
    pub instance: StaticInstance,
}

/// Body-to-world × binding-local TRS, identical to the editor model placement
/// convention. Collider extents never scale or displace the imported model.
fn instance(body: Transform3, local: LocalTransform) -> StaticInstance {
    let offset = body.rot.rotate(orr_view::Vec3::new(
        local.translation[0],
        local.translation[1],
        local.translation[2],
    ));
    let rotation = orr_view::Quat::new(
        local.rotation[0],
        local.rotation[1],
        local.rotation[2],
        local.rotation[3],
    );
    StaticInstance {
        translation: (body.pos + offset).to_array(),
        rotation: (body.rot * rotation).normalize().to_array(),
        scale: local.scale,
    }
}

/// CPU-only admission, including static-only enforcement under feature unification.
fn validate_models(models: &PreparedModels) -> Result<(), String> {
    models.document.validate()?;
    if models.document.bindings.is_empty()
        || models.document.bindings.len() > MAX_MODEL_BINDINGS
        || models.assets.is_empty()
        || models.assets.len() > MAX_MODEL_ASSETS
    {
        return Err("room presentation requires 1..=8 assets and 1..=68 bindings".into());
    }
    let mut used = BTreeSet::new();
    let mut draws = 5usize; // Reserve all procedural mesh/line kinds.
    for binding in models.document.bindings.values() {
        if binding.kind != ModelKind::Static || binding.animation.is_some() {
            return Err("room presentation supports static models only".into());
        }
        let key = (binding.package.clone(), binding.asset.clone());
        let loaded = models
            .assets
            .get(&key)
            .ok_or("room presentation missing admitted asset")?;
        binding.validate(loaded)?;
        let model = loaded
            .static_model()
            .ok_or("room presentation asset is not static")?;
        StaticInstance {
            translation: binding.transform.translation,
            rotation: binding.transform.rotation,
            scale: binding.transform.scale,
        }
        .validate_for(model)
        .map_err(|e| e.to_string())?;
        draws = draws
            .checked_add(model.source().primitives.len())
            .ok_or("room draw count overflow")?;
        if draws > orr_render::imported_scene::MAX_IMPORTED_DRAWS {
            return Err("room presentation exceeds imported draw limit".into());
        }
        used.insert(key);
    }
    if used.len() != models.assets.len() {
        return Err("room asset cache contains unbound assets".into());
    }
    Ok(())
}

/// Reject every initial model workload/placement before a project can pass its
/// zero-tick export smoke. The player may reach any corner of the closed envelope.
pub fn admit_presentation(
    frame: FrameView<'_>,
    index: &SceneIndex,
    models: &PreparedModels,
) -> Result<(), String> {
    placements(frame, index, models)?;
    for (guid, binding) in &models.document.bindings {
        let entity = index
            .entity(&Guid::parse(guid)?)
            .ok_or("missing admitted model actor")?;
        let actor = frame.get::<RoomActor>(entity).ok_or("missing room actor")?;
        if actor.kind != PLAYER {
            continue;
        }
        let body = frame.get::<Body>(entity).ok_or("missing room body")?;
        let model = models.assets[&(binding.package.clone(), binding.asset.clone())]
            .static_model()
            .ok_or("missing static model")?;
        for x in [-16.0, 16.0] {
            for z in [-16.0, 16.0] {
                let mut pose = orr_view::fp_to_transform3(body.pos, body.rot);
                pose.pos.x = x;
                pose.pos.z = z;
                instance(pose, binding.transform)
                    .validate_for(model)
                    .map_err(|e| format!("room reachable model placement: {e}"))?;
            }
        }
    }
    Ok(())
}

/// Validates closure before resolving GUID bindings. Stale generations, missing
/// actors, extra actors and duplicate reverse mappings fail instead of hiding errors.
pub fn placements(
    frame: FrameView<'_>,
    index: &SceneIndex,
    models: &PreparedModels,
) -> Result<Vec<ModelPlacement>, String> {
    validate_models(models)?;
    let count = frame.alive_count() as usize;
    if !(3..=MAX_MODEL_BINDINGS).contains(&count)
        || index.len() != count
        || frame.count::<RoomActor>() as usize != count
        || frame.count::<Body>() as usize != count
        || frame.count::<Collider>() as usize != count
    {
        return Err("room presentation frame/GUID closure mismatch".into());
    }
    for (guid, entity) in index.iter() {
        if !frame.exists(entity)
            || index.guid(entity) != Some(guid)
            || frame.get::<RoomActor>(entity).is_none()
            || frame.get::<Body>(entity).is_none()
            || frame.get::<Collider>(entity).is_none()
        {
            return Err("room presentation stale generation or GUID mapping".into());
        }
    }
    for (entity, _) in frame.iter::<RoomActor>() {
        if index.guid(entity).and_then(|guid| index.entity(guid)) != Some(entity) {
            return Err("room presentation actor has no exact GUID".into());
        }
    }
    let collected = frame.singleton::<RoomRun>().key_collected != 0;
    let mut result = Vec::new();
    for (guid, binding) in &models.document.bindings {
        let guid = Guid::parse(guid)?;
        let entity = index
            .entity(&guid)
            .ok_or("room model GUID absent from current scene")?;
        let actor = frame
            .get::<RoomActor>(entity)
            .ok_or("room model actor absent")?;
        let body = frame.get::<Body>(entity).ok_or("room model body absent")?;
        let loaded = &models.assets[&(binding.package.clone(), binding.asset.clone())];
        let model = loaded.static_model().ok_or("room model is not static")?;
        let instance = instance(
            orr_view::fp_to_transform3(body.pos, body.rot),
            binding.transform,
        );
        instance.validate_for(model).map_err(|e| e.to_string())?;
        if actor.kind == KEY && collected {
            continue;
        }
        result.push(ModelPlacement {
            entity,
            model: model.clone(),
            instance,
        });
    }
    Ok(result)
}

struct CachedModel {
    model: Arc<orr_model::StaticModel>,
    renderer: ModelRenderer<Wgpu>,
    instances: Vec<StaticInstance>,
}

/// One immutable admitted asset cache and one shared-depth, single-clear pass.
/// Construction performs no filesystem reads and must follow project admission.
pub struct RoomRenderer {
    target: OffscreenTarget<Wgpu>,
    core: RoomRenderCore,
}

struct RoomRenderCore {
    format: TextureFormat,
    procedural: Renderer3D<Wgpu>,
    imported: ImportedSceneRenderer<Wgpu>,
    models: Vec<CachedModel>,
}
impl RoomRenderer {
    pub fn new(rhi: &Wgpu, size: (u32, u32), models: &PreparedModels) -> Result<Self, String> {
        Self::new_with_format(rhi, size, models, TARGET_FORMAT)
    }

    /// Construct pipelines for an admitted window surface format. The same
    /// format is used by the optional owned offscreen target and direct draws.
    pub fn new_with_format(
        rhi: &Wgpu,
        size: (u32, u32),
        models: &PreparedModels,
        format: TextureFormat,
    ) -> Result<Self, String> {
        validate_models(models)?;
        if size.0 == 0 || size.1 == 0 || size.0 > 4096 || size.1 > 4096 {
            return Err("room target must be 1..=4096 pixels per dimension".into());
        }
        // The coordinator is CPU-only until draw. Check the complete admitted
        // local-placement workload before creating any GPU model or target.
        let imported =
            ImportedSceneRenderer::new(rhi.clone(), format).map_err(|e| e.to_string())?;
        let mut admission = Vec::new();
        for (key, loaded) in &models.assets {
            let instances: Vec<_> = models
                .document
                .bindings
                .values()
                .filter(|binding| (&binding.package, &binding.asset) == (&key.0, &key.1))
                .map(|binding| StaticInstance {
                    translation: binding.transform.translation,
                    rotation: binding.transform.rotation,
                    scale: binding.transform.scale,
                })
                .collect();
            admission.push((
                loaded
                    .static_model()
                    .ok_or("room cache requires static models")?,
                instances,
            ));
        }
        let prepared: Vec<_> = admission
            .iter()
            .map(|(model, instances)| (model.as_ref(), instances.as_slice()))
            .collect();
        let mut empty = RenderList3D::new();
        empty.lighting = yard3d_view::yard_lighting();
        empty.lighting.shadows = false;
        imported
            .preflight(
                size,
                &camera(size),
                &empty.lighting,
                &PointLightSettings::default(),
                &empty,
                &prepared,
            )
            .map_err(|e| e.to_string())?;
        let mut cache = Vec::new();
        for loaded in models.assets.values() {
            let model = loaded
                .static_model()
                .ok_or("room cache requires static models")?;
            cache.push(CachedModel {
                model: model.clone(),
                renderer: ModelRenderer::new(rhi.clone(), format, (**model).clone())
                    .map_err(|e| e.to_string())?,
                instances: Vec::new(),
            });
        }
        Ok(Self {
            target: OffscreenTarget::new(rhi, size.0, size.1, format),
            core: RoomRenderCore {
                format,
                procedural: Renderer3D::with_settings(rhi.clone(), format, Settings3D::LOW),
                imported,
                models: cache,
            },
        })
    }
    pub fn target(&self) -> &OffscreenTarget<Wgpu> {
        &self.target
    }
    pub fn read_rgba8(&self) -> Vec<u8> {
        self.target.read_rgba8()
    }

    /// Resize only the owned offscreen target. Direct surface drawing uses the
    /// dimensions of the acquired surface texture supplied to render_to.
    pub fn resize(&mut self, size: (u32, u32)) -> Result<(), String> {
        if size.0 == 0 || size.1 == 0 || size.0 > 4096 || size.1 > 4096 {
            return Err("room target must be 1..=4096 pixels per dimension".into());
        }
        self.target.resize(size.0, size.1);
        Ok(())
    }

    pub fn render(
        &mut self,
        frame: FrameView<'_>,
        index: &SceneIndex,
        models: &PreparedModels,
        camera: &Camera3D,
    ) -> Result<(), String> {
        let target = ImportedSceneTarget {
            view: self.target.render_view(),
            size: self.target.size(),
            format: self.core.format,
            sample_count: 1,
        };
        self.core.render(frame, index, models, camera, target)
    }

    /// Draw directly into a window-owned texture without readback, copying, or
    /// another shader. The caller supplies a single-sample view on this device.
    pub fn render_to(
        &mut self,
        frame: FrameView<'_>,
        index: &SceneIndex,
        models: &PreparedModels,
        camera: &Camera3D,
        target: ImportedSceneTarget<'_, Wgpu>,
    ) -> Result<(), String> {
        self.core.render(frame, index, models, camera, target)
    }
}

impl RoomRenderCore {
    fn render(
        &mut self,
        frame: FrameView<'_>,
        index: &SceneIndex,
        models: &PreparedModels,
        camera: &Camera3D,
        target: ImportedSceneTarget<'_, Wgpu>,
    ) -> Result<(), String> {
        // Check caller metadata before changing retained instance lists or GPU state.
        if target.sample_count != 1 {
            return Err("room target requires one sample".into());
        }
        if target.format != self.format {
            return Err("room target format differs from admitted renderer format".into());
        }
        if target.size.0 == 0 || target.size.1 == 0 || target.size.0 > 4096 || target.size.1 > 4096
        {
            return Err("room target must be 1..=4096 pixels per dimension".into());
        }
        let placed = placements(frame, index, models)?;
        // Reject replaced/reopened content, including hidden-key assets. Never upload
        // a new model while rendering or trust package/asset strings as GPU identity.
        if models.assets.len() != self.models.len()
            || models.assets.values().any(|loaded| {
                loaded.static_model().is_none_or(|model| {
                    !self
                        .models
                        .iter()
                        .any(|cached| Arc::ptr_eq(&cached.model, model))
                })
            })
        {
            return Err("room renderer asset identity changed; reopen renderer".into());
        }
        let mut groups = vec![Vec::new(); self.models.len()];
        for placement in &placed {
            let slot = self
                .models
                .iter()
                .position(|cached| Arc::ptr_eq(&cached.model, &placement.model))
                .ok_or("room placement not in admitted GPU cache")?;
            groups[slot].push(placement.instance);
        }
        let bound: BTreeSet<_> = placed.iter().map(|p| p.entity).collect();
        let visible: Vec<_> = items(frame)
            .into_iter()
            .filter(|item| !bound.contains(&item.entity))
            .collect();
        let mut list = RenderList3D::new();
        list.lighting = yard3d_view::yard_lighting();
        list.lighting.shadows = false;
        yard3d_view::fill_list(&visible, &mut list);
        let prepared: Vec<_> = self
            .models
            .iter()
            .zip(&groups)
            .filter(|(_, instances)| !instances.is_empty())
            .map(|(entry, instances)| (entry.model.as_ref(), instances.as_slice()))
            .collect();
        self.imported
            .preflight(
                target.size,
                camera,
                &list.lighting,
                &PointLightSettings::default(),
                &list,
                &prepared,
            )
            .map_err(|e| e.to_string())?;
        for (entry, group) in self.models.iter_mut().zip(groups) {
            entry.instances = group;
        }
        let mut batches = vec![ImportedBatch::Procedural {
            renderer: &mut self.procedural,
            list: &list,
        }];
        for entry in &mut self.models {
            if !entry.instances.is_empty() {
                batches.push(ImportedBatch::StaticInstances {
                    renderer: &mut entry.renderer,
                    instances: &entry.instances,
                });
            }
        }
        self.imported
            .draw(
                target,
                camera,
                &list.lighting,
                &PointLightSettings::default(),
                &mut batches,
            )
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_fp::{fp, FPVec3};
    use orr_games::room_escape_game::{RoomConfig, RoomEscapeV1};
    use orr_sim::{Game, Simulation};

    #[test]
    fn current_pose_and_collected_key_are_read_only() {
        let mut frame = orr_ecs::Frame::new(Simulation::<RoomEscapeV1>::build_registry());
        RoomEscapeV1::setup(&mut frame, &RoomConfig);
        let player = FrameView::of(&frame)
            .iter::<RoomActor>()
            .find(|(_, actor)| actor.kind == PLAYER)
            .unwrap()
            .0;
        let key = FrameView::of(&frame)
            .iter::<RoomActor>()
            .find(|(_, actor)| actor.kind == KEY)
            .unwrap()
            .0;
        frame.get_mut::<Body>(player).unwrap().pos = FPVec3::new(fp!(2), fp!(0.5), fp!(-3));
        let before = frame.to_bytes();
        let rendered = items(FrameView::of(&frame));
        assert_eq!(
            rendered
                .iter()
                .find(|item| item.entity == player)
                .unwrap()
                .transform
                .pos
                .to_array(),
            [2.0, 0.5, -3.0]
        );
        assert!(rendered.iter().any(|item| item.entity == key));
        assert_eq!(frame.to_bytes(), before);
        frame.singleton_mut::<RoomRun>().key_collected = 1;
        let before = frame.to_bytes();
        assert!(!items(FrameView::of(&frame))
            .iter()
            .any(|item| item.entity == key));
        assert!(frame.exists(key));
        assert_eq!(frame.to_bytes(), before);
    }

    #[test]
    fn local_translation_rotates_with_body_and_scale_stays_local() {
        let body = Transform3 {
            pos: orr_view::Vec3::new(3.0, 4.0, 5.0),
            rot: orr_view::Quat::new(0.0, 1.0, 0.0, 0.0),
        };
        let result = instance(
            body,
            LocalTransform {
                translation: [1.0, 2.0, 3.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [2.0, 3.0, 4.0],
            },
        );
        assert_eq!(result.translation, [2.0, 6.0, 2.0]);
        assert_eq!(result.rotation, [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(result.scale, [2.0, 3.0, 4.0]);
    }
}
