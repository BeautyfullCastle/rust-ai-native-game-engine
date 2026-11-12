//! Byte-only bounded embedded GLB subtree inspection and static extraction.
//! The production importer remains the authority for all source admission.
//! This module performs no filesystem access and preserves source-world placement.
use crate::{ModelSource, StaticModel, MAX_FILE_BYTES, MAX_NODES};
use serde::Serialize;
use serde_json::Value;

const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;

/// One selectable default-scene subtree. Node names are advisory and may repeat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NodeInfo {
    /// Exact source node index used for extraction.
    pub index: u32,
    pub name: Option<String>,
    /// Imported primitives in this node and all of its descendants.
    pub primitive_count: usize,
}

struct Admitted {
    model: StaticModel,
    children: Vec<Vec<usize>>,
    names: Vec<Option<String>>,
    reachable: Vec<bool>,
    primitive_nodes: Vec<usize>,
}

fn descendants(children: &[Vec<usize>], node: usize) -> Result<Vec<bool>, String> {
    if node >= children.len() {
        return Err("node index out of range".into());
    }
    let mut selected = vec![false; children.len()];
    let mut stack = vec![node];
    while let Some(index) = stack.pop() {
        let seen = selected.get_mut(index).ok_or("child index out of range")?;
        if *seen {
            return Err("repeated node in admitted subtree".into());
        }
        *seen = true;
        stack.extend(children[index].iter().copied());
        if stack.len() > MAX_NODES {
            return Err("node work limit exceeded".into());
        }
    }
    Ok(selected)
}

fn admit(input: &[u8], asset_id: &str) -> Result<Admitted, String> {
    // The envelope check excludes resource resolution. The production importer
    // remains the sole authority for supported schemas, geometry and graphs.
    let document = unpack(input)?;
    let model = crate::import::import_with_resolver(asset_id, input, |_| {
        Err(crate::Error::message("external resources are unsupported"))
    })
    .map_err(|e| format!("static model import: {e}"))?;
    let nodes = document
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or("missing nodes")?;
    if nodes.len() > MAX_NODES {
        return Err("node limit exceeded".into());
    }
    let mut children = Vec::with_capacity(nodes.len());
    let mut names = Vec::with_capacity(nodes.len());
    for node in nodes {
        let list = match node.get("children") {
            None => Vec::new(),
            Some(value) => value
                .as_array()
                .ok_or("invalid children")?
                .iter()
                .map(|value| {
                    let i = value.as_u64().ok_or("invalid child index")?;
                    usize::try_from(i).map_err(|_| "child index overflow")
                })
                .collect::<Result<Vec<_>, _>>()?,
        };
        children.push(list);
        names.push(node.get("name").and_then(Value::as_str).map(str::to_owned));
    }
    let scene_index = match document.get("scene") {
        Some(value) => usize::try_from(value.as_u64().ok_or("invalid default scene")?)
            .map_err(|_| "scene index overflow")?,
        None => 0,
    };
    let roots = document
        .get("scenes")
        .and_then(Value::as_array)
        .and_then(|s| s.get(scene_index))
        .and_then(|s| s.get("nodes"))
        .and_then(Value::as_array)
        .ok_or("missing default scene roots")?;
    let mut reachable = vec![false; nodes.len()];
    for root in roots {
        let root = usize::try_from(root.as_u64().ok_or("invalid scene root")?)
            .map_err(|_| "scene root overflow")?;
        for (to, from) in reachable.iter_mut().zip(descendants(&children, root)?) {
            *to |= from;
        }
    }
    // asset_id is importer-validated and cannot contain '#'. Read only the
    // guaranteed source-slot IDs emitted by that unchanged importer.
    let prefix = format!("{asset_id}#node=");
    let primitive_nodes = model
        .source()
        .primitives
        .iter()
        .map(|primitive| {
            let suffix = primitive
                .id
                .strip_prefix(&prefix)
                .ok_or("unexpected primitive identity")?;
            let (node, _) = suffix
                .split_once("/mesh=")
                .ok_or("missing primitive node identity")?;
            let node = node
                .parse::<usize>()
                .map_err(|_| "invalid primitive node identity")?;
            if !reachable.get(node).copied().unwrap_or(false) {
                return Err("primitive outside default scene");
            }
            Ok(node)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Admitted {
        model,
        children,
        names,
        reachable,
        primitive_nodes,
    })
}

/// List only default-scene nodes whose subtree contains imported geometry.
/// Names are advisory; selection always uses the unambiguous source index.
pub fn list_glb_nodes(input: &[u8], asset_id: &str) -> Result<Vec<NodeInfo>, String> {
    let admitted = admit(input, asset_id)?;
    let mut result = Vec::new();
    for (index, reachable) in admitted.reachable.iter().copied().enumerate() {
        if !reachable {
            continue;
        }
        let selected = descendants(&admitted.children, index)?;
        let count = admitted
            .primitive_nodes
            .iter()
            .filter(|&&node| selected[node])
            .count();
        if count != 0 {
            result.push(NodeInfo {
                index: index as u32,
                name: admitted.names[index].clone(),
                primitive_count: count,
            });
        }
    }
    Ok(result)
}

/// Extract one admitted default-scene subtree into the existing cooked format.
/// Geometry retains source-world coordinates; no centering or pivot rewrite.
pub fn extract_glb_node(
    input: &[u8],
    asset_id: &str,
    node_index: u32,
) -> Result<StaticModel, String> {
    let admitted = admit(input, asset_id)?;
    let node = usize::try_from(node_index).map_err(|_| "node index overflow")?;
    if !admitted.reachable.get(node).copied().unwrap_or(false) {
        return Err("selected node is outside the imported default scene or does not exist".into());
    }
    let selected = descendants(&admitted.children, node)?;
    let source = admitted.model.source();
    let primitives: Vec<_> = source
        .primitives
        .iter()
        .zip(&admitted.primitive_nodes)
        .filter(|(_, node)| selected[**node])
        .map(|(primitive, _)| primitive.clone())
        .collect();
    if primitives.is_empty() {
        return Err("selected subtree has no imported geometry".into());
    }
    let model = StaticModel::new(ModelSource {
        format: source.format.clone(),
        version: source.version,
        asset_id: source.asset_id.clone(),
        dependencies: source.dependencies.clone(),
        primitives,
        materials: source.materials.clone(),
        images: source.images.clone(),
    })
    .map_err(|e| format!("selected model validation: {e}"))?;
    let cooked = model
        .to_bytes()
        .map_err(|e| format!("selected model cook: {e}"))?;
    let reloaded =
        StaticModel::from_bytes(&cooked).map_err(|e| format!("selected model reload: {e}"))?;
    if reloaded.source() != model.source() {
        return Err("selected model changed during cooked round-trip".into());
    }
    Ok(reloaded)
}
fn word(bytes: &[u8], offset: usize) -> Result<usize, String> {
    let end = offset.checked_add(4).ok_or("GLB offset overflow")?;
    let bytes = bytes.get(offset..end).ok_or("truncated GLB")?;
    Ok(u32::from_le_bytes(bytes.try_into().expect("four bytes")) as usize)
}

fn unpack(bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() > MAX_FILE_BYTES
        || bytes.len() < 28
        || &bytes[..4] != b"glTF"
        || word(bytes, 4)? != 2
        || word(bytes, 8)? != bytes.len()
    {
        return Err("input must be a bounded, exact-length GLB 2 file".into());
    }
    let json_len = word(bytes, 12)?;
    if json_len > MAX_JSON_BYTES || json_len % 4 != 0 || &bytes[16..20] != b"JSON" {
        return Err("GLB must begin with a bounded aligned JSON chunk".into());
    }
    let bin_header = 20usize.checked_add(json_len).ok_or("GLB length overflow")?;
    let json_bytes = bytes.get(20..bin_header).ok_or("truncated GLB JSON")?;
    let bin_len = word(bytes, bin_header)?;
    let bin_start = bin_header.checked_add(8).ok_or("GLB length overflow")?;
    if bytes.get(bin_header + 4..bin_start) != Some(b"BIN\0")
        || bin_len % 4 != 0
        || bin_start.checked_add(bin_len) != Some(bytes.len())
    {
        return Err(
            "GLB requires exactly one aligned BIN chunk; extra chunks are unsupported".into(),
        );
    }
    let document: Value =
        serde_json::from_slice(json_bytes).map_err(|e| format!("GLB JSON: {e}"))?;
    let buffers = document
        .get("buffers")
        .and_then(Value::as_array)
        .ok_or("GLB requires one embedded buffer")?;
    if buffers.len() != 1 || buffers[0].get("uri").is_some() {
        return Err("only one embedded BIN buffer without a URI is supported".into());
    }
    if let Some(images) = document.get("images").and_then(Value::as_array) {
        if images
            .iter()
            .any(|image| image.get("uri").is_some() || image.get("bufferView").is_none())
        {
            return Err("images must use embedded bufferViews; every URI is unsupported".into());
        }
    }
    Ok(document)
}

#[cfg(test)]
mod tests;
