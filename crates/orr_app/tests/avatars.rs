//! The files of the `avatars` example agree with each other: the view assets load, the prefab
//! fragment bakes and names them by id, and an offline run gives each player the avatar of a
//! prefab whose view asset resolves. No window.
use std::path::{Path, PathBuf};

use orr_app::ViewAssets;
use orr_gfx::ViewRef;
use orr_reflect::{Scene, TypeRegistry};
use orr_session::{PlayConfig, PlaySession};
use orr_sim::{PlayerLink, Simulation};
use orr_testgame::lobby::{Lobby, LobbyConfig};

fn folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/avatars")
}

fn prefabs() -> LobbyConfig {
    let mut types = TypeRegistry::new();
    orr_gfx::register_reflect(&mut types);
    orr_gfx::register_view_ref_reflect(&mut types);
    orr_sim::register_player_link_reflect(&mut types);
    let text = std::fs::read_to_string(folder().join("avatars.prefab.yaml")).expect("the fragment exists");
    let scene = Scene::parse(&text, &types).expect("the fragment parses");
    LobbyConfig { avatars: scene.bake_prefabs(&types, &Simulation::<Lobby>::build_registry()).expect("the fragment bakes") }
}

#[test]
fn the_view_assets_of_the_example_load_and_their_models_exist() {
    let assets = ViewAssets::load_dir(folder()).expect("the assets load");

    assert_eq!(assets.len(), 2);
    let blue = assets.get(ViewRef::at("blue.orrview").asset).expect("the id of the path resolves");
    assert_eq!((blue.name.as_str(), blue.model.as_str()), ("Blue pawn", "blue.glb"));
    assert!(folder().join(&blue.model).is_file());
    let red = assets.get(ViewRef::at("red.orrview").asset).expect("the id of the path resolves");
    assert_eq!((red.name.as_str(), red.sound.as_deref()), ("Red pawn", Some("step")));
}

#[test]
fn the_prefab_fragment_names_the_view_assets_by_their_ids() {
    let config = prefabs();
    assert_eq!(config.avatars.len(), 2, "one prefab per entity of the fragment");

    let mut session = PlaySession::<Lobby>::new(PlayConfig::new(2, 1, 60), config);
    session.announce_players().unwrap();
    session.step_now().unwrap();

    let frame = session.frame();
    let mut avatars: Vec<(u8, u64)> = frame
        .entities()
        .filter_map(|e| Some((frame.get::<PlayerLink>(e)?.player.0, frame.get::<ViewRef>(e)?.asset)))
        .collect();
    avatars.sort_unstable();
    assert_eq!(avatars, vec![(0, ViewRef::at("blue.orrview").asset), (1, ViewRef::at("red.orrview").asset)]);
}
