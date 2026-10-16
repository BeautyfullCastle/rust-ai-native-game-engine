#![cfg(feature = "input-actions")]
use orr_bridge::{Bridge, InProc, PlayerSlot};
use orr_input::{Button, Key};
use orr_sample::arena_input::{default_map, ArenaControls};
use orr_sample::arena_view::{arena_bridge_config, loopback_pair, Keys, Loopback};
use std::time::Duration;
fn key(key: Key) -> Button {
    Button::Keyboard { key }
}
#[test]
fn canonical_defaults_and_multiple_bindings() {
    let mut controls = ArenaControls::new(default_map()).unwrap();
    controls.button(key(Key::A), true, false, false);
    controls.button(key(Key::ArrowLeft), true, false, false);
    controls.button(key(Key::A), false, false, false);
    controls.button(key(Key::Space), true, false, false);
    let expected = Keys {
        left: true,
        fire: true,
        ..Keys::default()
    }
    .to_input();
    assert_eq!(
        bytemuck::bytes_of(&controls.keys().to_input()),
        bytemuck::bytes_of(&expected)
    );
}
#[test]
fn pause_resume_focus_and_captured_mouse_require_fresh_input() {
    let mut map = default_map();
    map.actions
        .iter_mut()
        .find(|a| a.name == "fire")
        .unwrap()
        .bindings
        .push(Button::Mouse { button: 0 });
    let mut controls = ArenaControls::new(map).unwrap();
    controls.button(key(Key::D), true, false, false);
    assert!(
        controls
            .button(key(Key::P), true, false, false)
            .pause_changed
    );
    assert!(controls.paused());
    assert_eq!(controls.keys(), Keys::default());
    assert!(
        !controls
            .button(key(Key::P), true, true, false)
            .pause_changed
    );
    controls.button(key(Key::P), false, false, false);
    controls.button(key(Key::P), true, false, false);
    assert!(!controls.paused());
    assert_eq!(controls.keys(), Keys::default());
    controls.button(key(Key::D), true, true, false);
    assert_eq!(controls.keys(), Keys::default());
    controls.button(Button::Mouse { button: 0 }, true, false, true);
    assert!(!controls.keys().fire);
    controls.set_ui_capture(true);
    controls.button(key(Key::D), true, false, false);
    controls.set_ui_capture(false);
    assert_eq!(controls.keys(), Keys::default());
    controls.button(key(Key::D), true, false, false);
    controls.set_focused(false);
    controls.set_focused(true);
    assert_eq!(controls.keys(), Keys::default());
}
#[test]
fn reloaded_remap_drives_real_bridge_identically_to_canonical_inputs() {
    let mut map = default_map();
    map.actions
        .iter_mut()
        .find(|a| a.name == "right")
        .unwrap()
        .bindings = vec![key(Key::L)];
    let mut bytes = Vec::new();
    map.save(&mut bytes).unwrap();
    let mut controls =
        ArenaControls::new(orr_input::ActionMap::load(bytes.as_slice()).unwrap()).unwrap();
    let make = || InProc::new(loopback_pair(Loopback::default()), arena_bridge_config());
    let mut mapped = make();
    let mut canonical = make();
    for tick in 0..120 {
        if tick == 0 {
            controls.button(key(Key::L), true, false, false);
        }
        if tick == 60 {
            controls.button(key(Key::L), false, false, false);
        }
        mapped
            .set_input(PlayerSlot(0), controls.keys().to_input())
            .unwrap();
        canonical
            .set_input(
                PlayerSlot(0),
                Keys {
                    right: tick < 60,
                    ..Keys::default()
                }
                .to_input(),
            )
            .unwrap();
        mapped.update(Duration::from_nanos(16_666_667));
        canonical.update(Duration::from_nanos(16_666_667));
        let a = mapped.poll_view();
        let b = canonical.poll_view();
        assert_eq!(
            a.snapshot.as_ref().unwrap().predicted().checksum(),
            b.snapshot.as_ref().unwrap().predicted().checksum()
        );
    }
}
#[test]
fn focus_release_is_published_without_a_render_frame() {
    let mut controls = ArenaControls::new(default_map()).unwrap();
    let make = || InProc::new(loopback_pair(Loopback::default()), arena_bridge_config());
    let mut mapped = make();
    let mut neutral = make();
    controls.button(key(Key::D), true, false, false);
    controls.button(key(Key::Space), true, false, false);
    controls.publish(&mut mapped).unwrap();
    // Focus event immediately publishes a release. Only the sim runs afterward;
    // the renderer has not sampled controls, polled a view, or produced a frame.
    controls.set_focused(false);
    controls.publish(&mut mapped).unwrap();
    for _ in 0..60 {
        mapped.update(Duration::from_nanos(16_666_667));
        neutral.update(Duration::from_nanos(16_666_667));
    }
    assert_eq!(
        mapped.snapshot().unwrap().predicted().checksum(),
        neutral.snapshot().unwrap().predicted().checksum()
    );
}
