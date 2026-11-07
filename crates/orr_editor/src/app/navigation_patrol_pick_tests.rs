//! Actual CPU widgets + production App adapter; no rendered-pixel/GPU claim.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
fn settle(app: &mut EditorApp) {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        app.editor.sync();
        if app.editor.can_mutate()
            && app.editor.yard_rows_coherent()
            && app.editor.admitted_navigation().admitted
        {
            break;
        }
        assert!(std::time::Instant::now() < end);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
fn setup() -> (tempfile::TempDir, Harness<'static, EditorApp>) {
    let (root, mut app) = super::tests::fixture();
    app.navigation.start = ["-0.5".into(), "-0.5".into()];
    app.navigation.goal = ["0.5".into(), "0.5".into()];
    app.navigation.max_slope = "1".into();
    app.navigation.distance_per_tick = "0.25".into();
    app.navigation
        .build_route(&mut app.editor, &app.terrain)
        .unwrap();
    settle(&mut app);
    assert!(app.editor.save());
    app.navigation.sync_from_editor(&app.editor);
    let mut h = Harness::builder().with_size([1300., 1900.]).build_ui_state(
        |ui, app: &mut EditorApp| {
            app.editor.sync();
            app.navigation_endpoint_sync(ui.ctx());
            if app.ui.viewport_focus {
                app.yard_viewport(ui);
                return;
            }
            if let Some(endpoint) = app.navigation.show(ui, &mut app.editor, &app.terrain) {
                let _ = app.navigation_endpoint_arm(endpoint);
            }
            app.navigation_patrol_request();
            app.navigation_endpoint_validate();
        },
        app,
    );
    h.run_steps(3);
    click(&mut h, "Point navigation");
    click(&mut h, "Terrain waypoint patrol");
    click(&mut h, "Enable terrain waypoint patrol");
    click(&mut h, "Add terrain patrol waypoint");
    (root, h)
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.run_steps(2);
    h.get_by_label(label).click_accesskit();
    h.run_steps(3);
}
fn text(h: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(3);
}
#[test]
fn actual_row_pick_stages_only_then_explicit_apply_admits_chosen_cell_center() {
    let (_root, mut h) = setup();
    let history = h.state().editor.history().clone();
    let before = h.state().editor.snapshot().unwrap().predicted().checksum();
    let terrain = h.state().terrain.document.bytes().unwrap().to_vec();
    click(&mut h, "Pick terrain patrol waypoint 1");
    assert_eq!(
        h.state().navigation_endpoint.armed(),
        Some(Endpoint::Patrol(0))
    );
    let camera = super::tests::camera();
    let pixel = camera.world_to_screen([-0.5, 0., 0.5], (600, 600)).unwrap();
    assert!(
        h.state_mut()
            .navigation_endpoint_pick(&camera, pixel, (600, 600))
    );
    assert!(h.state().navigation_endpoint.armed().is_none());
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(
        h.state().editor.snapshot().unwrap().predicted().checksum(),
        before
    );
    assert_eq!(h.state().terrain.document.bytes().unwrap(), terrain);
    assert!(!h.state().terrain.document.dirty());
    click(&mut h, "Apply terrain waypoint patrol");
    settle(h.state_mut());
    let p = orr_remote::navigation_yard3d::patrol::from_view(
        h.state().editor.snapshot().unwrap().predicted(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        p.config.points[0],
        [FP::from_raw(-32768), FP::from_raw(32768)]
    );
    // Adding a waypoint copies the authored goal; picking row 1 must preserve row 2.
    assert_eq!(p.config.points[1], [FP::from_raw(32768); 2]);
    assert_eq!(p.config.count, 2);
}
#[test]
fn changed_row_generation_and_missing_fresh_frame_retire_without_retargeting() {
    let (_root, mut h) = setup();
    click(&mut h, "Pick terrain patrol waypoint 1");
    assert!(h.state().navigation_endpoint.armed().is_some());
    text(&mut h, "Terrain patrol waypoint 2 X", "0.75");
    assert!(h.state().navigation_endpoint.armed().is_none());
    text(&mut h, "Terrain patrol waypoint 2 X", "0.5");
    assert!(
        h.state().navigation_endpoint.armed().is_none(),
        "A-B-A must not revive retired row identity"
    );
    click(&mut h, "Pick terrain patrol waypoint 1");
    // Production viewport has no GPU in this harness. It must retire the
    // pending row instead of accepting an unseen/stale frame after recovery.
    h.state_mut().ui.viewport_focus = true;
    h.step();
    h.state_mut().ui.viewport_focus = false;
    assert!(h.state().navigation_endpoint.armed().is_none());
    assert!(h.state().navigation_endpoint_owns_input());
    let before = h.state().editor.snapshot().unwrap().predicted().checksum();
    let camera = super::tests::camera();
    assert!(
        h.state_mut()
            .navigation_endpoint_pick(&camera, [300., 300.], (600, 600))
    );
    assert_eq!(
        h.state().editor.snapshot().unwrap().predicted().checksum(),
        before
    );
}
#[test]
fn current_loss_event_and_cancelled_press_remain_owned_through_release() {
    let (_root, mut h) = setup();
    click(&mut h, "Pick terrain patrol waypoint 1");
    let history = h.state().editor.history().clone();
    let before = h.state().editor.snapshot().unwrap().predicted().checksum();
    let at = egui::pos2(400., 400.);
    h.input_mut().focused = true;
    h.input_mut().events.extend([
        egui::Event::WindowFocused(false),
        egui::Event::WindowFocused(true),
        egui::Event::PointerMoved(at),
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    h.step();
    assert!(h.state().navigation_endpoint.armed().is_none());
    assert!(h.state().navigation_endpoint_owns_input());
    h.input_mut().events.push(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    assert!(
        h.state().navigation_endpoint_owns_input(),
        "release must not select an entity"
    );
    h.run_steps(2);
    assert!(!h.state().navigation_endpoint_owns_input());
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(
        h.state().editor.snapshot().unwrap().predicted().checksum(),
        before
    );
}
