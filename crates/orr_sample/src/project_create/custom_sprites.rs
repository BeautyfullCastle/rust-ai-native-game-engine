//! New-project-only custom actor art; package/runtime admission stays authoritative.
use super::*;
use crate::project_sprites::{Binding, Document, Source};

#[derive(Clone, Debug)]
pub struct CustomSpriteOptions {
    pub base: CreateOptions,
    /// Absolute, nonsymlink directory containing the four-file sprite package.
    pub package: PathBuf,
    /// Explicit clip IDs in the same sprite document; they may be equal.
    pub idle: String,
    pub walk: String,
    /// Existing presentation scale range: finite 0.0001..=100 world units per pixel.
    pub units_per_pixel: f32,
}

pub fn create_custom_sprites(options: &CustomSpriteOptions) -> Result<CreateReport, String> {
    create_custom_with(options, |_, _| Ok(()), publish_no_replace)
}

fn create_custom_with(
    options: &CustomSpriteOptions,
    mut checkpoint: impl FnMut(&str, &Path) -> Result<(), String>,
    publish: impl FnOnce(&Path, &Path) -> Result<(), String>,
) -> Result<CreateReport, String> {
    if options.base.template != CUSTOM_SPRITE_TEMPLATE {
        return Err(format!("expected {CUSTOM_SPRITE_TEMPLATE}"));
    }
    let captured = capture(&options.package)?;
    let manifest: orr_package::Manifest =
        serde_json::from_slice(&captured["orr.package.json"]).map_err(error)?;
    if manifest.name == template::PACKAGE
        || manifest.capabilities != BTreeSet::from(["sprite".to_owned()])
        || !manifest.dependencies.is_empty()
    {
        return Err(
            "custom actor package must be sprite-only, dependency-free and not sample-sprites"
                .into(),
        );
    }
    let sprite = orr_sprite::SpriteDocument::from_json(
        std::str::from_utf8(&captured["sprites.json"]).map_err(error)?,
    )
    .map_err(error)?;
    let image = &sprite.atlas().image;
    if !image.ends_with(".png") || image.contains('/') || image.contains('\\') {
        return Err("custom actor atlas must be a portable PNG leaf".into());
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
        return Err("custom actor package requires exactly its manifest, sprites.json, atlas PNG and LICENSE.txt".into());
    }
    let license = std::str::from_utf8(&captured["LICENSE.txt"]).map_err(error)?;
    if license.trim().is_empty() || license.len() > 64 * 1024 {
        return Err("custom actor license must be nonempty UTF-8, at most 64 KiB".into());
    }
    let binding = Binding {
        package: manifest.name.clone(),
        document: "sprites.json".into(),
        source: Source::Locomotion {
            idle: options.idle.clone(),
            walk: options.walk.clone(),
        },
        units_per_pixel: options.units_per_pixel,
        orientation: None,
    };
    // Resolve both explicitly selected clips and the scale before any stage is made.
    binding.region(&sprite, 0)?;
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
                return Err("custom actor source changed during creation".into());
            }
            Ok(())
        },
        publish,
        |root, expected| finalize(root, expected, &captured, &manifest, &binding),
    )?;
    report.template = CUSTOM_SPRITE_TEMPLATE.into();
    Ok(report)
}

/// Freeze the complete flat source inventory and bounded payload bytes. Repeat at
/// publication to detect ordinary concurrent edits, not adversarial filesystem races.
fn capture(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let text = root.to_str().ok_or("sprite package path must be UTF-8")?;
    if !root.is_absolute()
        || text.len() > 4096
        || text.chars().any(char::is_control)
        || text.split('/').any(|part| matches!(part, "." | ".."))
        || root.components().count() > 64
    {
        return Err("sprite package requires an absolute bounded path without traversal".into());
    }
    let mut current = PathBuf::new();
    for component in root.components() {
        if !matches!(component, Component::RootDir | Component::Normal(_)) {
            return Err("invalid sprite package path".into());
        }
        current.push(component);
        let metadata = fs::symlink_metadata(&current).map_err(error)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("sprite package ancestors must be nonsymlink directories".into());
        }
    }
    let mut files = BTreeMap::new();
    let mut total = 0usize;
    for entry in fs::read_dir(root).map_err(error)? {
        let entry = entry.map_err(error)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "sprite filenames must be UTF-8")?;
        if files.len() >= 4 || !entry.file_type().map_err(error)?.is_file() {
            return Err("sprite package must contain exactly four regular nonsymlink files".into());
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
            return Err("sprite source must be a bounded regular file".into());
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(error)?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err("sprite source exceeds file byte limit".into());
        }
        total = total
            .checked_add(bytes.len())
            .ok_or("sprite source size overflow")?;
        if total > MAX_TOTAL_BYTES {
            return Err("sprite source exceeds byte limit".into());
        }
        files.insert(name, bytes);
    }
    if files.len() != 4
        || !["orr.package.json", "sprites.json", "LICENSE.txt"]
            .iter()
            .all(|name| files.contains_key(*name))
    {
        return Err("sprite package requires exactly four files including manifest, sprites.json and LICENSE.txt".into());
    }
    Ok(files)
}

fn finalize(
    root: &Path,
    expected: &mut BTreeMap<String, Vec<u8>>,
    captured: &BTreeMap<String, Vec<u8>>,
    manifest: &orr_package::Manifest,
    binding: &Binding,
) -> Result<(), String> {
    verify_profile_stage(root, expected, false, false)?;
    // Validate the unchanged closed starter before selecting its camera-follow hero.
    let original = PreparedRuntime::open(root)?;
    let original_checksum = original.initial_frame().checksum();
    let mut document = Document::from_bytes(&expected["arena.sprites.json"])?;
    let hero = document
        .camera_follow
        .clone()
        .ok_or("starter has no camera-follow hero")?;
    let old_binding = document
        .bindings
        .get_mut(&hero)
        .ok_or("starter hero has no sprite binding")?;
    if old_binding.package != template::PACKAGE {
        return Err("starter hero does not use the closed sample package".into());
    }
    *old_binding = binding.clone();
    document.validate()?;
    crate::project::validate_arena_sprite_targets(original.project().scene().scene(), &document)?;

    // Install only captured bytes in the privately owned transaction, never from
    // the caller's live source. The original actor package must remain identical.
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
        return Err("custom actor installation changed the closed package set".into());
    }
    let installed = lock
        .packages
        .get(&manifest.name)
        .ok_or("custom actor package missing after install")?;
    if installed.manifest != *manifest {
        return Err("installed custom actor manifest changed".into());
    }
    // This loads and strictly decodes the installed PNG, not just its JSON header.
    let asset =
        crate::project_sprites::load_project_asset(&project, &manifest.name, "sprites.json")?;
    binding.region(&asset.document, 0)?;
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
    let bytes = json_line(&document)?;
    replace_owned(root, "arena.sprites.json", &bytes)?;
    expected.insert("arena.sprites.json".into(), bytes);
    let mut readme = String::from_utf8(expected["README.md"].clone())
        .map_err(error)?
        .replacen(
            &format!("Template: {TEMPLATE}"),
            &format!("Template: {CUSTOM_SPRITE_TEMPLATE}"),
            1,
        );
    readme.push_str(&format!("\nCustom actor template: {CUSTOM_SPRITE_TEMPLATE}. Only the camera-follow hero uses package {} with {:?}, at {} world units per pixel. The target keeps the bundled sample-sprites art. Original custom sprites.json, PNG and LICENSE.txt bytes are installed unchanged; the package manifest is canonicalized. Source package files are not needed after creation. Existing-project attachment/reimport and combined custom tilemap creation are not provided.\n", binding.package, binding.source, binding.units_per_pixel));
    replace_owned(root, "README.md", readme.as_bytes())?;
    expected.insert("README.md".into(), readme.into_bytes());
    if PreparedRuntime::open(root)?.initial_frame().checksum() != original_checksum {
        return Err("custom actor changed the Arena initial state".into());
    }
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
