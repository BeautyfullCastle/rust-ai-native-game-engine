//! Metadata admits a closed profile, never grants consuming-code capabilities.
use orr_package::{Project, ProjectGame, Runtime};
#[test]
fn navigation_profile_accepts_only_schema_two_without_foreign_descriptors() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("orr.project.json");
    let valid = serde_json::json!({"schema":2,"engine":"^0.0.1","entry":{"game":"terrain-point-route-3d-v1","scene":"navigation.scene.yaml"}});
    std::fs::write(&path, serde_json::to_vec(&valid).unwrap()).unwrap();
    assert_eq!(
        Project::open(temp.path(), Runtime::content_only())
            .unwrap()
            .manifest()
            .unwrap()
            .entry
            .as_ref()
            .unwrap()
            .game,
        ProjectGame::TerrainPointRoute3dV1
    );
    for (key, value) in [
        ("sprites", serde_json::json!("sprites.json")),
        ("models", serde_json::json!("models.json")),
        ("camera", serde_json::json!("camera.json")),
        (
            "ui",
            serde_json::json!({"profile":"arena-korean-v1","font":{"package":"font","asset":"font.otf"}}),
        ),
    ] {
        let mut bad = valid.clone();
        bad["entry"][key] = value;
        std::fs::write(&path, serde_json::to_vec(&bad).unwrap()).unwrap();
        assert!(
            Project::open(temp.path(), Runtime::content_only()).is_err(),
            "{key}"
        );
    }
    for schema in [1, 3, 4, 5] {
        let mut bad = valid.clone();
        bad["schema"] = schema.into();
        std::fs::write(&path, serde_json::to_vec(&bad).unwrap()).unwrap();
        assert!(Project::open(temp.path(), Runtime::content_only()).is_err());
    }
}
