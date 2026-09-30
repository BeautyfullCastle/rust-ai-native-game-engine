//! Writes `scenes/physics_demo.scene.yaml` from `PhysGame`'s own setup.
//!
//! Run from the repo root: `cargo run -p orr_edit --example gen_physics_demo`.
//! The scene has 40 dynamic bodies (rain layout), the floor and walls,
//! static obstacles and two paddles, with readable GUIDs and names.

use orr_ecs::Entity;
use orr_physics::{Body, BODY_DYNAMIC};
use orr_reflect::{Guid, Scene, SceneIndex, TypeRegistry};
use orr_sample::physics_game::{register_reflect, PaddleTag, PhysConfig, PhysGame, SceneMode};
use orr_sim::Simulation;

fn main() {
    let cfg = PhysConfig::new(40, SceneMode::Rain);
    let sim = Simulation::<PhysGame>::new(cfg, 60, 1);
    let frame = sim.frame();

    let mut types = TypeRegistry::new();
    register_reflect(&mut types);

    // Readable GUIDs (spawn order) and names.
    let mut index = SceneIndex::default();
    let (mut walls, mut obstacles, mut bodies) = (0, 0, 0);
    let entities: Vec<Entity> = frame.entities().collect();
    for (i, &e) in entities.iter().enumerate() {
        let guid = Guid::from_u32(i as u32 + 1);
        let name = if let Some(tag) = frame.get::<PaddleTag>(e) {
            format!("paddle_{}", tag.slot)
        } else if frame.get::<Body>(e).is_some_and(|b| b.kind == BODY_DYNAMIC) {
            bodies += 1;
            format!("body_{:02}", bodies)
        } else if walls < 3 {
            walls += 1;
            ["floor", "wall_left", "wall_right"][walls - 1].to_string()
        } else {
            obstacles += 1;
            format!("obstacle_{obstacles}")
        };
        index.insert(guid.clone(), e);
        index.set_name(&guid, Some(name));
    }

    let mut scene = Scene::unbake(&types, frame, Some(&index)).expect("unbake");
    scene.header_comments = vec![
        "Physics demo: 40 bodies, floor, walls, obstacles and two paddles.".to_string(),
        "Generated from PhysGame's setup: cargo run -p orr_edit --example gen_physics_demo".to_string(),
    ];
    let text = scene.to_yaml();
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenes/physics_demo.scene.yaml");
    std::fs::write(path, &text).expect("write scene");
    println!("wrote {path}: {} entities, {} bytes", scene.entities.len(), text.len());
}
