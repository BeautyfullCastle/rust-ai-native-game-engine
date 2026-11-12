//! Grid form widgets and ticketed worker results; no texture/render creation.
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};

fn context() -> Context {
    Context {
        bindings_path: "/local/view.yaml".into(),
        project: "/local/project".into(),
        project_input: "project".into(),
        scene: "/local/scene.yaml".into(),
        load_epoch: 4,
        host_epoch: 7,
    }
}
fn image() -> backend::PngInspection {
    backend::PngInspection {
        width: 64,
        height: 16,
        sha256: "a".repeat(64),
    }
}
fn prepared() -> State {
    let mut state = State::default();
    state.draft.png = "/source.png".into();
    state.draft.clip = "walk".into();
    state.draft.license = "/license.txt".into();
    state.draft.name = "grid-prop".into();
    state.draft.destination = "/output".into();
    state.sync(Some(context()));
    state
}
fn inspect_fixture(state: &mut State) {
    state.inspection = Some(Inspected {
        pin: state.inspect_pin().unwrap(),
        image: image(),
    });
}
fn report() -> backend::Report {
    backend::Report {
        root: "/output".into(),
        package: "grid-prop".into(),
        document: "sprites.json".into(),
        width: 64,
        height: 16,
        cells: 4,
        source_sha256: "a".repeat(64),
    }
}
fn harness(state: State) -> Harness<'static, (State, Option<Context>)> {
    let mut h = Harness::builder()
        .with_size([1500.0, 1800.0])
        .build_ui_state(
            |ui, (state, context): &mut (State, Option<Context>)| {
                state.poll(context.clone());
                state.show(ui, context.clone());
            },
            (state, Some(context())),
        );
    h.run_steps(2);
    h
}
fn click(h: &mut Harness<'_, (State, Option<Context>)>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(2);
}
fn replace(h: &mut Harness<'_, (State, Option<Context>)>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.key_press(egui::Key::Backspace);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(2);
}

#[test]
fn grid_creator_is_opt_in_collapsed_and_integer_grid_validation_is_strict() {
    for raw in ["", "0", "-1", "+1", "1.5", "1e2", "18446744073709551616"] {
        assert!(decimal(raw, 20).is_err(), "{raw}");
    }
    assert_eq!(decimal("18446744073709551615", 20).unwrap(), u64::MAX);
    let mut h = harness(State::default());
    assert!(h.query_by_label("Sprite grid PNG file").is_none());
    click(&mut h, "Create sprite package from PNG grid");
    assert!(h
        .get_by_label("Inspect sprite PNG")
        .accesskit_node()
        .is_disabled());
    assert!(h
        .get_by_label("Create sprite grid package")
        .accesskit_node()
        .is_disabled());
    let mut state = prepared();
    assert!(state.spec().is_err());
    inspect_fixture(&mut state);
    assert_eq!(state.spec().unwrap().cell_width, 16);
    let document =
        backend::validate_grid_spec(&state.spec().unwrap(), state.current_image().unwrap())
            .unwrap();
    assert_eq!(
        document
            .regions()
            .iter()
            .map(|region| (region.id, region.x, region.y))
            .collect::<Vec<_>>(),
        vec![(0, 0, 0), (1, 16, 0), (2, 32, 0), (3, 48, 0)]
    );
    assert_eq!(document.clips().len(), 1);
    for (width, height, duration, clip) in [
        ("3", "16", "100", "walk"),
        ("0", "16", "100", "walk"),
        ("4294967296", "16", "100", "walk"),
        ("16", "16", "0", "walk"),
        ("16", "16", "18446744073709551615", "walk"),
        ("16", "16", "100", ""),
        ("16", "16", "100", "bad\nclip"),
        ("1", "1", "100", "walk"),
    ] {
        state.draft.cell_width = width.into();
        state.draft.cell_height = height.into();
        state.draft.duration = duration.into();
        state.draft.clip = clip.into();
        state.sync(Some(context()));
        assert!(
            state.spec().is_err(),
            "{width}/{height}/{duration}/{clip:?}"
        );
    }
}

#[test]
fn actual_png_inspection_fields_modes_rights_and_readonly_path_aba_are_explicit() {
    use std::time::{Duration, Instant};
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("grid.png");
    let bytes = include_bytes!("../../../../assets/sprite_demo/lantern_keeper.png");
    std::fs::write(&png, bytes).unwrap();
    let mut state = prepared();
    state.draft.png = png.display().to_string();
    state.sync(Some(context()));
    let mut h = harness(state);
    click(&mut h, "Create sprite package from PNG grid");
    assert!(h.state().0.current_image().is_err());
    assert!(h.state().0.created.is_none());
    click(&mut h, "Inspect sprite PNG");
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.state().0.busy() {
        h.run_steps(2);
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let inspected = h.state().0.current_image().unwrap().clone();
    assert_eq!((inspected.width, inspected.height), (64, 16));
    assert!(!h.state().0.rights);
    assert!(h
        .get_by_label("Create sprite grid package")
        .accesskit_node()
        .is_disabled());
    click(&mut h, "Grid rights acknowledgement");
    assert!(!h
        .get_by_label("Create sprite grid package")
        .accesskit_node()
        .is_disabled());
    for (label, value) in [
        ("Grid cell width", "8"),
        ("Grid cell height", "8"),
        ("Grid clip name", "run"),
        ("Grid frame duration (ms)", "50"),
        ("Grid license file", "/new-license.txt"),
        ("Grid package name", "new-grid"),
        ("Grid package version", "1.0.1"),
        ("Grid package destination", "/new-output"),
    ] {
        h.state_mut().0.rights = true;
        let ticket = h.state().0.draft_ticket;
        replace(&mut h, label, value);
        assert!(!h.state().0.rights, "{label}");
        assert!(h.state().0.draft_ticket > ticket);
        assert_eq!(h.state().0.current_image().unwrap(), &inspected);
    }
    for (mode, option) in [
        (PlaybackMode::Once, "Play grid clip once"),
        (PlaybackMode::Loop, "Loop grid clip"),
    ] {
        h.state_mut().0.rights = true;
        click(&mut h, "Grid playback mode");
        for label in ["Loop grid clip", "Play grid clip once"] {
            assert!(h.query_by_label(label).is_some());
        }
        click(&mut h, option);
        for label in ["Loop grid clip", "Play grid clip once"] {
            assert!(h.query_by_label(label).is_none());
        }
        assert_eq!(h.state().0.spec().unwrap().mode, mode);
        assert!(!h.state().0.rights);
    }
    replace(&mut h, "Grid cell width", "3");
    assert!(h.state().0.spec().is_err());
    replace(&mut h, "Grid cell width", "8");
    assert!(h.state().0.spec().is_ok());
    replace(
        &mut h,
        "Sprite grid PNG file",
        &dir.path().join("other.png").display().to_string(),
    );
    assert!(h.state().0.current_image().is_err());
    replace(&mut h, "Sprite grid PNG file", &png.display().to_string());
    assert!(h.state().0.current_image().is_err());
    assert!(!h.state().0.busy());
    h.state_mut().1 = None;
    h.run_steps(2);
    for label in [
        "Sprite grid PNG file",
        "Grid cell width",
        "Grid clip name",
        "Inspect sprite PNG",
        "Grid rights acknowledgement",
        "Create sprite grid package",
    ] {
        assert!(
            h.get_by_label(label).accesskit_node().is_disabled(),
            "{label}"
        );
    }
    assert_eq!(std::fs::read(&png).unwrap().as_slice(), bytes);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn inspection_pending_source_context_aba_max_and_disconnect_reject_without_duplicate_workers() {
    for changed in [
        "current",
        "source-aba",
        "context-aba",
        "readonly",
        "context-max",
        "source-max",
        "draft-max",
        "disconnected",
    ] {
        let mut state = prepared();
        let pin = state.inspect_pin().unwrap();
        let (sender, receiver) = mpsc::channel();
        state.pending = Some(Pending::Inspect { pin, receiver });
        state.start_inspection();
        assert!(state.busy());
        let mut current = Some(context());
        match changed {
            "source-aba" => {
                state.draft.png = "/other.png".into();
                state.sync(current.clone());
                state.draft.png = "/source.png".into();
                state.sync(current.clone());
            }
            "context-aba" => {
                let mut other = context();
                other.bindings_path = "/other.yaml".into();
                state.sync(Some(other));
                state.sync(current.clone());
            }
            "readonly" => {
                current = None;
                state.sync(None);
            }
            "context-max" => {
                state.context_ticket = u64::MAX - 1;
                state.sync(None);
                state.sync(current.clone());
            }
            "source-max" => {
                state.source_ticket = u64::MAX - 1;
                state.draft.png = "/other.png".into();
                state.sync(current.clone());
                state.draft.png = "/source.png".into();
                state.sync(current.clone());
            }
            "draft-max" => {
                state.draft_ticket = u64::MAX - 1;
                state.draft.clip = "changed".into();
                state.sync(current.clone());
            }
            _ => {}
        }
        assert!(state.busy());
        if changed == "disconnected" {
            drop(sender);
        } else {
            sender.send(Ok(image())).unwrap();
        }
        state.poll(current);
        assert!(!state.busy());
        assert!(!state.rights);
        assert_eq!(
            state.inspection.is_some(),
            changed == "current",
            "{changed}"
        );
        if changed.ends_with("max") {
            assert!(state.exhausted());
            assert!(state.inspect_pin().is_none());
            state.start_inspection();
            assert!(!state.busy());
        }
        if changed == "disconnected" {
            assert!(state.status.contains("disconnected"));
        }
    }
}

#[test]
fn creation_full_pin_busy_stale_once_and_partial_output_paths_are_retained() {
    for changed in [
        "current",
        "source-aba",
        "context-aba",
        "bindings",
        "project",
        "project-input",
        "scene",
        "load-epoch",
        "host-epoch",
        "readonly",
        "width",
        "height",
        "clip",
        "mode",
        "duration",
        "license",
        "name",
        "version",
        "destination",
        "image-hash",
        "max",
        "unexpected-root",
        "unexpected-hash",
        "backend-error",
        "disconnected",
    ] {
        let mut state = prepared();
        inspect_fixture(&mut state);
        state.rights = true;
        let pin = state.creation_pin().unwrap();
        let (sender, receiver) = mpsc::channel();
        state.pending = Some(Pending::Create {
            pin: Box::new(pin),
            receiver,
        });
        state.start_creation();
        assert!(state.busy());
        let mut current = Some(context());
        match changed {
            "source-aba" => {
                state.draft.png = "/other.png".into();
                state.sync(current.clone());
                state.draft.png = "/source.png".into();
                state.sync(current.clone());
            }
            "context-aba" => {
                state.sync(None);
                state.sync(current.clone());
            }
            "bindings" | "project" | "project-input" | "scene" | "load-epoch" | "host-epoch" => {
                let mut context = context();
                match changed {
                    "bindings" => context.bindings_path = "/other.yaml".into(),
                    "project" => context.project.push('2'),
                    "project-input" => context.project_input.push('2'),
                    "scene" => context.scene.push('2'),
                    "load-epoch" => context.load_epoch += 1,
                    _ => context.host_epoch += 1,
                }
                current = Some(context);
            }
            "readonly" => current = None,
            "width" => state.draft.cell_width = "8".into(),
            "height" => state.draft.cell_height = "8".into(),
            "clip" => state.draft.clip = "other".into(),
            "mode" => state.draft.mode = PlaybackMode::Once,
            "duration" => state.draft.duration = "50".into(),
            "license" => state.draft.license.push('2'),
            "name" => state.draft.name.push('2'),
            "version" => state.draft.version.push('2'),
            "destination" => state.draft.destination.push('2'),
            "image-hash" => state.inspection.as_mut().unwrap().image.sha256 = "b".repeat(64),
            "max" => state.draft_ticket = u64::MAX,
            _ => {}
        }
        let mut completed = report();
        if changed == "unexpected-root" {
            completed.root = "/unexpected-output".into();
        }
        if changed == "unexpected-hash" {
            completed.source_sha256 = "b".repeat(64);
        }
        if changed == "disconnected" {
            drop(sender);
        } else {
            sender
                .send(if changed == "backend-error" {
                    Err("visible partial output retained".into())
                } else {
                    Ok(completed)
                })
                .unwrap();
        }
        state.poll(current.clone());
        assert!(!state.busy());
        assert!(!state.rights);
        assert_eq!(state.created.is_some(), changed == "current", "{changed}");
        let expected_path = if changed == "unexpected-root" {
            "/unexpected-output"
        } else {
            "/output"
        };
        assert_eq!(
            state.last_output,
            Some(PathBuf::from(expected_path)),
            "{changed}"
        );
        assert!(
            state.status.contains(expected_path),
            "{changed}: {}",
            state.status
        );
        if changed == "current" {
            let created = state.created.take().unwrap();
            assert_eq!(created.cells, 4);
        }
        state.poll(current);
        assert!(
            state.created.is_none(),
            "a completion is never returned twice"
        );
    }
    let mut state = prepared();
    inspect_fixture(&mut state);
    state.rights = true;
    let pin = state.creation_pin().unwrap();
    let (_sender, receiver) = mpsc::channel();
    state.pending = Some(Pending::Create {
        pin: Box::new(pin),
        receiver,
    });
    let mut h = harness(state);
    click(&mut h, "Create sprite package from PNG grid");
    for label in [
        "Sprite grid PNG file",
        "Grid cell width",
        "Grid cell height",
        "Grid clip name",
        "Grid playback mode",
        "Grid frame duration (ms)",
        "Grid license file",
        "Grid package name",
        "Grid package version",
        "Grid package destination",
        "Inspect sprite PNG",
        "Grid rights acknowledgement",
        "Create sprite grid package",
    ] {
        assert!(
            h.get_by_label(label).accesskit_node().is_disabled(),
            "{label}"
        );
    }
}
