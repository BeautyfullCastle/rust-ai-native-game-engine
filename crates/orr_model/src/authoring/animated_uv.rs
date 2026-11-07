//! Offline animated material UV authoring; no runtime, package-store or GPU changes.
//! Filesystem operations reuse the existing Linux x86_64 bounded, link-free helpers.
#![allow(clippy::float_arithmetic)]
use crate::{
    animation::AnimatedModel,
    animation_sequence::{
        checked_path, cooked, digest, logical_path, output_path, publish, read_input,
    },
    Dependency, MAX_FILE_BYTES,
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
    offset: [f64; 2],
    scale: [f64; 2],
    quarter_turns: u32,
}
fn parse_recipe(bytes: &[u8], asset_id: &str) -> Result<Recipe, String> {
    if bytes.len() > MAX_RECIPE_BYTES {
        return Err("recipe exceeds 16 KiB limit".into());
    }
    struct Object;
    impl<'de> Visitor<'de> for Object {
        type Value = Recipe;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an animated material-UV recipe object with seven required fields")
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
    if recipe.format != "orr_animated_material_uv" || recipe.version != 1 {
        return Err("unsupported animated UV recipe format/version".into());
    }
    logical_path(asset_id, ".uv.json")?;
    logical_path(&recipe.source, ".animated.json")?;
    // Admit the authored f64 values before narrowing: overflow must not be
    // rescued by rounding; positive scale must also survive f32 underflow.
    let bound = f64::from(f32::MAX);
    if !recipe
        .offset
        .iter()
        .all(|v| v.is_finite() && v.abs() <= bound)
        || !recipe
            .scale
            .iter()
            .all(|v| v.is_finite() && *v > 0.0 && *v <= bound && (*v as f32) > 0.0)
        || recipe.quarter_turns > 3
    {
        return Err("UV recipe requires finite f32-representable offset, positive finite f32 scale and quarter_turns 0..3".into());
    }
    Ok(recipe)
}
fn transformed(uv: [f32; 2], recipe: &Recipe) -> Result<[f32; 2], String> {
    let [x, y] = [
        uv[0] * recipe.scale[0] as f32,
        uv[1] * recipe.scale[1] as f32,
    ];
    if !x.is_finite() || !y.is_finite() {
        return Err("UV scale produces a nonfinite intermediate".into());
    }
    let rotated = match recipe.quarter_turns {
        0 => [x, y],
        1 => [-y, x],
        2 => [-x, -y],
        3 => [y, -x],
        _ => return Err("unsupported quarter turn".into()),
    };
    let result = [
        rotated[0] + recipe.offset[0] as f32,
        rotated[1] + recipe.offset[1] as f32,
    ];
    if !result.iter().all(|v| v.is_finite() && v.abs() <= 65536.0) {
        return Err("transformed UV must be finite and within +/-65536".into());
    }
    Ok(result)
}
/// Transform UV0 for all primitives using one material. Preserve all rig,
/// animation, geometry except UVs, image and material data; resolve direct input only.
pub fn transform_model_uv(
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
    // Exact identity bypasses arithmetic, including the sign of zero UVs.
    let identity =
        recipe.scale == [1.0, 1.0] && recipe.quarter_turns == 0 && recipe.offset == [0.0, 0.0];
    for primitive in &mut result.primitives {
        let suffix = primitive
            .id
            .strip_prefix(&source.asset_id)
            .filter(|suffix| suffix.starts_with("#node="))
            .ok_or("unexpected source primitive identity")?;
        primitive.id = format!("{recipe_asset_id}{suffix}");
        if primitive.material == recipe.material && !identity {
            for vertex in &mut primitive.vertices {
                vertex.vertex.uv = transformed(vertex.vertex.uv, &recipe)?;
            }
        }
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
        .map_err(|e| format!("UV-authored animated model validation: {e}"))?;
    let encoded = cooked(&model)?;
    let reloaded = AnimatedModel::from_bytes(&encoded)
        .map_err(|e| format!("UV-authored animated cooked reload: {e}"))?;
    if cooked(&reloaded)? != encoded {
        return Err("UV-authored animated model changed during cooked round-trip".into());
    }
    Ok(reloaded)
}
/// In-process bounded authoring from stable source/recipe files. No publication.
pub fn transform_recipe(recipe_path: &Path, asset_id: &str) -> Result<Vec<u8>, String> {
    let recipe_path = checked_path(recipe_path, false)?;
    let recipe = read_input(&recipe_path, MAX_RECIPE_BYTES)?;
    let parent = recipe_path.parent().ok_or("recipe parent missing")?;
    let mut direct = None;
    let model = transform_model_uv(&recipe, asset_id, |relative| {
        let path = parent.join(relative);
        let bytes = read_input(&path, MAX_FILE_BYTES - recipe.len())?;
        direct = Some((path, bytes.clone()));
        Ok(bytes)
    })?;
    let bytes = cooked(&model)?;
    if read_input(&recipe_path, MAX_RECIPE_BYTES)? != recipe {
        return Err("recipe changed during animated material UV authoring".into());
    }
    let (path, source) = direct.ok_or("direct source was not resolved")?;
    if read_input(&path, MAX_FILE_BYTES - recipe.len())? != source {
        return Err("source changed during animated material UV authoring".into());
    }
    Ok(bytes)
}
/// Strict offline CLI. New output only; publication uses the existing no-replace
/// helper and never installs, invokes a compiler/script, or resolves provenance.
pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let args: Vec<_> = args.into_iter().take(8).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        return writeln!(io::stdout(),"Usage: animated_model_uv --recipe FILE.uv.json --asset-id variants/color.uv.json --output NEW.animated.json\nRequires animation,import. Transforms one used material's UV0: positive scale, quarter_turns 0..3 about origin, then offset; finite results within +/-65536. Images/materials/positions/normals/skins/clips retained. Identity preserves UV bits. Linux x86_64 filesystem CLI; trusted stable link-free ancestry; absent output; no overwrite/install. Exit 0: help/published (check warnings); 2: error/not published.").map_err(|e|e.to_string());
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
    let bytes = transform_recipe(Path::new(get("--recipe")?), identity)?;
    publish(&output, &bytes)?;
    let _ = writeln!(
        io::stdout(),
        "published {} bytes to {}",
        bytes.len(),
        output.display()
    );
    Ok(())
}
