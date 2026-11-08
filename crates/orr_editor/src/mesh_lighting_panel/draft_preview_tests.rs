use super::*;
fn context() -> Context {
    Context {
        profile: EditorGame::MeshNavigation,
        scene: PathBuf::from("level.scene.yaml"),
        scene_epoch: 7,
        graph_revision: [4; 32],
        host_epoch: 9,
    }
}
fn document() -> Document {
    let mut document = Document::from_lighting(orr_render::Lighting::default()).unwrap();
    document.point_light = Some(orr_render::PointLight {
        position: [1., 7., 3.],
        color: [0.2, 0.4, 0.8],
        intensity: 9.,
        range: 17.,
    });
    document
}
fn enabled() -> State {
    State {
        context: Some(context()),
    }
}
#[test]
fn default_off_never_implicitly_enables_even_for_a_valid_draft() {
    let mut state = State::default();
    assert!(state.sample(Some(context()), Ok(document())).is_none());
    assert!(state.context.is_none());
    let panel = Panel::default();
    assert!(!panel.point_light_preview_enabled());
    assert!(!panel.dirty());
}
#[test]
fn live_valid_changes_return_only_latest_point_settings_without_modifying_documents() {
    let mut state = enabled();
    let original = document();
    let mut edited = original.clone();
    edited.point_light.as_mut().unwrap().position = [-3., 12., 5.];
    edited.point_light.as_mut().unwrap().color = [0.9, 0.1, 0.3];
    edited.point_light.as_mut().unwrap().intensity = 2.;
    edited.point_light.as_mut().unwrap().range = 6.;
    for d in [&original, &edited] {
        assert_eq!(
            state.sample(Some(context()), Ok(d.clone())).unwrap(),
            d.point_light_settings().unwrap()
        );
    }
    assert_eq!(original, document());
    assert!(state.context.is_some());
    let mut disabled = edited;
    disabled.point_light = None;
    assert_eq!(
        state.sample(Some(context()), Ok(disabled)),
        Some(PointLightSettings::default())
    );
    assert!(state.context.is_some());
}
#[test]
fn invalid_light_retires_opt_in_and_valid_value_does_not_revive_it() {
    for candidate in [Err("invalid environment or raw number".into()), {
        let mut d = document();
        d.point_light.as_mut().unwrap().range = f32::NAN;
        Ok(d)
    }] {
        let mut state = enabled();
        assert!(state.sample(Some(context()), candidate).is_none());
        assert!(state.sample(Some(context()), Ok(document())).is_none());
        assert!(state.context.is_none());
    }
}
#[test]
fn every_source_identity_component_and_unavailable_context_irreversibly_retire_preview() {
    let mut changed = Vec::new();
    let mut c = context();
    c.profile = EditorGame::Arena;
    changed.push(c);
    let mut c = context();
    c.scene = PathBuf::from("other.scene.yaml");
    changed.push(c);
    let mut c = context();
    c.scene_epoch += 1;
    changed.push(c);
    let mut c = context();
    c.host_epoch += 1;
    changed.push(c);
    let mut c = context();
    c.graph_revision[0] += 1;
    changed.push(c);
    for next in changed.into_iter().map(Some).chain([None]) {
        let mut state = enabled();
        assert!(state.sample(next, Ok(document())).is_none());
        assert!(state.sample(Some(context()), Ok(document())).is_none());
    }
}
#[test]
fn validator_limits_and_height_bits_are_preserved_for_preview_settings() {
    let mut d = document();
    let light = d.point_light.as_mut().unwrap();
    light.position = [-1e9, -0., 1e9];
    light.color = [0., 1e4, 0.];
    light.intensity = 1e4;
    light.range = 1e-6;
    let mut state = enabled();
    let result = state.sample(Some(context()), Ok(d.clone())).unwrap();
    assert_eq!(
        result.point_light.unwrap().position[1].to_bits(),
        (-0.0f32).to_bits()
    );
    assert_eq!(result, d.point_light_settings().unwrap());
    d.point_light.as_mut().unwrap().position[0] = f32::INFINITY;
    assert!(state.sample(Some(context()), Ok(d)).is_none());
}
