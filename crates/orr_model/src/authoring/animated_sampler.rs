//! Offline animated material sampler authoring; no runtime, package-store or GPU changes.
//! Filesystem operations reuse the existing Linux x86_64 bounded, link-free helpers.
#![allow(clippy::float_arithmetic)]
use crate::{
    animation::AnimatedModel,
    animation_sequence::{
        checked_path, cooked, digest, logical_path, output_path, publish, read_input,
    },
    Dependency, Wrap, MAX_FILE_BYTES,
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
    linear_filter: bool,
    #[serde(deserialize_with = "read_wrap")]
    wrap_s: Wrap,
    #[serde(deserialize_with = "read_wrap")]
    wrap_t: Wrap,
}
// String-only decoding excludes serde enum object aliases such as {"clamp":null}.
fn read_wrap<'de, D: Deserializer<'de>>(decoder: D) -> Result<Wrap, D::Error> {
    match String::deserialize(decoder)?.as_str() {
        "clamp" => Ok(Wrap::Clamp),
        "repeat" => Ok(Wrap::Repeat),
        "mirror" => Ok(Wrap::Mirror),
        value => Err(serde::de::Error::unknown_variant(
            value,
            &["clamp", "repeat", "mirror"],
        )),
    }
}
fn parse_recipe(bytes: &[u8], asset_id: &str) -> Result<Recipe, String> {
    if bytes.len() > MAX_RECIPE_BYTES {
        return Err("recipe exceeds 16 KiB limit".into());
    }
    struct Object;
    impl<'de> Visitor<'de> for Object {
        type Value = Recipe;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an animated material-sampler recipe object with seven required fields")
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
    if recipe.format != "orr_animated_material_sampler" || recipe.version != 1 {
        return Err("unsupported animated sampler recipe format/version".into());
    }
    logical_path(asset_id, ".sampler.json")?;
    logical_path(&recipe.source, ".animated.json")?;
    Ok(recipe)
}
/// Replace one used material's three sampler defaults, preserving images,
/// color/alpha, geometry, node/skin data and every clip. Resolves direct input only.
pub fn author_sampler(
    recipe_bytes: &[u8],
    recipe_asset_id: &str,
    mut resolver: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<AnimatedModel, String> {
    let recipe = parse_recipe(recipe_bytes, recipe_asset_id)?;
    let bytes = resolver(&recipe.source)?;
    recipe_bytes
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
    let mut result = source.clone();
    result.materials[material].linear_filter = recipe.linear_filter;
    result.materials[material].wrap_s = recipe.wrap_s;
    result.materials[material].wrap_t = recipe.wrap_t;
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
    ];
    let model = AnimatedModel::new(result)
        .map_err(|e| format!("sampler-authored animated model validation: {e}"))?;
    let encoded = cooked(&model)?;
    let reloaded = AnimatedModel::from_bytes(&encoded)
        .map_err(|e| format!("sampler-authored animated cooked reload: {e}"))?;
    if cooked(&reloaded)? != encoded {
        return Err("sampler-authored animated model changed during cooked round-trip".into());
    }
    Ok(reloaded)
}
/// In-process bounded authoring from stable source/recipe files. No publication.
pub fn sampler_recipe(recipe_path: &Path, asset_id: &str) -> Result<Vec<u8>, String> {
    let recipe_path = checked_path(recipe_path, false)?;
    let recipe = read_input(&recipe_path, MAX_RECIPE_BYTES)?;
    let parent = recipe_path.parent().ok_or("recipe parent missing")?;
    let mut direct = None;
    let model = author_sampler(&recipe, asset_id, |relative| {
        let path = parent.join(relative);
        let bytes = read_input(&path, MAX_FILE_BYTES - recipe.len())?;
        direct = Some((path, bytes.clone()));
        Ok(bytes)
    })?;
    let bytes = cooked(&model)?;
    if read_input(&recipe_path, MAX_RECIPE_BYTES)? != recipe {
        return Err("recipe changed during animated material sampler authoring".into());
    }
    let (path, source) = direct.ok_or("direct source was not resolved")?;
    if read_input(&path, MAX_FILE_BYTES - recipe.len())? != source {
        return Err("source changed during animated material sampler authoring".into());
    }
    Ok(bytes)
}
/// Strict offline CLI. New output only; publication uses the existing no-replace
/// helper and never installs, invokes a compiler/script, or resolves provenance.
pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let args: Vec<_> = args.into_iter().take(8).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        return writeln!(io::stdout(),"Usage: animated_model_sampler --recipe FILE.sampler.json --asset-id variants/color.sampler.json --output NEW.animated.json\nRequires animation,import. Replaces one used material's linear_filter/wrap_s/wrap_t; wrap values clamp/repeat/mirror. Images/color/alpha/geometry/nodes/skins/clips retained. No mipmaps/anisotropy/separate min-mag filters. Linux x86_64 filesystem CLI; trusted stable link-free ancestry; absent output; no overwrite/install. Exit 0: help/published (check warnings); 2: error/not published.").map_err(|e|e.to_string());
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
    let bytes = sampler_recipe(Path::new(get("--recipe")?), identity)?;
    publish(&output, &bytes)?;
    let _ = writeln!(
        io::stdout(),
        "published {} bytes to {}",
        bytes.len(),
        output.display()
    );
    Ok(())
}
