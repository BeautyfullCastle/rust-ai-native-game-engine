use super::*;

#[test]
fn cpu_pose_check_counts_visible_static_models_and_rejects_stale_guid() {
    let fixture = Fixture::new();
    fs::rename(&fixture.source, fixture.source.with_extension("hidden")).unwrap();
    let prepared = PreparedProject::open(&fixture.root).unwrap();
    let mut sim = prepared.scene().simulation().unwrap();
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
            static_models: 2,
            animated_models: 0
        }
    );
    assert_eq!(sim.frame().checksum(), before);
    sim.frame_mut().singleton_mut::<RoomRun>().key_collected = 1;
    let counts = room_view::check_model_poses(
        FrameView::of(sim.frame()),
        prepared.scene().index(),
        prepared.models(),
        #[cfg(feature = "room-character")]
        None,
    )
    .unwrap();
    assert_eq!(counts.static_models, 1);
    sim.frame_mut()
        .despawn(actor(prepared.scene().frame(), PLAYER));
    assert!(room_view::check_model_poses(
        FrameView::of(sim.frame()),
        prepared.scene().index(),
        prepared.models(),
        #[cfg(feature = "room-character")]
        None
    )
    .is_err());
}

#[test]
fn runtime_cpu_pose_flag_checks_tick_zero_and_each_step_without_gpu() {
    let fixture = Fixture::new();
    let before = fs::read(fixture.root.join("room.models.json")).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_room_escape"))
        .args(["--project"])
        .arg(&fixture.root)
        .args(["--headless", "--ticks", "3", "--check-model-poses"])
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
        .contains("room model poses: 4 frames checked; final static: 2 animated: 0"));
    assert_eq!(
        before,
        fs::read(fixture.root.join("room.models.json")).unwrap()
    );
}
