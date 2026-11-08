//! Rectangle form validation and ticketed worker completions; CPU-only egui.
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};

fn context() -> Context {
    Context {
        bindings_path: "/local/view.yaml".into(),
        bindings_document: b"{}".to_vec(),
        project: "/local/project".into(),
        project_input: "project".into(),
        scene: "/local/scene.yaml".into(),
        scene_load_epoch: 4,
        host_epoch: 7,
    }
}

fn image() -> backend::PngInspection {
    backend::PngInspection {
        width: 7,
        height: 5,
        sha256: "a".repeat(64),
    }
}

fn configured() -> State {
    let mut state = State::default();
    state.draft.png = "/source.png".into();
    state.draft.license = "/license.txt".into();
    state.draft.name = "rectangles".into();
    state.draft.version = "1.0.0".into();
    state.draft.destination = "/output".into();
    state.sync(Some(context()));
    state.inspection = Some(Inspected {
        pin: state.inspect_pin().unwrap(),
        image: image(),
    });
    state
}

fn three_regions(state: &mut State) {
    state.draft.rectangles = vec![
        RectangleDraft {
            row_key: 1,
            id: "10".into(),
            x: "0".into(),
            y: "0".into(),
            width: "3".into(),
            height: "2".into(),
        },
        RectangleDraft {
            row_key: 2,
            id: "42".into(),
            x: "4".into(),
            y: "1".into(),
            width: "2".into(),
            height: "3".into(),
        },
        RectangleDraft {
            row_key: 3,
            id: "900".into(),
            x: "1".into(),
            y: "3".into(),
            width: "4".into(),
            height: "2".into(),
        },
    ];
    state.sync(Some(context()));
}

fn report() -> backend::Report {
    backend::Report {
        root: "/output".into(),
        package: "rectangles".into(),
        document: "sprites.json".into(),
        width: 7,
        height: 5,
        region_count: 3,
        source_sha256: "a".repeat(64),
    }
}

fn harness(state: State) -> Harness<'static, (State, Option<Context>)> {
    let mut h = Harness::builder()
        .with_size([1400.0, 1800.0])
        .build_ui_state(
            |ui, (state, context): &mut (State, Option<Context>)| {
                state.poll(context.clone());
                state.show(ui, context.clone(), false);
            },
            (state, Some(context())),
        );
    h.run_steps(2);
    h
}

fn click(h: &mut Harness<'_, (State, Option<Context>)>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(2);
    h.get_by_label(label).click_accesskit();
    h.run_steps(2);
}

fn replace(h: &mut Harness<'_, (State, Option<Context>)>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    h.run_steps(2);
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
fn ascii_decimal_is_exact_checked_and_invalid_drafts_are_preserved() {
    assert_eq!(decimal_u32("0", true), Ok(0));
    assert_eq!(decimal_u32("4294967295", true), Ok(u32::MAX));
    assert_eq!(decimal_u32("1", false), Ok(1));
    for raw in [
        "",
        " ",
        " 0",
        "0 ",
        "+1",
        "-1",
        "1.0",
        "1e1",
        "１２",
        "4294967296",
    ] {
        assert!(decimal_u32(raw, true).is_err(), "accepted {raw:?}");
    }
    assert!(decimal_u32("0", false).is_err());

    let mut state = configured();
    state.draft.rectangles[0].width = " 3".into();
    state.sync(Some(context()));
    assert_eq!(state.draft.rectangles[0].width, " 3");
    assert!(state.rectangle_spec().unwrap_err().contains("ASCII"));
    state.draft.rectangles[0].width = "3".into();
    state.draft.rectangles[0].id = "4294967296".into();
    state.sync(Some(context()));
    assert_eq!(state.draft.rectangles[0].id, "4294967296");
    assert!(state.rectangle_spec().unwrap_err().contains("u32"));
}

#[test]
fn semantic_validation_keeps_colliding_default_ids_visible_and_accepts_exact_rectangles() {
    let mut state = configured();
    let second = RectangleDraft::new(2);
    state.draft.rectangles.push(second.clone());
    state.sync(Some(context()));
    let error = state.validate_draft().unwrap_err();
    assert!(error.contains("duplicate region 0"), "{error}");
    assert_eq!(state.draft.rectangles[1].id, "0");

    three_regions(&mut state);
    state.sync(Some(context()));
    let document = state.validate_draft().unwrap();
    assert_eq!(
        document
            .regions()
            .iter()
            .map(|region| (region.id, region.x, region.y, region.width, region.height))
            .collect::<Vec<_>>(),
        vec![(10, 0, 0, 3, 2), (42, 4, 1, 2, 3), (900, 1, 3, 4, 2)]
    );
    for (id, x, y, width, height) in [
        ("10", "0", "0", "0", "2"),
        ("10", "0", "0", "3", "0"),
        ("10", "5", "4", "3", "2"),
        ("10", "4294967295", "0", "1", "1"),
    ] {
        state.draft.rectangles[0] = RectangleDraft {
            row_key: 1,
            id: id.into(),
            x: x.into(),
            y: y.into(),
            width: width.into(),
            height: height.into(),
        };
        state.sync(Some(context()));
        assert!(
            state.validate_draft().is_err(),
            "{id}/{x}/{y}/{width}/{height}"
        );
    }
}

#[test]
fn validation_feedback_identifies_parse_duplicate_and_bounds_rows() {
    let mut state = configured();
    three_regions(&mut state);

    state.draft.rectangles[1].height = "two".into();
    state.sync(Some(context()));
    let parse_error = state.rectangle_spec().unwrap_err();
    assert!(
        parse_error.contains("Rectangle row 2 (key 2, ID draft \"42\")"),
        "{parse_error}"
    );
    assert!(parse_error.contains("Height: use ASCII decimal digits only"));

    state.draft.rectangles[1].height = "3".into();
    state.draft.rectangles.push(RectangleDraft {
        row_key: 4,
        id: "42".into(),
        x: "0".into(),
        y: "0".into(),
        width: "1".into(),
        height: "1".into(),
    });
    state.sync(Some(context()));
    let duplicate = state.validate_draft().unwrap_err();
    assert!(duplicate.contains("duplicate region 42"), "{duplicate}");
    assert!(duplicate.contains("row 2 (key 2, ID 42)"), "{duplicate}");
    assert!(duplicate.contains("row 4 (key 4, ID 42)"), "{duplicate}");

    state.draft.rectangles.pop();
    state.draft.rectangles[2].x = "5".into();
    state.sync(Some(context()));
    let bounds = state.validate_draft().unwrap_err();
    assert!(
        bounds.contains("region 900 outside atlas or empty"),
        "{bounds}"
    );
    assert!(bounds.contains("row 3 (key 3, ID 900)"), "{bounds}");
    assert_eq!(state.draft.rectangles[2].x, "5");
}

#[test]
fn rectangle_panel_is_collapsed_context_gated_and_bounded_to_256_rows() {
    let mut h = harness(State::default());
    assert!(h.query_by_label("Rectangle PNG file").is_none());
    click(&mut h, "Create sprite package from rectangles");
    assert!(h
        .get_by_label("Inspect rectangle PNG")
        .accesskit_node()
        .is_disabled());
    assert!(h
        .get_by_label("Create rectangle sprite package")
        .accesskit_node()
        .is_disabled());

    let mut state = configured();
    state.draft.rectangles = (0..MAX_REGIONS)
        .map(|index| {
            let mut row = RectangleDraft::new(index as u64 + 1);
            row.id = index.to_string();
            row.x = "0".into();
            row.y = "0".into();
            row.width = "1".into();
            row.height = "1".into();
            row
        })
        .collect();
    state.next_row_key = MAX_REGIONS as u64 + 1;
    state.observed_draft = state.draft.clone();
    let mut h = harness(state);
    click(&mut h, "Create sprite package from rectangles");
    assert_eq!(h.state().0.draft.rectangles.len(), MAX_REGIONS);
    assert!(h
        .get_by_label("Add rectangle")
        .accesskit_node()
        .is_disabled());
}

#[test]
fn add_and_remove_rows_preserve_stable_ui_keys_and_never_renumber_region_ids() {
    let mut h = harness(configured());
    click(&mut h, "Create sprite package from rectangles");
    click(&mut h, "Add rectangle");
    assert_eq!(
        h.state()
            .0
            .draft
            .rectangles
            .iter()
            .map(|row| (row.row_key, row.id.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, "0"), (2, "0")]
    );
    assert!(h
        .state()
        .0
        .validate_draft()
        .unwrap_err()
        .contains("duplicate region 0"));

    click(&mut h, "Remove rectangle row 2");
    assert_eq!(h.state().0.draft.rectangles.len(), 1);
    assert_eq!(h.state().0.draft.rectangles[0].row_key, 1);
    assert_eq!(h.state().0.draft.rectangles[0].id, "0");
    assert!(h
        .get_by_label("Remove rectangle row 1")
        .accesskit_node()
        .is_disabled());
}

#[test]
fn edits_invalidate_rights_results_and_source_inspections_without_clamping_or_renumbering() {
    let mut state = configured();
    three_regions(&mut state);
    state.rights = true;
    state.created = Some(report());
    state.inspection = Some(Inspected {
        pin: state.inspect_pin().unwrap(),
        image: image(),
    });
    let mut h = harness(state);
    click(&mut h, "Create sprite package from rectangles");
    let ticket = h.state().0.draft_ticket;
    replace(&mut h, "Rectangle X row 2", "4294967295");
    assert!(h.state().0.draft_ticket > ticket);
    assert_eq!(h.state().0.draft.rectangles[1].x, "4294967295");
    assert!(!h.state().0.rights);
    assert!(h.state().0.created.is_none());
    assert!(h.state().0.current_image().is_ok());

    h.state_mut().0.rights = true;
    replace(&mut h, "Rectangle package name", "next-name");
    assert!(!h.state().0.rights);
    assert!(h.state().0.created.is_none());
    assert_eq!(
        h.state()
            .0
            .draft
            .rectangles
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        ["10", "42", "900"]
    );

    replace(&mut h, "Rectangle PNG file", "/different.png");
    assert!(h.state().0.current_image().is_err());
    assert_eq!(h.state().0.draft.rectangles.len(), 3);
}

#[test]
fn inspection_and_creation_workers_reject_aba_contexts_and_preserve_started_output_paths() {
    for changed in [
        "current",
        "source-aba",
        "context-aba",
        "readonly",
        "max",
        "disconnected",
    ] {
        let mut state = configured();
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
            "max" => {
                state.context_ticket = u64::MAX - 1;
                state.sync(None);
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
        assert_eq!(
            state.inspection.is_some(),
            changed == "current",
            "{changed}"
        );
        assert!(!state.rights);
    }

    for changed in [
        "current",
        "draft-aba",
        "context-aba",
        "unexpected",
        "error",
        "disconnected",
    ] {
        let mut state = configured();
        three_regions(&mut state);
        state.sync(Some(context()));
        let pin = state.creation_pin().unwrap();
        let (sender, receiver) = mpsc::channel();
        state.pending = Some(Pending::Create {
            pin: Box::new(pin),
            receiver,
        });
        state.rights = true;
        state.start_creation();
        assert!(state.busy());
        let mut current = Some(context());
        match changed {
            "draft-aba" => {
                state.draft.name = "changed".into();
                state.sync(current.clone());
                state.draft.name = "rectangles".into();
                state.sync(current.clone());
            }
            "context-aba" => {
                state.sync(None);
                state.sync(current.clone());
            }
            _ => {}
        }
        let mut completed = report();
        if changed == "unexpected" {
            completed.root = "/unexpected".into();
        }
        if changed == "disconnected" {
            drop(sender);
        } else {
            sender
                .send(if changed == "error" {
                    Err("partial output".into())
                } else {
                    Ok(completed)
                })
                .unwrap();
        }
        state.poll(current);
        assert!(!state.busy());
        assert_eq!(state.created.is_some(), changed == "current", "{changed}");
        let expected_path = if changed == "unexpected" {
            "/unexpected"
        } else {
            "/output"
        };
        assert_eq!(
            state.last_output,
            Some(PathBuf::from(expected_path)),
            "{changed}"
        );
        assert!(state.status.contains(expected_path), "{}", state.status);
    }
}

#[test]
fn busy_rectangle_worker_disables_all_editable_controls() {
    let mut state = configured();
    let pin = state.inspect_pin().unwrap();
    let (_sender, receiver) = mpsc::channel();
    state.pending = Some(Pending::Inspect { pin, receiver });
    let mut h = harness(state);
    click(&mut h, "Create sprite package from rectangles");
    for label in [
        "Rectangle PNG file",
        "Rectangle ID row 1",
        "Rectangle X row 1",
        "Rectangle Y row 1",
        "Rectangle Width row 1",
        "Rectangle Height row 1",
        "Add rectangle",
        "Remove rectangle row 1",
        "Inspect rectangle PNG",
        "Rectangle rights acknowledgement",
        "Create rectangle sprite package",
    ] {
        assert!(
            h.get_by_label(label).accesskit_node().is_disabled(),
            "{label}"
        );
    }
}
