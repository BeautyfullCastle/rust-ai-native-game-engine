//! Required offscreen terrain-triangle point-agent lab, not physical/window play.
use orr_fp::FP;
use orr_navigation::{NavigationStatus, Navigator, SearchBudget};
use orr_navigation_view::{
    fixture, gpu,
    package::{self, InstalledNavigation},
    PathDisplay, Result,
};
use orr_package::{Project, Runtime};
use orr_render::orr_rhi::{Rhi, Wgpu, WgpuOptions};
use orr_terrain::{Edit, TerrainDocument};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn hex(bytes: [u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn snapshot(
    output: &Path,
    name: &str,
    device: &Wgpu,
    loaded: &InstalledNavigation,
    agent: &Navigator,
    tick: u32,
    display: PathDisplay,
) -> Result<serde_json::Value> {
    let size = (1000, 750);
    let mut renderer = gpu::renderer(
        device,
        &loaded.terrain,
        agent.path(),
        agent.position(),
        display,
    )?;
    let pixels = gpu::capture(&mut renderer, &gpu::top_camera(), &gpu::lighting(), size)?;
    let png = format!("{name}.png");
    gpu::save_png(&output.join(&png), &pixels, size)?;
    let path = agent.path();
    let evidence = serde_json::json!({
        "stage":name,"tick":tick,"png":png,"rendering":"one StaticModel / one ModelRenderer draw containing terrain + path + agent",
        "terrain_asset_id":loaded.terrain.asset_id(),"terrain_revision":hex(loaded.terrain.revision()),
        "navigation_asset_id":loaded.scenario.navigation_asset_id,"navigation_revision":hex(loaded.graph.revision()),
        "navigation_terrain_revision":hex(loaded.graph.terrain_revision()),"profile":{"max_slope_raw":loaded.graph.profile().max_slope.raw(),"radius_raw":loaded.graph.profile().radius.raw(),"headroom_raw":loaded.graph.profile().headroom.raw(),"max_step_raw":loaded.graph.profile().max_step.raw()},
        "package_digest":loaded.package_digest,"scenario_asset_id":loaded.scenario.asset_id,
        "agent_checksum":hex(agent.checksum()),"agent_position_raw":agent.position().map(FP::raw),"agent_status":format!("{:?}",agent.status()),"path_display":format!("{display:?}"),
        "corridor":path.map(|p|p.corridor().iter().map(|k|k.0).collect::<Vec<_>>()),"waypoints_raw":path.map(|p|p.waypoints().iter().map(|p|p.map(FP::raw)).collect::<Vec<_>>()),
        "path_cost_raw":path.map(|p|p.cost().raw()),"active_path_graph_revision":path.map(|p|hex(p.graph_revision())),"active_path_terrain_revision":path.map(|p|hex(p.terrain_revision())),"rgba_bytes":pixels.len(),
        "agent_orange_pixel_count":pixels.chunks_exact(4).filter(|p|p[0]>220&&p[1]>60&&p[1]<120&&p[2]<50).count(),
    });
    fs::write(
        output.join(format!("{name}.json")),
        serde_json::to_vec_pretty(&evidence)?,
    )?;
    Ok(evidence)
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 {
        return Err("usage: navigation_lab NEW_OUTPUT_DIRECTORY (writes actual installed package, PNGs and JSON; GPU or software Vulkan required)".into());
    }
    let output = PathBuf::from(&args[0]);
    fs::create_dir(&output)?;
    let device = Wgpu::headless(WgpuOptions::default())?;
    eprintln!(
        "navigation adapter={} software={}",
        device.adapter_name(),
        device.is_software()
    );
    let root = output.join("project");
    let before =
        package::install_and_reload(&fixture(), &root, &output.join("source-v1"), "1.0.0")?;
    let mut agent = Navigator::new(&before.graph, &before.terrain, before.scenario.start())?;
    agent.replan(
        &before.graph,
        &before.terrain,
        before.scenario.goal(),
        SearchBudget::default(),
    )?;
    let mut stages = vec![snapshot(
        &output,
        "01-initial",
        &device,
        &before,
        &agent,
        0,
        PathDisplay::Current,
    )?];
    let distance = before.scenario.distance_per_tick();
    let initial = agent.position();
    for _ in 0..6 {
        agent.advance(&before.graph, &before.terrain, distance)?;
    }
    assert_ne!(agent.position(), initial);
    assert_eq!(agent.status(), NavigationStatus::Moving);
    let middle = agent.position();
    stages.push(snapshot(
        &output,
        "02-midtick",
        &device,
        &before,
        &agent,
        6,
        PathDisplay::Current,
    )?);
    let mut tick = 6;
    while agent.status() != NavigationStatus::Arrived {
        if tick >= 4096 {
            return Err("bounded lab agent did not arrive within 4096 ticks".into());
        }
        agent.advance(&before.graph, &before.terrain, distance)?;
        tick += 1;
    }
    assert_eq!(
        [agent.position()[0], agent.position()[2]],
        before.scenario.goal()
    );
    stages.push(snapshot(
        &output,
        "03-arrival",
        &device,
        &before,
        &agent,
        tick,
        PathDisplay::Current,
    )?);
    // Restart the same loaded scenario to witness an edit during an active path.
    agent = Navigator::new(&before.graph, &before.terrain, before.scenario.start())?;
    agent.replan(
        &before.graph,
        &before.terrain,
        before.scenario.goal(),
        SearchBudget::default(),
    )?;
    for _ in 0..6 {
        agent.advance(&before.graph, &before.terrain, distance)?;
    }
    assert_eq!(agent.position(), middle);
    let old_waypoints = agent.path().unwrap().waypoints().to_vec();
    let mut document = TerrainDocument::new(before.terrain.clone());
    document.apply(&[Edit::SetHole {
        x: 3,
        z: 6,
        hole: true,
    }])?;
    let edited_revision = document.terrain().revision();
    assert!(document.undo());
    assert_eq!(document.terrain(), &before.terrain);
    assert!(document.redo());
    assert_eq!(document.terrain().revision(), edited_revision);
    let after = package::install_and_reload(
        document.terrain(),
        &root,
        &output.join("source-v2"),
        "1.0.1",
    )?;
    let frozen_position = agent.position();
    let stale_error = agent
        .advance(&before.graph, &after.terrain, distance)
        .unwrap_err()
        .to_string();
    assert_eq!(agent.position(), frozen_position);
    let replaced_graph_error = agent
        .advance(&after.graph, &after.terrain, distance)
        .unwrap_err()
        .to_string();
    assert_eq!(agent.position(), frozen_position);
    assert_eq!(agent.path().unwrap().waypoints(), old_waypoints);
    stages.push(snapshot(
        &output,
        "04-edit-stale",
        &device,
        &after,
        &agent,
        6,
        PathDisplay::Stale,
    )?);
    agent.replan(
        &after.graph,
        &after.terrain,
        after.scenario.goal(),
        SearchBudget::default(),
    )?;
    assert_ne!(agent.path().unwrap().waypoints(), old_waypoints);
    agent.advance(&after.graph, &after.terrain, distance)?;
    assert_ne!(agent.position(), frozen_position);
    stages.push(snapshot(
        &output,
        "05-replan",
        &device,
        &after,
        &agent,
        7,
        PathDisplay::Current,
    )?);
    // Source mutation cannot replace immutable installed bytes used by host/view.
    for path in [
        package::TERRAIN_PATH,
        package::NAVIGATION_PATH,
        package::SCENARIO_PATH,
    ] {
        fs::write(
            output.join("source-v2").join(path),
            b"post-install source edit",
        )?;
    }
    let reopened = package::load_installed(&root)?;
    assert_eq!(reopened.terrain, after.terrain);
    assert_eq!(reopened.graph.cook(), after.graph.cook());
    assert_eq!(reopened.scenario, after.scenario);
    let project = Project::open(&root, package::runtime())?;
    assert!(project
        .read_asset(package::PACKAGE_NAME, "missing.orrnav")
        .is_err());
    assert!(Project::open(&root, Runtime::content_only())?
        .read_asset(package::PACKAGE_NAME, package::NAVIGATION_PATH)
        .is_err());
    let boundaries = package::exercise_failure_cases(&output.join("negative-boundary-projects"))?;
    let report = serde_json::json!({
        "scope":"bounded terrain-triangle point-agent prototype, partial #103; not a general navmesh implementation",
        "source_sha":std::env::var("ORR_SOURCE_SHA").ok(),"adapter":device.adapter_name(),"software_adapter":device.is_software(),"offscreen":true,"physical_window_play":false,
        "loaded_from":"Project::install -> reopen with compiled terrain_v1/navigation_v1 capabilities -> verified Project::read_asset for terrain, cooked graph and scenario",
        "scenario":after.scenario,"stages":stages,"stale_advance_error":stale_error,"replacement_graph_advance_error":replaced_graph_error,"negative_boundaries":boundaries,
        "source_mutation_isolated":true,"undo_redo_revision_restored":true,
        "limitations":["zero radius/headroom/max_step point agent only","portal midpoint polyline, no funnel or smoothing","full graph/model rebuild after terrain edit","fresh independent color/depth pass, no overlay into Renderer3D","no avoidance, scene, physics, editor, streaming, live package swapping or windowed integration"]
    });
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
