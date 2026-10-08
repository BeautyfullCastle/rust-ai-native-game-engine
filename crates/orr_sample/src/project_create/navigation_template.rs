//! Closed owned heightfield starter for one deterministic point-route agent.
use orr_fp::{fp, FP};
use orr_games::navigation_yard3d_game::{
    NavigationAgentSpec, NavigationScenePin, NavigationYard3D, PIN_NAME,
};
use orr_navigation::{AgentProfile, TerrainGraph};
use orr_reflect::Scene;
use orr_terrain::Terrain;
use sha2::{Digest, Sha256};

pub(super) fn terrain(seed: &str) -> Result<Terrain, String> {
    let mut digest = Sha256::new();
    digest.update(b"orrery.terrain-point-route.template.v1\0");
    digest.update(seed.as_bytes());
    let id = digest.finalize()[..12]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let heights = (0..81)
        .map(|i| FP::from_raw(i64::from(i % 9) * FP::ONE.raw() / 4))
        .collect();
    // Central two-cell-wide barrier: any left-to-right route must detour in Z.
    let holes = (0..64)
        .map(|i| (3..=4).contains(&(i % 8)) && (2..=5).contains(&(i / 8)))
        .collect();
    Terrain::new(
        format!("terrain-point-route/{id}"),
        9,
        9,
        [fp!(-4), fp!(-4)],
        FP::ONE,
        heights,
        holes,
    )
    .map_err(super::error)
}

pub(super) fn scene(seed: &str) -> Result<Scene, String> {
    let terrain = terrain(seed)?;
    let agent = NavigationAgentSpec {
        start: [fp!(-3), FP::ZERO],
        goal: [fp!(3), FP::ZERO],
        max_slope: fp!(0.5),
        distance_per_tick: fp!(0.125),
    };
    let graph = TerrainGraph::build(
        &terrain,
        AgentProfile {
            max_slope: agent.max_slope,
            radius: FP::ZERO,
            headroom: FP::ZERO,
            max_step: FP::ZERO,
        },
    )
    .map_err(super::error)?;
    let pin = NavigationScenePin::new("terrain.orrt", &terrain, &graph, agent)?;
    let types = crate::navigation_project::types();
    let mut simulation = orr_sim::Simulation::<NavigationYard3D>::new(
        (),
        crate::navigation_project::TICK_RATE,
        crate::navigation_project::SEED,
    );
    simulation.frame_mut().set_singleton(pin);
    let mut scene = Scene::unbake(&types, simulation.frame(), None).map_err(super::error)?;
    scene.singletons.retain(|(name, _)| name == PIN_NAME);
    scene.header_comments = vec![format!(
        "Orrery {} closed starter; authoring seed {seed}.",
        super::NAVIGATION_TEMPLATE
    )];
    crate::navigation_project::PreparedScene::parse(&scene.to_yaml(), &terrain.cook())?;
    Ok(scene)
}

pub(super) fn readme(seed: &str) -> String {
    format!("Orrery Terrain point-route playground\nTemplate: {}\nAuthoring seed: {seed}\n\nOpen with navigation-project-enabled orr_editor --navigation-project /absolute/project. Run navigation_playground --project /absolute/project; Space/P pauses or resumes and R restarts the admitted initial Frame. Export with orr_export_navigation and a trusted prebuilt navigation_playground runtime.\n\nThe owned terrain.orrt contains a slope and central hole barrier forcing a visible detour. Open Terrain authoring and Point navigation. Edit heights/holes and Save terrain; edit start/goal/slope/distance per tick, explicitly Build route, then Save scene. Unsaved or changed route/source drafts remain stale and block Play until rebuilt. Failed builds preserve the previous admitted Frame. Save, reopen, Play/Pause/Step/Seek/Stop use the existing editor lifecycle.\n\nOne zero-radius, zero-headroom, zero-step point follows deterministic terrain-triangle portal midpoints. This is not a WASD character, clearance-aware navmesh, funnel shortest path, or crowd simulation. Terrain is bounded to 17x17 vertices/512triangles and rendering coordinates to +/-256. Status reports reached, moving, or failure honestly.\n\nThe generator writes owned local scene/terrain bytes; it installs no package, runs no external script and downloads nothing. Template/tool/seed reproduce bytes. Seed namespaces terrain identity only; runtime seed42, 60Hz, two idle input slots and build identity remain unchanged. Runtime tick/seek/restart read no source files. No UI/camera/model/sprite/progress metadata is accepted. The inherited project feature still enables its existing sprite dependencies, though this closed consumer does not admit sprite content.\n", super::NAVIGATION_TEMPLATE)
}
