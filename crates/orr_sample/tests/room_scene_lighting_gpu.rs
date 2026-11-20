//! The picture of a Room that gets its lighting from scene components.
//! A room with the sun that lit every Room before looks like a room without lights, and a
//! room with an environment, a sun and a carried point light looks different and follows the
//! player. Skipped when no GPU adapter is available.
//! cargo test -p orr_sample --features room-project,project-create --test room_scene_lighting_gpu
#![cfg(all(feature = "room-project", feature = "project-create"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

#[path = "support/room_lit_scene.rs"]
mod room_lit_scene;

use orr_bridge::FrameView;
use orr_render::orr_rhi::{Wgpu, WgpuOptions};
use orr_sample::{
    project_create::{create, CreateOptions},
    room_game::RoomInput,
    room_project::PreparedProject,
    room_view::RoomRenderer,
};
use orr_sim::{PlayerSlot, TickInputs};
use std::{fs, path::Path};

const SIZE: (u32, u32) = (512, 384);

/// A project with `scene` as its scene text.
fn project(root: &Path, name: &str, edit: impl FnOnce(&str) -> String) -> PreparedProject {
    let output = root.join(name);
    create(&CreateOptions { output: output.clone(), template: "room-escape-3d-v1".into(), seed: "lighting-gpu".into() }).unwrap();
    let scene = output.join("room.scene.yaml");
    let text = fs::read_to_string(&scene).unwrap();
    fs::write(&scene, edit(&text)).unwrap();
    PreparedProject::open(&output).unwrap()
}

fn picture(gpu: &Wgpu, project: &PreparedProject, ticks: u32) -> Vec<u8> {
    let mut sim = project.scene().simulation().unwrap();
    for _ in 0..ticks {
        let mut inputs = TickInputs::new(sim.tick() + 1, 1);
        inputs.set_input(PlayerSlot(0), RoomInput { move_x: 1, ..Default::default() });
        sim.step(&inputs);
    }
    let mut renderer = RoomRenderer::new(gpu, SIZE, project.models()).unwrap();
    // The camera of the project, as the player uses it: the room fills the picture.
    let view = project.camera().expect("the template declares a camera");
    let camera = view.document.camera_for_frame(&view.document.orbit(), SIZE, FrameView::of(sim.frame())).unwrap();
    renderer.render(FrameView::of(sim.frame()), project.scene().index(), project.models(), &camera).unwrap();
    let first = renderer.read_rgba8();
    renderer.render(FrameView::of(sim.frame()), project.scene().index(), project.models(), &camera).unwrap();
    assert_eq!(first, renderer.read_rgba8(), "drawing twice gives the same picture");
    first
}

/// (differing pixels, largest channel difference, mean absolute difference)
fn difference(a: &[u8], b: &[u8]) -> (usize, u8, f64) {
    assert_eq!(a.len(), b.len());
    let pixels = a.chunks(4).zip(b.chunks(4)).filter(|(x, y)| x != y).count();
    let largest = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
    let mean = a.iter().zip(b).map(|(x, y)| f64::from(x.abs_diff(*y))).sum::<f64>() / a.len() as f64;
    (pixels, largest, mean)
}

#[test]
fn the_picture_follows_the_lights_of_the_scene() {
    let Ok(gpu) = Wgpu::headless(WgpuOptions::default()) else {
        eprintln!("SKIP: no GPU adapter");
        return;
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let plain = project(&root, "plain", str::to_owned);
    let sun = project(&root, "sun", room_lit_scene::with_default_sun);
    let lit = project(&root, "lit", room_lit_scene::with_lights);

    let plain_start = picture(&gpu, &plain, 0);
    // The sun that lit every Room draws the picture of a room with no light, but for the
    // rounding of the fixed-point direction and color (a few hundred pixels, one level off).
    let (pixels, largest, _) = difference(&plain_start, &picture(&gpu, &sun, 0));
    assert!(largest <= 2 && pixels < 2_000, "default sun: {pixels} pixels differ, by up to {largest}");

    // An environment, a sun and a point light make another picture.
    let lit_start = picture(&gpu, &lit, 0);
    let (pixels, _, mean) = difference(&plain_start, &lit_start);
    assert!(pixels > 10_000 && mean > 1.0, "lights changed {pixels} pixels, mean {mean}");

    // The point light follows the walking player, so the picture moves with it.
    let (pixels, _, _) = difference(&lit_start, &picture(&gpu, &lit, 40));
    assert!(pixels > 1_000, "walking changed only {pixels} pixels");
}
