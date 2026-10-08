//! CPU-only starter/admission/route evidence; no GPU or editor acceptance claim.
#![cfg(all(
    feature = "navigation-project",
    feature = "project-create",
    target_os = "linux"
))]

use orr_ecs::Frame;
use orr_fp::FP;
use orr_games::navigation_yard3d_game::{
    NavigationScenePin, NavigationStepStatus, NavigationYard3D, PIN_NAME,
};
use orr_navigation::{AgentProfile, NavigationStatus, TerrainGraph};
use orr_navigation_runtime::{AgentReadback, NavigationRuntime, RuntimeState};
use orr_reflect::Scene;
use orr_sample::{
    navigation_project::{self, PreparedProject, PreparedScene},
    project_create::{self, CreateOptions, NAVIGATION_TEMPLATE},
};
use orr_session::{ControlOp, PlaySession};
use orr_terrain::Terrain;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

fn create(parent: &Path, name: &str, seed: &str) -> PathBuf {
    let root = parent.join(name);
    let report = project_create::create(&CreateOptions {
        output: root.clone(),
        template: NAVIGATION_TEMPLATE.into(),
        seed: seed.into(),
    })
    .unwrap();
    let project = PreparedProject::open(&root).unwrap();
    assert_eq!(report.initial_checksum, project.scene().frame().checksum());
    assert!(report.entity_guids.is_empty());
    root
}

fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let kind = entry.file_type().unwrap();
            assert!(!kind.is_symlink());
            if kind.is_dir() {
                visit(root, &path, files);
            } else {
                assert!(kind.is_file());
                files.insert(
                    path.strip_prefix(root).unwrap().to_str().unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

fn agent(frame: &Frame) -> AgentReadback {
    let entity = frame.singleton::<RuntimeState>().agent;
    NavigationRuntime::agent(frame, entity).unwrap()
}

#[test]
fn generated_project_is_byte_deterministic_owned_and_package_free() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().canonicalize().unwrap();
    let first = create(&parent, "first", "fixed-seed");
    let second = create(&parent, "second", "fixed-seed");
    let third = create(&parent, "third", "other-seed");
    assert_eq!(files(&first), files(&second));
    assert_ne!(files(&first), files(&third));
    for root in [&first, &second, &third] {
        assert!(!root.join("orr.packages.lock.json").exists());
        assert!(!root.join(".orr").exists());
        let prepared = PreparedProject::open(root).unwrap();
        let parsed =
            PreparedScene::parse(prepared.scene().text(), prepared.terrain_bytes()).unwrap();
        assert_eq!(
            parsed.frame().to_bytes(),
            prepared.scene().frame().to_bytes()
        );
        assert_eq!(parsed.frame().alive_count(), 1);
        assert_eq!(parsed.frame().count::<orr_physics3d::Body>(), 0);
        assert_eq!(parsed.frame().count::<orr_physics3d::Collider>(), 0);
        assert_eq!(agent(parsed.frame()).status, NavigationStatus::Moving);
    }
}

#[test]
fn visible_hole_detour_reaches_exact_goal_with_surface_y_seek_restart_and_replay() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().canonicalize().unwrap();
    let root = create(&parent, "route", "detour");
    let project = PreparedProject::open(&root).unwrap();
    let terrain = Terrain::load(project.terrain_bytes()).unwrap();
    let initial = project.scene().frame().to_bytes();
    let initial_agent = agent(project.scene().frame());
    let path = initial_agent.path.as_ref().unwrap();
    assert!(path.corridor().len() > 2);
    assert!(path.waypoints().iter().any(|point| point[2] != FP::ZERO));
    assert!(
        terrain.sample(FP::ZERO, FP::ZERO).is_none(),
        "straight start-goal line crosses the central hole"
    );
    let goal = *path.waypoints().last().unwrap();
    assert_eq!(goal[0], initial_agent.spec.goal[0]);
    assert_eq!(goal[2], initial_agent.spec.goal[1]);
    let mut first = project.scene().session().unwrap();
    let mut repeat = project.scene().session().unwrap();
    let mut positions = BTreeSet::new();
    let mut frames = BTreeMap::from([(0, initial.clone())]);
    fs::remove_dir_all(&root).unwrap();
    for _ in 0..240 {
        first.step_now().unwrap();
        repeat.step_now().unwrap();
        assert_eq!(first.frame().to_bytes(), repeat.frame().to_bytes());
        assert_eq!(first.frame().singleton::<NavigationStepStatus>().failed, 0);
        let current = agent(first.frame());
        assert_eq!(
            terrain.sample(current.position[0], current.position[2]),
            Some(current.position[1])
        );
        positions.insert(current.position.map(FP::raw));
        frames.insert(first.frame().tick(), first.frame().to_bytes());
        if current.status == NavigationStatus::Arrived {
            break;
        }
    }
    let final_agent = agent(first.frame());
    assert_eq!(final_agent.status, NavigationStatus::Arrived);
    assert_eq!(final_agent.position, goal);
    assert!(positions.len() > 10);
    assert!(positions.iter().any(|point| point[2] != 0));
    let final_tick = first.frame().tick();
    let final_bytes = first.frame().to_bytes();
    let halfway = final_tick / 2;
    first.control(ControlOp::Seek(halfway));
    assert_eq!(&first.frame().to_bytes(), frames.get(&halfway).unwrap());
    first.control(ControlOp::Step((final_tick - halfway) as u32));
    assert_eq!(first.frame().to_bytes(), final_bytes);
    first.control(ControlOp::Seek(0));
    assert_eq!(first.frame().to_bytes(), initial);
    first.control(ControlOp::Branch);
    first.control(ControlOp::Step(final_tick as u32));
    assert_eq!(first.frame().to_bytes(), final_bytes);
    let replay_bytes = first.save_replay();
    let mut replay = PlaySession::<NavigationYard3D>::open_replay(
        &replay_bytes,
        (),
        navigation_project::build_id(),
    )
    .unwrap();
    replay.control(ControlOp::Step(final_tick as u32));
    assert_eq!(replay.frame().to_bytes(), final_bytes);
    assert_eq!(
        project.scene().session().unwrap().frame().to_bytes(),
        initial
    );
    assert_eq!(project.scene().frame().to_bytes(), initial);
}

fn repin(project: &PreparedProject, terrain: &Terrain, max_slope: FP) -> String {
    let mut spec = project
        .scene()
        .frame()
        .singleton::<NavigationScenePin>()
        .agent;
    spec.max_slope = max_slope;
    let graph = TerrainGraph::build(
        terrain,
        AgentProfile {
            max_slope,
            radius: FP::ZERO,
            headroom: FP::ZERO,
            max_step: FP::ZERO,
        },
    )
    .unwrap();
    let pin = NavigationScenePin::new("terrain.orrt", terrain, &graph, spec).unwrap();
    let types = navigation_project::types();
    let mut scene = Scene::parse(project.scene().text(), &types).unwrap();
    scene.singletons[0].1 = types.get(PIN_NAME).unwrap().read(bytemuck::bytes_of(&pin));
    scene.to_yaml()
}

#[test]
fn disconnected_holes_and_unwalkable_slope_fail_without_mutating_admitted_state() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().canonicalize().unwrap();
    let root = create(&parent, "route", "negative");
    let project = PreparedProject::open(&root).unwrap();
    let initial = project.scene().frame().to_bytes();
    let terrain = Terrain::load(project.terrain_bytes()).unwrap();
    let too_steep = repin(&project, &terrain, FP::ZERO);
    assert!(PreparedScene::parse(&too_steep, &terrain.cook()).is_err());
    let disconnected = Terrain::new(
        terrain.asset_id().into(),
        terrain.width(),
        terrain.depth(),
        terrain.origin(),
        terrain.spacing(),
        terrain.heights().to_vec(),
        (0..64).map(|i| (3..=4).contains(&(i % 8))).collect(),
    )
    .unwrap();
    let text = repin(
        &project,
        &disconnected,
        project
            .scene()
            .frame()
            .singleton::<NavigationScenePin>()
            .agent
            .max_slope,
    );
    assert!(PreparedScene::parse(&text, &disconnected.cook()).is_err());
    assert_eq!(project.scene().frame().to_bytes(), initial);
    assert_eq!(
        project.scene().session().unwrap().frame().to_bytes(),
        initial
    );
}
