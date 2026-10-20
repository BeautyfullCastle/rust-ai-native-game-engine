//! Actual closed CLI starter -> saved map -> PlayHost -> production compositor.
//! Four retained GPU captures, not native-window or exported-player evidence.
#![cfg(all(feature = "project-create", feature = "tilemap", target_os = "linux"))]
#![allow(clippy::float_arithmetic)]
use orr_bridge::{Bridge, PlayerSlot};
use orr_fp::FP;
use orr_render::orr_rhi::{Rhi, TextureFormat, Wgpu, WgpuOptions};
use orr_render::{Camera, OffscreenTarget};
use orr_sample::{
    arena_game::{ArenaInput, PlayerTag, Position},
    project_compositor::ProjectCompositor,
    project_runtime::PreparedRuntime,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, path::Path, process::Command};
#[allow(dead_code)] // Reuse copy_tree without constructing the unrelated fixture.
#[path = "common/project.rs"]
mod project_fixture;
use project_fixture::copy_tree;
const SIZE: (u32, u32) = (192, 128);
const SOURCE: &[u8] = include_bytes!("../../../assets/sprite_demo/lantern_keeper.rgba");
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn png(path: &Path, bytes: &[u8]) {
    let mut encoder = png::Encoder::new(fs::File::create(path).unwrap(), SIZE.0, SIZE.1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(bytes)
        .unwrap();
}
fn linear(v: u8) -> f64 {
    let v = f64::from(v) / 255.;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
fn srgb(v: f64) -> u8 {
    ((if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1. / 2.4) - 0.055
    }) * 255.)
        .round() as u8
}
fn over(out: &mut [f64; 3], region: u32, x: usize, y: usize) {
    // Literal original atlas layout, deliberately not uv_rect/placements/draws.
    let offset = match region {
        10 => 0,
        11 => 16,
        20 => 32,
        21 => 48,
        _ => panic!("unexpected region"),
    };
    let at = (y * 64 + offset + x) * 4;
    let alpha = f64::from(SOURCE[at + 3]) / 255.;
    for c in 0..3 {
        out[c] = linear(SOURCE[at + c]) * alpha + out[c] * (1. - alpha);
    }
}
fn oracle(
    cells: &[Option<u32>],
    actors: &[(u32, [f32; 2])],
    moving: bool,
    actor_sprites: bool,
) -> Vec<Option<[u8; 4]>> {
    (0..SIZE.1)
        .flat_map(|y| (0..SIZE.0).map(move |x| (x, y)))
        .map(|(x, y)| {
            let wx = f64::from(x) + 0.5 - 96.;
            let wy = 64. - f64::from(y) - 0.5;
            let mut out = [0.02, 0.02, 0.035];
            let tx = wx + 84.;
            let ty = 16. - wy;
            if (0.0..48.).contains(&tx) && (0.0..32.).contains(&ty) {
                if let Some(id) = cells[(ty as usize / 16) * 3 + tx as usize / 16] {
                    over(&mut out, id, tx as usize % 16, ty as usize % 16);
                }
            }
            // Avoid rasterizer edge AA, but check every interior/background pixel.
            for (slot, pos) in actors {
                let dx = wx - f64::from(pos[0]);
                let dy = wy - f64::from(pos[1]);
                let radius = dx.hypot(dy);
                if (radius - 20.).abs() < 1.5 {
                    return None;
                }
                if radius < 20. {
                    out = if *slot == 0 {
                        [0.25, 0.6, 1.]
                    } else {
                        [1., 0.55, 0.2]
                    };
                    // Literal authored circle orientation mark, independent of draw extraction.
                    let mark_distance = (dx / 20. - 0.55).hypot(dy / 20.);
                    let t = ((mark_distance - 0.16) / 0.06).clamp(0., 1.);
                    let mark = 1. - t * t * (3. - 2. * t);
                    for channel in &mut out {
                        *channel *= 1. - 0.55 * mark;
                    }
                }
            }
            if actor_sprites {
                for (slot, pos) in actors {
                    let sx = (wx - f64::from(pos[0]) + 16.) / 2.;
                    let sy = (f64::from(pos[1]) + 16. - wy) / 2.;
                    if (0.0..16.).contains(&sx) && (0.0..16.).contains(&sy) {
                        over(
                            &mut out,
                            if moving && *slot == 0 { 20 } else { 10 },
                            sx as usize,
                            sy as usize,
                        );
                    }
                }
            }
            Some([srgb(out[0]), srgb(out[1]), srgb(out[2]), 255])
        })
        .collect()
}
#[test]
#[ignore = "mandatory explicit tilemap CLI/software Vulkan saved-runtime acceptance"]
fn cli_saved_map_four_captures_playhost_frame_parity() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let cli = std::path::PathBuf::from(
        std::env::var_os("ORR_NEW_ARENA_BIN").expect("built tilemap CLI path required"),
    );
    assert!(cli.is_absolute() && cli.is_file());
    let evidence = std::path::PathBuf::from(
        std::env::var_os("ORR_TILEMAP_RUNTIME_EVIDENCE")
            .expect("retained evidence directory required"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("created");
    let output = Command::new(&cli)
        .args([
            "--template",
            "arena-tilemap-2d-v1",
            "--seed",
            "runtime-acceptance",
            "--output",
        ])
        .arg(&root)
        .output()
        .unwrap();
    fs::write(
        evidence.join("creator.log"),
        [output.stdout.as_slice(), output.stderr.as_slice()].concat(),
    )
    .unwrap();
    assert!(output.status.success(), "real creation CLI failed");
    let mut map: orr_tilemap::Document =
        serde_json::from_slice(&fs::read(root.join("arena.tilemap.json")).unwrap()).unwrap();
    // Editing an already CLI-created sidecar, not manufacturing its manifest.
    map.width = 3;
    map.height = 2;
    map.origin = [-84., 16.];
    map.cells = vec![Some(10), None, Some(21), Some(20), Some(11), None];
    let good_cells = map.cells.clone();
    fs::write(
        root.join("arena.tilemap.json"),
        serde_json::to_vec(&map).unwrap(),
    )
    .unwrap();
    let scene = fs::read(root.join("arena.scene.yaml")).unwrap();
    let lock = fs::read(root.join("orr.packages.lock.json")).unwrap();
    let control = temp.path().join("tile-off");
    fs::create_dir(&control).unwrap();
    copy_tree(&root, &control);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(control.join("orr.project.json")).unwrap()).unwrap();
    manifest["entry"].as_object_mut().unwrap().remove("tilemap");
    fs::write(
        control.join("orr.project.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert_eq!(fs::read(control.join("arena.scene.yaml")).unwrap(), scene);
    assert_eq!(
        fs::read(control.join("orr.packages.lock.json")).unwrap(),
        lock
    );
    let gpu = Wgpu::headless(WgpuOptions::default()).expect("mandatory GPU unavailable");
    assert_eq!(format!("{:?}", gpu.adapter().get_info().backend), "Vulkan");
    let target = OffscreenTarget::new(&gpu, SIZE.0, SIZE.1, TextureFormat::Rgba8UnormSrgb);
    let camera = Camera::new([0., 0.], 64.);
    let mut captures = Vec::new();
    let mut saved_pixels = None;
    for (name, moving, tile_only, wrong) in [
        ("saved", false, false, false),
        ("fresh-reopen", false, false, false),
        ("actor-moving", true, false, false),
        ("tile-only-wrong-region", false, true, true),
    ] {
        if tile_only {
            let mut manifest: serde_json::Value =
                serde_json::from_slice(&fs::read(root.join("orr.project.json")).unwrap()).unwrap();
            manifest["entry"].as_object_mut().unwrap().remove("sprites");
            fs::write(
                root.join("orr.project.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            fs::remove_file(root.join("arena.sprites.json")).unwrap();
        }
        if wrong {
            map.cells[0] = Some(11);
            fs::write(
                root.join("arena.tilemap.json"),
                serde_json::to_vec(&map).unwrap(),
            )
            .unwrap();
        }
        let prepared = PreparedRuntime::open(&root).unwrap();
        let off = PreparedRuntime::open(&control).unwrap();
        let mut host = prepared.bridge().unwrap();
        let mut no_tiles = off.bridge().unwrap();
        let (_, mut presentation) = prepared.into_parts().unwrap();
        let (_, mut off_presentation) = off.into_parts().unwrap();
        presentation.update(host.snapshot().as_ref()).unwrap();
        if moving {
            let input = ArenaInput::new(FP::ONE, FP::ZERO, false);
            host.set_input(PlayerSlot(0), input).unwrap();
            no_tiles.set_input(PlayerSlot(0), input).unwrap();
            host.step(1);
            no_tiles.step(1);
        }
        let before = host.host().session().frame().to_bytes();
        let checksum = host.host().session().frame().checksum();
        presentation.update(host.snapshot().as_ref()).unwrap();
        off_presentation
            .update(no_tiles.snapshot().as_ref())
            .unwrap();
        let actors: Vec<_> = orr_bridge::FrameView::of(host.host().session().frame())
            .iter::<Position>()
            .filter_map(|(entity, pos)| {
                host.host()
                    .session()
                    .frame()
                    .get::<PlayerTag>(entity)
                    .map(|tag| (tag.slot, [pos.pos.x.to_f32(), pos.pos.y.to_f32()]))
            })
            .collect();
        let mut compositor =
            ProjectCompositor::new(gpu.clone(), target.format(), presentation.assets()).unwrap();
        compositor
            .draw_with_background(
                target.render_view(),
                target.size(),
                presentation.shapes(),
                presentation.background(),
                presentation.sprites(),
                &camera,
            )
            .unwrap();
        let pixels = target.read_rgba8();
        let file = format!("{name}.png");
        png(&evidence.join(&file), &pixels);
        captures.push(json!({"file":file,"sha256":hash(&pixels),"frame_checksum":checksum,"tile_only":tile_only,"wrong_region":wrong}));
        fs::write(evidence.join("partial-receipt.json"),serde_json::to_vec_pretty(&json!({"completed":false,"pixel_verdict_pending":true,"adapter":gpu.adapter_name(),"creator_sha256":hash(&fs::read(&cli).unwrap()),"source_sha256":hash(SOURCE),"scene_sha256":hash(&scene),"lock_sha256":hash(&lock),"captures":captures})).unwrap()).unwrap();
        assert_eq!(
            off_presentation.shapes().shapes.len(),
            presentation.shapes().shapes.len() + 1,
            "only default floor disappears"
        );
        assert_eq!(presentation.background().len(), 4);
        assert_eq!(presentation.assets().len(), 1, "unified tile/actor atlas");
        assert_eq!(presentation.sprites().is_empty(), tile_only);
        assert_eq!(
            format!("{:?}", gpu.adapter().get_info().device_type),
            "Cpu",
            "software GPU gate"
        );
        if moving {
            assert!(
                actors.iter().any(|(slot, pos)| *slot == 0 && pos[0] > -60.),
                "fixed input must move the authored hero"
            );
            assert_eq!(host.host().session().frame().tick(), 1);
        }
        let expected = oracle(&map.cells, &actors, moving, !tile_only);
        let mut checked = 0;
        for (index, expected) in expected.iter().enumerate() {
            if let Some(expected) = expected {
                checked += 1;
                assert!(
                    pixels[index * 4..index * 4 + 4]
                        .iter()
                        .zip(expected)
                        .all(|(a, b)| a.abs_diff(*b) <= 3),
                    "{name} original-texel oracle mismatch at pixel {index}"
                );
            }
        }
        assert!(
            checked > 20000,
            "oracle must check interiors, not a vacuous probe"
        );
        if wrong {
            let good = oracle(&good_cells, &actors, false, false);
            // These five original atlas texels remain outside the hero and AA band.
            // Region 21 differs only under the actor, so use visibly different region 11.
            for (x, y, literal) in [
                (16, 50, [39, 39, 53, 255]),
                (17, 50, [39, 39, 53, 255]),
                (18, 50, [39, 39, 53, 255]),
                (16, 52, [51, 45, 74, 255]),
                (17, 52, [51, 45, 74, 255]),
            ] {
                let i = y * SIZE.0 as usize + x;
                let actual = &pixels[i * 4..i * 4 + 4];
                assert!(actual.iter().zip(literal).all(|(a, b)| a.abs_diff(b) <= 3));
                let original = good[i].expect("wrong-region probe must not be masked");
                assert!(
                    actual.iter().zip(original).any(|(a, b)| a.abs_diff(b) > 3),
                    "wrong-region control must differ at exposed pixel ({x},{y})"
                );
            }
        }
        if name == "saved" {
            saved_pixels = Some(pixels);
        } else if name == "fresh-reopen" {
            assert_eq!(Some(pixels), saved_pixels);
        }
        assert_eq!(host.host().session().frame().to_bytes(), before);
        assert_eq!(before, no_tiles.host().session().frame().to_bytes());
        assert_eq!(checksum, no_tiles.host().session().frame().checksum());
        assert_eq!(fs::read(root.join("arena.scene.yaml")).unwrap(), scene);
        assert_eq!(fs::read(root.join("orr.packages.lock.json")).unwrap(), lock);
    }
    assert_eq!(captures.len(), 4);
    fs::write(evidence.join("receipt.json"),serde_json::to_vec_pretty(&json!({"completed":true,"captures":captures,"native_os_input":false,"export":false,"windows":false})).unwrap()).unwrap();
}
