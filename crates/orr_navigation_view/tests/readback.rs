//! Release acceptance MUST set ORR_REQUIRE_GPU=1; no ignored tests. Software
//! Vulkan is valid evidence and its identity is printed, never called window play.
#![cfg(feature = "gpu")]
#![allow(clippy::float_arithmetic)]
use orr_fp::FP;
use orr_navigation::{NavigationStatus, Navigator, SearchBudget};
use orr_navigation_view::{fixture, gpu, package, PathDisplay};
use orr_render::{
    orr_rhi::{Rhi, Wgpu, WgpuOptions},
    Camera3D,
};
use orr_terrain::{Edit, TerrainDocument};

fn adapter() -> Option<Wgpu> {
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(gpu) => {
            eprintln!(
                "navigation acceptance adapter={} software={}",
                gpu.adapter_name(),
                gpu.is_software()
            );
            Some(gpu)
        }
        Err(error) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required navigation GPU unavailable: {error}"
            );
            eprintln!(
                "SKIP optional navigation readback, set ORR_REQUIRE_GPU=1 for acceptance: {error}"
            );
            None
        }
    }
}
fn pixel(bytes: &[u8], world: [FP; 3], camera: &Camera3D, size: (u32, u32)) -> [u8; 4] {
    let xy = camera.world_to_screen(world.map(FP::to_f32), size).unwrap();
    assert!(xy[0] >= 0.0 && xy[1] >= 0.0 && xy[0] < (size.0 as f32) && xy[1] < (size.1 as f32));
    let offset = ((xy[1] as u32 * size.0 + xy[0] as u32) * 4) as usize;
    bytes[offset..offset + 4].try_into().unwrap()
}
fn orange(c: [u8; 4]) -> bool {
    c[0] > 220 && c[1] > 60 && c[1] < 120 && c[2] < 50
}
fn cyan(c: [u8; 4]) -> bool {
    c[0] < 100 && c[1] > 160 && c[2] > 220
}
fn red(c: [u8; 4]) -> bool {
    c[0] > 220 && c[1] < 60 && c[2] < 100
}
fn green(c: [u8; 4]) -> bool {
    let [r, g, b, _] = c.map(u16::from);
    g > 110 && g > r + 30 && g > b + 20
}
fn capture(
    device: &Wgpu,
    loaded: &package::InstalledNavigation,
    agent: &Navigator,
    display: PathDisplay,
    size: (u32, u32),
) -> Vec<u8> {
    let mut renderer = gpu::renderer(
        device,
        &loaded.terrain,
        agent.path(),
        agent.position(),
        display,
    )
    .unwrap();
    gpu::capture(&mut renderer, &gpu::top_camera(), &gpu::lighting(), size).unwrap()
}
#[test]
fn pixel_color_predicates_reject_route_agent_confusion_and_white_terrain() {
    assert!(!green([255, 255, 255, 255]));
    assert!(!green([250, 255, 250, 255]));
    assert!(green([62, 150, 91, 255]));
    assert!(green([89, 179, 113, 255]));
    assert!(orange([255, 90, 24, 255]));
    assert!(!orange([255, 35, 45, 255]));
    assert!(red([255, 35, 45, 255]));
    assert!(!red([255, 90, 24, 255]));
    for green in 0..=255 {
        let color = [255, green, 24, 255];
        assert!(!(orange(color) && red(color)));
    }
}

#[test]
fn packaged_point_agent_initial_midtick_arrival_edit_stale_replan_readback() {
    let Some(device) = adapter() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("project");
    let before =
        package::install_and_reload(&fixture(), &root, &tmp.path().join("source-v1"), "1.0.0")
            .unwrap();
    let size = (512, 512);
    let camera = gpu::top_camera();
    let mut agent =
        Navigator::new(&before.graph, &before.terrain, before.scenario.start()).unwrap();
    agent
        .replan(
            &before.graph,
            &before.terrain,
            before.scenario.goal(),
            SearchBudget::default(),
        )
        .unwrap();
    let initial_position = agent.position();
    let old_path = agent.path().unwrap().clone();
    let initial = capture(&device, &before, &agent, PathDisplay::Current, size);
    assert!(
        orange(pixel(&initial, initial_position, &camera, size)),
        "agent absent from composed initial draw"
    );
    let terrain_pixel = pixel(
        &initial,
        [FP::from_raw(229376), FP::ZERO, FP::from_raw(229376)],
        &camera,
        size,
    );
    assert!(
        green(terrain_pixel),
        "terrain absent from combined draw: {terrain_pixel:?}"
    );
    assert_eq!(
        pixel(
            &initial,
            [FP::from_raw(-32768), FP::ZERO, FP::from_raw(-163840)],
            &camera,
            size
        ),
        [0, 0, 0, 255],
        "hole was filled by another pass"
    );
    // Interior point of the first opening cell witnesses path composition and
    // turns red when that cell is removed under the old active path.
    let old_portal = *old_path
        .waypoints()
        .iter()
        .find(|p| {
            p[0] > FP::from_int(-1)
                && p[0] < FP::ZERO
                && p[2] > FP::from_int(2)
                && p[2] < FP::from_int(3)
        })
        .expect("path passes opening cell 3,6");
    assert!(
        cyan(pixel(&initial, old_portal, &camera, size)),
        "path absent from combined terrain draw"
    );
    for _ in 0..6 {
        agent
            .advance(
                &before.graph,
                &before.terrain,
                before.scenario.distance_per_tick(),
            )
            .unwrap();
    }
    let middle_position = agent.position();
    assert_ne!(middle_position, initial_position);
    assert_eq!(agent.status(), NavigationStatus::Moving);
    let middle = capture(&device, &before, &agent, PathDisplay::Current, size);
    assert_ne!(initial, middle);
    assert!(
        orange(pixel(&middle, middle_position, &camera, size)),
        "midtick agent not at new simulated position"
    );
    assert!(
        !orange(pixel(&middle, initial_position, &camera, size)),
        "midtick agent remained at old position"
    );
    let mut ticks = 6;
    while agent.status() != NavigationStatus::Arrived {
        assert!(ticks < 4096, "agent failed bounded arrival");
        agent
            .advance(
                &before.graph,
                &before.terrain,
                before.scenario.distance_per_tick(),
            )
            .unwrap();
        ticks += 1;
    }
    let arrival_position = agent.position();
    assert_eq!(
        [arrival_position[0], arrival_position[2]],
        before.scenario.goal()
    );
    let arrival = capture(&device, &before, &agent, PathDisplay::Current, size);
    assert_ne!(middle, arrival);
    assert!(orange(pixel(&arrival, arrival_position, &camera, size)));
    assert!(!orange(pixel(&arrival, middle_position, &camera, size)));
    // Restore the actual deterministic midtick state, then edit/cook/reinstall.
    agent = Navigator::new(&before.graph, &before.terrain, before.scenario.start()).unwrap();
    agent
        .replan(
            &before.graph,
            &before.terrain,
            before.scenario.goal(),
            SearchBudget::default(),
        )
        .unwrap();
    for _ in 0..6 {
        agent
            .advance(
                &before.graph,
                &before.terrain,
                before.scenario.distance_per_tick(),
            )
            .unwrap();
    }
    assert_eq!(agent.position(), middle_position);
    let mut document = TerrainDocument::new(before.terrain.clone());
    document
        .apply(&[Edit::SetHole {
            x: 3,
            z: 6,
            hole: true,
        }])
        .unwrap();
    let after = package::install_and_reload(
        document.terrain(),
        &root,
        &tmp.path().join("source-v2"),
        "1.0.1",
    )
    .unwrap();
    assert_ne!(before.package_digest, after.package_digest);
    let frozen = agent.clone();
    assert!(agent
        .advance(&before.graph, &after.terrain, FP::ONE)
        .is_err());
    assert_eq!(agent, frozen);
    assert!(agent
        .advance(&after.graph, &after.terrain, FP::ONE)
        .is_err());
    assert_eq!(agent, frozen);
    assert!(
        gpu::renderer(
            &device,
            &after.terrain,
            agent.path(),
            agent.position(),
            PathDisplay::Current
        )
        .is_err(),
        "stale path displayed as current"
    );
    let stale = capture(&device, &after, &agent, PathDisplay::Stale, size);
    assert_ne!(middle, stale);
    assert!(
        red(pixel(&stale, old_portal, &camera, size)),
        "stale path not visibly red"
    );
    assert!(
        orange(pixel(&stale, middle_position, &camera, size)),
        "stale advance moved agent"
    );
    agent
        .replan(
            &after.graph,
            &after.terrain,
            after.scenario.goal(),
            SearchBudget::default(),
        )
        .unwrap();
    assert_ne!(agent.path().unwrap().waypoints(), old_path.waypoints());
    agent
        .advance(
            &after.graph,
            &after.terrain,
            after.scenario.distance_per_tick(),
        )
        .unwrap();
    let replan_position = agent.position();
    assert_ne!(replan_position, middle_position);
    let replanned = capture(&device, &after, &agent, PathDisplay::Current, size);
    assert_ne!(stale, replanned);
    assert!(orange(pixel(&replanned, replan_position, &camera, size)));
    assert!(!orange(pixel(&replanned, middle_position, &camera, size)));
    assert_eq!(
        pixel(&replanned, old_portal, &camera, size),
        [0, 0, 0, 255],
        "new path still traverses removed opening cell"
    );
    // Package source changes cannot replace loaded simulation or rendered state.
    for path in [
        package::TERRAIN_PATH,
        package::NAVIGATION_PATH,
        package::SCENARIO_PATH,
    ] {
        std::fs::write(tmp.path().join("source-v2").join(path), b"changed source").unwrap();
    }
    let reopened = package::load_installed(&root).unwrap();
    assert_eq!(reopened.terrain, after.terrain);
    assert_eq!(reopened.graph, after.graph);
    assert_eq!(reopened.scenario, after.scenario);
    assert_eq!(
        capture(&device, &reopened, &agent, PathDisplay::Current, size),
        replanned
    );
    let mut renderer = gpu::renderer(
        &device,
        &after.terrain,
        agent.path(),
        agent.position(),
        PathDisplay::Current,
    )
    .unwrap();
    let wide = (640, 320);
    let resized = gpu::capture(&mut renderer, &camera, &gpu::lighting(), wide).unwrap();
    assert_eq!(resized.len(), (wide.0 * wide.1 * 4) as usize);
    assert!(orange(pixel(&resized, replan_position, &camera, wide)));
    assert_eq!(
        gpu::capture(&mut renderer, &camera, &gpu::lighting(), size).unwrap(),
        replanned
    );
    if let Some(output) = std::env::var_os("ORR_NAVIGATION_SCREENSHOTS") {
        let output = std::path::Path::new(&output);
        std::fs::create_dir(output).unwrap();
        for (name, bytes) in [
            ("01-initial", &initial),
            ("02-midtick", &middle),
            ("03-arrival", &arrival),
            ("04-edit-stale", &stale),
            ("05-replan", &replanned),
        ] {
            gpu::save_png(&output.join(format!("{name}.png")), bytes, size).unwrap();
        }
    }
    eprintln!("navigation acceptance PASS: real package/reopen/read_asset -> initial/midtick/arrival -> edit-stale atomic freeze -> replan; GPU pixel positions and terrain/path/agent shared draw; ticks={ticks}; adapter={}; software={}",device.adapter_name(),device.is_software());
}
