use crate::viewport3d::SceneModel;
use orr_model::{ModelSource, StaticModel};
use orr_package::Manifest;
use orr_render::StaticInstance;
use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

const SIDECAR_SCHEMA: u32 = 1;
const MAX_PATH_BYTES: usize = 4096;
const MAX_PATH_COMPONENTS: usize = 64;
const MAX_SIDE_CAR_BYTES: usize = orr_model::MAX_FILE_BYTES;
const MAX_LICENSE_BYTES: u64 = 1024 * 1024;
const MAX_WORK_VERTICES: usize = 100_000;
const MAX_WORK_TRIANGLES: usize = 100_000;
const MAX_WORK_IMAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_DECODED_WORK_BYTES: usize = 32 * 1024 * 1024;
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

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
    model: Arc<StaticModel>,
    render_model: Arc<StaticModel>,
    tint: [f32; 4],
    material_slots: Vec<crate::mesh_material_slots::Settings>,
    surface: Option<crate::mesh_surface::Settings>,
    offset: [f32; 3],
    scale: f32,
    rotation: [f32; 4],
    facing: crate::mesh_facing::Facing,
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
    model: ModelSource,
    #[serde(
        default = "crate::mesh_material::identity",
        skip_serializing_if = "crate::mesh_material::is_identity"
    )]
    tint: [f32; 4],
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "deserialize_material_slots"
    )]
    material_slots: Vec<crate::mesh_material_slots::Settings>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_surface"
    )]
    surface: Option<crate::mesh_surface::Settings>,
    offset: [f32; 3],
    scale: f32,
    #[serde(
        default = "identity_rotation",
        skip_serializing_if = "rotation_is_identity"
    )]
    rotation: [f32; 4],
    #[serde(default, skip_serializing_if = "crate::mesh_facing::is_fixed")]
    facing: crate::mesh_facing::Facing,
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
        let package_root = checked_directory(root, "package root")?;
        let mut runtime = orr_package::Runtime::content_only();
        runtime.capabilities.insert("models".to_owned());
        let manifest_bytes =
            read_package_file(&package_root, Path::new("orr.package.json"), 1024 * 1024)?;
        let source_manifest: Manifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|error| format!("invalid model package manifest: {error}"))?;
        preflight_manifest_files(&package_root, &source_manifest)?;
        let locked = orr_package::inspect(&package_root, &runtime)
            .map_err(|error| format!("verify model package: {error}"))?;
        if locked.manifest != source_manifest {
            return Err("model package manifest changed during inspection".into());
        }
        if !locked.manifest.dependencies.is_empty()
            || locked.manifest.capabilities != std::collections::BTreeSet::from(["models".into()])
        {
            return Err("mesh art requires a dependency-free static-model package".into());
        }
        let asset_path = declared_path(asset, "model asset")?;
        let license_path = declared_path(license, "license file")?;
        if !locked.manifest.files.contains(asset)
            || !locked.manifest.files.contains(license)
            || asset == license
        {
            return Err(
                "model asset and license must be distinct files declared by the package".into(),
            );
        }
        let asset_bytes =
            read_package_file(&package_root, &asset_path, orr_model::MAX_FILE_BYTES as u64)?;
        let license_bytes = read_package_file(&package_root, &license_path, MAX_LICENSE_BYTES)?;
        let asset_sha256 = digest(&asset_bytes);
        let license_sha256 = digest(&license_bytes);
        if locked.files.get(asset) != Some(&asset_sha256)
            || locked.files.get(license) != Some(&license_sha256)
        {
            return Err("model package changed while its declared files were read".into());
        }
        let license_text = std::str::from_utf8(&license_bytes)
            .map_err(|_| "declared license must be UTF-8 text")?;
        if license_text.trim().is_empty() {
            return Err("declared license must not be empty".into());
        }
        let model = import_static_asset(asset, &asset_bytes)?;
        let model_bytes = model.to_bytes().map_err(|error| error.to_string())?;
        validate_source_budget(model.source())?;
        if model_bytes.len() > MAX_SIDE_CAR_BYTES
            || model
                .source()
                .dependencies
                .iter()
                .any(|dependency| dependency.uri != "$source")
        {
            return Err(
                "mesh art model exceeds the embedded snapshot limits or has external dependencies"
                    .into(),
            );
        }
        let locked_again = orr_package::inspect(&package_root, &runtime)
            .map_err(|error| format!("recheck model package: {error}"))?;
        if locked_again != locked {
            return Err("model package changed while its snapshot was prepared".into());
        }
        if checked_directory(&package_root, "package root")? != package_root {
            return Err("model package root changed while its snapshot was prepared".into());
        }
        let model = Arc::new(model);
        let snapshot = Self {
            graph_revision,
            manifest: locked.manifest,
            package_digest: locked.digest,
            asset: asset.to_owned(),
            asset_sha256,
            cooked_sha256: digest(&model_bytes),
            license: license.to_owned(),
            license_sha256,
            license_text: license_text.to_owned(),
            render_model: Arc::clone(&model),
            model,
            tint: crate::mesh_material::identity(),
            material_slots: Vec::new(),
            surface: None,
            offset: [0.0; 3],
            scale: 1.0,
            rotation: identity_rotation(),
            facing: crate::mesh_facing::Facing::Fixed,
        };
        snapshot.sidecar_bytes("room.scene.yaml")?;
        Ok(snapshot)
    }

    pub(crate) fn with_transform(mut self, offset: [f32; 3], scale: f32) -> Result<Self, String> {
        if !offset
            .iter()
            .all(|value| value.is_finite() && value.abs() <= 1_000_000.0)
            || !scale.is_finite()
            || scale <= 0.0
            || scale > 1_000_000.0
        {
            return Err("mesh art transform must be finite and bounded with positive scale".into());
        }
        self.offset = offset;
        self.scale = scale;
        self.placement([orr_fp::FP::ZERO; 3])?;
        Ok(self)
    }

    pub(crate) fn offset(&self) -> [f32; 3] {
        self.offset
    }

    pub(crate) fn scale(&self) -> f32 {
        self.scale
    }

    pub(crate) fn rotation(&self) -> [f32; 4] {
        self.rotation
    }

    pub(crate) fn tint(&self) -> [f32; 4] {
        self.tint
    }

    pub(crate) fn material_slots(&self) -> &[crate::mesh_material_slots::Settings] {
        &self.material_slots
    }

    pub(crate) fn source_model_arc(&self) -> &Arc<StaticModel> {
        &self.model
    }

    pub(crate) fn surface(&self) -> Option<crate::mesh_surface::Settings> {
        self.surface
    }

    pub(crate) fn material_choices(&self) -> Vec<u32> {
        self.model
            .source()
            .primitives
            .iter()
            .map(|primitive| primitive.material)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub(crate) fn with_material_slots(
        mut self,
        material_slots: Vec<crate::mesh_material_slots::Settings>,
    ) -> Result<Self, String> {
        crate::mesh_material_slots::validate(&material_slots, &self.model)?;
        self.material_slots = material_slots;
        self.render_model =
            render_model(&self.model, self.tint, self.surface, &self.material_slots)?;
        self.placement([orr_fp::FP::ZERO; 3])?;
        Ok(self)
    }

    pub(crate) fn with_surface(
        mut self,
        surface: Option<crate::mesh_surface::Settings>,
    ) -> Result<Self, String> {
        self.surface = surface
            .map(|settings| crate::mesh_surface::validate(settings, &self.model))
            .transpose()?;
        self.render_model =
            render_model(&self.model, self.tint, self.surface, &self.material_slots)?;
        self.placement([orr_fp::FP::ZERO; 3])?;
        Ok(self)
    }

    pub(crate) fn with_tint(mut self, tint: [f32; 4]) -> Result<Self, String> {
        self.tint = crate::mesh_material::validate(tint)?;
        self.render_model =
            render_model(&self.model, self.tint, self.surface, &self.material_slots)?;
        self.placement([orr_fp::FP::ZERO; 3])?;
        Ok(self)
    }

    pub(crate) fn with_rotation(mut self, rotation: [f32; 4]) -> Result<Self, String> {
        self.rotation = crate::mesh_rotation::validate_normalize(rotation)?;
        self.placement([orr_fp::FP::ZERO; 3])?;
        Ok(self)
    }

    pub(crate) fn facing(&self) -> crate::mesh_facing::Facing {
        self.facing
    }

    pub(crate) fn with_facing(
        mut self,
        facing: crate::mesh_facing::Facing,
    ) -> Result<Self, String> {
        self.facing = facing;
        self.placement([orr_fp::FP::ZERO; 3])?;
        Ok(self)
    }

    pub(crate) fn graph_revision(&self) -> [u8; 32] {
        self.graph_revision
    }

    pub(crate) fn label(&self) -> String {
        format!("{}/{}", self.manifest.name, self.asset)
    }

    pub(crate) fn placement(&self, position: [orr_fp::FP; 3]) -> Result<SceneModel, String> {
        let position = position.map(orr_fp::FP::to_f32);
        self.placement_with_rotation(position, self.rotation)
    }

    pub(crate) fn placement_for_view(
        &self,
        view: &orr_remote::mesh_navigation::MeshView,
    ) -> Result<SceneModel, String> {
        let rotation = crate::mesh_facing::rotation(self.facing, self.rotation, view)?;
        self.placement_with_rotation(view.agent.position.map(orr_fp::FP::to_f32), rotation)
    }

    fn placement_with_rotation(
        &self,
        position: [f32; 3],
        rotation: [f32; 4],
    ) -> Result<SceneModel, String> {
        let instance = StaticInstance {
            translation: std::array::from_fn(|axis| position[axis] + self.offset[axis]),
            scale: [self.scale; 3],
            rotation,
        };
        instance
            .validate_for(&self.render_model)
            .map_err(|error| format!("mesh art placement: {error}"))?;
        Ok(SceneModel {
            model: Arc::clone(&self.render_model),
            instance,
        })
    }

    pub(crate) fn encode_embedded(&self, scene_filename: &str) -> Result<Vec<u8>, String> {
        self.sidecar_bytes(scene_filename)
    }

    pub(crate) fn decode_embedded(
        bytes: &[u8],
        scene_filename: &str,
        graph_revision: [u8; 32],
    ) -> Result<Self, String> {
        if bytes.len() > MAX_SIDE_CAR_BYTES {
            return Err("embedded mesh art exceeds the 64 MiB limit".into());
        }
        validate_leaf(scene_filename, "scene filename")?;
        let decoded: Sidecar = serde_json::from_slice(bytes)
            .map_err(|error| format!("invalid embedded mesh art: {error}"))?;
        Self::from_sidecar(decoded, scene_filename, graph_revision)
    }

    fn sidecar_bytes(&self, scene_filename: &str) -> Result<Vec<u8>, String> {
        validate_leaf(scene_filename, "scene filename")?;
        validate_source_budget(self.model.source())?;
        crate::mesh_material::validate(self.tint)?;
        crate::mesh_material_slots::validate(&self.material_slots, &self.model)?;
        if let Some(settings) = self.surface {
            crate::mesh_surface::validate(settings, &self.model)?;
        }
        let model_bytes = self.model.to_bytes().map_err(|error| error.to_string())?;
        if digest(&model_bytes) != self.cooked_sha256 {
            return Err("embedded cooked model digest changed".into());
        }
        validate_provenance(Provenance {
            manifest: &self.manifest,
            package_digest: &self.package_digest,
            asset: &self.asset,
            asset_sha256: &self.asset_sha256,
            cooked_sha256: &self.cooked_sha256,
            license: &self.license,
            license_sha256: &self.license_sha256,
            license_text: &self.license_text,
        })?;
        self.placement([orr_fp::FP::ZERO; 3])?;
        let bytes = serde_json::to_vec(&Sidecar {
            schema: SIDECAR_SCHEMA,
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
            tint: self.tint,
            material_slots: self.material_slots.clone(),
            surface: self.surface,
            offset: self.offset,
            scale: self.scale,
            rotation: self.rotation,
            facing: self.facing,
        })
        .map_err(|error| format!("serialize mesh art sidecar: {error}"))?;
        if bytes.len() > MAX_SIDE_CAR_BYTES {
            return Err("mesh art sidecar exceeds the 64 MiB limit".into());
        }
        Ok(bytes)
    }

    fn from_sidecar(
        sidecar: Sidecar,
        scene_filename: &str,
        revision: [u8; 32],
    ) -> Result<Self, String> {
        if sidecar.schema != SIDECAR_SCHEMA
            || sidecar.scene_filename != scene_filename
            || sidecar.graph_revision != revision
        {
            return Err("mesh art sidecar belongs to a different scene or mesh revision".into());
        }
        validate_provenance(Provenance {
            manifest: &sidecar.manifest,
            package_digest: &sidecar.package_digest,
            asset: &sidecar.asset,
            asset_sha256: &sidecar.asset_sha256,
            cooked_sha256: &sidecar.cooked_sha256,
            license: &sidecar.license,
            license_sha256: &sidecar.license_sha256,
            license_text: &sidecar.license_text,
        })?;
        validate_source_budget(&sidecar.model)?;
        let model =
            StaticModel::new(sidecar.model).map_err(|error| format!("embedded model: {error}"))?;
        let tint = crate::mesh_material::validate(sidecar.tint)?;
        let model = Arc::new(model);
        let model_bytes = model.to_bytes().map_err(|error| error.to_string())?;
        if model_bytes.len() > MAX_SIDE_CAR_BYTES
            || digest(&model_bytes) != sidecar.cooked_sha256
            || model
                .source()
                .dependencies
                .iter()
                .any(|dependency| dependency.uri != "$source")
        {
            return Err("embedded model exceeds limits or has external dependencies".into());
        }
        let surface = sidecar
            .surface
            .map(|settings| crate::mesh_surface::validate(settings, &model))
            .transpose()?;
        let material_slots = sidecar.material_slots;
        crate::mesh_material_slots::validate(&material_slots, &model)?;
        let render_model = render_model(&model, tint, surface, &material_slots)?;
        let snapshot = Self {
            graph_revision: sidecar.graph_revision,
            manifest: sidecar.manifest,
            package_digest: sidecar.package_digest,
            asset: sidecar.asset,
            asset_sha256: sidecar.asset_sha256,
            cooked_sha256: sidecar.cooked_sha256,
            license: sidecar.license,
            license_sha256: sidecar.license_sha256,
            license_text: sidecar.license_text,
            render_model,
            model,
            tint,
            material_slots,
            surface,
            offset: sidecar.offset,
            scale: sidecar.scale,
            rotation: sidecar.rotation,
            facing: sidecar.facing,
        };
        let offset = snapshot.offset;
        let scale = snapshot.scale;
        let rotation = snapshot.rotation;
        snapshot
            .with_rotation(rotation)?
            .with_transform(offset, scale)
    }
}

fn identity_rotation() -> [f32; 4] {
    [0.0, 0.0, 0.0, 1.0]
}

fn deserialize_surface<'de, Deserializer>(
    deserializer: Deserializer,
) -> Result<Option<crate::mesh_surface::Settings>, Deserializer::Error>
where
    Deserializer: serde::Deserializer<'de>,
{
    crate::mesh_surface::Settings::deserialize(deserializer).map(Some)
}

fn deserialize_material_slots<'de, Deserializer>(
    deserializer: Deserializer,
) -> Result<Vec<crate::mesh_material_slots::Settings>, Deserializer::Error>
where
    Deserializer: serde::Deserializer<'de>,
{
    struct SettingsVisitor;

    impl<'de> Visitor<'de> for SettingsVisitor {
        type Value = Vec<crate::mesh_material_slots::Settings>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("at most 256 material-slot settings")
        }

        fn visit_seq<Access>(self, mut access: Access) -> Result<Self::Value, Access::Error>
        where
            Access: SeqAccess<'de>,
        {
            let mut settings = Vec::new();
            while let Some(setting) = access.next_element()? {
                if settings.len() == 256 {
                    return Err(de::Error::custom(
                        "material slot settings exceed the 256-slot limit",
                    ));
                }
                settings.push(setting);
            }
            Ok(settings)
        }
    }

    deserializer.deserialize_seq(SettingsVisitor)
}

fn render_model(
    model: &Arc<StaticModel>,
    tint: [f32; 4],
    surface: Option<crate::mesh_surface::Settings>,
    material_slots: &[crate::mesh_material_slots::Settings],
) -> Result<Arc<StaticModel>, String> {
    if !material_slots.is_empty() {
        let derived = Arc::new(crate::mesh_material_slots::apply(model, material_slots)?);
        return render_model(&derived, tint, surface, &[]);
    }
    match surface {
        Some(settings) => {
            let settings = crate::mesh_surface::validate(settings, model)?;
            if crate::mesh_surface::is_identity(&settings)
                && crate::mesh_material::is_identity(&tint)
            {
                Ok(Arc::clone(model))
            } else {
                crate::mesh_surface::derived_model(model, settings, tint).map(Arc::new)
            }
        }
        None if crate::mesh_material::is_identity(&tint) => Ok(Arc::clone(model)),
        None => crate::mesh_material::tinted_model(model, tint).map(Arc::new),
    }
}

fn rotation_is_identity(rotation: &[f32; 4]) -> bool {
    *rotation == identity_rotation()
}

pub(crate) fn sidecar_path(scene: &Path) -> Result<PathBuf, String> {
    let (canonical_scene, _) = checked_scene(scene)?;
    let filename = canonical_scene
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("scene filename must be Unicode")?;
    Ok(canonical_scene.with_file_name(format!("{filename}.mesh-art.json")))
}

pub(crate) fn load(
    scene: &Path,
    graph_revision: [u8; 32],
) -> Result<Option<(Snapshot, String)>, String> {
    let (canonical_scene, filename) = checked_scene(scene)?;
    let sidecar = sidecar_path(&canonical_scene)?;
    let bytes = match read_optional_file(&sidecar, MAX_SIDE_CAR_BYTES as u64, "mesh art sidecar")? {
        Some(bytes) => bytes,
        None => return Ok(None),
    };
    let decoded: Sidecar = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid mesh art sidecar: {error}"))?;
    let snapshot = Snapshot::from_sidecar(decoded, &filename, graph_revision)?;
    Ok(Some((snapshot, digest(&bytes))))
}

pub(crate) fn sidecar_stamp(scene: &Path) -> Result<Option<String>, String> {
    let sidecar = sidecar_path(scene)?;
    Ok(
        read_optional_file(&sidecar, MAX_SIDE_CAR_BYTES as u64, "mesh art sidecar")?
            .as_deref()
            .map(digest),
    )
}

pub(crate) fn save(
    scene: &Path,
    snapshot: Option<&Snapshot>,
    expected: Option<&str>,
) -> Result<Option<String>, String> {
    let (canonical_scene, filename) = checked_scene(scene)?;
    let sidecar = sidecar_path(&canonical_scene)?;
    validate_expected(expected)?;
    let current = read_optional_file(&sidecar, MAX_SIDE_CAR_BYTES as u64, "mesh art sidecar")?;
    match (current.as_deref().map(digest), expected) {
        (None, None) => {}
        (Some(current), Some(expected)) if current == expected => {}
        (None, Some(_)) => return Err("mesh art sidecar was removed by another editor".into()),
        (Some(_), None) => {
            return Err("mesh art sidecar already exists; reload before saving".into())
        }
        (Some(_), Some(_)) => return Err("mesh art sidecar changed; reload before saving".into()),
    }
    let Some(snapshot) = snapshot else {
        if current.is_some() {
            recheck_scene(&canonical_scene)?;
            let final_current =
                read_optional_file(&sidecar, MAX_SIDE_CAR_BYTES as u64, "mesh art sidecar")?;
            if final_current.as_deref().map(digest).as_deref() != expected {
                return Err(
                    "mesh art sidecar changed before removal; reload before removing".into(),
                );
            }
            fs::remove_file(&sidecar)
                .map_err(|error| format!("remove mesh art sidecar: {error}"))?;
        }
        return Ok(None);
    };
    let bytes = snapshot.sidecar_bytes(&filename)?;
    let parent = canonical_scene.parent().ok_or("scene parent missing")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("create mesh art staging file: {error}"))?;
    temporary
        .write_all(&bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|error| format!("write mesh art sidecar: {error}"))?;
    recheck_scene(&canonical_scene)?;
    let final_current =
        read_optional_file(&sidecar, MAX_SIDE_CAR_BYTES as u64, "mesh art sidecar")?;
    if final_current.as_deref().map(digest).as_deref() != expected {
        return Err("mesh art sidecar changed before save; reload before saving".into());
    }
    if expected.is_some() {
        temporary
            .persist(&sidecar)
            .map_err(|error| format!("publish mesh art sidecar: {}", error.error))?;
    } else {
        temporary.persist_noclobber(&sidecar).map_err(|error| {
            format!(
                "publish mesh art sidecar without replacement: {}",
                error.error
            )
        })?;
    }
    Ok(Some(digest(&bytes)))
}

fn import_static_asset(asset: &str, bytes: &[u8]) -> Result<StaticModel, String> {
    let extension = Path::new(asset)
        .extension()
        .and_then(|value| value.to_str())
        .ok_or("model asset must be a .glb or .orrmodel")?;
    let model = if extension.eq_ignore_ascii_case("glb") {
        orr_model::import::import_with_resolver(asset, bytes, |_| {
            Err(orr_model::Error::message(
                "external model resources are not supported",
            ))
        })
        .map_err(|error| format!("import self-contained GLB: {error}"))?
    } else if extension.eq_ignore_ascii_case("orrmodel") {
        StaticModel::from_bytes(bytes)
            .map_err(|error| format!("load cooked static model: {error}"))?
    } else {
        return Err("model asset must be a .glb or .orrmodel".into());
    };
    if model
        .source()
        .dependencies
        .iter()
        .any(|dependency| dependency.uri != "$source")
    {
        return Err("model asset contains external dependencies".into());
    }
    Ok(model)
}

fn validate_source_budget(source: &ModelSource) -> Result<(), String> {
    if source.primitives.is_empty()
        || source.primitives.len() > 256
        || source.materials.len() > 256
        || source.images.len() > 32
    {
        return Err("mesh art model exceeds the primitive/material/image count limits".into());
    }
    let mut vertices = 0usize;
    let mut indices = 0usize;
    for primitive in &source.primitives {
        vertices = vertices
            .checked_add(primitive.vertices.len())
            .ok_or("mesh art vertex count overflow")?;
        indices = indices
            .checked_add(primitive.indices.len())
            .ok_or("mesh art index count overflow")?;
    }
    let image_bytes = source.images.iter().try_fold(0usize, |total, image| {
        total
            .checked_add(image.rgba8.len())
            .ok_or("mesh art image byte count overflow")
    })?;
    let work_bytes = vertices
        .checked_mul(std::mem::size_of::<orr_model::Vertex>())
        .and_then(|bytes| bytes.checked_add(indices.checked_mul(std::mem::size_of::<u32>())?))
        .and_then(|bytes| bytes.checked_add(image_bytes))
        .ok_or("mesh art decoded work budget overflow")?;
    if vertices > MAX_WORK_VERTICES
        || !indices.is_multiple_of(3)
        || indices / 3 > MAX_WORK_TRIANGLES
        || image_bytes > MAX_WORK_IMAGE_BYTES
        || work_bytes > MAX_DECODED_WORK_BYTES
    {
        return Err("mesh art model exceeds the decoded geometry or texture work limits".into());
    }
    Ok(())
}

fn validate_provenance(provenance: Provenance<'_>) -> Result<(), String> {
    let Provenance {
        manifest,
        package_digest,
        asset,
        asset_sha256,
        cooked_sha256,
        license,
        license_sha256,
        license_text,
    } = provenance;
    if manifest.schema != 1
        || !manifest.dependencies.is_empty()
        || manifest.capabilities != std::collections::BTreeSet::from(["models".into()])
        || manifest.name.is_empty()
        || manifest.name.len() > 64
        || !manifest
            .name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_".contains(&byte))
        || manifest.version.is_empty()
        || manifest.version.len() > 128
        || manifest.version.chars().any(char::is_control)
        || manifest.engine.is_empty()
        || manifest.engine.len() > 128
        || manifest.engine.chars().any(char::is_control)
        || manifest.files.is_empty()
        || manifest.files.len() > 4096
        || !valid_digest(package_digest)
        || !valid_digest(asset_sha256)
        || !valid_digest(cooked_sha256)
        || !valid_digest(license_sha256)
    {
        return Err("mesh art provenance is not a verified dependency-free static package".into());
    }
    declared_path(asset, "model asset")?;
    declared_path(license, "license file")?;
    if asset == license || !manifest.files.contains(asset) || !manifest.files.contains(license) {
        return Err("mesh art asset and license must be distinct declared package files".into());
    }
    let mut folded_paths = std::collections::BTreeSet::new();
    for path in &manifest.files {
        declared_path(path, "package provenance path")?;
        if !folded_paths.insert(path.to_ascii_lowercase()) {
            return Err("mesh art package has case-colliding paths".into());
        }
    }
    for path in &folded_paths {
        for (index, _) in path.match_indices('/') {
            if folded_paths.contains(&path[..index]) {
                return Err("mesh art package has file/directory path collisions".into());
            }
        }
    }
    if license_text.trim().is_empty()
        || license_text.len() as u64 > MAX_LICENSE_BYTES
        || digest(license_text.as_bytes()) != license_sha256
    {
        return Err("embedded license is empty or its digest does not match".into());
    }
    Ok(())
}

pub(crate) fn preflight_manifest_files(root: &Path, manifest: &Manifest) -> Result<(), String> {
    if manifest.files.is_empty() || manifest.files.len() > 4096 {
        return Err("model package has an invalid declared file count".into());
    }
    let mut total = 0u64;
    for declared in &manifest.files {
        let relative = declared_path(declared, "declared package file")?;
        let path = root.join(&relative);
        let mut current = root.to_path_buf();
        for component in relative.components() {
            let Component::Normal(part) = component else {
                return Err("declared package path is not normalized".into());
            };
            current.push(part);
            let metadata = fs::symlink_metadata(&current)
                .map_err(|error| format!("declared package path metadata: {error}"))?;
            if current == path {
                if !regular_non_reparse(&metadata) || metadata.len() > 64 * 1024 * 1024 {
                    return Err("declared package file is not a bounded regular file".into());
                }
                total = total
                    .checked_add(metadata.len())
                    .ok_or("declared package byte count overflow")?;
            } else if !regular_directory(&metadata) {
                return Err(
                    "declared package path contains a reparse point or non-directory".into(),
                );
            }
        }
        if total > 256 * 1024 * 1024 {
            return Err("declared package exceeds the 256 MiB total byte limit".into());
        }
    }
    Ok(())
}

pub(crate) fn declared_path(value: &str, label: &str) -> Result<PathBuf, String> {
    if value.is_empty() || value.len() > 240 || value.starts_with('/') || value.contains('\\') {
        return Err(format!(
            "{label} must be a normalized declared package path"
        ));
    }
    let parts: Vec<_> = value.split('/').collect();
    if parts.len() > 16
        || parts.iter().any(|part| {
            part.is_empty()
                || *part == "."
                || *part == ".."
                || part.len() > 100
                || part.ends_with('.')
                || part.ends_with(' ')
                || reserved_device_name(part)
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        })
    {
        return Err(format!(
            "{label} must be a normalized declared package path"
        ));
    }
    let mut path = PathBuf::new();
    for part in parts {
        path.push(part);
    }
    Ok(path)
}

pub(crate) fn checked_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    validate_absolute_path(path, label)?;
    let mut current = PathBuf::new();
    let components: Vec<_> = path.components().collect();
    #[cfg(windows)]
    {
        let mut iter = components.iter().copied();
        let prefix = match iter.next() {
            Some(Component::Prefix(prefix))
                if matches!(
                    prefix.kind(),
                    std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
                ) =>
            {
                prefix
            }
            _ => return Err(format!("{label} must use an absolute local disk path")),
        };
        current.push(prefix.as_os_str());
        match iter.next() {
            Some(Component::RootDir) => current.push("\\"),
            _ => return Err(format!("{label} must be rooted")),
        }
        ensure_directory(&current, label)?;
        for component in iter {
            let Component::Normal(part) = component else {
                return Err(format!("{label} path is not normalized"));
            };
            validate_leaf(
                part.to_str().ok_or("path component must be Unicode")?,
                label,
            )?;
            current.push(part);
            ensure_directory(&current, label)?;
        }
    }
    #[cfg(not(windows))]
    {
        for component in components {
            match component {
                Component::RootDir => current.push(component.as_os_str()),
                Component::Normal(part) => {
                    validate_leaf(
                        part.to_str().ok_or("path component must be Unicode")?,
                        label,
                    )?;
                    current.push(part);
                    ensure_directory(&current, label)?;
                }
                _ => return Err(format!("{label} path is not normalized")),
            }
        }
    }
    fs::canonicalize(path).map_err(|error| format!("canonicalize {label}: {error}"))
}

pub(crate) fn checked_scene(scene: &Path) -> Result<(PathBuf, String), String> {
    validate_absolute_path(scene, "scene")?;
    let parent = scene.parent().ok_or("scene needs a parent directory")?;
    let parent = checked_directory(parent, "scene parent")?;
    let filename = scene
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("scene filename must be Unicode")?;
    validate_leaf(filename, "scene filename")?;
    if filename.len() > 180 {
        return Err("scene filename is too long for its mesh art sidecar".into());
    }
    let canonical_scene = parent.join(filename);
    let metadata = fs::symlink_metadata(&canonical_scene)
        .map_err(|error| format!("scene metadata: {error}"))?;
    if !regular_non_reparse(&metadata) {
        return Err("scene must be a regular non-reparse file".into());
    }
    Ok((canonical_scene, filename.to_owned()))
}

pub(crate) fn recheck_scene(scene: &Path) -> Result<(), String> {
    let parent = scene.parent().ok_or("scene parent missing")?;
    if checked_directory(parent, "scene parent")? != parent {
        return Err("scene parent changed during mesh art save".into());
    }
    let metadata =
        fs::symlink_metadata(scene).map_err(|error| format!("scene recheck: {error}"))?;
    if !regular_non_reparse(&metadata) {
        return Err("scene changed to a non-regular file".into());
    }
    Ok(())
}

pub(crate) fn read_package_file(
    root: &Path,
    relative: &Path,
    limit: u64,
) -> Result<Vec<u8>, String> {
    let path = root.join(relative);
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err("declared package path is not normalized".into());
        };
        current.push(part);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("package file metadata: {error}"))?;
        let final_component = current == path;
        if final_component {
            if !regular_non_reparse(&metadata) {
                return Err("declared package file must be regular and non-reparse".into());
            }
        } else {
            ensure_directory(&current, "package file parent")?;
        }
    }
    read_regular_file(&path, limit, "declared package file")
}

pub(crate) fn read_optional_file(
    path: &Path,
    limit: u64,
    label: &str,
) -> Result<Option<Vec<u8>>, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => read_regular_file(path, limit, label).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("{label} metadata: {error}")),
    }
}

fn read_regular_file(path: &Path, limit: u64, label: &str) -> Result<Vec<u8>, String> {
    let before =
        fs::symlink_metadata(path).map_err(|error| format!("{label} metadata: {error}"))?;
    if !regular_non_reparse(&before) || before.len() > limit {
        return Err(format!(
            "{label} must be a regular non-reparse file within its byte limit"
        ));
    }
    let mut file =
        open_without_following(path).map_err(|error| format!("open {label}: {error}"))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("{label} handle metadata: {error}"))?;
    if !regular_non_reparse(&opened) || opened.len() > limit {
        return Err(format!(
            "{label} changed to a non-regular or oversized file"
        ));
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    Read::by_ref(&mut file)
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {label}: {error}"))?;
    let after = fs::symlink_metadata(path).map_err(|error| format!("{label} recheck: {error}"))?;
    let handle_after = file
        .metadata()
        .map_err(|error| format!("{label} handle recheck: {error}"))?;
    if !regular_non_reparse(&after)
        || !regular_non_reparse(&handle_after)
        || bytes.len() as u64 > limit
        || bytes.len() as u64 != opened.len()
        || !same_observation(&before, &opened)
        || !same_observation(&opened, &handle_after)
        || !same_observation(&before, &after)
    {
        return Err(format!("{label} changed while being read"));
    }
    Ok(bytes)
}

fn same_observation(first: &Metadata, second: &Metadata) -> bool {
    first.len() == second.len()
        && match (first.modified(), second.modified()) {
            (Ok(first_time), Ok(second_time)) => first_time == second_time,
            _ => true,
        }
}

fn validate_absolute_path(path: &Path, label: &str) -> Result<(), String> {
    let text = path
        .to_str()
        .ok_or_else(|| format!("{label} path must be Unicode"))?;
    let components: Vec<_> = path.components().collect();
    if !path.is_absolute()
        || text.is_empty()
        || text.len() > MAX_PATH_BYTES
        || components.len() > MAX_PATH_COMPONENTS
        || text
            .chars()
            .any(|character| character.is_control() || character == '\0')
        || text
            .split(['\\', '/'])
            .any(|part| part == "." || part == "..")
    {
        return Err(format!(
            "{label} path must be bounded, absolute and normalized"
        ));
    }
    Ok(())
}

fn validate_leaf(name: &str, label: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 200
        || name.ends_with('.')
        || name.ends_with(' ')
        || name.contains([':', '\\', '/', '\0'])
        || name.chars().any(char::is_control)
        || reserved_device_name(name)
    {
        return Err(format!("{label} contains an invalid filename"));
    }
    Ok(())
}

fn reserved_device_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                suffix.chars().count() == 1
                    && matches!(
                        suffix,
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    )
            })
        })
}

fn ensure_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("{label} {}: {error}", path.display()))?;
    if !regular_directory(&metadata) {
        return Err(format!("{label} must contain only non-reparse directories"));
    }
    Ok(())
}

fn open_without_following(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options.open(path)
}

fn regular_non_reparse(metadata: &Metadata) -> bool {
    if !metadata.is_file() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
    }
    #[cfg(not(windows))]
    {
        !metadata.file_type().is_symlink()
    }
}

fn regular_directory(metadata: &Metadata) -> bool {
    if !metadata.is_dir() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
    }
    #[cfg(not(windows))]
    {
        !metadata.file_type().is_symlink()
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn validate_expected(expected: Option<&str>) -> Result<(), String> {
    if expected.is_some_and(|value| !valid_digest(value)) {
        return Err("expected mesh art sidecar digest must be lowercase SHA-256".into());
    }
    Ok(())
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
#[path = "mesh_art/tests.rs"]
mod tests;
