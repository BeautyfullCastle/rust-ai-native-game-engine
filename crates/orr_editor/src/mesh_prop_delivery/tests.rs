use super::*;
use crate::{mesh_art, mesh_props};
use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use orr_remote::mesh_navigation::{authoring::create_mesh_scene, AgentSpec};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
};

fn fixture() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let scene = source.join("Shaded props.scene.yaml");
    let graph = StaticMeshGraph::bake(
        &[
            [FP::ZERO, FP::ZERO, FP::ZERO],
            [fp!(2), FP::ZERO, FP::ZERO],
            [FP::ZERO, FP::ZERO, fp!(2)],
            [fp!(2), FP::ZERO, fp!(2)],
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap();
    let mut doc = create_mesh_scene(
        &graph,
        AgentSpec {
            start: [fp!(0.25), FP::ZERO, fp!(0.25)],
            goal: [fp!(1.75), FP::ZERO, fp!(1.75)],
            max_vertical: FP::ZERO,
            distance_per_tick: fp!(0.25),
        },
    )
    .unwrap();
    fs::write(&scene, doc.save_yaml()).unwrap();
    let package = source.join("package");
    fs::create_dir(&package).unwrap();
    fs::write(
        package.join("model.glb"),
        include_bytes!("../../../orr_model/tests/fixtures/model.glb"),
    )
    .unwrap();
    fs::write(package.join("LICENSE.txt"), b"Test fixture license\n").unwrap();
    let manifest = orr_package::Manifest {
        schema: 1,
        name: "delivery-fixture".into(),
        version: "1.0.0".into(),
        engine: format!("={}", orr_package::Runtime::content_only().engine_version),
        capabilities: BTreeSet::from(["models".into()]),
        dependencies: BTreeMap::new(),
        files: BTreeSet::from(["model.glb".into(), "LICENSE.txt".into()]),
    };
    fs::write(
        package.join("orr.package.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let art =
        mesh_art::Snapshot::from_package(&package, "model.glb", "LICENSE.txt", graph.revision())
            .unwrap();
    let model = art.source_model_arc();
    let shading = crate::mesh_prop_shading::Settings::new(
        vec![crate::mesh_prop_shading::Slot {
            material: model.source().primitives[0].material,
            metallic: 0.8,
            roughness: 0.3,
        }],
        model,
    )
    .unwrap();
    mesh_props::save(
        &scene,
        Some(&mesh_props::Document {
            props: vec![mesh_props::Prop {
                id: 1,
                name: "shaded crate".into(),
                art,
                shading: Some(shading),
            }],
        }),
        None,
        graph.revision(),
    )
    .unwrap();
    (temp, scene)
}

fn sample_manifest() -> Manifest {
    let scene = "Shaded props.scene.yaml".to_owned();
    Manifest::new(
        scene.clone(),
        42,
        [7; 32],
        vec![
            Descriptor::new(scene.clone(), b"scene", 0o644),
            Descriptor::new(format!("{scene}.mesh-props.json"), b"props", 0o644),
            Descriptor::new(RUNTIME_NAME.into(), b"runtime", 0o755),
        ],
    )
}

fn write_file(path: &Path, bytes: &[u8], mode: u32) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

// No runtime is executed here: this synthetic ELF header exercises admission only.
fn bundle(temp: &Path, scene: &Path) -> PathBuf {
    let source = SavedSource::capture(scene).unwrap();
    let root = temp.join("bundle");
    fs::create_dir(&root).unwrap();
    let mut files = Vec::new();
    for (name, snapshot) in &source.files {
        write_file(&root.join(name), &snapshot.bytes, 0o644);
        files.push(Descriptor::new(name.clone(), &snapshot.bytes, 0o644));
    }
    let mut runtime = vec![0u8; 64];
    runtime[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    runtime[16..18].copy_from_slice(&2u16.to_le_bytes());
    runtime[18..20].copy_from_slice(&62u16.to_le_bytes());
    runtime[52..54].copy_from_slice(&64u16.to_le_bytes());
    write_file(&root.join(RUNTIME_NAME), &runtime, 0o755);
    files.push(Descriptor::new(RUNTIME_NAME.into(), &runtime, 0o755));
    let manifest = Manifest::new(
        scene.file_name().unwrap().to_str().unwrap().into(),
        source.initial_checksum(),
        source.graph_revision(),
        files,
    );
    write_file(
        &root.join(MANIFEST_NAME),
        &manifest.encode().unwrap(),
        0o644,
    );
    root
}

#[test]
fn exact_basename_and_strict_manifest_round_trip() {
    let manifest = sample_manifest();
    assert_eq!(
        Manifest::parse(&manifest.encode().unwrap()).unwrap(),
        manifest
    );
    for name in [
        "",
        "scene.yaml",
        ".scene.yaml",
        "../a.scene.yaml",
        "sub/a.scene.yaml",
        "a\\b.scene.yaml",
        "a:b.scene.yaml",
        "a\n.scene.yaml",
    ] {
        assert!(scene_name(name).is_err(), "{name:?}");
    }
    assert!(scene_name(&format!("{}.scene.yaml", "a".repeat(170))).is_err());
    let mut renamed = manifest.clone();
    renamed.scene = "renamed.scene.yaml".into();
    assert!(renamed.validate().is_err());
    for field in ["schema", "profile", "target", "capability"] {
        let mut value = serde_json::to_value(&manifest).unwrap();
        value[field] = if field == "schema" {
            serde_json::json!(2)
        } else {
            serde_json::json!("unsupported")
        };
        assert!(
            Manifest::parse(&serde_json::to_vec(&value).unwrap()).is_err(),
            "{field}"
        );
    }
    for nested in [false, true] {
        let mut value = serde_json::to_value(&manifest).unwrap();
        if nested {
            value["files"][0]["unknown"] = true.into();
        } else {
            value["unknown"] = true.into();
        }
        assert!(Manifest::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    assert!(Manifest::parse(&vec![b' '; MAX_MANIFEST + 1]).is_err());
}

#[test]
fn descriptor_inventory_modes_hashes_and_bounds_are_closed() {
    let manifest = sample_manifest();
    for i in 0..3 {
        for bytes in [0, manifest.limit(&manifest.files[i].name).unwrap().0 + 1] {
            let mut bad = manifest.clone();
            bad.files[i].bytes = bytes;
            assert!(bad.validate().is_err());
        }
        let mut bad = manifest.clone();
        bad.files[i].mode ^= 0o111;
        assert!(bad.validate().is_err());
        for digest in [
            "0".repeat(63),
            "A".repeat(64),
            "g".repeat(64),
            format!("{}\n", "0".repeat(63)),
        ] {
            let mut bad = manifest.clone();
            bad.files[i].sha256 = digest;
            assert!(bad.validate().is_err());
        }
        let mut missing = manifest.clone();
        missing.files.remove(i);
        assert!(missing.validate().is_err());
    }
    let mut duplicate = manifest.clone();
    duplicate.files.push(duplicate.files[0].clone());
    assert!(duplicate.validate().is_err());
    for name in [
        "extra",
        "../escape",
        "sub/file",
        MANIFEST_NAME,
        "Shaded props.scene.yaml.mesh-art.json",
    ] {
        let mut bad = manifest.clone();
        bad.files.push(Descriptor::new(name.into(), b"x", 0o644));
        assert!(bad.validate().is_err());
    }
    let mut optional = manifest;
    for suffix in [".mesh-camera.json", ".mesh-lighting.json"] {
        optional.files.push(Descriptor::new(
            format!("{}{suffix}", optional.scene),
            b"x",
            0o644,
        ));
        optional.validate().unwrap();
        let last = optional.files.len() - 1;
        optional.files[last].bytes = MAX_SETTINGS + 1;
        assert!(optional.validate().is_err());
        optional.files[last].bytes = 1;
    }
}

#[test]
fn saved_shaded_source_preserves_exact_filename_and_rejects_renaming() {
    let (temp, scene) = fixture();
    let source = SavedSource::capture(&scene).unwrap();
    source.recheck().unwrap();
    assert_eq!(source.scene_path(), scene);
    assert_eq!(source.files[0].0, "Shaded props.scene.yaml");
    let renamed = temp.path().join("renamed.scene.yaml");
    fs::copy(&scene, &renamed).unwrap();
    fs::copy(
        mesh_props::sidecar_path(&scene).unwrap(),
        mesh_props::sidecar_path(&renamed).unwrap(),
    )
    .unwrap();
    assert!(SavedSource::capture(&renamed).is_err());
}

#[test]
fn source_recheck_detects_mutation_deletion_replacement_and_mode_change() {
    for change in 0..4 {
        let (_temp, scene) = fixture();
        let source = SavedSource::capture(&scene).unwrap();
        let props = mesh_props::sidecar_path(&scene).unwrap();
        match change {
            0 => {
                let mut bytes = fs::read(&props).unwrap();
                bytes.push(b' ');
                fs::write(&props, bytes).unwrap();
            }
            1 => fs::remove_file(&props).unwrap(),
            2 => {
                let bytes = fs::read(&props).unwrap();
                fs::rename(&props, props.with_extension("old")).unwrap();
                fs::write(&props, bytes).unwrap();
            }
            _ => {
                let mode = fs::metadata(&props).unwrap().permissions().mode();
                fs::set_permissions(&props, fs::Permissions::from_mode(mode ^ 0o100)).unwrap();
            }
        }
        assert!(source.recheck().is_err(), "change {change}");
    }
    let (_temp, scene) = fixture();
    let source = SavedSource::capture(&scene).unwrap();
    let mut bytes = fs::read(&scene).unwrap();
    bytes.push(b'\n');
    fs::write(&scene, bytes).unwrap();
    assert!(source.recheck().is_err());
}

#[test]
fn unsupported_or_new_sidecars_and_symlinks_fail_closed() {
    for suffix in [
        ".mesh-art.json",
        ".mesh-animation.json",
        ".mesh-hud.json",
        ".mesh-camera.json",
        ".mesh-lighting.json",
    ] {
        let (_temp, scene) = fixture();
        let source = SavedSource::capture(&scene).unwrap();
        fs::write(
            scene.with_file_name(format!(
                "{}{suffix}",
                scene.file_name().unwrap().to_str().unwrap()
            )),
            b"{}",
        )
        .unwrap();
        assert!(source.recheck().is_err(), "{suffix}");
        assert!(SavedSource::capture(&scene).is_err(), "{suffix}");
    }
    let (temp, scene) = fixture();
    let props = mesh_props::sidecar_path(&scene).unwrap();
    let original = temp.path().join("props-original");
    fs::rename(&props, &original).unwrap();
    symlink(&original, &props).unwrap();
    assert!(SavedSource::capture(&scene).is_err());
    let linked_parent = temp.path().join("linked-source");
    symlink(scene.parent().unwrap(), &linked_parent).unwrap();
    assert!(SavedSource::capture(&linked_parent.join(scene.file_name().unwrap())).is_err());
}

#[test]
fn bundle_relocation_hash_inventory_and_executable_identity_are_checked() {
    let (temp, scene) = fixture();
    let original = bundle(temp.path(), &scene);
    let moved = temp.path().join("relocated bundle");
    fs::rename(original, &moved).unwrap();
    let verified = verify_bundle(&moved, &moved.join(RUNTIME_NAME)).unwrap();
    verified.recheck().unwrap();
    assert_eq!(
        verified.scene.file_name().unwrap(),
        scene.file_name().unwrap()
    );
    assert!(verify_bundle(&moved, &std::env::current_exe().unwrap()).is_err());
    fs::write(moved.join("unexpected"), b"x").unwrap();
    assert!(verified.recheck().is_err());
    assert!(verify(&moved, None).is_err());
    fs::remove_file(moved.join("unexpected")).unwrap();
    fs::create_dir(moved.join("unexpected-directory")).unwrap();
    assert!(verify(&moved, None).is_err());
    fs::remove_dir(moved.join("unexpected-directory")).unwrap();
    let path = moved.join(scene.file_name().unwrap());
    let mut bytes = fs::read(&path).unwrap();
    bytes.push(b'\n');
    fs::write(path, bytes).unwrap();
    assert!(verify(&moved, None).is_err());
}

#[test]
fn saved_trajectory_is_repeatable_and_bounded() {
    let (_temp, scene) = fixture();
    let first = player::trajectory(&scene, 60).unwrap();
    assert_eq!(first.tick, 60);
    assert_eq!(first.verified_tick, 60);
    assert_eq!(first, player::trajectory(&scene, 60).unwrap());
    assert!(player::trajectory(&scene, 6001).is_err());
}

#[test]
fn exporter_requires_explicit_trust_and_canonical_digest_before_execution() {
    let (temp, scene) = fixture();
    let source = SavedSource::capture(&scene).unwrap();
    let mut request = export::ExportRequest {
        runtime: temp.path().join("absent-runtime"),
        runtime_sha256: "0".repeat(64),
        output: temp.path().join("output"),
        trusted_runtime: false,
    };
    assert!(export::export_for_source(&request, &source)
        .unwrap_err()
        .contains("trust"));
    request.trusted_runtime = true;
    request.runtime_sha256 = "A".repeat(64);
    assert!(export::export_for_source(&request, &source)
        .unwrap_err()
        .contains("SHA256"));
    assert!(!request.output.exists());
}

// Build both delivery binaries first, then provide their absolute paths. These tests
// deliberately fail if invoked without the promised runtime; they never silently skip.
#[test]
#[ignore = "requires built delivery binaries via ORR_TEST_MESH_PROP_EXPORT and ORR_TEST_MESH_PROP_PLAYER"]
fn actual_cli_exports_relocates_and_replays_saved_shaded_props() {
    actual_cli(false);
}

#[test]
#[ignore = "requires built delivery binaries and a working headless GPU adapter"]
fn actual_cli_capture_restarts_with_identical_pixels() {
    actual_cli(true);
}

fn visual_fixture(scene: &Path) {
    let revision = SavedSource::capture(scene).unwrap().graph_revision();
    let (mut props, stamp) = mesh_props::load(scene, revision).unwrap().unwrap();
    let first = props.props[0].clone();
    props.props[0].art = first
        .art
        .clone()
        .with_transform([0.1, 0.1, 0.2], 0.35)
        .unwrap();
    let mut second = first.clone();
    second.id = 2;
    second.name = "rough dielectric".into();
    second.art = second.art.with_transform([1.2, 0.1, 0.2], 0.35).unwrap();
    let model = second.art.source_model_arc();
    second.shading = Some(
        crate::mesh_prop_shading::Settings::new(
            vec![crate::mesh_prop_shading::Slot {
                material: model.source().primitives[0].material,
                metallic: 0.0,
                roughness: 0.9,
            }],
            model,
        )
        .unwrap(),
    );
    let mut diffuse = first;
    diffuse.id = 3;
    diffuse.name = "diffuse control".into();
    diffuse.shading = None;
    diffuse.art = diffuse.art.with_transform([0.6, 0.1, 1.4], 0.35).unwrap();
    props.props.extend([second, diffuse]);
    mesh_props::save(scene, Some(&props), Some(&stamp), revision).unwrap();
    let camera = crate::mesh_camera::Document::from_orbit(orr_render::camera3d::OrbitCamera::new(
        [1.0, 0.0, 1.0],
        0.4,
        0.8,
        6.0,
    ))
    .unwrap();
    crate::mesh_camera_store::save(scene, Some(&camera), None, revision).unwrap();
    let lighting = crate::mesh_lighting::Document::from_lighting(Default::default())
        .unwrap()
        .with_post_process(orr_render::PostProcessSettings {
            enabled: true,
            bloom: true,
            ..Default::default()
        })
        .unwrap();
    crate::mesh_lighting_store::save(scene, Some(&lighting), None, revision).unwrap();
}

fn actual_cli(capture: bool) {
    let evidence = std::env::var_os("ORR_TEST_MESH_PROP_EVIDENCE_DIR").map(|root| {
        fs::create_dir_all(&root).expect("create requested evidence directory");
        let directory = tempfile::Builder::new()
            .prefix(if capture { "gpu-" } else { "cli-" })
            .tempdir_in(root)
            .expect("create unique evidence directory")
            .keep();
        eprintln!("mesh prop delivery evidence: {}", directory.display());
        directory
    });
    let exporter =
        PathBuf::from(std::env::var_os("ORR_TEST_MESH_PROP_EXPORT").expect("built exporter path"));
    let built_runtime =
        PathBuf::from(std::env::var_os("ORR_TEST_MESH_PROP_PLAYER").expect("built player path"));
    let (temp, scene) = fixture();
    visual_fixture(&scene);
    let source = SavedSource::capture(&scene).unwrap();
    let expected_start = player::trajectory(&scene, 0).unwrap();
    let expected_end = player::trajectory(&scene, 3).unwrap();
    assert_ne!(
        expected_start.agent_position_raw,
        expected_end.agent_position_raw
    );
    let runtime = temp.path().join("selected-runtime");
    fs::copy(built_runtime, &runtime).unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o755)).unwrap();
    let output = temp.path().join("delivered");
    let result = Command::new(exporter)
        .arg("--scene")
        .arg(&scene)
        .arg("--runtime")
        .arg(&runtime)
        .arg("--runtime-sha256")
        .arg(hash(&fs::read(&runtime).unwrap()))
        .arg("--trusted-runtime")
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "export: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let moved = temp.path().join("moved delivery");
    fs::rename(output, &moved).unwrap();
    for (name, snapshot) in &source.files {
        assert_eq!(
            fs::read(moved.join(name)).unwrap(),
            snapshot.bytes,
            "exact saved sidecar roundtrip: {name}"
        );
    }
    let loaded = mesh_props::load(
        &moved.join(scene.file_name().unwrap()),
        source.graph_revision(),
    )
    .unwrap()
    .unwrap()
    .0;
    assert_eq!(loaded.props.len(), 3);
    assert!(loaded.props[0].shading.is_some());
    assert!(loaded.props[1].shading.is_some());
    assert!(loaded.props[2].shading.is_none());
    if let Some(root) = &evidence {
        fs::copy(moved.join(MANIFEST_NAME), root.join(MANIFEST_NAME)).unwrap();
        for (name, _) in &source.files {
            fs::copy(moved.join(name), root.join(name)).unwrap();
        }
    }
    // Hide both authoring source and selected runtime, without touching shared binaries.
    fs::rename(scene.parent().unwrap(), temp.path().join("hidden-source")).unwrap();
    fs::rename(runtime, temp.path().join("hidden-runtime")).unwrap();
    let mut captures = Vec::new();
    for (ticks, expected) in [(0, expected_start), (3, expected_end)] {
        let png = temp.path().join(format!("capture-{ticks}.png"));
        let mut command = Command::new(moved.join(RUNTIME_NAME));
        command
            .current_dir(temp.path())
            .arg("--bundle")
            .arg(&moved)
            .arg("--ticks")
            .arg(ticks.to_string())
            .arg("--restart-check");
        if capture {
            command
                .arg("--capture")
                .arg(&png)
                .args(["--size", "256x256"]);
        } else {
            command.arg("--no-capture");
        }
        let result = command.output().unwrap();
        assert!(
            result.status.success(),
            "player: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        if let Some(root) = &evidence {
            fs::write(root.join(format!("receipt-{ticks}.json")), &result.stdout).unwrap();
            if capture {
                fs::copy(&png, root.join(format!("capture-{ticks}.png"))).unwrap();
            }
        }
        let receipt: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(
            receipt["trajectory"],
            serde_json::to_value(expected).unwrap()
        );
        assert_eq!(receipt["restart_checked"], true);
        assert_eq!(
            receipt["manifest_sha256"],
            hash(&fs::read(moved.join(MANIFEST_NAME)).unwrap())
        );
        if capture {
            assert_eq!(receipt["capture"]["width"], 256);
            assert_eq!(receipt["capture"]["height"], 256);
            let decoder = png::Decoder::new(fs::File::open(&png).unwrap());
            let mut reader = decoder.read_info().unwrap();
            let mut bytes = vec![0; reader.output_buffer_size()];
            let info = reader.next_frame(&mut bytes).unwrap();
            assert_eq!((info.width, info.height), (256, 256));
            bytes.truncate(info.buffer_size());
            assert_eq!(receipt["capture"]["rgba_sha256"], hash(&bytes));
            assert!(
                bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|p| p.as_slice() != &bytes[..4]),
                "capture is not a uniform clear frame"
            );
            captures.push(bytes);
        } else {
            assert!(receipt["capture"].is_null());
        }
    }
    if capture {
        assert_ne!(
            captures[0], captures[1],
            "moving agent must change captured pixels"
        );
        // Counterfactual uses identical camera, lighting, scene and third diffuse prop.
        // Whole-image difference proves an effect, not per-instance pixel isolation.
        let bundled_scene = moved.join(scene.file_name().unwrap());
        let props_path = mesh_props::sidecar_path(&bundled_scene).unwrap();
        let before: serde_json::Value =
            serde_json::from_slice(&fs::read(&props_path).unwrap()).unwrap();
        let (mut props, stamp) = mesh_props::load(&bundled_scene, source.graph_revision())
            .unwrap()
            .unwrap();
        props.props[0].shading = None;
        props.props[1].shading = None;
        mesh_props::save(
            &bundled_scene,
            Some(&props),
            Some(&stamp),
            source.graph_revision(),
        )
        .unwrap();
        fs::set_permissions(&props_path, fs::Permissions::from_mode(0o644)).unwrap();
        let bytes = fs::read(&props_path).unwrap();
        let after: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            before["props"][2], after["props"][2],
            "diffuse control must be unchanged"
        );
        let mut manifest = Manifest::parse(&fs::read(moved.join(MANIFEST_NAME)).unwrap()).unwrap();
        let descriptor = manifest
            .files
            .iter_mut()
            .find(|file| file.name == props_path.file_name().unwrap().to_str().unwrap())
            .unwrap();
        descriptor.bytes = bytes.len();
        descriptor.sha256 = hash(&bytes);
        fs::write(moved.join(MANIFEST_NAME), manifest.encode().unwrap()).unwrap();
        verify(&moved, None).unwrap();
        let png = temp.path().join("diffuse-control.png");
        let result = Command::new(moved.join(RUNTIME_NAME))
            .current_dir(temp.path())
            .arg("--bundle")
            .arg(&moved)
            .args(["--ticks", "3", "--size", "256x256", "--restart-check"])
            .arg("--capture")
            .arg(&png)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "diffuse counterfactual: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let receipt: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_ne!(
            receipt["capture"]["rgba_sha256"],
            hash(&captures[1]),
            "shading removal must change same-tick rendered pixels"
        );
        if let Some(root) = &evidence {
            fs::copy(png, root.join("diffuse-control.png")).unwrap();
            fs::write(root.join("diffuse-control-receipt.json"), result.stdout).unwrap();
            fs::write(root.join("diffuse-control-props.json"), bytes).unwrap();
            fs::write(
                root.join("diffuse-control-manifest.json"),
                manifest.encode().unwrap(),
            )
            .unwrap();
        }
    }
}

#[test]
fn required_props_and_scene_revision_cannot_be_omitted_or_forged() {
    let (_temp, scene) = fixture();
    let props = mesh_props::sidecar_path(&scene).unwrap();
    let original = fs::read(&props).unwrap();
    fs::remove_file(&props).unwrap();
    assert!(SavedSource::capture(&scene).is_err());
    for field in ["props", "graph_revision"] {
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        value[field] = if field == "props" {
            serde_json::json!([])
        } else {
            serde_json::json!(vec![0; 32])
        };
        fs::write(&props, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(SavedSource::capture(&scene).is_err(), "{field}");
    }
}

#[test]
fn saved_optional_camera_and_lighting_are_admitted_and_snapshotted() {
    let (_temp, scene) = fixture();
    let revision = SavedSource::capture(&scene).unwrap().graph_revision();
    let camera = crate::mesh_camera::Document::from_orbit(orr_render::camera3d::OrbitCamera::new(
        [1.0, 0.0, 1.0],
        0.4,
        0.8,
        6.0,
    ))
    .unwrap();
    crate::mesh_camera_store::save(&scene, Some(&camera), None, revision).unwrap();
    let lighting = crate::mesh_lighting::Document::from_lighting(Default::default()).unwrap();
    crate::mesh_lighting_store::save(&scene, Some(&lighting), None, revision).unwrap();
    let source = SavedSource::capture(&scene).unwrap();
    assert_eq!(source.files.len(), 4);
    source.recheck().unwrap();
    let path = crate::mesh_camera_store::sidecar_path(&scene).unwrap();
    let mut bytes = fs::read(&path).unwrap();
    bytes.push(b' ');
    fs::write(path, bytes).unwrap();
    assert!(source.recheck().is_err());
}

#[test]
fn bounded_snapshot_rejects_oversize_and_nonregular_inputs() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("input");
    fs::write(&path, b"12345").unwrap();
    assert!(io::Snapshot::read(&path, 4).is_err());
    io::Snapshot::read(&path, 5).unwrap().recheck().unwrap();
    assert!(io::Snapshot::read(temp.path(), 5).is_err());
}

#[cfg(feature = "mesh-mission")]
#[test]
fn authored_mission_is_not_silently_downgraded_to_base_navigation() {
    let temp = tempfile::tempdir().unwrap();
    let scene = temp.path().join("mission.scene.yaml");
    let _editor = crate::mesh_workspace::mission::prepare(&scene, &Default::default()).unwrap();
    assert!(source::admit_bytes(&fs::read(scene).unwrap()).is_err());
}

#[test]
#[ignore = "requires built player via ORR_TEST_MESH_PROP_PLAYER"]
fn actual_runtime_publication_checkpoints_fail_closed_and_never_clobber() {
    let runtime =
        PathBuf::from(std::env::var_os("ORR_TEST_MESH_PROP_PLAYER").expect("built player path"));
    let digest = hash(&fs::read(&runtime).unwrap());
    for checkpoint in [
        "stage-created",
        "before-smoke",
        "before-publish",
        "after-publish",
        "race",
    ] {
        let (temp, scene) = fixture();
        let source = SavedSource::capture(&scene).unwrap();
        let request = export::ExportRequest {
            runtime: runtime.clone(),
            runtime_sha256: digest.clone(),
            output: temp.path().join("output"),
            trusted_runtime: true,
        };
        let result = export::export_checked(&request, &source, |at| {
            if checkpoint == "race" && at == "before-publish" {
                fs::create_dir(&request.output).unwrap();
                fs::write(request.output.join("sentinel"), b"do not clobber").unwrap();
            }
            if at == checkpoint {
                Err("injected checkpoint failure".into())
            } else {
                Ok(())
            }
        });
        let error = result.unwrap_err();
        if checkpoint == "after-publish" {
            assert!(error.contains("uncertain"), "{error}");
            verify(&request.output, None).unwrap();
        } else if checkpoint == "race" {
            assert_eq!(
                fs::read(request.output.join("sentinel")).unwrap(),
                b"do not clobber"
            );
            assert!(!request.output.join(MANIFEST_NAME).exists());
        } else {
            assert!(!request.output.exists(), "{checkpoint}: {error}");
        }
    }
}
