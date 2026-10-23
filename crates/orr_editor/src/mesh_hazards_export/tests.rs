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
