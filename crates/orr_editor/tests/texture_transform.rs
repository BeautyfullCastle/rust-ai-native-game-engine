//! Installed glTF texture transforms through the production editor and viewport.
//! ORR_REQUIRE_GPU=1 makes GPU evidence mandatory. Optional PPM evidence goes to
//! ORR_TEXTURE_TRANSFORM_CAPTURE_DIR. No isolated renderer or synthetic pixels.
#![cfg(feature = "models")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_editor::{
    app::{LBL_PAUSE, LBL_PLAY, LBL_STOP},
    game::EditorGame,
    Editor, EditorApp, Mode,
};
use orr_package::{Project, Runtime};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const PACKAGE: &str = "texture-transform-acceptance";
const TRANSFORMED: &str = "transformed.gltf";
const IDENTITY: &str = "identity.gltf";
const BAKED: &str = "baked.gltf";
const INVALID: [&str; 5] = [
    "bad-shape.gltf",
    "bad-texcoord.gltf",
    "undeclared.gltf",
    "unknown.gltf",
    "bad-output.gltf",
];

// Extract unchanged, original asymmetric geometry from the existing CC0 fixture.
// The explicit four-corner UV oracle below does not call the importer or repeat
// its transform implementation. Nearest sampling keeps GPU comparisons exact.
fn write_package(source: &Path) {
    fs::create_dir_all(source).unwrap();
    let glb = include_bytes!("../../../assets/imported_scene_demo/foreground.glb");
    let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let mut document: Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
    let binary_start = 28 + json_len;
    let binary_len = document["buffers"][0]["byteLength"].as_u64().unwrap() as usize;
    let binary = &glb[binary_start..binary_start + binary_len];
    document["buffers"][0]["uri"] = json!("source.bin");
    document["images"] = json!([{"uri":"asymmetric.png"}]);
    fs::write(source.join("source.bin"), binary).unwrap();
    let uv_index = document["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"]
        .as_u64()
        .unwrap() as usize;
    let accessor = &document["accessors"][uv_index];
    let view = &document["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
    let start = view["byteOffset"].as_u64().unwrap() as usize;
    let count = accessor["count"].as_u64().unwrap() as usize;
    let mut baked_binary = binary.to_vec();
    for index in 0..count {
        let at = start + index * 8;
        let u = f32::from_le_bytes(binary[at..at + 4].try_into().unwrap());
        let v = f32::from_le_bytes(binary[at + 4..at + 8].try_into().unwrap());
        let expected: [f32; 2] = match [u, v] {
            [0.0, 1.0] => [0.125, 0.125],
            [1.0, 1.0] => [0.125, 0.625],
            [1.0, 0.0] => [0.875, 0.625],
            [0.0, 0.0] => [0.875, 0.125],
            _ => panic!("fixture contains an unexpected UV corner"),
        };
        baked_binary[at..at + 4].copy_from_slice(&expected[0].to_le_bytes());
        baked_binary[at + 4..at + 8].copy_from_slice(&expected[1].to_le_bytes());
    }
    fs::write(source.join("baked.bin"), baked_binary).unwrap();
    let mut rgba = Vec::new();
    for y in 0..8_u8 {
        for x in 0..8_u8 {
            rgba.extend_from_slice(&[25 + 29 * x, 20 + 31 * y, 240 - 17 * x - 11 * y, 255]);
        }
    }
    let mut encoder = png::Encoder::new(
        fs::File::create(source.join("asymmetric.png")).unwrap(),
        8,
        8,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&rgba)
        .unwrap();
    let write = |name: &str, value: &Value| {
        fs::write(source.join(name), serde_json::to_vec(value).unwrap()).unwrap()
    };
    write(IDENTITY, &document);
    let mut baked = document.clone();
    baked["buffers"][0]["uri"] = json!("baked.bin");
    write(BAKED, &baked);
    document["extensionsUsed"] = json!(["KHR_texture_transform"]);
    document["extensionsRequired"] = json!(["KHR_texture_transform"]);
    // Extension texCoord=0 explicitly overrides a base texCoord=1.
    document["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"] = json!({
        "index":0,"texCoord":1,"extensions":{"KHR_texture_transform":{
            "offset":[0.875,0.125],"rotation":std::f32::consts::FRAC_PI_2,
            "scale":[0.5,0.75],"texCoord":0
        }}
    });
    write(TRANSFORMED, &document);
    for (name, field, value) in [
        (INVALID[0], "offset", json!([0.5])),
        (INVALID[1], "texCoord", json!(1)),
        (INVALID[3], "unexpected", json!(true)),
        (INVALID[4], "offset", json!([1.0e30, 0.0])),
    ] {
        let mut invalid = document.clone();
        invalid["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["extensions"]
            ["KHR_texture_transform"][field] = value;
        write(name, &invalid);
    }
    document.as_object_mut().unwrap().remove("extensionsUsed");
    document
        .as_object_mut()
        .unwrap()
        .remove("extensionsRequired");
    write(INVALID[2], &document);
    let mut files = vec![
        TRANSFORMED,
        IDENTITY,
        BAKED,
        "source.bin",
        "baked.bin",
        "asymmetric.png",
    ];
    files.extend(INVALID);
    write(
        "orr.package.json",
        &json!({"schema":1,"name":PACKAGE,"version":"1.0.0","engine":"^0.0.1","capabilities":["models"],"dependencies":{},"files":files}),
    );
}

fn prepare(root: &Path) -> PathBuf {
    let root = root.canonicalize().unwrap();
    let source = root.join("source-package");
    write_package(&source);
    Project::open_for_install(&root, Runtime::content_only().engine_version)
        .unwrap()
        .install(&[source])
        .unwrap();
    let scene = root.join("yard.scene.yaml");
    fs::write(
        &scene,
        include_str!("../../../scenes/yard3d_authoring.scene.yaml"),
    )
    .unwrap();
    scene
}
fn editor(scene: &Path) -> Editor {
    let mut editor = Editor::open_game(scene, EditorGame::Yard3D).unwrap();
    editor.sync();
    editor
}
fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..2 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
    assert!(h.state().editor.down().is_none());
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(2);
    h.get_by_label(label).click();
    settle(h);
}
fn wait_tick(h: &mut Harness<'_, EditorApp>, tick: u64) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        settle(h);
        if h.state().editor.yard_rows_coherent()
            && h.state()
                .editor
                .timeline()
                .is_some_and(|t| t.tick == tick && !t.playing)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "host failed to reach paused tick {tick}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn assign(h: &mut Harness<'_, EditorApp>, asset: &str) {
    h.state_mut().models.package = PACKAGE.into();
    h.state_mut().models.asset = asset.into();
    click(h, "Assign static model");
    assert!(
        h.state().models.error().is_none(),
        "{:?}",
        h.state().models.error()
    );
    assert_eq!(h.state().models.placements(&h.state().editor).len(), 1);
}
fn frame_bytes(h: &mut Harness<'_, EditorApp>) -> Vec<u8> {
    settle(h);
    let snapshot = h.state().editor.snapshot().unwrap();
    let (tick, checksum) = (snapshot.predicted().tick(), snapshot.predicted().checksum());
    let client = h
        .state_mut()
        .editor
        .agent_client("texture-transform-byte-probe")
        .unwrap();
    client.local_frames.clear();
    client
        .call(
            "watch.subscribe",
            json!({"topics":["frames"],"source":"view","max_fps":60}),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        client.poll().unwrap();
        while let Some(frame) = client.local_frames.pop_front() {
            if frame.frame.tick() == tick && frame.frame.checksum() == checksum {
                return frame.frame.to_bytes();
            }
        }
        assert!(
            Instant::now() < deadline,
            "matching authoritative frame did not arrive"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn trajectory(h: &mut Harness<'_, EditorApp>) -> Vec<Vec<u8>> {
    let mut frames = vec![frame_bytes(h)];
    assert!(h.state_mut().editor.start_play());
    wait_tick(h, 0);
    frames.push(frame_bytes(h));
    h.state_mut().editor.step(30);
    wait_tick(h, 30);
    frames.push(frame_bytes(h));
    click(h, "⏮");
    wait_tick(h, 0);
    assert_eq!(frame_bytes(h), frames[1]);
    click(h, LBL_STOP);
    assert_eq!(frame_bytes(h), frames[0]);
    frames
}

#[test]
fn installed_transform_widgets_save_reopen_reject_malformed_and_preserve_host_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let scene = prepare(temp.path());
    let ed = editor(&scene);
    let mut h = Harness::builder()
        .with_size([1500.0, 1200.0])
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |_| EditorApp::new(ed, None));
    settle(&mut h);
    click(&mut h, "box_right");
    let without = trajectory(&mut h);
    click(&mut h, "Static model binding");
    click(&mut h, "Create model bindings");
    assign(&mut h, TRANSFORMED);
    let transformed = h.state().models.placements(&h.state().editor)[0]
        .model
        .clone();
    let baked = orr_editor::model_bindings::load_asset(temp.path(), PACKAGE, BAKED).unwrap();
    let expected = baked.static_model().unwrap().source();
    let actual = transformed.source();
    assert_eq!(actual.format, "orr_static_model");
    assert_eq!(actual.version, 1);
    assert_eq!(actual.materials, expected.materials);
    assert_eq!(actual.images, expected.images);
    assert_eq!(actual.primitives.len(), expected.primitives.len());
    for (a, b) in actual.primitives.iter().zip(&expected.primitives) {
        assert_eq!(a.indices, b.indices);
        assert_eq!(a.transform, b.transform);
        assert_eq!(a.vertices.len(), b.vertices.len());
        for (a, b) in a.vertices.iter().zip(&b.vertices) {
            assert_eq!(a.position, b.position);
            assert_eq!(a.normal, b.normal);
            for (u, v) in a.uv.into_iter().zip(b.uv) {
                assert!(
                    (u - v).abs() < 1.0e-6,
                    "wrong once-baked UV: {a:?} vs {b:?}"
                );
            }
        }
    }
    let scene_before = fs::read(&scene).unwrap();
    click(&mut h, "Save model bindings");
    let bindings = h.state().models.bindings.as_ref().unwrap();
    let document = bindings.document().clone();
    let sidecar = bindings.path.clone();
    let saved = fs::read(&sidecar).unwrap();
    assert_eq!(
        document.version, 2,
        "source extension must not change binding schema"
    );
    for invalid in INVALID {
        h.state_mut().models.asset = invalid.into();
        click(&mut h, "Assign static model");
        assert!(h.state().models.error().is_some(), "admitted {invalid}");
        assert_eq!(
            h.state().models.bindings.as_ref().unwrap().document(),
            &document
        );
        assert!(std::sync::Arc::ptr_eq(
            &transformed,
            &h.state().models.placements(&h.state().editor)[0].model
        ));
        assert_eq!(fs::read(&sidecar).unwrap(), saved);
    }
    click(&mut h, "Discard model bindings");
    click(&mut h, "Open model bindings");
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &document
    );
    assert_eq!(
        trajectory(&mut h),
        without,
        "texture transform must not enter simulation state"
    );
    let reopened = editor(&scene);
    let mut panel = orr_editor::model_panel::ModelPanel::default();
    panel.open_for_editor(&reopened, false).unwrap();
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &document);
    assert_eq!(
        panel.placements(&reopened)[0].model.source(),
        transformed.source()
    );
    assert_eq!(fs::read(&scene).unwrap(), scene_before);
    assert_eq!(fs::read(sidecar).unwrap(), saved);
}

fn pixels(h: &Harness<'_, EditorApp>, name: &str) -> Vec<u8> {
    let viewport = h
        .state()
        .viewport3d_gpu()
        .expect("production EditorApp main GPU viewport");
    let size = h.state().ui.viewport_px;
    let rgba = viewport.gpu().read_rgba8();
    assert_eq!(rgba.len(), (size.0 * size.1 * 4) as usize);
    if let Some(dir) = std::env::var_os("ORR_TEXTURE_TRANSFORM_CAPTURE_DIR") {
        fs::create_dir_all(&dir).unwrap();
        let mut output = fs::File::create(Path::new(&dir).join(format!("{name}.ppm"))).unwrap();
        write!(output, "P6\n{} {}\n255\n", size.0, size.1).unwrap();
        for pixel in rgba.as_chunks::<4>().0 {
            output.write_all(&pixel[..3]).unwrap();
        }
    }
    rgba
}
fn changed(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len());
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .filter(|(a, b)| a != b)
        .count()
}

#[test]
fn production_main_viewport_matches_baked_uv_oracle_through_play_seek_stop() {
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(gpu) => eprintln!(
            "texture transform GPU adapter: {} (software: {})",
            gpu.adapter_name(),
            gpu.is_software()
        ),
        Err(error) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required GPU unavailable: {error}"
            );
            eprintln!("SKIP: texture transform GPU unavailable: {error}");
            return;
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let scene = prepare(temp.path());
    let ed = editor(&scene);
    let mut h = Harness::builder()
        .with_size([1500.0, 1200.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(move |cc| EditorApp::new(ed, cc.wgpu_render_state.clone()));
    settle(&mut h);
    click(&mut h, "box_right");
    click(&mut h, "Static model binding");
    click(&mut h, "Create model bindings");
    assign(&mut h, IDENTITY);
    let identity = pixels(&h, "identity");
    let identity_ui = h.render().unwrap();
    let checksum = h.state().editor.checksum();
    let edit_bytes = frame_bytes(&mut h);
    assign(&mut h, BAKED);
    let baked = pixels(&h, "baked-oracle");
    assign(&mut h, TRANSFORMED);
    let transformed = pixels(&h, "transformed");
    assert!(
        changed(&identity, &transformed) > 100,
        "extension must visibly change asymmetric texture sampling"
    );
    assert_eq!(
        transformed, baked,
        "GPU appearance must match independent baked UV corners"
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(frame_bytes(&mut h), edit_bytes);
    let rendered = h.render().unwrap();
    let rect = h.state().ui.viewport_rect.unwrap().shrink(4.0);
    assert!(
        rendered
            .enumerate_pixels()
            .filter(|(x, y, p)| {
                let point = egui::pos2(*x as f32, *y as f32);
                rect.contains(point)
                    && point.y > rect.min.y + 40.0
                    && *p != identity_ui.get_pixel(*x, *y)
            })
            .count()
            > 100,
        "composed native main viewport must display transformed texture"
    );
    click(&mut h, "Save model bindings");
    let sidecar = h.state().models.bindings.as_ref().unwrap().path.clone();
    let saved = fs::read(&sidecar).unwrap();
    click(&mut h, "Discard model bindings");
    click(&mut h, "Open model bindings");
    assert_eq!(pixels(&h, "reopened"), transformed);
    click(&mut h, LBL_PLAY);
    click(&mut h, LBL_PAUSE);
    click(&mut h, "⏮");
    wait_tick(&mut h, 0);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    assert!(h
        .get_by_label("Assign static model")
        .accesskit_node()
        .is_disabled());
    let play0 = pixels(&h, "play-zero");
    let play0_bytes = frame_bytes(&mut h);
    h.state_mut().editor.step(30);
    wait_tick(&mut h, 30);
    assert!(changed(&play0, &pixels(&h, "step-30")) > 100);
    click(&mut h, "⏮");
    wait_tick(&mut h, 0);
    assert_eq!(pixels(&h, "seek-zero"), play0);
    assert_eq!(frame_bytes(&mut h), play0_bytes);
    click(&mut h, LBL_STOP);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    assert_eq!(pixels(&h, "stopped"), transformed);
    assert_eq!(frame_bytes(&mut h), edit_bytes);
    assert_eq!(fs::read(sidecar).unwrap(), saved);
}
