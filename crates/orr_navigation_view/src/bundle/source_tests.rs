use super::*;
use crate::bundle::export::{export_for_source, ExportRequest};
use std::fs;
fn fixture(root: &Path) -> PathBuf {
    fs::create_dir_all(root.join("navigation")).unwrap();
    let scene = root.join("point.yaml");
    fs::write(
        &scene,
        include_bytes!("../../../../scenes/navigation_point.scene.yaml"),
    )
    .unwrap();
    fs::write(
        root.join("navigation/point_demo.orrt"),
        include_bytes!("../../../../scenes/navigation/point_demo.orrt"),
    )
    .unwrap();
    scene
}
#[test]
fn captured_source_is_send_sync_and_matches_actual_initial_frame() {
    fn require<T: Send + Sync>() {}
    require::<SavedSource>();
    let root = tempfile::tempdir().unwrap();
    let scene = fixture(root.path());
    let source = SavedSource::capture(&scene).unwrap();
    let player = AuthoredPlayer::open(&scene).unwrap();
    assert_eq!(source.scene_path(), scene);
    assert_eq!(source.scene_bytes(), fs::read(scene).unwrap());
    assert_eq!(source.initial_frame_bytes(), player.frame().to_bytes());
    assert_eq!(source.initial_checksum(), player.checksum());
    source.recheck().unwrap();
}
#[test]
fn source_changes_before_worker_refused_without_inspecting_runtime_or_creating_output() {
    for change in ["scene", "terrain", "same-bytes-replacement"] {
        let root = tempfile::tempdir().unwrap();
        let scene = fixture(root.path());
        let source = SavedSource::capture(&scene).unwrap();
        match change {
            "scene" => {
                let mut bytes = fs::read(&scene).unwrap();
                bytes.extend_from_slice(b"\n# changed\n");
                fs::write(&scene, bytes).unwrap();
            }
            "terrain" => {
                fs::write(root.path().join("navigation/point_demo.orrt"), b"changed").unwrap();
            }
            _ => {
                fs::rename(&scene, root.path().join("old.yaml")).unwrap();
                fs::write(&scene, source.scene_bytes()).unwrap();
            }
        }
        let request = ExportRequest {
            runtime: root.path().join("missing-runtime"),
            runtime_sha256: "0".repeat(64),
            output: root.path().join("output"),
            trusted_runtime: true,
            source_revision: None,
        };
        assert!(source.recheck().is_err());
        assert!(export_for_source(&request, &source).is_err());
        assert!(!request.output.exists());
        assert!(!fs::read_dir(root.path()).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".orr-navigation-")));
    }
}
#[test]
fn capture_refuses_missing_oversize_and_symlink_sources() {
    let root = tempfile::tempdir().unwrap();
    assert!(SavedSource::capture(&root.path().join("missing")).is_err());
    let scene = fixture(root.path());
    let link = root.path().join("link.yaml");
    std::os::unix::fs::symlink(&scene, &link).unwrap();
    assert!(SavedSource::capture(&link).is_err());
    fs::write(&scene, vec![b' '; super::super::MAX_SCENE + 1]).unwrap();
    assert!(SavedSource::capture(&scene).is_err());
}
