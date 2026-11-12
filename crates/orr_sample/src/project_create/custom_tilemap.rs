//! Bounded custom-tileset creation into a new, privately owned Arena stage.
//! The package installer and runtime sprite/PNG readers remain the authorities.
use super::*;

#[derive(Clone, Debug)]
pub struct CustomTilemapOptions {
    pub base: CreateOptions,
    pub package: PathBuf,
    pub tile_size: [u32; 2],
}

pub fn create_custom_tilemap(options: &CustomTilemapOptions) -> Result<CreateReport, String> {
    create_custom_with(options, |_, _| Ok(()), publish_no_replace)
}

fn create_custom_with(
    options: &CustomTilemapOptions,
    mut checkpoint: impl FnMut(&str, &Path) -> Result<(), String>,
    publish: impl FnOnce(&Path, &Path) -> Result<(), String>,
) -> Result<CreateReport, String> {
    if options.base.template != CUSTOM_TILEMAP_TEMPLATE {
        return Err(format!("expected {CUSTOM_TILEMAP_TEMPLATE}"));
    }
    let captured = capture(&options.package)?;
    let manifest: orr_package::Manifest =
        serde_json::from_slice(&captured["orr.package.json"]).map_err(error)?;
    if manifest.name == template::PACKAGE
        || manifest.capabilities != BTreeSet::from(["sprite".to_owned()])
        || !manifest.dependencies.is_empty()
    {
        return Err(
            "custom tileset must be sprite-only, dependency-free and not sample-sprites".into(),
        );
    }
    let sprite = orr_sprite::SpriteDocument::from_json(
        std::str::from_utf8(&captured["sprites.json"]).map_err(error)?,
    )
    .map_err(error)?;
    let image = &sprite.atlas().image;
    if !image.ends_with(".png") || image.contains('/') || image.contains('\\') {
        return Err("custom tileset atlas must be a portable PNG leaf".into());
    }
    let files = BTreeSet::from([
        "sprites.json".to_owned(),
        image.clone(),
        "LICENSE.txt".to_owned(),
    ]);
    if manifest.files != files
        || captured.keys().cloned().collect::<BTreeSet<_>>()
            != files
                .iter()
                .cloned()
                .chain(std::iter::once("orr.package.json".to_owned()))
                .collect()
    {
        return Err(
            "custom tileset requires exactly its manifest, sprites.json, atlas PNG and LICENSE.txt"
                .into(),
        );
    }
    let license = std::str::from_utf8(&captured["LICENSE.txt"]).map_err(error)?;
    if license.trim().is_empty() || license.len() > 64 * 1024 {
        return Err("custom tileset license must be nonempty UTF-8, at most 64 KiB".into());
    }
    let document = orr_tilemap::Document {
        version: 1,
        sprite: orr_tilemap::SpriteRef {
            package: manifest.name.clone(),
            document: "sprites.json".into(),
        },
        width: 16,
        height: 12,
        tile_size: options.tile_size,
        origin: [
            -8.0 * options.tile_size[0] as f32,
            6.0 * options.tile_size[1] as f32,
        ],
        units_per_pixel: 1.0,
        cells: vec![None; 16 * 12],
    };
    document.validate(&sprite)?;
    if !sprite
        .regions()
        .iter()
        .any(|region| [region.width, region.height] == options.tile_size)
    {
        return Err("custom tileset requires a static region matching tile size".into());
    }
    let base = CreateOptions {
        output: options.base.output.clone(),
        template: TEMPLATE.into(),
        seed: options.base.seed.clone(),
    };
    let mut report = create_transaction_finalized(
        &base,
        None,
        |point, transaction| {
            checkpoint(point, transaction)?;
            if point == "before-publish" && capture(&options.package)? != captured {
                return Err("custom tileset source changed during creation".into());
            }
            Ok(())
        },
        publish,
        |root, expected| finalize(root, expected, &captured, &manifest, &document),
    )?;
    report.template = CUSTOM_TILEMAP_TEMPLATE.into();
    Ok(report)
}

/// Freeze the complete flat source inventory and bounded payload bytes. Repeat at
/// publication to detect ordinary concurrent edits, not adversarial filesystem races.
fn capture(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let text = root.to_str().ok_or("tileset package path must be UTF-8")?;
    if !root.is_absolute()
        || text.len() > 4096
        || text.chars().any(char::is_control)
        || text.split('/').any(|part| matches!(part, "." | ".."))
        || root.components().count() > 64
    {
        return Err("tileset package requires an absolute bounded path without traversal".into());
    }
    let mut current = PathBuf::new();
    for component in root.components() {
        if !matches!(component, Component::RootDir | Component::Normal(_)) {
            return Err("invalid tileset package path".into());
        }
        current.push(component);
        let metadata = fs::symlink_metadata(&current).map_err(error)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("tileset package ancestors must be nonsymlink directories".into());
        }
    }
    let mut files = BTreeMap::new();
    let mut total = 0usize;
    for entry in fs::read_dir(root).map_err(error)? {
        let entry = entry.map_err(error)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "tileset filenames must be UTF-8")?;
        if files.len() >= 4 || !entry.file_type().map_err(error)?.is_file() {
            return Err(
                "tileset package must contain exactly four regular nonsymlink files".into(),
            );
        }
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(
                (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32,
            )
            .open(entry.path())
            .map_err(error)?;
        let metadata = file.metadata().map_err(error)?;
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES as u64 {
            return Err("tileset source must be a bounded regular file".into());
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(error)?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err("tileset source exceeds file byte limit".into());
        }
        total = total
            .checked_add(bytes.len())
            .ok_or("tileset source size overflow")?;
        if total > MAX_TOTAL_BYTES {
            return Err("tileset source exceeds byte limit".into());
        }
        files.insert(name, bytes);
    }
    if files.len() != 4
        || !["orr.package.json", "sprites.json", "LICENSE.txt"]
            .iter()
            .all(|name| files.contains_key(*name))
    {
        return Err("tileset package requires exactly four files including manifest, sprites.json and LICENSE.txt".into());
    }
    Ok(files)
}

fn finalize(
    root: &Path,
    expected: &mut BTreeMap<String, Vec<u8>>,
    captured: &BTreeMap<String, Vec<u8>>,
    manifest: &orr_package::Manifest,
    document: &orr_tilemap::Document,
) -> Result<(), String> {
    verify_profile_stage(root, expected, false, false)?;
    // Install captured bytes, never reread the mutable caller source. This private
    // sibling is owned by the base transaction and is never part of the project.
    let source = tempfile::Builder::new()
        .prefix("custom-source-")
        .tempdir_in(root.parent().ok_or("missing transaction parent")?)
        .map_err(error)?;
    for (name, bytes) in captured {
        write_new(&source.path().join(name), bytes)?;
    }
    let project = orr_package::Project::open(root, compiled_runtime()).map_err(error)?;
    let before = project.verify().map_err(error)?;
    let lock = project
        .install(&[source.path().to_path_buf()])
        .map_err(error)?;
    if project.verify().map_err(error)? != lock
        || lock.packages.len() != 2
        || lock.direct.len() != 2
        || lock.packages.get(template::PACKAGE) != before.packages.get(template::PACKAGE)
        || lock.direct.get(template::PACKAGE) != before.direct.get(template::PACKAGE)
        || lock.direct.get(&manifest.name) != Some(&manifest.version)
    {
        return Err("custom tileset installation changed the closed package set".into());
    }
    let installed = lock
        .packages
        .get(&manifest.name)
        .ok_or("custom package missing after install")?;
    if installed.manifest != *manifest {
        return Err("installed custom manifest changed".into());
    }
    let asset =
        crate::project_sprites::load_project_asset(&project, &manifest.name, "sprites.json")?;
    document.validate(&asset.document)?;
    if !asset
        .document
        .regions()
        .iter()
        .any(|region| [region.width, region.height] == document.tile_size)
    {
        return Err("installed tileset has no matching static region".into());
    }
    let object = format!(".orr/packages/objects/{}", installed.digest);
    expected.insert(
        format!("{object}/orr.package.json"),
        serde_json::to_vec_pretty(&installed.manifest).map_err(error)?,
    );
    for name in &manifest.files {
        expected.insert(format!("{object}/{name}"), captured[name].clone());
    }
    expected.insert("orr.packages.lock.json".into(), json_line(&lock)?);
    drop(source);
    verify_profile_stage(root, expected, false, false)?;
    // Only now declare the tilemap. Existing-project package mutation remains locked.
    let mut project_manifest: orr_package::ProjectManifest =
        serde_json::from_slice(&expected["orr.project.json"]).map_err(error)?;
    project_manifest
        .entry
        .as_mut()
        .ok_or("missing Arena entry")?
        .tilemap = Some("arena.tilemap.json".into());
    let map = json_line(document)?;
    write_new(&root.join("arena.tilemap.json"), &map)?;
    expected.insert("arena.tilemap.json".into(), map);
    let bytes = json_line(&project_manifest)?;
    replace_owned(root, "orr.project.json", &bytes)?;
    expected.insert("orr.project.json".into(), bytes);
    let readme = format!("# New Arena custom tileset project\n\nTemplate: {CUSTOM_TILEMAP_TEMPLATE}\n\nThe empty 16x12 background uses the installed {} atlas with {}x{} pixel cells and one world unit per pixel. Original sample-sprites actor art is retained.\n\nOpen using a tilemap-enabled orr_editor --project PATH. Paint matching static regions, erase, Undo, Save tilemap, reopen and Play. Scene Save does not save tiles. Run with a tilemap-enabled arena --project PATH. Empty cells reveal DEFAULT_CLEAR; tiles do not change collision or simulation.\n\nCreation is Linux-only and new-output-only. Saved projects can be exported with the existing Linux tilemap exporter and a trusted prebuilt tilemap runtime. Existing-project Attach, live package changes and image reimport are unsupported. Recreate a new project to change the package. The source package is left unchanged. Original sprites.json, PNG and LICENSE.txt payload bytes are retained; the installed package manifest is canonicalized. Original MIT notices remain with sample-sprites and the scene. Arena identity, simulation seed 42, two players and shared preferences are unchanged.\n", manifest.name, document.tile_size[0], document.tile_size[1]);
    replace_owned(root, "README.md", readme.as_bytes())?;
    expected.insert("README.md".into(), readme.into_bytes());
    PreparedRuntime::open(root)?;
    Ok(())
}

fn replace_owned(root: &Path, relative: &str, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("starter file exceeds byte limit".into());
    }
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path).map_err(error)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("staged replacement must be a regular file".into());
    }
    let mut temporary = tempfile::NamedTempFile::new_in(root).map_err(error)?;
    temporary.write_all(bytes).map_err(error)?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o644))
        .map_err(error)?;
    temporary.as_file().sync_all().map_err(error)?;
    temporary.persist(&path).map_err(error)?;
    Ok(())
}

#[cfg(test)]
mod tests;
