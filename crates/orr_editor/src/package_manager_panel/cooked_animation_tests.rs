use super::*;
use egui_kittest::{kittest::Queryable, Harness};
use std::collections::BTreeMap;
#[test]
fn cooked_animation_suffix_is_typed_and_compiled_capability_gated() {
    use crate::package_model_preview::supports_asset;
    for asset in ["model.animated.json", "clips/WALK.ANIMATED.JSON"] {
        assert!(supports_asset(ModelKind::Animated, asset));
        assert!(!supports_asset(ModelKind::Static, asset));
    }
    for asset in [
        "settings.json",
        "walk.sequence.json",
        "model.animated.json.bak",
        "model.model.json",
    ] {
        assert!(!supports_asset(ModelKind::Animated, asset));
    }
    let caps = ["models".into(), "animation".into()].into();
    assert_eq!(
        preview_kind_allowed(ModelKind::Animated, &caps, "model.animated.json"),
        cfg!(feature = "room-animated-models")
    );
    assert!(!preview_kind_allowed(
        ModelKind::Animated,
        &["models".into()].into(),
        "model.animated.json"
    ));
}
#[test]
fn production_browser_exposes_only_explicit_compiled_cooked_animation_actions() {
    let manifest = orr_package::Manifest {
        schema: 1,
        name: "clips".into(),
        version: "1.0.0".into(),
        engine: "*".into(),
        capabilities: ["models".into(), "animation".into()].into(),
        dependencies: BTreeMap::new(),
        files: [
            "model.animated.json",
            "prop.model.json",
            "walk.sequence.json",
        ]
        .map(String::from)
        .into(),
    };
    let package = orr_package::LockedPackage {
        files: manifest
            .files
            .iter()
            .map(|p| (p.clone(), "a".repeat(64)))
            .collect(),
        manifest,
        digest: "b".repeat(64),
    };
    let lock = Lock {
        schema: 1,
        direct: BTreeMap::new(),
        packages: BTreeMap::from([("clips".into(), package)]),
    };
    let panel = Panel {
        root: Some(PathBuf::from("/project")),
        source: "/source".into(),
        source_identity: "/source".into(),
        verified: Some(lock.clone()),
        ..Default::default()
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 600.0))
        .build_ui_state(
            |ui, panel: &mut Panel| {
                model_asset_buttons(
                    ui,
                    ModelBrowser {
                        lock: panel.verified.as_ref().unwrap(),
                        filter: &mut panel.model_filter,
                        preview_request: &mut panel.preview_request,
                    },
                    true,
                    &mut panel.model_request,
                    panel.root.as_ref(),
                    &panel.source,
                    (panel.project_generation, panel.source_generation),
                );
            },
            panel,
        );
    harness.run_steps(2);
    assert!(harness
        .query_by_label("Use model clips/model.animated.json")
        .is_none());
    assert!(harness
        .query_by_label("Use animated model clips/prop.model.json")
        .is_none());
    if cfg!(feature = "room-animated-models") {
        assert!(harness
            .query_by_label("Showing 2 of 2 verified model assets")
            .is_some());
        harness
            .get_by_label("Preview animated model clips/model.animated.json")
            .click();
        harness.run_steps(2);
        let request = harness.state_mut().take_preview_request().unwrap();
        assert_eq!(request.kind, ModelKind::Animated);
        assert_eq!(request.asset_path, "model.animated.json");
        harness
            .get_by_label("Use animated model clips/model.animated.json")
            .click();
        harness.run_steps(2);
        assert_eq!(
            harness.state_mut().take_model_asset_request(),
            Some((
                "clips".into(),
                "model.animated.json".into(),
                ModelKind::Animated
            ))
        );
        assert_eq!(harness.state().verified.as_ref(), Some(&lock));
        harness
            .get_by_label("Preview animated model clips/model.animated.json")
            .click();
        harness.run_steps(2);
        harness
            .state_mut()
            .verified
            .as_mut()
            .unwrap()
            .packages
            .get_mut("clips")
            .unwrap()
            .manifest
            .capabilities
            .remove("animation");
        assert!(harness.state_mut().take_preview_request().is_none());
    } else {
        assert!(harness
            .query_by_label("Showing 1 of 1 verified model assets")
            .is_some());
        assert!(harness
            .query_by_label("Use animated model clips/model.animated.json")
            .is_none());
        assert!(harness
            .query_by_label("Preview animated model clips/model.animated.json")
            .is_none());
    }
}
