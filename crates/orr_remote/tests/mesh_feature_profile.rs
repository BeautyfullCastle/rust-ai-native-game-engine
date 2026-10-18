#![cfg(feature = "mesh-navigation")]

#[test]
fn admission_requires_matching_game_and_host_mission_features() {
    let result = orr_remote::mesh_navigation::mesh_doc_from_yaml(
        "schema: orr.scene/1\nsingletons: {}\nentities: {}\n",
    );
    let game_enabled = orr_games::mesh_navigation_game::MESH_MISSION_ENABLED;
    let host_enabled = cfg!(feature = "mesh-mission");
    if game_enabled == host_enabled {
        assert!(result.is_ok());
    } else {
        let error = result
            .err()
            .expect("mismatched mission features must fail admission");
        assert!(
            error
                .to_string()
                .contains("mission feature profile differs"),
            "{error}"
        );
    }
}
