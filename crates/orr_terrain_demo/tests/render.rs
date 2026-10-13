#![cfg(feature = "render")]
#![allow(clippy::float_arithmetic)]

use orr_fp::FP;
use orr_render::{
    orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions},
    Camera3D, Projection,
};
use orr_terrain::Terrain;
use orr_terrain_demo::{
    render::{self, Scene},
    Course, Game, Input,
};
use std::{
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

#[path = "common/mod.rs"]
mod common;

fn fixed(integer: i64) -> FP {
    FP::from_raw(integer * 65536)
}

fn fixture(heights: Vec<FP>, spawn: [&str; 2]) -> Course {
    let terrain = Terrain::new(
        "terrain-view-fixture".into(),
        9,
        9,
        [fixed(-4), fixed(-4)],
        fixed(1),
        heights,
        vec![false; 64],
    )
    .unwrap();
    let json = serde_json::json!({
        "version": 1, "title": "View fixture", "terrain": "view.orrt",
        "spawn": spawn, "beacons": [["-3", "-3"], ["-2", "-3"], ["2", "-3"], ["3", "-3"]],
        "finish": ["3", "-2"], "radius": "0.25"
    });
    Course::from_bytes(&serde_json::to_vec(&json).unwrap(), &terrain.cook()).unwrap()
}

fn walk_axis(game: &mut Game, axis: usize, target: FP) {
    let start_tick = game.state().tick;
    for _ in 0..1024 {
        let value = game.state().anchor[axis];
        if value == target {
            return;
        }
        let direction = if target > value { 1 } else { -1 };
        game.tick(if axis == 0 {
            Input { x: direction, z: 0 }
        } else {
            Input { x: 0, z: direction }
        });
    }
    panic!(
        "scripted path blocked at {:?}, axis {axis}, target {target:?}, start tick {start_tick}",
        game.state()
    );
}

#[test]
fn actor_geometry_uses_sampled_xyz_and_camera_changes_do_not_change_gameplay() {
    let mut game = Game::new(fixture(
        (0..81).map(|i| fixed(i % 9)).collect(),
        ["-3", "0"],
    ));
    let before = game.state();
    let list = render::actors(&game);
    assert_eq!(list.capsules.len(), 1);
    assert_eq!(list.capsules[0].pos, render::explorer_center(&before));
    assert!(
        (list.capsules[0].pos[1]
            - render::EXPLORER_RADIUS
            - render::EXPLORER_HALF_LENGTH
            - before.anchor[1].to_f32())
        .abs()
            < 1.0e-5
    );
    let terrain_bytes = game.course().terrain().cook();
    let mut camera = render::follow_camera(&before);
    for _ in 0..64 {
        game.tick(Input { x: 1, z: 0 });
    }
    let moved = game.state();
    assert!(moved.anchor[1] > before.anchor[1]);
    render::orbit(&mut camera, [20.0, -5.0]);
    render::zoom(&mut camera, 2.0);
    let orbit = (camera.yaw, camera.pitch, camera.distance);
    render::follow_target(&mut camera, &moved);
    assert_eq!((camera.yaw, camera.pitch, camera.distance), orbit);
    assert_eq!(
        camera.target,
        [
            moved.anchor[0].to_f32(),
            moved.anchor[1].to_f32() + 0.7,
            moved.anchor[2].to_f32()
        ]
    );
    assert_eq!(game.state(), moved);
    assert_eq!(game.course().terrain().cook(), terrain_bytes);
    game.restart();
    assert_eq!(game.state(), before);
    assert_eq!(
        render::follow_camera(&game.state()),
        render::follow_camera(&before)
    );
}

#[test]
fn nonfinite_and_extreme_camera_input_remains_bounded() {
    let game = Game::new(Course::builtin().unwrap());
    let mut camera = render::follow_camera(&game.state());
    let initial = camera;
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        render::orbit(&mut camera, [bad, 0.0]);
        render::orbit(&mut camera, [0.0, bad]);
        render::zoom(&mut camera, bad);
        assert_eq!(camera, initial);
    }
    for _ in 0..64 {
        for huge in [-f32::MAX, f32::MAX] {
            render::orbit(&mut camera, [huge, huge]);
            render::zoom(&mut camera, huge);
            assert!((render::MIN_PITCH..=render::MAX_PITCH).contains(&camera.pitch));
            assert!((render::MIN_DISTANCE..=render::MAX_DISTANCE).contains(&camera.distance));
            assert!(camera.eye().into_iter().all(f32::is_finite));
            assert!(camera.eye()[1] > camera.target[1]);
        }
    }
}

const SIZE: (u32, u32) = (480, 360);
const SMALL_SIZE: (u32, u32) = (320, 240);
const FORMAT: TextureFormat = TextureFormat::Rgba8UnormSrgb;

type Bounds = [[f64; 3]; 2];

// Frozen before successor execution: the original distance 9.5 numeric pass
// still had clipped stage 2/3 goals. This independent framing gate covers the
// full artwork rather than treating any other gold checkpoint as the target.
const FRAME_MARGIN_PX: f64 = 16.0;

fn explorer_bounds(game: &Game) -> Bounds {
    let [x, y, z] = game.state().anchor.map(|v| f64::from(v.to_f32()));
    // Capsule radius 0.24/half-length 0.32 plus the white cap's top at 1.13.
    [[x - 0.24, y, z - 0.24], [x + 0.24, y + 1.13, z + 0.24]]
}

struct Goal {
    bounds: Bounds,
    probes: Vec<[f64; 3]>,
    won: bool,
}

fn active_goal(game: &Game) -> Goal {
    let state = game.state();
    if state.collected < 4 {
        let index = usize::from(state.collected);
        let [x, y, z] = game.course().beacons()[index].map(|v| f64::from(v.to_f32()));
        // Independently enumerate each authored digit's actual stroke centers,
        // not a generic number rectangle or a global gold-fragment count.
        let strokes: &[[f64; 2]] = match index {
            0 => &[[0.16, 0.14], [0.16, -0.14]],
            1 => &[
                [0.0, 0.28],
                [0.16, 0.14],
                [0.0, 0.0],
                [-0.16, -0.14],
                [0.0, -0.28],
            ],
            2 => &[
                [0.0, 0.28],
                [0.16, 0.14],
                [0.16, -0.14],
                [0.0, -0.28],
                [0.0, 0.0],
            ],
            _ => &[[-0.16, 0.14], [0.16, 0.14], [0.16, -0.14], [0.0, 0.0]],
        };
        let mut probes = vec![[x, y + 0.85, z]];
        probes.extend(
            strokes
                .iter()
                .map(|offset| [x + offset[0], y + 1.42 + offset[1], z]),
        );
        Goal {
            // Includes the full pole, active sphere and seven-segment boxes.
            bounds: [[x - 0.28, y, z - 0.28], [x + 0.28, y + 1.728, z + 0.28]],
            probes,
            won: false,
        }
    } else {
        let [x, y, z] = game.course().finish().map(|v| f64::from(v.to_f32()));
        Goal {
            bounds: [[x - 0.06, y, z - 0.06], [x + 0.65, y + 1.6, z + 0.06]],
            probes: vec![[x + 0.35, y + 1.4, z]],
            won: state.finished,
        }
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(v: [f64; 3]) -> [f64; 3] {
    let length = dot(v, v).sqrt();
    v.map(|v| v / length)
}

/// Direct perspective basis calculation, independent of the production matrix
/// and world_to_screen helpers. Inputs are the actual production camera fields.
fn project(camera: &Camera3D, point: [f64; 3], size: (u32, u32)) -> [f64; 2] {
    let eye = camera.eye.map(f64::from);
    let target = camera.target.map(f64::from);
    let forward = unit(std::array::from_fn(|i| target[i] - eye[i]));
    let right = unit(cross(forward, camera.up.map(f64::from)));
    let up = cross(right, forward);
    let offset = std::array::from_fn(|i| point[i] - eye[i]);
    let depth = dot(offset, forward);
    assert!(depth > 0.0, "framing point is behind the camera");
    let Projection::Perspective { fov_y, .. } = camera.projection else {
        panic!("default framing oracle requires the production perspective camera");
    };
    let half_height = depth * (f64::from(fov_y) * 0.5).tan();
    let half_width = half_height * f64::from(size.0) / f64::from(size.1);
    [
        (0.5 + dot(offset, right) / (2.0 * half_width)) * f64::from(size.0),
        (0.5 - dot(offset, up) / (2.0 * half_height)) * f64::from(size.1),
    ]
}

fn projected_bounds(camera: &Camera3D, bounds: Bounds, size: (u32, u32)) -> [f64; 4] {
    let mut result = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for corner in 0..8 {
        let point =
            std::array::from_fn(|axis| bounds[usize::from(corner & (1 << axis) != 0)][axis]);
        let p = project(camera, point, size);
        result[0] = result[0].min(p[0]);
        result[1] = result[1].min(p[1]);
        result[2] = result[2].max(p[0]);
        result[3] = result[3].max(p[1]);
    }
    result
}

fn assert_framing(game: &Game, size: (u32, u32)) {
    let camera = render::follow_camera(&game.state()).camera();
    for (name, bounds) in [
        ("explorer", explorer_bounds(game)),
        ("active goal", active_goal(game).bounds),
    ] {
        let b = projected_bounds(&camera, bounds, size);
        assert!(
            b[0] >= FRAME_MARGIN_PX
                && b[1] >= FRAME_MARGIN_PX
                && b[2] <= f64::from(size.0) - FRAME_MARGIN_PX
                && b[3] <= f64::from(size.1) - FRAME_MARGIN_PX,
            "{name} needs the frozen 16px margin: size={size:?}, bounds={b:?}, state={:?}",
            game.state()
        );
    }
}

#[test]
fn default_follow_frames_explorer_and_active_goal_through_every_script_tick() {
    let mut game = Game::new(Course::builtin().unwrap());
    assert_terrain_cannot_satisfy_won_green(&game);
    let sizes = [(960, 640), SIZE, SMALL_SIZE];
    for size in sizes {
        assert_framing(&game, size);
    }
    let mut inputs = 0;
    for (input, ticks) in common::builtin_script() {
        for _ in 0..ticks {
            game.tick(input);
            inputs += 1;
            // The final accepted XYZ after both X/Z controller substeps is the
            // state the real window follows; no interpolated route is invented.
            for size in sizes {
                assert_framing(&game, size);
            }
        }
    }
    assert_eq!(inputs, 448);
    assert!(game.state().finished);
}

fn gold(pixel: &[u8]) -> bool {
    pixel[0] >= 180
        && pixel[1] >= 120
        && i16::from(pixel[0]) - i16::from(pixel[2]) >= 60
        && i16::from(pixel[1]) - i16::from(pixel[2]) >= 45
}

fn won_green(pixel: &[u8]) -> bool {
    pixel[1] >= 240
        && i16::from(pixel[1]) - i16::from(pixel[0]) >= 20
        && i16::from(pixel[1]) - i16::from(pixel[2]) >= 10
}

fn assert_terrain_cannot_satisfy_won_green(game: &Game) {
    // shader_model.wgsl uses diffuse texture*(ambient+sun+point). Here point
    // lights/probes are absent, exposure is 1, normals are normalized, and ACES
    // is followed by one sRGB encode. Bound every possible terrain normal.
    let model = orr_terrain_view::to_static_model(game.course().terrain(), &[])
        .unwrap()
        .unwrap();
    let max_green = model
        .source()
        .images
        .iter()
        .flat_map(|image| image.rgba8.chunks_exact(4).map(|pixel| pixel[1]))
        .max()
        .unwrap();
    let encoded = f64::from(max_green) / 255.0;
    let linear = if encoded <= 0.04045 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    };
    let light = render::lighting();
    assert!(light.tonemap);
    assert_eq!(light.exposure, 1.0);
    let illumination = f64::from(
        light.sky[1].max(light.ground[1]) * light.ambient + light.color[1] * light.intensity,
    );
    let x = linear * illumination;
    let aces = ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0);
    let display = if aces <= 0.0031308 {
        aces * 12.92
    } else {
        1.055 * aces.powf(1.0 / 2.4) - 0.055
    } * 255.0;
    assert!(
        display < 230.0,
        "terrain upper bound {display} must remain separated from the won-flag threshold 240"
    );
}

fn assert_stage_fragments(pixels: &[u8], game: &Game, size: (u32, u32)) {
    assert_framing(game, size);
    let camera = render::follow_camera(&game.state()).camera();
    let bounds = projected_bounds(&camera, explorer_bounds(game), size);
    let visible_blue = blue_mask(pixels)
        .into_iter()
        .filter(|index| {
            let x = (index % size.0 as usize) as f64 + 0.5;
            let y = (index / size.0 as usize) as f64 + 0.5;
            (bounds[0]..=bounds[2]).contains(&x) && (bounds[1]..=bounds[3]).contains(&y)
        })
        .count();
    assert!(
        visible_blue >= 8,
        "predicted explorer needs 8 blue pixels, got {visible_blue}, size={size:?}"
    );
    let goal = active_goal(game);
    for (probe, point) in goal.probes.iter().enumerate() {
        let [x, y] = project(&camera, *point, size);
        let center = [x.floor() as i32, y.floor() as i32];
        let mut fragments = 0;
        for py in center[1] - 2..=center[1] + 2 {
            for px in center[0] - 2..=center[0] + 2 {
                if px >= 0 && py >= 0 && px < size.0 as i32 && py < size.1 as i32 {
                    let index = (py as usize * size.0 as usize + px as usize) * 4;
                    let pixel = &pixels[index..index + 4];
                    let matches = if goal.won {
                        won_green(pixel)
                    } else {
                        gold(pixel)
                    };
                    if matches {
                        fragments += 1;
                    }
                }
            }
        }
        eprintln!("active goal probe {probe}: fragments={fragments}, point={point:?}, screen={:?}, size={size:?}, state={:?}", [x,y], game.state());
        assert!(fragments >= 1,
            "active goal probe {probe} has no declared-color fragment in its 5x5 region: point={point:?}, screen={:?}, size={size:?}, state={:?}", [x,y], game.state());
    }
}

fn new_evidence_file(path: &Path) -> fs::File {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .expect("capture evidence must use fresh filenames; never overwrite previous results")
}

#[allow(clippy::too_many_arguments)]
fn capture(
    gpu: &Wgpu,
    scene: &mut Scene<Wgpu>,
    game: &Game,
    camera: &Camera3D,
    size: (u32, u32),
    reversed: bool,
    name: &str,
) -> Vec<u8> {
    let texture = gpu.create_texture(&TextureDesc {
        label: "terrain course acceptance",
        width: size.0,
        height: size.1,
        format: FORMAT,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let view = gpu.create_texture_view(&texture, None);
    if reversed {
        scene.draw_reversed(&view, size, game, camera).unwrap();
    } else {
        scene.draw(&view, size, game, camera).unwrap();
    }
    let pixels = gpu.read_texture(&texture);
    assert_eq!(pixels.len(), (size.0 * size.1 * 4) as usize);
    eprintln!(
        "terrain capture {name}: size={size:?}, state={:?}",
        game.state()
    );
    if let Some(directory) = std::env::var_os("ORR_TERRAIN_CAPTURE_DIR") {
        let directory = PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        new_evidence_file(&directory.join(format!("{name}.rgba")))
            .write_all(&pixels)
            .unwrap();
        let file = new_evidence_file(&directory.join(format!("{name}.png")));
        let mut encoder = png::Encoder::new(BufWriter::new(file), size.0, size.1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        let state = game.state();
        let metadata = serde_json::json!({
            "fixture": name, "size": [size.0, size.1], "reversed": reversed,
            "tick": state.tick, "anchor_raw": state.anchor.map(|v| v.raw()),
            "collected": state.collected, "finished": state.finished,
            "terrain_revision": game.course().terrain().revision(),
            "adapter": gpu.adapter_name(), "software": gpu.is_software(),
            "camera_eye": camera.eye, "camera_target": camera.target,
        });
        new_evidence_file(&directory.join(format!("{name}.json")))
            .write_all(&serde_json::to_vec_pretty(&metadata).unwrap())
            .unwrap();
    }
    pixels
}

fn blue_mask(pixels: &[u8]) -> Vec<usize> {
    pixels
        .chunks_exact(4)
        .enumerate()
        .filter_map(|(index, p)| {
            (i16::from(p[2]) > i16::from(p[0]) + 70 && i16::from(p[2]) > i16::from(p[1]) + 35)
                .then_some(index)
        })
        .collect()
}

fn assert_side_projection(pixels: &[u8], game: &Game) {
    // Independent orthographic oracle for eye [0,4,16], target [0,4,0],
    // half-height5. Do not call the production world_to_screen helper.
    let mask = blue_mask(pixels);
    assert!(
        mask.len() > 25,
        "explorer must produce actual blue fragments"
    );
    let mean = mask
        .iter()
        .fold([0.0; 2], |sum, &index| {
            [
                sum[0] + (index % SIZE.0 as usize) as f32 + 0.5,
                sum[1] + (index / SIZE.0 as usize) as f32 + 0.5,
            ]
        })
        .map(|sum| sum / mask.len() as f32);
    let state = game.state();
    let x = state.anchor[0].to_f32();
    let y = state.anchor[1].to_f32() + render::EXPLORER_HALF_LENGTH + render::EXPLORER_RADIUS;
    let scale = SIZE.1 as f32 / 10.0;
    let expected = [
        SIZE.0 as f32 / 2.0 + x * scale,
        SIZE.1 as f32 / 2.0 - (y - 4.0) * scale,
    ];
    assert!(
        (mean[0] - expected[0]).abs() < 2.5,
        "x centroid {mean:?} vs {expected:?}"
    );
    // The white cap replaces a small part of the top blue silhouette.
    assert!(
        (mean[1] - expected[1]).abs() < 3.5,
        "sampled-height centroid {mean:?} vs {expected:?}"
    );
}

/// Explicit opt-in: absence of an adapter is a failure, never a skipped success.
/// This proves production offscreen rendering, not a native window or inputs.
#[test]
#[ignore = "requires an explicitly admitted GPU run; set ORR_TERRAIN_CAPTURE_DIR to retain PNG/raw evidence"]
fn production_shared_depth_projection_progress_resize_and_restart() {
    let gpu = Wgpu::headless(WgpuOptions::default())
        .expect("terrain render acceptance requires an actual adapter");
    eprintln!(
        "terrain acceptance adapter: {} (software: {})",
        gpu.adapter_name(),
        gpu.is_software()
    );
    let mut executed = 0;

    // Real accepted FP movement up a ramp changes only the actor anchor; the
    // camera and immutable terrain stay fixed for both projection comparisons.
    let mut ramp = Game::new(fixture(
        (0..81).map(|i| fixed(i % 9)).collect(),
        ["-3", "0"],
    ));
    let ramp_bytes = ramp.course().terrain().cook();
    let mut scene = Scene::new(gpu.clone(), FORMAT, ramp.course()).unwrap();
    let side = Camera3D::orthographic([0.0, 4.0, 16.0], [0.0, 4.0, 0.0], 5.0);
    for (name, target) in [("ramp-start", -3), ("ramp-uphill", 1)] {
        walk_axis(&mut ramp, 0, fixed(target));
        let pixels = capture(&gpu, &mut scene, &ramp, &side, SIZE, false, name);
        assert_side_projection(&pixels, &ramp);
        executed += 1;
    }
    assert_eq!(ramp.course().terrain().cook(), ramp_bytes);
    assert_eq!(scene.depth_generation(), 1);

    // A cross-course ridge lies between the camera and the far explorer. Both
    // endpoints have the same y=0 and the same independent screen projection.
    // Near is visible, far is hidden, in both actual batch submission orders.
    let heights = (0..81)
        .map(|i| if i / 9 == 5 { fixed(3) } else { FP::ZERO })
        .collect();
    let mut ridge = Game::new(fixture(heights, ["0", "3"]));
    let mut scene = Scene::new(gpu.clone(), FORMAT, ridge.course()).unwrap();
    let front = Camera3D::orthographic([0.0, 2.0, 12.0], [0.0, 2.0, 0.0], 4.0);
    for (name, target, visible) in [("ridge-near", 3, true), ("ridge-far", -2, false)] {
        walk_axis(&mut ridge, 2, fixed(target));
        assert_eq!(ridge.state().anchor[1], FP::ZERO);
        let pixels = capture(&gpu, &mut scene, &ridge, &front, SIZE, false, name);
        let reversed = capture(
            &gpu,
            &mut scene,
            &ridge,
            &front,
            SIZE,
            true,
            &format!("{name}-reversed"),
        );
        let mask = blue_mask(&pixels);
        assert_eq!(
            mask,
            blue_mask(&reversed),
            "shared depth must not depend on batch order"
        );
        if visible {
            assert!(mask.len() > 25);
        } else {
            assert!(mask.is_empty(), "ridge must occlude the far explorer");
        }
        executed += 2;
    }

    let mut game = Game::new(Course::builtin().unwrap());
    let bytes = game.course().terrain().cook();
    let mut scene = Scene::new(gpu.clone(), FORMAT, game.course()).unwrap();
    let start_camera = render::follow_camera(&game.state()).camera();
    let start = capture(
        &gpu,
        &mut scene,
        &game,
        &start_camera,
        SIZE,
        false,
        "builtin-start",
    );
    assert!(!blue_mask(&start).is_empty());
    assert_stage_fragments(&start, &game, SIZE);
    executed += 1;
    let unchanged = game.state();
    let mut orbit_camera = render::follow_camera(&unchanged);
    render::orbit(&mut orbit_camera, [110.0, 18.0]);
    render::zoom(&mut orbit_camera, 1.0);
    let orbited = capture(
        &gpu,
        &mut scene,
        &game,
        &orbit_camera.camera(),
        SIZE,
        false,
        "builtin-orbit",
    );
    assert_ne!(
        start, orbited,
        "the production image must respond to orbit/zoom"
    );
    assert_eq!(game.state(), unchanged);
    executed += 1;
    for (segment, (input, ticks)) in common::builtin_script().into_iter().enumerate() {
        for _ in 0..ticks {
            game.tick(input);
        }
        if segment % 2 == 1 {
            let checkpoint = segment / 2 + 1;
            assert_eq!(game.state().collected, checkpoint.min(4) as u8);
            let progress = render::actors(&game);
            assert!(progress.spheres[1].color[1] > progress.spheres[1].color[0]);
            let camera = render::follow_camera(&game.state()).camera();
            let pixels = capture(
                &gpu,
                &mut scene,
                &game,
                &camera,
                SIZE,
                false,
                &format!("builtin-stage-{checkpoint}"),
            );
            assert_stage_fragments(&pixels, &game, SIZE);
            executed += 1;
        }
    }
    assert!(game.state().finished);
    assert_eq!(scene.depth_generation(), 1);
    let camera = render::follow_camera(&game.state()).camera();
    let small_win = capture(
        &gpu,
        &mut scene,
        &game,
        &camera,
        (320, 240),
        false,
        "builtin-small-window",
    );
    assert_stage_fragments(&small_win, &game, SMALL_SIZE);
    assert_eq!(scene.depth_size(), Some((320, 240)));
    assert_eq!(scene.depth_generation(), 2);
    game.restart();
    let reset_camera = render::follow_camera(&game.state()).camera();
    let reset = capture(
        &gpu,
        &mut scene,
        &game,
        &reset_camera,
        SIZE,
        false,
        "builtin-reset",
    );
    assert_eq!(
        start, reset,
        "restart must recreate the exact baseline image"
    );
    assert_eq!(game.course().terrain().cook(), bytes);
    assert_eq!(scene.depth_generation(), 3);
    executed += 2;

    // Approach the authored hole from supported +Z; repeated negative-Z input
    // cannot enter it. Capture that accepted anchor using the production view.
    walk_axis(&mut game, 0, FP::ZERO);
    walk_axis(&mut game, 2, fixed(2));
    let at_edge = game.state().anchor;
    for _ in 0..32 {
        game.tick(Input { x: 0, z: -1 });
    }
    assert_eq!(game.state().anchor, at_edge);
    let camera = render::follow_camera(&game.state()).camera();
    capture(
        &gpu,
        &mut scene,
        &game,
        &camera,
        SIZE,
        false,
        "builtin-blocked-hole",
    );
    executed += 1;

    let mut top = Camera3D::orthographic([0.0, 20.0, 1.0], [0.0, 0.0, 1.0], 5.0);
    top.up = [0.0, 0.0, -1.0];
    let hole = capture(
        &gpu,
        &mut scene,
        &game,
        &top,
        SIZE,
        false,
        "builtin-hole-top",
    );
    let center = ((SIZE.1 / 2 * SIZE.0 + SIZE.0 / 2) * 4) as usize;
    for channel in 0..3 {
        let linear = render::CLEAR[channel];
        let srgb = if linear <= 0.0031308 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        };
        assert!(
            (f64::from(hole[center + channel]) - srgb * 255.0).abs() <= 1.0,
            "the authored hole must expose the clear background at its center"
        );
    }
    executed += 1;

    // A mismatched public Game must fail before touching existing target/depth.
    let texture = gpu.create_texture(&TextureDesc {
        label: "terrain invalid-frame acceptance",
        width: SIZE.0,
        height: SIZE.1,
        format: FORMAT,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let view = gpu.create_texture_view(&texture, None);
    scene.draw(&view, SIZE, &game, &camera).unwrap();
    let valid = gpu.read_texture(&texture);
    let generation = scene.depth_generation();
    assert!(scene.draw(&view, SIZE, &ramp, &camera).is_err());
    assert!(scene.draw(&view, (0, SIZE.1), &game, &camera).is_err());
    assert_eq!(gpu.read_texture(&texture), valid);
    assert_eq!(scene.depth_generation(), generation);
    assert_eq!(executed, 17, "every explicit GPU fixture must execute");

    // Five additional original readbacks, with the same immutable terrain
    // renderer. Leave every predecessor resize/depth/restart assertion intact.
    // The existing builtin-small-window capture already covers the won stage.
    let mut small_game = Game::new(Course::builtin().unwrap());
    let camera = render::follow_camera(&small_game.state()).camera();
    let pixels = capture(
        &gpu,
        &mut scene,
        &small_game,
        &camera,
        SMALL_SIZE,
        false,
        "builtin-small-start",
    );
    assert_stage_fragments(&pixels, &small_game, SMALL_SIZE);
    executed += 1;
    for (segment, (input, ticks)) in common::builtin_script().into_iter().enumerate() {
        for _ in 0..ticks {
            small_game.tick(input);
        }
        if segment % 2 == 1 && segment < 8 {
            let stage = segment / 2 + 1;
            let camera = render::follow_camera(&small_game.state()).camera();
            let pixels = capture(
                &gpu,
                &mut scene,
                &small_game,
                &camera,
                SMALL_SIZE,
                false,
                &format!("builtin-small-stage-{stage}"),
            );
            assert_stage_fragments(&pixels, &small_game, SMALL_SIZE);
            executed += 1;
        }
    }
    assert!(small_game.state().finished);
    assert_eq!(
        executed, 22,
        "all original and added framing captures must execute"
    );
    eprintln!("terrain acceptance executed {executed} GPU captures; native window interaction was not tested");
}
