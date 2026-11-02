//! Versioned semantic Room challenge identity, independent of paths and rendering.
//! Room admission requires canonical body fields (including rotation), collider
//! material/filter fields, zero shape padding, ordinal zero and initial run state.
//! RULES_REVISION covers that canonical contract and compiled gameplay rules.
use crate::room_project::{PreparedProject, SEED};
use orr_ecs::Frame;
use orr_games::room_escape_game::{RoomActor, TICK_RATE};
use orr_physics3d::{Body, Collider, PhysicsConfig, PhysicsState};
use sha2::{Digest, Sha256};
const RULES_REVISION: u32 = 1;

pub(crate) fn challenge_digest(project: &PreparedProject) -> [u8; 32] {
    let frame = project.scene().frame();
    encode(
        frame,
        RULES_REVISION,
        SEED,
        TICK_RATE,
        &frame.singleton::<PhysicsState>().config,
    )
}
fn encode(
    frame: &Frame,
    rules: u32,
    seed: u64,
    tick_rate: u32,
    physics: &PhysicsConfig,
) -> [u8; 32] {
    let mut hash = Sha256::new();
    let domain = b"orrery.room-escape.challenge.v1";
    hash.update((domain.len() as u32).to_le_bytes());
    hash.update(domain);
    hash.update(rules.to_le_bytes());
    hash.update(seed.to_le_bytes());
    hash.update(tick_rate.to_le_bytes());
    // Explicit fields, not serde, pointer width, struct padding or raw memory.
    for value in [
        physics.gravity.x,
        physics.gravity.y,
        physics.gravity.z,
        physics.dt,
        physics.baumgarte,
        physics.linear_slop,
        physics.contact_margin,
        physics.restitution_threshold,
        physics.max_correction_speed,
        physics.max_linear_speed,
        physics.max_angular_speed,
        physics.sleep_linear_speed,
        physics.sleep_angular_speed,
    ] {
        hash.update(value.0.to_le_bytes());
    }
    for value in [
        physics.velocity_iterations,
        physics.substeps,
        physics._pad,
        physics.sleep_ticks,
    ] {
        hash.update(value.to_le_bytes());
    }
    let mut actors = Vec::new();
    for entity in frame.entities() {
        let actor = frame.get::<RoomActor>(entity).expect("admitted Room actor");
        let body = frame.get::<Body>(entity).expect("admitted Room body");
        let shape = frame
            .get::<Collider>(entity)
            .expect("admitted Room collider")
            .shape;
        actors.push((
            actor.kind,
            actor.ordinal,
            [body.pos.x.0, body.pos.y.0, body.pos.z.0],
            shape.kind,
            shape.radius.0,
            [shape.half.x.0, shape.half.y.0, shape.half.z.0],
        ));
    }
    // All walls have the same role/ordinal. Sort their entire physical tuple,
    // preserving duplicate counts rather than tying identity to entity order.
    actors.sort();
    hash.update((actors.len() as u32).to_le_bytes());
    for (kind, ordinal, pos, shape, radius, half) in actors {
        hash.update(kind.to_le_bytes());
        hash.update(ordinal.to_le_bytes());
        for coordinate in pos {
            hash.update(coordinate.to_le_bytes());
        }
        hash.update(shape.to_le_bytes());
        hash.update(radius.to_le_bytes());
        for coordinate in half {
            hash.update(coordinate.to_le_bytes());
        }
    }
    hash.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room_project::{self, PreparedScene};
    use orr_games::room_escape_game::{
        RoomConfig, RoomEscapeV1, RoomInput, RoomRun, EXIT, KEY, PLAYER, WALL,
    };
    use orr_physics3d::Shape;
    use orr_reflect::Scene;
    use orr_sim::{Simulation, TickInputs};
    use std::{collections::BTreeMap, fs, path::Path};
    fn initial() -> Simulation<RoomEscapeV1> {
        Simulation::new(RoomConfig, TICK_RATE, SEED)
    }
    fn actor(frame: &Frame, kind: u32) -> orr_ecs::Entity {
        frame
            .entities()
            .find(|&e| frame.get::<RoomActor>(e).unwrap().kind == kind)
            .unwrap()
    }
    fn scene(frame: &Frame) -> String {
        Scene::unbake(&room_project::types(), frame, None)
            .unwrap()
            .to_yaml()
    }
    fn digest_scene(text: &str) -> [u8; 32] {
        let p = PreparedScene::parse(text).unwrap();
        encode(
            p.frame(),
            RULES_REVISION,
            SEED,
            TICK_RATE,
            &p.frame().singleton::<PhysicsState>().config,
        )
    }
    fn copy_tree(from: &Path, to: &Path) {
        fs::create_dir(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            assert!(!entry.file_type().unwrap().is_symlink());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    fn project(text: &str, name: &str, translation: f32) -> (tempfile::TempDir, PreparedProject) {
        use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
        let dir = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let root = dir.path().join(name);
        fs::create_dir(&root).unwrap();
        fs::write(root.join("room.scene.yaml"), text).unwrap();
        fs::write(root.join("orr.project.json"),serde_json::to_vec(&serde_json::json!({"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json"}})).unwrap()).unwrap();
        let package = dir.path().join("source");
        copy_tree(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/imported_scene_demo"),
            &package,
        );
        orr_package::Project::open_for_install(
            &root,
            orr_package::Runtime::content_only().engine_version,
        )
        .unwrap()
        .install(&[package])
        .unwrap();
        let loaded =
            model_bindings::load_asset(&root, "sample-imported-scene", "foreground.glb").unwrap();
        let transform = LocalTransform {
            translation: [translation, 0.0, 0.0],
            ..LocalTransform::default()
        };
        let binding = Binding::from_asset(
            "sample-imported-scene".into(),
            "foreground.glb".into(),
            &loaded,
            transform,
        )
        .unwrap();
        let prepared = PreparedScene::parse(text).unwrap();
        let guid = prepared
            .index()
            .guid(actor(prepared.frame(), PLAYER))
            .unwrap()
            .to_string();
        let document = Document {
            version: 2,
            scene: "room.scene.yaml".into(),
            project: ".".into(),
            bindings: BTreeMap::from([(guid, binding)]),
        };
        fs::write(
            root.join("room.models.json"),
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
        let prepared = PreparedProject::open(root).unwrap();
        (dir, prepared)
    }
    #[test]
    fn admitted_projects_ignore_paths_guids_names_and_render_transform() {
        let text = scene(initial().frame());
        let (_a, a) = project(&text, "first", 0.0);
        let renamed = format!("# cosmetic\n{}", text.replace("e_000000", "e_111111"));
        let mut parsed = Scene::parse(&renamed, &room_project::types()).unwrap();
        for entity in parsed.entities.values_mut() {
            entity.name = Some("renamed".into());
        }
        let (_b, b) = project(&parsed.to_yaml(), "moved", 2.0);
        assert_ne!(a.root(), b.root());
        assert_eq!(challenge_digest(&a), challenge_digest(&b));
    }
    #[test]
    fn wall_entity_permutation_keeps_digest_and_actual_trajectory() {
        let text = scene(initial().frame());
        let prepared = PreparedScene::parse(&text).unwrap();
        let walls: Vec<_> = prepared
            .frame()
            .entities()
            .filter(|&e| prepared.frame().get::<RoomActor>(e).unwrap().kind == WALL)
            .collect();
        let first = prepared.index().guid(walls[0]).unwrap().clone();
        let second = prepared.index().guid(walls[1]).unwrap().clone();
        let mut parsed = Scene::parse(&text, &room_project::types()).unwrap();
        let a = parsed.entities[&first].clone();
        let b = parsed.entities[&second].clone();
        parsed.entities.insert(first, b);
        parsed.entities.insert(second, a);
        let changed = parsed.to_yaml();
        assert_eq!(digest_scene(&text), digest_scene(&changed));
        let mut a = prepared.simulation().unwrap();
        let mut b = PreparedScene::parse(&changed)
            .unwrap()
            .simulation()
            .unwrap();
        for tick in 0..480 {
            let input = RoomInput {
                move_x: if tick < 160 { 1 } else { -1 },
                move_z: if tick < 320 { 1 } else { -1 },
                buttons: if tick % 31 == 0 { 1 } else { 0 },
                ..Default::default()
            };
            for sim in [&mut a, &mut b] {
                let mut inputs = TickInputs::new(sim.tick(), 1);
                inputs.set_input(orr_bridge::PlayerSlot(0), input);
                sim.step(&inputs);
            }
            assert_eq!(
                a.frame().get::<Body>(actor(a.frame(), PLAYER)).unwrap().pos,
                b.frame().get::<Body>(actor(b.frame(), PLAYER)).unwrap().pos
            );
            assert_eq!(
                a.frame().singleton::<RoomRun>(),
                b.frame().singleton::<RoomRun>()
            );
        }
    }
    #[test]
    fn admitted_geometry_and_role_changes_separate_challenges() {
        let baseline = digest_scene(&scene(initial().frame()));
        for mode in 0..4 {
            let mut sim = initial();
            let entity = actor(sim.frame(), if mode == 0 { WALL } else { KEY });
            match mode {
                0 => sim.frame_mut().get_mut::<Body>(entity).unwrap().pos.x += orr_fp::FP::ONE,
                1 => {
                    sim.frame_mut().get_mut::<Collider>(entity).unwrap().shape =
                        Shape::sphere(orr_fp::FP::from_ratio(3, 10))
                }
                2 => sim.frame_mut().get_mut::<Body>(entity).unwrap().pos.z += orr_fp::FP::ONE,
                _ => {
                    let exit = actor(sim.frame(), EXIT);
                    sim.frame_mut().get_mut::<RoomActor>(entity).unwrap().kind = EXIT;
                    sim.frame_mut().get_mut::<RoomActor>(exit).unwrap().kind = KEY;
                }
            }
            assert_ne!(baseline, digest_scene(&scene(sim.frame())), "mode {mode}");
        }
    }
    #[test]
    fn rules_seed_tickrate_and_every_physics_field_are_encoded() {
        let sim = initial();
        let config = sim.frame().singleton::<PhysicsState>().config;
        let baseline = encode(sim.frame(), 1, SEED, TICK_RATE, &config);
        assert_ne!(baseline, encode(sim.frame(), 2, SEED, TICK_RATE, &config));
        assert_ne!(
            baseline,
            encode(sim.frame(), 1, SEED + 1, TICK_RATE, &config)
        );
        assert_ne!(
            baseline,
            encode(sim.frame(), 1, SEED, TICK_RATE + 1, &config)
        );
        macro_rules! changed { ($($field:ident$(.$sub:ident)?),*) => { $(let mut c=config; c.$field$(.$sub)?.0+=1; assert_ne!(baseline,encode(sim.frame(),1,SEED,TICK_RATE,&c));)* }; }
        changed!(
            gravity.x,
            gravity.y,
            gravity.z,
            dt,
            baumgarte,
            linear_slop,
            contact_margin,
            restitution_threshold,
            max_correction_speed,
            max_linear_speed,
            max_angular_speed,
            sleep_linear_speed,
            sleep_angular_speed
        );
        macro_rules! integer { ($($field:ident),*) => { $(let mut c=config; c.$field+=1; assert_ne!(baseline,encode(sim.frame(),1,SEED,TICK_RATE,&c));)* }; }
        integer!(velocity_iterations, substeps, _pad, sleep_ticks);
    }
}
