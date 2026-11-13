//! Writes Forest Valley as an ordinary engine scene made of `orr_gfx`
//! components: two heightmap terrains with the `terrain` surface shader, the
//! sun, sky and post effects, a camera, one water surface per pool of the
//! stream, and scatters of trees, rocks and grass.
//!
//! ```text
//! cargo run -p orr_scenery --example forest_valley_scene -- scenes/forest_valley
//! ```
//!
//! The folder then holds `forest_valley.scene.yaml` and `terrain/*.png`; the
//! shader `shaders/terrain.wgsl` is kept in the repository.
#![allow(clippy::float_arithmetic)]

use orr_scenery::world::{HALF, GRID};
use orr_scenery::{viewpoints, Settings, World};
use std::fmt::Write as _;
use std::path::PathBuf;

const SEED: u32 = 7;
/// Side of the far terrain in meters and its grid.
const FAR_SIDE: f32 = 6144.0;
const FAR_GRID: usize = 257;

fn write_png16(path: &std::path::Path, side: usize, heights: &[f32], low: f32, high: f32) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), side as u32, side as u32);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Sixteen);
    let mut writer = encoder.write_header().unwrap();
    let data: Vec<u8> = heights.iter().flat_map(|h| ((((h - low) / (high - low)).clamp(0.0, 1.0) * 65535.0).round() as u16).to_be_bytes()).collect();
    writer.write_image_data(&data).unwrap();
}

fn v3(v: [f32; 3]) -> String {
    format!("[{:.3}, {:.3}, {:.3}]", v[0], v[1], v[2])
}

/// Pitch and yaw in degrees of an entity that faces `d`.
fn facing(d: [f32; 3]) -> [f32; 3] {
    let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let d = d.map(|c| c / length);
    [(-d[1]).asin().to_degrees(), d[0].atan2(d[2]).to_degrees(), 0.0]
}

fn main() {
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "scenes/forest_valley".into()));
    std::fs::create_dir_all(out.join("terrain")).unwrap();
    let world = World::generate(SEED);
    let settings = Settings::default();

    // The detailed valley: the generator's own grid, 2 m cells over 1024 m.
    let (low, high) = world.heights.iter().fold((f32::MAX, f32::MIN), |(a, b), h| (a.min(*h), b.max(*h)));
    let (low, high) = (low.floor(), high.ceil());
    write_png16(&out.join("terrain/valley.png"), GRID, &world.heights, low, high);

    // Far ridges around it, sunk a little under the detailed square so the two never fight.
    let mut far = Vec::with_capacity(FAR_GRID * FAR_GRID);
    for j in 0..FAR_GRID {
        for i in 0..FAR_GRID {
            let x = (i as f32 / (FAR_GRID - 1) as f32 - 0.5) * FAR_SIDE;
            let z = (j as f32 / (FAR_GRID - 1) as f32 - 0.5) * FAR_SIDE;
            let inside = x.abs() < HALF - 30.0 && z.abs() < HALF - 30.0;
            far.push(world.ground(x, z) - if inside { 12.0 } else { 1.5 });
        }
    }
    let (far_low, far_high) = far.iter().fold((f32::MAX, f32::MIN), |(a, b), h| (a.min(*h), b.max(*h)));
    let (far_low, far_high) = (far_low.floor(), far_high.ceil());
    write_png16(&out.join("terrain/ridges.png"), FAR_GRID, &far, far_low, far_high);

    let sun = settings.sun_direction();
    let view = viewpoints(&world)[0];
    let look = view.forward();
    let mut text = String::new();
    let mut guid = 0u32;
    let mut entity = |text: &mut String, name: &str| {
        guid += 1;
        writeln!(text, "  e_{guid:08x}:\n    name: {name}").unwrap();
        format!("e_{guid:08x}")
    };
    text.push_str("# Forest Valley built only from engine components. Written by\n");
    text.push_str("# `cargo run -p orr_scenery --example forest_valley_scene`.\n");
    text.push_str("schema: orr.scene/1\nsingletons:\n");
    text.push_str("  Yard3dScene: { rain_batch: 0, rain_interval: 0, max_entities: 128, rain_seed: 42 }\n");
    writeln!(text, "  orr_gfx::Environment:\n    sky_color: [0.42, 0.58, 0.9]\n    ground_color: [0.22, 0.2, 0.12]\n    ambient: 1").unwrap();
    text.push_str("  orr_gfx::Sky:\n    brightness: 1.5\n    sun_glow: 1\n    cloud_cover: 0.34\n    cloud_speed: 1\n    fog_density: 0.7\n    fog_falloff: 0.03\n");
    writeln!(text, "    fog_height: {:.1}\n    haze: 0.08\n    wind_strength: 1\n    wind_speed: 1\n    enabled: true", world.stream(view.position[2]).level).unwrap();
    text.push_str("  orr_gfx::PostProcess:\n    exposure: 1\n    tone_mapping: aces\n    effects:\n");
    for line in [
        "{ kind: auto_exposure, enabled: true, params: [0.55, 4.5, 2.2, 0.165] }",
        "{ kind: bloom, enabled: true, params: [0.9, 0.1, 4, 2] }",
        "{ kind: light_shafts, enabled: true, params: [0.35, 0.85, 0, 0] }",
        "{ kind: color_grade, enabled: true, params: [1.08, 0.18, 0.15, 0] }",
        "{ kind: vignette, enabled: true, params: [0.3, 0.85, 0.5, 0] }",
        "{ kind: film_grain, enabled: true, params: [0.04, 0, 0, 0] }",
        "{ kind: none, enabled: false, params: [0, 0, 0, 0] }",
        "{ kind: none, enabled: false, params: [0, 0, 0, 0] }",
    ] {
        writeln!(text, "      - {line}").unwrap();
    }
    // A coarse copy of the ground for the dragon's flight: the highest point near each grid point.
    text.push_str("  orr_games::DragonGround:\n    center: [0, 0]\n    size: 1024\n    heights:\n");
    for j in 0..32 {
        let mut row = Vec::new();
        for i in 0..32 {
            let (x, z) = ((i as f32 / 31.0 - 0.5) * 1024.0, (j as f32 / 31.0 - 0.5) * 1024.0);
            let mut high = f32::MIN;
            for dz in -2..=2 {
                for dx in -2..=2 {
                    let (sx, sz) = (x + dx as f32 * 8.0, z + dz as f32 * 8.0);
                    high = high.max(world.height(sx, sz).max(world.stream(sz).level));
                }
            }
            row.push(format!("{high:.1}"));
        }
        for h in row {
            writeln!(text, "      - {h}").unwrap();
        }
    }
    text.push_str("entities:\n");

    let terrain_material = |text: &mut String| {
        text.push_str("    orr_gfx::Material:\n      shader: terrain\n      values: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]\n      overrides: 0\n");
    };
    let valley = entity(&mut text, "valley");
    writeln!(text, "    orr_gfx::Transform3:\n      position: [0, {low:.1}, 0]\n      rotation: [0, 0, 0]\n      scale: [1, 1, 1]").unwrap();
    writeln!(text, "    orr_gfx::Terrain:\n      heightmap: terrain/valley.png\n      size: [{s}, {s}]\n      height: {h:.1}\n      resolution: 352\n      cast_shadows: true", s = 2.0 * HALF, h = high - low).unwrap();
    terrain_material(&mut text);
    entity(&mut text, "far_ridges");
    writeln!(text, "    orr_gfx::Transform3:\n      position: [0, {far_low:.1}, 0]\n      rotation: [0, 0, 0]\n      scale: [1, 1, 1]").unwrap();
    writeln!(text, "    orr_gfx::Terrain:\n      heightmap: terrain/ridges.png\n      size: [{FAR_SIDE}, {FAR_SIDE}]\n      height: {:.1}\n      resolution: 256\n      cast_shadows: false", far_high - far_low).unwrap();
    terrain_material(&mut text);

    entity(&mut text, "sun");
    writeln!(text, "    orr_gfx::Transform3:\n      position: [0, 200, 0]\n      rotation: {}\n      scale: [1, 1, 1]", v3(facing([-sun[0], -sun[1], -sun[2]]))).unwrap();
    text.push_str("    orr_gfx::Light:\n      color: [1, 0.9, 0.76]\n      intensity: 8.5\n      range: 10\n      inner_angle: 20\n      outer_angle: 30\n      shadow_strength: 1\n      shadow_range: 140\n      kind: directional\n      enabled: true\n      cast_shadows: true\n      layers: [default, layer2, layer3, layer4, layer5, layer6, layer7, layer8]\n");

    // The dragon starts ahead of the camera, above the valley, flying the way the camera looks.
    let flat = {
        let l = (look[0] * look[0] + look[2] * look[2]).sqrt();
        [look[0] / l, 0.0, look[2] / l]
    };
    let start = [view.position[0] + flat[0] * 40.0, 0.0, view.position[2] + flat[2] * 40.0];
    let start = [start[0], world.height(start[0], start[2]).max(world.stream(start[2]).level) + 30.0, start[2]];
    let dragon = entity(&mut text, "dragon");
    text.push_str("    orr_games::Dragon:\n");
    writeln!(text, "      position: {}\n      yaw: {:.2}\n      pitch: 0\n      bank: 0\n      speed: 20\n      cruise_speed: 20\n      boost_speed: 45", v3(start), flat[0].atan2(flat[2]).to_degrees()).unwrap();
    text.push_str("      turn_rate: 70\n      climb_rate: 45\n      clearance: 14\n      ceiling: 600\n      bounds: 480\n      player: 0\n      next: 0\n      trail:\n");
    // A straight path behind the head, oldest first, so the body lies out before the first tick.
    for k in 0..48 {
        let back = 1.25 * (48 - k) as f32;
        writeln!(text, "        - {}", v3([start[0] - flat[0] * back, start[1], start[2] - flat[2] * back])).unwrap();
    }

    entity(&mut text, "camera");
    writeln!(text, "    orr_gfx::Transform3:\n      position: {}\n      rotation: {}\n      scale: [1, 1, 1]", v3(view.position), v3(facing(look))).unwrap();
    text.push_str("    orr_gfx::Camera:\n      fov_y: 55\n      half_height: 10\n      near: 0.3\n      far: 9000\n      projection: perspective\n      layers: [default, layer2, layer3, layer4, layer5, layer6, layer7, layer8]\n      priority: 0\n      active: true\n");
    // In Play the camera follows the dragon.
    writeln!(text, "    orr_games::DragonCamera:\n      target: {dragon}\n      distance: 34\n      height: 9").unwrap();

    // One water surface per pool of the stream; the shader hides it where the ground is higher.
    for k in 0..18 {
        let z = -HALF + 72.0 * (k as f32 + 0.4);
        let s = world.stream(z);
        entity(&mut text, &format!("pool_{k:02}"));
        writeln!(text, "    orr_gfx::Transform3:\n      position: [{:.2}, {:.3}, {:.2}]\n      rotation: [0, 0, 0]\n      scale: [1, 1, 1]", s.center, s.level, -HALF + 72.0 * (k as f32 + 0.5)).unwrap();
        writeln!(text, "    orr_gfx::Water:\n      size: [260, 72]\n      color: [0.012, 0.05, 0.04]\n      clarity: 1\n      ripples: 1\n      flow: [0, 0.6]\n      glint: 1\n      terrain: {valley}").unwrap();
    }

    let lowest = (0..18).map(|k| world.stream(-HALF + 72.0 * k as f32).level).fold(f32::MAX, f32::min);
    let scatter = |text: &mut String, name: &str, kind: &str, center: [f32; 2], area: f32, count: u32, slope: f32, scale: [f32; 2], seed: u32, shadows: bool, entity: &mut dyn FnMut(&mut String, &str) -> String| {
        entity(text, name);
        writeln!(text, "    orr_gfx::Transform3:\n      position: [{:.1}, 0, {:.1}]\n      rotation: [0, 0, 0]\n      scale: [1, 1, 1]", center[0], center[1]).unwrap();
        writeln!(
            text,
            "    orr_gfx::Scatter:\n      terrain: {valley}\n      area: [{area}, {area}]\n      heights: [{:.1}, 400]\n      max_slope: {slope}\n      scale: [{}, {}]\n      kind: {kind}\n      count: {count}\n      seed: {seed}\n      cast_shadows: {shadows}",
            lowest - 1.0,
            scale[0],
            scale[1]
        )
        .unwrap();
    };
    let near = [view.position[0], view.position[2]];
    scatter(&mut text, "firs", "conifer", [0.0, 0.0], 2.0 * HALF - 20.0, 9000, 36.0, [0.8, 1.3], 11, true, &mut entity);
    scatter(&mut text, "broadleaves", "broadleaf", [0.0, 0.0], 2.0 * HALF - 20.0, 2600, 28.0, [0.75, 1.2], 12, true, &mut entity);
    scatter(&mut text, "boulders", "rock", [0.0, 0.0], 2.0 * HALF - 20.0, 1200, 60.0, [0.4, 2.2], 13, true, &mut entity);
    scatter(&mut text, "meadow", "grass", near, 240.0, 20000, 32.0, [0.7, 1.4], 14, false, &mut entity);
    std::fs::write(out.join("forest_valley.scene.yaml"), text).unwrap();
    println!("wrote {} (valley {low}..{high} m, ridges {far_low}..{far_high} m)", out.display());
}
