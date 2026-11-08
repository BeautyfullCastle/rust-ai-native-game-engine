#[path="model_package_import/sampler.rs"]
mod sampler;
pub(crate) use sampler::{SamplerVariant,import_sampler_variant,import_sampler_variant_checked};
use sampler::{validate_sampler_variant,sampler_variant_bytes};
use crate::{model_bindings::ModelKind, package_manager};
use orr_package::Manifest;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

const MAX_SOURCE_BYTES: u64 = orr_model::MAX_FILE_BYTES as u64;
const MAX_LICENSE_BYTES: u64 = 1024 * 1024;
const MAX_PATH_BYTES: usize = 4096;
const MAX_PATH_COMPONENTS: usize = 64;
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SourceFormat {
    #[default]
    RawGlb,
    CookedStatic,
    CookedAnimated,
}

impl SourceFormat {
    fn asset_name(self) -> &'static str {
        match self {
            Self::RawGlb => "model.glb",
            Self::CookedStatic => "model.orrmodel",
            Self::CookedAnimated => "model.animated.json",
        }
    }

    fn source_label(self) -> &'static str {
        match self {
            Self::RawGlb => "GLB source",
            Self::CookedStatic => "cooked static model source",
            Self::CookedAnimated => "cooked animated model source",
        }
    }
}

pub(crate) struct Request {
    pub source: PathBuf,
    pub license: PathBuf,
    pub destination: PathBuf,
    pub name: String,
    pub version: String,
    pub kind: ModelKind,
    pub source_format: SourceFormat,
    pub license_acknowledged: bool,
}

/// Explicit, immutable authoring request. Source data and installed assets stay unchanged.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ColorVariant {
    pub material: u32,
    pub linear_rgb: [f64; 3],
    pub asset_id: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TextureVariant {
    pub material: u32,
    pub texture: PathBuf,
    pub asset_id: String,
}

/// Append a source clip segment without changing any existing clip index.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TrimVariant {
    pub clip: u32,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub name: String,
    pub asset_id: String,
}

enum AuthoringVariant {
    Color(ColorVariant),
    Texture(TextureVariant),
    Trim(TrimVariant),
    Sampler(SamplerVariant),
}

pub(crate) struct Report {
    pub root: PathBuf,
    pub package: String,
    pub asset: String,
    pub kind: ModelKind,
    /// SHA-256 of the published model payload (the cooked variant when authored).
    pub source_sha256: String,
    pub license_sha256: String,
}

struct InputBytes {
    bytes: Vec<u8>,
    digest: String,
}

pub(crate) fn import_package(request: Request) -> Result<Report, String> {
    import_package_inner(request, None, None)
}

pub(crate) fn import_color_variant(
    request: Request,
    variant: ColorVariant,
) -> Result<Report, String> {
    import_color_variant_checked(request, variant, None)
}

pub(crate) fn import_color_variant_checked(
    request: Request,
    variant: ColorVariant,
    expected_source_sha256: Option<String>,
) -> Result<Report, String> {
    validate_expected_source_hash(expected_source_sha256.as_deref())?;
    // All option/feature/license admission precedes any filesystem access.
    if !request.license_acknowledged {
        return Err("explicit license acknowledgement is required".into());
    }
    if request.source_format != SourceFormat::CookedAnimated || request.kind != ModelKind::Animated
    {
        return Err("color variant requires CookedAnimated source and Animated model mode".into());
    }
    if !cfg!(feature = "room-animated-models") {
        return Err(
            "color variant requires room-animated-models; no input or destination opened".into(),
        );
    }
    validate_color_variant(&variant)?;
    import_package_inner(
        request,
        Some(AuthoringVariant::Color(variant)),
        expected_source_sha256,
    )
}

pub(crate) fn import_texture_variant(
    request: Request,
    variant: TextureVariant,
) -> Result<Report, String> {
    import_texture_variant_checked(request, variant, None)
}

pub(crate) fn import_texture_variant_checked(
    request: Request,
    variant: TextureVariant,
    expected_source_sha256: Option<String>,
) -> Result<Report, String> {
    validate_expected_source_hash(expected_source_sha256.as_deref())?;
    if !request.license_acknowledged {
        return Err("explicit license acknowledgement is required".into());
    }
    if request.source_format != SourceFormat::CookedAnimated || request.kind != ModelKind::Animated
    {
        return Err(
            "texture variant requires CookedAnimated source and Animated model mode".into(),
        );
    }
    if !cfg!(feature = "room-animated-models") {
        return Err(
            "texture variant requires room-animated-models; no input or destination opened".into(),
        );
    }
    validate_texture_variant(&variant)?;
    import_package_inner(
        request,
        Some(AuthoringVariant::Texture(variant)),
        expected_source_sha256,
    )
}

pub(crate) fn import_trim_variant(request: Request, variant: TrimVariant) -> Result<Report, String> {
    import_trim_variant_checked(request, variant, None)
}

pub(crate) fn import_trim_variant_checked(
    request: Request, variant: TrimVariant, expected_source_sha256: Option<String>,
) -> Result<Report, String> {
    validate_expected_source_hash(expected_source_sha256.as_deref())?;
    if !request.license_acknowledged { return Err("explicit license acknowledgement is required".into()); }
    if request.source_format != SourceFormat::CookedAnimated || request.kind != ModelKind::Animated {
        return Err("clip trim requires CookedAnimated source and Animated model mode".into());
    }
    if !cfg!(feature = "room-animated-models") {
        return Err("clip trim requires room-animated-models; no input or destination opened".into());
    }
    validate_trim_variant(&variant)?;
    import_package_inner(request, Some(AuthoringVariant::Trim(variant)), expected_source_sha256)
}

fn validate_trim_variant(variant: &TrimVariant) -> Result<(), String> {
    validate_variant_identity(&variant.asset_id, ".trim.json")?;
    if variant.name.trim().is_empty() || variant.name.len() > 128 || variant.name.chars().any(char::is_control) {
        return Err("trim clip name must be nonempty, at most 128 UTF-8 bytes, without controls".into());
    }
    if !variant.start_seconds.is_finite() || !variant.end_seconds.is_finite()
        || variant.start_seconds < 0.0 || variant.end_seconds <= variant.start_seconds || variant.end_seconds > 3600.0
        || (variant.end_seconds as f32) <= (variant.start_seconds as f32) {
        return Err("trim range must be finite 0 <= start < end <= 3600 and distinct after f32 conversion".into());
    }
    #[cfg(feature = "room-animated-models")]
    if variant.clip as usize >= orr_model::animation::MAX_CLIPS { return Err("trim clip index exceeds the model clip limit".into()); }
    Ok(())
}

fn trim_variant_bytes(source: &[u8], variant: &TrimVariant) -> Result<Vec<u8>, String> {
    validate_trim_variant(variant)?;
    #[cfg(feature = "room-animated-models")]
    {
        let recipe = serde_json::to_vec(&serde_json::json!({
            "format":"orr_animation_clip_trim", "version":1, "source":"input.animated.json",
            "clip":variant.clip, "start_seconds":variant.start_seconds, "end_seconds":variant.end_seconds,"name":variant.name,
        })).map_err(|error|format!("clip trim recipe: {error}"))?;
        let model = orr_model::animation_trim::trim_clip(&recipe,&variant.asset_id,|path| {
            if path != "input.animated.json" { return Err("unexpected clip trim dependency".into()); }
            Ok(source.to_vec())
        })?;
        model.to_bytes().map_err(|error|format!("clip trim cook: {error}"))
    }
    #[cfg(not(feature = "room-animated-models"))]
    { let _ = source; Err("clip trim requires room-animated-models".into()) }
}

fn validate_texture_variant(variant: &TextureVariant) -> Result<(), String> {
    validate_variant_identity(&variant.asset_id, ".retexture.json")?;
    validate_path_shape(&variant.texture, "PNG texture")?;
    if !variant
        .texture
        .extension()
        .and_then(|v| v.to_str())
        .is_some_and(|v| v.eq_ignore_ascii_case("png"))
    {
        return Err("texture variant requires a .png file".into());
    }
    Ok(())
}

fn texture_variant_bytes(
    source: &[u8],
    png: &[u8],
    variant: &TextureVariant,
) -> Result<Vec<u8>, String> {
    #[cfg(feature = "room-animated-models")]
    {
        let recipe = serde_json::to_vec(
            &serde_json::json!({"format":"orr_animated_material_texture","version":1,
            "source":"input.animated.json","texture":"input.png","material":variant.material}),
        )
        .map_err(|e| format!("texture recipe: {e}"))?;
        let model =
            orr_model::animated_retexture::retexture_model(&recipe, &variant.asset_id, |path| {
                match path {
                    "input.animated.json" => Ok(source.to_vec()),
                    "input.png" => Ok(png.to_vec()),
                    _ => Err("unexpected texture variant dependency".into()),
                }
            })?;
        model
            .to_bytes()
            .map_err(|e| format!("texture variant cook: {e}"))
    }
    #[cfg(not(feature = "room-animated-models"))]
    {
        let _ = (source, png, variant);
        Err("texture variant requires room-animated-models".into())
    }
}

fn validate_color_variant(variant: &ColorVariant) -> Result<(), String> {
    if !variant
        .linear_rgb
        .iter()
        .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
    {
        return Err("linear_rgb must contain three finite linear-light values in 0..=1".into());
    }
    validate_variant_identity(&variant.asset_id, ".recolor.json")
}

fn validate_variant_identity(id: &str, suffix: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 240
        || !id.ends_with(suffix)
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._/".contains(&b))
    {
        return Err(format!(
            "variant identity must be a portable relative ASCII {suffix} path of at most 240 bytes"
        ));
    }
    for part in id.split('/') {
        let device = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if part.is_empty()
            || part.starts_with('.')
            || part.ends_with('.')
            || ["CON", "PRN", "AUX", "NUL"].contains(&device.as_str())
            || (device.len() == 4
                && (device.starts_with("COM") || device.starts_with("LPT"))
                && device.as_bytes()[3].is_ascii_digit())
        {
            return Err(
                "variant identity contains an empty, hidden, traversal or reserved segment".into(),
            );
        }
    }
    Ok(())
}

fn color_variant_bytes(source: &[u8], variant: &ColorVariant) -> Result<Vec<u8>, String> {
    #[cfg(feature = "room-animated-models")]
    {
        let recipe = serde_json::to_vec(&serde_json::json!({
            "format":"orr_animated_material_color", "version":1,
            "source":"input.animated.json", "material":variant.material,
            "linear_rgb":variant.linear_rgb
        }))
        .map_err(|e| format!("color recipe: {e}"))?;
        let model =
            orr_model::animated_recolor::recolor_model(&recipe, &variant.asset_id, |path| {
                if path != "input.animated.json" {
                    return Err("unexpected color variant dependency".into());
                }
                Ok(source.to_vec())
            })?;
        model
            .to_bytes()
            .map_err(|e| format!("color variant cook: {e}"))
    }
    #[cfg(not(feature = "room-animated-models"))]
    {
        let _ = (source, variant);
        Err("color variant requires room-animated-models".into())
    }
}

fn import_package_inner(
    request: Request,
    variant: Option<AuthoringVariant>,
    expected_source_sha256: Option<String>,
) -> Result<Report, String> {
    if !request.license_acknowledged {
        return Err("explicit license acknowledgement is required".into());
    }
    if request.source_format == SourceFormat::CookedAnimated {
        if request.kind != ModelKind::Animated {
            return Err("cooked animated model import requires Animated model mode".into());
        }
        if !cfg!(feature = "room-animated-models") {
            return Err("cooked animated model import requires room-animated-models; no input or destination opened".into());
        }
    }
    validate_destination_policy(&request.destination)?;
    let (destination_parent, destination) = checked_new_directory(&request.destination)?;
    validate_destination_policy(&destination)?;
    if request.source_format == SourceFormat::CookedStatic && request.kind != ModelKind::Static {
        return Err("cooked static model import requires Static model mode".into());
    }
    let source_label = request.source_format.source_label();
    let asset_name = request.source_format.asset_name();
    let source_path = checked_file_path(&request.source, source_label)?;
    let license_path = checked_file_path(&request.license, "license source")?;
    match request.source_format {
        SourceFormat::RawGlb => {
            if !source_path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("glb"))
            {
                return Err("source must be a self-contained .glb file".into());
            }
        }
        SourceFormat::CookedAnimated => {
            if !source_path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.to_ascii_lowercase().ends_with(".animated.json"))
            {
                return Err("cooked animated source must be a .animated.json file".into());
            }
        }
        SourceFormat::CookedStatic => {
            if !source_path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.to_ascii_lowercase().ends_with(".model.json"))
            {
                return Err("cooked static source must be a .model.json file".into());
            }
        }
    }

    let mut source = read_bounded_file(&source_path, MAX_SOURCE_BYTES, source_label)?;
    if expected_source_sha256
        .as_deref()
        .is_some_and(|expected| !expected.eq_ignore_ascii_case(&source.digest))
    {
        return Err("inspected animated source changed; inspect again before importing".into());
    }
    let license = read_bounded_file(&license_path, MAX_LICENSE_BYTES, "license source")?;
    if std::str::from_utf8(&license.bytes)
        .ok()
        .is_none_or(|text| text.trim().is_empty())
    {
        return Err("license must be nonempty UTF-8 text".into());
    }
    match request.source_format {
        SourceFormat::RawGlb => validate_glb(&source.bytes, request.kind)?,
        SourceFormat::CookedAnimated => validate_cooked_animated(&source.bytes)?,
        SourceFormat::CookedStatic => {
            // Validate the cooked payload directly. Dependency digests describe its
            // history; they are neither resolver inputs nor package dependencies.
            orr_model::StaticModel::from_bytes(&source.bytes)
                .map_err(|error| format!("cooked static model import: {error}"))?;
        }
    }

    if let Some(variant) = variant {
        let bytes = match variant {
            AuthoringVariant::Color(spec) => color_variant_bytes(&source.bytes, &spec)?,
            AuthoringVariant::Trim(spec) => trim_variant_bytes(&source.bytes, &spec)?,
            AuthoringVariant::Sampler(spec) => sampler_variant_bytes(&source.bytes, &spec)?,
            AuthoringVariant::Texture(spec) => {
                let path = checked_file_path(&spec.texture, "PNG texture")?;
                let png = read_bounded_file(&path, MAX_SOURCE_BYTES, "PNG texture")?;
                texture_variant_bytes(&source.bytes, &png.bytes, &spec)?
            }
        };
        if bytes.len() as u64 > MAX_SOURCE_BYTES {
            return Err("authored variant cooked output exceeds 64 MiB limit".into());
        }
        source = InputBytes {
            digest: digest(&bytes),
            bytes,
        };
    }

    let stage = tempfile::Builder::new()
        .prefix(".orr-model-import-")
        .tempdir_in(&destination_parent)
        .map_err(|error| format!("create package staging directory: {error}"))?;
    let manifest = package_manifest(&request, asset_name)?;
    let manifest_bytes = serde_json::to_vec(&manifest)
        .map_err(|error| format!("serialize package manifest: {error}"))?;

    write_new(&stage.path().join(asset_name), &source.bytes)?;
    write_new(&stage.path().join("LICENSE.txt"), &license.bytes)?;
    write_new(&stage.path().join("orr.package.json"), &manifest_bytes)?;
    let runtime = orr_sample::room_project::compiled_runtime();
    let inspected = orr_package::inspect(stage.path(), &runtime)
        .map_err(|error| format!("package validation: {error}"))?;
    if inspected.manifest != manifest {
        return Err("package manifest changed during validation".into());
    }
    let parent_after_stage =
        package_manager::checked_directory(&destination_parent, "package destination parent")?;
    if parent_after_stage != destination_parent {
        return Err("package destination parent changed during import".into());
    }

    publish_directory(
        stage.path(),
        &destination,
        asset_name,
        &source.bytes,
        &license.bytes,
        &manifest_bytes,
    )?;
    Ok(Report {
        root: destination,
        package: request.name,
        asset: asset_name.into(),
        kind: request.kind,
        source_sha256: source.digest,
        license_sha256: license.digest,
    })
}

fn package_manifest(request: &Request, asset_name: &str) -> Result<Manifest, String> {
    let capabilities = match request.kind {
        ModelKind::Static => BTreeSet::from(["models".to_owned()]),
        ModelKind::Animated => {
            #[cfg(feature = "room-animated-models")]
            {
                BTreeSet::from(["animation".to_owned(), "models".to_owned()])
            }
            #[cfg(not(feature = "room-animated-models"))]
            return Err("animated model package import requires room-animated-models".into());
        }
    };
    let manifest = Manifest {
        schema: 1,
        name: request.name.clone(),
        version: request.version.clone(),
        engine: format!(
            "={}",
            orr_sample::room_project::compiled_runtime().engine_version
        ),
        capabilities,
        dependencies: BTreeMap::new(),
        files: BTreeSet::from(["LICENSE.txt".into(), asset_name.into()]),
    };
    Ok(manifest)
}

fn validate_cooked_animated(bytes: &[u8]) -> Result<(), String> {
    #[cfg(feature = "room-animated-models")]
    {
        let model = orr_model::animation::AnimatedModel::from_bytes(bytes)
            .map_err(|error| format!("cooked animated model import: {error}"))?;
        if model.source().clips.is_empty() {
            return Err("cooked animated model must contain at least one animation clip".into());
        }
        Ok(())
    }
    #[cfg(not(feature = "room-animated-models"))]
    {
        let _ = bytes;
        Err("cooked animated model import requires room-animated-models".into())
    }
}

fn validate_glb(bytes: &[u8], kind: ModelKind) -> Result<(), String> {
    match kind {
        ModelKind::Static => {
            orr_model::import::import_with_resolver("model.glb", bytes, |_| {
                Err(orr_model::Error::message(
                    "external resources are unsupported for raw model package import",
                ))
            })
            .map_err(|error| format!("static GLB import: {error}"))?;
        }
        ModelKind::Animated => {
            #[cfg(feature = "room-animated-models")]
            {
                let model =
                    orr_model::animation_import::import_with_resolver("model.glb", bytes, |_| {
                        Err(orr_model::Error::message(
                            "external resources are unsupported for raw model package import",
                        ))
                    })
                    .map_err(|error| format!("animated GLB import: {error}"))?;
                if model.source().clips.is_empty() {
                    return Err("animated GLB must contain at least one animation clip".into());
                }
            }
            #[cfg(not(feature = "room-animated-models"))]
            return Err("animated model package import requires room-animated-models".into());
        }
    }
    Ok(())
}

fn checked_file_path(path: &Path, label: &str) -> Result<PathBuf, String> {
    validate_path_shape(path, label)?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("{label} needs a parent"))?;
    let parent = package_manager::checked_directory(parent, label)?;
    let name = path
        .file_name()
        .ok_or_else(|| format!("{label} needs a filename"))?;
    let canonical = parent.join(name);
    let metadata =
        fs::symlink_metadata(&canonical).map_err(|error| format!("{label} metadata: {error}"))?;
    if !regular_non_reparse(&metadata) {
        return Err(format!("{label} must be a regular non-reparse file"));
    }
    Ok(canonical)
}

fn read_bounded_file(path: &Path, limit: u64, label: &str) -> Result<InputBytes, String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("{label} metadata: {error}"))?;
    if !regular_non_reparse(&metadata) || metadata.len() > limit {
        return Err(format!(
            "{label} must be a regular file within its byte limit"
        ));
    }
    let mut file = open_without_following(path).map_err(|error| format!("{label}: {error}"))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("{label} opened metadata: {error}"))?;
    if !regular_non_reparse(&opened) || opened.len() > limit {
        return Err(format!(
            "{label} changed to a non-regular or oversized file"
        ));
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    Read::by_ref(&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{label} read: {error}"))?;
    let after = fs::symlink_metadata(path).map_err(|error| format!("{label} recheck: {error}"))?;
    let handle_after = file
        .metadata()
        .map_err(|error| format!("{label} opened recheck: {error}"))?;
    if !regular_non_reparse(&after)
        || !regular_non_reparse(&handle_after)
        || bytes.len() as u64 > limit
        || bytes.len() as u64 != opened.len()
        || !same_file_observation(&metadata, &opened)
        || !same_file_observation(&opened, &handle_after)
        || !same_file_observation(&metadata, &after)
        || !same_file_observation(&after, &handle_after)
    {
        return Err(format!("{label} changed while it was read"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("{label} needs a parent"))?;
    let checked_parent = package_manager::checked_directory(parent, label)?;
    if checked_parent != parent {
        return Err(format!("{label} parent changed while it was read"));
    }
    let digest = digest(&bytes);
    Ok(InputBytes { bytes, digest })
}

fn same_file_observation(first: &fs::Metadata, second: &fs::Metadata) -> bool {
    first.len() == second.len()
        && match (first.modified(), second.modified()) {
            (Ok(first_modified), Ok(second_modified)) => first_modified == second_modified,
            _ => true,
        }
}

fn validate_path_shape(path: &Path, label: &str) -> Result<(), String> {
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
    for component in components {
        if let Component::Normal(name) = component {
            validate_leaf(
                name.to_str()
                    .ok_or_else(|| format!("{label} path must be Unicode"))?,
                label,
            )?;
        }
    }
    Ok(())
}

fn validate_leaf(name: &str, label: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 200
        || matches!(name.chars().last(), Some('.' | ' '))
        || name.contains([':', '\\', '/', '\0'])
        || name.chars().any(char::is_control)
        || reserved_device_name(name)
    {
        return Err(format!("{label} path contains an invalid Windows filename"));
    }
    Ok(())
}

fn reserved_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default();
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            upper.strip_prefix(prefix).is_some_and(|suffix| {
                suffix.chars().count() == 1
                    && matches!(
                        suffix,
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    )
            })
        })
}

fn validate_destination_policy(path: &Path) -> Result<(), String> {
    validate_path_shape(path, "package destination")?;
    if path.components().any(|component| {
        matches!(component, Component::Normal(name) if name.to_string_lossy().eq_ignore_ascii_case(".orr"))
    }) {
        return Err("package destination cannot be inside the .orr store".into());
    }
    Ok(())
}

fn checked_new_directory(path: &Path) -> Result<(PathBuf, PathBuf), String> {
    let parent = path
        .parent()
        .ok_or("package destination needs an existing parent")?;
    let parent = package_manager::checked_directory(parent, "package destination parent")?;
    let name = path
        .file_name()
        .ok_or("package destination needs a directory name")?;
    let destination = parent.join(name);
    match fs::symlink_metadata(&destination) {
        Ok(_) => return Err("package destination already exists".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("package destination: {error}")),
    }
    Ok((parent, destination))
}

fn publish_directory(
    stage: &Path,
    destination: &Path,
    asset_name: &str,
    source: &[u8],
    license: &[u8],
    manifest: &[u8],
) -> Result<(), String> {
    fs::create_dir(destination).map_err(|error| format!("create package destination: {error}"))?;
    let mut created = Vec::new();
    let result = (|| {
        for (name, bytes) in [(asset_name, source), ("LICENSE.txt", license)] {
            let staged = stage.join(name);
            let staged_bytes =
                fs::read(&staged).map_err(|error| format!("read staged {name}: {error}"))?;
            if staged_bytes != bytes {
                return Err(format!("staged {name} changed before publication"));
            }
            let path = destination.join(name);
            write_new_tracked(&path, bytes, &mut created)?;
        }
        let manifest_path = destination.join("orr.package.json");
        write_new_tracked(&manifest_path, manifest, &mut created)?;
        Ok(())
    })();
    if let Err(error) = result {
        let cleanup = cleanup_owned_output(destination, &created);
        return Err(match cleanup {
            Ok(()) => error,
            Err(cleanup_error) => {
                format!("{error}; partial output cleanup failed: {cleanup_error}")
            }
        });
    }
    Ok(())
}

fn cleanup_owned_output(destination: &Path, created: &[PathBuf]) -> Result<(), String> {
    let mut errors = Vec::new();
    for path in created.iter().rev() {
        if let Err(error) = fs::remove_file(path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                errors.push(format!("{}: {error}", path.display()));
            }
        }
    }
    if let Err(error) = fs::remove_dir(destination) {
        if error.kind() != std::io::ErrorKind::NotFound {
            errors.push(format!("{}: {error}", destination.display()));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("create {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("write {}: {error}", path.display()))
}

fn write_new_tracked(path: &Path, bytes: &[u8], created: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("create {}: {error}", path.display()))?;
    created.push(path.to_path_buf());
    file.write_all(bytes)
        .map_err(|error| format!("write {}: {error}", path.display()))
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

fn regular_non_reparse(metadata: &fs::Metadata) -> bool {
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

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
#[path = "model_package_import/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "model_package_import/cooked_animated_tests.rs"]
mod cooked_animated_tests;

#[cfg(test)]
#[path = "model_package_import/animated_color_tests.rs"]
mod animated_color_tests;

#[cfg(test)]
#[path = "model_package_import/animated_texture_tests.rs"]
mod animated_texture_tests;

fn validate_expected_source_hash(expected: Option<&str>) -> Result<(), String> {
    if expected.is_some_and(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit())) {
        return Err(
            "expected source SHA-256 must contain exactly 64 hexadecimal characters".into(),
        );
    }
    Ok(())
}
#[path = "model_package_import/source_inspection.rs"]
pub(crate) mod source_inspection;
#[cfg(test)]
#[path = "model_package_import/source_inspection_tests.rs"]
mod source_inspection_tests;

#[cfg(feature = "room-animated-models")]
#[path = "model_package_import/variant_preview.rs"]
pub(crate) mod variant_preview;

#[cfg(test)]
#[path = "model_package_import/animation_trim.rs"]
mod animation_trim_tests;
