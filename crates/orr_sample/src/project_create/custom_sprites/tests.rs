use super::*;

fn fixture(root: &Path) -> PathBuf {
    let source = root.join("actor");
    fs::create_dir(&source).unwrap();
    let mut manifest: orr_package::Manifest =
        serde_json::from_slice(template::SOURCE[0].1).unwrap();
    manifest.name = "custom-actor".into();
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
fn options(root: &Path, package: PathBuf) -> CustomSpriteOptions {
    CustomSpriteOptions {
        base: CreateOptions {
            output: root.join("created"),
            template: CUSTOM_SPRITE_TEMPLATE.into(),
            seed: "custom-test".into(),
        },
        package,
        idle: "idle".into(),
        walk: "walk".into(),
        units_per_pixel: 2.0,
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
                    fs::write(transaction.join("project/arena.sprites.json"), "{}").unwrap();
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
        assert!(create_custom_sprites(&options).is_err(), "{name}");
        assert_eq!(capture(&options.package).unwrap(), original);
        cleaned(temp.path(), &options.base.output);
    }
}

#[test]
fn invalid_scale_or_clip_never_reaches_a_stage() {
    let temp = tempfile::tempdir().unwrap();
    let source = fixture(temp.path());
    let mut options = options(temp.path(), source);
    for scale in [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        0.0,
        -1.0,
        0.00001,
        100.01,
    ] {
        options.units_per_pixel = scale;
        let error = create_custom_with(
            &options,
            |_, _| panic!("invalid scale staged"),
            |_, _| panic!("invalid scale published"),
        )
        .unwrap_err();
        assert!(error.contains("units per pixel"), "{error}");
        cleaned(temp.path(), &options.base.output);
    }
    options.units_per_pixel = 2.0;
    for (idle, walk) in [("missing", "walk"), ("idle", "missing"), ("", "walk")] {
        options.idle = idle.into();
        options.walk = walk.into();
        let error = create_custom_with(
            &options,
            |_, _| panic!("invalid clip staged"),
            |_, _| panic!("invalid clip published"),
        )
        .unwrap_err();
        assert!(error.contains("clip"), "{error}");
        cleaned(temp.path(), &options.base.output);
    }
}

#[test]
fn collision_created_at_publication_is_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let options = options(temp.path(), fixture(temp.path()));
    let result = create_custom_with(
        &options,
        |point, _| {
            if point == "before-publish" {
                fs::create_dir(&options.base.output).unwrap();
                fs::write(options.base.output.join("keep"), "existing destination").unwrap();
            }
            Ok(())
        },
        publish_no_replace,
    );
    assert!(result.unwrap_err().contains("already exists"));
    assert_eq!(
        fs::read(options.base.output.join("keep")).unwrap(),
        b"existing destination"
    );
    assert_eq!(fs::read_dir(&options.base.output).unwrap().count(), 1);
    assert!(!fs::read_dir(temp.path()).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".orr-create-")));
}
