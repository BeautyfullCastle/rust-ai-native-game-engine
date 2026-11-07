//! CPU widgets and production render placements; no GPU or physical input claim.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use super::*;
use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable};
use orr_fp::FP;
use std::{
    fs,
    time::{Duration, Instant},
};
type State = (Panel, crate::mesh_props_panel::Panel, Editor);
fn text(h: &mut Harness<'_, State>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(3);
}
fn click(h: &mut Harness<'_, State>, label: &str) {
    h.run_steps(2);
    h.get_by_label(label).click_accesskit();
    h.run_steps(3);
}
#[test]
fn authored_static_outputs_actual_import_agent_and_prop_save_source_hidden_reopen() {
    for (name, bytes) in crate::mesh_art_import::cooked_tests::variants() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let scene = root.join("static.scene.yaml");
        let p = |x, z| [FP::from_int(x), FP::ZERO, FP::from_int(z)];
        let graph = orr_navigation::mesh::StaticMeshGraph::bake(
            &[p(0, 0), p(0, 4), p(4, 0), p(4, 4)],
            &[[0, 1, 2], [2, 1, 3]],
            orr_navigation::AgentProfile::default(),
        )
        .unwrap();
        let mut doc = orr_remote::mesh_navigation::authoring::create_mesh_scene(
            &graph,
            orr_remote::mesh_navigation::AgentSpec {
                start: p(0, 0),
                goal: p(4, 0),
                max_vertical: FP::ZERO,
                distance_per_tick: FP::ONE,
            },
        )
        .unwrap();
        fs::write(&scene, doc.save_yaml()).unwrap();
        let scene_bytes = fs::read(&scene).unwrap();
        let source = root.join("input.model.json");
        let license = root.join("input-LICENSE.txt");
        let package = root.join("package");
        fs::write(&source, &bytes).unwrap();
        fs::write(&license, b"Explicit author fixture license").unwrap();
        let mut editor = Editor::open_game(&scene, EditorGame::MeshNavigation).unwrap();
        editor.sync();
        let checksum = editor.checksum();
        let history = editor.history().clone();
        let mut art = Panel::default();
        art.sync(&editor);
        let mut props = crate::mesh_props_panel::Panel::default();
        props.sync(&editor);
        let art_sidecar = crate::mesh_art::sidecar_path(&scene).unwrap();
        let props_sidecar = crate::mesh_props::sidecar_path(&scene).unwrap();
        let mut h = Harness::builder().with_size([1700., 5000.]).build_ui_state(
            |ui, state: &mut State| {
                state.0.show(ui, &state.2);
                state.1.show(ui, &state.2, false);
            },
            (art, props, editor),
        );
        h.run_steps(3);
        click(&mut h, "Import agent art source");
        for (label, value) in [
            (
                "Agent art source (.glb or cooked model)",
                source.to_str().unwrap(),
            ),
            ("Agent art source license", license.to_str().unwrap()),
            ("New agent package directory", package.to_str().unwrap()),
            ("Agent package name", "authored-static"),
            ("Agent package version", "1.0.0"),
        ] {
            text(&mut h, label, value);
        }
        click(&mut h, "I have reviewed rights to import this agent art");
        click(&mut h, "Import agent art package");
        let deadline = Instant::now() + Duration::from_secs(15);
        while h.state().0.candidate.is_none() || h.state().0.import_busy() {
            h.run_steps(3);
            assert!(Instant::now() < deadline, "{name}: {:?}", h.state().0.error);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(h.state().0.asset, "model.model.json");
        assert_eq!(fs::read(package.join("model.model.json")).unwrap(), bytes);
        assert!(h.state().0.current_snapshot().is_none());
        assert!(!art_sidecar.exists());
        assert!(!props_sidecar.exists());
        click(&mut h, "Mesh agent art");
        click(&mut h, "I have reviewed rights to use this agent art");
        click(&mut h, "Load agent art");
        text(&mut h, "Agent art offset X", "0.5");
        text(&mut h, "Agent art scale", "1.25");
        click(&mut h, "Apply agent art");
        assert!(
            h.state().0.error.is_none(),
            "{name}: {:?}",
            h.state().0.error
        );
        let agent_before = h.state().0.placement(&h.state().2).unwrap();
        assert_eq!(agent_before.model.to_bytes().unwrap(), bytes);
        assert_eq!(agent_before.instance.translation, [0.5, 0., 0.]);
        assert!(!art_sidecar.exists());
        click(&mut h, "Save agent art");
        assert!(art_sidecar.exists(), "{name}: {:?}", h.state().0.error);
        click(&mut h, "Mesh props");
        for (label, value) in [
            ("Prop package source directory", package.to_str().unwrap()),
            ("Prop model asset", "model.model.json"),
            ("Prop license asset", "LICENSE.txt"),
        ] {
            text(&mut h, label, value);
        }
        click(&mut h, "I have reviewed rights to use these props");
        click(&mut h, "Load prop candidate");
        for (label, value) in [
            ("Prop position X", "2"),
            ("Prop position Y", "0.75"),
            ("Prop position Z", "1"),
            ("Prop scale", "0.5"),
        ] {
            text(&mut h, label, value);
        }
        click(&mut h, "Add prop");
        let placed = h.state().1.placements(&h.state().2);
        assert_eq!(placed.len(), 1, "{name}: prop was not added");
        assert_eq!(placed[0].model.to_bytes().unwrap(), bytes);
        assert_eq!(placed[0].instance.translation, [2., 0.75, 1.]);
        assert!(!props_sidecar.exists());
        click(&mut h, "Save props");
        assert!(props_sidecar.exists());
        assert_eq!(h.state().2.checksum(), checksum);
        assert_eq!(h.state().2.history(), &history);
        assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
        assert_eq!(fs::read(&source).unwrap(), bytes);
        assert_eq!(
            fs::read(&license).unwrap(),
            b"Explicit author fixture license"
        );
        let agent_instance = agent_before.instance;
        let prop_instance = placed[0].instance;
        drop(h);
        fs::rename(&source, root.join("hidden-source")).unwrap();
        fs::rename(&license, root.join("hidden-license")).unwrap();
        fs::rename(&package, root.join("hidden-package")).unwrap();
        let mut editor = Editor::open_game(&scene, EditorGame::MeshNavigation).unwrap();
        editor.sync();
        let mut art = Panel::default();
        art.sync(&editor);
        let mut props = crate::mesh_props_panel::Panel::default();
        props.sync(&editor);
        let agent = art.placement(&editor).unwrap();
        let prop_placements = props.placements(&editor);
        assert_eq!(prop_placements.len(), 1);
        assert_eq!(agent.model.to_bytes().unwrap(), bytes);
        assert_eq!(prop_placements[0].model.to_bytes().unwrap(), bytes);
        assert_eq!(agent.instance.translation, agent_instance.translation);
        assert_eq!(agent.instance.scale, agent_instance.scale);
        assert_eq!(
            prop_placements[0].instance.translation,
            prop_instance.translation
        );
        assert_eq!(prop_placements[0].instance.scale, prop_instance.scale);
        assert!(crate::mesh_prop_view::bounds(&agent).is_ok());
        assert!(crate::mesh_prop_view::bounds(&prop_placements[0]).is_ok());
        assert_eq!(editor.checksum(), checksum);
        assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
        let mut baseline =
            orr_sim::Simulation::<orr_remote::mesh_navigation::MeshNavigation>::from_frame(
                doc.frame(),
                orr_remote::mesh_navigation::YARD_TICK_RATE,
                orr_remote::mesh_navigation::MESH_NAVIGATION_BUILD_ID,
            )
            .unwrap();
        editor.step(1);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            editor.sync();
            if editor
                .snapshot()
                .is_some_and(|s| s.predicted().tick() == 1 && s.timeline().is_some())
            {
                break;
            }
            assert!(Instant::now() < deadline, "{name}: missing Play frame");
            std::thread::sleep(Duration::from_millis(2));
        }
        baseline.step(&orr_sim::TickInputs::new(1, 1));
        assert_eq!(
            editor.checksum(),
            baseline.frame().checksum(),
            "{name}: saved presentation changed simulation"
        );
        art.sync(&editor);
        props.sync(&editor);
        let moved = art.placement(&editor).unwrap();
        let view =
            orr_remote::mesh_navigation::mesh_from_view(editor.snapshot().unwrap().predicted())
                .unwrap();
        assert_eq!(
            moved.instance.translation,
            [
                view.agent.position[0].to_f32() + 0.5,
                view.agent.position[1].to_f32(),
                view.agent.position[2].to_f32()
            ]
        );
        assert_eq!(moved.model.to_bytes().unwrap(), bytes);
        assert_eq!(props.placements(&editor)[0].instance, prop_instance);
        assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    }
}
