//! Avatars from prefabs, drawn from view assets: `cargo run -p orr_app --release --example avatars`.
//!
//! Two local players (WASD and the arrow keys). When the run starts, `PlayerAdded` makes the avatar
//! of each player from a prefab (`avatars/avatars.prefab.yaml`, baked before the simulation starts).
//! The prefab holds a `ViewRef`; the view asset it names (`avatars/*.orrview`) holds the model
//! (`avatars/*.glb`). On a server, one window per player: `-- --host` and `-- --connect 127.0.0.1:7777`.
//! Escape quits.
use std::error::Error;
use std::path::Path;

use orr_app::{default_lighting, App, Lighting, Look3D, OrbitCamera, ViewAssets};
use orr_ecs::ComponentRegistry;
use orr_reflect::{Scene, TypeRegistry};
use orr_sim::Simulation;
use orr_testgame::lobby::{Lobby, LobbyConfig, LobbyInput};

/// Bakes the prefab fragment: one prefab per entity of the file, before the simulation exists.
fn avatar_prefabs(file: &Path, registry: &ComponentRegistry) -> Result<LobbyConfig, Box<dyn Error>> {
    let mut types = TypeRegistry::new();
    orr_gfx::register_reflect(&mut types);
    orr_gfx::register_view_ref_reflect(&mut types);
    orr_sim::register_player_link_reflect(&mut types);
    let text = std::fs::read_to_string(file).map_err(|e| format!("read {}: {e}", file.display()))?;
    let scene = Scene::parse(&text, &types).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(LobbyConfig { avatars: scene.bake_prefabs(&types, registry).map_err(|e| format!("{}: {e}", file.display()))? })
}

fn main() -> Result<(), Box<dyn Error>> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/avatars");
    let assets = ViewAssets::load_dir(&folder)?;
    let config = avatar_prefabs(&folder.join("avatars.prefab.yaml"), &Simulation::<Lobby>::build_registry())?;
    println!("{} view assets, {} avatar prefabs", assets.len(), config.avatars.len());
    // A softer sun: the picture that draws models has no tone mapping, so a bright sun would clip.
    let sun = Lighting { intensity: 1.05, ambient: 0.32, tonemap: false, ..default_lighting() };
    // A closer camera than the default: the pawns are two units tall.
    let look = Look3D::default().orbit(OrbitCamera::new([0.0, 0.9, 0.0], 0.35, 0.42, 8.5)).lighting(sun).assets(assets);
    App::<Lobby>::with_config(config)
        .local_players(2)
        .input(|keys, slot| {
            let stick = keys.stick_for_slot(slot);
            LobbyInput { dx: stick.x, dz: stick.y }
        })
        .view3d(look)
        .title("Orrery avatars (orr_app)")
        .run()?;
    Ok(())
}
