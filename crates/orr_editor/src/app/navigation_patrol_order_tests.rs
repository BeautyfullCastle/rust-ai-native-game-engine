//! CPU App input ownership checks; no window or GPU claim.
use super::*;
use egui_kittest::kittest::{NodeT, Queryable};
#[test]
fn structural_buttons_follow_actual_picker_ownership_through_cancelled_release() {
    let (_root, mut h) = super::patrol_tests::setup();
    let history = h.state().editor.history().clone();
    let frame = h.state().editor.snapshot().unwrap().predicted().checksum();
    let bytes = h.state().terrain.document.bytes().unwrap().to_vec();
    let label = "Move patrol waypoint 2 up";
    assert!(!h.get_by_label(label).accesskit_node().is_disabled());
    h.get_by_label("Pick terrain patrol waypoint 1")
        .click_accesskit();
    h.run_steps(3);
    assert_eq!(
        h.state().navigation_endpoint.armed(),
        Some(Endpoint::Patrol(0))
    );
    assert!(h.get_by_label(label).accesskit_node().is_disabled());
    let pos = egui::pos2(400.0, 400.0);
    h.input_mut().focused = true;
    h.input_mut().events.extend([
        egui::Event::WindowFocused(false),
        egui::Event::WindowFocused(true),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    h.step();
    assert!(h.state().navigation_endpoint.armed().is_none());
    assert!(h.state().navigation_endpoint_owns_input());
    assert!(h.get_by_label(label).accesskit_node().is_disabled());
    h.input_mut().events.push(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    assert!(h.state().navigation_endpoint_owns_input());
    assert!(h.get_by_label(label).accesskit_node().is_disabled());
    h.run_steps(2);
    assert!(!h.state().navigation_endpoint_owns_input());
    assert!(!h.get_by_label(label).accesskit_node().is_disabled());
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(
        h.state().editor.snapshot().unwrap().predicted().checksum(),
        frame
    );
    assert_eq!(h.state().terrain.document.bytes().unwrap(), bytes);
}
