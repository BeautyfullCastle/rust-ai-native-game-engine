//! Actual explicitly built CLI process. CPU asset/project admission only;
//! no native game window, GPU pixels or physical playtest is claimed.
use super::animated_tests::animated_fixture;
use super::*;
fn call(executable: &std::ffi::OsStr, f: &Fixture, output: &Path) -> std::process::Output {
    std::process::Command::new(executable)
        .arg("--project")
        .arg(&f.root)
        .arg("--source")
        .arg(&f.source)
        .arg("--output")
        .arg(output)
        .arg("--acknowledge-model-changes")
        .output()
        .unwrap()
}
fn no_stages(root: &Path) {
    assert!(fs::read_dir(root).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".orr-room-package-upgrade-")
    }));
}
#[test]
#[ignore = "requires explicitly built ORR_ROOM_PACKAGE_UPGRADE_TOOL with package-upgrade-animated"]
fn actual_cli_animated_upgrade_reopens_without_sources_and_rejects_incompatible_clip() {
    let executable = std::env::var_os("ORR_ROOM_PACKAGE_UPGRADE_TOOL")
        .expect("explicitly built animated-capable tool required");
    assert!(Path::new(&executable).is_absolute());
    let modes = vec![false];
    #[cfg(feature = "room-character")]
    let modes = {
        let mut modes = modes;
        modes.push(true);
        modes
    };
    for character in modes {
        let f = animated_fixture(character);
        let before = files(&f.root);
        let source = files(&f.source);
        let v1 = files(&f.v1);
        let old = open_room_project(&f.root).unwrap();
        let output = f.output("actual animated copy");
        let result = call(&executable, &f, &output);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let receipt: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(receipt["before_version"], "3.0.0");
        assert_eq!(receipt["after_version"], "4.0.0");
        assert_eq!(
            receipt["bindings"].as_array().unwrap().len(),
            old.models().document.bindings.len()
        );
        assert_eq!(files(&f.root), before);
        assert_eq!(files(&f.source), source);
        assert_eq!(files(&f.v1), v1);
        let copied = open_room_project(&output).unwrap();
        assert_eq!(
            copied.scene().frame().to_bytes(),
            old.scene().frame().to_bytes()
        );
        assert_eq!(
            fs::read(output.join("room.camera.json")).unwrap(),
            before[Path::new("room.camera.json")]
        );
        #[cfg(feature = "room-character")]
        if character {
            assert_eq!(
                fs::read(output.join("room.character.json")).unwrap(),
                before[Path::new("room.character.json")]
            );
            assert_eq!(
                copied.character().unwrap().document,
                old.character().unwrap().document
            );
        }
        for (guid, binding) in &copied.models().document.bindings {
            let original = &old.models().document.bindings[guid];
            assert_eq!(binding.kind, model_bindings::ModelKind::Animated);
            assert_eq!(binding.animation, original.animation);
            assert_eq!(binding.transform, original.transform);
            assert_eq!(binding.asset, original.asset);
            assert_ne!(binding.package_digest, original.package_digest);
            assert_ne!(binding.source_hash, original.source_hash);
            let asset = &copied.models().assets[&(TARGET.into(), MODEL.into())];
            binding.validate(asset).unwrap();
            let model = asset.animated_model().unwrap();
            assert_eq!(model.source().materials[0].base_color[..3], [0.2, 0.7, 0.4]);
            for (index, clip) in model.source().clips.iter().enumerate() {
                let time = clip.duration() * 0.5;
                assert!(model.sample_clip(index as u32, time).is_ok());
            }
        }
        let mut expected = old.scene().simulation().unwrap();
        let mut actual = copied.scene().simulation().unwrap();
        for tick in 1..=8 {
            let input = orr_sim::TickInputs::new(tick, 1);
            expected.step(&input);
            actual.step(&input);
            assert_eq!(expected.frame().to_bytes(), actual.frame().to_bytes());
        }
        let copied_files = files(&output);
        let collision = call(&executable, &f, &output);
        assert!(!collision.status.success());
        assert!(collision.stdout.is_empty());
        assert_eq!(files(&output), copied_files);
        assert_eq!(files(&f.root), before);
        drop(copied);
        drop(old);
        fs::rename(&f.root, f.output("hidden original")).unwrap();
        fs::rename(&f.source, f.output("hidden replacement")).unwrap();
        fs::rename(&f.v1, f.output("hidden old package")).unwrap();
        assert!(!f.root.exists() && !f.source.exists() && !f.v1.exists());
        let reopened = open_room_project(&output).unwrap();
        assert_eq!(
            reopened.models().document.bindings.len(),
            if character { 1 } else { 2 }
        );
        for asset in reopened.models().assets.values() {
            assert!(asset.animated_model().unwrap().sample_clip(0, 0.5).is_ok());
        }
        assert_eq!(files(&output), copied_files);
        no_stages(f.temp.path());
    }
    // Both source bindings select a valid old clip which the replacement lacks.
    let f = animated_fixture(false);
    let models = f.root.join("room.models.json");
    let mut bindings: Document = serde_json::from_slice(&fs::read(&models).unwrap()).unwrap();
    for binding in bindings.bindings.values_mut() {
        binding.animation.as_mut().unwrap().clip_index = 2;
    }
    fs::write(&models, serde_json::to_vec(&bindings).unwrap()).unwrap();
    open_room_project(&f.root).unwrap();
    let sourcepath = f.source.join(MODEL);
    let source =
        orr_model::animation::AnimatedModel::from_bytes(&fs::read(&sourcepath).unwrap()).unwrap();
    let mut shorter = source.source().clone();
    shorter.clips.truncate(1);
    fs::write(
        sourcepath,
        orr_model::animation::AnimatedModel::new(shorter)
            .unwrap()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    let before = files(f.temp.path());
    let output = f.output("incompatible output");
    let failure = call(&executable, &f, &output);
    assert!(!failure.status.success());
    assert!(failure.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&failure.stderr).contains("clip"),
        "{}",
        String::from_utf8_lossy(&failure.stderr)
    );
    assert!(!output.exists());
    assert_eq!(files(f.temp.path()), before);
    no_stages(f.temp.path());
    // An unrelated existing destination must also remain byte-for-byte intact.
    fs::create_dir(&output).unwrap();
    fs::write(output.join("keep"), b"foreign output").unwrap();
    let before = files(f.temp.path());
    let failure = call(&executable, &f, &output);
    assert!(!failure.status.success());
    assert!(failure.stdout.is_empty());
    assert_eq!(files(f.temp.path()), before);
    no_stages(f.temp.path());
}
