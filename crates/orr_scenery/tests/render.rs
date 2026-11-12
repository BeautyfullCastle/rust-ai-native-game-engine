//! Real frames on the hardware adapter, read back and checked.
//!
//! A machine without a hardware adapter prints SKIP and draws nothing. Set
//! `ORR_REQUIRE_GPU=1` to make that a failure, as the other GPU tests of the
//! workspace do.
#![allow(clippy::float_arithmetic)]

use orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions};
use orr_scenery::{viewpoints, Camera, Environment, Quality, Renderer, Settings, World};

const SIZE: (u32, u32) = (320, 180);

struct Frame {
    pixels: Vec<u8>,
}

impl Frame {
    /// Mean color of a rectangle given in fractions of the frame.
    fn mean(&self, x: (f32, f32), y: (f32, f32)) -> [f32; 3] {
        let (x0, x1) = ((x.0 * SIZE.0 as f32) as usize, (x.1 * SIZE.0 as f32) as usize);
        let (y0, y1) = ((y.0 * SIZE.1 as f32) as usize, (y.1 * SIZE.1 as f32) as usize);
        let mut sum = [0.0; 3];
        for row in y0..y1 {
            for column in x0..x1 {
                let texel = &self.pixels[(row * SIZE.0 as usize + column) * 4..][..4];
                for channel in 0..3 {
                    sum[channel] += f32::from(texel[channel]);
                }
            }
        }
        sum.map(|v| v / ((x1 - x0) * (y1 - y0)) as f32)
    }

    fn brightness(&self) -> f32 {
        let all = self.mean((0.0, 1.0), (0.0, 1.0));
        (all[0] + all[1] + all[2]) / 3.0
    }

    fn distinct_colors(&self) -> usize {
        let mut colors: Vec<[u8; 3]> = self.pixels.chunks(4).map(|t| [t[0] >> 3, t[1] >> 3, t[2] >> 3]).collect();
        colors.sort_unstable();
        colors.dedup();
        colors.len()
    }

    /// Share of channel values that differ from `other` by more than two steps.
    fn difference(&self, other: &Frame) -> f32 {
        let different = self.pixels.iter().zip(&other.pixels).filter(|(a, b)| a.abs_diff(**b) > 2).count();
        different as f32 / self.pixels.len() as f32
    }
}

struct Scene {
    rhi: Wgpu,
    world: World,
    views: Vec<Camera>,
}

fn scene() -> Option<Scene> {
    let rhi = match Wgpu::headless(WgpuOptions { allow_software_fallback: false, ..Default::default() }) {
        Ok(rhi) => rhi,
        Err(error) => {
            assert!(std::env::var_os("ORR_REQUIRE_GPU").is_none_or(|v| v != "1"), "a hardware adapter is required: {error}");
            eprintln!("SKIP: no hardware adapter ({error})");
            return None;
        }
    };
    let world = World::generate(7);
    let views = viewpoints(&world);
    Some(Scene { rhi, world, views })
}

impl Scene {
    /// The first frame of a new renderer, so no earlier frame affects the exposure.
    fn draw(&self, view: usize, settings: &Settings, quality: Quality) -> (Frame, orr_scenery::renderer::FrameStats) {
        let format = TextureFormat::Rgba8UnormSrgb;
        let texture = self.rhi.create_texture(&TextureDesc {
            label: "scenery test target",
            width: SIZE.0,
            height: SIZE.1,
            format,
            usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
            sample_count: 1,
            view_formats: &[],
        });
        let target = self.rhi.create_texture_view(&texture, None);
        let mut renderer = Renderer::new(self.rhi.clone(), format, &self.world, quality);
        let camera = &self.views[view];
        renderer.render(&target, SIZE, camera, &Environment::at(&self.world, camera, 20.0, 0.0), settings);
        (Frame { pixels: self.rhi.read_texture(&texture) }, renderer.last_frame_stats())
    }
}

const AERIAL: usize = 4;
const POOL: usize = 1;

#[test]
fn a_frame_shows_sky_forest_and_water() {
    let Some(scene) = scene() else { return };
    let (aerial, stats) = scene.draw(AERIAL, &Settings::default(), Quality::default());
    assert_eq!(aerial.pixels.len(), (SIZE.0 * SIZE.1 * 4) as usize);
    assert!(aerial.pixels.chunks(4).all(|t| t[3] == 255));
    assert!(aerial.distinct_colors() > 300, "{} colors", aerial.distinct_colors());
    let sky = aerial.mean((0.55, 0.8), (0.0, 0.08));
    assert!(sky[2] > sky[0] + 15.0 && sky[2] > 90.0, "sky {sky:?}");
    let forest = aerial.mean((0.55, 0.95), (0.6, 0.95));
    assert!(forest[1] > forest[0] && forest[1] > forest[2] && forest[1] > 20.0, "forest {forest:?}");
    assert!(stats.triangles > 1_000_000 && stats.instances > 5_000 && stats.draws > 10, "{stats:?}");

    // In a pool the lower part of the frame is water: more green than red.
    let (pool, _) = scene.draw(POOL, &Settings::default(), Quality::default());
    let water = pool.mean((0.1, 0.5), (0.7, 0.95));
    assert!(water[1] > water[0] + 8.0, "water {water:?}");

    // The same inputs give the same frame.
    let (again, _) = scene.draw(AERIAL, &Settings::default(), Quality::default());
    assert!(aerial.difference(&again) < 0.005, "{} of the frame differs", aerial.difference(&again));
}

#[test]
fn settings_change_the_frame_they_describe() {
    let Some(scene) = scene() else { return };
    let base = Settings::default();
    let (reference, reference_stats) = scene.draw(AERIAL, &base, Quality::default());
    let sky = |frame: &Frame| frame.mean((0.55, 0.8), (0.0, 0.08));

    // After sunset the sky is darker and less blue than in the afternoon.
    let mut dusk = base;
    dusk.sun.hour = 18.6;
    let (dusk, _) = scene.draw(AERIAL, &dusk, Quality::default());
    assert!(sky(&dusk)[2] < sky(&reference)[2] - 10.0, "afternoon {:?}, dusk {:?}", sky(&reference), sky(&dusk));

    // Manual exposure: a higher value gives a brighter frame.
    let mut manual = base;
    manual.look.auto_exposure = false;
    manual.look.exposure = 0.4;
    let (dark, _) = scene.draw(AERIAL, &manual, Quality::default());
    manual.look.exposure = 2.5;
    let (bright, _) = scene.draw(AERIAL, &manual, Quality::default());
    assert!(bright.brightness() > dark.brightness() + 40.0, "{} and {}", dark.brightness(), bright.brightness());

    // No saturation leaves a gray frame.
    let mut gray = base;
    gray.look.saturation = 0.0;
    gray.look.grain = 0.0;
    let (gray, _) = scene.draw(AERIAL, &gray, Quality::default());
    assert!(gray.pixels.chunks(4).all(|t| t[0].abs_diff(t[1]) <= 12 && t[1].abs_diff(t[2]) <= 12), "a pixel kept its color");

    // Thick fog hides the far ridges: the frame loses contrast.
    let mut fog = base;
    fog.fog.density = 5.0;
    fog.fog.haze = 1.0;
    let (fog, _) = scene.draw(AERIAL, &fog, Quality::default());
    assert!(fog.distinct_colors() * 2 < reference.distinct_colors(), "{} and {} colors", fog.distinct_colors(), reference.distinct_colors());

    // Without shadows nothing is drawn into the shadow maps.
    let mut flat = base;
    flat.shadows.enabled = false;
    let (flat, flat_stats) = scene.draw(AERIAL, &flat, Quality::default());
    assert!(flat_stats.triangles * 3 < reference_stats.triangles * 2, "{flat_stats:?} and {reference_stats:?}");
    assert!(flat.difference(&reference) > 0.05, "shadows off changed {} of the frame", flat.difference(&reference));

    // A leaf tint toward red makes the forest less green.
    let forest = |frame: &Frame| frame.mean((0.55, 0.95), (0.6, 0.95));
    let mut red = base;
    red.vegetation.tint = [2.0, 0.6, 0.6];
    let (red, _) = scene.draw(AERIAL, &red, Quality::default());
    assert!(forest(&red)[0] > forest(&red)[1], "forest {:?}", forest(&red));

    // Lower quality still draws the scene.
    let low = Quality { antialiasing: 1, shadow_resolution: 1024 };
    let (low, _) = scene.draw(AERIAL, &base, low);
    assert!(low.difference(&reference) < 0.6 && low.distinct_colors() > 300);
}
