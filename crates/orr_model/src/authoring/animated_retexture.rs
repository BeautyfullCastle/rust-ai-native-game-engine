//! Offline animated material PNG authoring; no runtime, package-store or GPU changes.
//! Filesystem operations reuse the existing Linux x86_64 bounded, link-free helpers.
#![allow(clippy::float_arithmetic)]
use crate::{
    animation::AnimatedModel,
    animation_sequence::{
        checked_path, cooked, digest, logical_path, output_path, publish, read_input,
    },
    static_retexture::decode_png,
    Dependency, MAX_DECODED_BYTES, MAX_FILE_BYTES, MAX_IMAGES,
};
use serde::{
    de::{MapAccess, Visitor},
    Deserialize, Deserializer,
};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fmt,
    io::{self, Write},
    path::Path,
};

pub const MAX_RECIPE_BYTES: usize = 16 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recipe {
    format: String,
    version: u32,
    source: String,
    material: u32,
    texture: String,
}
fn parse_recipe(bytes: &[u8], asset_id: &str) -> Result<Recipe, String> {
    if bytes.len() > MAX_RECIPE_BYTES {
        return Err("recipe exceeds 16 KiB limit".into());
    }
    struct Object;
    impl<'de> Visitor<'de> for Object {
        type Value = Recipe;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an animated material-texture recipe object with five required fields")
        }
        fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Recipe, M::Error> {
            Recipe::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let recipe = decoder
        .deserialize_map(Object)
        .map_err(|e| format!("recipe JSON: {e}"))?;
    decoder.end().map_err(|e| format!("recipe JSON: {e}"))?;
    if recipe.format != "orr_animated_material_texture" || recipe.version != 1 {
        return Err("unsupported animated texture recipe format/version".into());
    }
    logical_path(asset_id, ".retexture.json")?;
    logical_path(&recipe.source, ".animated.json")?;
    logical_path(&recipe.texture, ".png")?;
    if recipe.source.eq_ignore_ascii_case(&recipe.texture)
        || asset_id.eq_ignore_ascii_case(&recipe.source)
        || asset_id.eq_ignore_ascii_case(&recipe.texture)
    {
        return Err("recipe identity and direct input paths must be distinct".into());
    }
    Ok(recipe)
}
/// Append one PNG and redirect one used material, preserving color/alpha factors,
/// sampling, geometry, node/skin data and every clip. Only direct inputs resolve.
pub fn retexture_model(
    recipe_bytes: &[u8],
    recipe_asset_id: &str,
    mut resolver: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<AnimatedModel, String> {
    let recipe = parse_recipe(recipe_bytes, recipe_asset_id)?;
    let bytes = resolver(&recipe.source)?;
    let total = recipe_bytes
        .len()
        .checked_add(bytes.len())
        .filter(|&size| size <= MAX_FILE_BYTES)
        .ok_or("recipe plus source exceeds 64 MiB input budget")?;
    let original =
        AnimatedModel::from_bytes(&bytes).map_err(|e| format!("source animated model: {e}"))?;
    let source = original.source();
    if source.clips.is_empty() {
        return Err("source animated model must contain at least one animation clip".into());
    }
    if recipe_asset_id.eq_ignore_ascii_case(&source.asset_id) {
        return Err("new asset identity must differ from source identity".into());
    }
    let material = usize::try_from(recipe.material).map_err(|_| "material index overflow")?;
    if material >= source.materials.len()
        || !source
            .primitives
            .iter()
            .any(|primitive| primitive.material == recipe.material)
    {
        return Err("material index must name a material used by a primitive".into());
    }
    if source.images.len() >= MAX_IMAGES {
        return Err("appended image exceeds image count limit".into());
    }
    let decoded: usize = source.images.iter().map(|image| image.rgba8.len()).sum();
    let remaining = MAX_DECODED_BYTES
        .checked_sub(decoded)
        .ok_or("decoded image budget exceeded")?;
    let texture_bytes = resolver(&recipe.texture)?;
    total
        .checked_add(texture_bytes.len())
        .filter(|&size| size <= MAX_FILE_BYTES)
        .ok_or("recipe plus source plus PNG exceeds 64 MiB input budget")?;
    let image = decode_png(&texture_bytes, remaining)?;
    let mut result = source.clone();
    result.materials[material].image = result.images.len() as u32;
    result.images.push(image);
    for primitive in &mut result.primitives {
        let suffix = primitive
            .id
            .strip_prefix(&source.asset_id)
            .filter(|suffix| suffix.starts_with("#node="))
            .ok_or("unexpected source primitive identity")?;
        primitive.id = format!("{recipe_asset_id}{suffix}");
    }
    result.asset_id = recipe_asset_id.into();
    result.dependencies = vec![
        Dependency {
            uri: "$source".into(),
            sha256: digest(recipe_bytes),
        },
        Dependency {
            uri: recipe.source,
            sha256: digest(&bytes),
        },
        Dependency {
            uri: recipe.texture,
            sha256: digest(&texture_bytes),
        },
    ];
    let model = AnimatedModel::new(result)
        .map_err(|e| format!("retextured animated model validation: {e}"))?;
    let encoded = cooked(&model)?;
    let reloaded = AnimatedModel::from_bytes(&encoded)
        .map_err(|e| format!("retextured animated cooked reload: {e}"))?;
    if cooked(&reloaded)? != encoded {
        return Err("retextured animated model changed during cooked round-trip".into());
    }
    Ok(reloaded)
}
/// In-process bounded authoring from stable source/recipe files. No publication.
pub fn retexture_recipe(recipe_path: &Path, asset_id: &str) -> Result<Vec<u8>, String> {
    let recipe_path = checked_path(recipe_path, false)?;
    let recipe = read_input(&recipe_path, MAX_RECIPE_BYTES)?;
    let parent = recipe_path.parent().ok_or("recipe parent missing")?;
    let mut direct = Vec::new();
    let model = retexture_model(&recipe, asset_id, |relative| {
        let path = parent.join(relative);
        let bytes = read_input(&path, MAX_FILE_BYTES - recipe.len())?;
        direct.push((path, bytes.clone()));
        Ok(bytes)
    })?;
    let bytes = cooked(&model)?;
    if read_input(&recipe_path, MAX_RECIPE_BYTES)? != recipe {
        return Err("recipe changed during animated material retexturing".into());
    }
    for (path, source) in direct {
        if read_input(&path, MAX_FILE_BYTES - recipe.len())? != source {
            return Err("direct input changed during animated material retexturing".into());
        }
    }
    Ok(bytes)
}
/// Strict offline CLI. New output only; publication uses the existing no-replace
/// helper and never installs, invokes a compiler/script, or resolves provenance.
pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let args: Vec<_> = args.into_iter().take(8).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        return writeln!(io::stdout(),"Usage: animated_model_retexture --recipe FILE.retexture.json --asset-id variants/color.retexture.json --output NEW.animated.json\nRequires animation,import. Appends one RGBA8 PNG and redirects one used material; factors/samplers/geometry/nodes/skins/clips retained. Linux x86_64 filesystem CLI; trusted stable link-free ancestry; absent output; no overwrite/install. Exit 0: help/published (check warnings); 2: error/not published.").map_err(|e|e.to_string());
    }
    if args.len() != 6 {
        return Err("expected three unique option/value pairs; use --help".into());
    }
    let mut options = BTreeMap::new();
    for pair in args.chunks_exact(2) {
        let key = pair[0].to_str().ok_or("option names must be UTF-8")?;
        if !["--recipe", "--asset-id", "--output"].contains(&key) {
            return Err(format!("unknown option {key}"));
        }
        if pair[1].as_os_str().is_empty() || pair[1].as_os_str().len() > 4096 {
            return Err(format!("{key} requires bounded nonempty text"));
        }
        if options.insert(key, &pair[1]).is_some() {
            return Err(format!("duplicate option {key}"));
        }
    }
    let get = |key: &str| {
        options
            .get(key)
            .copied()
            .ok_or_else(|| format!("missing {key}"))
    };
    let identity = get("--asset-id")?
        .to_str()
        .ok_or("asset identity must be UTF-8")?;
    let output = output_path(Path::new(get("--output")?))?;
    let bytes = retexture_recipe(Path::new(get("--recipe")?), identity)?;
    publish(&output, &bytes)?;
    let _ = writeln!(
        io::stdout(),
        "published {} bytes to {}",
        bytes.len(),
        output.display()
    );
    Ok(())
}
