#![cfg(feature = "room-animated-models")]
use super::*;
use orr_bridge::FrameView;
use orr_sample::room_view;

#[test]
fn cpu_pose_check_samples_animated_binding_and_rejects_invalid_clip() {
    let fixture = Fixture::new();
    fixture.write_animated_binding(0);
    let prepared = room_project::PreparedProject::open(&fixture.root).unwrap();
    let sim = prepared.scene().simulation().unwrap();
    let before = sim.frame().checksum();
    let counts = room_view::check_model_poses(
        FrameView::of(sim.frame()),
        prepared.scene().index(),
        prepared.models(),
        #[cfg(feature = "room-character")]
        None,
    )
    .unwrap();
    assert_eq!(
        counts,
        room_view::ModelPoseCounts {
            static_models: 0,
            animated_models: 1
        }
    );
    assert_eq!(before, sim.frame().checksum());
    // Admission must reject a changed saved clip; the pose checker never repairs it.
    fixture.write_animated_binding(31);
    let bad = fixture.bytes();
    assert!(room_project::PreparedProject::open(&fixture.root).is_err());
    assert_eq!(bad, fixture.bytes());
    // Retained prepared assets still validate without reopening changed files.
    room_view::check_model_poses(
        FrameView::of(sim.frame()),
        prepared.scene().index(),
        prepared.models(),
        #[cfg(feature = "room-character")]
        None,
    )
    .unwrap();
}

#[cfg(feature = "project-export")]
#[test]
fn exported_launcher_cpu_checks_saved_animation_after_source_project_hidden() {
    use orr_sample::project_export::{self, ExportOptions};
    use sha2::{Digest, Sha256};
    let fixture = Fixture::new();
    fixture.write_animated_binding(0);
    let runtime = PathBuf::from(env!("CARGO_BIN_EXE_room_escape"))
        .canonicalize()
        .unwrap();
    let digest: String = Sha256::digest(fs::read(&runtime).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let bundle = fixture._temp.path().join("deployed game");
    project_export::export_room(&ExportOptions {
        project: fixture.root.clone(),
        runtime,
        runtime_sha256: digest,
        output: bundle.clone(),
        trusted_runtime: true,
        source_revision: None,
    })
    .unwrap();
    fs::rename(&fixture.root, fixture.root.with_extension("hidden")).unwrap();
    let output = std::process::Command::new(bundle.join("run-room-escape"))
        .current_dir(fixture._temp.path())
        .args(["--headless", "--ticks", "4", "--check-model-poses"])
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout)
        .contains("room model poses: 5 frames checked; final static: 0 animated: 1"));
}
