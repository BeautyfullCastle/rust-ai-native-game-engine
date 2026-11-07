use super::*;
fn root(t: &tempfile::TempDir, name: &str) -> PathBuf {
    t.path().canonicalize().unwrap().join(name)
}
fn light() -> Document {
    Document {
        follow_player: Some([1.0, 2.0, -1.0]),
        activation: Some(crate::room_lighting::Activation::KeyMissing),
        ..Document::enabled_default()
    }
}
#[test]
fn creates_reproducible_complete_lit_room_and_retains_base_game() {
    let t = tempfile::tempdir().unwrap();
    let a = root(&t, "first room");
    let b = root(&t, "second room");
    let d = light();
    let first = create(&a, "lit-room-test", &d).unwrap();
    let second = create(&b, "lit-room-test", &d).unwrap();
    assert_eq!(first.template, TEMPLATE);
    assert_eq!(first.initial_checksum, second.initial_checksum);
    assert_eq!(snapshot(&a).unwrap(), snapshot(&b).unwrap());
    let base = root(&t, "unlit baseline");
    let original = project_create::create(&CreateOptions {
        output: base.clone(),
        template: project_create::ROOM_TEMPLATE.into(),
        seed: "lit-room-test".into(),
    })
    .unwrap();
    assert_eq!(first.initial_checksum, original.initial_checksum);
    assert_eq!(first.entity_guids, original.entity_guids);
    let mut lit = snapshot(&a).unwrap();
    let mut unlit = snapshot(&base).unwrap();
    for path in ["orr.project.json", "README.md", "room.lighting.json"] {
        lit.remove(Path::new(path));
        unlit.remove(Path::new(path));
    }
    assert_eq!(lit, unlit);
    assert!(crate::room_project::PreparedProject::open(&a).is_err());
    let prepared = crate::room_project::PreparedProject::open_with_lighting(&a, true).unwrap();
    assert_eq!(prepared.lighting().unwrap().document, d);
    assert!(prepared
        .point_light_settings_for_frame(orr_bridge::FrameView::of(prepared.scene().frame()))
        .unwrap()
        .point_light
        .is_some());
}
#[test]
fn invalid_light_existing_outputs_and_symlink_ancestors_are_preserved() {
    let t = tempfile::tempdir().unwrap();
    let out = root(&t, "output");
    fs::create_dir(&out).unwrap();
    fs::write(out.join("sentinel"), b"existing").unwrap();
    let before = snapshot(&out).unwrap();
    assert!(create(&out, "seed", &light()).is_err());
    assert_eq!(snapshot(&out).unwrap(), before);
    let missing = root(&t, "absent");
    let mut d = light();
    d.point_light = None;
    assert!(create(&missing, "seed", &d).is_err());
    assert!(!missing.exists());
    assert!(create(&missing, "invalid seed", &light()).is_err());
    assert!(!missing.exists());
    assert!(create(Path::new("relative"), "seed", &light()).is_err());
    let alias = root(&t, "alias");
    std::os::unix::fs::symlink(t.path(), &alias).unwrap();
    assert!(create(&alias.join("game"), "seed", &light()).is_err());
}
#[test]
fn interrupted_tampered_and_concurrent_publication_leave_no_partial_project() {
    let t = tempfile::tempdir().unwrap();
    for stage in ["base-created", "before-validation", "before-publish"] {
        let out = root(&t, stage);
        assert!(create_with(
            &out,
            "seed",
            &light(),
            |name, _| if name == stage {
                Err("injected".into())
            } else {
                Ok(())
            },
            crate::project_publish::publish_no_replace
        )
        .is_err());
        assert!(!out.exists());
    }
    let out = root(&t, "changed");
    assert!(create_with(
        &out,
        "seed",
        &light(),
        |name, root| {
            if name == "before-publish" {
                fs::write(
                    root.join("room.lighting.json"),
                    Document::enabled_default().to_bytes()?,
                )
                .map_err(err)?;
            }
            Ok(())
        },
        crate::project_publish::publish_no_replace
    )
    .unwrap_err()
    .contains("bytes changed"));
    assert!(!out.exists());
    let out = root(&t, "collision");
    assert!(create_with(
        &out,
        "seed",
        &light(),
        |_, _| Ok(()),
        |stage, dest| {
            fs::create_dir(dest).map_err(err)?;
            fs::write(dest.join("sentinel"), b"other writer").map_err(err)?;
            crate::project_publish::publish_no_replace(stage, dest)
        }
    )
    .is_err());
    assert_eq!(fs::read(out.join("sentinel")).unwrap(), b"other writer");
    assert_eq!(fs::read_dir(&out).unwrap().count(), 1);
    assert!(!fs::read_dir(t.path()).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".orr-lit-room-")));
}
#[test]
fn lighting_input_is_bounded_regular_nonsymlink_and_copied_by_value() {
    let t = tempfile::tempdir().unwrap();
    let source = root(&t, "light.json");
    let d = light();
    fs::write(&source, d.to_bytes().unwrap()).unwrap();
    assert_eq!(read_lighting(&source).unwrap(), d);
    let out = root(&t, "game");
    create(&out, "seed", &read_lighting(&source).unwrap()).unwrap();
    fs::remove_file(&source).unwrap();
    assert_eq!(
        crate::room_project::PreparedProject::open_with_lighting(&out, true)
            .unwrap()
            .lighting()
            .unwrap()
            .document,
        d
    );
    fs::write(&source, vec![b' '; crate::room_lighting::MAX_BYTES + 1]).unwrap();
    assert!(read_lighting(&source).is_err());
    fs::write(&source, b"{bad}").unwrap();
    assert!(read_lighting(&source).is_err());
    let alias = root(&t, "alias");
    std::os::unix::fs::symlink(&source, &alias).unwrap();
    assert!(read_lighting(&alias).is_err());
    assert!(read_lighting(t.path()).is_err());
}

#[test]
fn fifo_lighting_input_is_rejected_without_waiting_for_a_writer() {
    let t = tempfile::tempdir().unwrap();
    let fifo = root(&t, "light.fifo");
    assert!(std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());
    assert!(read_lighting(&fifo).unwrap_err().contains("regular file"));
}

#[cfg(not(feature = "room-character"))]
#[test]
fn character_request_requires_feature_before_validating_or_opening_output() {
    let mut invalid = light();
    invalid.point_light = None;
    let error = create_with_character(
        Path::new("relative/no-parent/game"),
        "invalid seed",
        &invalid,
        true,
    )
    .unwrap_err();
    assert!(error.contains("requires room-character feature"));
    let t = tempfile::tempdir().unwrap();
    let out = root(&t, "not created");
    assert!(create_with_character(&out, "seed", &light(), true).is_err());
    assert_eq!(fs::read_dir(t.path()).unwrap().count(), 0);
}
#[cfg(feature = "room-character")]
#[test]
fn character_lit_starter_preserves_character_baseline_and_requires_both_consents() {
    use crate::room_project::{PreparedProject, RoomProjectCapabilities};
    let t = tempfile::tempdir().unwrap();
    let a = root(&t, "first animated light room");
    let b = root(&t, "second animated light room");
    let base = root(&t, "original character room");
    let d = light();
    let first = create_with_character(&a, "lit-character-test", &d, true).unwrap();
    let second = create_with_character(&b, "lit-character-test", &d, true).unwrap();
    assert_eq!(first.template, CHARACTER_TEMPLATE);
    assert_eq!(first.initial_checksum, second.initial_checksum);
    assert_eq!(snapshot(&a).unwrap(), snapshot(&b).unwrap());
    let original = project_create::create(&CreateOptions {
        output: base.clone(),
        template: project_create::ROOM_CHARACTER_TEMPLATE.into(),
        seed: "lit-character-test".into(),
    })
    .unwrap();
    assert_eq!(first.entity_guids, original.entity_guids);
    assert_eq!(first.initial_checksum, original.initial_checksum);
    let mut lit = snapshot(&a).unwrap();
    let mut unlit = snapshot(&base).unwrap();
    for path in ["orr.project.json", "README.md", "room.lighting.json"] {
        lit.remove(Path::new(path));
        unlit.remove(Path::new(path));
    }
    assert_eq!(
        lit, unlit,
        "character document, animated model/package and complete base game retained"
    );
    for capabilities in [
        RoomProjectCapabilities {
            lighting: false,
            character: false,
        },
        RoomProjectCapabilities {
            lighting: true,
            character: false,
        },
        RoomProjectCapabilities {
            lighting: false,
            character: true,
        },
    ] {
        assert!(PreparedProject::open_with_capabilities(&a, capabilities).is_err());
    }
    let prepared = PreparedProject::open_with_capabilities(
        &a,
        RoomProjectCapabilities {
            lighting: true,
            character: true,
        },
    )
    .unwrap();
    assert_eq!(prepared.lighting().unwrap().document, d);
    assert!(prepared.character().is_some());
    assert_eq!(prepared.scene().frame().checksum(), first.initial_checksum);
    let legacy = root(&t, "legacy create");
    let explicit = root(&t, "explicit static");
    create(&legacy, "same-static", &d).unwrap();
    let static_report = create_with_character(&explicit, "same-static", &d, false).unwrap();
    assert_eq!(static_report.template, TEMPLATE);
    assert_eq!(snapshot(&legacy).unwrap(), snapshot(&explicit).unwrap());
}
#[cfg(feature = "room-character")]
#[test]
fn character_sidecar_tampering_rejects_before_publication_without_touching_existing_output() {
    let t = tempfile::tempdir().unwrap();
    let out = root(&t, "never published");
    let error = create_profile(
        &out,
        "seed",
        &light(),
        true,
        |name, root| {
            if name == "before-publish" {
                let path = root.join("room.character.json");
                let mut bytes = fs::read(&path).map_err(err)?;
                bytes.push(b' ');
                fs::write(path, bytes).map_err(err)?;
            }
            Ok(())
        },
        crate::project_publish::publish_no_replace,
    )
    .unwrap_err();
    assert!(error.contains("bytes changed"));
    assert!(!out.exists());
    assert_eq!(fs::read_dir(t.path()).unwrap().count(), 0);
    fs::create_dir(&out).unwrap();
    fs::write(out.join("sentinel"), b"existing game").unwrap();
    assert!(create_with_character(&out, "seed", &light(), true).is_err());
    assert_eq!(fs::read(out.join("sentinel")).unwrap(), b"existing game");
    assert_eq!(fs::read_dir(&out).unwrap().count(), 1);
}
