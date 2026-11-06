use super::*;
use crate::model_bindings;
const COOKED: &str = "model.animated.json";
#[test]
fn cooked_animation_samples_assigns_saves_and_reopens_with_source_hidden() {
    let expected = model().to_bytes().unwrap();
    let fixture = Fixture::with_asset(model(), &["models", "animation"], COOKED);
    fs::rename(
        &fixture.source,
        fixture.source.with_file_name("source-hidden"),
    )
    .unwrap();
    let mut state = fixture.state();
    state.sample(1, 1.0).unwrap();
    let ready = state.ready().unwrap();
    assert_eq!(ready.animation.as_ref().unwrap().clip_index, 1);
    let descriptor = model_bindings::AnimationDescriptor {
        clip_index: 0,
        playback: model_bindings::PlaybackMode::Loop,
    };
    let binding = model_bindings::Binding::from_animated_asset(
        PACKAGE.into(),
        COOKED.into(),
        &ready.loaded,
        descriptor,
        model_bindings::LocalTransform::default(),
    )
    .unwrap();
    let path = fixture.root.join("scene.models.json");
    let mut bindings =
        model_bindings::Bindings::create(path.clone(), "scene.scene.yaml".into(), ".".into())
            .unwrap();
    let guid = orr_reflect::Guid::parse("e_00000001").unwrap();
    let key = guid.to_string();
    bindings
        .assign_validated(&[guid], &binding, &ready.loaded)
        .unwrap();
    bindings.save().unwrap();
    let reopened = model_bindings::Bindings::open(path).unwrap();
    let saved = &reopened.document().bindings[&key];
    assert_eq!(saved.animation, Some(descriptor));
    let project = Project::open(
        reopened.project_root().unwrap(),
        orr_sample::room_project::compiled_runtime(),
    )
    .unwrap();
    let loaded = model_bindings::load_binding_from_project(&project, saved).unwrap();
    assert_eq!(loaded.kind(), ModelKind::Animated);
    assert_eq!(
        loaded.animated_model().unwrap().to_bytes().unwrap(),
        expected
    );
    assert!(!fixture.source.exists());
}
#[test]
fn cooked_animation_requires_declared_capability_and_correct_kind() {
    let fixture = Fixture::with_asset(model(), &["models"], COOKED);
    assert!(load(&fixture.request).is_err());
    let fixture = Fixture::with_asset(model(), &["models", "animation"], COOKED);
    let mut request = fixture.request.clone();
    request.kind = ModelKind::Static;
    assert!(load(&request).is_err());
    request = fixture.request.clone();
    request.digest = "f".repeat(64);
    assert!(load(&request).is_err());
}
#[test]
fn invalid_cooked_animation_samples_keep_last_accepted_pose() {
    let fixture = Fixture::with_asset(model(), &["models", "animation"], COOKED);
    let mut state = fixture.state();
    state.sample(1, 1.0).unwrap();
    let before = state.ready().unwrap().animation.clone();
    let bounds = state.ready().unwrap().bounds;
    for (clip, time) in [(u32::MAX, 0.0), (0, -1.0), (0, f32::NAN)] {
        assert!(state.sample(clip, time).is_err());
        assert_eq!(state.ready().unwrap().animation, before);
        assert_eq!(state.ready().unwrap().bounds, bounds);
    }
}
