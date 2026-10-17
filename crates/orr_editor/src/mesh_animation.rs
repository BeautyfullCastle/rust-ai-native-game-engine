use crate::model_bindings::PlaybackMode;
use crate::{mesh_art, mesh_rotation, viewport3d::AnimatedPlacement};
use orr_model::{
    animation::{AnimatedModel, AnimatedSource, Matrix4, Trs},
    Error as ModelError,
};
use orr_package::Manifest;
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Arc};

pub(crate) const MAX_SIDECAR_BYTES: usize = 32 * 1024 * 1024;
const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_LICENSE_BYTES: u64 = 1024 * 1024;
const MAX_DECODED_BYTES: usize = 32 * 1024 * 1024;
const MAX_CLIPS: usize = 64;
const MAX_NODES: usize = 256;
const MAX_DURATION_SECONDS: f32 = 3600.0;
const MAX_TRANSFORM: f32 = 1_000_000.0;
const MIN_SCALE: f32 = 0.001;
const MAX_SCALE: f32 = 1000.0;
const MIN_RATE: f32 = 0.125;
const MAX_RATE: f32 = 4.0;
const SCHEMA: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    pub(crate) offset: [f32; 3],
    pub(crate) scale: f32,
    pub(crate) rotation: [f32; 4],
    pub(crate) clip: u32,
    pub(crate) rate: f32,
    pub(crate) looped: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            offset: [0.0; 3],
            scale: 1.0,
            rotation: [0.0, 0.0, 0.0, 1.0],
            clip: 0,
            rate: 1.0,
            looped: true,
        }
    }
}

#[derive(Clone)]
pub(crate) struct Snapshot {
    graph_revision: [u8; 32],
    manifest: Manifest,
    package_digest: String,
    asset: String,
    asset_sha256: String,
    cooked_sha256: String,
    license: String,
    license_sha256: String,
    license_text: String,
    model: Arc<AnimatedModel>,
    settings: Settings,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sidecar {
    schema: u32,
    scene_filename: String,
    graph_revision: [u8; 32],
    manifest: Manifest,
    package_digest: String,
    asset: String,
    asset_sha256: String,
    cooked_sha256: String,
    license: String,
    license_sha256: String,
    license_text: String,
    model: AnimatedSource,
    settings: Settings,
}

struct Provenance<'a> {
    manifest: &'a Manifest,
    package_digest: &'a str,
    asset: &'a str,
    asset_sha256: &'a str,
    cooked_sha256: &'a str,
    license: &'a str,
    license_sha256: &'a str,
    license_text: &'a str,
}

impl Snapshot {
    pub(crate) fn from_package(
        root: &Path,
        asset: &str,
        license: &str,
        graph_revision: [u8; 32],
    ) -> Result<Self, String> {
        let package_root = mesh_art::checked_directory(root, "animation package root")?;
        let mut runtime = orr_package::Runtime::content_only();
        runtime.capabilities.insert("animation".to_owned());
        let manifest_bytes =
            mesh_art::read_package_file(&package_root, Path::new("orr.package.json"), 1024 * 1024)?;
        let source_manifest: Manifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|error| format!("invalid animation package manifest: {error}"))?;
        mesh_art::preflight_manifest_files(&package_root, &source_manifest)?;
        let locked = orr_package::inspect(&package_root, &runtime)
            .map_err(|error| format!("verify animation package: {error}"))?;
        if locked.manifest != source_manifest
            || !locked.manifest.dependencies.is_empty()
            || locked.manifest.capabilities
                != std::collections::BTreeSet::from(["animation".into()])
        {
            return Err(
                "mesh animation requires a verified dependency-free animation package".into(),
            );
        }
        let asset_path = mesh_art::declared_path(asset, "animated asset")?;
        let license_path = mesh_art::declared_path(license, "license file")?;
        if asset == license
            || !locked.manifest.files.contains(asset)
            || !locked.manifest.files.contains(license)
        {
            return Err(
                "animated asset and license must be distinct declared package files".into(),
            );
        }
        let asset_bytes =
            mesh_art::read_package_file(&package_root, &asset_path, MAX_SOURCE_BYTES)?;
        let license_bytes =
            mesh_art::read_package_file(&package_root, &license_path, MAX_LICENSE_BYTES)?;
        let asset_sha256 = mesh_art::digest(&asset_bytes);
        let license_sha256 = mesh_art::digest(&license_bytes);
        if locked.files.get(asset) != Some(&asset_sha256)
            || locked.files.get(license) != Some(&license_sha256)
        {
            return Err("animation package changed while its declared files were read".into());
        }
        let license_text = std::str::from_utf8(&license_bytes)
            .map_err(|_| "declared license must be UTF-8 text")?;
        if license_text.trim().is_empty() {
            return Err("declared license must not be empty".into());
        }
        let model = load_animated_asset(asset, &asset_bytes)?;
        validate_model(&model)?;
        let cooked = model.to_bytes().map_err(|error| error.to_string())?;
        if cooked.len() > MAX_SIDECAR_BYTES
            || model
                .source()
                .dependencies
                .iter()
                .any(|dependency| dependency.uri != "$source")
        {
            return Err(
                "animated model exceeds the embedded bounds or has external dependencies".into(),
            );
        }
        let locked_again = orr_package::inspect(&package_root, &runtime)
            .map_err(|error| format!("recheck animation package: {error}"))?;
        if locked_again != locked
            || mesh_art::checked_directory(&package_root, "animation package root")? != package_root
        {
            return Err("animation package changed while its snapshot was prepared".into());
        }
        let snapshot = Self {
            graph_revision,
            manifest: locked.manifest,
            package_digest: locked.digest,
            asset: asset.to_owned(),
            asset_sha256,
            cooked_sha256: mesh_art::digest(&cooked),
            license: license.to_owned(),
            license_sha256,
            license_text: license_text.to_owned(),
            model: Arc::new(model),
            settings: Settings::default(),
        };
        snapshot.encode_embedded("mesh.scene.yaml")?;
        Ok(snapshot)
    }

    pub(crate) fn settings(&self) -> Settings {
        self.settings
    }

    pub(crate) fn graph_revision(&self) -> [u8; 32] {
        self.graph_revision
    }

    pub(crate) fn with_settings(mut self, settings: Settings) -> Result<Self, String> {
        self.settings = validate_settings(settings, &self.model)?;
        self.validate_embedded("mesh.scene.yaml")?;
        Ok(self)
    }

    pub(crate) fn label(&self) -> String {
        format!(
            "{}/{} · {} clip{}",
            self.manifest.name,
            self.asset,
            self.model.source().clips.len(),
            if self.model.source().clips.len() == 1 {
                ""
            } else {
                "s"
            }
        )
    }

    pub(crate) fn clips(&self) -> Vec<(u32, String, f32)> {
        self.model
            .source()
            .clips
            .iter()
            .enumerate()
            .filter_map(|(index, clip)| {
                u32::try_from(index)
                    .ok()
                    .map(|index| (index, clip.name.clone(), clip.duration()))
            })
            .collect()
    }

    pub(crate) fn placement(
        &self,
        entity: orr_ecs::Entity,
        position: [orr_fp::FP; 3],
        tick: u64,
    ) -> Result<AnimatedPlacement, String> {
        self.validate_settings(self.settings)?;
        let world_position = position.map(orr_fp::FP::to_f32);
        if world_position
            .iter()
            .any(|value| !value.is_finite() || value.abs() > MAX_TRANSFORM)
        {
            return Err("mesh agent position is outside the animation placement bound".into());
        }
        let settings = self.settings;
        let duration = self.model.source().clips[settings.clip as usize].duration();
        let adjusted_duration = duration / settings.rate;
        let playback = if settings.looped {
            PlaybackMode::Loop
        } else {
            PlaybackMode::Once
        };
        let base_time = crate::yard_animation::sample_clip_time(
            tick,
            orr_sample::yard3d_game::TICK_RATE,
            adjusted_duration,
            playback,
        )?;
        let mut sample_time = (base_time * settings.rate).clamp(0.0, duration);
        if settings.looped && duration > 0.0 && sample_time >= duration {
            sample_time = 0.0;
        }
        let pose = self
            .model
            .sample_clip(settings.clip, sample_time)
            .map_err(|error| format!("sample embedded mesh animation: {error}"))?;
        self.model
            .validate_pose(&pose)
            .map_err(|error| format!("validate embedded mesh animation pose: {error}"))?;
        let bounds = self
            .model
            .bounds(&pose)
            .map_err(|error| format!("bound embedded mesh animation pose: {error}"))?;
        if !bounds
            .min
            .iter()
            .chain(bounds.max.iter())
            .all(|value| value.is_finite() && value.abs() <= MAX_TRANSFORM)
        {
            return Err("animated mesh pose exceeds the bounded presentation range".into());
        }
        let transform = Trs {
            translation: [0.0; 3],
            rotation: settings.rotation,
            scale: [settings.scale; 3],
        }
        .matrix();
        let mut instance: Matrix4 = transform;
        instance[3] = [
            world_position[0] + settings.offset[0],
            world_position[1] + settings.offset[1],
            world_position[2] + settings.offset[2],
            1.0,
        ];
        if instance
            .iter()
            .flatten()
            .any(|value| !value.is_finite() || value.abs() > MAX_TRANSFORM)
        {
            return Err("animated mesh instance transform exceeds its bound".into());
        }
        validate_instance_bounds(bounds, instance)?;
        Ok(AnimatedPlacement {
            entity,
            model: Arc::clone(&self.model),
            instance,
            pose,
        })
    }

    pub(crate) fn encode_embedded(&self, scene_filename: &str) -> Result<Vec<u8>, String> {
        validate_scene_filename(scene_filename)?;
        self.validate_provenance()?;
        validate_model(&self.model)?;
        let settings = validate_settings(self.settings, &self.model)?;
        let cooked = self.model.to_bytes().map_err(|error| error.to_string())?;
        if mesh_art::digest(&cooked) != self.cooked_sha256 {
            return Err("embedded animated source changed after admission".into());
        }
        let bytes = serde_json::to_vec(&Sidecar {
            schema: SCHEMA,
            scene_filename: scene_filename.to_owned(),
            graph_revision: self.graph_revision,
            manifest: self.manifest.clone(),
            package_digest: self.package_digest.clone(),
            asset: self.asset.clone(),
            asset_sha256: self.asset_sha256.clone(),
            cooked_sha256: self.cooked_sha256.clone(),
            license: self.license.clone(),
            license_sha256: self.license_sha256.clone(),
            license_text: self.license_text.clone(),
            model: self.model.source().clone(),
            settings,
        })
        .map_err(|error| format!("serialize mesh animation sidecar: {error}"))?;
        if bytes.len() > MAX_SIDECAR_BYTES {
            return Err(format!(
                "mesh animation sidecar exceeds {MAX_SIDECAR_BYTES} bytes"
            ));
        }
        Ok(bytes)
    }

    pub(crate) fn decode_embedded(
        bytes: &[u8],
        scene_filename: &str,
        graph_revision: [u8; 32],
    ) -> Result<Self, String> {
        if bytes.len() > MAX_SIDECAR_BYTES {
            return Err("mesh animation sidecar exceeds its byte limit".into());
        }
        validate_scene_filename(scene_filename)?;
        let sidecar: Sidecar = serde_json::from_slice(bytes)
            .map_err(|error| format!("invalid mesh animation sidecar: {error}"))?;
        if sidecar.schema != SCHEMA
            || sidecar.scene_filename != scene_filename
            || sidecar.graph_revision != graph_revision
        {
            return Err(
                "mesh animation sidecar belongs to a different scene or graph revision".into(),
            );
        }
        validate_provenance_fields(&Provenance {
            manifest: &sidecar.manifest,
            package_digest: &sidecar.package_digest,
            asset: &sidecar.asset,
            asset_sha256: &sidecar.asset_sha256,
            cooked_sha256: &sidecar.cooked_sha256,
            license: &sidecar.license,
            license_sha256: &sidecar.license_sha256,
            license_text: &sidecar.license_text,
        })?;
        let model = AnimatedModel::new(sidecar.model)
            .map_err(|error| format!("validate embedded animated model: {error}"))?;
        validate_model(&model)?;
        let cooked = model.to_bytes().map_err(|error| error.to_string())?;
        if mesh_art::digest(&cooked) != sidecar.cooked_sha256 {
            return Err("embedded animated model digest does not match".into());
        }
        let settings = validate_settings(sidecar.settings, &model)?;
        Ok(Self {
            graph_revision,
            manifest: sidecar.manifest,
            package_digest: sidecar.package_digest,
            asset: sidecar.asset,
            asset_sha256: sidecar.asset_sha256,
            cooked_sha256: sidecar.cooked_sha256,
            license: sidecar.license,
            license_sha256: sidecar.license_sha256,
            license_text: sidecar.license_text,
            model: Arc::new(model),
            settings,
        })
    }

    fn validate_embedded(&self, filename: &str) -> Result<(), String> {
        self.encode_embedded(filename).map(|_| ())
    }

    fn validate_provenance(&self) -> Result<(), String> {
        validate_provenance_fields(&Provenance {
            manifest: &self.manifest,
            package_digest: &self.package_digest,
            asset: &self.asset,
            asset_sha256: &self.asset_sha256,
            cooked_sha256: &self.cooked_sha256,
            license: &self.license,
            license_sha256: &self.license_sha256,
            license_text: &self.license_text,
        })
    }

    fn validate_settings(&self, settings: Settings) -> Result<Settings, String> {
        validate_settings(settings, &self.model)
    }
}

fn load_animated_asset(asset: &str, bytes: &[u8]) -> Result<AnimatedModel, String> {
    if let Ok(model) = AnimatedModel::from_bytes(bytes) {
        return Ok(model);
    }
    orr_model::animation_import::import_with_resolver(asset, bytes, |_| {
        Err(ModelError::message(
            "mesh animation snapshots do not read external resources",
        ))
    })
    .map_err(|error| format!("import animated asset: {error}"))
}

fn validate_model(model: &AnimatedModel) -> Result<(), String> {
    let source = model.source();
    if source.nodes.len() > MAX_NODES || source.clips.is_empty() || source.clips.len() > MAX_CLIPS {
        return Err("animated mesh exceeds node or clip limits".into());
    }
    let mut decoded_bytes = 0usize;
    let mut vertex_count = 0usize;
    let mut index_count = 0usize;
    let mut channel_count = 0usize;
    let mut image_bytes = 0usize;
    for image in &source.images {
        image_bytes = image_bytes
            .checked_add(image.rgba8.len())
            .ok_or("animated image bytes overflow")?;
    }
    decoded_bytes = decoded_bytes
        .checked_add(image_bytes)
        .ok_or("animated decoded bytes overflow")?;
    for primitive in &source.primitives {
        vertex_count = vertex_count
            .checked_add(primitive.vertices.len())
            .ok_or("animated vertex count overflow")?;
        index_count = index_count
            .checked_add(primitive.indices.len())
            .ok_or("animated index count overflow")?;
        decoded_bytes = decoded_bytes
            .checked_add(
                primitive
                    .vertices
                    .len()
                    .saturating_mul(std::mem::size_of::<orr_model::animation::SkinnedVertex>()),
            )
            .and_then(|sum| {
                sum.checked_add(
                    primitive
                        .indices
                        .len()
                        .saturating_mul(std::mem::size_of::<u32>()),
                )
            })
            .ok_or("animated geometry byte count overflow")?;
    }
    for clip in &source.clips {
        let duration = clip.duration();
        if !duration.is_finite() || !(0.0..=MAX_DURATION_SECONDS).contains(&duration) {
            return Err("animated clip duration exceeds its bounded range".into());
        }
        channel_count = channel_count
            .checked_add(clip.channels.len())
            .ok_or("animated channel count overflow")?;
        for channel in &clip.channels {
            decoded_bytes = decoded_bytes
                .checked_add(channel.times.len().saturating_mul(20))
                .ok_or("animated channel byte count overflow")?;
        }
    }
    if vertex_count > 100_000
        || index_count > 600_000
        || channel_count > 4096
        || image_bytes > MAX_DECODED_BYTES
        || decoded_bytes > MAX_DECODED_BYTES
    {
        return Err("animated mesh exceeds decoded workload bounds".into());
    }
    let rest_pose = model
        .rest_pose()
        .map_err(|error| format!("validate animated rest pose: {error}"))?;
    validate_pose_bounds(model, &rest_pose)?;
    for (clip_index, clip) in source.clips.iter().enumerate() {
        let clip_index = u32::try_from(clip_index).map_err(|_| "clip index exceeds u32")?;
        for time in [0.0, clip.duration()] {
            let pose = model
                .sample_clip(clip_index, time)
                .map_err(|error| format!("validate animation endpoint: {error}"))?;
            validate_pose_bounds(model, &pose)?;
        }
    }
    Ok(())
}

fn validate_pose_bounds(
    model: &AnimatedModel,
    pose: &orr_model::animation::Pose,
) -> Result<(), String> {
    let bounds = model
        .bounds(pose)
        .map_err(|error| format!("validate animated pose bounds: {error}"))?;
    if !bounds
        .min
        .iter()
        .chain(bounds.max.iter())
        .all(|value| value.is_finite() && value.abs() <= MAX_TRANSFORM)
    {
        return Err("animated mesh pose exceeds the bounded presentation range".into());
    }
    Ok(())
}

fn validate_instance_bounds(
    bounds: orr_model::animation::Bounds,
    instance: Matrix4,
) -> Result<(), String> {
    for corner_mask in 0..8 {
        let model_corner: [f32; 3] = std::array::from_fn(|coordinate_index| {
            if corner_mask & (1 << coordinate_index) == 0 {
                bounds.min[coordinate_index]
            } else {
                bounds.max[coordinate_index]
            }
        });
        for (coordinate_index, _) in model_corner.iter().enumerate() {
            let world_coordinate = model_corner[0] * instance[0][coordinate_index]
                + model_corner[1] * instance[1][coordinate_index]
                + model_corner[2] * instance[2][coordinate_index]
                + instance[3][coordinate_index];
            if !world_coordinate.is_finite() || world_coordinate.abs() > MAX_TRANSFORM {
                return Err("animated mesh instance bounds exceed the presentation range".into());
            }
        }
    }
    Ok(())
}

fn validate_settings(settings: Settings, model: &AnimatedModel) -> Result<Settings, String> {
    if settings
        .offset
        .iter()
        .any(|value| !value.is_finite() || value.abs() > MAX_TRANSFORM)
        || !settings.scale.is_finite()
        || !(MIN_SCALE..=MAX_SCALE).contains(&settings.scale)
        || !settings.rate.is_finite()
        || !(MIN_RATE..=MAX_RATE).contains(&settings.rate)
    {
        return Err(
            "mesh animation transform or playback rate is outside its finite bounds".into(),
        );
    }
    let rotation = mesh_rotation::validate_normalize(settings.rotation)?;
    let clip = model
        .source()
        .clips
        .get(settings.clip as usize)
        .ok_or_else(|| format!("animation clip {} does not exist", settings.clip))?;
    if clip.duration() > MAX_DURATION_SECONDS {
        return Err("animation clip exceeds the duration bound".into());
    }
    Ok(Settings {
        rotation,
        ..settings
    })
}

fn validate_provenance_fields(provenance: &Provenance<'_>) -> Result<(), String> {
    let manifest = provenance.manifest;
    let package_digest = provenance.package_digest;
    let asset = provenance.asset;
    let asset_sha256 = provenance.asset_sha256;
    let cooked_sha256 = provenance.cooked_sha256;
    let license = provenance.license;
    let license_sha256 = provenance.license_sha256;
    let license_text = provenance.license_text;
    if manifest.schema != 1
        || manifest.name.is_empty()
        || manifest.name.len() > 64
        || !manifest
            .name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_".contains(&byte))
        || manifest.version.is_empty()
        || manifest.version.len() > 128
        || manifest.engine.is_empty()
        || manifest.engine.len() > 128
        || manifest.capabilities != std::collections::BTreeSet::from(["animation".into()])
        || !manifest.dependencies.is_empty()
        || manifest.files.is_empty()
        || manifest.files.len() > 4096
        || !valid_digest(package_digest)
        || !valid_digest(asset_sha256)
        || !valid_digest(cooked_sha256)
        || !valid_digest(license_sha256)
    {
        return Err(
            "mesh animation provenance is not a verified dependency-free animation package".into(),
        );
    }
    let asset_path = mesh_art::declared_path(asset, "animated asset")?;
    let license_path = mesh_art::declared_path(license, "license file")?;
    if asset == license
        || asset_path == license_path
        || !manifest.files.contains(asset)
        || !manifest.files.contains(license)
    {
        return Err("animated asset and license must be distinct declared package files".into());
    }
    let mut folded = std::collections::BTreeSet::new();
    for path in &manifest.files {
        let normalized = mesh_art::declared_path(path, "provenance file")?;
        let key = normalized.to_string_lossy().to_ascii_lowercase();
        if !folded.insert(key) {
            return Err("animation package provenance has case-colliding paths".into());
        }
    }
    if license_text.trim().is_empty()
        || license_text.len() as u64 > MAX_LICENSE_BYTES
        || mesh_art::digest(license_text.as_bytes()) != license_sha256
    {
        return Err("embedded animation license is empty or its digest does not match".into());
    }
    Ok(())
}

fn validate_scene_filename(filename: &str) -> Result<(), String> {
    if filename.is_empty()
        || filename.len() > 200
        || filename.contains(['/', '\\', ':', '\0'])
        || filename.ends_with('.')
        || filename.ends_with(' ')
        || filename.chars().any(char::is_control)
    {
        return Err("mesh scene filename is not a bounded ordinary filename".into());
    }
    Ok(())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
#[path = "mesh_animation/tests.rs"]
mod tests;
