use super::*;
use egui_kittest::{
    kittest::Queryable,
    Harness,
};
use std::collections::BTreeMap;

#[test]
fn cooked_suffix_is_static_only_and_does_not_admit_arbitrary_json() {
    use crate::package_model_preview::supports_asset;
    for path in [
        "model.model.json",
        "variants/paint.MODEL.JSON",
        "hero.orrmodel",
        "hero.GLB",
    ] {
        assert!(supports_asset(ModelKind::Static, path));
    }
    for path in [
        "settings.json",
        "paint.retexture.json",
        "walk.animated.json",
        "model.model.json.bak",
    ] {
        assert!(!supports_asset(ModelKind::Static, path));
    }
    assert!(!supports_asset(ModelKind::Animated, "model.model.json"));
    assert!(supports_asset(ModelKind::Animated, "hero.glb"));
    let models = ["models".to_owned()].into_iter().collect();
    assert!(preview_kind_allowed(
        ModelKind::Static,
        &models,
        "model.model.json"
    ));
    assert!(!preview_kind_allowed(
        ModelKind::Static,
        &Default::default(),
        "model.model.json"
    ));
}

#[test]
fn production_browser_shows_cooked_static_actions_and_preserves_request_guards() {
    let mut lock = Lock {
        schema: 1,
        direct: BTreeMap::new(),
        packages: BTreeMap::new(),
    };
    let manifest = orr_package::Manifest {
        schema: 1,
        name: "variants".into(),
        version: "1.0.0".into(),
        engine: "*".into(),
        capabilities: ["models".into(), "animation".into()].into(),
        dependencies: BTreeMap::new(),
        files: [
            "model.model.json",
            "legacy.glb",
            "settings.json",
            "paint.retexture.json",
            "walk.animated.json",
        ]
        .map(String::from)
        .into(),
    };
    lock.packages.insert(
        "variants".into(),
        orr_package::LockedPackage {
            files: manifest
                .files
                .iter()
                .map(|p| (p.clone(), "a".repeat(64)))
                .collect(),
            manifest,
            digest: "b".repeat(64),
        },
    );
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
        .query_by_label("Showing 2 of 2 verified model assets")
        .is_some());
    assert!(harness
        .query_by_label("Use animated model variants/model.model.json")
        .is_none());
    assert!(harness
        .query_by_label("Use model variants/settings.json")
        .is_none());
    harness
        .get_by_label("Preview static model variants/model.model.json")
        .click();
    harness.run_steps(2);
    let preview = harness.state_mut().take_preview_request().unwrap();
    assert_eq!(preview.asset_path, "model.model.json");
    assert_eq!(preview.kind, ModelKind::Static);
    harness
        .get_by_label("Use model variants/model.model.json")
        .click();
    harness.run_steps(2);
    assert_eq!(
        harness.state_mut().take_model_asset_request(),
        Some((
            "variants".into(),
            "model.model.json".into(),
            ModelKind::Static
        ))
    );
    assert_eq!(harness.state().verified.as_ref(), Some(&lock));
    harness
        .get_by_label("Preview static model variants/model.model.json")
        .click();
    harness.run_steps(2);
    harness
        .state_mut()
        .verified
        .as_mut()
        .unwrap()
        .packages
        .get_mut("variants")
        .unwrap()
        .manifest
        .capabilities
        .remove("models");
    assert!(harness.state_mut().take_preview_request().is_none());
}

