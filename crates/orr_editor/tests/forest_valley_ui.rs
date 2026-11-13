//! Forest Valley as an ordinary editor scene made only of `orr_gfx`
//! components: terrains from heightmaps with a surface shader, the sun with
//! shadows, the sky, water, scattered trees and grass, post effects and a
//! camera. The test opens it, looks through the scene camera and checks the
//! picture; with ORR_FOREST_CAPTURE set it also saves the viewport images.
#![cfg(feature = "outdoor")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

mod common;

use common::*;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use orr_editor::game::EditorGame;
use orr_editor::{Editor, EditorApp};
use std::path::{Path, PathBuf};

fn scene_copy(dir: &Path) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenes/forest_valley");
    for sub in ["terrain", "shaders"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
        for entry in std::fs::read_dir(source.join(sub)).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), dir.join(sub).join(entry.file_name())).unwrap();
        }
    }
    let scene = dir.join("forest_valley.scene.yaml");
    std::fs::copy(source.join("forest_valley.scene.yaml"), &scene).unwrap();
    scene
}

fn frames(h: &mut Harness<'_, EditorApp>, n: usize) {
    for _ in 0..n {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}

/// Saves `rgba` (`width` pixels per row) cropped to `rect` as a PNG.
fn save_crop(path: &Path, rgba: &[u8], width: u32, rect: egui::Rect) {
    let (x0, y0, w, h) = (rect.min.x as u32, rect.min.y as u32, rect.width() as u32, rect.height() as u32);
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in y0..y0 + h {
        let start = ((y * width + x0) * 4) as usize;
        data.extend_from_slice(&rgba[start..start + (w * 4) as usize]);
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut encoder = png::Encoder::new(file, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&data).unwrap();
}

#[test]
fn forest_valley_is_drawn_from_engine_components_through_the_scene_camera() {
    let Some((_guard, probe)) = gpu() else { return };
    if probe.is_software() {
        eprintln!("SKIP: this picture check needs a hardware adapter");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let scene = scene_copy(dir.path());
    let setup = egui_wgpu::WgpuSetup::CreateNew(egui_wgpu::WgpuSetupCreateNew::without_display_handle());
    let mut h = Harness::builder().with_size([1600.0, 1000.0]).wgpu_setup(setup).build_eframe(|cc| {
        let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
        editor.sync();
        EditorApp::new(editor, cc.wgpu_render_state.clone())
    });
    frames(&mut h, 4);
    h.get_by_label("Camera view").click();
    // Heightmaps load, the shader compiles and the eye adapts over a few frames.
    frames(&mut h, 30);
    assert!(h.query_by_label_contains("Terrain:").is_none(), "the heightmaps load");

    let image = h.render().expect("kittest wgpu render");
    let rect = h.state().ui.viewport_rect.expect("the viewport was drawn");
    let (width, _) = image.dimensions();
    let rgba = image.as_raw();
    if let Some(folder) = std::env::var_os("ORR_FOREST_CAPTURE") {
        let folder = PathBuf::from(folder);
        std::fs::create_dir_all(&folder).unwrap();
        save_crop(&folder.join("forest_valley_camera.png"), rgba, width, rect);
        image.save(folder.join("forest_valley_editor.png")).unwrap();
        eprintln!("captured into {}", folder.display());
    }
    // Mean color of the viewport rows from `top` to `bottom` (0..1 of its height).
    let mean = |top: f32, bottom: f32| {
        let (x0, x1) = (rect.min.x as u32, rect.max.x as u32);
        let (y0, y1) = ((rect.min.y + rect.height() * top) as u32, (rect.min.y + rect.height() * bottom) as u32);
        let mut sum = [0.0f32; 3];
        let mut n = 0.0;
        for y in y0..y1 {
            for x in x0..x1 {
                let i = ((y * width + x) * 4) as usize;
                for c in 0..3 {
                    sum[c] += f32::from(rgba[i + c]);
                }
                n += 1.0;
            }
        }
        sum.map(|s| s / n)
    };
    let sky = mean(0.02, 0.12);
    let ground = mean(0.75, 0.98);
    assert!(sky[2] > sky[0] + 15.0 && sky[2] > 90.0, "the top of the view is blue sky: {sky:?}");
    assert!(ground[1] > ground[2] && ground[1] > 20.0, "the lower view is green ground and plants: {ground:?}");
    let magenta = (rect.min.y as u32..rect.max.y as u32)
        .flat_map(|y| (rect.min.x as u32..rect.max.x as u32).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let i = ((y * width + x) * 4) as usize;
            rgba[i] > 230 && rgba[i + 1] < 30 && rgba[i + 2] > 230
        })
        .count();
    assert_eq!(magenta, 0, "the terrain shader compiles");
}

/// Holds `keys` down for `ticks` simulation ticks of realtime Play.
fn hold(h: &mut Harness<'_, EditorApp>, keys: &[egui::Key], ticks: u64) {
    for key in keys {
        h.input_mut().events.push(egui::Event::Key { key: *key, physical_key: Some(*key), pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    }
    let start = h.state().editor.sim().head_tick;
    let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while h.state().editor.sim().head_tick < start + ticks {
        assert!(std::time::Instant::now() < end, "{:?}", h.state().editor.status());
        h.state_mut().editor.sync();
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    for key in keys {
        h.input_mut().events.push(egui::Event::Key { key: *key, physical_key: Some(*key), pressed: false, repeat: false, modifiers: egui::Modifiers::NONE });
    }
}

#[test]
fn the_dragon_flies_over_the_valley_with_the_keys_and_the_camera_follows() {
    use orr_editor::editor::input::Phase;
    let Some((_guard, probe)) = gpu() else { return };
    if probe.is_software() {
        eprintln!("SKIP: this picture check needs a hardware adapter");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let scene = scene_copy(dir.path());
    let setup = egui_wgpu::WgpuSetup::CreateNew(egui_wgpu::WgpuSetupCreateNew::without_display_handle());
    let mut h = Harness::builder().with_size([1600.0, 1000.0]).wgpu_setup(setup).build_eframe(|cc| {
        let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
        editor.sync();
        EditorApp::new(editor, cc.wgpu_render_state.clone())
    });
    frames(&mut h, 4);
    let start = h.state().editor.yard_frame().dragons[0].0[0];
    h.get_by_label("Camera view").click();
    h.state_mut().editor.play();
    frames(&mut h, 4);
    h.get_by_label("Take control").click();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while h.state().editor.input_phase() != Phase::Active {
        assert!(std::time::Instant::now() < end, "{:?} {:?} {}", h.state().editor.input_phase(), h.state().editor.status(), h.state().editor.input_hint());
        h.state_mut().editor.sync();
        h.run_steps(1);
    }
    // Climb while turning right, then fly on.
    hold(&mut h, &[egui::Key::D, egui::Key::W], 90);
    hold(&mut h, &[egui::Key::E], 60);
    frames(&mut h, 6);
    let (path, bank) = h.state().editor.yard_frame().dragons[0].clone();
    let head = path[0];
    let moved = ((head[0] - start[0]).powi(2) + (head[2] - start[2]).powi(2)).sqrt();
    assert!(moved > 40.0, "the dragon flew: {start:?} to {head:?}");
    assert!(head[1] > start[1] + 5.0, "it climbed: {start:?} to {head:?}");
    let _ = bank;
    let image = h.render().expect("kittest wgpu render");
    let rect = h.state().ui.viewport_rect.expect("the viewport was drawn");
    let (width, _) = image.dimensions();
    if let Some(folder) = std::env::var_os("ORR_FOREST_CAPTURE") {
        let folder = PathBuf::from(folder);
        std::fs::create_dir_all(&folder).unwrap();
        save_crop(&folder.join("forest_valley_dragon.png"), image.as_raw(), width, rect);
        eprintln!("captured into {}", folder.display());
    }
}
