use super::*;
use orr_fp::{FP, fp};
use orr_remote::mesh_navigation::{MISSION_NAME, PIN_NAME};

fn scene_bytes() -> Vec<u8> {
    let (_, pin, mission) =
        crate::mesh_workspace::mission::configured_scene(&Default::default()).unwrap();
    let mut config = hazards::MeshHazards {
        enabled: 1,
        count: 1,
        graph_revision: pin.graph_revision,
        ..Default::default()
    };
    config.spheres[0] = hazards::HazardSphere {
        center: [fp!(4), FP::ZERO, fp!(0.5)],
        radius: fp!(0.5),
    };
    let types = hazards::hazards_types();
    let mut singletons = serde_json::Map::new();
    for (name, bytes) in [
        (PIN_NAME, orr_reflect::bytemuck::bytes_of(&pin)),
        (MISSION_NAME, orr_reflect::bytemuck::bytes_of(&mission)),
        (
            hazards::HAZARDS_NAME,
            orr_reflect::bytemuck::bytes_of(&config),
        ),
    ] {
        singletons.insert(
            name.into(),
            orr_remote::json::value_to_json(&types.get(name).unwrap().read(bytes)),
        );
    }
    serde_json::json!({"schema":"orr.scene/1","singletons":singletons,"entities":{}})
        .to_string()
        .into_bytes()
}
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    scene: PathBuf,
    runtime: PathBuf,
    checksum: u64,
}
impl Fixture {
    fn new(platform: RuntimePlatform) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let scene = root.join("source.scene.yaml");
        let bytes = scene_bytes();
        let checksum = admit_scene(&bytes).unwrap().0;
        fs::write(&scene, bytes).unwrap();
        let runtime = root.join(platform.runtime_filename());
        fs::write(&runtime, runtime::valid_test_runtime(platform)).unwrap();
        Self {
            _temp: temp,
            root,
            scene,
            runtime,
            checksum,
        }
    }
    fn export(&self, destination: &Path, trusted: bool) -> Result<Receipt, String> {
        export_interactive(ExportRequest {
            scene: &self.scene,
            expected_checksum: self.checksum,
            destination,
            runtime: &self.runtime,
            trusted_runtime: trusted,
        })
    }
}
#[test]
fn roundtrip_exact_three_files_for_both_runtime_platforms() {
    for platform in [RuntimePlatform::WindowsX86_64, RuntimePlatform::LinuxX86_64] {
        let fixture = Fixture::new(platform);
        let dest = fixture.root.join("Game with spaces & Unicode 한글");
        let receipt = fixture.export(&dest, true).unwrap();
        let bundle = open_bundle(&dest).unwrap();
        assert_eq!(bundle.platform, platform);
        assert_eq!(bundle.checksum, fixture.checksum);
        assert_eq!(bundle.runtime_sha256, receipt.runtime_sha256);
        assert_eq!(
            fs::read(&bundle.scene).unwrap(),
            fs::read(&fixture.scene).unwrap()
        );
        assert_eq!(
            fs::read(&bundle.runtime).unwrap(),
            fs::read(&fixture.runtime).unwrap()
        );
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 3);
        assert!(fixture.export(&dest, true).is_err());
        assert_eq!(open_bundle(&dest).unwrap(), bundle);
    }
}
#[test]
fn trust_and_changed_checksum_refuse_before_writes() {
    let fixture = Fixture::new(RuntimePlatform::LinuxX86_64);
    let dest = fixture.root.join("out");
    assert!(fixture.export(&dest, false).is_err());
    assert!(!dest.exists());
    assert!(
        export_interactive(ExportRequest {
            scene: &fixture.scene,
            expected_checksum: fixture.checksum ^ 1,
            destination: &dest,
            runtime: &fixture.runtime,
            trusted_runtime: true
        })
        .is_err()
    );
    assert!(!dest.exists());
    assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 2);
}
#[test]
fn all_presentation_and_unknown_scene_attachments_fail_closed() {
    let fixture = Fixture::new(RuntimePlatform::LinuxX86_64);
    for suffix in [
        "mesh-art.json",
        "mesh-props.json",
        "mesh-camera.json",
        "mesh-animation.json",
        "mesh-lighting.json",
        "mesh-hud.json",
        "mesh-unknown.json",
    ] {
        let attachment = fixture.root.join(format!("source.scene.yaml.{suffix}"));
        fs::write(&attachment, b"{}").unwrap();
        assert!(
            fixture.export(&fixture.root.join("out"), true).is_err(),
            "{suffix}"
        );
        fs::remove_file(attachment).unwrap();
    }
    assert!(!fixture.root.join("out").exists());
}
fn mutate_manifest(root: &Path, mutate: impl FnOnce(&mut serde_json::Value)) {
    let path = root.join(MANIFEST_FILENAME);
    let mut value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    mutate(&mut value);
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}
#[test]
fn strict_manifest_profiles_schema_names_digest_and_platform() {
    for case in 0..11 {
        let fixture = Fixture::new(RuntimePlatform::LinuxX86_64);
        let dest = fixture.root.join("out");
        fixture.export(&dest, true).unwrap();
        mutate_manifest(&dest, |value| match case {
            0 => value["schema"] = 2.into(),
            1 => value["game_profile"] = "mesh-mission-v1".into(),
            2 => value["input_profile"] = "unknown".into(),
            3 => value["platform"] = "windows-x86-64".into(),
            4 => value["scene"]["name"] = "../source.scene.yaml".into(),
            5 => value["runtime"]["sha256"] = "0".repeat(64).into(),
            6 => value["checksum"] = (fixture.checksum ^ 1).into(),
            7 => value["unexpected"] = true.into(),
            8 => {
                value.as_object_mut().unwrap().remove("game_profile");
            }
            9 => value["scene"]["bytes"] = 0.into(),
            _ => {
                value["graph_revision"][0] =
                    (value["graph_revision"][0].as_u64().unwrap() ^ 1).into()
            }
        });
        assert!(open_bundle(&dest).is_err(), "case {case}");
    }
}
#[test]
fn extra_missing_tampered_and_oversized_files_are_rejected() {
    for case in 0..5 {
        let fixture = Fixture::new(RuntimePlatform::LinuxX86_64);
        let dest = fixture.root.join("out");
        fixture.export(&dest, true).unwrap();
        match case {
            0 => fs::write(dest.join("unexpected"), b"extra").unwrap(),
            1 => fs::remove_file(dest.join(SCENE_FILENAME)).unwrap(),
            2 => fs::write(dest.join(SCENE_FILENAME), b"{}").unwrap(),
            3 => fs::write(dest.join("orr_mesh_hazards_player"), b"bad runtime").unwrap(),
            _ => fs::OpenOptions::new()
                .write(true)
                .open(dest.join(MANIFEST_FILENAME))
                .unwrap()
                .set_len(MAX_MANIFEST_BYTES + 1)
                .unwrap(),
        }
        assert!(open_bundle(&dest).is_err(), "case {case}");
    }
}
#[test]
fn invalid_and_legacy_scenes_cannot_be_admitted() {
    assert!(admit_scene(b"schema: orr.scene/1\nsingletons: {}\nentities: {}\n").is_err());
    assert!(admit_scene(b"\xff").is_err());
    let mut value: serde_json::Value = serde_json::from_slice(&scene_bytes()).unwrap();
    value["singletons"]
        .as_object_mut()
        .unwrap()
        .remove(hazards::HAZARDS_NAME);
    assert!(admit_scene(&serde_json::to_vec(&value).unwrap()).is_err());
}
#[test]
fn normalized_local_paths_and_existing_destination_are_required() {
    let fixture = Fixture::new(RuntimePlatform::LinuxX86_64);
    assert!(fixture.export(Path::new("relative"), true).is_err());
    for name in ["CON", "bad.", "bad ", "bad:leaf"] {
        assert!(fixture.export(&fixture.root.join(name), true).is_err());
    }
    let empty = fixture.root.join("empty");
    fs::create_dir(&empty).unwrap();
    assert!(fixture.export(&empty, true).is_err());
    assert_eq!(fs::read_dir(&empty).unwrap().count(), 0);
    let dest = fixture.root.join("exists");
    fs::create_dir(&dest).unwrap();
    fs::write(dest.join("keep"), b"keep").unwrap();
    assert!(fixture.export(&dest, true).is_err());
    assert_eq!(fs::read(dest.join("keep")).unwrap(), b"keep");
}
#[cfg(unix)]
#[test]
fn symlink_files_parent_and_bundle_are_rejected() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new(RuntimePlatform::LinuxX86_64);
    let alias = fixture.root.join("alias.scene.yaml");
    symlink(&fixture.scene, &alias).unwrap();
    assert!(io::read_bounded(&alias, MAX_SCENE_BYTES, "alias").is_err());
    let parent_alias = fixture.root.join("parent-alias");
    symlink(&fixture.root, &parent_alias).unwrap();
    assert!(fixture.export(&parent_alias.join("out"), true).is_err());
    let dest = fixture.root.join("out");
    fixture.export(&dest, true).unwrap();
    let bundle_alias = fixture.root.join("bundle-alias");
    symlink(&dest, &bundle_alias).unwrap();
    assert!(open_bundle(&bundle_alias).is_err());
    fs::remove_file(dest.join(SCENE_FILENAME)).unwrap();
    symlink(&fixture.scene, dest.join(SCENE_FILENAME)).unwrap();
    assert!(open_bundle(&dest).is_err());
}

#[test]
fn malformed_runtime_never_publishes_or_leaves_staging() {
    let fixture = Fixture::new(RuntimePlatform::LinuxX86_64);
    fs::write(
        &fixture.runtime,
        b"not an executable ORR_MESH_HAZARDS_PLAYER_V1",
    )
    .unwrap();
    assert!(fixture.export(&fixture.root.join("out"), true).is_err());
    assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 2);
}
#[test]
fn manifest_arrays_duplicates_and_nested_unknowns_are_rejected() {
    for case in 0..4 {
        let fixture = Fixture::new(RuntimePlatform::LinuxX86_64);
        let dest = fixture.root.join("out");
        fixture.export(&dest, true).unwrap();
        let path = dest.join(MANIFEST_FILENAME);
        let bytes = fs::read(&path).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let text = match case {
            0 => serde_json::to_string(&serde_json::json!([
                1,
                "mesh-hazards-v1",
                "mesh-click-to-move-v1",
                value["platform"],
                value["scene"],
                value["runtime"],
                value["checksum"],
                value["graph_revision"]
            ]))
            .unwrap(),
            1 => String::from_utf8(bytes)
                .unwrap()
                .replacen('{', "{\"schema\":1,", 1),
            2 => {
                let mut value = value;
                value["scene"]["extra"] = true.into();
                value.to_string()
            }
            _ => {
                let mut value = value;
                value["scene"] = serde_json::json!([
                    value["scene"]["name"],
                    value["scene"]["bytes"],
                    value["scene"]["sha256"]
                ]);
                value.to_string()
            }
        };
        fs::write(path, text).unwrap();
        assert!(open_bundle(&dest).is_err(), "case {case}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn prechecked_regular_file_replaced_by_fifo_is_rejected_without_blocking() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp
        .path()
        .canonicalize()
        .unwrap()
        .join("replace.scene.yaml");
    fs::write(&path, b"regular before check").unwrap();
    let checked = io::checked_existing_file(&path, MAX_SCENE_BYTES, "precheck").unwrap();
    let observed = fs::symlink_metadata(&checked).unwrap();
    assert!(observed.is_file());
    fs::remove_file(&checked).unwrap();
    // Test fixture only: no writer is opened, so a blocking read-open would hang.
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&checked)
            .status()
            .unwrap()
            .success()
    );
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        sender
            .send(io::read_observed(
                &checked,
                observed,
                MAX_SCENE_BYTES,
                "replacement",
            ))
            .unwrap();
    });
    let error = receiver
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("FIFO replacement blocked checked-open")
        .unwrap_err();
    assert!(error.contains("changed before reading"), "{error}");
    worker.join().unwrap();
}

#[test]
fn published_stage_releases_old_path_before_bundle_revalidation() {
    let fixture = Fixture::new(RuntimePlatform::LinuxX86_64);
    let original = fixture.root.join("original");
    fixture.export(&original, true).unwrap();
    let stage = tempfile::Builder::new()
        .prefix("private-stage-")
        .tempdir_in(&fixture.root)
        .unwrap();
    let former = stage.path().to_path_buf();
    for entry in fs::read_dir(&original).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), stage.path().join(entry.file_name())).unwrap();
    }
    let published = fixture.root.join("published");
    assert_eq!(io::publish_stage(stage, &published).unwrap(), former);
    fs::create_dir(&former).unwrap();
    fs::write(former.join("new-owner"), b"keep").unwrap();
    open_bundle(&published).unwrap();
    assert_eq!(fs::read(former.join("new-owner")).unwrap(), b"keep");
    assert!(published.is_dir());
    fs::write(published.join(MANIFEST_FILENAME), b"corrupt manifest").unwrap();
    assert!(open_bundle(&published).is_err());
    assert_eq!(fs::read(former.join("new-owner")).unwrap(), b"keep");
    assert_eq!(
        fs::read(published.join(MANIFEST_FILENAME)).unwrap(),
        b"corrupt manifest"
    );
}

#[test]
fn failed_publication_cleans_owned_stage_and_preserves_existing_destination() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let destination = root.join("existing");
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("keep"), b"existing destination").unwrap();
    let stage = tempfile::Builder::new()
        .prefix("private-stage-")
        .tempdir_in(&root)
        .unwrap();
    let stage_path = stage.path().to_path_buf();
    fs::write(stage_path.join("owned"), b"private staged data").unwrap();
    assert!(io::publish_stage(stage, &destination).is_err());
    assert!(!stage_path.exists());
    assert_eq!(
        fs::read(destination.join("keep")).unwrap(),
        b"existing destination"
    );
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);
}

impl Fixture {
    fn camera_path(&self) -> PathBuf {
        self.scene
            .with_file_name("source.scene.yaml.mesh-camera.json")
    }
    fn add_camera(&self, platform: RuntimePlatform) -> Vec<u8> {
        let revision = admit_scene(&fs::read(&self.scene).unwrap()).unwrap().1;
        let document =
            crate::mesh_camera::Document::from_orbit(orr_render::camera3d::OrbitCamera {
                target: [1.0, 0.0, 0.5],
                yaw: 0.9,
                pitch: 0.7,
                distance: 14.0,
                fov_y_degrees: 61.0,
            })
            .unwrap()
            .with_projection(
                crate::mesh_camera::Projection::Orthographic { half_height: 6.0 },
                0.2,
                200.0,
            )
            .unwrap()
            .with_follow_offset(Some([0.0, 1.0, 0.0]))
            .unwrap();
        let bytes = document.encode("source.scene.yaml", revision).unwrap();
        fs::write(self.camera_path(), &bytes).unwrap();
        fs::write(&self.runtime, runtime::valid_test_camera_runtime(platform)).unwrap();
        bytes
    }
    fn request<'a>(&'a self, destination: &'a Path) -> ExportRequest<'a> {
        ExportRequest {
            scene: &self.scene,
            expected_checksum: self.checksum,
            destination,
            runtime: &self.runtime,
            trusted_runtime: true,
        }
    }
}

#[test]
fn camera_v2_rebinds_only_exported_copy_and_relocates_exact_four_files() {
    for platform in [RuntimePlatform::LinuxX86_64, RuntimePlatform::WindowsX86_64] {
        let fixture = Fixture::new(platform);
        let source_bytes = fixture.add_camera(platform);
        let destination = fixture.root.join("camera-game");
        let receipt = fixture.export(&destination, true).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&receipt.manifest).unwrap()).unwrap();
        assert_eq!(manifest["schema"], 2);
        assert_eq!(manifest["camera"]["name"], CAMERA_FILENAME);
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 4);
        assert_eq!(fs::read(fixture.camera_path()).unwrap(), source_bytes);
        let camera_bytes = fs::read(destination.join(CAMERA_FILENAME)).unwrap();
        assert_ne!(camera_bytes, source_bytes);
        assert_eq!(
            receipt.camera_sha256.as_deref(),
            Some(digest(&camera_bytes).as_str())
        );
        let source = crate::mesh_camera::Document::decode(
            &source_bytes,
            "source.scene.yaml",
            receipt.graph_revision,
        )
        .unwrap();
        assert_eq!(
            crate::mesh_camera::Document::decode(
                &camera_bytes,
                SCENE_FILENAME,
                receipt.graph_revision
            )
            .unwrap(),
            source
        );
        fs::rename(&fixture.scene, fixture.root.join("hidden-scene")).unwrap();
        fs::rename(fixture.camera_path(), fixture.root.join("hidden-camera")).unwrap();
        let relocated = fixture.root.join("relocated");
        fs::rename(&destination, &relocated).unwrap();
        let prepared = open_bundle(&relocated).unwrap();
        assert_eq!(prepared.checksum, fixture.checksum);
        assert_eq!(prepared.camera_sha256, receipt.camera_sha256);
    }
}

#[test]
fn camera_bundle_requires_both_capability_and_explicit_runtime_trust() {
    for platform in [RuntimePlatform::LinuxX86_64, RuntimePlatform::WindowsX86_64] {
        let fixture = Fixture::new(platform);
        fixture.add_camera(platform);
        let destination = fixture.root.join("out");
        assert!(fixture.export(&destination, false).is_err());
        fs::write(&fixture.runtime, runtime::valid_test_runtime(platform)).unwrap();
        assert!(fixture.export(&destination, true).is_err());
        assert!(!destination.exists());
        fs::remove_file(fixture.camera_path()).unwrap();
        assert!(
            fixture
                .export(&destination, true)
                .unwrap()
                .camera_sha256
                .is_none()
        );
    }
}

#[test]
fn camera_manifest_shape_name_hash_schema_and_binding_fail_closed() {
    for case in 0..12 {
        let fixture = Fixture::new(RuntimePlatform::LinuxX86_64);
        fixture.add_camera(RuntimePlatform::LinuxX86_64);
        let destination = fixture.root.join("out");
        fixture.export(&destination, true).unwrap();
        mutate_manifest(&destination, |value| match case {
            0 => value["schema"] = 1.into(),
            1 => value["camera"] = serde_json::Value::Null,
            2 => {
                value.as_object_mut().unwrap().remove("camera");
            }
            3 => value["camera"]["name"] = "../escape.json".into(),
            4 => value["camera"]["sha256"] = "0".repeat(64).into(),
            5 => value["camera"]["bytes"] = (MAX_CAMERA_BYTES + 1).into(),
            6 => value["camera"]["unknown"] = true.into(),
            7 => value["camera"] = serde_json::json!([CAMERA_FILENAME, 1, "0".repeat(64)]),
            8 => {
                fs::remove_file(destination.join(CAMERA_FILENAME)).unwrap();
            }
            9 => {
                fs::write(destination.join(CAMERA_FILENAME), b"corrupt").unwrap();
            }
            10 => {
                let path = destination.join(CAMERA_FILENAME);
                let mut camera: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                camera["scene_filename"] = "source.scene.yaml".into();
                let bytes = serde_json::to_vec(&camera).unwrap();
                fs::write(path, &bytes).unwrap();
                value["camera"] = serde_json::to_value(record(CAMERA_FILENAME, &bytes)).unwrap();
            }
            _ => {
                let bytes = runtime::valid_test_runtime(RuntimePlatform::LinuxX86_64);
                fs::write(
                    destination.join(RuntimePlatform::LinuxX86_64.runtime_filename()),
                    &bytes,
                )
                .unwrap();
                value["runtime"] = serde_json::to_value(record(
                    RuntimePlatform::LinuxX86_64.runtime_filename(),
                    &bytes,
                ))
                .unwrap();
            }
        });
        assert!(open_bundle(&destination).is_err(), "case {case}");
    }
    let fixture = Fixture::new(RuntimePlatform::LinuxX86_64);
    let destination = fixture.root.join("scene-only");
    fixture.export(&destination, true).unwrap();
    mutate_manifest(&destination, |value| {
        value["camera"] = serde_json::Value::Null
    });
    assert!(
        open_bundle(&destination).is_err(),
        "v1 must not accept a camera field even when null"
    );
}

#[test]
fn camera_source_corrupt_wrong_scene_graph_oversized_and_case_alias_rejected() {
    for case in 0..6 {
        let fixture = Fixture::new(RuntimePlatform::LinuxX86_64);
        let bytes = fixture.add_camera(RuntimePlatform::LinuxX86_64);
        let path = fixture.camera_path();
        match case {
            0 => fs::write(&path, b"{}").unwrap(),
            1 | 2 => {
                let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                if case == 1 {
                    value["scene_filename"] = "other.scene.yaml".into();
                } else {
                    value["graph_revision"][0] =
                        (value["graph_revision"][0].as_u64().unwrap() ^ 1).into();
                }
                fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            }
            3 => fs::write(&path, vec![b' '; MAX_CAMERA_BYTES as usize + 1]).unwrap(),
            4 => {
                fs::rename(
                    &path,
                    fixture.root.join("source.scene.yaml.MESH-CAMERA.JSON"),
                )
                .unwrap();
            }
            _ => {
                fs::write(fixture.root.join("source.scene.yaml.mesh-art.json"), b"{}").unwrap();
            }
        }
        let destination = fixture.root.join("out");
        assert!(fixture.export(&destination, true).is_err(), "case {case}");
        assert!(!destination.exists());
    }
}

#[test]
fn camera_presence_and_original_bytes_bound_at_admission_and_prepublication() {
    for before_publication in [false, true] {
        for case in 0..3 {
            let fixture = Fixture::new(RuntimePlatform::LinuxX86_64);
            let bytes = fixture.add_camera(RuntimePlatform::LinuxX86_64);
            if case == 0 {
                fs::remove_file(fixture.camera_path()).unwrap();
            }
            let snapshot = capture_camera_source(&fixture.scene).unwrap();
            let change = || match case {
                0 => fs::write(fixture.camera_path(), &bytes).unwrap(),
                1 => fs::remove_file(fixture.camera_path()).unwrap(),
                _ => {
                    // Semantically identical JSON with distinct source bytes must be stale.
                    let mut replacement = bytes.clone();
                    assert_eq!(replacement.last(), Some(&b'\n'));
                    *replacement.last_mut().unwrap() = b' ';
                    assert_eq!(replacement.len(), bytes.len());
                    fs::write(fixture.camera_path(), replacement).unwrap();
                }
            };
            let destination = fixture.root.join("out");
            let result = if before_publication {
                export_with_recheck(fixture.request(&destination), &snapshot, change)
            } else {
                change();
                export_interactive_bound(fixture.request(&destination), &snapshot)
            };
            assert!(
                result.is_err(),
                "before_publication={before_publication} case={case}"
            );
            assert!(!destination.exists());
            assert!(!fs::read_dir(&fixture.root).unwrap().any(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".orr-hazards-stage-")
            }));
        }
    }
}

#[cfg(unix)]
#[test]
fn camera_source_and_bundle_symlinks_are_rejected() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new(RuntimePlatform::LinuxX86_64);
    let bytes = fixture.add_camera(RuntimePlatform::LinuxX86_64);
    let real = fixture.root.join("camera-real.json");
    fs::write(&real, bytes).unwrap();
    fs::remove_file(fixture.camera_path()).unwrap();
    symlink(&real, fixture.camera_path()).unwrap();
    assert!(capture_camera_source(&fixture.scene).is_err());
    fs::remove_file(fixture.camera_path()).unwrap();
    fixture.add_camera(RuntimePlatform::LinuxX86_64);
    let destination = fixture.root.join("out");
    fixture.export(&destination, true).unwrap();
    fs::remove_file(destination.join(CAMERA_FILENAME)).unwrap();
    symlink(real, destination.join(CAMERA_FILENAME)).unwrap();
    assert!(open_bundle(&destination).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn camera_fifo_source_rejects_without_blocking() {
    let fixture = Fixture::new(RuntimePlatform::LinuxX86_64);
    assert!(
        std::process::Command::new("mkfifo")
            .arg(fixture.camera_path())
            .status()
            .unwrap()
            .success()
    );
    assert!(capture_camera_source(&fixture.scene).is_err());
    assert!(fixture.export(&fixture.root.join("out"), true).is_err());
}
