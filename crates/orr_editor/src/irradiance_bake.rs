//! Owned, bounded presentation snapshots and one cancellable CPU bake worker.
//! Nothing here can write a host frame, a scene, a package, or an irradiance
//! document. The panel alone admits a completed result after its revision guard.
//! The 64 MiB snapshot admission counts its struct and every retained buffer's
//! capacity, including strings, paths, textures, grid and verification metadata.
//! It excludes allocator bookkeeping, thread stacks and separately bounded
//! temporary capture maps, BVH/tracing buffers and file-verification stamps.
use crate::{
    backend::HostSpec,
    editor::Editor,
    game::EditorGame,
    irradiance_bindings::BakeReceipt,
    model::Mode,
    model_bindings::{Binding, ModelKind},
    model_panel::ModelPanel,
};
use orr_render::{
    irradiance::{IrradianceGrid, IrradianceProvenance},
    irradiance_bake::{
        self as cpu, BakeChecker, BakeFilter, BakeMaterial, BakeScene, BakeSettings, BakeSun,
        BakeTexture, BakeTriangle, BakeWrap,
    },
    math3::{self, Mat4},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Instant, SystemTime},
};

pub const BAKE_ALGORITHM_VERSION: u32 = cpu::BAKE_ALGORITHM_VERSION;
pub const MAX_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_TEXTURE_BYTES: usize = 32 * 1024 * 1024;
const MAX_TRIANGLES: usize = 8192;
const MAX_PARTICIPANTS: usize = 4096;
const MAX_FRAME_BODIES: usize = 65536;
const MAX_ASSET_FILES: usize = 4096;
const MAX_ASSET_BYTES: u64 = 256 * 1024 * 1024;
static WORKER_ACTIVE: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BakeCost {
    pub triangles: u32,
    pub participants: u32,
    pub excluded: u32,
    pub probes: u32,
    pub directions: u32,
    /// Retained snapshot payload capacity, excluding allocator bookkeeping and
    /// separately bounded temporary/worker allocations.
    pub snapshot_bytes: u64,
    pub max_primary_rays: u64,
}

#[derive(Clone)]
struct AssetCheck {
    root: PathBuf,
    binding: Binding,
    /// Source glTF dependencies are actual package files. A cooked model's
    /// embedded source digests are provenance; its own bytes cover its images.
    dependencies: Vec<(String, String)>,
}

/// No borrowed frame, mutable renderer, host handle, or editor state crosses the
/// worker boundary. Geometry and linear material factors are already world-space.
pub struct BakeSceneSnapshot {
    pub fingerprint: String,
    /// Runtime file authority only, separate from portable content SHA256.
    pub source_identity: String,
    pub scene_path: PathBuf,
    pub cost: BakeCost,
    scene: BakeScene,
    grid: IrradianceGrid,
    settings: BakeSettings,
    assets: Vec<AssetCheck>,
}

impl BakeSceneSnapshot {
    pub fn capture(
        editor: &Editor,
        models: &ModelPanel,
        grid: &IrradianceGrid,
        settings: &BakeSettings,
    ) -> Result<Self, String> {
        Self::capture_with_cancel(editor, models, grid, settings, &AtomicBool::new(false))
    }

    pub fn capture_with_cancel(
        editor: &Editor,
        models: &ModelPanel,
        grid: &IrradianceGrid,
        settings: &BakeSettings,
        cancel: &AtomicBool,
    ) -> Result<Self, String> {
        capture(editor, models, grid, settings, cancel, true)
    }

    /// Rendering may continue in Play. Only BODY_STATIC participants enter the
    /// hash, so motion of dynamic/kinematic/animated entities cannot stale it.
    pub fn capture_for_validation(
        editor: &Editor,
        models: &ModelPanel,
        grid: &IrradianceGrid,
        settings: &BakeSettings,
    ) -> Result<Self, String> {
        capture(
            editor,
            models,
            grid,
            settings,
            &AtomicBool::new(false),
            false,
        )
    }

    /// Cheap process-local cache key, not the portable bake receipt. It reads
    /// static input fields and immutable loaded-asset identity only: no triangle
    /// generation, texture copies or model-byte hashes. Tick, camera, selection,
    /// exposure and excluded dynamic/kinematic/animated changes are absent.
    pub fn participating_input_key(
        editor: &Editor,
        models: &ModelPanel,
        grid: &IrradianceGrid,
        settings: &BakeSettings,
    ) -> Result<String, String> {
        participating_input_key(editor, models, grid, settings)
    }

    pub fn grid(&self) -> &IrradianceGrid {
        &self.grid
    }
    pub fn settings(&self) -> &BakeSettings {
        &self.settings
    }

    fn retained_bytes(&self) -> Result<u64, String> {
        let mut bytes = std::mem::size_of::<Self>() as u64;
        let mut allocation = |capacity: usize, element_bytes: usize| -> Result<(), String> {
            let extra = capacity
                .checked_mul(element_bytes)
                .ok_or("Bake snapshot allocation size overflow")?;
            bytes = bytes
                .checked_add(
                    u64::try_from(extra).map_err(|_| "Bake snapshot allocation size overflow")?,
                )
                .ok_or("Bake snapshot allocation size overflow")?;
            Ok(())
        };
        allocation(self.fingerprint.capacity(), 1)?;
        allocation(self.source_identity.capacity(), 1)?;
        allocation(self.scene_path.capacity(), 1)?;
        allocation(
            self.scene.triangles.capacity(),
            std::mem::size_of::<BakeTriangle>(),
        )?;
        allocation(
            self.scene.materials.capacity(),
            std::mem::size_of::<BakeMaterial>(),
        )?;
        for material in &self.scene.materials {
            if let Some(texture) = &material.texture {
                allocation(texture.rgba8_srgb.capacity(), 1)?;
            }
        }
        allocation(
            self.grid.coefficients.capacity(),
            std::mem::size_of::<orr_render::irradiance::Sh9>(),
        )?;
        allocation(self.assets.capacity(), std::mem::size_of::<AssetCheck>())?;
        for asset in &self.assets {
            allocation(asset.root.capacity(), 1)?;
            allocation(asset.binding.package.capacity(), 1)?;
            allocation(asset.binding.asset.capacity(), 1)?;
            allocation(asset.binding.package_digest.capacity(), 1)?;
            allocation(asset.binding.source_hash.capacity(), 1)?;
            allocation(
                asset.dependencies.capacity(),
                std::mem::size_of::<(String, String)>(),
            )?;
            for (path, digest) in &asset.dependencies {
                allocation(path.capacity(), 1)?;
                allocation(digest.capacity(), 1)?;
            }
        }
        Ok(bytes)
    }
}

fn participating_input_key(
    editor: &Editor,
    models: &ModelPanel,
    grid: &IrradianceGrid,
    settings: &BakeSettings,
) -> Result<String, String> {
    if editor.game() != EditorGame::Yard3D
        || !editor.spec().is_local()
        || editor.previewing().is_some()
        || !editor.yard_rows_coherent()
    {
        return Err("Bake input key requires coherent local Yard3D without a preview".into());
    }
    let expected: &std::path::Path = match editor.spec() {
        HostSpec::Local { scene, .. } | HostSpec::LocalGame { scene, .. } => scene,
        #[cfg(feature = "sprites")]
        HostSpec::PreparedArena { scene, .. } => scene.path(),
        #[cfg(feature = "collect-dodge")]
        HostSpec::PreparedCollect { .. } => return Err("CollectDodge does not support local 3D asset authoring".into()),
        HostSpec::Remote { .. } => return Err("Remote scene has no local bake input key".into()),
    };
    if editor.sim().scene_path.as_deref() != expected.to_str() {
        return Err("Wait for the local scene path to synchronize".into());
    }
    let frame = editor
        .snapshot()
        .ok_or("Bake input key requires a frame")?
        .predicted();
    if frame.alive_count() as usize > MAX_FRAME_BODIES || editor.rows().len() > MAX_FRAME_BODIES {
        return Err("Bake input key exceeds the bounded body map".into());
    }
    grid.validate()?;
    let rows: BTreeMap<_, _> = editor
        .rows()
        .iter()
        .map(|row| ((row.entity.index, row.entity.version), row))
        .collect();
    let placements = models.placements(editor);
    let placements: BTreeMap<_, _> = placements
        .iter()
        .map(|p| ((p.entity.index, p.entity.version), p))
        .collect();
    let bindings = models
        .bindings
        .as_ref()
        .filter(|_| models.scene_matches(editor));
    let mut participants = Vec::new();
    for body in orr_sample::yard3d_view::editor_static_bake_bodies(frame)? {
        let key = (body.entity.index, body.entity.version);
        let guid = rows
            .get(&key)
            .and_then(|row| row.guid.as_ref())
            .map(ToString::to_string);
        let binding = guid
            .as_ref()
            .and_then(|guid| bindings.and_then(|b| b.document().bindings.get(guid)));
        if binding.is_some_and(|b| b.kind == ModelKind::Animated) {
            continue;
        }
        if binding.is_none() && body.item.is_none() {
            continue;
        }
        let guid = guid.ok_or("A rendered static body lacks a persistent GUID")?;
        if participants.len() == MAX_PARTICIPANTS {
            return Err("Bake input key exceeds 4096 static participants".into());
        }
        participants.push((guid, body, binding));
    }
    participants.sort_by(|a, b| a.0.cmp(&b.0));
    if participants.windows(2).any(|p| p[0].0 == p[1].0) {
        return Err("Duplicate static participant GUID".into());
    }
    let model_root = if participants.iter().any(|(_, _, binding)| binding.is_some()) {
        Some(
            bindings
                .ok_or("Missing static model sidecar")?
                .project_root()?,
        )
    } else {
        None
    };
    let mut hash = CanonicalHash::new();
    hash.string("orrery-static-bake-runtime-input-key");
    hash.u32(BAKE_ALGORITHM_VERSION);
    hash.u32(orr_render::Settings3D::LOW.mesh_segments);
    hash.bytes(expected.as_os_str().as_encoded_bytes());
    hash.u64(cpu::MIN_RAY_OFFSET.to_bits());
    hash.u64(cpu::RAY_OFFSET_FACTOR.to_bits());
    hash.u32(settings.rays_per_probe);
    hash.u64(settings.max_triangle_tests);
    hash.u64(settings.max_node_visits);
    hash.u64(
        u64::try_from(settings.max_duration.as_nanos())
            .map_err(|_| "Bake duration exceeds bounded key")?,
    );
    for dimension in grid.dimensions {
        hash.u32(dimension);
    }
    hash.floats(&grid.origin);
    hash.floats(&grid.spacing);
    let light = orr_sample::yard3d_view::yard_lighting();
    hash.floats(&light.direction);
    hash.floats(&light.color);
    hash.float(light.intensity);
    hash.u32(participants.len() as u32);
    for (guid, body, binding) in participants {
        hash.string(&guid);
        hash.floats(&body.pose.pos.to_array());
        hash.floats(&body.pose.rot.to_array());
        if let Some(binding) = binding {
            hash.u32(1);
            let placement = placements
                .get(&(body.entity.index, body.entity.version))
                .ok_or_else(|| format!("Static model {guid} is unresolved"))?;
            let sidecar = bindings.ok_or("Missing static model sidecar")?;
            hash.bytes(sidecar.path.as_os_str().as_encoded_bytes());
            hash.string(&sidecar.document().project);
            hash.bytes(
                model_root
                    .as_ref()
                    .ok_or("Missing static model root")?
                    .as_os_str()
                    .as_encoded_bytes(),
            );
            hash.string(&binding.package);
            hash.string(&binding.asset);
            hash.string(&binding.package_digest);
            hash.string(&binding.source_hash);
            hash.floats(&binding.transform.translation);
            hash.floats(&binding.transform.rotation);
            hash.floats(&binding.transform.scale);
            hash.floats(&placement.instance.translation);
            hash.floats(&placement.instance.rotation);
            hash.floats(&placement.instance.scale);
            // Loaded models are immutable. An Arc replacement invalidates this
            // runtime cache even if the binding still names the same source.
            // Pointer identity is deliberately absent from persisted SHA256.
            hash.u64(Arc::as_ptr(&placement.model) as usize as u64);
        } else {
            hash.u32(0);
            if !body.supported_shape {
                return Err("Unsupported static collider geometry".into());
            }
            let item = body.item.ok_or("Missing static procedural render item")?;
            hash.floats(&item.transform.pos.to_array());
            hash.floats(&item.transform.rot.to_array());
            match item.style.shape {
                orr_view::Shape3::Box { half } => {
                    hash.u32(0);
                    hash.floats(&half);
                }
                orr_view::Shape3::Sphere { radius } => {
                    hash.u32(1);
                    hash.float(radius);
                }
                orr_view::Shape3::Capsule {
                    half_length,
                    radius,
                } => {
                    hash.u32(2);
                    hash.float(half_length);
                    hash.float(radius);
                }
                orr_view::Shape3::Plane { half_x, half_z } => {
                    hash.u32(3);
                    hash.float(half_x);
                    hash.float(half_z);
                }
            }
            hash.floats(&item.style.color);
            hash.float(item.style.roughness);
            hash.float(item.style.metallic);
            hash.u32(u32::from(item.style.checker));
        }
    }
    Ok(hash.finish())
}

fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Static diffuse bake cancelled".into())
    } else {
        Ok(())
    }
}

fn capture(
    editor: &Editor,
    models: &ModelPanel,
    grid: &IrradianceGrid,
    settings: &BakeSettings,
    cancel: &AtomicBool,
    require_edit: bool,
) -> Result<BakeSceneSnapshot, String> {
    cancelled(cancel)?;
    if editor.game() != EditorGame::Yard3D
        || !editor.spec().is_local()
        || editor.previewing().is_some()
        || !editor.yard_rows_coherent()
        || (require_edit && (editor.mode() != Mode::Edit || !editor.can_mutate()))
    {
        return Err("Static diffuse bake requires a coherent local Yard3D scene without preview; start and completion require Edit mode".into());
    }
    let expected: &std::path::Path = match editor.spec() {
        HostSpec::Local { scene, .. } | HostSpec::LocalGame { scene, .. } => scene,
        #[cfg(feature = "sprites")]
        HostSpec::PreparedArena { scene, .. } => scene.path(),
        #[cfg(feature = "collect-dodge")]
        HostSpec::PreparedCollect { .. } => return Err("CollectDodge does not support local 3D asset authoring".into()),
        HostSpec::Remote { .. } => return Err("Remote paths cannot authorize a bake".into()),
    };
    if editor.sim().scene_path.as_deref() != expected.to_str() {
        return Err("Wait for the local scene path to synchronize before baking".into());
    }
    let scene_path = expected
        .canonicalize()
        .map_err(|e| format!("Bake scene: {e}"))?;
    grid.validate()?;
    if !(64..=2048).contains(&settings.rays_per_probe) || !settings.rays_per_probe.is_multiple_of(2)
    {
        return Err("Static diffuse bake supports an even 64..=2048 directions per probe".into());
    }
    if settings.max_triangle_tests == 0
        || settings.max_triangle_tests > cpu::MAX_BAKE_TRIANGLE_TESTS
        || settings.max_node_visits == 0
        || settings.max_node_visits > cpu::MAX_BAKE_NODE_VISITS
        || settings.max_duration.is_zero()
        || settings.max_duration > cpu::MAX_BAKE_DURATION
    {
        return Err("Static diffuse bake execution budgets are outside bounded limits".into());
    }
    let frame = editor
        .snapshot()
        .ok_or("Bake requires a local frame")?
        .predicted();
    if frame.alive_count() as usize > MAX_FRAME_BODIES || editor.rows().len() > MAX_FRAME_BODIES {
        return Err("Bake scene exceeds the bounded body map".into());
    }
    let guid_by_entity: BTreeMap<_, _> = editor
        .rows()
        .iter()
        .filter_map(|row| {
            row.guid
                .as_ref()
                .map(|guid| ((row.entity.index, row.entity.version), guid.to_string()))
        })
        .collect();
    // The panel's placements are the exact already-verified immutable assets
    // rendered by this viewport. Missing static bindings are rejected, never
    // silently replaced by a collider. Disk identities are rechecked in worker.
    let placements = models.placements(editor);
    let placements: BTreeMap<_, _> = placements
        .iter()
        .map(|p| ((p.entity.index, p.entity.version), p))
        .collect();
    let bindings = models
        .bindings
        .as_ref()
        .filter(|_| models.scene_matches(editor));
    let mut candidates = Vec::new();
    let mut excluded = 0_u32;
    for body in orr_sample::yard3d_view::editor_bake_bodies(frame)? {
        cancelled(cancel)?;
        if !body.is_static {
            excluded += 1;
            continue;
        }
        let guid = guid_by_entity.get(&(body.entity.index, body.entity.version));
        let binding = guid.and_then(|guid| bindings.and_then(|b| b.document().bindings.get(guid)));
        if binding.is_some_and(|b| b.kind == ModelKind::Animated) {
            excluded += 1;
            continue;
        }
        if binding.is_none() && body.item.is_none() {
            continue;
        }
        let guid = guid
            .ok_or("A rendered static body lacks a persistent GUID")?
            .clone();
        if candidates.len() == MAX_PARTICIPANTS {
            return Err("Bake exceeds 4096 static participants".into());
        }
        candidates.push((guid, body, binding));
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    if candidates.windows(2).any(|p| p[0].0 == p[1].0) {
        return Err("Duplicate static participant GUID".into());
    }
    let lighting = orr_sample::yard3d_view::yard_lighting();
    let mut scene = BakeScene {
        triangles: Vec::new(),
        materials: Vec::new(),
        sun: BakeSun {
            direction: math3::normalize(lighting.direction.map(|v| -v)),
            color: lighting.color,
            intensity: lighting.intensity,
        },
    };
    let mut identity = CanonicalHash::new();
    identity.string("orrery-static-sun-diffuse-snapshot");
    identity.u32(BAKE_ALGORITHM_VERSION);
    identity.u32(orr_render::Settings3D::LOW.mesh_segments);
    let mut assets = Vec::new();
    let mut asset_keys = BTreeSet::new();
    let mut metadata_bytes = 0_usize;
    let mut texture_bytes = 0_usize;
    identity.u32(candidates.len() as u32);
    for (guid, body, binding) in &candidates {
        cancelled(cancel)?;
        identity.string(guid);
        if let Some(binding) = binding {
            let placement = placements
                .get(&(body.entity.index, body.entity.version))
                .ok_or_else(|| {
                    format!(
                        "Static model {guid} is unresolved; bake cannot substitute its collider"
                    )
                })?;
            binding.transform.validate()?;
            placement
                .instance
                .validate_for(&placement.model)
                .map_err(|e| e.to_string())?;
            identity.u32(1);
            identity.string(&binding.package);
            identity.string(&binding.asset);
            identity.string(&binding.package_digest);
            identity.string(&binding.source_hash);
            identity.floats(&binding.transform.translation);
            identity.floats(&binding.transform.rotation);
            identity.floats(&binding.transform.scale);
            let source = placement.model.source();
            let mut dependencies: Vec<_> = source.dependencies.iter().collect();
            dependencies.sort_by(|a, b| a.uri.cmp(&b.uri));
            identity.u32(dependencies.len() as u32);
            for dependency in &dependencies {
                identity.string(&dependency.uri);
                identity.string(&dependency.sha256);
            }
            if asset_keys.insert((
                binding.package.clone(),
                binding.asset.clone(),
                binding.package_digest.clone(),
            )) {
                let root = bindings.ok_or("Missing model sidecar")?.project_root()?;
                metadata_bytes = metadata_bytes
                    .checked_add(
                        dependencies
                            .iter()
                            .map(|d| d.uri.len() + d.sha256.len())
                            .sum::<usize>()
                            + binding.package.len()
                            + binding.asset.len()
                            + binding.package_digest.len()
                            + binding.source_hash.len()
                            + root.capacity()
                            + std::mem::size_of::<AssetCheck>()
                            + dependencies.len() * std::mem::size_of::<(String, String)>(),
                    )
                    .ok_or("Bake asset metadata overflow")?;
                if metadata_bytes > 4 * 1024 * 1024 {
                    return Err("Bake asset metadata exceeds 4 MiB".into());
                }
                let checks = if dependencies
                    .iter()
                    .any(|d| d.uri == "$source" && d.sha256 == binding.source_hash)
                {
                    dependencies
                        .into_iter()
                        .filter(|d| d.uri != "$source")
                        .map(|d| (d.uri.clone(), d.sha256.clone()))
                        .collect()
                } else {
                    Vec::new()
                };
                assets.push(AssetCheck {
                    root,
                    binding: (*binding).clone(),
                    dependencies: checks,
                });
            }
            append_model(&mut scene, placement, &mut texture_bytes, cancel)?;
        } else {
            identity.u32(0);
            if !body.supported_shape {
                return Err(format!(
                    "Static participant {guid} has unsupported collider geometry"
                ));
            }
            append_procedural(
                &mut scene,
                body.item.as_ref().ok_or("Missing procedural mesh")?,
                cancel,
            )?;
        }
    }
    // All variable-size buffers were admitted before allocation. Hashing is
    // field-by-field and endian-explicit, never Debug output or generic JSON.
    fingerprint_scene(&mut identity, &scene, grid, settings, cancel)?;
    let probes = grid.dimensions.iter().product::<u32>();
    let cost = BakeCost {
        triangles: scene.triangles.len() as u32,
        participants: candidates.len() as u32,
        excluded,
        probes,
        directions: settings.rays_per_probe,
        snapshot_bytes: 0,
        max_primary_rays: u64::from(probes) * u64::from(settings.rays_per_probe),
    };
    let mut snapshot = BakeSceneSnapshot {
        fingerprint: identity.finish(),
        source_identity: source_location_identity(&assets)?,
        scene_path,
        cost,
        scene,
        grid: grid.clone(),
        settings: settings.clone(),
        assets,
    };
    snapshot.cost.snapshot_bytes = snapshot.retained_bytes()?;
    if snapshot.cost.snapshot_bytes > MAX_SNAPSHOT_BYTES {
        return Err("Bake retained snapshot buffer capacity exceeds 64 MiB".into());
    }
    Ok(snapshot)
}

/// Local authority changes even when a retargeted package has identical bytes.
/// This hash covers exactly the source and lock paths checked by file stamps.
fn source_location_identity(assets: &[AssetCheck]) -> Result<String, String> {
    let mut paths = BTreeSet::new();
    for asset in assets {
        paths.insert(asset.root.join("orr.packages.lock.json"));
        let object = asset
            .root
            .join(".orr/packages/objects")
            .join(&asset.binding.package_digest);
        paths.insert(object.join(&asset.binding.asset));
        for (uri, _) in &asset.dependencies {
            let parent = Path::new(&asset.binding.asset)
                .parent()
                .unwrap_or(Path::new(""));
            let relative = parent.join(uri);
            if relative
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            {
                return Err("Unsupported baked model dependency path".into());
            }
            paths.insert(object.join(relative));
            if paths.len() > MAX_ASSET_FILES + MAX_PARTICIPANTS {
                return Err("Bake source-location identity exceeds bounded file count".into());
            }
        }
        if paths.len() > MAX_ASSET_FILES + MAX_PARTICIPANTS {
            return Err("Bake source-location identity exceeds bounded file count".into());
        }
    }
    let mut hash = CanonicalHash::new();
    hash.string("orrery-static-bake-local-source-authority");
    hash.u32(paths.len() as u32);
    for path in paths {
        hash.bytes(path.as_os_str().as_encoded_bytes());
    }
    Ok(hash.finish())
}

fn append_procedural(
    scene: &mut BakeScene,
    item: &orr_view::RenderItem3,
    cancel: &AtomicBool,
) -> Result<(), String> {
    use orr_view::Shape3;
    let segments = orr_render::Settings3D::LOW.mesh_segments;
    let (mesh, scale, half_length, rotation) = match item.style.shape {
        Shape3::Box { half } => (
            orr_render::mesh::cuboid(),
            half,
            0.0,
            item.transform.rot.to_array(),
        ),
        Shape3::Sphere { radius } => (
            orr_render::mesh::sphere_with(segments),
            [radius; 3],
            0.0,
            item.transform.rot.to_array(),
        ),
        Shape3::Capsule {
            half_length,
            radius,
        } => (
            orr_render::mesh::capsule_with(segments),
            [radius; 3],
            half_length,
            item.transform.rot.to_array(),
        ),
        // RenderList3D::plane deliberately uses identity rotation.
        Shape3::Plane { half_x, half_z } => (
            orr_render::mesh::plane(),
            [half_x, 1.0, half_z],
            0.0,
            orr_render::IDENTITY_ROT,
        ),
    };
    if !scale.iter().all(|s| s.is_finite() && *s > 0.0)
        || !half_length.is_finite()
        || half_length < 0.0
    {
        return Err("Unsupported nonpositive procedural mesh dimensions".into());
    }
    reserve_triangles(scene, mesh.indices.len() / 3)?;
    let style = &item.style;
    if style.color[3] != 1.0
        || !style.metallic.is_finite()
        || !(0.0..=1.0).contains(&style.metallic)
    {
        return Err("Bake only supports opaque procedural diffuse materials".into());
    }
    let material = scene.materials.len();
    scene.materials.push(BakeMaterial {
        base_color: std::array::from_fn(|i| style.color[i] * (1.0 - style.metallic)),
        texture: None,
        checker: style.checker.then_some(BakeChecker {
            scale: 0.5,
            dark_multiplier: 0.72,
        }),
        double_sided: false,
    });
    let position = item.transform.pos.to_array();
    for tri in mesh.indices.as_chunks::<3>().0.iter() {
        cancelled(cancel)?;
        let vertices = tri.map_vertices(&mesh.vertices);
        let positions = vertices.map(|v| {
            let mut local = std::array::from_fn(|i| v.pos[i] * scale[i]);
            local[1] += v.cap * half_length;
            math3::add(position, math3::quat_rotate(rotation, local))
        });
        let normals = vertices.map(|v| {
            math3::quat_rotate(
                rotation,
                math3::normalize(std::array::from_fn(|i| v.normal[i] / scale[i].max(1.0e-6))),
            )
        });
        push_triangle(
            scene,
            BakeTriangle {
                positions,
                normals,
                texcoords: [[0.0; 2]; 3],
                material,
            },
        )?;
    }
    Ok(())
}

trait TriangleVertices<T: Copy> {
    fn map_vertices(&self, vertices: &[T]) -> [T; 3];
}
impl<T: Copy> TriangleVertices<T> for [u32] {
    fn map_vertices(&self, vertices: &[T]) -> [T; 3] {
        [
            vertices[self[0] as usize],
            vertices[self[1] as usize],
            vertices[self[2] as usize],
        ]
    }
}

fn reserve_triangles(scene: &BakeScene, count: usize) -> Result<(), String> {
    if count > MAX_TRIANGLES.saturating_sub(scene.triangles.len()) {
        Err(
            "Static diffuse bake exceeds 8192 rendered triangles; no truncated bake was started"
                .into(),
        )
    } else {
        Ok(())
    }
}

fn push_triangle(scene: &mut BakeScene, triangle: BakeTriangle) -> Result<(), String> {
    if !triangle
        .positions
        .iter()
        .flatten()
        .all(|v| v.is_finite() && v.abs() <= 1.0e6)
        || !triangle.normals.iter().flatten().all(|v| v.is_finite())
    {
        return Err("Static geometry exceeds finite bounded world coordinates".into());
    }
    let a: [f64; 3] = std::array::from_fn(|i| {
        f64::from(triangle.positions[1][i]) - f64::from(triangle.positions[0][i])
    });
    let b: [f64; 3] = std::array::from_fn(|i| {
        f64::from(triangle.positions[2][i]) - f64::from(triangle.positions[0][i])
    });
    let cross = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    // Only exactly zero-area raster triangles are omitted, without a scale-
    // dependent threshold that would decimate small but valid source geometry.
    if cross.iter().any(|&v| v != 0.0) {
        scene.triangles.push(triangle);
    }
    Ok(())
}

fn append_model(
    scene: &mut BakeScene,
    placement: &crate::viewport3d::ModelPlacement,
    texture_bytes: &mut usize,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let model = placement.model.source();
    let count = model
        .primitives
        .iter()
        .try_fold(0_usize, |sum, p| sum.checked_add(p.indices.len() / 3))
        .ok_or("Model triangle count overflow")?;
    reserve_triangles(scene, count)?;
    let external = instance_matrix(placement.instance);
    let mut materials = BTreeMap::new();
    for primitive in &model.primitives {
        cancelled(cancel)?;
        let material = if let Some(&index) = materials.get(&primitive.material) {
            index
        } else {
            let input = &model.materials[primitive.material as usize];
            let image = &model.images[input.image as usize];
            let next_bytes = texture_bytes
                .checked_add(image.rgba8.len())
                .ok_or("Texture byte count overflow")?;
            if next_bytes > MAX_TEXTURE_BYTES {
                return Err("Bake textures exceed the 32 MiB owned snapshot budget".into());
            }
            *texture_bytes = next_bytes;
            let index = scene.materials.len();
            // The existing opaque model pipeline ignores texture and factor
            // alpha. It has no supported alpha-mask/blend or normal-map modes.
            scene.materials.push(BakeMaterial {
                base_color: [
                    input.base_color[0],
                    input.base_color[1],
                    input.base_color[2],
                ],
                texture: Some(BakeTexture {
                    width: image.width,
                    height: image.height,
                    rgba8_srgb: image.rgba8.clone(),
                    wrap_s: wrap(input.wrap_s),
                    wrap_t: wrap(input.wrap_t),
                    filter: if input.linear_filter {
                        BakeFilter::Linear
                    } else {
                        BakeFilter::Nearest
                    },
                }),
                checker: None,
                double_sided: false,
            });
            materials.insert(primitive.material, index);
            index
        };
        let world = external.mul(&Mat4(primitive.transform));
        let normal = orr_model::normal_matrix(world.0).map_err(|e| e.to_string())?;
        for indices in primitive.indices.as_chunks::<3>().0.iter() {
            cancelled(cancel)?;
            let mut vertices = indices.map_vertices(&primitive.vertices);
            // ModelRenderer uploads flipped winding for mirrored imported
            // nodes. External instance scale is positive by admission.
            if orr_model::determinant(primitive.transform) < 0.0 {
                vertices.swap(1, 2);
            }
            let positions = vertices.map(|v| {
                let p = world.transform_point4(v.position);
                [p[0], p[1], p[2]]
            });
            // Preserve inverse-transpose magnitudes until interpolation, exactly
            // as shader_model.wgsl does under nonuniform scale and shear.
            let normals = vertices.map(|v| {
                std::array::from_fn(|r| {
                    normal[0][r] * v.normal[0]
                        + normal[1][r] * v.normal[1]
                        + normal[2][r] * v.normal[2]
                })
            });
            push_triangle(
                scene,
                BakeTriangle {
                    positions,
                    normals,
                    texcoords: vertices.map(|v| v.uv),
                    material,
                },
            )?;
        }
    }
    Ok(())
}

fn instance_matrix(instance: orr_render::StaticInstance) -> Mat4 {
    let length = instance.rotation.iter().map(|v| v * v).sum::<f32>().sqrt();
    let q = instance.rotation.map(|v| v / length);
    let mut m = Mat4::IDENTITY;
    for axis in 0..3 {
        let mut basis = [0.0; 3];
        basis[axis] = instance.scale[axis];
        m.0[axis][..3].copy_from_slice(&math3::quat_rotate(q, basis));
    }
    m.0[3][..3].copy_from_slice(&instance.translation);
    m
}
fn wrap(value: orr_model::Wrap) -> BakeWrap {
    match value {
        orr_model::Wrap::Clamp => BakeWrap::Clamp,
        orr_model::Wrap::Repeat => BakeWrap::Repeat,
        orr_model::Wrap::Mirror => BakeWrap::Mirror,
    }
}

struct CanonicalHash(Sha256);
impl CanonicalHash {
    fn new() -> Self {
        Self(Sha256::new())
    }
    fn bytes(&mut self, bytes: &[u8]) {
        self.u64(bytes.len() as u64);
        self.0.update(bytes);
    }
    fn string(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }
    fn u32(&mut self, value: u32) {
        self.0.update(value.to_le_bytes());
    }
    fn u64(&mut self, value: u64) {
        self.0.update(value.to_le_bytes());
    }
    fn float(&mut self, value: f32) {
        self.u32(if value == 0.0 { 0 } else { value.to_bits() });
    }
    fn floats(&mut self, values: &[f32]) {
        for &value in values {
            self.float(value);
        }
    }
    fn finish(self) -> String {
        hex_digest(&self.0.finalize())
    }
}
fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn fingerprint_scene(
    hash: &mut CanonicalHash,
    scene: &BakeScene,
    grid: &IrradianceGrid,
    settings: &BakeSettings,
    cancel: &AtomicBool,
) -> Result<(), String> {
    hash.u64(cpu::MIN_RAY_OFFSET.to_bits());
    hash.u64(cpu::RAY_OFFSET_FACTOR.to_bits());
    hash.u32(settings.rays_per_probe);
    hash.u64(settings.max_triangle_tests);
    hash.u64(settings.max_node_visits);
    hash.u64(settings.max_duration.as_nanos() as u64);
    for value in grid.dimensions {
        hash.u32(value);
    }
    hash.floats(&grid.origin);
    hash.floats(&grid.spacing);
    hash.floats(&scene.sun.direction);
    hash.floats(&scene.sun.color);
    hash.float(scene.sun.intensity);
    hash.u32(scene.triangles.len() as u32);
    for triangle in &scene.triangles {
        cancelled(cancel)?;
        for position in &triangle.positions {
            hash.floats(position);
        }
        for normal in &triangle.normals {
            hash.floats(normal);
        }
        for uv in &triangle.texcoords {
            hash.floats(uv);
        }
        hash.u32(triangle.material as u32);
    }
    hash.u32(scene.materials.len() as u32);
    for material in &scene.materials {
        cancelled(cancel)?;
        hash.floats(&material.base_color);
        hash.u32(u32::from(material.double_sided));
        match &material.checker {
            Some(checker) => {
                hash.u32(1);
                hash.float(checker.scale);
                hash.float(checker.dark_multiplier);
            }
            None => hash.u32(0),
        }
        match &material.texture {
            Some(texture) => {
                hash.u32(1);
                hash.u32(texture.width);
                hash.u32(texture.height);
                let wrapping = |w| match w {
                    BakeWrap::Clamp => 0,
                    BakeWrap::Repeat => 1,
                    BakeWrap::Mirror => 2,
                };
                hash.u32(wrapping(texture.wrap_s));
                hash.u32(wrapping(texture.wrap_t));
                hash.u32(match texture.filter {
                    BakeFilter::Nearest => 0,
                    BakeFilter::Linear => 1,
                });
                hash.u64(texture.rgba8_srgb.len() as u64);
                for chunk in texture.rgba8_srgb.chunks(64 * 1024) {
                    cancelled(cancel)?;
                    hash.0.update(chunk);
                }
            }
            None => hash.u32(0),
        }
    }
    Ok(())
}

pub struct BakeOutput {
    pub grid: IrradianceGrid,
    pub receipt: BakeReceipt,
    pub scene_path: PathBuf,
    pub fingerprint: String,
    pub source_identity: String,
    pub sources: VerifiedBakeSources,
}
impl BakeOutput {
    pub fn verify_sources_for_commit(&self) -> Result<(), String> {
        self.sources.verify_current()
    }
}

/// The hashes were verified in a worker. These cheap same-file stamps detect
/// edits/removals between verification, commit and subsequent viewport frames.
/// Stamps are never part of the canonical portable content fingerprint.
#[derive(Clone, Debug)]
pub struct VerifiedBakeSources {
    stamps: Vec<FileStamp>,
}
impl VerifiedBakeSources {
    pub fn verify_current(&self) -> Result<(), String> {
        for stamp in &self.stamps {
            if FileStamp::read(&stamp.path)? != *stamp {
                return Err("Baked model source/package changed; bake is stale".into());
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct FileStamp {
    path: PathBuf,
    len: u64,
    modified: SystemTime,
    #[cfg(unix)]
    unix_identity: (u64, u64, i64, i64),
}
impl FileStamp {
    fn read(path: &Path) -> Result<Self, String> {
        reject_source_links(path)?;
        let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !metadata.is_file() {
            return Err("Bake source/package must remain a regular file".into());
        }
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            path: path.to_owned(),
            len: metadata.len(),
            modified: metadata.modified().map_err(|e| e.to_string())?,
            #[cfg(unix)]
            unix_identity: (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            ),
        })
    }
}

fn reject_source_links(path: &Path) -> Result<(), String> {
    let mut prefix = PathBuf::new();
    let mut components = path.components().peekable();
    while let Some(component) = components.next() {
        prefix.push(component.as_os_str());
        // A bare verbatim Windows drive prefix is a device, not a directory.
        // Match package/model admission: stat it together with RootDir, while
        // a prefix-only UNC root still gets its own metadata check.
        if matches!(component, Component::Prefix(_))
            && components.peek() == Some(&Component::RootDir)
        {
            continue;
        }
        let metadata =
            std::fs::symlink_metadata(&prefix).map_err(|e| format!("Bake source: {e}"))?;
        if metadata.file_type().is_symlink() {
            return Err("Bake source/package cannot be a symbolic link".into());
        }
    }
    Ok(())
}

pub struct BakeValidationOutput {
    pub fingerprint: String,
    pub source_identity: String,
    pub scene_path: PathBuf,
    pub sources: VerifiedBakeSources,
}
pub struct BakeValidationJob {
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<BakeValidationOutput, String>>,
    finished: bool,
}
impl BakeValidationJob {
    pub fn start(snapshot: BakeSceneSnapshot) -> Result<Self, String> {
        if WORKER_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("A static diffuse bake worker is already running or cancelling".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        let spawn = thread::Builder::new()
            .name("static-diffuse-verify".into())
            .spawn(move || {
                let result = catch_worker_panic(|| {
                    let deadline = Instant::now() + snapshot.settings.max_duration;
                    let sources = verify_assets(&snapshot.assets, &worker_cancel, deadline)?;
                    Ok(BakeValidationOutput {
                        fingerprint: snapshot.fingerprint,
                        source_identity: snapshot.source_identity,
                        scene_path: snapshot.scene_path,
                        sources,
                    })
                });
                WORKER_ACTIVE.store(false, Ordering::Release);
                let _ = sender.send(result);
            });
        if let Err(error) = spawn {
            WORKER_ACTIVE.store(false, Ordering::Release);
            return Err(format!(
                "Cannot start static diffuse source validation: {error}"
            ));
        }
        Ok(Self {
            cancel,
            receiver,
            finished: false,
        })
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn poll(&mut self) -> Option<Result<BakeValidationOutput, String>> {
        if self.finished {
            return None;
        }
        let result = match self.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Static diffuse source validation disconnected".into())
            }
        };
        self.finished = true;
        Some(if self.cancel.load(Ordering::Relaxed) {
            Err("Static diffuse source validation cancelled".into())
        } else {
            result
        })
    }
}
impl Drop for BakeValidationJob {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub struct BakeJob {
    cancel: Arc<AtomicBool>,
    completed: Arc<AtomicU32>,
    probes: u32,
    receiver: mpsc::Receiver<Result<BakeOutput, String>>,
    finished: bool,
}
impl BakeJob {
    pub fn start(snapshot: BakeSceneSnapshot) -> Result<Self, String> {
        let probes = snapshot.cost.probes;
        Self::start_with_work(probes, move |cancel, completed| {
            run_snapshot(snapshot, cancel, completed)
        })
    }

    /// Private worker boundary allows a deterministic panic regression without
    /// exposing an injectable production bake API.
    fn start_with_work(
        probes: u32,
        work: impl FnOnce(&AtomicBool, &AtomicU32) -> Result<BakeOutput, String> + Send + 'static,
    ) -> Result<Self, String> {
        if WORKER_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("A static diffuse bake worker is already running or cancelling".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let completed = Arc::new(AtomicU32::new(0));
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker_cancel = cancel.clone();
        let worker_completed = completed.clone();
        let spawn = thread::Builder::new()
            .name("static-diffuse-bake".into())
            .spawn(move || {
                struct Release;
                impl Drop for Release {
                    fn drop(&mut self) {
                        WORKER_ACTIVE.store(false, Ordering::Release);
                    }
                }
                let _release = Release;
                let result = catch_worker_panic(|| work(&worker_cancel, &worker_completed));
                drop(_release);
                let _ = sender.send(result);
            });
        if let Err(error) = spawn {
            WORKER_ACTIVE.store(false, Ordering::Release);
            return Err(format!("Cannot start static diffuse bake worker: {error}"));
        }
        Ok(Self {
            cancel,
            completed,
            probes,
            receiver,
            finished: false,
        })
    }
    pub fn progress(&self) -> (u32, u32) {
        (
            self.completed.load(Ordering::Relaxed).min(self.probes),
            self.probes,
        )
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn poll(&mut self) -> Option<Result<BakeOutput, String>> {
        if self.finished {
            return None;
        }
        let result = match self.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Static diffuse bake worker disconnected".into())
            }
        };
        self.finished = true;
        Some(if self.cancel.load(Ordering::Relaxed) {
            Err("Static diffuse bake cancelled".into())
        } else {
            result
        })
    }
}
impl Drop for BakeJob {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn catch_worker_panic<T>(work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).unwrap_or_else(|_| {
        Err("Static diffuse bake worker panicked; previous irradiance preserved".into())
    })
}

fn run_snapshot(
    snapshot: BakeSceneSnapshot,
    cancel: &AtomicBool,
    progress: &AtomicU32,
) -> Result<BakeOutput, String> {
    let deadline = Instant::now() + snapshot.settings.max_duration;
    check_deadline(cancel, deadline)?;
    verify_assets(&snapshot.assets, cancel, deadline)?;
    let mut remaining = snapshot.settings.clone();
    remaining.max_duration = deadline
        .checked_duration_since(Instant::now())
        .ok_or("Static diffuse bake elapsed-time budget exceeded")?;
    let coefficients = cpu::bake_with_progress(
        &snapshot.scene,
        &snapshot.grid,
        &remaining,
        cancel,
        progress,
    )
    .map_err(|e| e.to_string())?;
    // Verify again after trace. Immutable baked bytes never race model reload,
    // and changed/missing/tampered source files cannot produce an accepted job.
    let sources = verify_assets(&snapshot.assets, cancel, deadline)?;
    check_deadline(cancel, deadline)?;
    let mut grid = snapshot.grid;
    grid.enabled = true;
    grid.provenance = IrradianceProvenance::Baked;
    grid.coefficients = coefficients;
    grid.validate()?;
    let receipt = BakeReceipt {
        version: 1,
        algorithm_version: BAKE_ALGORITHM_VERSION,
        fingerprint: snapshot.fingerprint.clone(),
        directions: snapshot.cost.directions,
        triangle_count: snapshot.cost.triangles,
        participant_count: snapshot.cost.participants,
        excluded_count: snapshot.cost.excluded,
        snapshot_bytes: snapshot.cost.snapshot_bytes,
        epsilon: cpu::MIN_RAY_OFFSET as f32,
        max_triangle_tests: snapshot.settings.max_triangle_tests,
        max_node_visits: snapshot.settings.max_node_visits,
        max_duration_nanos: snapshot.settings.max_duration.as_nanos() as u64,
    };
    Ok(BakeOutput {
        grid,
        receipt,
        scene_path: snapshot.scene_path,
        fingerprint: snapshot.fingerprint,
        source_identity: snapshot.source_identity,
        sources,
    })
}

fn check_deadline(cancel: &AtomicBool, deadline: Instant) -> Result<(), String> {
    cancelled(cancel)?;
    if Instant::now() >= deadline {
        Err("Static diffuse bake elapsed-time budget exceeded".into())
    } else {
        Ok(())
    }
}

/// Verify only participating source/dependency files, with a 64 KiB buffer and
/// cancellation between chunks. This avoids reimporting assets or allocating
/// another cooked model on the UI thread or in the worker.
fn verify_assets(
    assets: &[AssetCheck],
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<VerifiedBakeSources, String> {
    let mut seen = BTreeSet::new();
    let mut lock_paths = BTreeSet::new();
    let mut stamps = Vec::new();
    let mut total_bytes = 0_u64;
    for asset in assets {
        check_deadline(cancel, deadline)?;
        let lock_path = asset.root.join("orr.packages.lock.json");
        let lock_before = FileStamp::read(&lock_path)?;
        let mut runtime = orr_package::Runtime::content_only();
        runtime.capabilities.insert("models".into());
        runtime.capabilities.insert("irradiance-probes".into());
        #[cfg(feature = "animated-models")]
        runtime.capabilities.insert("animation".into());
        #[cfg(feature = "sprites")]
        runtime.capabilities.insert("sprite".into());
        let project =
            orr_package::Project::open(&asset.root, runtime).map_err(|e| e.to_string())?;
        let lock = project.list().map_err(|e| e.to_string())?;
        let package = lock
            .packages
            .get(&asset.binding.package)
            .ok_or("Baked model package was removed")?;
        if package.digest != asset.binding.package_digest
            || package.files.get(&asset.binding.asset) != Some(&asset.binding.source_hash)
        {
            return Err(
                "Baked model asset identity changed; reassign the binding and bake again".into(),
            );
        }
        let mut files = vec![(
            asset.binding.asset.clone(),
            asset.binding.source_hash.clone(),
        )];
        for (uri, digest) in &asset.dependencies {
            let parent = Path::new(&asset.binding.asset)
                .parent()
                .unwrap_or(Path::new(""));
            let path = parent.join(uri);
            if path
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            {
                return Err("Unsupported baked model dependency path".into());
            }
            let path = path
                .to_str()
                .ok_or("Non-UTF8 model dependency")?
                .replace('\\', "/");
            if package.files.get(&path) != Some(digest) {
                return Err("Baked model dependency identity changed".into());
            }
            files.push((path, digest.clone()));
        }
        for (relative, digest) in files {
            let key = (asset.root.clone(), package.digest.clone(), relative.clone());
            if !seen.insert(key) {
                continue;
            }
            if seen.len() > MAX_ASSET_FILES {
                return Err("Bake asset verification exceeds 4096 files".into());
            }
            check_deadline(cancel, deadline)?;
            let path = asset
                .root
                .join(".orr/packages/objects")
                .join(&package.digest)
                .join(&relative);
            stamps.push(verify_file(
                &path,
                &digest,
                cancel,
                deadline,
                &mut total_bytes,
            )?);
        }
        let after = project.list().map_err(|e| e.to_string())?;
        if after.packages.get(&asset.binding.package) != Some(package) {
            return Err("Baked model package changed during verification".into());
        }
        if FileStamp::read(&lock_path)? != lock_before {
            return Err("Baked package lock changed during verification".into());
        }
        if lock_paths.insert(lock_path) {
            stamps.push(lock_before);
        }
    }
    check_deadline(cancel, deadline)?;
    let sources = VerifiedBakeSources { stamps };
    sources.verify_current()?;
    Ok(sources)
}

fn verify_file(
    path: &Path,
    expected: &str,
    cancel: &AtomicBool,
    deadline: Instant,
    total: &mut u64,
) -> Result<FileStamp, String> {
    check_deadline(cancel, deadline)?;
    let before = FileStamp::read(path)?;
    reject_source_links(path)?;
    let mut file = std::fs::File::open(path).map_err(|e| format!("Bake asset: {e}"))?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 * 1024 {
        return Err("Bake asset must be a regular bounded file".into());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut bytes = 0_u64;
    loop {
        check_deadline(cancel, deadline)?;
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        *total = total
            .checked_add(count as u64)
            .ok_or("Bake asset byte count overflow")?;
        if bytes > 64 * 1024 * 1024 || *total > MAX_ASSET_BYTES {
            return Err("Bake source verification exceeds bounded byte budget".into());
        }
        hash.update(&buffer[..count]);
    }
    if bytes != metadata.len() || hex_digest(&hash.finalize()) != expected {
        return Err(
            "Baked model source/dependency bytes changed; previous irradiance preserved".into(),
        );
    }
    if FileStamp::read(path)? != before {
        return Err("Bake source changed during hash verification".into());
    }
    Ok(before)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewport3d::ModelPlacement;
    use orr_render::irradiance::constant_irradiance;
    use orr_view::{RenderItem3, Shape3, Style3, Transform3, Vec3};
    use std::time::Duration;
    static WORKER_TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn scene() -> BakeScene {
        BakeScene {
            triangles: vec![BakeTriangle {
                positions: [[-2.0, 0.0, -2.0], [-2.0, 0.0, 2.0], [2.0, 0.0, -2.0]],
                normals: [[0.0, 1.0, 0.0]; 3],
                texcoords: [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0]],
                material: 0,
            }],
            materials: vec![BakeMaterial {
                base_color: [0.4, 0.6, 0.2],
                texture: Some(BakeTexture {
                    width: 1,
                    height: 1,
                    rgba8_srgb: vec![120, 160, 80, 255],
                    wrap_s: BakeWrap::Repeat,
                    wrap_t: BakeWrap::Mirror,
                    filter: BakeFilter::Linear,
                }),
                checker: None,
                double_sided: false,
            }],
            sun: BakeSun {
                direction: [0.0, 1.0, 0.0],
                color: [1.0; 3],
                intensity: 2.0,
            },
        }
    }
    fn hash(scene: &BakeScene, grid: &IrradianceGrid, settings: &BakeSettings) -> String {
        let mut hash = CanonicalHash::new();
        fingerprint_scene(&mut hash, scene, grid, settings, &AtomicBool::new(false)).unwrap();
        hash.finish()
    }
    fn snapshot() -> BakeSceneSnapshot {
        let scene = scene();
        let grid = IrradianceGrid::default();
        let settings = BakeSettings {
            rays_per_probe: 64,
            ..BakeSettings::default()
        };
        BakeSceneSnapshot {
            fingerprint: hash(&scene, &grid, &settings),
            source_identity: source_location_identity(&[]).unwrap(),
            scene_path: PathBuf::from("test.yaml"),
            cost: BakeCost {
                triangles: 1,
                participants: 1,
                excluded: 2,
                probes: 8,
                directions: 64,
                snapshot_bytes: 512,
                max_primary_rays: 512,
            },
            scene,
            grid,
            settings,
            assets: Vec::new(),
        }
    }
    fn settle(editor: &mut Editor) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            editor.sync();
            if editor.yard_rows_coherent() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "coherent local Yard frame did not arrive"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }
    fn fixture_hash(contents: &str) -> (String, BakeCost) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("yard.yaml");
        std::fs::write(&path, contents).unwrap();
        let mut editor = Editor::open_game(&path, EditorGame::Yard3D).unwrap();
        settle(&mut editor);
        let models = ModelPanel::default();
        let snapshot = BakeSceneSnapshot::capture(
            &editor,
            &models,
            &IrradianceGrid::default(),
            &BakeSettings::default(),
        )
        .unwrap();
        (snapshot.fingerprint, snapshot.cost)
    }

    #[test]
    fn fingerprint_includes_each_transport_input_and_excludes_existing_coefficients() {
        let scene = scene();
        let grid = IrradianceGrid::default();
        let settings = BakeSettings::default();
        let original = hash(&scene, &grid, &settings);
        let mut existing = grid.clone();
        existing.enabled = !existing.enabled;
        existing.provenance = IrradianceProvenance::Imported;
        existing
            .coefficients
            .fill(constant_irradiance([5.0, 1.0, 2.0]).unwrap());
        assert_eq!(hash(&scene, &existing, &settings), original);
        let mut geometry = grid.clone();
        geometry.origin[0] += 0.25;
        assert_ne!(hash(&scene, &geometry, &settings), original);
        let mut geometry = grid.clone();
        geometry.spacing[2] += 0.25;
        assert_ne!(hash(&scene, &geometry, &settings), original);
        let mut changed = scene.clone();
        changed.triangles[0].positions[0][0] += 0.1;
        assert_ne!(hash(&changed, &grid, &settings), original);
        let mut changed = scene.clone();
        changed.triangles[0].normals[0][0] += 0.1;
        assert_ne!(hash(&changed, &grid, &settings), original);
        let mut changed = scene.clone();
        changed.triangles[0].texcoords[0][0] += 0.1;
        assert_ne!(hash(&changed, &grid, &settings), original);
        let mut changed = scene.clone();
        changed.materials[0].base_color[0] += 0.1;
        assert_ne!(hash(&changed, &grid, &settings), original);
        let mut changed = scene.clone();
        changed.materials[0].texture.as_mut().unwrap().rgba8_srgb[0] += 1;
        assert_ne!(hash(&changed, &grid, &settings), original);
        let mut changed = scene.clone();
        changed.materials[0].texture.as_mut().unwrap().wrap_t = BakeWrap::Clamp;
        assert_ne!(hash(&changed, &grid, &settings), original);
        let mut changed = scene.clone();
        changed.materials[0].texture.as_mut().unwrap().filter = BakeFilter::Nearest;
        assert_ne!(hash(&changed, &grid, &settings), original);
        let mut changed = scene.clone();
        changed.sun.intensity += 0.1;
        assert_ne!(hash(&changed, &grid, &settings), original);
        let mut limits = settings.clone();
        limits.max_triangle_tests -= 1;
        assert_ne!(hash(&scene, &grid, &limits), original);
        let mut limits = settings.clone();
        limits.max_node_visits -= 1;
        assert_ne!(hash(&scene, &grid, &limits), original);
        let mut limits = settings.clone();
        limits.max_duration -= Duration::from_nanos(1);
        assert_ne!(hash(&scene, &grid, &limits), original);
        let mut limits = settings.clone();
        limits.rays_per_probe = 512;
        assert_ne!(hash(&scene, &grid, &limits), original);
    }

    #[test]
    fn coherent_static_guid_participants_ignore_dynamic_and_kinematic_motion() {
        let fixture = include_str!("../../../scenes/yard3d_authoring.scene.yaml");
        let (original, cost) = fixture_hash(fixture);
        assert_eq!(
            (cost.participants, cost.excluded, cost.triangles),
            (1, 2, 2)
        );
        let moved = fixture
            .replace("pos: [-2, 1, 0]", "pos: [-8, 9, 4]")
            .replace("pos: [2, 3, 0]", "pos: [7, 14, -5]")
            .replace("kind: dynamic", "kind: kinematic");
        assert_eq!(fixture_hash(&moved).0, original);
        assert_eq!(
            fixture_hash(&fixture.replace("name: ground", "name: renamed-ground")).0,
            original
        );
        assert_ne!(
            fixture_hash(&fixture.replace("pos: [0, -0.5, 0]", "pos: [0, -0.75, 0]")).0,
            original
        );
        assert_ne!(
            fixture_hash(&fixture.replace("e_00000001:", "e_00000099:")).0,
            original
        );
    }

    #[test]
    fn cheap_input_key_ignores_play_ticks_and_excluded_motion_but_tracks_static_inputs() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("yard.yaml");
        let fixture = include_str!("../../../scenes/yard3d_authoring.scene.yaml");
        std::fs::write(&path, fixture).unwrap();
        let mut editor = Editor::open_game(&path, EditorGame::Yard3D).unwrap();
        settle(&mut editor);
        let models = ModelPanel::default();
        let grid = IrradianceGrid::default();
        let settings = BakeSettings::default();
        let original =
            BakeSceneSnapshot::participating_input_key(&editor, &models, &grid, &settings).unwrap();
        assert!(editor.start_play());
        editor.step(4);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            editor.sync();
            if editor.yard_rows_coherent()
                && editor
                    .snapshot()
                    .is_some_and(|s| s.timeline().is_some() && s.tick() >= 4)
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Play did not advance to a coherent frame"
            );
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            BakeSceneSnapshot::participating_input_key(&editor, &models, &grid, &settings).unwrap(),
            original
        );
        drop(editor);
        let capture_key = |text: &str| {
            std::fs::write(&path, text).unwrap();
            let mut editor = Editor::open_game(&path, EditorGame::Yard3D).unwrap();
            settle(&mut editor);
            BakeSceneSnapshot::participating_input_key(&editor, &models, &grid, &settings).unwrap()
        };
        let excluded = fixture
            .replace("pos: [-2, 1, 0]", "pos: [-8, 9, 4]")
            .replace("pos: [2, 3, 0]", "pos: [7, 14, -5]")
            .replace("kind: dynamic", "kind: kinematic");
        assert_eq!(capture_key(&excluded), original);
        assert_ne!(
            capture_key(&fixture.replace("pos: [0, -0.5, 0]", "pos: [0, -0.75, 0]")),
            original
        );
        assert_ne!(
            capture_key(
                &fixture.replace("half_extents: [24, 0.5, 24]", "half_extents: [22, 0.5, 24]")
            ),
            original
        );
    }

    #[test]
    fn procedural_capsule_uses_exact_render_mesh_cap_offsets_and_diffuse_checker() {
        let mut target = scene();
        target.triangles.clear();
        target.materials.clear();
        let item = RenderItem3 {
            entity: orr_ecs::Entity {
                index: 0,
                version: 0,
            },
            transform: Transform3 {
                pos: Vec3::new(3.0, 4.0, 5.0),
                rot: orr_view::Quat::IDENTITY,
            },
            style: Style3 {
                shape: Shape3::Capsule {
                    half_length: 2.0,
                    radius: 0.5,
                },
                color: [0.8, 0.4, 0.2, 1.0],
                roughness: 0.6,
                metallic: 0.25,
                checker: true,
            },
        };
        append_procedural(&mut target, &item, &AtomicBool::new(false)).unwrap();
        assert!(!target.triangles.is_empty());
        assert!(target
            .triangles
            .iter()
            .flat_map(|t| t.positions)
            .any(|p| (p[1] - 6.5).abs() < 1e-5));
        assert!(target
            .triangles
            .iter()
            .flat_map(|t| t.positions)
            .any(|p| (p[1] - 1.5).abs() < 1e-5));
        assert_eq!(target.materials[0].base_color, [0.6, 0.3, 0.15]);
        let checker = target.materials[0].checker.unwrap();
        assert_eq!((checker.scale, checker.dark_multiplier), (0.5, 0.72));
        assert!(target
            .triangles
            .iter()
            .flat_map(|t| t.normals)
            .all(|n| (math3::dot(n, n) - 1.0).abs() < 1e-5));
        assert!(append_procedural(&mut target, &item, &AtomicBool::new(true)).is_err());
    }

    #[test]
    fn model_snapshot_uses_actual_triangles_node_trs_and_raw_inverse_transpose_normals() {
        let mut node = orr_model::IDENTITY;
        node[1][1] = 2.0;
        let model = orr_model::StaticModel::new(orr_model::ModelSource {
            format: "orr_static_model".into(),
            version: 1,
            asset_id: "test.glb".into(),
            dependencies: vec![orr_model::Dependency {
                uri: "$source".into(),
                sha256: "0".repeat(64),
            }],
            primitives: vec![orr_model::Primitive {
                id: "test.glb#node=0/mesh=0/primitive=0".into(),
                vertices: vec![
                    orr_model::Vertex {
                        position: [0.0, 0.0, 0.0],
                        normal: [0.0, 1.0, 0.0],
                        uv: [0.0, 0.0],
                    },
                    orr_model::Vertex {
                        position: [0.0, 0.0, 1.0],
                        normal: [0.0, 1.0, 0.0],
                        uv: [0.0, 1.0],
                    },
                    orr_model::Vertex {
                        position: [1.0, 0.0, 0.0],
                        normal: [0.0, 1.0, 0.0],
                        uv: [1.0, 0.0],
                    },
                ],
                indices: vec![0, 1, 2],
                material: 0,
                transform: node,
            }],
            materials: vec![orr_model::Material {
                base_color: [0.6, 0.4, 0.2, 0.1],
                image: 0,
                linear_filter: true,
                wrap_s: orr_model::Wrap::Mirror,
                wrap_t: orr_model::Wrap::Repeat,
            }],
            images: vec![orr_model::Image {
                width: 1,
                height: 1,
                rgba8: vec![128, 128, 128, 32],
            }],
        })
        .unwrap();
        let placement = ModelPlacement {
            entity: orr_ecs::Entity {
                index: 0,
                version: 0,
            },
            model: Arc::new(model),
            instance: orr_render::StaticInstance {
                translation: [4.0, 5.0, 6.0],
                scale: [2.0, 3.0, 4.0],
                ..Default::default()
            },
        };
        let mut target = scene();
        target.triangles.clear();
        target.materials.clear();
        append_model(&mut target, &placement, &mut 0, &AtomicBool::new(false)).unwrap();
        assert_eq!(target.triangles.len(), 1);
        assert_eq!(
            target.triangles[0].positions,
            [[4.0, 5.0, 6.0], [4.0, 5.0, 10.0], [6.0, 5.0, 6.0]]
        );
        assert!((target.triangles[0].normals[0][1] - 1.0 / 6.0).abs() < 1e-6);
        assert_eq!(
            target.materials[0].texture.as_ref().unwrap().rgba8_srgb,
            [128, 128, 128, 32]
        );
        let mut mirrored = placement.model.source().clone();
        mirrored.primitives[0].transform[0][0] = -1.0;
        let mirrored = ModelPlacement {
            model: Arc::new(orr_model::StaticModel::new(mirrored).unwrap()),
            ..placement
        };
        let mut target = scene();
        target.triangles.clear();
        target.materials.clear();
        append_model(&mut target, &mirrored, &mut 0, &AtomicBool::new(false)).unwrap();
        let triangle = &target.triangles[0];
        assert_eq!(
            triangle.positions,
            [[4.0, 5.0, 6.0], [2.0, 5.0, 6.0], [4.0, 5.0, 10.0]]
        );
        let geometric = math3::cross(
            math3::sub(triangle.positions[1], triangle.positions[0]),
            math3::sub(triangle.positions[2], triangle.positions[0]),
        );
        assert!(math3::dot(geometric, triangle.normals[0]) > 0.0);
        assert_eq!(triangle.texcoords, [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
    }

    #[test]
    fn cancellation_limits_and_stale_source_never_expose_partial_output() {
        let mut source = snapshot();
        let old_grid = source.grid.clone();
        assert!(run_snapshot(source, &AtomicBool::new(true), &AtomicU32::new(0)).is_err());
        source = snapshot();
        source.settings.max_triangle_tests = 1;
        assert!(run_snapshot(source, &AtomicBool::new(false), &AtomicU32::new(0)).is_err());
        assert_eq!(old_grid, IrradianceGrid::default());
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("source.bin");
        std::fs::write(&file, b"original").unwrap();
        let digest = hex_digest(&Sha256::digest(b"original"));
        let deadline = Instant::now() + Duration::from_secs(2);
        let stamp = verify_file(&file, &digest, &AtomicBool::new(false), deadline, &mut 0).unwrap();
        let verified = VerifiedBakeSources {
            stamps: vec![stamp],
        };
        verified.verify_current().unwrap();
        std::fs::write(&file, b"tampered").unwrap();
        assert!(verified.verify_current().is_err());
        assert!(verify_file(&file, &digest, &AtomicBool::new(false), deadline, &mut 0).is_err());
        assert!(verify_file(&file, &digest, &AtomicBool::new(true), deadline, &mut 0).is_err());
        assert!(check_deadline(&AtomicBool::new(false), Instant::now()).is_err());
    }

    #[test]
    fn worker_cancellation_is_nonblocking_and_result_is_all_or_nothing() {
        let _serial = WORKER_TEST_SERIAL.lock().unwrap();
        let mut job = BakeJob::start(snapshot()).unwrap();
        job.cancel();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(result) = job.poll() {
                assert!(result.is_err());
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert!(job.poll().is_none());
    }

    #[test]
    fn caught_worker_panic_preserves_sidecar_history_dirty_state_and_file() {
        let _serial = WORKER_TEST_SERIAL.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("yard.yaml.irradiance.json");
        let mut bindings = crate::irradiance_bindings::IrradianceBindings::create(
            path.clone(),
            "yard.yaml".into(),
        )
        .unwrap();
        bindings.save().unwrap();
        let previous = bindings.document().clone();
        let bytes = std::fs::read(&path).unwrap();
        let revision = bindings.revision();
        let mut job =
            BakeJob::start_with_work(8, |_, _| panic!("injected worker failure")).unwrap();
        let await_result = |job: &mut BakeJob| {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if let Some(result) = job.poll() {
                    break result;
                }
                assert!(
                    Instant::now() < deadline,
                    "worker failed to return its result"
                );
                thread::sleep(Duration::from_millis(1));
            }
        };
        let error = await_result(&mut job)
            .err()
            .expect("panic must produce no output");
        assert!(error.contains("panicked"));
        assert!(job.poll().is_none());
        // Polling the panic result guarantees the one-worker admission flag
        // has been released, so a subsequent real bake starts immediately.
        let mut next = BakeJob::start(snapshot()).unwrap();
        assert!(await_result(&mut next).is_ok());
        assert_eq!(bindings.document(), &previous);
        assert_eq!(bindings.revision(), revision);
        assert!(!bindings.dirty() && !bindings.can_undo() && !bindings.can_redo());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn tiny_nonzero_triangles_are_preserved_and_exact_degenerates_are_omitted() {
        let mut target = scene();
        target.triangles.clear();
        let mut triangle = scene().triangles.remove(0);
        triangle.positions = [[0.0; 3], [0.0, 0.0, 1.0e-9], [1.0e-9, 0.0, 0.0]];
        push_triangle(&mut target, triangle.clone()).unwrap();
        assert_eq!(target.triangles.len(), 1);
        triangle.positions[2] = triangle.positions[1];
        push_triangle(&mut target, triangle).unwrap();
        assert_eq!(target.triangles.len(), 1);
    }

    #[test]
    fn retained_snapshot_memory_counts_spare_capacity_paths_grid_and_asset_metadata() {
        let mut source = snapshot();
        let before = source.retained_bytes().unwrap();
        let capacity = source.scene.triangles.capacity();
        source.scene.triangles.reserve(100);
        assert_eq!(
            source.retained_bytes().unwrap() - before,
            ((source.scene.triangles.capacity() - capacity) * std::mem::size_of::<BakeTriangle>())
                as u64
        );
        let before = source.retained_bytes().unwrap();
        let capacity = source.grid.coefficients.capacity();
        source.grid.coefficients.reserve(100);
        assert_eq!(
            source.retained_bytes().unwrap() - before,
            ((source.grid.coefficients.capacity() - capacity)
                * std::mem::size_of::<orr_render::irradiance::Sh9>()) as u64
        );
        let before = source.retained_bytes().unwrap();
        let capacity = source.scene_path.capacity()
            + source.fingerprint.capacity()
            + source.source_identity.capacity();
        source.scene_path.reserve(1024);
        source.fingerprint.reserve(256);
        source.source_identity.reserve(128);
        assert_eq!(
            source.retained_bytes().unwrap() - before,
            (source.scene_path.capacity()
                + source.fingerprint.capacity()
                + source.source_identity.capacity()
                - capacity) as u64
        );
        let before = source.retained_bytes().unwrap();
        let texture = source.scene.materials[0].texture.as_mut().unwrap();
        let capacity = texture.rgba8_srgb.capacity();
        texture.rgba8_srgb.reserve(1024);
        let increase = texture.rgba8_srgb.capacity() - capacity;
        assert_eq!(source.retained_bytes().unwrap() - before, increase as u64);
        source.assets.push(AssetCheck {
            root: PathBuf::from("/project"),
            binding: Binding {
                kind: ModelKind::Static,
                package: "demo".into(),
                asset: "test.glb".into(),
                package_digest: "0".repeat(64),
                source_hash: "1".repeat(64),
                transform: crate::model_bindings::LocalTransform::default(),
                animation: None,
            },
            dependencies: vec![("texture.png".into(), "2".repeat(64))],
        });
        let before = source.retained_bytes().unwrap();
        let asset = &mut source.assets[0];
        let capacity = asset.root.capacity()
            + asset.binding.package.capacity()
            + asset.dependencies[0].0.capacity()
            + asset.dependencies[0].1.capacity();
        asset.root.reserve(1024);
        asset.binding.package.reserve(1024);
        asset.dependencies[0].0.reserve(1024);
        asset.dependencies[0].1.reserve(1024);
        let increase = asset.root.capacity()
            + asset.binding.package.capacity()
            + asset.dependencies[0].0.capacity()
            + asset.dependencies[0].1.capacity()
            - capacity;
        assert_eq!(source.retained_bytes().unwrap() - before, increase as u64);
        assert!(source.retained_bytes().unwrap() < MAX_SNAPSHOT_BYTES);
    }

    #[test]
    fn identical_content_in_two_roots_has_distinct_source_authority_and_stamps() {
        let temp = tempfile::tempdir().unwrap();
        let bytes = b"identical immutable model bytes";
        let digest = hex_digest(&Sha256::digest(bytes));
        let binding = Binding {
            kind: ModelKind::Static,
            package: "demo".into(),
            asset: "test.glb".into(),
            package_digest: "0".repeat(64),
            source_hash: digest.clone(),
            transform: crate::model_bindings::LocalTransform::default(),
            animation: None,
        };
        let make = |name: &str| {
            let root = temp.path().join(name);
            let object = root
                .join(".orr/packages/objects")
                .join(&binding.package_digest);
            std::fs::create_dir_all(&object).unwrap();
            std::fs::write(object.join(&binding.asset), bytes).unwrap();
            AssetCheck {
                root: root.canonicalize().unwrap(),
                binding: binding.clone(),
                dependencies: Vec::new(),
            }
        };
        let a = make("project-a");
        let mut b = make("project-b");
        let mut left = snapshot();
        let mut right = snapshot();
        left.source_identity = source_location_identity(std::slice::from_ref(&a)).unwrap();
        right.source_identity = source_location_identity(std::slice::from_ref(&b)).unwrap();
        assert_eq!(a.binding, b.binding);
        assert_eq!(
            left.fingerprint, right.fingerprint,
            "portable content identity does not encode local roots"
        );
        assert_ne!(left.source_identity, right.source_identity);
        b.binding.transform.translation[0] = 5.0;
        assert_eq!(
            source_location_identity(std::slice::from_ref(&b)).unwrap(),
            right.source_identity,
            "source authority excludes a participant's transform"
        );
        let path_a = a
            .root
            .join(".orr/packages/objects")
            .join(&a.binding.package_digest)
            .join(&a.binding.asset);
        let path_b = b
            .root
            .join(".orr/packages/objects")
            .join(&b.binding.package_digest)
            .join(&b.binding.asset);
        let deadline = Instant::now() + Duration::from_secs(2);
        let stamp_a =
            verify_file(&path_a, &digest, &AtomicBool::new(false), deadline, &mut 0).unwrap();
        let stamp_b =
            verify_file(&path_b, &digest, &AtomicBool::new(false), deadline, &mut 0).unwrap();
        std::fs::write(&path_b, b"tampered model bytes").unwrap();
        assert!(VerifiedBakeSources {
            stamps: vec![stamp_a]
        }
        .verify_current()
        .is_ok());
        assert!(VerifiedBakeSources {
            stamps: vec![stamp_b]
        }
        .verify_current()
        .is_err());
    }

    #[cfg(windows)]
    #[test]
    fn canonical_windows_drive_prefix_is_checked_together_with_root() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("source.bin");
        std::fs::write(&path, b"source").unwrap();
        let path = path.canonicalize().unwrap();
        let digest = hex_digest(&Sha256::digest(b"source"));
        assert!(verify_file(
            &path,
            &digest,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(2),
            &mut 0
        )
        .is_ok());
    }
}
