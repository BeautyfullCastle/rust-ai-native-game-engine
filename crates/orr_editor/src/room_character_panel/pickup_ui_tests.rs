use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
mod asset {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/room_motion.rs"
    ));
}
fn fixture() -> Fixture {
    let mut f = Fixture::new();
    asset::install(&f.root, &f._temp.path().join("pickup model source"));
    f.editor.select(Some(crate::Target::Guid(
        orr_reflect::Guid::parse(&f.panel.document.player).unwrap(),
    )));
    f.models.package = asset::PACKAGE.into();
    f.models.asset = asset::ASSET.into();
    f.models.kind = crate::model_bindings::ModelKind::Animated;
    f.models.animation = crate::model_bindings::AnimationDescriptor {
        clip_index: f.panel.document.searching,
        playback: crate::model_bindings::PlaybackMode::Loop,
    };
    f.models.assign(&f.editor).unwrap();
    f
}
fn harness(f: Fixture) -> Harness<'static, Fixture> {
    let mut h = Harness::builder()
        .with_size([1600.0, 2400.0])
        .build_ui_state(
            |ui, f: &mut Fixture| {
                f.panel.show(ui, &mut f.editor, &mut f.models);
            },
            f,
        );
    h.run_steps(2);
    h.get_by_label("Room character").click_accesskit();
    h.run_steps(2);
    h
}
fn click(h: &mut Harness<'_, Fixture>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(2);
}
fn pickup(doc: &Document) -> orr_sample::room_character::Pickup {
    doc.motion.as_ref().unwrap().pickup.unwrap()
}
#[test]
fn actual_pickup_optin_catalog_rate_apply_history_pair_save_and_reopen() {
    let f = fixture();
    let files = f.pair_bytes();
    let original = f.panel.document.clone();
    let checksum = f.editor.checksum();
    let mut h = harness(f);
    click(&mut h, "Play key pickup clip once");
    assert_eq!(h.state().panel.candidate.schema, 6);
    assert_eq!(h.state().panel.document, original);
    assert_eq!(h.state().pair_bytes(), files);
    assert!(h.state().panel.undo.is_empty());
    click(&mut h, "Key pickup clip");
    click(&mut h, "Key pickup clip 3: idle (1.00s)");
    click(&mut h, "Key pickup speed");
    click(&mut h, "Key pickup speed 1/2x");
    let expected = h.state().panel.candidate.clone();
    assert_eq!(pickup(&expected).clip, asset::IDLE);
    assert_eq!(pickup(&expected).speed, PlaybackRate::Half);
    assert!(
        h.get_by_label("Save character and model bindings")
            .accesskit_node()
            .is_disabled()
    );
    click(&mut h, "Apply character settings");
    assert!(h.state().panel.error.is_none());
    assert_eq!(h.state().panel.document, expected);
    assert_eq!(h.state().editor.checksum(), checksum);
    click(&mut h, "Undo character edit");
    assert_eq!(h.state().panel.document, original);
    click(&mut h, "Redo character edit");
    assert_eq!(h.state().panel.document, expected);
    assert_eq!(h.state().pair_bytes(), files);
    click(&mut h, "Save character and model bindings");
    assert!(h.state().panel.error.is_none());
    h.state().reopen(&expected);
}
#[test]
fn legacy_controls_preserve_pickup_and_explicit_optout_keeps_idle_policy() {
    let mut h = harness(fixture());
    click(&mut h, "Play key pickup clip once");
    let initial = pickup(&h.state().panel.candidate);
    assert!(
        h.get_by_label("Use idle clips")
            .accesskit_node()
            .is_disabled()
    );
    click(&mut h, "Searching speed");
    click(&mut h, "Searching speed 2x");
    assert_eq!(h.state().panel.candidate.schema, 6);
    assert_eq!(pickup(&h.state().panel.candidate), initial);
    click(&mut h, "Play escaped clip once");
    assert_eq!(h.state().panel.candidate.schema, 6);
    assert_eq!(pickup(&h.state().panel.candidate), initial);
    let (target_node, target_tree) = h
        .get_by_role_and_label(egui::accesskit::Role::Slider, "Crossfade ticks")
        .accesskit_node()
        .locate();
    h.event(egui::Event::AccessKitActionRequest(
        egui::accesskit::ActionRequest {
            target_node,
            target_tree,
            action: egui::accesskit::Action::SetValue,
            data: Some(egui::accesskit::ActionData::NumericValue(4.0)),
        },
    ));
    h.run_steps(2);
    assert_eq!(h.state().panel.candidate.crossfade_ticks, Some(4));
    assert_eq!(h.state().panel.candidate.schema, 6);
    assert_eq!(pickup(&h.state().panel.candidate), initial);
    let motion = h.state().panel.candidate.motion.unwrap();
    click(&mut h, "Play key pickup clip once");
    assert_eq!(h.state().panel.candidate.schema, 5);
    let after = h.state().panel.candidate.motion.unwrap();
    assert!(after.pickup.is_none());
    assert_eq!(after.searching_idle, motion.searching_idle);
    assert_eq!(after.carrying_idle, motion.carrying_idle);
    assert_eq!(after.escaped_once, motion.escaped_once);
    assert!(
        !h.get_by_label("Use idle clips")
            .accesskit_node()
            .is_disabled()
    );
    assert!(h.state().panel.error.is_none());
}
#[test]
fn invalid_pickup_mapping_rejects_apply_and_stale_source_disables_optin_without_pair_writes() {
    let mut h = harness(fixture());
    let files = h.state().pair_bytes();
    click(&mut h, "Play key pickup clip once");
    let original = h.state().panel.document.clone();
    let bindings = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    h.state_mut()
        .panel
        .candidate
        .motion
        .as_mut()
        .unwrap()
        .pickup
        .as_mut()
        .unwrap()
        .clip = 31;
    click(&mut h, "Apply character settings");
    assert!(h.state().panel.error.is_some());
    assert_eq!(h.state().panel.document, original);
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &bindings
    );
    assert_eq!(h.state().pair_bytes(), files);
    // A different installed source document retires the panel rather than rebinding.
    let f = h.state_mut();
    let mut replacement = original.clone();
    std::mem::swap(&mut replacement.searching, &mut replacement.carrying);
    f.editor.install_room_character(replacement).unwrap();
    h.run_steps(2);
    assert!(
        h.get_by_label("Play key pickup clip once")
            .accesskit_node()
            .is_disabled()
    );
    assert_eq!(h.state().pair_bytes(), files);
}
