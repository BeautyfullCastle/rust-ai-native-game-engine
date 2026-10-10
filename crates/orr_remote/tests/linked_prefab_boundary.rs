#![cfg(all(feature = "collect-dodge", not(feature = "linked-prefabs")))]
use orr_sample::{collect_game::CollectDodgeV1, collect_project};
use orr_sim::Simulation;

#[test]
fn unsupported_consumers_reject_even_when_reflect_parser_is_unified() {
    let text = include_str!("linked_prefab_boundary.scene.yaml");
    let parsed = orr_reflect::Scene::parse(text, &collect_project::types());
    if std::env::var_os("ORR_REQUIRE_LINKED_PARSER").is_some() {
        assert!(
            parsed.is_ok(),
            "this lane must exercise the enabled dependency parser: {parsed:?}"
        );
        assert!(parsed.as_ref().unwrap().has_prefab_links());
    }
    assert!(orr_edit::EditorDoc::from_yaml(
        text,
        collect_project::types(),
        Simulation::<CollectDodgeV1>::build_registry(),
        42
    )
    .is_err());
    assert!(orr_remote::collect_dodge::document(text).is_err());
    assert!(collect_project::PreparedScene::parse(text).is_err());
}

#[test]
fn unsupported_host_advertises_only_legacy_schema_under_feature_unification() {
    let initial = include_str!("../../../scenes/collect_dodge_v1.scene.yaml");
    let mut doc = orr_remote::collect_dodge::document(initial).unwrap();
    let legacy = collect_project::types().legacy_json_schema();
    assert_eq!(doc.view().json_schema(), legacy);
    let mut limits = orr_remote::HostLimits::default();
    orr_remote::collect_dodge::configure(&mut limits, None);
    let mut play: Option<orr_edit::PlayController<CollectDodgeV1>> = None;
    let actual = orr_remote::call_local(
        &mut orr_remote::ErpTarget {
            doc: &mut doc,
            play: &mut play,
        },
        &limits,
        "boundary-test",
        orr_remote::Caps::ALL,
        "registry.schema",
        &serde_json::json!({}),
    )
    .unwrap();
    let expected: serde_json::Value = serde_json::from_str(&legacy).unwrap();
    assert_eq!(actual["schema"], expected);
    assert_eq!(
        actual["schema"]["properties"]["schema"]["const"],
        "orr.scene/1"
    );
    assert!(actual["schema"]["properties"].get("prefabs").is_none());
}
