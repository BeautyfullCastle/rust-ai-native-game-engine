//! Actual import worker -> explicit package load -> saved embedded animation -> Play poses.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use super::*;
use egui::accesskit::Role;
use egui_kittest::{kittest::Queryable, Harness};
use orr_fp::FP;
use std::{
    fs,
    time::{Duration, Instant},
};
fn text(h: &mut Harness<'_, (Panel, Editor)>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(3);
}
fn step(editor: &mut Editor) {
    let target = editor.snapshot().unwrap().predicted().tick() + 1;
    if editor.mode() == Mode::Play {
        let state = editor
            .host_call("sim.step", serde_json::json!({"n":1}))
            .unwrap();
        assert_eq!(state["head_tick"].as_u64(), Some(target));
    } else {
        editor.step(1);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        editor.sync();
        if editor
            .snapshot()
            .is_some_and(|s| s.predicted().tick() == target && s.timeline().is_some())
        {
            return;
        }
        assert!(Instant::now() < deadline, "missing tick {target}");
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn four_authored_variants_actual_import_apply_save_source_hidden_reopen_and_move_idle_poses() {
    for (name, bytes) in crate::mesh_animation_import::cooked_tests::variants() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let scene = root.join("variant.scene.yaml");
        let p = |x, z| [FP::from_int(x), FP::ZERO, FP::from_int(z)];
        let graph = orr_navigation::mesh::StaticMeshGraph::bake(
            &[p(0, 0), p(0, 4), p(4, 0), p(4, 4)],
            &[[0, 1, 2], [2, 1, 3]],
            orr_navigation::AgentProfile::default(),
        )
        .unwrap();
        let mut document = orr_remote::mesh_navigation::authoring::create_mesh_scene(
            &graph,
            orr_remote::mesh_navigation::AgentSpec {
                start: p(0, 0),
                goal: p(4, 0),
                max_vertical: FP::ZERO,
                distance_per_tick: FP::ONE,
            },
        )
        .unwrap();
        fs::write(&scene, document.save_yaml()).unwrap();
        let scene_bytes = fs::read(&scene).unwrap();
        let source = root.join("input.animated.json");
        fs::write(&source, &bytes).unwrap();
        let license = root.join("input-LICENSE.txt");
        let license_bytes = b"Explicit author fixture license";
        fs::write(&license, license_bytes).unwrap();
        let package = root.join("produced-package");
        let mut editor = Editor::open_game(&scene, EditorGame::MeshNavigation).unwrap();
        editor.sync();
        let checksum = editor.checksum();
        let history = editor.history().clone();
        let mut panel = Panel::default();
        panel.sync(&editor);
        let sidecar = crate::mesh_animation_store::sidecar_path(&scene).unwrap();
        assert!(!sidecar.exists());
        let mut h = Harness::builder()
            .with_size([1500.0, 2600.0])
            .build_ui_state(
                |ui, state: &mut (Panel, Editor)| state.0.show(ui, &state.1),
                (panel, editor),
            );
        h.run_steps(3);
        h.get_by_label("Mesh agent animation").click_accesskit();
        h.run_steps(3);
        h.get_by_label("Import animation source").click_accesskit();
        h.run_steps(3);
        for (label, value) in [
            (
                "Animation source (.glb or cooked JSON)",
                source.to_str().unwrap(),
            ),
            ("Animation source license", license.to_str().unwrap()),
            ("New animation package directory", package.to_str().unwrap()),
            ("Animation package name", "mesh-variant"),
            ("Animation package version", "1.0.0"),
        ] {
            text(&mut h, label, value);
        }
        h.get_by_label("I have reviewed rights to import this animation")
            .click_accesskit();
        h.run_steps(3);
        h.get_by_label("Import animation package").click_accesskit();
        h.run_steps(3);
        let deadline = Instant::now() + Duration::from_secs(15);
        while h.state().0.candidate.is_none() || h.state().0.import_busy() {
            h.run_steps(3);
            assert!(
                Instant::now() < deadline,
                "{name}: worker busy={}, candidate={}, panel={:?}",
                h.state().0.import_busy(),
                h.state().0.candidate.is_some(),
                h.state().0.error
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(h.state().0.asset, "model.animated.json");
        assert_eq!(
            fs::read(package.join("model.animated.json")).unwrap(),
            bytes
        );
        assert!(h.state().0.current_snapshot().is_none());
        assert!(!sidecar.exists());
        assert_eq!(h.state().1.checksum(), checksum);
        assert_eq!(h.state().1.history(), &history);
        h.get_by_label("I have reviewed rights to use this animation")
            .click_accesskit();
        h.run_steps(3);
        h.get_by_label("Load mesh animation").click_accesskit();
        h.run_steps(3);
        assert!(
            h.state().0.error.is_none(),
            "{name}: {:?}",
            h.state().0.error
        );
        h.get_by_label("Use idle and moving clips")
            .click_accesskit();
        h.run_steps(3);
        text(&mut h, "Animation idle clip index", "1");
        text(&mut h, "Animation clip index", "0");
        h.get_by_label("Apply mesh animation").click_accesskit();
        h.run_steps(3);
        assert!(
            h.state().0.error.is_none(),
            "{name}: {:?}",
            h.state().0.error
        );
        assert_eq!(
            h.state().0.current_snapshot().unwrap().settings().idle_clip,
            Some(1)
        );
        assert!(!sidecar.exists());
        h.get_by_label("Save mesh animation").click_accesskit();
        h.run_steps(3);
        assert!(
            h.state().0.error.is_none(),
            "{name}: {:?}",
            h.state().0.error
        );
        assert!(sidecar.exists());
        assert_eq!(h.state().1.checksum(), checksum);
        assert_eq!(h.state().1.history(), &history);
        assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
        assert_eq!(fs::read(&source).unwrap(), bytes);
        assert_eq!(fs::read(&license).unwrap(), license_bytes);
        drop(h);
        fs::rename(&source, root.join("hidden-input")).unwrap();
        fs::rename(&license, root.join("hidden-license")).unwrap();
        fs::rename(&package, root.join("hidden-package")).unwrap();
        let mut editor = Editor::open_game(&scene, EditorGame::MeshNavigation).unwrap();
        editor.sync();
        let mut panel = Panel::default();
        panel.sync(&editor);
        assert!(panel.error.is_none(), "{name}: {:?}", panel.error);
        assert!(panel.current_snapshot().is_some());
        let mut baseline =
            orr_sim::Simulation::<orr_games::mesh_navigation_game::MeshNavigation>::from_frame(
                document.frame(),
                orr_remote::mesh_navigation::YARD_TICK_RATE,
                orr_remote::mesh_navigation::MESH_NAVIGATION_BUILD_ID,
            )
            .unwrap();
        let mut moving = false;
        let mut idle = false;
        for _ in 0..7 {
            step(&mut editor);
            let inputs = orr_sim::TickInputs::new(baseline.tick() + 1, 1);
            baseline.step(&inputs);
            assert_eq!(
                editor.checksum(),
                baseline.frame().checksum(),
                "{name}: presentation changed simulation"
            );
            panel.sync(&editor);
            let placement = panel.placement(&editor).unwrap().unwrap();
            assert_eq!(
                placement.model.to_bytes().unwrap(),
                bytes,
                "{name}: embedded model or provenance mutated"
            );
            let view =
                orr_remote::mesh_navigation::mesh_from_view(editor.snapshot().unwrap().predicted())
                    .unwrap();
            for axis in 0..3 {
                assert_eq!(
                    placement.instance[3][axis],
                    view.agent.position[axis].to_f32(),
                    "{name}: model must follow actual agent position"
                );
            }
            let is_moving = view.agent.status == orr_navigation::NavigationStatus::Moving;
            moving |= is_moving;
            idle |= !is_moving;
            let clip = if is_moving { 0 } else { 1 };
            let duration = placement.model.source().clips[clip].duration();
            let seconds =
                (baseline.tick() as f32 / orr_sample::yard3d_game::TICK_RATE as f32) % duration;
            let expected = placement.model.sample_clip(clip as u32, seconds).unwrap();
            assert_eq!(placement.pose, expected, "{name}: wrong moving/idle clip");
        }
        assert!(
            moving && idle,
            "{name}: route must exercise moving and arrival/idle"
        );
        assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    }
}
