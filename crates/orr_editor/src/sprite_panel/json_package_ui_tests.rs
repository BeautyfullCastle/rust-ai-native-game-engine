//! JSON package form accessibility, invalidation and ticketed worker handling.
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::time::{Duration, Instant};

const JSON: &str = r#"{
  "format": "orr_sprite",
  "version": 1,
  "atlas": { "image": "atlas.png", "width": 1, "height": 1 },
  "regions": [{ "id": 0, "x": 0, "y": 0, "width": 1, "height": 1 }],
  "clips": []
}"#;

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

fn source_files() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let json = temp.path().join("sprites.json");
    let png_path = temp.path().join("atlas.png");
    std::fs::write(&json, JSON).unwrap();
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[11, 23, 47, 255])
            .unwrap();
    }
    std::fs::write(&png_path, png).unwrap();
    (temp, json, png_path)
}

fn sources() -> (tempfile::TempDir, PathBuf, PathBuf, backend::Inspection) {
    let (temp, json, png_path) = source_files();
    let inspection = backend::inspect_sources(&json, &png_path).unwrap();
    (temp, json, png_path, inspection)
}

fn configured() -> (tempfile::TempDir, State) {
    let (temp, json, png, inspection) = sources();
    let mut state = State::default();
    state.draft.json = json.to_string_lossy().into_owned();
    state.draft.png = png.to_string_lossy().into_owned();
    state.draft.license = temp
        .path()
        .join("LICENSE.txt")
        .to_string_lossy()
        .into_owned();
    state.draft.name = "json-atlas".into();
    state.draft.version = "1.0.0".into();
    state.draft.destination = temp.path().join("output").to_string_lossy().into_owned();
    state.sync(Some(context()));
    state.inspection = Some(Inspected {
        pin: state.inspect_pin().unwrap(),
        inspection,
    });
    (temp, state)
}

fn report(state: &State) -> backend::Report {
    let inspection = state.inspection.as_ref().unwrap().inspection.clone();
    backend::Report {
        root: state.draft.destination.trim().into(),
        package: state.draft.name.trim().into(),
        document: "sprites.json".into(),
        image_filename: inspection.image_filename().into(),
        width: inspection.width(),
        height: inspection.height(),
        region_count: inspection.region_count(),
        clip_count: inspection.clip_count(),
        json_sha256: inspection.json_sha256().into(),
        png_sha256: inspection.png_sha256().into(),
    }
}

fn harness(state: State, context: Option<Context>) -> Harness<'static, (State, Option<Context>)> {
    let mut h = Harness::builder()
        .with_size([1300.0, 1100.0])
        .build_ui_state(
            |ui, (state, context): &mut (State, Option<Context>)| {
                state.poll(context.clone());
                state.show(ui, context.clone(), false);
            },
            (state, context),
        );
    h.run_steps(2);
    h
}

fn blocked_harness(
    state: State,
    context: Option<Context>,
) -> Harness<'static, (State, Option<Context>)> {
    let mut h = Harness::builder()
        .with_size([1300.0, 1100.0])
        .build_ui_state(
            |ui, (state, context): &mut (State, Option<Context>)| {
                state.poll(context.clone());
                state.show(ui, context.clone(), true);
            },
            (state, context),
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

fn wait_json(h: &mut Harness<'_, (State, Option<Context>)>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().0.busy() {
        h.run_steps(3);
        assert!(Instant::now() < deadline, "{}", h.state().0.status);
        std::thread::sleep(Duration::from_millis(5));
    }
    h.run_steps(3);
}

#[test]
fn panel_is_collapsed_local_edit_gated_and_text_inputs_have_exact_accessible_labels() {
    let mut h = harness(State::default(), None);
    assert!(h.query_by_label("Sprite JSON file").is_none());
    click(&mut h, "Create sprite package from JSON");
    assert!(
        h.get_by_label("Inspect sprite JSON and PNG")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        h.get_by_label("Create sprite JSON package")
            .accesskit_node()
            .is_disabled()
    );

    let (_temp, state) = configured();
    let mut h = harness(state, Some(context()));
    click(&mut h, "Create sprite package from JSON");
    for label in [
        "Sprite JSON file",
        "Sprite JSON PNG file",
        "Sprite JSON license file",
        "Sprite JSON package name",
        "Sprite JSON package version",
        "Sprite JSON package destination",
    ] {
        let _ = h.get_by_role_and_label(Role::TextInput, label);
    }
}

#[test]
fn inspect_button_runs_worker_and_shows_summary_without_acknowledging_rights() {
    let (temp, json, png) = source_files();
    let state = State::default();
    let mut h = harness(state, Some(context()));
    click(&mut h, "Create sprite package from JSON");
    let license = temp.path().join("LICENSE.txt");
    let destination = temp.path().join("output");
    for (label, value) in [
        ("Sprite JSON file", json.to_str().unwrap()),
        ("Sprite JSON PNG file", png.to_str().unwrap()),
        ("Sprite JSON license file", license.to_str().unwrap()),
        ("Sprite JSON package name", "json-ui-test"),
        ("Sprite JSON package version", "1.0.0"),
        (
            "Sprite JSON package destination",
            destination.to_str().unwrap(),
        ),
    ] {
        replace(&mut h, label, value);
    }
    assert!(h.state().0.creation_pin().is_err());
    click(&mut h, "Inspect sprite JSON and PNG");
    wait_json(&mut h);

    let (summary, inspection) = {
        let state = &h.state().0;
        let inspection = &state
            .inspection
            .as_ref()
            .expect("worker inspection")
            .inspection;
        (
            format!(
                "Inspected {} x {}, {} regions, {} clips; image {}. JSON SHA-256 {}; PNG SHA-256 {}",
                inspection.width(),
                inspection.height(),
                inspection.region_count(),
                inspection.clip_count(),
                inspection.image_filename(),
                inspection.json_sha256(),
                inspection.png_sha256()
            ),
            inspection.clone(),
        )
    };
    assert!(
        h.query_by_label(&summary).is_some(),
        "inspection summary is visible"
    );
    assert_eq!(inspection.width(), 1);
    assert_eq!(inspection.height(), 1);
    assert_eq!(inspection.region_count(), 1);
    assert_eq!(inspection.clip_count(), 0);
    assert_eq!(h.state().0.inspection.as_ref().unwrap().pin.json, json);
    assert_eq!(h.state().0.inspection.as_ref().unwrap().pin.png, png);
    assert!(!h.state().0.rights);
    assert!(h.state().0.creation_pin().is_ok());
    assert!(
        h.get_by_label("Create sprite JSON package")
            .accesskit_node()
            .is_disabled()
    );
}

#[test]
fn source_context_and_package_draft_changes_invalidate_the_right_evidence() {
    let (_temp, mut state) = configured();
    state.rights = true;
    state.created = Some(report(&state));
    let inspection = state.inspection.clone();
    state.draft.license.push_str("-changed");
    state.sync(Some(context()));
    assert_eq!(
        state.inspection, inspection,
        "license edits retain source inspection"
    );
    assert!(!state.rights);
    assert!(state.created.is_none());

    state.rights = true;
    state.created = Some(report(&state));
    state.draft.json.push_str("-changed");
    state.sync(Some(context()));
    assert!(
        state.inspection.is_none(),
        "JSON source path invalidates inspection"
    );
    assert!(!state.rights);
    assert!(state.created.is_none());

    let (temp, mut state) = configured();
    state.rights = true;
    state.created = Some(report(&state));
    let before_context_ticket = state.context_ticket;
    state.sync(None);
    assert!(state.context_ticket > before_context_ticket);
    assert!(state.inspection.is_none());
    assert!(!state.rights);
    assert!(state.created.is_none());
    drop(temp);
}

#[test]
fn a_b_a_source_context_and_draft_changes_stale_the_original_worker_tickets() {
    let (_temp, mut state) = configured();
    let source_pin = state.inspect_pin().unwrap();
    let inspection = state.inspection.as_ref().unwrap().inspection.clone();
    let source_ticket = state.source_ticket;
    let original_json = state.draft.json.clone();
    state.draft.json = "/different/sprites.json".into();
    state.sync(Some(context()));
    state.draft.json.clone_from(&original_json);
    state.sync(Some(context()));
    assert_eq!(state.source_ticket, source_ticket + 2);
    assert!(state.inspection.is_none());

    let (sender, receiver) = mpsc::channel();
    state.pending = Some(Pending::Inspect {
        pin: source_pin,
        receiver,
    });
    sender.send(Ok(inspection)).unwrap();
    state.poll(Some(context()));
    assert!(state.inspection.is_none());
    assert!(state.status.contains("prior editor context or source"));

    let (_temp, mut state) = configured();
    let context_pin = state.inspect_pin().unwrap();
    let inspection = state.inspection.as_ref().unwrap().inspection.clone();
    let context_ticket = state.context_ticket;
    let context_a = context();
    let mut context_b = context_a.clone();
    context_b.host_epoch += 1;
    state.sync(Some(context_b));
    state.sync(Some(context_a.clone()));
    assert_eq!(state.context_ticket, context_ticket + 2);

    let (sender, receiver) = mpsc::channel();
    state.pending = Some(Pending::Inspect {
        pin: context_pin,
        receiver,
    });
    sender.send(Ok(inspection)).unwrap();
    state.poll(Some(context_a));
    assert!(state.inspection.is_none());
    assert!(state.status.contains("prior editor context or source"));

    let (_temp, mut state) = configured();
    let creation_pin = state.creation_pin().unwrap();
    let result = report(&state);
    let draft_ticket = state.draft_ticket;
    let original_name = state.draft.name.clone();
    state.draft.name = "json-atlas-b".into();
    state.sync(Some(context()));
    state.draft.name = original_name;
    state.sync(Some(context()));
    assert_eq!(state.draft_ticket, draft_ticket + 2);

    let (sender, receiver) = mpsc::channel();
    state.pending = Some(Pending::Create {
        pin: Box::new(creation_pin),
        receiver,
    });
    sender.send(Ok(result)).unwrap();
    state.poll(Some(context()));
    assert!(state.created.is_none());
    assert!(state.status.contains("prior context/draft"));
}

#[test]
fn context_source_and_draft_tickets_saturate_at_u64_max() {
    let (_temp, mut state) = configured();
    state.context_ticket = u64::MAX - 1;
    state.source_ticket = u64::MAX - 1;
    state.draft_ticket = u64::MAX - 1;
    state.rights = true;
    state.created = Some(report(&state));
    let mut changed_context = context();
    changed_context.host_epoch += 1;
    state.draft.json.push_str("-changed");
    state.draft.name.push_str("-changed");
    state.sync(Some(changed_context.clone()));
    assert_eq!(state.context_ticket, u64::MAX);
    assert_eq!(state.source_ticket, u64::MAX);
    assert_eq!(state.draft_ticket, u64::MAX);
    assert!(state.inspection.is_none());
    assert!(!state.rights);
    assert!(state.created.is_none());
    assert!(state.status.contains("tickets exhausted"));

    changed_context.host_epoch += 1;
    state.draft.json.push_str("-again");
    state.draft.name.push_str("-again");
    state.sync(Some(changed_context));
    assert_eq!(state.context_ticket, u64::MAX);
    assert_eq!(state.source_ticket, u64::MAX);
    assert_eq!(state.draft_ticket, u64::MAX);
}

#[test]
fn queued_inspection_and_creation_completions_are_discarded_after_context_changes() {
    let (_temp, mut state) = configured();
    let pin = state.inspect_pin().unwrap();
    let inspection = state.inspection.as_ref().unwrap().inspection.clone();
    let (sender, receiver) = mpsc::channel();
    state.pending = Some(Pending::Inspect { pin, receiver });
    sender.send(Ok(inspection)).unwrap();
    state.poll(None);
    assert!(state.inspection.is_none());
    assert!(!state.busy());
    assert!(state.status.contains("prior editor context or source"));

    let (_temp, mut state) = configured();
    let pin = state.creation_pin().unwrap();
    let report = report(&state);
    let (sender, receiver) = mpsc::channel();
    state.pending = Some(Pending::Create {
        pin: Box::new(pin),
        receiver,
    });
    sender.send(Ok(report)).unwrap();
    let mut changed_context = context();
    changed_context.host_epoch += 1;
    state.poll(Some(changed_context));
    assert!(state.created.is_none());
    assert!(!state.busy());
    assert!(state.status.contains("prior context/draft"));
}

#[test]
fn disconnected_workers_clear_pending_state_and_preserve_create_destination() {
    let (_temp, mut state) = configured();
    let pin = state.inspect_pin().unwrap();
    let (sender, receiver) = mpsc::channel();
    drop(sender);
    state.pending = Some(Pending::Inspect { pin, receiver });
    state.poll(Some(context()));
    assert!(!state.busy());
    assert!(state.inspection.is_none());
    assert!(!state.rights);
    assert!(state.created.is_none());
    assert!(state.status.contains("inspection worker disconnected"));
    assert!(!state.status.contains("retry"));

    let (_temp, mut state) = configured();
    let pin = state.creation_pin().unwrap();
    let destination = PathBuf::from(pin.draft.destination.trim());
    state.created = Some(report(&state));
    let (sender, receiver) = mpsc::channel();
    drop(sender);
    state.pending = Some(Pending::Create {
        pin: Box::new(pin),
        receiver,
    });
    state.poll(Some(context()));
    assert!(!state.busy());
    assert_eq!(state.last_output, Some(destination));
    assert!(
        state.created.is_none(),
        "a disconnected result is never adopted"
    );
    assert!(state.status.contains("worker disconnected"));
    assert!(
        state
            .status
            .contains("may have completed or left partial output")
    );
    assert!(state.status.contains("No automatic retry"));
}

#[test]
fn pending_inspect_and_create_reject_duplicates_and_disable_their_controls() {
    let (_temp, mut state) = configured();
    let pin = state.inspect_pin().unwrap();
    let inspection = state.inspection.as_ref().unwrap().inspection.clone();
    let (sender, receiver) = mpsc::channel();
    state.pending = Some(Pending::Inspect {
        pin: pin.clone(),
        receiver,
    });
    state.start_inspection();
    assert!(state.busy());
    match state.pending.as_ref().unwrap() {
        Pending::Inspect {
            pin: pending_pin, ..
        } => assert_eq!(pending_pin, &pin),
        Pending::Create { .. } => panic!("duplicate inspect replaced pending create"),
    }

    let mut h = harness(state, Some(context()));
    click(&mut h, "Create sprite package from JSON");
    assert!(h.state().0.busy());
    assert!(
        h.get_by_role_and_label(Role::TextInput, "Sprite JSON file")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        h.get_by_label("Inspect sprite JSON and PNG")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        h.get_by_label("Create sprite JSON package")
            .accesskit_node()
            .is_disabled()
    );
    sender
        .send(Ok(inspection))
        .expect("pending inspect receiver remains owned");
    wait_json(&mut h);
    assert!(h.state().0.inspection.is_some());
    assert!(!h.state().0.rights);

    let (_temp, mut state) = configured();
    state.rights = true;
    let pin = state.creation_pin().unwrap();
    let result = report(&state);
    let (sender, receiver) = mpsc::channel();
    state.pending = Some(Pending::Create {
        pin: Box::new(pin.clone()),
        receiver,
    });
    state.start_creation();
    assert!(state.busy());
    assert!(
        state.rights,
        "duplicate create does not consume the acknowledgement"
    );
    match state.pending.as_ref().unwrap() {
        Pending::Create {
            pin: pending_pin, ..
        } => assert_eq!(pending_pin.as_ref(), &pin),
        Pending::Inspect { .. } => panic!("duplicate create replaced pending inspect"),
    }

    let mut h = harness(state, Some(context()));
    click(&mut h, "Create sprite package from JSON");
    assert!(h.state().0.busy());
    assert!(
        h.get_by_role_and_label(Role::TextInput, "Sprite JSON package name")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        h.get_by_label("Inspect sprite JSON and PNG")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        h.get_by_label("Create sprite JSON package")
            .accesskit_node()
            .is_disabled()
    );
    sender
        .send(Ok(result))
        .expect("pending create receiver remains owned");
    wait_json(&mut h);
    assert!(!h.state().0.busy());
    assert!(
        h.state().0.created.is_some(),
        "only the one pending result is adopted"
    );
}

#[test]
fn editable_fields_clear_rights_and_results_but_preserve_matching_inspection() {
    let (_temp, state) = configured();
    let mut h = harness(state, Some(context()));
    click(&mut h, "Create sprite package from JSON");
    click(&mut h, "Sprite JSON rights acknowledgement");
    assert!(h.state().0.rights);
    replace(&mut h, "Sprite JSON package name", "renamed-json-atlas");
    assert!(!h.state().0.rights);
    assert!(h.state().0.inspection.is_some());
    assert!(h.state().0.created.is_none());

    replace(&mut h, "Sprite JSON PNG file", "/different-atlas.png");
    assert!(!h.state().0.rights);
    assert!(h.state().0.inspection.is_none());
    assert!(h.state().0.created.is_none());
}

#[test]
fn active_installer_grid_or_rectangle_work_disables_json_authoring_controls() {
    let (_temp, mut state) = configured();
    state.rights = true;
    let mut h = blocked_harness(state, Some(context()));
    click(&mut h, "Create sprite package from JSON");
    assert!(
        h.get_by_label("Inspect sprite JSON and PNG")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        h.get_by_label("Create sprite JSON package")
            .accesskit_node()
            .is_disabled()
    );
}
