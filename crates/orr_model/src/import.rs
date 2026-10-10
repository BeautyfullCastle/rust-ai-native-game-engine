//! Bounded original parser for an explicit glTF 2.0/GLB subset. This is not a
//! general glTF implementation. Unknown schema fields/extensions fail closed.
//! Indexed triangles, f32 POSITION/NORMAL/UV0, node TRS/affine matrices, PNG,
//! opaque diffuse base-color materials. No network or ambient filesystem reads.
use crate::*;
use base64::Engine;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufReader, Cursor, Read},
    path::Path,
};
const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_DEPTH: usize = 64;
const MAX_RESOURCES: usize = 256;

// Names/extras are non-semantic exporter metadata. All other unknown fields
// (including unsupported extensions, sparse accessors and skin data) reject.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Document {
    pub(crate) asset: Asset,
    #[serde(default)]
    pub(crate) extensions_used: Vec<String>,
    #[serde(default)]
    pub(crate) extensions_required: Vec<String>,
    #[serde(default)]
    pub(crate) buffers: Vec<Buffer>,
    #[serde(default)]
    pub(crate) buffer_views: Vec<View>,
    #[serde(default)]
    pub(crate) accessors: Vec<Accessor>,
    #[serde(default)]
    pub(crate) images: Vec<ImageDef>,
    #[serde(default)]
    pub(crate) textures: Vec<Texture>,
    #[serde(default)]
    pub(crate) samplers: Vec<Sampler>,
    #[serde(default)]
    pub(crate) materials: Vec<MaterialDef>,
    #[serde(default)]
    pub(crate) meshes: Vec<Mesh>,
    #[serde(default)]
    pub(crate) nodes: Vec<Node>,
    #[serde(default, deserialize_with = "present_value")]
    pub(crate) skins: Option<Vec<SkinDef>>,
    #[serde(default, deserialize_with = "present_value")]
    pub(crate) animations: Option<Vec<ClipDef>>,
    pub(crate) scenes: Vec<Scene>,
    pub(crate) scene: Option<usize>,
    #[serde(default, rename = "extras")]
    pub(crate) _extras: Option<serde_json::Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Asset {
    pub(crate) version: String,
    pub(crate) min_version: Option<String>,
    #[serde(rename = "generator")]
    pub(crate) _generator: Option<String>,
    #[serde(rename = "copyright")]
    pub(crate) _copyright: Option<String>,
    #[serde(default, rename = "extras")]
    pub(crate) _extras: Option<serde_json::Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Buffer {
    pub(crate) byte_length: usize,
    pub(crate) uri: Option<String>,
    #[serde(rename = "name")]
    pub(crate) _name: Option<String>,
    #[serde(default, rename = "extras")]
    pub(crate) _extras: Option<serde_json::Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct View {
    pub(crate) buffer: usize,
    #[serde(default)]
    pub(crate) byte_offset: usize,
    pub(crate) byte_length: usize,
    pub(crate) byte_stride: Option<usize>,
    pub(crate) target: Option<u32>,
    #[serde(rename = "name")]
    pub(crate) _name: Option<String>,
    #[serde(default, rename = "extras")]
    pub(crate) _extras: Option<serde_json::Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Accessor {
    pub(crate) buffer_view: usize,
    #[serde(default)]
    pub(crate) byte_offset: usize,
    pub(crate) component_type: u32,
    pub(crate) count: usize,
    #[serde(rename = "type")]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) normalized: bool,
    pub(crate) min: Option<Vec<f32>>,
    pub(crate) max: Option<Vec<f32>>,
    #[serde(rename = "name")]
    pub(crate) _name: Option<String>,
    #[serde(default, rename = "extras")]
    pub(crate) _extras: Option<serde_json::Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ImageDef {
    pub(crate) uri: Option<String>,
    pub(crate) buffer_view: Option<usize>,
    pub(crate) mime_type: Option<String>,
    #[serde(rename = "name")]
    pub(crate) _name: Option<String>,
    #[serde(default, rename = "extras")]
    pub(crate) _extras: Option<serde_json::Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Texture {
    pub(crate) source: usize,
    pub(crate) sampler: Option<usize>,
    #[serde(rename = "name")]
    pub(crate) _name: Option<String>,
    #[serde(default, rename = "extras")]
    pub(crate) _extras: Option<serde_json::Value>,
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Sampler {
    pub(crate) mag_filter: Option<u32>,
    pub(crate) min_filter: Option<u32>,
    pub(crate) wrap_s: Option<u32>,
    pub(crate) wrap_t: Option<u32>,
    #[serde(rename = "name")]
    pub(crate) _name: Option<String>,
    #[serde(default, rename = "extras")]
    pub(crate) _extras: Option<serde_json::Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MaterialDef {
    pub(crate) pbr_metallic_roughness: Pbr,
    pub(crate) alpha_mode: Option<String>,
    #[serde(default)]
    pub(crate) double_sided: bool,
    #[serde(rename = "name")]
    pub(crate) _name: Option<String>,
    #[serde(default, rename = "extras")]
    pub(crate) _extras: Option<serde_json::Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Pbr {
    pub(crate) base_color_texture: TextureInfo,
    pub(crate) base_color_factor: Option<[f32; 4]>,
    pub(crate) metallic_factor: Option<f32>,
    pub(crate) roughness_factor: Option<f32>,
    #[serde(default, rename = "extras")]
    pub(crate) _extras: Option<serde_json::Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TextureInfo {
    pub(crate) index: usize,
    #[serde(default)]
    pub(crate) tex_coord: u32,
    #[serde(default, deserialize_with = "present_texture_extensions")]
    extensions: Option<TextureExtensions>,
    #[serde(default, rename = "extras")]
    pub(crate) _extras: Option<serde_json::Value>,
}

const TEXTURE_TRANSFORM: &str = "KHR_texture_transform";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextureExtensions {
    #[serde(rename = "KHR_texture_transform", deserialize_with = "map_value")]
    transform: TextureTransform,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TextureTransform {
    #[serde(default)]
    offset: [f32; 2],
    #[serde(default)]
    rotation: f32,
    #[serde(default = "unit_texture_scale")]
    scale: [f32; 2],
    #[serde(default, deserialize_with = "present_value")]
    tex_coord: Option<u32>,
}

fn unit_texture_scale() -> [f32; 2] {
    [1.0; 2]
}

// serde's derived structs also accept positional arrays. Extension descriptors
// are glTF objects, so both nested boundaries explicitly require maps.
fn map_value<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<T, D::Error> {
    struct Map<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Map<T> {
        type Value = T;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a texture extension object")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
            T::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }
    deserializer.deserialize_map(Map(std::marker::PhantomData))
}

fn present_texture_extensions<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<TextureExtensions>, D::Error> {
    map_value(deserializer).map(Some)
}

impl TextureInfo {
    fn effective_tex_coord(&self) -> u32 {
        self.extensions
            .as_ref()
            .and_then(|e| e.transform.tex_coord)
            .unwrap_or(self.tex_coord)
    }

    fn transformed_uv(&self, uv: [f32; 2]) -> Result<[f32; 2], Error> {
        let valid = |v: [f32; 2]| v.iter().all(|x| x.is_finite() && x.abs() <= 65536.0);
        // Check the source too: a zero scale must not hide invalid source data.
        if !valid(uv) {
            return Err(invalid("invalid source texture UV"));
        }
        let Some(extension) = &self.extensions else {
            return Ok(uv);
        };
        let t = &extension.transform;
        if t.offset == [0.0; 2] && t.rotation == 0.0 && t.scale == [1.0; 2] {
            return Ok(uv); // Preserve identity coordinates, including signed zero.
        }
        let scaled = [uv[0] * t.scale[0], uv[1] * t.scale[1]];
        let (s, c) = t.rotation.sin_cos();
        let result = [
            t.offset[0] + c * scaled[0] - s * scaled[1],
            t.offset[1] + s * scaled[0] + c * scaled[1],
        ];
        if !valid(result) {
            return Err(invalid("invalid transformed texture UV"));
        }
        Ok(result)
    }
}

fn validate_texture_extensions(doc: &Document, animated: bool) -> Result<(), Error> {
    let used: BTreeSet<_> = doc.extensions_used.iter().collect();
    let required: BTreeSet<_> = doc.extensions_required.iter().collect();
    if used.len() != doc.extensions_used.len()
        || required.len() != doc.extensions_required.len()
        || used.iter().any(|name| name.as_str() != TEXTURE_TRANSFORM)
        || required.iter().any(|name| !used.contains(name))
    {
        return Err(invalid("unsupported or inconsistent glTF extensions"));
    }
    for material in &doc.materials {
        let info = &material.pbr_metallic_roughness.base_color_texture;
        if let Some(extension) = &info.extensions {
            if animated
                || !doc
                    .extensions_used
                    .iter()
                    .any(|name| name == TEXTURE_TRANSFORM)
            {
                return Err(invalid(
                    "texture transform requires a declared static import",
                ));
            }
            let t = &extension.transform;
            if !t
                .offset
                .iter()
                .chain(&t.scale)
                .chain(std::iter::once(&t.rotation))
                .all(|x| x.is_finite())
            {
                return Err(invalid("nonfinite texture transform"));
            }
        }
        if info.effective_tex_coord() != 0 {
            return Err(invalid("only effective TEXCOORD_0 is supported"));
        }
    }
    if animated && (!used.is_empty() || !required.is_empty()) {
        return Err(invalid(
            "animated importer does not support texture transforms",
        ));
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Mesh {
    pub(crate) primitives: Vec<PrimitiveDef>,
    #[serde(rename = "name")]
    pub(crate) _name: Option<String>,
    #[serde(default, rename = "extras")]
    pub(crate) _extras: Option<serde_json::Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PrimitiveDef {
    pub(crate) attributes: BTreeMap<String, usize>,
    pub(crate) indices: usize,
    pub(crate) material: usize,
    pub(crate) mode: Option<u32>,
    #[serde(default, rename = "extras")]
    pub(crate) _extras: Option<serde_json::Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Node {
    pub(crate) mesh: Option<usize>,
    #[serde(default, deserialize_with = "present_value")]
    pub(crate) skin: Option<usize>,
    #[serde(default)]
    pub(crate) children: Vec<usize>,
    pub(crate) matrix: Option<[f32; 16]>,
    pub(crate) translation: Option<[f32; 3]>,
    pub(crate) rotation: Option<[f32; 4]>,
    pub(crate) scale: Option<[f32; 3]>,
    #[serde(rename = "name")]
    pub(crate) _name: Option<String>,
    #[serde(default, rename = "extras")]
    pub(crate) _extras: Option<serde_json::Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Scene {
    pub(crate) nodes: Vec<usize>,
    #[serde(rename = "name")]
    pub(crate) _name: Option<String>,
    #[serde(default, rename = "extras")]
    pub(crate) _extras: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(not(feature = "animation"), allow(dead_code))]
pub(crate) struct SkinDef {
    pub(crate) joints: Vec<usize>,
    #[serde(default, deserialize_with = "present_value")]
    pub(crate) inverse_bind_matrices: Option<usize>,
    #[serde(default, deserialize_with = "present_value")]
    pub(crate) skeleton: Option<usize>,
    #[serde(default, deserialize_with = "present_value")]
    pub(crate) name: Option<String>,
    #[serde(default, rename = "extras")]
    pub(crate) _extras: Option<serde_json::Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(not(feature = "animation"), allow(dead_code))]
pub(crate) struct ClipDef {
    pub(crate) samplers: Vec<AnimationSampler>,
    pub(crate) channels: Vec<ChannelDef>,
    #[serde(default, deserialize_with = "present_value")]
    pub(crate) name: Option<String>,
    #[serde(default, rename = "extras")]
    pub(crate) _extras: Option<serde_json::Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(not(feature = "animation"), allow(dead_code))]
pub(crate) struct AnimationSampler {
    pub(crate) input: usize,
    pub(crate) output: usize,
    #[serde(default, deserialize_with = "present_value")]
    pub(crate) interpolation: Option<String>,
    #[serde(default, rename = "extras")]
    pub(crate) _extras: Option<serde_json::Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(not(feature = "animation"), allow(dead_code))]
pub(crate) struct ChannelDef {
    pub(crate) sampler: usize,
    pub(crate) target: ChannelTarget,
    #[serde(default, rename = "extras")]
    pub(crate) _extras: Option<serde_json::Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(not(feature = "animation"), allow(dead_code))]
pub(crate) struct ChannelTarget {
    pub(crate) node: usize,
    pub(crate) path: String,
    #[serde(default, rename = "extras")]
    pub(crate) _extras: Option<serde_json::Value>,
}

// An absent declaration differs from an explicitly present value for static
// import. Explicit null is not a valid glTF property and must not erase it.
fn present_value<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

/// The normalized package-relative `asset_id` remains stable across reimport.
/// Canonical containment also rejects symlinks escaping the immutable snapshot.
pub fn import_path(root: &Path, asset_id: &str) -> Result<StaticModel, Error> {
    import_path_with(root, asset_id, |id, bytes, resolve| {
        import_with_resolver(id, bytes, resolve)
    })
}

pub(crate) fn import_path_with<T>(
    root: &Path,
    asset_id: &str,
    importer: impl FnOnce(
        &str,
        &[u8],
        &mut dyn FnMut(&str) -> Result<Vec<u8>, Error>,
    ) -> Result<T, Error>,
) -> Result<T, Error> {
    valid_path(asset_id)?;
    let root = root.canonicalize().map_err(|e| invalid(e.to_string()))?;
    let source_path = root.join(asset_id);
    let read = |path: &Path| -> Result<Vec<u8>, Error> {
        let path = path.canonicalize().map_err(|e| invalid(e.to_string()))?;
        if !path.starts_with(&root) {
            return Err(invalid("asset dependency escapes package root"));
        }
        // Check BEFORE open: opening a FIFO could block before File::metadata.
        // The caller supplies an immutable snapshot; retain the handle check too.
        let meta = std::fs::metadata(&path).map_err(|e| invalid(e.to_string()))?;
        if !meta.is_file() || meta.len() > MAX_FILE_BYTES as u64 {
            return Err(invalid("invalid dependency file/size"));
        }
        let file = File::open(path).map_err(|e| invalid(e.to_string()))?;
        let meta = file.metadata().map_err(|e| invalid(e.to_string()))?;
        if !meta.is_file() || meta.len() > MAX_FILE_BYTES as u64 {
            return Err(invalid("invalid dependency file/size"));
        }
        let mut data = Vec::new();
        file.take((MAX_FILE_BYTES + 1) as u64)
            .read_to_end(&mut data)
            .map_err(|e| invalid(e.to_string()))?;
        if data.len() > MAX_FILE_BYTES {
            return Err(invalid("dependency exceeds byte budget"));
        }
        Ok(data)
    };
    let bytes = read(&source_path)?;
    let base = source_path
        .parent()
        .ok_or_else(|| invalid("missing asset parent"))?;
    importer(asset_id, &bytes, &mut |uri| read(&base.join(uri)))
}

/// Resolver receives only normalized relative URIs, once per URI. It must enforce
/// package containment and bounded reads. Returned bytes are checked/charged again.
/// Embedded GLB and data-URI resources never invoke the resolver.
pub(crate) struct Loaded {
    pub(crate) doc: Document,
    pub(crate) buffers: Vec<Vec<u8>>,
    pub(crate) dependencies: Vec<Dependency>,
    pub(crate) materials: Vec<Material>,
    pub(crate) images: Vec<Image>,
}

/// Both importers use this strict schema/resource reader; the static entrypoint
/// explicitly rejects every animation/skin declaration before resolving data.
pub(crate) fn load_resources(
    asset_id: &str,
    bytes: &[u8],
    mut resolver: impl FnMut(&str) -> Result<Vec<u8>, Error>,
    animated: bool,
) -> Result<Loaded, Error> {
    valid_path(asset_id)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(invalid("source exceeds byte budget"));
    }
    let (json, blob) = split_source(bytes)?;
    if json.len() > MAX_JSON_BYTES {
        return Err(invalid("glTF JSON exceeds byte budget"));
    }
    let doc: Document =
        serde_json::from_slice(json).map_err(|e| invalid(format!("glTF subset: {e}")))?;
    if !animated
        && (doc.skins.is_some()
            || doc.animations.is_some()
            || doc.nodes.iter().any(|node| node.skin.is_some()))
    {
        return Err(invalid("static importer does not support skins/animations"));
    }
    if doc.asset.version != "2.0" || doc.asset.min_version.as_deref().is_some_and(|s| s != "2.0") {
        return Err(invalid("unsupported glTF version/extensions"));
    }
    if doc.nodes.len() > MAX_NODES
        || doc.images.len() > MAX_IMAGES
        || doc.materials.len() > MAX_MATERIALS
        || doc.buffers.len() > MAX_RESOURCES
        || doc.buffer_views.len() > MAX_RESOURCES * 4
        || doc.accessors.len() > MAX_RESOURCES * 4
        || doc.meshes.len() > MAX_PRIMITIVES
        || doc.textures.len() > MAX_MATERIALS
        || doc.samplers.len() > MAX_MATERIALS
        || doc.scenes.is_empty()
        || doc.scenes.len() > MAX_NODES
    {
        return Err(invalid("glTF collection budget exceeded"));
    }
    // Admit the entire document before any caller-provided resource resolver.
    validate_texture_extensions(&doc, animated)?;
    #[cfg(feature = "animation")]
    if animated {
        crate::animation_import::validate_document_limits(&doc)?;
    }
    let mut dependencies = BTreeMap::from([("$source".to_owned(), digest(bytes))]);
    let mut cache: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut total_bytes = bytes.len();
    let mut resolve = |uri: &str| -> Result<Vec<u8>, Error> {
        if let Some(encoded) = uri.strip_prefix("data:") {
            let (kind, payload) = encoded
                .split_once(',')
                .ok_or_else(|| invalid("invalid data URI"))?;
            if !matches!(
                kind,
                "application/octet-stream;base64"
                    | "application/gltf-buffer;base64"
                    | "image/png;base64"
            ) {
                return Err(invalid("unsupported data URI MIME/encoding"));
            }
            if payload.len() > MAX_FILE_BYTES {
                return Err(invalid("data URI budget exceeded"));
            }
            // Charge conservative decoded bound BEFORE decoding/allocation.
            let bound = payload.len() / 4 * 3;
            if total_bytes.saturating_add(bound) > MAX_FILE_BYTES {
                return Err(invalid("data URI aggregate budget exceeded"));
            }
            let data = base64::engine::general_purpose::STANDARD
                .decode(payload)
                .map_err(|e| invalid(e.to_string()))?;
            charge(&mut total_bytes, data.len())?;
            return Ok(data);
        }
        valid_path(uri)?;
        if uri == "$source" {
            return Err(invalid("reserved dependency URI"));
        }
        if let Some(data) = cache.get(uri) {
            charge(&mut total_bytes, data.len())?;
            return Ok(data.clone());
        }
        if cache.len() >= MAX_RESOURCES - 1 {
            return Err(invalid("dependency count exceeded"));
        }
        let data = resolver(uri)?;
        charge(&mut total_bytes, data.len())?;
        dependencies.insert(uri.to_owned(), digest(&data));
        cache.insert(uri.to_owned(), data.clone());
        Ok(data)
    };
    let mut buffers = Vec::new();
    for (i, b) in doc.buffers.iter().enumerate() {
        if b.byte_length == 0 || b.byte_length > MAX_FILE_BYTES {
            return Err(invalid("invalid buffer byteLength"));
        }
        let data = match &b.uri {
            Some(uri) => resolve(uri)?,
            None if i == 0 => blob
                .ok_or_else(|| invalid("missing GLB BIN chunk"))?
                .to_vec(),
            None => return Err(invalid("only GLB buffer zero may omit URI")),
        };
        let pad = if b.uri.is_none() { 3 } else { 0 };
        if data.len() < b.byte_length || data.len() > b.byte_length.saturating_add(pad) {
            return Err(invalid("buffer byteLength mismatch"));
        }
        buffers.push(data);
    }
    for v in &doc.buffer_views {
        let b = at(&doc.buffers, v.buffer, "buffer")?;
        let end = v
            .byte_offset
            .checked_add(v.byte_length)
            .ok_or_else(|| invalid("view overflow"))?;
        if v.byte_length == 0
            || end > b.byte_length
            || v.byte_stride
                .is_some_and(|s| !(4..=252).contains(&s) || s % 4 != 0)
            || v.target.is_some_and(|t| t != 34962 && t != 34963)
        {
            return Err(invalid("invalid buffer view range/stride/target"));
        }
    }
    let mut accessor_validation_bytes = 0usize;
    for a in &doc.accessors {
        let v = at(&doc.buffer_views, a.buffer_view, "buffer view")?;
        let (component, elements) = a.layout()?;
        let size = component * elements;
        let stride = v.byte_stride.unwrap_or(size);
        let offset = v
            .byte_offset
            .checked_add(a.byte_offset)
            .ok_or_else(|| invalid("accessor offset overflow"))?;
        if a.count == 0
            || a.count > MAX_INDICES
            || stride < size
            || (!animated && (a.normalized || matches!(a.kind.as_str(), "VEC4" | "MAT4")))
            || (a.normalized && !matches!(a.component_type, 5121 | 5123))
            || (a.kind == "MAT4" && a.component_type != 5126)
            || stride % component != 0
            || a.byte_offset % component != 0
            || offset % component != 0
        {
            return Err(invalid("accessor count/stride/type/alignment"));
        }
        let end = (a.count - 1)
            .checked_mul(stride)
            .and_then(|n| n.checked_add(size))
            .and_then(|n| n.checked_add(a.byte_offset))
            .ok_or_else(|| invalid("accessor range overflow"))?;
        if end > v.byte_length {
            return Err(invalid("accessor outside buffer view"));
        }
        for bound in [&a.min, &a.max].into_iter().flatten() {
            if bound.len() != elements || !bound.iter().all(|v| v.is_finite()) {
                return Err(invalid("invalid accessor bounds"));
            }
        }
        if animated && a.component_type == 5126 {
            // Include orphan accessor values without permitting repeated
            // overlapping views to turn a small source into unbounded work.
            accessor_validation_bytes = accessor_validation_bytes
                .checked_add(
                    a.count
                        .checked_mul(size)
                        .ok_or_else(|| invalid("accessor scan byte overflow"))?,
                )
                .ok_or_else(|| invalid("accessor scan byte overflow"))?;
            if accessor_validation_bytes > MAX_FILE_BYTES {
                return Err(invalid("accessor value validation byte budget exceeded"));
            }
            for i in 0..a.count {
                for component in a.bytes(&doc, &buffers, i).chunks_exact(4) {
                    if !f32::from_le_bytes(component.try_into().expect("validated f32 component"))
                        .is_finite()
                    {
                        return Err(invalid("nonfinite f32 accessor value"));
                    }
                }
            }
        }
    }
    let mut images = Vec::new();
    let mut decoded = 0usize;
    for image in &doc.images {
        if image.mime_type.as_deref().is_some_and(|m| m != "image/png") {
            return Err(invalid("only PNG supported"));
        }
        let data = match (&image.uri, image.buffer_view) {
            (Some(uri), None) => resolve(uri)?,
            (None, Some(i)) if image.mime_type.as_deref() == Some("image/png") => {
                let v = at(&doc.buffer_views, i, "image view")?;
                if v.byte_stride.is_some() || v.target.is_some() {
                    return Err(invalid("image view must not be vertex/index data"));
                }
                buffers[v.buffer][v.byte_offset..v.byte_offset + v.byte_length].to_vec()
            }
            _ => return Err(invalid("image requires exactly URI or PNG bufferView")),
        };
        let image = decode_png(&data, MAX_DECODED_BYTES - decoded)?;
        decoded += image.rgba8.len();
        images.push(image);
    }
    for t in &doc.textures {
        at(&images, t.source, "texture image")?;
        if let Some(i) = t.sampler {
            at(&doc.samplers, i, "sampler")?;
        }
    }
    let mut materials = Vec::new();
    for m in &doc.materials {
        let p = &m.pbr_metallic_roughness;
        if m.alpha_mode.as_deref().unwrap_or("OPAQUE") != "OPAQUE"
            || m.double_sided
            || p.metallic_factor.unwrap_or(1.0) != 0.0
            || p.roughness_factor.unwrap_or(1.0) != 1.0
            || p.base_color_texture.effective_tex_coord() != 0
        {
            return Err(invalid(
                "requires opaque single-sided diffuse material (metallic=0, roughness=1), UV0",
            ));
        }
        let t = at(
            &doc.textures,
            p.base_color_texture.index,
            "base color texture",
        )?;
        let default = Sampler::default();
        let s = match t.sampler {
            Some(i) => at(&doc.samplers, i, "sampler")?,
            None => &default,
        };
        let min = s.min_filter.unwrap_or(9729);
        let mag = s.mag_filter.unwrap_or(min);
        if !matches!(min, 9728 | 9729) || mag != min {
            return Err(invalid(
                "only matching nearest/linear non-mip filters supported",
            ));
        }
        let wrap = |n| match n {
            33071 => Ok(Wrap::Clamp),
            10497 => Ok(Wrap::Repeat),
            33648 => Ok(Wrap::Mirror),
            _ => Err(invalid("invalid wrap mode")),
        };
        materials.push(Material {
            base_color: p.base_color_factor.unwrap_or([1.0; 4]),
            image: t.source as u32,
            linear_filter: min == 9729,
            wrap_s: wrap(s.wrap_s.unwrap_or(10497))?,
            wrap_t: wrap(s.wrap_t.unwrap_or(10497))?,
        });
    }
    Ok(Loaded {
        doc,
        buffers,
        materials,
        images,
        dependencies: dependencies
            .into_iter()
            .map(|(uri, sha256)| Dependency { uri, sha256 })
            .collect(),
    })
}

pub fn import_with_resolver(
    asset_id: &str,
    bytes: &[u8],
    resolver: impl FnMut(&str) -> Result<Vec<u8>, Error>,
) -> Result<StaticModel, Error> {
    let Loaded {
        doc,
        buffers,
        dependencies,
        materials,
        images,
    } = load_resources(asset_id, bytes, resolver, false)?;
    // Validate every source mesh before allocating any decoded geometry.
    let mut primitive_count = 0usize;
    for mesh in &doc.meshes {
        primitive_count += mesh.primitives.len();
        if mesh.primitives.is_empty() || primitive_count > MAX_PRIMITIVES {
            return Err(invalid("primitive budget exceeded"));
        }
        for p in &mesh.primitives {
            validate_primitive(&doc, p)?;
        }
    }
    let mut parents = vec![0u32; doc.nodes.len()];
    let mut locals = Vec::new();
    for n in &doc.nodes {
        if let Some(i) = n.mesh {
            at(&doc.meshes, i, "node mesh")?;
        }
        for &c in &n.children {
            let count = parents
                .get_mut(c)
                .ok_or_else(|| invalid("child index out of range"))?;
            *count += 1;
            if *count > 1 {
                return Err(invalid("duplicate child/multiple parents"));
            }
        }
        locals.push(n.transform()?);
    }
    fn check(i: usize, doc: &Document, state: &mut [u8], depth: usize) -> Result<(), Error> {
        if depth > MAX_DEPTH {
            return Err(invalid("node depth exceeded"));
        }
        if state[i] == 1 {
            return Err(invalid("node cycle"));
        }
        if state[i] == 2 {
            return Ok(());
        }
        state[i] = 1;
        for &c in &doc.nodes[i].children {
            check(c, doc, state, depth + 1)?;
        }
        state[i] = 2;
        Ok(())
    }
    let mut state = vec![0u8; doc.nodes.len()];
    // Start at graph roots so previously memoized descendants cannot hide an
    // over-deep unused tree whose node indices happen to be reverse-ordered.
    for (i, &count) in parents.iter().enumerate() {
        if count == 0 {
            check(i, &doc, &mut state, 0)?;
        }
    }
    for i in 0..doc.nodes.len() {
        check(i, &doc, &mut state, 0)?;
    }
    for scene in &doc.scenes {
        let mut roots = BTreeSet::new();
        for &i in &scene.nodes {
            if *at(&parents, i, "scene root")? != 0 || !roots.insert(i) {
                return Err(invalid("invalid/duplicate scene root"));
            }
        }
    }
    let scene = at(&doc.scenes, doc.scene.unwrap_or(0), "default scene")?;
    let mut builder = Builder {
        doc: &doc,
        buffers: &buffers,
        locals: &locals,
        asset_id,
        primitives: Vec::new(),
        vertices: 0,
        indices: 0,
    };
    for &i in &scene.nodes {
        builder.visit(i, IDENTITY, 0)?;
    }
    StaticModel::new(ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: asset_id.into(),
        dependencies,
        primitives: builder.primitives,
        materials,
        images,
    })
}
pub(crate) fn at<'a, T>(items: &'a [T], i: usize, label: &str) -> Result<&'a T, Error> {
    items
        .get(i)
        .ok_or_else(|| invalid(format!("{label} index out of range")))
}
impl Accessor {
    pub(crate) fn layout(&self) -> Result<(usize, usize), Error> {
        let c = match self.component_type {
            5121 => 1,
            5123 => 2,
            5125 | 5126 => 4,
            _ => return Err(invalid("unsupported accessor component type")),
        };
        let n = match self.kind.as_str() {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            "MAT4" => 16,
            _ => return Err(invalid("unsupported accessor dimensions")),
        };
        Ok((c, n))
    }
    pub(crate) fn bytes<'a>(&self, doc: &Document, buffers: &'a [Vec<u8>], i: usize) -> &'a [u8] {
        let v = &doc.buffer_views[self.buffer_view];
        let (c, n) = self.layout().expect("validated accessor");
        let start = v.byte_offset + self.byte_offset + i * v.byte_stride.unwrap_or(c * n);
        &buffers[v.buffer][start..start + c * n]
    }
    pub(crate) fn floats<const N: usize>(
        &self,
        doc: &Document,
        buffers: &[Vec<u8>],
        i: usize,
    ) -> [f32; N] {
        let b = self.bytes(doc, buffers, i);
        std::array::from_fn(|n| {
            f32::from_le_bytes(b[n * 4..n * 4 + 4].try_into().expect("validated f32"))
        })
    }
}
fn validate_primitive(doc: &Document, p: &PrimitiveDef) -> Result<(), Error> {
    if p.mode.unwrap_or(4) != 4
        || p.attributes.len() != 3
        || !["POSITION", "NORMAL", "TEXCOORD_0"]
            .iter()
            .all(|s| p.attributes.contains_key(*s))
    {
        return Err(invalid(
            "requires indexed TRIANGLES with only POSITION/NORMAL/TEXCOORD_0",
        ));
    }
    at(&doc.materials, p.material, "primitive material")?;
    let pos = at(&doc.accessors, p.attributes["POSITION"], "POSITION")?;
    if pos.count > MAX_VERTICES {
        return Err(invalid("vertex count exceeds budget"));
    }
    for (name, kind) in [
        ("POSITION", "VEC3"),
        ("NORMAL", "VEC3"),
        ("TEXCOORD_0", "VEC2"),
    ] {
        let a = at(&doc.accessors, p.attributes[name], name)?;
        let view = &doc.buffer_views[a.buffer_view];
        if a.component_type != 5126
            || a.kind != kind
            || a.count != pos.count
            || view.target.is_some_and(|v| v != 34962)
        {
            return Err(invalid("f32 vertex attribute count/type/target mismatch"));
        }
    }
    if pos.min.as_ref().is_none_or(|v| v.len() != 3)
        || pos.max.as_ref().is_none_or(|v| v.len() != 3)
    {
        return Err(invalid("POSITION bounds required"));
    }
    let idx = at(&doc.accessors, p.indices, "indices")?;
    let view = &doc.buffer_views[idx.buffer_view];
    if idx.kind != "SCALAR"
        || !matches!(idx.component_type, 5121 | 5123 | 5125)
        || idx.count % 3 != 0
        || view.byte_stride.is_some()
        || view.target.is_some_and(|v| v != 34963)
    {
        return Err(invalid("invalid triangle indices"));
    }
    Ok(())
}
struct Builder<'a> {
    doc: &'a Document,
    buffers: &'a [Vec<u8>],
    locals: &'a [[[f32; 4]; 4]],
    asset_id: &'a str,
    primitives: Vec<Primitive>,
    vertices: usize,
    indices: usize,
}
impl Builder<'_> {
    fn visit(&mut self, i: usize, parent: [[f32; 4]; 4], depth: usize) -> Result<(), Error> {
        if depth > MAX_DEPTH {
            return Err(invalid("node depth exceeded"));
        }
        let local = self.locals[i];
        let transform = std::array::from_fn(|c| {
            std::array::from_fn(|r| (0..4).map(|k| parent[k][r] * local[c][k]).sum())
        });
        normal_matrix(transform)?;
        let node = &self.doc.nodes[i];
        if let Some(mesh) = node.mesh {
            for (slot, p) in self.doc.meshes[mesh].primitives.iter().enumerate() {
                let pos = &self.doc.accessors[p.attributes["POSITION"]];
                let normal = &self.doc.accessors[p.attributes["NORMAL"]];
                let uv = &self.doc.accessors[p.attributes["TEXCOORD_0"]];
                let idx = &self.doc.accessors[p.indices];
                self.vertices += pos.count;
                self.indices += idx.count;
                if self.vertices > MAX_VERTICES
                    || self.indices > MAX_INDICES
                    || self.primitives.len() >= MAX_PRIMITIVES
                {
                    return Err(invalid("expanded geometry budget exceeded"));
                }
                let min = pos.min.as_ref().expect("validated bounds");
                let max = pos.max.as_ref().expect("validated bounds");
                let mut vertices = Vec::with_capacity(pos.count);
                for v in 0..pos.count {
                    let position = pos.floats::<3>(self.doc, self.buffers, v);
                    if (0..3).any(|a| {
                        !position[a].is_finite()
                            || min[a] > max[a]
                            || position[a] < min[a]
                            || position[a] > max[a]
                    }) {
                        return Err(invalid("invalid POSITION bounds/value"));
                    }
                    vertices.push(Vertex {
                        position,
                        normal: normal.floats(self.doc, self.buffers, v),
                        uv: self.doc.materials[p.material]
                            .pbr_metallic_roughness
                            .base_color_texture
                            .transformed_uv(uv.floats(self.doc, self.buffers, v))?,
                    });
                }
                let mut indices = Vec::with_capacity(idx.count);
                for n in 0..idx.count {
                    let b = idx.bytes(self.doc, self.buffers, n);
                    let index = match idx.component_type {
                        5121 => u32::from(b[0]),
                        5123 => u32::from(u16::from_le_bytes(b.try_into().expect("u16 index"))),
                        _ => u32::from_le_bytes(b.try_into().expect("u32 index")),
                    };
                    if index as usize >= pos.count {
                        return Err(invalid("index out of bounds"));
                    }
                    indices.push(index);
                }
                self.primitives.push(Primitive {
                    id: format!("{}#node={i}/mesh={mesh}/primitive={slot}", self.asset_id),
                    vertices,
                    indices,
                    material: p.material as u32,
                    transform,
                });
            }
        }
        for &child in &node.children {
            self.visit(child, transform, depth + 1)?;
        }
        Ok(())
    }
}
impl Node {
    pub(crate) fn transform(&self) -> Result<[[f32; 4]; 4], Error> {
        let result = if let Some(m) = self.matrix {
            if self.translation.is_some() || self.rotation.is_some() || self.scale.is_some() {
                return Err(invalid("matrix and TRS are mutually exclusive"));
            }
            std::array::from_fn(|c| std::array::from_fn(|r| m[c * 4 + r]))
        } else {
            let [x, y, z, w] = self.rotation.unwrap_or([0.0, 0.0, 0.0, 1.0]);
            let norm = x * x + y * y + z * z + w * w;
            if !norm.is_finite() || (norm - 1.0).abs() > 1e-4 {
                return Err(invalid("node quaternion must be unit length"));
            }
            let s = self.scale.unwrap_or([1.0; 3]);
            let t = self.translation.unwrap_or([0.0; 3]);
            [
                [
                    (1.0 - 2.0 * (y * y + z * z)) * s[0],
                    2.0 * (x * y + z * w) * s[0],
                    2.0 * (x * z - y * w) * s[0],
                    0.0,
                ],
                [
                    2.0 * (x * y - z * w) * s[1],
                    (1.0 - 2.0 * (x * x + z * z)) * s[1],
                    2.0 * (y * z + x * w) * s[1],
                    0.0,
                ],
                [
                    2.0 * (x * z + y * w) * s[2],
                    2.0 * (y * z - x * w) * s[2],
                    (1.0 - 2.0 * (x * x + y * y)) * s[2],
                    0.0,
                ],
                [t[0], t[1], t[2], 1.0],
            ]
        };
        normal_matrix(result)?;
        Ok(result)
    }
}
fn split_source(bytes: &[u8]) -> Result<(&[u8], Option<&[u8]>), Error> {
    if !bytes.starts_with(b"glTF") {
        return Ok((bytes, None));
    }
    let u32_at = |offset: usize| -> Result<u32, Error> {
        Ok(u32::from_le_bytes(
            bytes
                .get(offset..offset + 4)
                .ok_or_else(|| invalid("truncated GLB header"))?
                .try_into()
                .expect("4 bytes"),
        ))
    };
    if u32_at(4)? != 2 || u32_at(8)? as usize != bytes.len() {
        return Err(invalid("GLB version/length mismatch"));
    }
    let mut offset = 12usize;
    let mut json = None;
    let mut bin = None;
    while offset < bytes.len() {
        let len = u32_at(offset)? as usize;
        let kind = u32_at(offset + 4)?;
        let start = offset
            .checked_add(8)
            .ok_or_else(|| invalid("GLB chunk overflow"))?;
        let end = start
            .checked_add(len)
            .ok_or_else(|| invalid("GLB chunk overflow"))?;
        let chunk = bytes
            .get(start..end)
            .ok_or_else(|| invalid("truncated GLB chunk"))?;
        if len % 4 != 0 {
            return Err(invalid("unaligned GLB chunk"));
        }
        match kind {
            0x4e4f534a if offset == 12 && json.is_none() => json = Some(chunk),
            0x004e4942 if json.is_some() && bin.is_none() => bin = Some(chunk),
            _ => return Err(invalid("unsupported/duplicate/out-of-order GLB chunk")),
        }
        offset = end;
    }
    Ok((json.ok_or_else(|| invalid("missing GLB JSON chunk"))?, bin))
}
fn charge(total: &mut usize, bytes: usize) -> Result<(), Error> {
    *total = total
        .checked_add(bytes)
        .ok_or_else(|| invalid("byte budget overflow"))?;
    if *total > MAX_FILE_BYTES {
        return Err(invalid("aggregate source byte budget exceeded"));
    }
    Ok(())
}
fn digest(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn decode_png(bytes: &[u8], remaining: usize) -> Result<Image, Error> {
    let mut decoder = png::Decoder::new(BufReader::new(Cursor::new(bytes)));
    decoder.set_limits(png::Limits {
        bytes: MAX_DECODED_BYTES,
    });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| invalid(format!("PNG: {e}")))?;
    let (width, height) = (reader.info().width, reader.info().height);
    if width == 0
        || height == 0
        || width > MAX_IMAGE_EXTENT
        || height > MAX_IMAGE_EXTENT
        || reader.info().animation_control.is_some()
    {
        return Err(invalid("PNG extent or APNG unsupported"));
    }
    let expected = width as usize * height as usize * 4;
    if expected > remaining {
        return Err(invalid("decoded image budget exceeded"));
    }
    let len = reader
        .output_buffer_size()
        .ok_or_else(|| invalid("PNG size overflow"))?;
    if len != expected {
        return Err(invalid("PNG must decode to exact RGBA8"));
    }
    let mut rgba8 = vec![0; len];
    let info = reader
        .next_frame(&mut rgba8)
        .map_err(|e| invalid(format!("PNG: {e}")))?;
    if info.width != width
        || info.height != height
        || info.buffer_size() != expected
        || info.color_type != png::ColorType::Rgba
        || info.bit_depth != png::BitDepth::Eight
    {
        return Err(invalid("PNG frame does not match static RGBA8 image"));
    }
    reader.finish().map_err(|e| invalid(format!("PNG: {e}")))?;
    Ok(Image {
        width,
        height,
        rgba8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sha256_known_vectors() {
        assert_eq!(
            digest(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            digest(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
