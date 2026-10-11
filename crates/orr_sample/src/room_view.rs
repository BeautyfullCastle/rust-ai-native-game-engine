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
fn instance(
    body: Transform3,
    local: LocalTransform,
    material_override: Option<orr_model::MaterialOverride>,
) -> StaticInstance {
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
        material_override,
    }
}

/// An explicit presentation clock. Edit/Stop uses rest, while Play at tick zero
/// samples the authored time-zero pose. Native play uses the default fixed rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresentationTime {
    pub tick_rate: u32,
    pub rest: bool,
}
impl Default for PresentationTime {
    fn default() -> Self {
        Self {
            tick_rate: orr_games::room_escape_game::TICK_RATE,
            rest: false,
        }
    }
}

#[cfg(feature = "room-character")]
pub struct CharacterPlacement {
    pub entity: Entity,
    pub model: Arc<orr_model::animation::AnimatedModel>,
    pub pose: orr_model::animation::Pose,
    pub transform: orr_model::animation::Matrix4,
}
#[cfg(feature = "room-character")]
impl CharacterPlacement {
    fn instance(&self) -> orr_render::SkinnedInstance<'_> {
        orr_render::SkinnedInstance {
            pose: &self.pose,
            transform: self.transform,
        }
    }
}
#[cfg(feature = "room-character")]
fn animated_transform(instance: StaticInstance) -> orr_model::animation::Matrix4 {
    orr_model::animation::Trs {
        translation: instance.translation,
        rotation: instance.rotation,
        scale: instance.scale,
    }
    .matrix()
}

/// CPU-only admission. Unrelated unified animation dependencies never enable
/// animation here: the dedicated feature and valid character declaration do.
fn validate_models(models: &PreparedModels) -> Result<(), String> {
    models.document.validate()?;
    if models.document.bindings.is_empty()
        || models.document.bindings.len() > MAX_MODEL_BINDINGS
        || models.assets.is_empty()
        || models.assets.len() > MAX_MODEL_ASSETS
    {
        return Err("room presentation requires 1..=8 assets and 1..=68 bindings".into());
    }
    #[cfg(feature = "room-character")]
    if let Some(character) = &models.character {
        character.validate()?;
        let binding = models
            .document
            .bindings
            .get(&character.player)
            .ok_or("room character PLAYER binding missing")?;
        if binding.kind != ModelKind::Animated
            || binding.animation
                != Some(orr_model_bindings::model_bindings::AnimationDescriptor {
                    clip_index: character.searching,
                    playback: orr_model_bindings::model_bindings::PlaybackMode::Loop,
                })
        {
            return Err("room character PLAYER binding must default to searching + Loop".into());
        }
    }
    let mut used = BTreeSet::new();
    let mut draws = 5usize; // Reserve all procedural mesh/line kinds.
    for (guid, binding) in &models.document.bindings {
        let _ = guid; // Character-disabled builds still validate every binding.
        let key = (binding.package.clone(), binding.asset.clone());
        let loaded = models
            .assets
            .get(&key)
            .ok_or("room presentation missing admitted asset")?;
        binding.validate(loaded)?;
        let local = StaticInstance {
            translation: binding.transform.translation,
            rotation: binding.transform.rotation,
            scale: binding.transform.scale,
            material_override: binding.material_override,
        };
        let primitives = match binding.kind {
            ModelKind::Static => {
                let model = loaded
                    .static_model()
                    .ok_or("room presentation asset is not static")?;
                local.validate_for(model).map_err(|e| e.to_string())?;
                model.source().primitives.len()
            }
            ModelKind::Animated => {
                #[cfg(not(feature = "room-character"))]
                return Err("room presentation supports static models only".into());
                #[cfg(feature = "room-character")]
                {
                    let character = models
                        .character
                        .as_ref()
                        .ok_or("room animation requires a character declaration")?;
                    if guid != &character.player {
                        return Err(
                            "room character permits only one animated PLAYER binding".into()
                        );
                    }
                    let model = loaded
                        .animated_model()
                        .ok_or("room character asset is not animated")?;
                    character.validate_clips(model)?;
                    let pose = model.rest_pose().map_err(|e| e.to_string())?;
                    orr_render::SkinnedInstance {
                        pose: &pose,
                        transform: animated_transform(local),
                    }
                    .validate_for(model)
                    .map_err(|e| e.to_string())?;
                    model.source().primitives.len()
                }
            }
        };
        draws = draws
            .checked_add(primitives)
            .ok_or("room draw count overflow")?;
        if draws > orr_render::imported_scene::MAX_IMPORTED_DRAWS {
            return Err("room presentation exceeds imported draw limit".into());
        }
        used.insert(key);
    }
    if used.len() != models.assets.len() {
        return Err("room asset cache contains unbound assets".into());
    }
    crate::room_project::validate_decoded_model_budget(models.assets.values())?;
    Ok(())
}

fn validate_frame(
    frame: FrameView<'_>,
    index: &SceneIndex,
    models: &PreparedModels,
) -> Result<(), String> {
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
    #[cfg(feature = "room-character")]
    if let Some(character) = &models.character {
        let entity = index
            .entity(&Guid::parse(&character.player)?)
            .ok_or("room character PLAYER GUID missing")?;
        if frame
            .get::<RoomActor>(entity)
            .is_none_or(|actor| actor.kind != PLAYER)
            || frame
                .iter::<RoomActor>()
                .filter(|(_, actor)| actor.kind == PLAYER)
                .count()
                != 1
        {
            return Err("room character requires the exact sole PLAYER entity".into());
        }
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
    #[cfg(feature = "room-character")]
    if let Some(character) = &models.character {
        character.validate_bindings(frame, index, &models.document, &models.assets)?;
        character_placement(frame, index, models, PresentationTime::default())?;
    }
    for (guid, binding) in &models.document.bindings {
        let entity = index
            .entity(&Guid::parse(guid)?)
            .ok_or("missing admitted model actor")?;
        let actor = frame.get::<RoomActor>(entity).ok_or("missing room actor")?;
        if actor.kind != PLAYER {
            continue;
        }
        let body = frame.get::<Body>(entity).ok_or("missing room body")?;
        let loaded = &models.assets[&(binding.package.clone(), binding.asset.clone())];
        for x in [-16.0, 16.0] {
            for z in [-16.0, 16.0] {
                let mut pose = orr_view::fp_to_transform3(body.pos, body.rot);
                pose.pos.x = x;
                pose.pos.z = z;
                let placed = instance(pose, binding.transform, binding.material_override);
                if let Some(model) = loaded.static_model() {
                    placed
                        .validate_for(model)
                        .map_err(|e| format!("room reachable model placement: {e}"))?;
                }
                #[cfg(feature = "room-character")]
                if let Some(model) = loaded.animated_model() {
                    let character = models
                        .character
                        .as_ref()
                        .ok_or("missing room character declaration")?;
                    let transform = animated_transform(placed);
                    let rest = model.rest_pose().map_err(|e| e.to_string())?;
                    orr_render::SkinnedInstance {
                        pose: &rest,
                        transform,
                    }
                    .validate_for(model)
                    .map_err(|e| e.to_string())?;
                    for clip in [character.searching, character.carrying, character.escaped] {
                        let duration = model.source().clips[clip as usize].duration();
                        for time in [0.0, duration * 0.5, duration] {
                            let pose = model.sample_clip(clip, time).map_err(|e| e.to_string())?;
                            orr_render::SkinnedInstance {
                                pose: &pose,
                                transform,
                            }
                            .validate_for(model)
                            .map_err(|e| format!("room reachable character placement: {e}"))?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Static placements, validating the whole closed frame before resolving GUIDs.
/// Hidden collected-key assets are still admitted before visibility filtering.
pub fn placements(
    frame: FrameView<'_>,
    index: &SceneIndex,
    models: &PreparedModels,
) -> Result<Vec<ModelPlacement>, String> {
    validate_frame(frame, index, models)?;
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
        if binding.kind == ModelKind::Animated {
            continue;
        }
        let loaded = &models.assets[&(binding.package.clone(), binding.asset.clone())];
        let model = loaded.static_model().ok_or("room model is not static")?;
        let instance = instance(
            orr_view::fp_to_transform3(body.pos, body.rot),
            binding.transform,
            binding.material_override,
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

/// Current absolute-tick character pose and body × local placement, shared by
/// native, capture and editor. There is no remembered transition state.
#[cfg(feature = "room-character")]
pub fn character_placement(
    frame: FrameView<'_>,
    index: &SceneIndex,
    models: &PreparedModels,
    time: PresentationTime,
) -> Result<Option<CharacterPlacement>, String> {
    validate_frame(frame, index, models)?;
    let Some(character) = &models.character else {
        return Ok(None);
    };
    let guid = Guid::parse(&character.player)?;
    let entity = index
        .entity(&guid)
        .ok_or("room character PLAYER GUID missing")?;
    let body = frame
        .get::<Body>(entity)
        .ok_or("room character PLAYER body missing")?;
    let binding = &models.document.bindings[&character.player];
    let model = models.assets[&(binding.package.clone(), binding.asset.clone())]
        .animated_model()
        .ok_or("room character PLAYER asset is not animated")?;
    let pose =
        crate::room_character::sample_pose(character, model, frame, time.tick_rate, time.rest)?;
    let transform = animated_transform(instance(
        orr_view::fp_to_transform3(body.pos, body.rot),
        binding.transform,
        binding.material_override,
    ));
    let placement = CharacterPlacement {
        entity,
        model: model.clone(),
        pose,
        transform,
    };
    placement
        .instance()
        .validate_for(model)
        .map_err(|e| e.to_string())?;
    Ok(Some(placement))
}

struct CachedModel {
    model: Arc<orr_model::StaticModel>,
    renderer: ModelRenderer<Wgpu>,
    instances: Vec<StaticInstance>,
}

#[cfg(feature = "room-character")]
struct CachedCharacter {
    model: Arc<orr_model::animation::AnimatedModel>,
    renderer: orr_render::SkinnedModelRenderer<Wgpu>,
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
    #[cfg(feature = "room-character")]
    character: Option<CachedCharacter>,
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
        #[cfg(feature = "room-character")]
        let mut character_admission = Vec::new();
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
                    material_override: binding.material_override,
                })
                .collect();
            if let Some(model) = loaded.static_model() {
                admission.push((model, instances));
            } else {
                #[cfg(not(feature = "room-character"))]
                return Err("room cache requires static models".into());
                #[cfg(feature = "room-character")]
                {
                    let model = loaded
                        .animated_model()
                        .ok_or("room cache missing animated model")?;
                    let pose = model.rest_pose().map_err(|e| e.to_string())?;
                    // validate_models enforces one animated binding, not one
                    // asset reused by several unapproved entities.
                    if instances.len() != 1 {
                        return Err("room character requires one placement".into());
                    }
                    character_admission.push((model, pose, animated_transform(instances[0])));
                }
            }
        }
        let prepared: Vec<_> = admission
            .iter()
            .map(|(model, instances)| (model.as_ref(), instances.as_slice()))
            .collect();
        let mut empty = RenderList3D::new();
        empty.lighting = yard3d_view::yard_lighting();
        empty.lighting.shadows = false;
        #[cfg(not(feature = "room-character"))]
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
        #[cfg(feature = "room-character")]
        {
            let instances: Vec<_> = character_admission
                .iter()
                .map(|(_, pose, transform)| {
                    [orr_render::SkinnedInstance {
                        pose,
                        transform: *transform,
                    }]
                })
                .collect();
            let prepared_character: Vec<_> = character_admission
                .iter()
                .zip(&instances)
                .map(|((model, _, _), instances)| (model.as_ref(), instances.as_slice()))
                .collect();
            imported
                .preflight_mixed(
                    size,
                    &camera(size),
                    &empty.lighting,
                    &PointLightSettings::default(),
                    &empty,
                    &prepared,
                    &prepared_character,
                )
                .map_err(|e| e.to_string())?;
        }
        let mut cache = Vec::new();
        #[cfg(feature = "room-character")]
        let mut character_cache = None;
        for loaded in models.assets.values() {
            if let Some(model) = loaded.static_model() {
                cache.push(CachedModel {
                    model: model.clone(),
                    renderer: ModelRenderer::new(rhi.clone(), format, (**model).clone())
                        .map_err(|e| e.to_string())?,
                    instances: Vec::new(),
                });
            }
            #[cfg(feature = "room-character")]
            if let Some(model) = loaded.animated_model() {
                character_cache = Some(CachedCharacter {
                    model: model.clone(),
                    renderer: orr_render::SkinnedModelRenderer::new(
                        rhi.clone(),
                        format,
                        (**model).clone(),
                    )
                    .map_err(|e| e.to_string())?,
                });
            }
        }
        Ok(Self {
            target: OffscreenTarget::new(rhi, size.0, size.1, format),
            core: RoomRenderCore {
                format,
                procedural: Renderer3D::with_settings(rhi.clone(), format, Settings3D::LOW),
                imported,
                models: cache,
                #[cfg(feature = "room-character")]
                character: character_cache,
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
        self.render_with_time(frame, index, models, camera, PresentationTime::default())
    }

    pub fn render_with_time(
        &mut self,
        frame: FrameView<'_>,
        index: &SceneIndex,
        models: &PreparedModels,
        camera: &Camera3D,
        time: PresentationTime,
    ) -> Result<(), String> {
        let target = ImportedSceneTarget {
            view: self.target.render_view(),
            size: self.target.size(),
            format: self.core.format,
            sample_count: 1,
        };
        self.core.render(frame, index, models, camera, target, time)
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
        self.render_to_with_time(
            frame,
            index,
            models,
            camera,
            target,
            PresentationTime::default(),
        )
    }

    #[allow(clippy::too_many_arguments)] // Additive direct-target API with explicit presentation clock.
    pub fn render_to_with_time(
        &mut self,
        frame: FrameView<'_>,
        index: &SceneIndex,
        models: &PreparedModels,
        camera: &Camera3D,
        target: ImportedSceneTarget<'_, Wgpu>,
        time: PresentationTime,
    ) -> Result<(), String> {
        self.core.render(frame, index, models, camera, target, time)
    }
}

impl RoomRenderCore {
    #[allow(clippy::too_many_arguments)] // Shared direct/offscreen path retains target and clock explicitly.
    fn render(
        &mut self,
        frame: FrameView<'_>,
        index: &SceneIndex,
        models: &PreparedModels,
        camera: &Camera3D,
        target: ImportedSceneTarget<'_, Wgpu>,
        time: PresentationTime,
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
        if time.tick_rate == 0 {
            return Err("room presentation requires a nonzero tick rate".into());
        }
        let placed = placements(frame, index, models)?;
        #[cfg(feature = "room-character")]
        let character = character_placement(frame, index, models, time)?;
        // Check all cache identities, including collected-key assets, before
        // mutation. A reopened equivalent asset is still a new GPU identity.
        let cache_count = self.models.len();
        #[cfg(feature = "room-character")]
        let cache_count = cache_count + usize::from(self.character.is_some());
        if models.assets.len() != cache_count
            || models.assets.values().any(|loaded| {
                if let Some(model) = loaded.static_model() {
                    return !self
                        .models
                        .iter()
                        .any(|cached| Arc::ptr_eq(&cached.model, model));
                }
                #[cfg(feature = "room-character")]
                if let Some(model) = loaded.animated_model() {
                    return self
                        .character
                        .as_ref()
                        .is_none_or(|cached| !Arc::ptr_eq(&cached.model, model));
                }
                true
            })
        {
            return Err("room renderer asset identity changed; reopen renderer".into());
        }
        #[cfg(feature = "room-character")]
        if character.is_some() != self.character.is_some() {
            return Err("room renderer character identity changed; reopen renderer".into());
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
        #[cfg(feature = "room-character")]
        let bound = {
            let mut bound = bound;
            if let Some(character) = &character {
                bound.insert(character.entity);
            }
            bound
        };
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
        #[cfg(not(feature = "room-character"))]
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
        #[cfg(feature = "room-character")]
        let character_instances: Vec<_> =
            character.iter().map(CharacterPlacement::instance).collect();
        #[cfg(feature = "room-character")]
        {
            let prepared_character: Vec<_> = character
                .iter()
                .map(|placement| (placement.model.as_ref(), character_instances.as_slice()))
                .collect();
            self.imported
                .preflight_mixed(
                    target.size,
                    camera,
                    &list.lighting,
                    &PointLightSettings::default(),
                    &list,
                    &prepared,
                    &prepared_character,
                )
                .map_err(|e| e.to_string())?;
        }
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
        #[cfg(feature = "room-character")]
        if let Some(entry) = &mut self.character {
            batches.push(ImportedBatch::Skinned {
                renderer: &mut entry.renderer,
                instances: &character_instances,
            });
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
            None,
        );
        assert_eq!(result.translation, [2.0, 6.0, 2.0]);
        assert_eq!(result.rotation, [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(result.scale, [2.0, 3.0, 4.0]);
    }
    #[cfg(feature = "room-character")]
    mod character {
        use super::*;
        use orr_ecs::Frame;
        use orr_model_bindings::model_bindings::{
            self, AnimationDescriptor, Binding, Document, PlaybackMode,
        };
        use std::{collections::BTreeMap, fs, path::PathBuf};

        struct Fixture {
            _temp: tempfile::TempDir,
            root: PathBuf,
            frame: Frame,
            index: SceneIndex,
            models: PreparedModels,
            player: Entity,
            key: Entity,
        }
        impl Fixture {
            fn new() -> Self {
                let temp = tempfile::tempdir().unwrap();
                let base = temp.path().canonicalize().unwrap();
                let root = base.join("project");
                let source = base.join("occluder");
                fs::create_dir(&root).unwrap();
                fs::create_dir(&source).unwrap();
                // A real indexed static quad, in front of the courier when
                // viewed from +Z. The key binding hides this same verified
                // asset after collection, exercising mixed visibility.
                let model = orr_model::StaticModel::new(orr_model::ModelSource {
                    format: "orr_static_model".into(),
                    version: 1,
                    asset_id: "occluder.model.json".into(),
                    dependencies: vec![orr_model::Dependency {
                        uri: "$source".into(),
                        sha256: "a".repeat(64),
                    }],
                    materials: vec![orr_model::Material {
                        base_color: [0.9, 0.1, 0.1, 1.0],
                        image: 0,
                        linear_filter: false,
                        wrap_s: orr_model::Wrap::Clamp,
                        wrap_t: orr_model::Wrap::Clamp,
                    }],
                    images: vec![orr_model::Image {
                        width: 1,
                        height: 1,
                        rgba8: vec![255; 4],
                    }],
                    primitives: vec![orr_model::Primitive {
                        id: "occluder.model.json#node=0/mesh=0/primitive=0".into(),
                        material: 0,
                        transform: orr_model::IDENTITY,
                        vertices: [
                            [-3.0, -1.0, 1.0],
                            [3.0, -1.0, 1.0],
                            [3.0, 4.0, 1.0],
                            [-3.0, 4.0, 1.0],
                        ]
                        .into_iter()
                        .map(|position| orr_model::Vertex {
                            position,
                            normal: [0.0, 0.0, 1.0],
                            uv: [0.0; 2],
                        })
                        .collect(),
                        indices: vec![0, 1, 2, 0, 2, 3],
                    }],
                })
                .unwrap();
                fs::write(
                    source.join("occluder.model.json"),
                    model.to_bytes().unwrap(),
                )
                .unwrap();
                fs::write(source.join("orr.package.json"),br#"{"schema":1,"name":"room-occluder","version":"1.0.0","engine":"^0.0.1","capabilities":["models"],"dependencies":{},"files":["occluder.model.json"]}"#).unwrap();
                let courier = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../assets/room_character_demo")
                    .canonicalize()
                    .unwrap();
                orr_package::Project::open_for_install(
                    &root,
                    orr_package::Runtime::content_only().engine_version,
                )
                .unwrap()
                .install(&[courier, source])
                .unwrap();
                let animated = model_bindings::load_asset_for_kind(
                    &root,
                    "sample-room-character",
                    "courier.orrmodel.json",
                    ModelKind::Animated,
                )
                .unwrap();
                let static_model =
                    model_bindings::load_asset(&root, "room-occluder", "occluder.model.json")
                        .unwrap();
                let mut frame = Frame::new(Simulation::<RoomEscapeV1>::build_registry());
                RoomEscapeV1::setup(&mut frame, &RoomConfig);
                let player = frame
                    .entities()
                    .find(|&e| frame.get::<RoomActor>(e).unwrap().kind == PLAYER)
                    .unwrap();
                let key = frame
                    .entities()
                    .find(|&e| frame.get::<RoomActor>(e).unwrap().kind == KEY)
                    .unwrap();
                let mut index = SceneIndex::default();
                for (ordinal, entity) in frame.entities().enumerate() {
                    index.insert(Guid::from_u32(ordinal as u32 + 1), entity);
                }
                let player_guid = index.guid(player).unwrap().to_string();
                let key_guid = index.guid(key).unwrap().to_string();
                let bindings = BTreeMap::from([
                    (
                        player_guid.clone(),
                        Binding::from_animated_asset(
                            "sample-room-character".into(),
                            "courier.orrmodel.json".into(),
                            &animated,
                            AnimationDescriptor {
                                clip_index: 0,
                                playback: PlaybackMode::Loop,
                            },
                            LocalTransform::default(),
                        )
                        .unwrap(),
                    ),
                    (
                        key_guid,
                        Binding::from_asset(
                            "room-occluder".into(),
                            "occluder.model.json".into(),
                            &static_model,
                            LocalTransform {
                                translation: [-1.0, -0.5, 0.0],
                                ..LocalTransform::default()
                            },
                        )
                        .unwrap(),
                    ),
                ]);
                let models = PreparedModels {
                    path: root.join("room.models.json"),
                    document: Document {
                        version: 2,
                        scene: "room.scene.yaml".into(),
                        project: ".".into(),
                        bindings,
                    },
                    assets: BTreeMap::from([
                        (
                            (
                                "sample-room-character".into(),
                                "courier.orrmodel.json".into(),
                            ),
                            animated,
                        ),
                        (
                            ("room-occluder".into(), "occluder.model.json".into()),
                            static_model,
                        ),
                    ]),
                    character: Some(crate::room_character::Document {
                        schema: 1,
                        player: player_guid,
                        searching: 0,
                        carrying: 1,
                        escaped: 2,
                        speeds: None,
                    }),
                };
                admit_presentation(FrameView::of(&frame), &index, &models).unwrap();
                Self {
                    _temp: temp,
                    root,
                    frame,
                    index,
                    models,
                    player,
                    key,
                }
            }
            fn placement(&self, rest: bool) -> CharacterPlacement {
                character_placement(
                    FrameView::of(&self.frame),
                    &self.index,
                    &self.models,
                    PresentationTime {
                        tick_rate: 60,
                        rest,
                    },
                )
                .unwrap()
                .unwrap()
            }
        }

        #[test]
        fn authored_character_poses_are_read_only_and_share_exact_generation_placement() {
            let mut f = Fixture::new();
            // At tick30 the symmetric searching clip crosses the rest pose;
            // tick15 deliberately samples a non-rest quarter-key phase.
            f.frame.set_tick(15);
            let before = f.frame.to_bytes();
            let searching = f.placement(false);
            assert_eq!(searching.entity, f.player);
            assert_eq!(searching.transform[3][..3], [0.0, 0.5, 0.0]);
            assert_ne!(searching.pose, f.placement(true).pose);
            assert_eq!(searching.pose, f.placement(false).pose);
            assert_eq!(f.frame.to_bytes(), before);
            assert_eq!(
                placements(FrameView::of(&f.frame), &f.index, &f.models)
                    .unwrap()
                    .len(),
                1
            );
            f.frame.singleton_mut::<RoomRun>().key_collected = 1;
            let before = f.frame.to_bytes();
            let carrying = f.placement(false);
            assert_ne!(carrying.pose, searching.pose);
            assert!(placements(FrameView::of(&f.frame), &f.index, &f.models)
                .unwrap()
                .is_empty());
            assert!(f.frame.exists(f.key));
            assert_eq!(f.frame.to_bytes(), before);
            f.frame.singleton_mut::<RoomRun>().won = 1;
            let escaped = f.placement(false);
            assert_ne!(escaped.pose, carrying.pose);
            f.frame.singleton_mut::<RoomRun>().won = 0;
            f.frame.singleton_mut::<RoomRun>().key_collected = 0;
            assert_eq!(f.placement(false).pose, searching.pose);
            let player_guid = f.index.guid(f.player).unwrap().clone();
            f.frame.despawn(f.player);
            // Even if the old GUID text is unchanged, a recycled/missing
            // entity generation cannot drive this character.
            assert!(f.index.entity(&player_guid).is_some());
            assert!(character_placement(
                FrameView::of(&f.frame),
                &f.index,
                &f.models,
                PresentationTime::default()
            )
            .is_err());
        }

        #[test]
        fn character_requires_one_player_binding_and_every_mapped_clip() {
            let mut f = Fixture::new();
            let saved = f.models.character.clone();
            f.models.character = None;
            assert!(validate_models(&f.models).is_err());
            f.models.character = saved;
            let player = f.index.guid(f.player).unwrap().to_string();
            let key = f.index.guid(f.key).unwrap().to_string();
            let original = f.models.document.bindings[&player].clone();
            f.models
                .document
                .bindings
                .get_mut(&player)
                .unwrap()
                .animation
                .as_mut()
                .unwrap()
                .playback = PlaybackMode::Once;
            assert!(validate_models(&f.models).is_err());
            f.models
                .document
                .bindings
                .insert(player.clone(), original.clone());
            f.models.character.as_mut().unwrap().escaped = 31;
            assert!(validate_models(&f.models).is_err());
            f.models.character.as_mut().unwrap().escaped = 2;
            f.models.document.bindings.insert(key.clone(), original);
            assert!(validate_models(&f.models).is_err());
            f.models.document.bindings.remove(&key);
            f.models
                .assets
                .remove(&("room-occluder".into(), "occluder.model.json".into()));
            assert!(validate_models(&f.models).is_ok());
            f.models.character.as_mut().unwrap().player = key;
            assert!(character_placement(
                FrameView::of(&f.frame),
                &f.index,
                &f.models,
                PresentationTime::default()
            )
            .is_err());
        }

        #[test]
        #[ignore = "explicit mixed GPU acceptance: requires ORR_REQUIRE_GPU=1 and a working adapter"]
        fn mixed_character_depth_visibility_repeat_resize_and_failure_are_atomic() {
            use orr_render::orr_rhi::{Rhi, WgpuOptions};
            assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
            let mut f = Fixture::new();
            f.frame.set_tick(30);
            let gpu =
                Wgpu::headless(WgpuOptions::default()).expect("required GPU adapter unavailable");
            println!("Room character GPU adapter: {}",gpu.adapter_name());
            let size = (320, 240);
            let camera = Camera3D::orthographic([0.0, 1.0, 3.0], [0.0, 1.0, 0.0], 2.0);
            let mut renderer = RoomRenderer::new(&gpu, size, &f.models).unwrap();
            let player = f.index.guid(f.player).unwrap().to_string();
            let render = |renderer: &mut RoomRenderer, f: &Fixture| {
                let before = f.frame.to_bytes();
                renderer
                    .render(FrameView::of(&f.frame), &f.index, &f.models, &camera)
                    .unwrap();
                assert_eq!(f.frame.to_bytes(), before);
                renderer.read_rgba8()
            };
            let occluded = render(&mut renderer, &f);
            f.models
                .document
                .bindings
                .get_mut(&player)
                .unwrap()
                .transform
                .translation = [1000.0, 0.0, 0.0];
            assert_eq!(
                occluded,
                render(&mut renderer, &f),
                "rear skinned geometry must not overwrite the static foreground quad"
            );
            f.frame.singleton_mut::<RoomRun>().key_collected = 1;
            let no_character = render(&mut renderer, &f);
            f.models
                .document
                .bindings
                .get_mut(&player)
                .unwrap()
                .transform
                .translation = [0.0; 3];
            let carrying = render(&mut renderer, &f);
            assert!(carrying.chunks_exact(4).zip(no_character.chunks_exact(4)).filter(|(a,b)| a!=b).count()>20,"animated character must contribute visible pixels after the key occluder disappears");
            assert_ne!(carrying, occluded);
            assert_eq!(carrying, render(&mut renderer, &f));
            f.frame.singleton_mut::<RoomRun>().won = 1;
            let escaped = render(&mut renderer, &f);
            assert_ne!(
                escaped, carrying,
                "authored state clip must change visible pose"
            );
            // An invalid future state map is rejected even while another state
            // is currently visible; pixels and current GPU bounds are retained.
            f.models.character.as_mut().unwrap().escaped = 31;
            assert!(renderer
                .render(FrameView::of(&f.frame), &f.index, &f.models, &camera)
                .is_err());
            assert_eq!(renderer.read_rgba8(), escaped);
            f.models.character.as_mut().unwrap().escaped = 2;
            let key = ("room-occluder".into(), "occluder.model.json".into());
            let old = f.models.assets[&key].clone();
            let reopened =
                model_bindings::load_asset(&f.root, "room-occluder", "occluder.model.json")
                    .unwrap();
            f.models.assets.insert(key.clone(), reopened);
            assert!(
                renderer
                    .render(FrameView::of(&f.frame), &f.index, &f.models, &camera)
                    .is_err(),
                "hidden-key asset identity must still match the GPU cache"
            );
            assert_eq!(renderer.read_rgba8(), escaped);
            f.models.assets.insert(key, old);
            renderer.resize((240, 320)).unwrap();
            let resized = render(&mut renderer, &f);
            assert_eq!(resized.len(), 240 * 320 * 4);
            assert_eq!(resized, render(&mut renderer, &f));
        }
    }
}
