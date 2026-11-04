use super::*;

fn fixture(root: &Path) -> PathBuf {
    let source = root.join("tileset");
    fs::create_dir(&source).unwrap();
    let mut manifest: orr_package::Manifest =
        serde_json::from_slice(template::SOURCE[0].1).unwrap();
    manifest.name = "custom-floor".into();
    manifest.files.remove("lantern_keeper.rgba");
    fs::write(
        source.join("orr.package.json"),
        json_line(&manifest).unwrap(),
    )
    .unwrap();
    for (name, bytes) in template::SOURCE.iter().skip(1) {
        if manifest.files.contains(*name) {
            fs::write(source.join(name), bytes).unwrap();
        }
    }
    source
}
fn options(root: &Path, package: PathBuf) -> CustomTilemapOptions {
    CustomTilemapOptions {
        base: CreateOptions {
            output: root.join("created"),
            template: CUSTOM_TILEMAP_TEMPLATE.into(),
            seed: "custom-test".into(),
        },
        package,
        tile_size: [16, 16],
    }
}
fn cleaned(root: &Path, output: &Path) {
    assert!(!output.exists());
    assert!(!fs::read_dir(root).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".orr-create-")));
}

#[test]
fn source_changes_at_publication_abort_and_clean_owned_stage() {
    for change in ["bytes", "inventory", "symlink"] {
        let temp = tempfile::tempdir().unwrap();
        let options = options(temp.path(), fixture(temp.path()));
        let result = create_custom_with(
            &options,
            |phase, _| {
                if phase == "before-publish" {
                    match change {
                        "bytes" => {
                            fs::write(options.package.join("LICENSE.txt"), "changed license")
                                .unwrap()
                        }
                        "inventory" => {
                            fs::write(options.package.join("extra.txt"), "unexpected").unwrap()
                        }
                        _ => {
                            fs::remove_file(options.package.join("LICENSE.txt")).unwrap();
                            std::os::unix::fs::symlink(
                                "sprites.json",
                                options.package.join("LICENSE.txt"),
                            )
                            .unwrap();
                        }
                    }
                }
                Ok(())
            },
            publish_no_replace,
        );
        assert!(result.is_err(), "{change}");
        cleaned(temp.path(), &options.base.output);
        assert!(options.package.exists());
    }
}

#[test]
fn staged_closure_tampering_and_publish_failure_leave_source_intact() {
    for phase in ["before-validation", "before-publish", "publish"] {
        let temp = tempfile::tempdir().unwrap();
        let options = options(temp.path(), fixture(temp.path()));
        let original = capture(&options.package).unwrap();
        let published = std::cell::Cell::new(false);
        let result = create_custom_with(
            &options,
            |point, transaction| {
                if point == phase {
                    fs::write(transaction.join("project/arena.tilemap.json"), "{}").unwrap();
                }
                Ok(())
            },
            |_, _| {
                published.set(true);
                assert_eq!(phase, "publish", "tampered stage reached the publisher");
                Err("injected publish failure".into())
            },
        );
        let error = result.unwrap_err();
        if phase == "publish" {
            assert!(published.get());
            assert_eq!(error, "injected publish failure");
        } else {
            assert!(!published.get());
            assert_eq!(error, "starter stage file size changed");
        }
        assert_eq!(capture(&options.package).unwrap(), original);
        cleaned(temp.path(), &options.base.output);
    }
}

#[test]
fn source_edits_before_install_cannot_change_captured_payload() {
    let temp = tempfile::tempdir().unwrap();
    let options = options(temp.path(), fixture(temp.path()));
    let result = create_custom_with(
        &options,
        |phase, _| {
            if phase == "before-install" {
                fs::write(options.package.join("sprites.json"), "{}").unwrap();
            }
            Ok(())
        },
        publish_no_replace,
    );
    assert!(result.unwrap_err().contains("source changed"));
    cleaned(temp.path(), &options.base.output);
}

#[test]
fn installer_rejects_reserved_and_nonportable_png_leaf_names() {
    for name in ["CON.png", "LPT1.png", "bad name.png", "bad:name.png"] {
        let temp = tempfile::tempdir().unwrap();
        let source = fixture(temp.path());
        let mut manifest: orr_package::Manifest =
            serde_json::from_slice(&fs::read(source.join("orr.package.json")).unwrap()).unwrap();
        manifest.files.remove("lantern_keeper.png");
        manifest.files.insert(name.into());
        fs::write(
            source.join("orr.package.json"),
            json_line(&manifest).unwrap(),
        )
        .unwrap();
        let mut sprite: serde_json::Value =
            serde_json::from_slice(&fs::read(source.join("sprites.json")).unwrap()).unwrap();
        sprite["atlas"]["image"] = name.into();
        fs::write(source.join("sprites.json"), json_line(&sprite).unwrap()).unwrap();
        fs::rename(source.join("lantern_keeper.png"), source.join(name)).unwrap();
        let options = options(temp.path(), source);
        let original = capture(&options.package).unwrap();
        assert!(create_custom_tilemap(&options).is_err(), "{name}");
        assert_eq!(capture(&options.package).unwrap(), original);
        cleaned(temp.path(), &options.base.output);
    }
}
