use super::*;
const COOKED: &str = "model.model.json";

#[test]
fn cooked_package_preview_assignment_save_and_reopen_use_installed_bytes() {
    let bytes = triangle().to_bytes().unwrap();
    let fixture = Fixture::with_asset(COOKED, &bytes);
    fs::rename(
        &fixture.source,
        fixture.source.with_file_name("source-hidden"),
    )
    .unwrap();
    let ready = load(&fixture.request).unwrap();
    assert_eq!(
        ready.loaded.static_model().unwrap().to_bytes().unwrap(),
        bytes
    );
    let binding = model_bindings::Binding::from_asset(
        PACKAGE.into(),
        COOKED.into(),
        &ready.loaded,
        model_bindings::LocalTransform::default(),
    )
    .unwrap();
    let path = fixture.root.join("scene.models.json");
    let mut bindings =
        model_bindings::Bindings::create(path.clone(), "scene.scene.yaml".into(), ".".into())
            .unwrap();
    let guid = orr_reflect::Guid::parse("e_00000001").unwrap();
    bindings
        .assign_validated(&[guid], &binding, &ready.loaded)
        .unwrap();
    bindings.save().unwrap();
    let reopened = model_bindings::Bindings::open(path).unwrap();
    assert_eq!(reopened.document(), bindings.document());
    let project = Project::open(
        reopened.project_root().unwrap(),
        orr_sample::room_project::compiled_runtime(),
    )
    .unwrap();
    let saved = &reopened.document().bindings[&guid.to_string()];
    let loaded = model_bindings::load_binding_from_project(&project, saved).unwrap();
    assert_eq!(loaded.static_model().unwrap().to_bytes().unwrap(), bytes);
    assert!(!fixture.source.exists());
}

#[test]
fn declared_cooked_suffix_still_requires_valid_model_bytes() {
    for bytes in [b"invalid model".as_slice(), b"{}".as_slice()] {
        let fixture = Fixture::with_asset(COOKED, bytes);
        assert!(load(&fixture.request)
            .unwrap_err_message()
            .contains("model"));
    }
}

#[test]
fn cooked_request_retains_capability_declaration_digest_and_kind_admission() {
    let fixture = Fixture::with_asset(COOKED, &triangle().to_bytes().unwrap());
    let lock = fixture.project().list().unwrap();
    assert!(check_identity(&fixture.request, &lock).is_ok());
    let mut missing = lock.clone();
    missing
        .packages
        .get_mut(PACKAGE)
        .unwrap()
        .manifest
        .capabilities
        .remove("models");
    assert!(check_identity(&fixture.request, &missing).is_err());
    let mut undeclared = lock.clone();
    undeclared
        .packages
        .get_mut(PACKAGE)
        .unwrap()
        .manifest
        .files
        .remove(COOKED);
    assert!(check_identity(&fixture.request, &undeclared).is_err());
    let mut stale = fixture.request.clone();
    stale.digest = "f".repeat(64);
    assert!(check_identity(&stale, &lock).is_err());
    let mut animated = fixture.request.clone();
    animated.kind = ModelKind::Animated;
    let mut both = lock;
    both.packages
        .get_mut(PACKAGE)
        .unwrap()
        .manifest
        .capabilities
        .insert("animation".into());
    assert!(check_identity(&animated, &both).is_err());
}
