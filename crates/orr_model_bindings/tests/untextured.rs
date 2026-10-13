//! Verified installed-package and persisted binding proof; no renderer dependency.
#[path = "../../orr_model/tests/common/untextured_fixture.rs"]
mod fixture;
use orr_model_bindings::model_bindings::{self, Binding, LocalTransform, ModelKind};
use orr_package::{Project, Runtime};
use sha2::{Digest, Sha256};
fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}
fn exercise(animated: bool) {
    let temp = fixture::Scratch::new(if animated {
        "bindings-animation"
    } else {
        "bindings-static"
    });
    let source = temp.0.join("source");
    let root = temp.0.join("project");
    std::fs::create_dir(&root).unwrap();
    fixture::package(&source, animated);
    let source_bytes = std::fs::read(source.join("factor.glb")).unwrap();
    let installer =
        Project::open_for_install(&root, Runtime::content_only().engine_version).unwrap();
    installer.install(std::slice::from_ref(&source)).unwrap();
    let hidden = temp.0.join("hidden-source");
    std::fs::rename(&source, &hidden).unwrap();
    assert!(model_bindings::list_assets(&root)
        .unwrap()
        .iter()
        .any(|a| a.asset == "factor.glb"));
    let kind = if animated {
        ModelKind::Animated
    } else {
        ModelKind::Static
    };
    let loaded =
        model_bindings::load_asset_for_kind(&root, "factor-fixture", "factor.glb", kind).unwrap();
    assert_eq!(loaded.source_hash(), hex(&source_bytes));
    let binding = if animated {
        Binding::from_animated_asset(
            "factor-fixture".into(),
            "factor.glb".into(),
            &loaded,
            model_bindings::AnimationDescriptor {
                clip_index: 0,
                playback: model_bindings::PlaybackMode::Loop,
            },
            LocalTransform::default(),
        )
        .unwrap()
    } else {
        Binding::from_asset(
            "factor-fixture".into(),
            "factor.glb".into(),
            &loaded,
            LocalTransform::default(),
        )
        .unwrap()
    };
    binding.validate(&loaded).unwrap();
    let document = model_bindings::Document {
        version: 2,
        scene: "scene.yaml".into(),
        project: ".".into(),
        bindings: std::collections::BTreeMap::from([("e_00000001".into(), binding.clone())]),
    };
    document.validate().unwrap();
    let bytes = serde_json::to_vec(&document).unwrap();
    let path = root.join("scene.models.json");
    std::fs::write(&path, &bytes).unwrap();
    let reopened_document: model_bindings::Document =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    reopened_document.validate().unwrap();
    assert_eq!(document, reopened_document);
    let reopened = reopened_document.bindings["e_00000001"].clone();
    assert_eq!(binding, reopened);
    let reloaded = model_bindings::load_binding(&root, &reopened).unwrap();
    assert_eq!(reloaded.source_hash(), loaded.source_hash());
    assert_eq!(reloaded.package_digest(), loaded.package_digest());
    if !animated {
        let m = reloaded.static_model().unwrap();
        let cooked = m.to_bytes().unwrap();
        assert_eq!(
            orr_model::StaticModel::from_bytes(&cooked)
                .unwrap()
                .to_bytes()
                .unwrap(),
            cooked
        );
        assert_eq!(m.source().images.len(), 1);
    }
    #[cfg(feature = "animated-models")]
    if animated {
        let m = reloaded.animated_model().unwrap();
        let cooked = m.to_bytes().unwrap();
        assert_eq!(
            orr_model::animation::AnimatedModel::from_bytes(&cooked)
                .unwrap()
                .to_bytes()
                .unwrap(),
            cooked
        );
        assert_eq!(m.source().images.len(), 1);
    }
    installer.remove("factor-fixture").unwrap();
    let before = installer.list().unwrap();
    let lock_before = std::fs::read(root.join("orr.packages.lock.json")).unwrap();
    assert!(model_bindings::load_binding(&root, &reopened).is_err());
    assert_eq!(
        serde_json::to_vec(&installer.list().unwrap()).unwrap(),
        serde_json::to_vec(&before).unwrap()
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(
        std::fs::read(root.join("orr.packages.lock.json")).unwrap(),
        lock_before
    );
    installer.install(std::slice::from_ref(&hidden)).unwrap();
    let restored = model_bindings::load_binding(&root, &reopened).unwrap();
    assert_eq!(restored.package_digest(), loaded.package_digest());
    assert_eq!(
        std::fs::read(hidden.join("factor.glb")).unwrap(),
        source_bytes
    );
}
#[test]
fn installed_static_factor_binding_survives_reload_removal_and_reinstall() {
    exercise(false);
}
#[cfg(feature = "animated-models")]
#[test]
fn installed_animated_factor_binding_survives_reload_removal_and_reinstall() {
    exercise(true);
}
