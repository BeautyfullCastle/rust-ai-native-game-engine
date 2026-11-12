//! Real EditorApp widgets and composed egui framebuffer, not native OS input.
//! No mock/no-device fallback. The receipt names the adapter actually selected.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use super::*;
use crate::sprite_bindings::{Document, Orientation, Source};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use std::{collections::BTreeMap, fs, path::Path};

const ATLAS: &[u8] = include_bytes!("../../../../assets/sprite_demo/lantern_keeper.rgba");
const ACTORS: &str = "arena.sprites.json";
const MAP: &str = "arena.tilemap.json";

fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}

fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(4);
    assert!(
        !h.get_by_label(label).accesskit_node().is_disabled(),
        "disabled widget: {label}"
    );
    h.get_by_label(label).click_accesskit();
    settle(h);
}

fn open(root: &Path) -> Harness<'static, EditorApp> {
    let prepared = crate::project::PreparedProject::open(root).unwrap();
    let mut h = Harness::builder()
        .with_size([1800., 1800.])
        .with_step_dt(1. / 60.)
        // Kittest's PREDICTABLE filter forces bilinear sampling even for
        // NEAREST textures. Honor the production actor atlas sampler instead.
        .with_render_options(egui_wgpu::RendererOptions {
            predictable_texture_filtering: false,
            ..egui_wgpu::RendererOptions::PREDICTABLE
        })
        .wgpu()
        .build_eframe(move |cc| {
            let info = cc
                .wgpu_render_state
                .as_ref()
                .expect("actual GPU adapter required; no device is a failure")
                .adapter
                .get_info();
            eprintln!(
                "tilemap actor source adapter={} backend={:?} device={:?}",
                info.name, info.backend, info.device_type
            );
            let mut editor = Editor::start(&prepared.host_spec()).unwrap();
            editor.sync();
            editor.camera = orr_render::Camera::new([0., 0.], 160.);
            prepared
                .into_app(editor, cc.wgpu_render_state.clone(), &cc.egui_ctx)
                .unwrap()
        });
    settle(&mut h);
    assert!(h.state().viewport_gpu().is_some());
    h
}

fn document<'a>(h: &'a Harness<'_, EditorApp>) -> &'a Document {
    h.state().sprites.bindings.as_ref().unwrap().document()
}

fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn collect(root: &Path, at: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect(root, &path, result);
            } else {
                result.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    collect(root, root, &mut result);
    result
}

fn simulation(h: &Harness<'_, EditorApp>) -> (u64, u64) {
    (
        h.state().editor.checksum(),
        h.state().editor.snapshot().unwrap().predicted().checksum(),
    )
}

fn screen(h: &Harness<'_, EditorApp>, world: [f32; 2]) -> egui::Pos2 {
    let app = h.state();
    let rect = app.ui.viewport_rect.unwrap();
    let px = app.ui.viewport_px;
    let at = app.editor.camera.world_to_screen(world, px);
    let point = rect.min
        + egui::vec2(
            at[0] * rect.width() / px.0 as f32,
            at[1] * rect.height() / px.1 as f32,
        );
    assert!(rect.contains(point), "fixture probe must be visible");
    point
}

fn pointer(h: &mut Harness<'_, EditorApp>, world: [f32; 2]) {
    let point = screen(h, world);
    for pressed in [true, false] {
        h.event(egui::Event::PointerMoved(point));
        h.event(egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        settle(h);
    }
}

struct Capture {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    native: Vec<u8>,
    points_to_pixels: f32,
}

fn capture(h: &mut Harness<'_, EditorApp>, dir: &Path, name: &str) -> Capture {
    h.event(egui::Event::PointerMoved(egui::pos2(2., 2.)));
    settle(h);
    let image = h.render().expect("real composed egui framebuffer readback");
    let width = image.width();
    let height = image.height();
    let rgba = image.into_raw();
    assert!(rgba.as_chunks::<4>().0.iter().any(|p| p[0] > 40));
    let image = egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
    write_png(&dir.join(format!("{name}.png")), &image).unwrap();
    let info = h.state().render_state.as_ref().unwrap().adapter.get_info();
    let fingerprint = rgba.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    });
    fs::write(
        dir.join(format!("{name}.json")),
        serde_json::to_vec_pretty(&serde_json::json!({
            "capture": format!("{name}.png"),
            "size": [width, height],
            "rgba_fnv1a64": format!("{fingerprint:016x}"),
            "adapter": info.name,
            "backend": format!("{:?}", info.backend),
            "device_type": format!("{:?}", info.device_type),
            "native_os_input": false,
            "scope": "full egui composition: native tile background and egui actor meshes"
        }))
        .unwrap(),
    )
    .unwrap();
    Capture {
        width,
        height,
        rgba,
        native: h.state().viewport_gpu().unwrap().gpu().read_rgba8(),
        points_to_pixels: h.ctx.pixels_per_point(),
    }
}

fn pixel(capture: &Capture, h: &Harness<'_, EditorApp>, world: [f32; 2]) -> [u8; 4] {
    let point = screen(h, world) * capture.points_to_pixels;
    let (x, y) = (point.x.floor() as u32, point.y.floor() as u32);
    assert!(x < capture.width && y < capture.height);
    let offset = ((y * capture.width + x) * 4) as usize;
    capture.rgba[offset..offset + 4].try_into().unwrap()
}

fn atlas(region: u32, column: usize, row: usize) -> [u8; 4] {
    // Literal original 64x16 atlas layout, independent of production UV,
    // source sampling, tile placement, or sprite draw extraction helpers.
    let left = match region {
        10 => 0,
        11 => 16,
        20 => 32,
        21 => 48,
        _ => panic!("unexpected fixture region"),
    };
    let offset = (row * 64 + left + column) * 4;
    ATLAS[offset..offset + 4].try_into().unwrap()
}

fn patch(
    capture: &Capture,
    h: &Harness<'_, EditorApp>,
    center: [f32; 2],
    scale: f32,
    oriented: bool,
) -> Vec<[u8; 4]> {
    (0..16)
        .flat_map(|row| {
            (0..16).map(move |column| {
                let x = (column as f32 - 7.5) * scale;
                let y = (7.5 - row as f32) * scale;
                // Authored fixture: flip X, then one counter-clockwise turn.
                let [x, y] = if oriented { [-y, -x] } else { [x, y] };
                pixel(capture, h, [center[0] + x, center[1] + y])
            })
        })
        .collect()
}

fn assert_atlas(pixels: &[[u8; 4]], expected: u32, wrong: u32) -> usize {
    let mut checked = 0;
    let mut wrong_rejected = 0;
    for (index, actual) in pixels.iter().enumerate() {
        let expected_pixel = atlas(expected, index % 16, index / 16);
        if expected_pixel[3] != 255 {
            continue; // No dependence on collider underlay or rasterizer edge AA.
        }
        assert!(
            actual.iter().zip(expected_pixel).all(|(a, b)| a.abs_diff(b) <= 3),
            "literal atlas region {expected} at ({}, {}): expected {expected_pixel:?}, got {actual:?}",
            index % 16,
            index / 16
        );
        checked += 1;
        if atlas(wrong, index % 16, index / 16) != expected_pixel {
            wrong_rejected += 1;
        }
    }
    assert!(checked >= 90, "enough independent opaque atlas probes");
    assert!(wrong_rejected > 0, "oracle distinguishes the old source");
    checked
}

fn paused_walk_sample(h: &Harness<'_, EditorApp>) -> (u64, u32) {
    let snapshot = h.state().editor.snapshot().unwrap();
    assert_eq!(snapshot.tick_rate(), 60, "closed Arena fixture tick rate");
    let timeline = snapshot.timeline().unwrap();
    assert!(
        !timeline.playing,
        "actual Pause must stop automatic advancement"
    );
    let milliseconds = timeline.tick * 1000 / 60;
    // Literal checked-in walk clip: regions 20/21 each last 140 ms.
    let region = if milliseconds % 280 < 140 { 20 } else { 21 };
    (timeline.tick, region)
}

fn step_to_walk_sample(h: &mut Harness<'_, EditorApp>, expected_region: u32) -> u32 {
    for steps in 0..=9 {
        let (before, region) = paused_walk_sample(h);
        if region == expected_region {
            return steps;
        }
        assert!(
            steps < 9,
            "one 140 ms walk frame needs at most nine 60 Hz steps"
        );
        click(h, LBL_STEP);
        assert_eq!(
            paused_walk_sample(h).0,
            before + 1,
            "actual Step advances once"
        );
    }
    unreachable!()
}

#[test]
#[ignore = "explicit actual EditorApp actor Source software-GPU acceptance"]
fn actual_actor_source_undo_redo_save_reopen_play_and_tile_save() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let evidence = PathBuf::from(
        std::env::var_os("ORR_TILEMAP_ACTOR_EVIDENCE")
            .expect("retained evidence directory required"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    orr_sample::project_create::create_tilemap(&orr_sample::project_create::CreateOptions {
        output: root.clone(),
        template: "arena-tilemap-2d-v1".into(),
        seed: "actor-source-ui".into(),
    })
    .unwrap();
    let mut original = Document::from_bytes(&fs::read(root.join(ACTORS)).unwrap()).unwrap();
    let hero = original.camera_follow.clone().unwrap();
    original.version = 3;
    original.bindings.get_mut(&hero).unwrap().orientation = Some(Orientation {
        quarter_turns: 1,
        flip_x: true,
        flip_y: false,
    });
    fs::write(root.join(ACTORS), serde_json::to_vec(&original).unwrap()).unwrap();
    let mut map: orr_tilemap::Document =
        serde_json::from_slice(&fs::read(root.join(MAP)).unwrap()).unwrap();
    map.cells[17] = Some(11); // column 1, row 1; center [-104, 72].
    fs::write(root.join(MAP), serde_json::to_vec(&map).unwrap()).unwrap();
    let mut expected_files = files(&root);
    let mut h = open(&root);
    click(&mut h, "hero");
    let baseline = simulation(&h);
    let history = h.state().editor.history().clone();
    let key = ("sample-sprites".into(), "sprites.json".into());
    let (asset, texture) = h.state().sprites.tilemap_asset(&key).unwrap();
    let atlas_bytes = asset.rgba.clone();
    let atlas_document = asset.document.to_json().unwrap();
    let initial = capture(&mut h, &evidence, "01-initial");
    let initial_actor = patch(&initial, &h, [-60., 0.], 2., true);
    let initial_target = patch(&initial, &h, [60., 0.], 2., false);
    let initial_tile = patch(&initial, &h, [-104., 72.], 1., false);
    assert_atlas(&initial_actor, 10, 11);
    assert_atlas(&initial_target, 10, 11);
    assert_atlas(&initial_tile, 11, 10);

    click(&mut h, "Actor region 11");
    click(&mut h, "Apply actor source");
    let mut region = original.clone();
    region.bindings.get_mut(&hero).unwrap().source = Source::Region(11);
    assert_eq!(
        document(&h),
        &region,
        "only selected existing Source changes"
    );
    assert_eq!(files(&root), expected_files, "no implicit sidecar save");
    let changed = capture(&mut h, &evidence, "02-region");
    let changed_actor = patch(&changed, &h, [-60., 0.], 2., true);
    assert_atlas(&changed_actor, 11, 10);
    assert_ne!(changed_actor, initial_actor);
    assert_eq!(patch(&changed, &h, [60., 0.], 2., false), initial_target);
    assert_eq!(
        changed.native, initial.native,
        "tile and collider pixels unchanged"
    );
    assert_eq!(patch(&changed, &h, [-104., 72.], 1., false), initial_tile);
    assert_eq!(simulation(&h), baseline);
    click(&mut h, "Undo actor source");
    assert_eq!(document(&h), &original);
    let undone = capture(&mut h, &evidence, "03-undo");
    assert_eq!(patch(&undone, &h, [-60., 0.], 2., true), initial_actor);
    click(&mut h, "Redo actor source");
    assert_eq!(document(&h), &region);
    let redone = capture(&mut h, &evidence, "04-redo");
    assert_eq!(patch(&redone, &h, [-60., 0.], 2., true), changed_actor);

    click(&mut h, "Actor clip walk");
    click(&mut h, "Apply actor source");
    let mut saved = original.clone();
    saved.bindings.get_mut(&hero).unwrap().source = Source::Clip("walk".into());
    assert_eq!(document(&h), &saved);
    click(&mut h, "Undo actor source");
    assert_eq!(document(&h), &region);
    click(&mut h, "Redo actor source");
    assert_eq!(document(&h), &saved);
    let clip = capture(&mut h, &evidence, "05-clip-dirty");
    let clip_actor = patch(&clip, &h, [-60., 0.], 2., true);
    assert_atlas(&clip_actor, 20, 11);
    assert_ne!(clip_actor, changed_actor);
    assert_eq!(patch(&clip, &h, [60., 0.], 2., false), initial_target);
    assert_eq!(clip.native, initial.native);
    assert_eq!(files(&root), expected_files);
    assert_eq!(simulation(&h), baseline);
    assert_eq!(h.state().editor.history(), &history);
    for label in [LBL_PLAY, LBL_STEP] {
        assert!(
            h.get_by_label(label).accesskit_node().is_disabled(),
            "dirty actor source must disable {label}"
        );
    }
    click(&mut h, "Save actor appearances");
    assert!(!h.state().sprites.bindings.as_ref().unwrap().dirty());
    let saved_bytes = fs::read(root.join(ACTORS)).unwrap();
    assert_eq!(Document::from_bytes(&saved_bytes).unwrap(), saved);
    assert_ne!(saved_bytes, expected_files[Path::new(ACTORS)]);
    expected_files.insert(ACTORS.into(), saved_bytes.clone());
    assert_eq!(
        files(&root),
        expected_files,
        "actor save touches only actor sidecar"
    );
    assert_eq!(simulation(&h), baseline);
    let (asset, current_texture) = h.state().sprites.tilemap_asset(&key).unwrap();
    assert_eq!(
        current_texture, texture,
        "Source history/save must not reupload atlas"
    );
    assert_eq!(asset.rgba, atlas_bytes);
    assert_eq!(asset.document.to_json().unwrap(), atlas_document);
    assert_eq!(h.state().sprites.tilemap_assets().len(), 1);

    // In this same admitted App, successful actor Save must advance the actor
    // byte stamp used by tile Save. Reopening first would hide a stale stamp.
    click(&mut h, "Tile tool (owns viewport clicks)");
    click(&mut h, "Static region 20");
    pointer(&mut h, [-88., 72.]); // column 2, row 1 was empty.
    assert_eq!(
        h.state()
            .tilemap
            .as_ref()
            .unwrap()
            .document()
            .cell(orr_tilemap::CellCoord { column: 2, row: 1 })
            .unwrap(),
        Some(20)
    );
    assert_eq!(files(&root), expected_files, "tile paint remains in memory");
    click(&mut h, "Save tilemap");
    assert!(!h.state().tilemap.as_ref().unwrap().dirty());
    let saved_map = fs::read(root.join(MAP)).unwrap();
    assert_ne!(saved_map, expected_files[Path::new(MAP)]);
    expected_files.insert(MAP.into(), saved_map);
    assert_eq!(files(&root), expected_files);
    assert_eq!(fs::read(root.join(ACTORS)).unwrap(), saved_bytes);
    assert_eq!(simulation(&h), baseline);
    click(&mut h, "Tile tool (owns viewport clicks)");
    let final_frame = capture(&mut h, &evidence, "06-tile-saved");
    assert_atlas(&patch(&final_frame, &h, [-88., 72.], 1., false), 20, 11);
    drop(h);

    let mut reopened = open(&root);
    click(&mut reopened, "hero");
    assert_eq!(
        document(&reopened),
        &saved,
        "fresh disk admission, no injected history"
    );
    assert_eq!(simulation(&reopened), baseline);
    let fresh = capture(&mut reopened, &evidence, "07-reopened");
    assert_eq!(patch(&fresh, &reopened, [-60., 0.], 2., true), clip_actor);
    assert_eq!(
        patch(&fresh, &reopened, [-104., 72.], 1., false),
        initial_tile
    );
    click(&mut reopened, LBL_PLAY);
    assert_eq!(reopened.state().editor.mode(), Mode::Play);
    click(&mut reopened, LBL_PAUSE);
    // Normalize any real-time progress to the first walk frame through actual
    // Step buttons, then cross its boundary without wall-clock sleeps or seeks.
    let first_steps = step_to_walk_sample(&mut reopened, 20);
    let first_tick = paused_walk_sample(&reopened).0;
    let play = capture(&mut reopened, &evidence, "08-play-paused");
    assert_eq!(paused_walk_sample(&reopened), (first_tick, 20));
    let first_actor = patch(&play, &reopened, [-60., 0.], 2., true);
    assert_atlas(&first_actor, 20, 21);
    let second_steps = step_to_walk_sample(&mut reopened, 21);
    assert!(second_steps > 0 && first_steps + second_steps <= 18);
    let second_tick = paused_walk_sample(&reopened).0;
    assert!(second_tick > first_tick && second_tick > 0);
    let advanced = capture(&mut reopened, &evidence, "09-play-second-frame");
    assert_eq!(paused_walk_sample(&reopened), (second_tick, 21));
    let second_actor = patch(&advanced, &reopened, [-60., 0.], 2., true);
    assert_atlas(&second_actor, 21, 20);
    assert_ne!(
        second_actor, first_actor,
        "real Step changes the actor's clip pixels"
    );
    assert_eq!(document(&reopened), &saved);
    assert_eq!(files(&root), expected_files);
    click(&mut reopened, LBL_STOP);
    assert_eq!(simulation(&reopened), baseline);

    let info = reopened
        .state()
        .render_state
        .as_ref()
        .unwrap()
        .adapter
        .get_info();
    fs::write(
        evidence.join("actor-source-receipt.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "completed": true,
            "adapter": info.name,
            "backend": format!("{:?}", info.backend),
            "device_type": format!("{:?}", info.device_type),
            "input": "actual AccessKit widget actions and egui viewport pointer events",
            "native_os_input": false,
            "source_pixel_oracle": "literal installed 64x16 original atlas; opaque source pixel centers; tolerance 3",
            "preserved": ["displayed Frame checksum", "document checksum", "scene history", "binding keys and metadata", "atlas TextureId and bytes", "tile pixels"],
            "saved_source": "Clip walk",
            "play_literal_regions": [20, 21],
            "play_first_tick": first_tick,
            "play_second_tick": second_tick,
            "actual_step_clicks": first_steps + second_steps,
            "fresh_host_reopen": true,
            "later_tile_save": true,
            "captures": ["01-initial", "02-region", "03-undo", "04-redo", "05-clip-dirty", "06-tile-saved", "07-reopened", "08-play-paused", "09-play-second-frame"]
        })).unwrap(),
    ).unwrap();
}
