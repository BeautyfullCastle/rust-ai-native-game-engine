//! Mandatory offscreen readback acceptance for authored project sprite runs.
//! Set ORR_REQUIRE_GPU=1 on the acceptance lane to make adapter absence fatal.
#![cfg(feature = "project")]
#![allow(clippy::float_arithmetic)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use orr_bridge::{Bridge, InProc, PlayHost, PlayerSlot};
use orr_render::orr_rhi::{Rhi, TextureFormat, Wgpu, WgpuOptions};
use orr_render::{Camera, OffscreenTarget, RenderList};
use orr_sample::arena_view::{Keys, arena_bridge_config};
use orr_sample::project_compositor::{ProjectCompositor, SpriteDraw};
use orr_sample::project_runtime::PreparedRuntime;
use orr_sample::project_sprites::{Asset, AssetKey};
use orr_sprite::{SpriteDocument, SpriteInstance};

#[path = "common/project.rs"]
mod project_fixture;
use project_fixture::ProjectFixture;

static GPU_SERIAL: Mutex<()> = Mutex::new(());

fn gpu() -> Option<(MutexGuard<'static, ()>, Wgpu)> {
    let guard = GPU_SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(gpu) => {
            eprintln!(
                "project sprite compositor adapter: {} (software: {})",
                gpu.adapter_name(),
                gpu.is_software()
            );
            Some((guard, gpu))
        }
        Err(error) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required GPU unavailable: {error}"
            );
            eprintln!("SKIP: {error}");
            None
        }
    }
}

fn asset() -> Asset {
    Asset {
        document: SpriteDocument::from_json(
            r#"{"format":"orr_sprite","version":1,"atlas":{"image":"opaque.rgba","width":1,"height":1},"regions":[{"id":0,"x":0,"y":0,"width":1,"height":1}],"clips":[]}"#,
        )
        .unwrap(),
        // The authored texel is fully opaque white; per-instance tint supplies
        // color while preserving a real atlas upload and sample.
        rgba: vec![255, 255, 255, 255],
    }
}

fn png_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(rgba)
            .unwrap();
    }
    bytes
}

fn sprite_package_source(root: &Path, name: &str, sprite_json: &str, atlas_png: &[u8]) -> PathBuf {
    let source = root.join("test-package-sources").join(name);
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("orr.package.json"),
        format!(
            r#"{{"schema":1,"name":"{name}","version":"1.0.0","engine":"^0.0.1","capabilities":["sprite"],"dependencies":{{}},"files":["sprites.json","atlas.png"]}}"#
        ),
    )
    .unwrap();
    fs::write(source.join("sprites.json"), sprite_json).unwrap();
    fs::write(source.join("atlas.png"), atlas_png).unwrap();
    source
}

fn pixel(bytes: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
    let offset = (y * width + x) * 4;
    bytes[offset..offset + 4].try_into().unwrap()
}

fn near(actual: [u8; 4], expected: [u8; 4]) {
    assert!(
        actual
            .iter()
            .zip(expected)
            .all(|(actual, expected)| actual.abs_diff(expected) <= 3),
        "actual {actual:?}, expected {expected:?}"
    );
}

fn opaque_region_pixels(asset: &Asset, region_id: u32) -> usize {
    let region = asset.document.region(region_id).unwrap();
    let atlas_width = asset.document.atlas().width as usize;
    (region.y as usize..(region.y + region.height) as usize)
        .flat_map(|y| {
            (region.x as usize..(region.x + region.width) as usize)
                .map(move |x| (y * atlas_width + x) * 4 + 3)
        })
        .filter(|alpha| asset.rgba[*alpha] == 255)
        .count()
}

fn matches_region_texel(asset: &Asset, region_id: u32, pixel: [u8; 4]) -> bool {
    if pixel[3] != 255 {
        return false;
    }
    let region = asset.document.region(region_id).unwrap();
    let atlas_width = asset.document.atlas().width as usize;
    for y in region.y as usize..(region.y + region.height) as usize {
        for x in region.x as usize..(region.x + region.width) as usize {
            let offset = (y * atlas_width + x) * 4;
            if asset.rgba[offset + 3] == 255
                && (0..3).all(|channel| pixel[channel].abs_diff(asset.rgba[offset + channel]) <= 3)
            {
                return true;
            }
        }
    }
    false
}

fn captured_actor_texels(
    pixels: &[u8],
    viewport: (u32, u32),
    camera: &Camera,
    instance: SpriteInstance,
    asset: &Asset,
) -> usize {
    let center = camera.world_to_screen(instance.position, viewport);
    let pixels_per_unit = camera.pixels_per_unit(viewport.0, viewport.1);
    let half_width = instance.size[0] * pixels_per_unit * 0.5 + 2.0;
    let half_height = instance.size[1] * pixels_per_unit * 0.5 + 2.0;
    let x0 = (center[0] - half_width).floor().max(0.0) as usize;
    let x1 = (center[0] + half_width).ceil().min(viewport.0 as f32) as usize;
    let y0 = (center[1] - half_height).floor().max(0.0) as usize;
    let y1 = (center[1] + half_height).ceil().min(viewport.1 as f32) as usize;
    let mut matched = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            matched += usize::from(matches_region_texel(
                asset,
                instance.region,
                pixel(pixels, viewport.0 as usize, x, y),
            ));
        }
    }
    matched
}

fn save_capture(directory: &Path, name: &str, size: (u32, u32), rgba: &[u8]) {
    fs::create_dir_all(directory).unwrap();
    let file = fs::File::create(directory.join(name)).unwrap();
    let mut encoder = png::Encoder::new(file, size.0, size.1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(rgba)
        .unwrap();
}

#[test]
fn overlapping_atlas_a_b_a_keeps_global_order_and_reads_opaque_texels() {
    let Some((_guard, gpu)) = gpu() else {
        return;
    };

    let a: AssetKey = ("sprite-package".into(), "atlas-a".into());
    let b: AssetKey = ("sprite-package".into(), "atlas-b".into());
    let assets = BTreeMap::from([(a.clone(), asset()), (b.clone(), asset())]);
    let target = OffscreenTarget::new(&gpu, 64, 64, TextureFormat::Rgba8Unorm);
    let mut compositor = ProjectCompositor::new(gpu.clone(), target.format(), &assets).unwrap();

    let sprite = |asset: AssetKey, tint: [f32; 4], order: i32| SpriteDraw {
        asset,
        instance: SpriteInstance {
            region: 0,
            position: [0.0, 0.0],
            size: [2.0, 2.0],
            tint,
            order,
            ..SpriteInstance::default()
        },
    };
    // Input order models the global GUID traversal. Deliberately conflicting
    // per-instance order values prove atlas-local sort metadata cannot reorder it.
    let ordered = [
        sprite(a.clone(), [1.0, 0.0, 0.0, 0.5], 900),
        sprite(b, [0.0, 1.0, 0.0, 0.5], -900),
        sprite(a, [0.0, 0.0, 1.0, 0.5], -1000),
    ];
    let shapes = RenderList::new();
    compositor
        .draw(
            target.render_view(),
            target.size(),
            &shapes,
            &ordered,
            &Camera::new([0.0, 0.0], 1.0),
        )
        .unwrap();

    let capture = target.read_rgba8();
    // On the default opaque background, red then green then blue at 50%
    // produces approximately [33, 64, 129]. Regrouping A/A/B would instead
    // put green on top and produce a visibly different sample.
    near(pixel(&capture, 64, 32, 32), [33, 64, 129, 255]);
    near(pixel(&capture, 64, 12, 12), [33, 64, 129, 255]);

    let opaque_sprite_texels = capture
        .chunks_exact(4)
        .filter(|texel| texel[3] == 255 && texel[0] > 30 && texel[1] > 60 && texel[2] > 120)
        .count();
    assert!(
        opaque_sprite_texels > 0,
        "GPU readback contained no opaque, layered sprite texels"
    );
    eprintln!(
        "project compositor capture: center={:?}, opaque_layered_sprite_texels={opaque_sprite_texels}",
        pixel(&capture, 64, 32, 32)
    );
}

#[test]
fn saved_project_admission_play_follow_and_gpu_captures_use_installed_atlas() {
    const SIZE: (u32, u32) = (512, 512);

    // Complete project/package validation and Play-session preparation before
    // acquiring a device, matching the runtime's asset-admission boundary.
    let project = ProjectFixture::new();
    let prepared = PreparedRuntime::open(&project.root).unwrap();
    let (session, mut presentation) = prepared.into_parts().unwrap();
    let mut bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), arena_bridge_config());
    let initial = bridge
        .snapshot()
        .expect("PlayHost publishes its initial snapshot");
    presentation.update(Some(&initial)).unwrap();

    let asset_key: AssetKey = ("sample-sprites".into(), "sprites.json".into());
    // Keep an owned test-side copy so image inspection survives the later
    // mutable presentation update without requiring Clone on production Asset.
    let asset = {
        let admitted = presentation
            .assets()
            .get(&asset_key)
            .expect("installed atlas admitted");
        assert_eq!(admitted.document.atlas().width, 64);
        assert_eq!(admitted.document.atlas().height, 16);
        Asset {
            document: admitted.document.clone(),
            rgba: admitted.rgba.clone(),
        }
    };
    assert_eq!(presentation.sprites().len(), 2);
    let hero = presentation.sprites()[0].instance;
    let target_actor = presentation.sprites()[1].instance;
    assert_eq!(hero.position, [-60.0, 0.0]);
    assert_eq!(target_actor.position, [60.0, 0.0]);
    assert_eq!(hero.size, [32.0, 32.0]);
    assert_eq!(target_actor.size, [32.0, 32.0]);
    assert_eq!(hero.region, 10);
    assert_eq!(target_actor.region, 20);
    assert!(!hero.flip_x && !hero.flip_y && !target_actor.flip_x && !target_actor.flip_y);
    assert_eq!(presentation.camera.center, hero.position);
    assert!(opaque_region_pixels(&asset, hero.region) > 0);
    assert!(opaque_region_pixels(&asset, target_actor.region) > 0);

    let Some((_guard, gpu)) = gpu() else {
        return;
    };

    let format = TextureFormat::Rgba8UnormSrgb;
    let target = OffscreenTarget::new(&gpu, SIZE.0, SIZE.1, format);
    let mut compositor =
        ProjectCompositor::new(gpu.clone(), format, presentation.assets()).unwrap();
    compositor
        .draw(
            target.render_view(),
            SIZE,
            presentation.shapes(),
            presentation.sprites(),
            &presentation.camera,
        )
        .unwrap();
    let initial_capture = target.read_rgba8();
    let initial_hero_pixels =
        captured_actor_texels(&initial_capture, SIZE, &presentation.camera, hero, &asset);
    let initial_target_pixels = captured_actor_texels(
        &initial_capture,
        SIZE,
        &presentation.camera,
        target_actor,
        &asset,
    );
    assert!(
        initial_hero_pixels > 20,
        "hero opaque atlas capture had {initial_hero_pixels} matching pixels"
    );
    assert!(
        initial_target_pixels > 20,
        "target opaque atlas capture had {initial_target_pixels} matching pixels"
    );

    if let Some(directory) = std::env::var_os("ORR_PROJECT_CAPTURE_DIR") {
        save_capture(
            Path::new(&directory),
            "saved-project-initial.png",
            SIZE,
            &initial_capture,
        );
    }

    bridge
        .set_input(
            PlayerSlot(0),
            Keys {
                right: true,
                ..Default::default()
            }
            .to_input(),
        )
        .unwrap();
    bridge.step(2);
    let moved = bridge.snapshot().expect("moved PlayHost snapshot");
    presentation.update(Some(&moved)).unwrap();
    let moved_hero = presentation.sprites()[0].instance;
    let moved_target = presentation.sprites()[1].instance;
    assert_eq!(moved_hero.position, [-48.0, 0.0]);
    assert_eq!(
        moved_hero.size, hero.size,
        "authored scale stays fixed as the actor moves"
    );
    assert_eq!(
        moved_hero.region, 20,
        "moving hero selects its authored walk clip"
    );
    assert_eq!(
        presentation.camera.center, moved_hero.position,
        "saved camera follow tracks the moving GUID"
    );
    assert!(presentation.camera.center[0] > hero.position[0]);
    let initial_target_screen =
        Camera::new([-60.0, 0.0], 240.0).world_to_screen(target_actor.position, SIZE);
    let moved_target_screen = presentation
        .camera
        .world_to_screen(moved_target.position, SIZE);
    assert!(
        initial_target_screen[0] - moved_target_screen[0] > 10.0,
        "camera follow should shift the stationary actor in the capture: {initial_target_screen:?} -> {moved_target_screen:?}"
    );

    compositor
        .draw(
            target.render_view(),
            SIZE,
            presentation.shapes(),
            presentation.sprites(),
            &presentation.camera,
        )
        .unwrap();
    let moved_capture = target.read_rgba8();
    let moved_hero_pixels = captured_actor_texels(
        &moved_capture,
        SIZE,
        &presentation.camera,
        moved_hero,
        &asset,
    );
    let moved_target_pixels = captured_actor_texels(
        &moved_capture,
        SIZE,
        &presentation.camera,
        moved_target,
        &asset,
    );
    assert!(
        moved_hero_pixels > 20,
        "moved hero capture had {moved_hero_pixels} matching pixels"
    );
    assert!(
        moved_target_pixels > 20,
        "follow-shifted target capture had {moved_target_pixels} matching pixels"
    );

    if let Some(directory) = std::env::var_os("ORR_PROJECT_CAPTURE_DIR") {
        save_capture(
            Path::new(&directory),
            "saved-project-followed.png",
            SIZE,
            &moved_capture,
        );
    }
    eprintln!(
        "saved project GPU captures: initial actors=({initial_hero_pixels},{initial_target_pixels}), moved actors=({moved_hero_pixels},{moved_target_pixels}), hero {:?} -> {:?}, camera {:?}",
        hero.position, moved_hero.position, presentation.camera.center
    );
}

#[test]
fn runtime_presenter_preserves_a_b_a_alpha_order_across_installed_atlases() {
    let fixture = ProjectFixture::new();
    fs::write(
        fixture.root.join("arena.scene.yaml"),
        "schema: orr.scene/1\nsingletons:\n  Score: { kills: [0,0,0,0,0,0,0,0] }\nentities:\n  e_00000001:\n    Position: { pos: [0,0] }\n    PlayerTag: { slot: 0 }\n  e_00000002:\n    Position: { pos: [0,0] }\n    PlayerTag: { slot: 1 }\n  e_00000003:\n    Position: { pos: [0,0] }\n    PlayerTag: { slot: 2 }\n",
    )
    .unwrap();
    fs::write(
        fixture.root.join("arena.sprites.json"),
        r#"{
          "version":2,"scene":"arena.scene.yaml","project":".",
          "bindings":{
            "e_00000001":{"package":"test-atlas-a","document":"sprites.json","source":{"Region":0},"units_per_pixel":16.0},
            "e_00000002":{"package":"test-atlas-b","document":"sprites.json","source":{"Region":0},"units_per_pixel":16.0},
            "e_00000003":{"package":"test-atlas-a","document":"sprites.json","source":{"Region":1},"units_per_pixel":16.0}
          }
        }"#,
    )
    .unwrap();

    let atlas_a_json = r#"{"format":"orr_sprite","version":1,"atlas":{"image":"atlas.png","width":2,"height":1},"regions":[{"id":0,"x":0,"y":0,"width":1,"height":1},{"id":1,"x":1,"y":0,"width":1,"height":1}],"clips":[]}"#;
    let atlas_b_json = r#"{"format":"orr_sprite","version":1,"atlas":{"image":"atlas.png","width":1,"height":1},"regions":[{"id":0,"x":0,"y":0,"width":1,"height":1}],"clips":[]}"#;
    let package_a = sprite_package_source(
        &fixture.root,
        "test-atlas-a",
        atlas_a_json,
        &png_rgba(2, 1, &[255, 0, 0, 128, 0, 0, 255, 128]),
    );
    let package_b = sprite_package_source(
        &fixture.root,
        "test-atlas-b",
        atlas_b_json,
        &png_rgba(1, 1, &[0, 255, 0, 128]),
    );
    let installer = orr_package::Project::open(
        &fixture.root,
        orr_sample::project_runtime::compiled_runtime(),
    )
    .unwrap();
    installer.install(&[package_a, package_b]).unwrap();
    fs::remove_dir_all(fixture.root.join("test-package-sources")).unwrap();

    // The complete local packages are installed and the authored scene is
    // admitted before this test requests its headless device.
    let prepared = PreparedRuntime::open(&fixture.root).unwrap();
    let (session, mut presentation) = prepared.into_parts().unwrap();
    let bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), arena_bridge_config());
    let snapshot = bridge
        .snapshot()
        .expect("PlayHost publishes its initial snapshot");
    presentation.update(Some(&snapshot)).unwrap();
    presentation.camera.half_extent = 16.0;
    let expected_assets: Vec<AssetKey> = [
        ("test-atlas-a".into(), "sprites.json".into()),
        ("test-atlas-b".into(), "sprites.json".into()),
        ("test-atlas-a".into(), "sprites.json".into()),
    ]
    .into();
    assert_eq!(
        presentation
            .sprites()
            .iter()
            .map(|sprite| sprite.asset.clone())
            .collect::<Vec<_>>(),
        expected_assets,
        "project presenter emits the GUID-ordered A/B/A sequence"
    );
    assert_eq!(presentation.sprites().len(), 3);
    assert!(
        presentation
            .sprites()
            .iter()
            .all(|sprite| sprite.instance.position == [0.0, 0.0])
    );
    assert!(
        presentation
            .sprites()
            .iter()
            .all(|sprite| sprite.instance.size == [16.0, 16.0])
    );

    let Some((_guard, gpu)) = gpu() else {
        return;
    };
    let format = TextureFormat::Rgba8Unorm;
    let target = OffscreenTarget::new(&gpu, 64, 64, format);
    let mut compositor =
        ProjectCompositor::new(gpu.clone(), format, presentation.assets()).unwrap();
    let camera = &presentation.camera;
    compositor
        .draw(
            target.render_view(),
            target.size(),
            presentation.shapes(),
            &[],
            camera,
        )
        .unwrap();
    let base = pixel(&target.read_rgba8(), 64, 32, 32);

    compositor
        .draw(
            target.render_view(),
            target.size(),
            presentation.shapes(),
            presentation.sprites(),
            camera,
        )
        .unwrap();
    let capture = target.read_rgba8();
    let actual = pixel(&capture, 64, 32, 32);
    let over = |destination: [u8; 4], color: [u8; 4]| {
        let alpha = f32::from(color[3]) / 255.0;
        let mut output = destination;
        for channel in 0..3 {
            let blended =
                f32::from(color[channel]) * alpha + f32::from(destination[channel]) * (1.0 - alpha);
            output[channel] = blended.round() as u8;
        }
        output[3] = (f32::from(color[3]) + f32::from(destination[3]) * (1.0 - alpha)).round() as u8;
        output
    };
    let expected = [[255, 0, 0, 128], [0, 255, 0, 128], [0, 0, 255, 128]]
        .into_iter()
        .fold(base, over);
    near(actual, expected);

    let regrouped = [[255, 0, 0, 128], [0, 0, 255, 128], [0, 255, 0, 128]]
        .into_iter()
        .fold(base, over);
    assert!(
        actual
            .iter()
            .zip(regrouped)
            .any(|(actual, regrouped)| actual.abs_diff(regrouped) > 10),
        "A/A/B regrouping should have a distinctly different center pixel: {actual:?} vs {regrouped:?}"
    );
    if let Some(directory) = std::env::var_os("ORR_PROJECT_CAPTURE_DIR") {
        save_capture(
            Path::new(&directory),
            "saved-project-runtime-ab-a.png",
            target.size(),
            &capture,
        );
    }
    eprintln!(
        "runtime A/B/A alpha capture: base={base:?}, result={actual:?}, regrouped={regrouped:?}"
    );
}
