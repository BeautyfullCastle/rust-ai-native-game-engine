use super::*;
use serde_json::json;
fn fixture() -> (tempfile::TempDir, PathBuf, Editor, Panel) {
    let (temp, root, mut editor) = Editor::room_import_fixture();
    #[cfg(feature = "room-input-map-play")]
    {
        let path = root.join("orr.project.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        manifest["entry"]
            .as_object_mut()
            .unwrap()
            .remove("controls");
        std::fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    }

    editor.host_call("sim.start", json!({})).unwrap();
    for (ticks, x, interact) in [(1, 0, true), (60, 1, false), (1, 0, true)] {
        let input = orr_sample::room_game::RoomInput {
            move_x: x,
            buttons: u32::from(interact),
            ..Default::default()
        };
        let encoded: String = bytemuck::bytes_of(&input)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        editor
            .host_call("sim.input", json!({"player":0,"input":encoded}))
            .unwrap();
        editor.host_call("sim.step", json!({"n":ticks})).unwrap();
    }
    editor.sync();
    assert_eq!(
        editor
            .snapshot()
            .unwrap()
            .predicted()
            .singleton::<orr_sample::room_game::RoomRun>()
            .won,
        1
    );
    editor.stop().unwrap();
    editor.sync();
    (
        temp,
        root,
        editor,
        Panel {
            open: true,
            ..Default::default()
        },
    )
}
#[test]
fn actual_stopped_play_prepares_copies_and_replays_through_cli_when_supplied() {
    let (_temp, root, mut editor, mut panel) = fixture();
    let before = editor.last_stopped().unwrap().clone();
    let sum = editor.checksum();
    let history = editor.history().entries.len();
    panel.prepare_recorded_route(&mut editor, None).unwrap();
    let script = panel.copy_recorded_route(&mut editor, None).unwrap();
    let v: serde_json::Value = serde_json::from_str(&script).unwrap();
    assert_eq!(v["steps"][2]["expect_won"], true);
    assert!(panel.copy_recorded_route(&mut editor, None).is_err());
    assert_eq!(editor.last_stopped(), Some(&before));
    assert_eq!(editor.checksum(), sum);
    assert_eq!(editor.history().entries.len(), history);
    // Explicit environment route is exercised separately with the current CLI;
    // no cached executable is silently considered production acceptance.
    if let Some(binary) = std::env::var_os("ORR_RECORDED_ROUTE_CLI") {
        let path = root.parent().unwrap().join("recorded-route.json");
        std::fs::write(&path, &script).unwrap();
        let output = std::process::Command::new(binary)
            .args(["--project"])
            .arg(&root)
            .arg("--script")
            .arg(&path)
            .arg("--json")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["status"], "passed");
        println!("RECORDED_ROUTE_CLI_PASSED");
        assert_eq!(
            report["steps"].as_array().unwrap().last().unwrap()["observed"]["won"],
            true
        );
    }
}
#[test]
fn close_blocker_play_restart_and_changed_file_invalidate_copy() {
    let (_temp, root, mut editor, mut panel) = fixture();
    for case in 0..3 {
        panel.prepare_recorded_route(&mut editor, None).unwrap();
        if case == 0 {
            panel.open = false;
        } else if case == 2 {
            editor.host_call("sim.start", json!({})).unwrap();
            editor.sync();
        }
        panel.sync_recorded_route(&editor, if case == 1 { Some("busy") } else { None });
        assert!(panel.recorded_route.prepared.is_none());
        panel.open = true;
        if case == 2 {
            editor.stop();
            editor.sync();
        }
    }
    // A fresh recording restores eligibility, but a changed saved source does not.
    let (_temp2, _root2, mut other, mut other_panel) = fixture();
    other_panel
        .prepare_recorded_route(&mut other, None)
        .unwrap();
    let path = other.path().unwrap();
    let source = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("# changed\n{source}")).unwrap();
    assert!(other_panel.copy_recorded_route(&mut other, None).is_err());
    assert!(root.exists());
}

#[test]
fn actual_widgets_prepare_cancel_copy_emit_only_explicit_clipboard_output() {
    use egui::accesskit::Role;
    use egui_kittest::{
        kittest::{NodeT, Queryable},
        Harness,
    };
    let (_temp, _root, editor, panel) = fixture();
    let mut h = Harness::builder().with_size([900.0, 600.0]).build_ui_state(
        |ui, state: &mut (Editor, Panel)| {
            state.0.sync();
            state.1.show_recorded_route(ui, &mut state.0, None);
        },
        (editor, panel),
    );
    h.run_steps(3);
    h.get_by_label("Recorded Room regression route").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "Prepare last Room route")
        .click();
    h.run_steps(3);
    assert!(h.state().1.recorded_route.prepared.is_some());
    assert!(h.output().platform_output.commands.is_empty());
    h.get_by_role_and_label(Role::Button, "Cancel Room route")
        .click();
    h.run_steps(3);
    assert!(h.state().1.recorded_route.prepared.is_none());
    h.get_by_role_and_label(Role::Button, "Prepare last Room route")
        .click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "Copy Room route JSON")
        .click();
    let mut copied = Vec::new();
    for _ in 0..4 {
        h.run_steps(1);
        for command in &h.output().platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                copied.push(text.clone());
            }
        }
    }
    assert_eq!(copied.len(), 1);
    let v: serde_json::Value = serde_json::from_str(&copied[0]).unwrap();
    assert_eq!(v["steps"][2]["expect_won"], true);
    assert!(h
        .get_by_role_and_label(Role::Button, "Copy Room route JSON")
        .accesskit_node()
        .is_disabled());
}
#[test]
fn same_path_reload_different_scene_and_corrupt_recording_cannot_reuse_prepared_route() {
    let (_temp, _root, mut editor, mut panel) = fixture();
    panel.prepare_recorded_route(&mut editor, None).unwrap();
    let path = editor.path().unwrap();
    editor
        .host_call(
            "scene.load",
            json!({"path":path,"text":std::fs::read_to_string(&path).unwrap()}),
        )
        .unwrap();
    editor.sync();
    assert!(panel.copy_recorded_route(&mut editor, None).is_err());
    panel.prepare_recorded_route(&mut editor, None).unwrap();
    // Test corruption stays in the prepared copy: the authoritative stopped
    // recording must still match byte-for-byte before Copy.
    panel
        .recorded_route
        .prepared
        .as_mut()
        .unwrap()
        .recording
        .replay[0] ^= 1;
    assert!(panel.copy_recorded_route(&mut editor, None).is_err());
    editor
        .apply_room_layout([
            [orr_fp::FP::ONE, orr_fp::FP::ZERO],
            [orr_fp::FP::from_int(2), orr_fp::FP::ZERO],
            [orr_fp::FP::from_int(3), orr_fp::FP::ZERO],
        ])
        .unwrap();
    editor.sync();
    assert!(panel.prepare_recorded_route(&mut editor, None).is_err());
}

#[cfg(feature = "room-input-map-play")]
#[test]
fn authored_controls_are_refused_at_prepare_and_after_prepare() {
    let (_temp, root, mut editor, mut panel) = fixture();
    panel.prepare_recorded_route(&mut editor, None).unwrap();
    let path = root.join("orr.project.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest["entry"]["controls"] = json!("room.controls.json");
    std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(panel
        .copy_recorded_route(&mut editor, None)
        .unwrap_err()
        .contains("authored controls"));
    assert!(panel
        .prepare_recorded_route(&mut editor, None)
        .unwrap_err()
        .contains("authored controls"));
}
