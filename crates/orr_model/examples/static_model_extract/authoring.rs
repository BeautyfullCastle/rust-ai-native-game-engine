//! Private GPU-free static prop authoring. No runtime/importer behavior changes.
//! Paths require trusted stable ancestry; portable checks are not a sandbox
//! against hostile concurrent directory replacement. Publication never replaces.
use orr_model::{ModelSource, StaticModel, MAX_FILE_BYTES, MAX_NODES};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;
static STAGE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NodeInfo {
    pub index: u32,
    pub name: Option<String>,
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
    let model = orr_model::import::import_with_resolver(asset_id, input, |_| {
        Err(orr_model::Error::message(
            "external resources are unsupported",
        ))
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

fn checked_path(path: &Path, output: bool) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("empty paths and parent traversal are unsupported".into());
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    if !absolute.is_absolute() {
        return Err("drive-relative paths are unsupported".into());
    }
    // Reject network/device namespaces before even querying their metadata.
    // Only local drive prefixes are admitted on Windows.
    for component in absolute.components() {
        if let Component::Prefix(prefix) = component {
            if !matches!(
                prefix.kind(),
                std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
            ) {
                return Err("nonlocal Windows path prefixes are unsupported".into());
            }
        }
        if let Component::Normal(name) = component {
            if name.to_str().is_some_and(|s| s.ends_with(['.', ' '])) {
                return Err("path components ending in a dot or space are unsupported".into());
            }
        }
    }
    let mut result = PathBuf::new();
    let components: Vec<_> = absolute
        .components()
        .filter(|p| !matches!(p, Component::CurDir))
        .collect();
    // Store exclusion is lexical; trusted callers must not use alternate
    // directory aliases or mounts to address an installed package store.
    for (i, part) in components.iter().enumerate() {
        if output
            && part
                .as_os_str()
                .to_str()
                .is_some_and(|s| s.eq_ignore_ascii_case(".orr"))
        {
            return Err("output cannot be inside an .orr package store".into());
        }
        result.push(part.as_os_str());
        if matches!(part, Component::Prefix(_)) {
            continue;
        }
        match fs::symlink_metadata(&result) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err("symlink paths or ancestry are unsupported".into())
            }
            Ok(m) if i + 1 < components.len() && !m.is_dir() => {
                return Err("path ancestor is not a directory".into())
            }
            Ok(_) => {}
            Err(e)
                if output && i + 1 == components.len() && e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("path metadata: {e}")),
        }
    }
    Ok(result)
}
fn same_file(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        a.len() == b.len() && a.modified().ok() == b.modified().ok()
    }
}
fn read_input(path: &Path) -> Result<Vec<u8>, String> {
    let path = checked_path(path, false)?;
    let before = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !before.file_type().is_file() || before.len() > MAX_FILE_BYTES as u64 {
        return Err("input must be a bounded regular file".into());
    }
    let file = File::open(&path).map_err(|e| e.to_string())?;
    let opened = file.metadata().map_err(|e| e.to_string())?;
    let current = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !opened.is_file()
        || !current.file_type().is_file()
        || !same_file(&before, &opened)
        || !same_file(&opened, &current)
        || before.len() != opened.len()
    {
        return Err("input changed during open".into());
    }
    let mut limited = file.take(MAX_FILE_BYTES as u64 + 1);
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    limited.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let after = limited.get_ref().metadata().map_err(|e| e.to_string())?;
    if bytes.len() > MAX_FILE_BYTES
        || bytes.len() as u64 != opened.len()
        || after.len() != opened.len()
        || after.modified().ok() != opened.modified().ok()
    {
        return Err("input changed during read or exceeds byte limit".into());
    }
    Ok(bytes)
}
fn output_path(path: &Path) -> Result<PathBuf, String> {
    let name = path.file_name().and_then(|v| v.to_str()).unwrap_or("");
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if name.is_empty()
        || name.len() > 100
        || !name.ends_with(".model.json")
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        || ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
    {
        return Err("output must be a portable ASCII .model.json filename of at most 100 bytes, not a reserved device name".into());
    }
    let path = checked_path(path, true)?;
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(path),
        Ok(_) => Err("output already exists; no overwrite or --force is supported".into()),
        Err(e) => Err(e.to_string()),
    }
}
fn publish(output: &Path, bytes: &[u8]) -> Result<(), String> {
    let destination = output_path(output)?;
    let parent = destination
        .parent()
        .ok_or("output requires an existing trusted parent")?;
    let mut stage = None;
    for _ in 0..32 {
        let path = parent.join(format!(
            ".orr-static-prop-{}-{}.tmp",
            std::process::id(),
            STAGE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => {
                stage = Some((path, file));
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("output stage: {e}")),
        }
    }
    let (stage, mut file) = stage.ok_or("owned stage-name budget exhausted")?;
    let commit = (|| -> Result<(), String> {
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut readback = Vec::with_capacity(bytes.len());
        (&mut file)
            .take(bytes.len() as u64 + 1)
            .read_to_end(&mut readback)
            .map_err(|e| e.to_string())?;
        if readback != bytes {
            return Err("stage readback differs".into());
        }
        output_path(&destination)?;
        fs::hard_link(&stage, &destination)
            .map_err(|e| format!("no-replace hard-link publication required: {e}"))?;
        Ok(())
    })();
    drop(file);
    let cleanup = fs::remove_file(&stage);
    if let Err(error) = commit {
        return Err(format!(
            "not published: {error}{}",
            cleanup
                .err()
                .map(|e| format!("; owned stage {stage:?} cleanup failed: {e}"))
                .unwrap_or_default()
        ));
    }
    if let Err(error) = cleanup {
        let _ = writeln!(
            std::io::stderr(),
            "warning: published; owned stage {stage:?} cleanup failed: {error}"
        );
    }
    #[cfg(unix)]
    if let Err(error) = File::open(parent).and_then(|f| f.sync_all()) {
        let _ = writeln!(
            std::io::stderr(),
            "warning: published; directory durability uncertain: {error}"
        );
    }
    #[cfg(not(unix))]
    {
        let _ = writeln!(
            std::io::stderr(),
            "warning: published; directory sync unavailable on this platform"
        );
    }
    Ok(())
}

/// Strict bounded CLI: listing is read-only; extraction publishes one new file.
pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let args: Vec<_> = args.into_iter().take(10).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        return writeln!(std::io::stdout(), "Usage: static_model_extract --input FILE.glb --asset-id LOGICAL/SOURCE.glb --node N --output NEW.model.json\nList selectable source nodes: static_model_extract --input FILE.glb --asset-id LOGICAL/SOURCE.glb --list\nRequires import; embedded static GLB only. Subtrees keep source-world coordinates and all materials/images. Trusted stable link-free parents; no overwrite/install. Exit 0: help/list/published (check warnings); 2: error/not published.").map_err(|e| e.to_string());
    }
    let list = args.iter().any(|value| value == "--list");
    if args.len() != if list { 5 } else { 8 } {
        return Err(
            "expected two input pairs and --list, or four extraction pairs; use --help".into(),
        );
    }
    let mut values = BTreeMap::new();
    let mut position = 0;
    let mut list_seen = false;
    while position < args.len() {
        let key = args[position]
            .to_str()
            .ok_or("option names must be UTF-8")?;
        if key == "--list" {
            if list_seen {
                return Err("duplicate --list".into());
            }
            list_seen = true;
            position += 1;
            continue;
        }
        let allowed = if list {
            &["--input", "--asset-id"][..]
        } else {
            &["--input", "--asset-id", "--node", "--output"][..]
        };
        if !allowed.contains(&key) {
            return Err(format!("unknown or incompatible option {key}"));
        }
        let value = args.get(position + 1).ok_or("missing option value")?;
        if value.as_os_str().len() > 4096 {
            return Err(format!("{key} value is too long"));
        }
        if values.insert(key, value).is_some() {
            return Err(format!("duplicate option {key}"));
        }
        position += 2;
    }
    let get = |key: &str| {
        values
            .get(key)
            .copied()
            .ok_or_else(|| format!("missing {key}"))
    };
    let input_path = Path::new(get("--input")?);
    let asset_id = get("--asset-id")?
        .to_str()
        .ok_or("--asset-id must be UTF-8")?;
    if list {
        let input = read_input(input_path)?;
        let nodes = list_glb_nodes(&input, asset_id)?;
        let json = serde_json::to_string(&nodes).map_err(|e| e.to_string())?;
        return writeln!(std::io::stdout(), "{json}").map_err(|e| e.to_string());
    }
    let node = get("--node")?.to_str().ok_or("--node must be UTF-8")?;
    if node.is_empty() || !node.bytes().all(|b| b.is_ascii_digit()) {
        return Err("--node must be an unsigned integer".into());
    }
    let node = node.parse().map_err(|_| "node index overflow")?;
    let output = output_path(Path::new(get("--output")?))?;
    let input = read_input(input_path)?;
    let model = extract_glb_node(&input, asset_id, node)?;
    let cooked = model.to_bytes().map_err(|e| e.to_string())?;
    publish(&output, &cooked)?;
    let _ = writeln!(
        std::io::stdout(),
        "published {} bytes to {}",
        cooked.len(),
        output.display()
    );
    Ok(())
}
