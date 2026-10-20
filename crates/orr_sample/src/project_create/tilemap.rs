//! Closed tilemap starter using the existing installation and atomic root publisher.
use super::*;

pub fn create_tilemap(options: &CreateOptions) -> Result<CreateReport, String> {
    if options.template != TILEMAP_TEMPLATE {
        return Err(format!("expected {TILEMAP_TEMPLATE}"));
    }
    let base = CreateOptions { output: options.output.clone(), template: TEMPLATE.into(), seed: options.seed.clone() };
    let mut report = create_transaction_finalized(&base, None, |_, _| Ok(()), publish_no_replace, finalize)?;
    report.template = TILEMAP_TEMPLATE.into();
    Ok(report)
}

fn finalize(root: &Path, expected: &mut BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    // The package lock is installed before a tile declaration prohibits mutations.
    // This is private unpublished-stage work, never an existing-project attach.
    let original = expected.get("orr.project.json").ok_or("missing staged manifest")?;
    if crate::project::read_regular(&root.join("orr.project.json"), MAX_FILE_BYTES as u64)? != *original {
        return Err("starter manifest changed before tilemap finalization".into());
    }
    let mut manifest: orr_package::ProjectManifest = serde_json::from_slice(original).map_err(error)?;
    manifest.entry.as_mut().ok_or("missing Arena entry")?.tilemap = Some("arena.tilemap.json".into());
    let document = orr_tilemap::Document {
        version: 1,
        sprite: orr_tilemap::SpriteRef { package: template::PACKAGE.into(), document: "sprites.json".into() },
        width: 16, height: 12, tile_size: [16, 16], origin: [-128.0, 96.0], units_per_pixel: 1.0,
        cells: vec![None; 16 * 12],
    };
    let map = json_line(&document)?;
    orr_tilemap::Document::from_bytes(&map)?;
    write_new(&root.join("arena.tilemap.json"), &map)?;
    expected.insert("arena.tilemap.json".into(), map);
    let manifest = json_line(&manifest)?;
    replace_owned(root, "orr.project.json", &manifest)?;
    expected.insert("orr.project.json".into(), manifest);
    let readme = format!("# New Arena tilemap project\n\nTemplate: {TILEMAP_TEMPLATE}\n\nThis Linux-only closed starter installs the original sample-sprites atlas. The finite 16x12 background begins empty; static regions 10, 11, 20 and 21 can be painted in the tilemap panel.\n\nOpen with a tilemap-enabled orr_editor --project PATH. Paint or erase, Undo, Save tilemap, reopen, then Play. Run the saved project with a tilemap-enabled arena --project PATH. Scene Save does not save tiles. The tile layer replaces only default Arena floor presentation: empty/transparent cells reveal DEFAULT_CLEAR, without changing collision or simulation.\n\nExport, later Attach to an existing project, arbitrary external atlas imports, package changes and image reimport are unsupported for this tilemap v1 project. Recreate a new project rather than changing its lock. Original Arena game identity, two players, seed42 and shared preferences remain unchanged; the authoring seed keeps the existing Arena GUID derivation. Existing MIT asset notices remain in the scene and installed package.\n");
    replace_owned(root, "README.md", readme.as_bytes())?;
    expected.insert("README.md".into(), readme.into_bytes());
    // Fresh admission observes the final declaration, never the pre-install handle.
    PreparedRuntime::open(root)?;
    Ok(())
}

fn replace_owned(root: &Path, relative: &str, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_FILE_BYTES { return Err("starter file exceeds byte limit".into()); }
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path).map_err(error)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() { return Err("staged replacement must be a regular file".into()); }
    let mut temporary = tempfile::NamedTempFile::new_in(root).map_err(error)?;
    temporary.write_all(bytes).map_err(error)?;
    temporary.as_file().set_permissions(fs::Permissions::from_mode(0o644)).map_err(error)?;
    temporary.as_file().sync_all().map_err(error)?;
    temporary.persist(&path).map_err(error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilemap_creator_is_real_bounded_and_atomic() {
        let temp = tempfile::tempdir().unwrap();
        let options = CreateOptions { output: temp.path().join("created"), template: TILEMAP_TEMPLATE.into(), seed: "tiles".into() };
        let report = create_tilemap(&options).unwrap();
        assert_eq!(report.template, TILEMAP_TEMPLATE);
        let prepared = PreparedRuntime::open(&report.output).unwrap();
        let map = prepared.project().tilemap().unwrap();
        assert_eq!(map.document.document().cells.len(), 192);
        assert_eq!(map.document.occupied_count(), 0);
        let before = fs::read(report.output.join("orr.project.json")).unwrap();
        assert!(create_tilemap(&options).is_err());
        assert_eq!(fs::read(report.output.join("orr.project.json")).unwrap(), before);
        let project = orr_package::Project::open(&report.output, compiled_runtime()).unwrap();
        assert!(project.remove(template::PACKAGE).unwrap_err().to_string().contains("tilemap projects"));
        assert!(fs::read_to_string(report.output.join("README.md")).unwrap().contains("unsupported"));
        for phase in ["finalizer", "before-validation", "before-publish"] {
            let options = CreateOptions { output: temp.path().join(phase), template: TEMPLATE.into(), seed: "tiles".into() };
            let result = create_transaction_finalized(&options, None,
                |point, _| if point == phase { Err("injected checkpoint".into()) } else { Ok(()) },
                publish_no_replace,
                |root, expected| { finalize(root, expected)?; if phase == "finalizer" { Err("injected finalizer".into()) } else { Ok(()) } });
            assert!(result.is_err());
            assert!(!options.output.exists());
        }
        assert!(!fs::read_dir(temp.path()).unwrap().any(|entry| entry.unwrap().file_name().to_string_lossy().starts_with(".orr-create-")));
    }
}
