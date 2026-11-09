use super::*;

fn event(logical: Key, physical: Key, pressed: bool, repeat: bool) -> Event {
    Event::Key {
        key: logical,
        physical_key: Some(physical),
        pressed,
        repeat,
        modifiers: egui::Modifiers::NONE,
    }
}
fn press(key: Key) -> Event {
    event(key, key, true, false)
}
fn release(key: Key) -> Event {
    event(key, key, false, false)
}
fn custom() -> ActionMap {
    let mut map = RoomControls::default_map();
    for (action, key) in map.actions.iter_mut().zip([
        PhysicalKey::J,
        PhysicalKey::L,
        PhysicalKey::I,
        PhysicalKey::K,
        PhysicalKey::Tab,
    ]) {
        action.bindings = vec![Button::Keyboard { key }];
    }
    map
}

#[test]
fn physical_aliases_signs_and_logical_escape_follow_admitted_map() {
    let mut router = Router::default();
    router.install(&custom()).unwrap();
    let frame = router.route(
        &[
            event(Key::Escape, Key::I, true, false),
            press(Key::L),
            press(Key::Tab),
        ],
        true,
    );
    assert_eq!(
        frame.keys,
        Keys {
            x: 1,
            y: 1,
            fire: true
        }
    );
    assert!(!frame.release);
    assert_eq!(frame.consumed, vec![0, 1, 2]);
    let frame = router.route(&[press(Key::J), press(Key::K)], true);
    assert_eq!(
        frame.keys,
        Keys {
            x: 0,
            y: 0,
            fire: true
        }
    );
    let frame = router.route(&[release(Key::I), release(Key::L), release(Key::Tab)], true);
    assert_eq!(
        frame.keys,
        Keys {
            x: -1,
            y: -1,
            fire: false
        }
    );
    let frame = router.route(&[event(Key::I, Key::Escape, true, false)], true);
    assert!(frame.release);
    assert_eq!(frame.keys, Keys::default());
}

#[test]
fn focus_reclaim_map_change_and_repeat_quarantine_until_release() {
    let mut router = Router::default();
    assert_eq!(router.route(&[press(Key::W)], false).keys, Keys::default());
    assert_eq!(router.route(&[], true).keys, Keys::default());
    assert_eq!(
        router
            .route(&[event(Key::W, Key::W, true, true)], true)
            .keys,
        Keys::default()
    );
    assert_eq!(
        router.route(&[release(Key::W), press(Key::W)], true).keys.y,
        1
    );
    router.route(&[], false);
    assert_eq!(router.route(&[], true).keys, Keys::default());
    router.install(&RoomControls::default_map()).unwrap();
    assert_eq!(router.route(&[press(Key::W)], true).keys, Keys::default());
    assert_eq!(
        router.route(&[release(Key::W), press(Key::W)], true).keys.y,
        1
    );
    let frame = router.route(&[press(Key::Tab)], true);
    assert!(frame.release);
    assert_eq!(frame.keys, Keys::default());
}

#[test]
fn invalid_map_and_pending_draft_do_not_replace_live_controls() {
    let mut controls = Controls::default();
    controls.panel.apply_document(custom()).unwrap();
    controls.apply().unwrap();
    let hint = controls.hint();
    controls
        .panel
        .replace_draft(vec![Button::Keyboard {
            key: PhysicalKey::X,
        }])
        .unwrap();
    assert!(controls.apply().is_err());
    assert_eq!(controls.hint(), hint);
    let mut invalid = custom();
    invalid.actions[0].bindings = invalid.actions[1].bindings.clone();
    assert!(controls.router.install(&invalid).is_err());
    assert_eq!(controls.hint(), hint);
}

#[test]
fn scene_change_resets_session_map_without_discarding_authoring_draft() {
    let mut controls = Controls::default();
    controls.sync(1, true, Some("room.yaml".into()));
    controls.panel.apply_document(custom()).unwrap();
    controls.apply().unwrap();
    controls
        .panel
        .replace_draft(vec![Button::Keyboard {
            key: PhysicalKey::X,
        }])
        .unwrap();
    controls.sync(1, true, Some("room.yaml".into()));
    assert!(controls.hint().contains("J left"));
    controls.sync(2, true, Some("room.yaml".into()));
    assert!(controls.hint().contains("A/ArrowLeft left"));
    assert!(controls.panel.has_pending_draft());
    assert_eq!(controls.panel.document(), &custom());
}

#[test]
fn missing_identity_and_shortcuts_do_not_create_gameplay_presses() {
    let mut router = Router::default();
    let fallback = Event::Key {
        key: Key::D,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    let frame = router.route(&[fallback], true);
    assert_eq!(frame.keys, Keys::default());
    assert!(frame.release && frame.missing_identity);
    router.route(&[release(Key::D)], true);
    let shortcut = Event::Key {
        key: Key::W,
        physical_key: Some(Key::W),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::CTRL,
    };
    assert_eq!(router.route(&[shortcut], true).keys, Keys::default());
    assert_eq!(router.route(&[press(Key::W)], true).keys, Keys::default());
    assert_eq!(
        router.route(&[release(Key::W), press(Key::W)], true).keys.y,
        1
    );
}

#[test]
fn project_snapshot_survives_restart_but_not_a_different_scene() {
    let mut controls = Controls::default();
    controls
        .install_project(1, Path::new("room.yaml"), custom())
        .unwrap();
    controls.panel.use_defaults().unwrap();
    controls.apply().unwrap();
    assert!(controls.hint().contains("A/ArrowLeft left"));
    controls.sync(2, true, Some("room.yaml".into()));
    assert!(controls.hint().contains("J left"));
    controls.sync(3, true, Some("another.yaml".into()));
    assert!(controls.hint().contains("A/ArrowLeft left"));
    controls.sync(4, true, Some("room.yaml".into()));
    assert!(controls.hint().contains("A/ArrowLeft left"));
}

fn live_app() -> (tempfile::TempDir, crate::EditorApp) {
    let temp = tempfile::tempdir().unwrap();
    let scene = temp.path().join("room.scene.yaml");
    let text = include_str!("../../tests/fixtures/room_initial.scene.yaml");
    std::fs::write(&scene, text).unwrap();
    let mut editor = crate::Editor::start(&crate::HostSpec::PreparedRoom {
        scene: scene.clone(),
        text: text.into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor.sync();
    let mut app = crate::EditorApp::new(editor, None);
    let mut bytes = Vec::new();
    custom().save(&mut bytes).unwrap();
    app.install_room_project_controls(&scene, Some(&bytes))
        .unwrap();
    (temp, app)
}

fn frame(app: &mut crate::EditorApp, ctx: &egui::Context, events: Vec<Event>, focused: bool) {
    ctx.begin_pass(egui::RawInput {
        events,
        focused,
        ..Default::default()
    });
    app.route_room_controls(ctx, focused);
    ctx.end_pass().textures_delta.clear();
    app.editor.sync();
}

fn wait(app: &mut crate::EditorApp, predicate: impl Fn(&crate::EditorApp) -> bool) {
    for _ in 0..150 {
        app.editor.sync();
        if predicate(app) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!(
        "Room host did not reach expected state; phase {:?}, hint {}",
        app.editor.input_phase(),
        app.editor.input_hint()
    );
}

fn position(app: &crate::EditorApp) -> orr_view::Vec3 {
    use orr_sample::room_game::{RoomActor, PLAYER};
    let snapshot = app.editor.snapshot().unwrap();
    let view = snapshot.predicted();
    orr_sample::room_view::items(view)
        .into_iter()
        .find(|item| {
            view.get::<RoomActor>(item.entity)
                .is_some_and(|actor| actor.kind == PLAYER)
        })
        .unwrap()
        .transform
        .pos
}

#[test]
fn actual_room_host_uses_physical_project_keys_and_releases_on_interrupted_focus() {
    use orr_sample::room_game::RoomRun;
    let (_temp, mut app) = live_app();
    let ctx = egui::Context::default();
    app.editor.play();
    wait(&mut app, |a| a.editor.can_take_control());
    assert!(app.editor.take_control(0, true));
    wait(&mut app, |a| a.editor.input_phase() == Phase::Active);
    frame(&mut app, &ctx, vec![], true); // neutral arms existing Room interact lease
    frame(&mut app, &ctx, vec![press(Key::Tab)], true);
    wait(&mut app, |a| {
        a.editor
            .snapshot()
            .unwrap()
            .predicted()
            .singleton::<RoomRun>()
            .key_collected
            == 1
    });
    frame(&mut app, &ctx, vec![release(Key::Tab)], true);
    let before = position(&app);
    frame(
        &mut app,
        &ctx,
        vec![event(Key::Escape, Key::I, true, false), press(Key::L)],
        true,
    );
    wait(&mut app, |a| {
        let p = position(a);
        p.x > before.x && p.z < before.z
    });
    assert_eq!(app.editor.input_phase(), Phase::Active);
    // A loss and return in the same input batch must still release the lease.
    frame(
        &mut app,
        &ctx,
        vec![Event::WindowFocused(false), Event::WindowFocused(true)],
        true,
    );
    wait(&mut app, |a| {
        a.editor.input_phase() == Phase::Off && !a.editor.input_cleanup_pending()
    });
    let stopped = position(&app);
    assert!(app.editor.take_control(0, true));
    wait(&mut app, |a| a.editor.input_phase() == Phase::Active);
    frame(&mut app, &ctx, vec![], true);
    std::thread::sleep(std::time::Duration::from_millis(50));
    app.editor.sync();
    assert_eq!(
        position(&app),
        stopped,
        "held physical keys must not resume after focus reclaim"
    );
    frame(
        &mut app,
        &ctx,
        vec![release(Key::I), release(Key::L), press(Key::L)],
        true,
    );
    wait(&mut app, |a| position(a).x > stopped.x);
    frame(
        &mut app,
        &ctx,
        vec![event(Key::I, Key::Escape, true, false)],
        true,
    );
    wait(&mut app, |a| a.editor.input_phase() == Phase::Off);
}

#[test]
fn actual_restart_restores_pinned_map_without_rereading_disk() {
    let (_temp, mut app) = live_app();
    app.ui.room_controls.panel.use_defaults().unwrap();
    app.ui.room_controls.apply().unwrap();
    assert!(app.ui.room_controls.hint().contains("A/ArrowLeft left"));
    assert!(app.editor.restart());
    app.editor.sync();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![], false);
    assert!(app.ui.room_controls.hint().contains("J left"));
    assert_eq!(
        app.ui.room_controls.panel.document(),
        &RoomControls::default_map()
    );
    assert!(app
        .install_room_project_controls(Path::new("wrong.yaml"), None)
        .is_err());
    assert!(app.ui.room_controls.hint().contains("J left"));
}

#[test]
fn production_widgets_require_explicit_use_for_play_and_disable_in_play() {
    use egui::accesskit::Role;
    use egui_kittest::{
        kittest::{NodeT, Queryable},
        Harness,
    };
    let (_temp, app) = live_app();
    let mut h = Harness::builder()
        .with_size([1100.0, 1600.0])
        .build_ui_state(
            |ui, app: &mut crate::EditorApp| {
                app.show_room_controls(ui);
            },
            app,
        );
    h.run_steps(3);
    h.get_by_label("Room controls for Play").click();
    h.run_steps(4);
    assert!(h.state().ui.room_controls.hint().contains("J left"));
    // Accepted panel changes still do not change the live map until this button.
    h.state_mut().ui.room_controls.panel.use_defaults().unwrap();
    h.run_steps(2);
    assert!(h.state().ui.room_controls.hint().contains("J left"));
    h.get_by_role_and_label(Role::Button, "Use for Play")
        .click();
    h.run_steps(3);
    assert!(h
        .state()
        .ui
        .room_controls
        .hint()
        .contains("A/ArrowLeft left"));
    h.state_mut().editor.play();
    wait(h.state_mut(), |a| a.editor.can_take_control());
    h.run_steps(3);
    assert!(h
        .get_by_role_and_label(Role::Button, "Use for Play")
        .accesskit_node()
        .is_disabled());
}

#[test]
fn full_editor_ui_does_not_treat_logical_escape_as_physical_release() {
    use egui_kittest::Harness;
    let (_temp, mut app) = live_app();
    app.editor.play();
    wait(&mut app, |a| a.editor.can_take_control());
    assert!(app.editor.take_control(0, true));
    wait(&mut app, |a| a.editor.input_phase() == Phase::Active);
    let mut h = Harness::builder()
        .with_size([1200.0, 1000.0])
        .build_ui_state(
            |ui, (app, first): &mut (crate::EditorApp, bool)| {
                if *first {
                    ui.ctx()
                        .memory_mut(|m| m.request_focus(egui::Id::new("room_keyboard_viewport")));
                    *first = false;
                }
                app.editor_ui(ui);
            },
            (app, true),
        );
    h.run_steps(3);
    assert_eq!(h.state().0.editor.input_phase(), Phase::Active);
    h.event(event(Key::Escape, Key::I, true, false));
    h.run_steps(3);
    assert_eq!(h.state().0.editor.input_phase(), Phase::Active);
    h.event(event(Key::I, Key::Escape, true, false));
    h.run_steps(3);
    wait(&mut h.state_mut().0, |a| {
        a.editor.input_phase() == Phase::Off
    });
}

#[test]
fn missing_physical_release_cannot_leave_a_remapped_key_held() {
    let mut router = Router::default();
    router.install(&custom()).unwrap();
    assert_eq!(
        router
            .route(&[event(Key::Escape, Key::I, true, false)], true)
            .keys
            .y,
        1
    );
    let missing = Event::Key {
        key: Key::Escape,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    let frame = router.route(&[missing], true);
    assert!(frame.release && frame.missing_identity);
    assert_eq!(frame.keys, Keys::default());
    assert_eq!(router.route(&[], true).keys, Keys::default());
    assert_eq!(router.route(&[press(Key::I)], true).keys, Keys::default());
    assert_eq!(
        router.route(&[release(Key::I), press(Key::I)], true).keys.y,
        1
    );
}

#[test]
fn public_project_install_rejects_play_lease_and_pending_draft() {
    let (_temp, mut app) = live_app();
    let path = app.editor.path().unwrap();
    app.ui
        .room_controls
        .panel
        .replace_draft(vec![Button::Keyboard {
            key: PhysicalKey::X,
        }])
        .unwrap();
    assert!(app.install_room_project_controls(&path, None).is_err());
    assert!(app.ui.room_controls.panel.has_pending_draft());
    assert!(app.ui.room_controls.hint().contains("J left"));
    app.ui.room_controls.panel.discard_draft();
    app.editor.play();
    wait(&mut app, |a| a.editor.can_take_control());
    assert!(app.install_room_project_controls(&path, None).is_err());
    assert!(app.editor.take_control(0, true));
    wait(&mut app, |a| a.editor.input_phase() == Phase::Active);
    assert!(app.install_room_project_controls(&path, None).is_err());
    assert!(app.ui.room_controls.hint().contains("J left"));
}

#[test]
fn fresh_cli_bootstrap_accepts_preadmitted_controls_after_play_ticks_only_once() {
    let temp = tempfile::tempdir().unwrap();
    let scene = temp.path().join("room.scene.yaml");
    let text = include_str!("../../tests/fixtures/room_initial.scene.yaml");
    std::fs::write(&scene, text).unwrap();
    let mut editor = crate::Editor::start(&crate::HostSpec::PreparedRoom {
        scene: scene.clone(),
        text: text.into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor.step(1);
    editor.sync();
    let mut app = crate::EditorApp::new(editor, None);
    assert_eq!(app.editor.mode(), Mode::Play);
    let mut bytes = Vec::new();
    custom().save(&mut bytes).unwrap();
    app.install_room_project_controls(&scene, Some(&bytes))
        .unwrap();
    assert!(app.ui.room_controls.hint().contains("J left"));
    assert!(app.install_room_project_controls(&scene, None).is_err());
    assert!(app.ui.room_controls.hint().contains("J left"));
}
