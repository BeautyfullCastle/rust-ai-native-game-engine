use super::*;
use crate::project_sprites::{Document, Source};
use orr_reflect::{Scene, SceneEntity, Value};
use std::os::unix::fs::symlink;

fn options(root: &Path, name: &str, seed: &str) -> CreateOptions {
    CreateOptions {
        output: root.join(name),
        template: TEMPLATE.into(),
        seed: seed.into(),
    }
}
fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                visit(root, &entry.path(), out);
            } else {
                out.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .into(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}
fn assert_no_stages(root: &Path) {
    assert!(!fs::read_dir(root).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".orr-create-")));
}

#[test]
fn reproducible_creation_uses_real_install_and_admission() {
    let temp = tempfile::tempdir().unwrap();
    let a = create(&options(temp.path(), "a", "same-seed")).unwrap();
    let b = create(&options(temp.path(), "different destination", "same-seed")).unwrap();
    assert_eq!(files(&a.output), files(&b.output));
    assert_eq!(a.entity_guids, b.entity_guids);
    let prepared = PreparedRuntime::open(&a.output).unwrap();
    assert_eq!(prepared.initial_frame().checksum(), a.initial_checksum);
    assert_eq!(a.initial_checksum, b.initial_checksum);
    assert_eq!(a.entity_guids.len(), 2);
    let package = orr_package::Project::open(&a.output, compiled_runtime()).unwrap();
    let lock = package.verify().unwrap();
    assert_eq!(lock.packages.len(), 1);
    assert_eq!(lock.direct.len(), 1);
    for (name, bytes) in template::SOURCE.iter().skip(1) {
        assert_eq!(package.read_asset(template::PACKAGE, name).unwrap(), *bytes);
    }
    for (index, guid) in a.entity_guids.iter().enumerate() {
        assert_eq!(guid.as_str().len(), 34);
        assert!(guid.as_str().ends_with(&format!("{:08x}", index + 1)));
        let entity = prepared.index().entity(guid).unwrap();
        assert_eq!(entity.index, index as u32);
        assert_eq!(
            prepared
                .initial_frame()
                .get::<orr_testgame::PlayerTag>(entity)
                .unwrap()
                .slot,
            index as u32
        );
    }
    let scene = prepared.project().scene().scene();
    assert_eq!(
        scene.entities[&a.entity_guids[0]].name.as_deref(),
        Some("hero")
    );
    assert_eq!(
        scene.entities[&a.entity_guids[1]].name.as_deref(),
        Some("target")
    );
    let sprites = &prepared.project().sprites().unwrap().document;
    assert_eq!(
        sprites.camera_follow.as_deref(),
        Some(a.entity_guids[0].as_str())
    );
    for binding in sprites.bindings.values() {
        assert_eq!(
            binding.source,
            Source::Locomotion {
                idle: "idle".into(),
                walk: "walk".into()
            }
        );
    }
    assert_eq!(prepared.restart_seed().config().game_id, "Arena");
    assert_no_stages(temp.path());
}

#[test]
fn different_seed_changes_only_authored_namespace_and_provenance() {
    let temp = tempfile::tempdir().unwrap();
    let a = create(&options(temp.path(), "a", "one")).unwrap();
    let b = create(&options(temp.path(), "b", "two")).unwrap();
    assert!(a.entity_guids.iter().all(|g| !b.entity_guids.contains(g)));
    assert_eq!(
        a.initial_checksum, b.initial_checksum,
        "GUID lexical order must preserve baked entity ordering"
    );
    for (path, bytes) in files(&a.output) {
        if !["arena.scene.yaml", "arena.sprites.json", "README.md"].contains(&path.as_str()) {
            assert_eq!(
                bytes,
                files(&b.output)[&path],
                "asset/lock changed with seed: {path}"
            );
        }
    }
    for report in [&a, &b] {
        let prepared = PreparedRuntime::open(&report.output).unwrap();
        assert_eq!(
            prepared
                .project()
                .sprites()
                .unwrap()
                .document
                .bindings
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            report
                .entity_guids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn every_transaction_checkpoint_cleans_only_its_owned_stage() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("unrelated")).unwrap();
    fs::write(temp.path().join("unrelated/keep"), "untouched").unwrap();
    let mut encountered = BTreeSet::new();
    create_with(
        &options(temp.path(), "baseline", "seed"),
        |name, _| {
            encountered.insert(name.to_owned());
            Ok(())
        },
        publish_no_replace,
    )
    .unwrap();
    assert!(encountered.len() >= 7);
    for (index, fail) in encountered.into_iter().enumerate() {
        let options = options(temp.path(), &format!("fail-{index}"), "seed");
        let result = create_with(
            &options,
            |name, _| {
                if name == fail {
                    Err(format!("injected {name}"))
                } else {
                    Ok(())
                }
            },
            publish_no_replace,
        );
        assert!(result.unwrap_err().contains("injected"));
        assert!(!options.output.exists());
        assert_no_stages(temp.path());
        assert_eq!(
            fs::read(temp.path().join("unrelated/keep")).unwrap(),
            b"untouched"
        );
    }
}

#[test]
fn package_install_failure_is_not_published_or_replaced_with_fixture_lock() {
    let temp = tempfile::tempdir().unwrap();
    let options = options(temp.path(), "failed", "seed");
    let result = create_with(
        &options,
        |name, stage| {
            if name == "before-install" {
                fs::remove_file(stage.join("source/lantern_keeper.png")).unwrap();
            }
            Ok(())
        },
        publish_no_replace,
    );
    assert!(result.unwrap_err().contains("installation"));
    assert!(!options.output.exists());
    assert_no_stages(temp.path());
}

#[test]
fn whole_lock_verification_precedes_runtime_validation() {
    let temp = tempfile::tempdir().unwrap();
    let options = options(temp.path(), "failed", "seed");
    let result = create_with(
        &options,
        |name, stage| {
            if name == "installed" {
                let project = stage.join("project");
                let object = fs::read_dir(project.join(".orr/packages/objects"))
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path();
                fs::write(
                    object.join("lantern_keeper.rgba"),
                    b"unused but declared and mandatory",
                )
                .unwrap();
            }
            Ok(())
        },
        publish_no_replace,
    );
    assert!(result.unwrap_err().contains("verification"));
    assert!(!options.output.exists());
    assert_no_stages(temp.path());
}

#[test]
fn stage_file_closure_and_bytes_are_rechecked_before_publish() {
    let temp = tempfile::tempdir().unwrap();
    for change in [
        "scene",
        "extra-file",
        "extra-directory",
        "root-symlink",
        "missing",
        "symlink",
        "fifo",
        "larger",
    ] {
        let options = options(temp.path(), change, "seed");
        let result = create_with(
            &options,
            |name, stage| {
                if name == "before-publish" {
                    let root = stage.join("project");
                    match change {
                        "scene" => fs::write(root.join("arena.scene.yaml"), b"changed").unwrap(),
                        "extra-file" => fs::write(root.join("settings.json"), b"private").unwrap(),
                        "extra-directory" => fs::create_dir(root.join("cache")).unwrap(),
                        "root-symlink" => {
                            fs::rename(&root, stage.join("relocated")).unwrap();
                            symlink("relocated", &root).unwrap();
                        }
                        "missing" => fs::remove_file(root.join("README.md")).unwrap(),
                        "symlink" => {
                            fs::remove_file(root.join("README.md")).unwrap();
                            symlink("/etc/passwd", root.join("README.md")).unwrap();
                        }
                        "fifo" => {
                            rustix::fs::mkfifoat(
                                rustix::fs::CWD,
                                root.join("fifo"),
                                rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
                            )
                            .unwrap();
                        }
                        "larger" => {
                            fs::write(root.join("README.md"), vec![0u8; MAX_FILE_BYTES + 1])
                                .unwrap()
                        }
                        _ => unreachable!(),
                    }
                }
                Ok(())
            },
            publish_no_replace,
        );
        assert!(result.is_err(), "accepted {change}");
        assert!(!options.output.exists());
        assert_no_stages(temp.path());
    }
}

#[test]
fn existing_destinations_are_preserved_including_empty_and_special() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("existing");
    fs::create_dir(&output).unwrap();
    assert!(create(&options(temp.path(), "existing", "seed"))
        .unwrap_err()
        .contains("exists"));
    assert!(output.is_dir());
    fs::write(output.join("keep"), b"original").unwrap();
    assert!(create(&options(temp.path(), "existing", "seed")).is_err());
    assert_eq!(fs::read(output.join("keep")).unwrap(), b"original");
    fs::write(temp.path().join("file"), b"original").unwrap();
    assert!(create(&options(temp.path(), "file", "seed")).is_err());
    assert_eq!(fs::read(temp.path().join("file")).unwrap(), b"original");
    symlink("absent", temp.path().join("link")).unwrap();
    assert!(create(&options(temp.path(), "link", "seed")).is_err());
    assert_eq!(
        fs::read_link(temp.path().join("link")).unwrap(),
        Path::new("absent")
    );
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        temp.path().join("fifo"),
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    assert!(create(&options(temp.path(), "fifo", "seed")).is_err());
    assert_no_stages(temp.path());
}

#[test]
fn concurrent_destination_wins_atomic_publication_without_overwrite() {
    let temp = tempfile::tempdir().unwrap();
    for kind in ["empty", "populated", "link", "file"] {
        let options = options(temp.path(), kind, "seed");
        let result = create_with(
            &options,
            |_, _| Ok(()),
            |stage, output| {
                match kind {
                    "empty" => fs::create_dir(output).unwrap(),
                    "populated" => {
                        fs::create_dir(output).unwrap();
                        fs::write(output.join("keep"), b"winner").unwrap();
                    }
                    "link" => symlink("not-here", output).unwrap(),
                    "file" => fs::write(output, b"winner").unwrap(),
                    _ => unreachable!(),
                }
                publish_no_replace(stage, output)
            },
        );
        assert!(result.unwrap_err().contains("atomic no-replace"));
        assert!(fs::symlink_metadata(&options.output).is_ok());
        match kind {
            "empty" => assert_eq!(fs::read_dir(options.output).unwrap().count(), 0),
            "populated" => assert_eq!(fs::read(options.output.join("keep")).unwrap(), b"winner"),
            "link" => assert_eq!(
                fs::read_link(options.output).unwrap(),
                Path::new("not-here")
            ),
            "file" => assert_eq!(fs::read(options.output).unwrap(), b"winner"),
            _ => unreachable!(),
        }
        assert_no_stages(temp.path());
    }
}

#[test]
fn unsupported_publication_fails_closed_and_cleans_owned_stage() {
    let temp = tempfile::tempdir().unwrap();
    let options = options(temp.path(), "failed", "seed");
    let result = create_with(
        &options,
        |_, _| Ok(()),
        |stage, output| {
            crate::project_publish::publish_with(stage, output, |_, _| {
                Err(rustix::io::Errno::NOSYS)
            })
        },
    );
    assert!(result.unwrap_err().contains("atomic no-replace"));
    assert!(!options.output.exists());
    assert_no_stages(temp.path());
}

#[test]
fn invalid_template_seed_and_paths_do_not_create_anything() {
    let temp = tempfile::tempdir().unwrap();
    for seed in [
        "",
        "space seed",
        "한글",
        "../seed",
        &"a".repeat(MAX_SEED_BYTES + 1),
    ] {
        assert!(create(&options(temp.path(), "out", seed))
            .unwrap_err()
            .contains("seed"));
    }
    let mut options = options(temp.path(), "out", "seed");
    options.template = "arbitrary-directory".into();
    assert!(create(&options).unwrap_err().contains("template"));
    options.template = TEMPLATE.into();
    for path in [
        PathBuf::from("relative"),
        PathBuf::from("/"),
        temp.path().join("../escape"),
        temp.path().join("./dot"),
        temp.path().join("missing/out"),
        temp.path().join("a".repeat(201)),
    ] {
        options.output = path;
        assert!(create(&options).is_err());
    }
    let target = temp.path().join("target");
    fs::create_dir(&target).unwrap();
    symlink(&target, temp.path().join("alias")).unwrap();
    options.output = temp.path().join("alias/out");
    assert!(create(&options).unwrap_err().contains("nonsymlink"));
    fs::write(temp.path().join("file-parent"), b"keep").unwrap();
    options.output = temp.path().join("file-parent/out");
    assert!(create(&options).is_err());
    assert_no_stages(temp.path());
}

#[test]
fn parent_permission_failure_preserves_parent_and_prior_content() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("read-only");
    fs::create_dir(&parent).unwrap();
    fs::write(parent.join("keep"), b"original").unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o500)).unwrap();
    let result = create(&options(&parent, "out", "seed"));
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    if rustix::process::geteuid().is_root() {
        return;
    }
    assert!(result.is_err());
    assert!(!parent.join("out").exists());
    assert_eq!(fs::read(parent.join("keep")).unwrap(), b"original");
    assert_no_stages(&parent);
}

#[test]
fn typed_remap_covers_nested_references_comments_bindings_and_follow() {
    let old = Guid::from_u32(1);
    let new = Guid::parse("e_0123456789abcdef0123456700000001").unwrap();
    let mut scene = Scene::default();
    let reference = Value::EntityGuid(Some(old.to_string()));
    scene.entities.insert(
        old.clone(),
        SceneEntity {
            name: Some(old.to_string()),
            components: vec![(
                "Link".into(),
                Value::Struct(vec![(
                    "nested".into(),
                    Value::Array(vec![Value::Variant(
                        "variant".into(),
                        vec![
                            ("target".into(), reference.clone()),
                            ("none".into(), Value::EntityGuid(None)),
                        ],
                    )]),
                )]),
            )],
        },
    );
    scene.singletons = vec![("Global".into(), reference)];
    scene
        .comments
        .insert(format!("entity:{old}"), vec!["entity notice".into()]);
    scene.comments.insert(
        format!("component:{old}:Link"),
        vec!["component notice".into()],
    );
    scene
        .comments
        .insert("singleton:Global".into(), vec!["singleton notice".into()]);
    let (_, mut sprites) = template::documents("seed").unwrap();
    let binding = sprites.bindings.values().next().unwrap().clone();
    sprites.bindings = BTreeMap::from([(old.to_string(), binding)]);
    sprites.camera_follow = Some(old.to_string());
    template::remap(
        &mut scene,
        &mut sprites,
        &BTreeMap::from([(old.clone(), new.clone())]),
    )
    .unwrap();
    assert!(scene.entities.contains_key(&new));
    assert_eq!(
        scene.entities[&new].name.as_deref(),
        Some(old.as_str()),
        "untyped text must not be rewritten"
    );
    assert_eq!(
        scene.singletons[0].1,
        Value::EntityGuid(Some(new.to_string()))
    );
    assert!(format!("{:?}", scene.entities[&new].components).contains(new.as_str()));
    assert!(!format!("{:?}", scene.entities[&new].components).contains(old.as_str()));
    assert!(scene.comments.contains_key(&format!("entity:{new}")));
    assert!(scene
        .comments
        .contains_key(&format!("component:{new}:Link")));
    assert!(scene.comments.contains_key("singleton:Global"));
    assert_eq!(sprites.camera_follow.as_deref(), Some(new.as_str()));
    assert_eq!(
        sprites.bindings.keys().collect::<Vec<_>>(),
        [&new.to_string()]
    );
    assert!(template::remap(&mut scene, &mut sprites, &BTreeMap::new()).is_err());
}

#[test]
fn generated_closure_preserves_licenses_and_excludes_user_data() {
    let temp = tempfile::tempdir().unwrap();
    let report = create(&options(temp.path(), "out", "seed")).unwrap();
    let all = files(&report.output);
    assert_eq!(all.len(), 10);
    let scene = String::from_utf8(all["arena.scene.yaml"].clone()).unwrap();
    assert!(scene.contains("Permission is hereby granted"));
    assert!(scene.contains("Copyright (c) 2026 Orrery contributors"));
    assert!(scene.contains("THE SOFTWARE IS PROVIDED"));
    assert!(all.keys().any(|p| p.ends_with("/LICENSE.txt")));
    for key in all.keys() {
        assert!(
            ![
                "writer.lock",
                "cache",
                "settings",
                ".git",
                "source/",
                "generate.py",
                "capture",
                "replay"
            ]
            .iter()
            .any(|part| key.contains(part)),
            "unexpected {key}"
        );
    }
    let sprites = Document::from_bytes(&all["arena.sprites.json"]).unwrap();
    assert_eq!(sprites.project, ".");
}

#[cfg(feature = "collect-dodge")]
mod collect {
    use super::*;
    use crate::collect_project::{PreparedProject, ProgressSupport, SpriteSupport};
    const ID: &str = "12345678-1234-4234-8234-123456789abc";
    fn opts(root: &Path, name: &str) -> CreateOptions {
        CreateOptions {
            template: COLLECT_TEMPLATE.into(),
            ..options(root, name, "same-seed")
        }
    }
    fn open(path: &Path) -> PreparedProject {
        PreparedProject::open_with_presentation(
            path,
            ProgressSupport::MetadataOnly,
            SpriteSupport::Supported,
        )
        .unwrap()
    }
    #[test]
    fn collect_creation_identity_is_explicit_reproducible_and_copy_stable() {
        let temp = tempfile::tempdir().unwrap();
        let a = create_collect(&opts(temp.path(), "a"), ID).unwrap();
        let b = create_collect(&opts(temp.path(), "b"), ID).unwrap();
        assert_eq!(files(&a.output), files(&b.output));
        assert_eq!(a.entity_guids.len(), 4);
        assert_eq!(
            a.initial_checksum,
            open(&a.output).scene().frame().checksum()
        );
        let before = files(&a.output);
        assert!(PreparedProject::open(&a.output).is_err());
        assert!(
            PreparedProject::open_with_progress(&a.output, ProgressSupport::MetadataOnly).is_err()
        );
        let p = open(&a.output);
        assert_eq!(p.progress().unwrap().game_id, ID);
        assert_eq!(p.sprites().unwrap().document.bindings.len(), 4);
        for (i, guid) in a.entity_guids.iter().enumerate() {
            let entity = p.scene().index().entity(guid).unwrap();
            let actor = p
                .scene()
                .frame()
                .get::<orr_games::collect_dodge_game::CollectActor>(entity)
                .unwrap();
            assert_eq!(actor.kind, [0, 1, 1, 2][i]);
            assert_eq!(actor.ordinal, [0, 0, 1, 0][i]);
            let sprites = p.sprites().unwrap();
            let binding = &sprites.document.bindings[guid.as_str()];
            let asset = &sprites.assets[&(binding.package.clone(), binding.document.clone())];
            let region = asset
                .document
                .region(binding.region(&asset.document, 0).unwrap())
                .unwrap();
            let opaque = (region.y..region.y + region.height)
                .flat_map(|y| {
                    (region.x..region.x + region.width)
                        .map(move |x| (y * asset.document.atlas().width + x) as usize * 4)
                })
                .filter(|&offset| asset.rgba[offset + 3] == 255)
                .count();
            assert!(
                opaque > 10,
                "declared actor sprite must have visible opaque texels"
            );
        }

        assert_eq!(
            p.sprites().unwrap().document.camera_follow.as_deref(),
            Some(a.entity_guids[0].as_str())
        );
        assert_eq!(files(&a.output), before);
        let c = create_collect(
            &opts(temp.path(), "c"),
            "12345678-1234-4234-8234-123456789abd",
        )
        .unwrap();
        assert_eq!(a.entity_guids, c.entity_guids);
        assert_eq!(a.initial_checksum, c.initial_checksum);
        assert_ne!(open(&c.output).progress(), p.progress());
        assert_eq!(
            fs::read(a.output.join("level.scene.yaml")).unwrap(),
            fs::read(c.output.join("level.scene.yaml")).unwrap()
        );
        let mut changed = opts(temp.path(), "different-seed");
        changed.seed = "new-authoring-namespace".into();
        let d = create_collect(&changed, ID).unwrap();
        assert_ne!(a.entity_guids, d.entity_guids);
        assert_eq!(a.initial_checksum, d.initial_checksum);
        assert_eq!(open(&d.output).progress().unwrap().game_id, ID);
        assert_no_stages(temp.path());
    }
    #[test]
    fn collect_invalid_uuid_or_template_never_creates_output() {
        let temp = tempfile::tempdir().unwrap();
        for id in [
            "",
            "12345678-1234-1234-8234-123456789abc",
            "12345678-1234-4234-7234-123456789abc",
            "12345678-1234-4234-8234-123456789ABC",
            "00000000-0000-0000-0000-000000000000",
        ] {
            assert!(create_collect(&opts(temp.path(), "bad"), id).is_err());
            assert!(!temp.path().join("bad").exists());
        }
        assert!(create(&opts(temp.path(), "bad")).is_err());
        assert!(create_collect(&options(temp.path(), "bad", "seed"), ID).is_err());
        assert_no_stages(temp.path());
    }
    #[test]
    fn collect_existing_output_and_special_parents_are_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let options = opts(temp.path(), "existing");
        create_collect(&options, ID).unwrap();
        let before = files(&options.output);
        assert!(create_collect(&options, ID).is_err());
        assert_eq!(files(&options.output), before);
        symlink(&options.output, temp.path().join("alias")).unwrap();
        assert!(create_collect(&opts(&temp.path().join("alias"), "new"), ID).is_err());
        assert_no_stages(temp.path());
    }
    #[test]
    fn collect_corrupted_package_aborts_owned_transaction() {
        let temp = tempfile::tempdir().unwrap();
        let progress = orr_package::ProjectProgress {
            schema: 1,
            game_id: ID.into(),
            profile: orr_package::ProgressProfile::CollectDodgeHighscoreV1,
        };
        let result = create_transaction(
            &opts(temp.path(), "output"),
            Some(&progress),
            |stage, root| {
                if stage == "before-install" {
                    fs::write(root.join("source/lantern_keeper.png"), b"corrupt").unwrap();
                }
                Ok(())
            },
            publish_no_replace,
        );
        assert!(result.is_err());
        assert!(!temp.path().join("output").exists());
        assert_no_stages(temp.path());
    }
}
