//! Generated schema-3 package -> actual main EditorApp widgets -> composed GPU.
//! No editor/runtime changes, constructed sprite document, or viewport-only readback.
#![cfg(all(feature = "sprites", target_os = "linux"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

#[path = "common/arena_sprites.rs"]
mod common;
use common::{body_position, click, document, key, moving, no_error, settle, wait_for, HERO};
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{
    app::{LBL_PAUSE, LBL_PLAY, LBL_STOP},
    editor::input::Phase,
    game::EditorGame,
    sprite_bindings::Source,
    Editor, EditorApp, Mode,
};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use orr_sample::arena_game::{Arena, Position, Score};
use orr_sim::{PlayerSlot, Simulation, TickInputs};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

const PACKAGE: &str = "clips-editor";
const Z: [u8; 4] = [0, 0, 0, 0];
const R: [u8; 4] = [233, 31, 19, 255];
const G: [u8; 4] = [19, 211, 47, 255];
const B: [u8; 4] = [27, 61, 229, 255];
const FOOT: [u8; 4] = [255, 0, 255, 255];
const F0: [[u8; 4]; 9] = [R, G, Z, B, G, R, Z, FOOT, B];
const F1: [[u8; 4]; 16] = [G, Z, B, Z, R, B, R, G, Z, R, G, B, B, Z, FOOT, R];
const F2: [[u8; 4]; 4] = [B, G, FOOT, Z];
const SOURCES: [&[[u8; 4]]; 3] = [&F0, &F1, &F2];
const SIZES: [(usize, usize); 3] = [(3, 3), (4, 4), (2, 2)];
const OFFSETS: [(usize, usize); 3] = [(1, 1), (0, 0), (2, 2)];
const SCENE: &str = "schema: orr.scene/1\nsingletons:\n  Score: { kills: [0,0,0,0,0,0,0,0] }\nentities:\n  e_00000001:\n    name: hero\n    Position: { pos: [0,0] }\n    PlayerTag: { slot: 0 }\n";

fn png(path: &Path, width: u32, height: u32, rgba: &[u8]) {
    let mut encoder = png::Encoder::new(fs::File::create(path).unwrap(), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(rgba)
        .unwrap();
}
fn bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, at: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}
fn app(scene: &Path) -> Harness<'static, EditorApp> {
    let mut editor = Editor::open_game(scene, EditorGame::Arena).unwrap();
    editor.sync();
    editor.camera = orr_render::Camera::new([0., 0.], 20.);
    Harness::builder()
        .with_size([1500., 1400.])
        .with_step_dt(1. / 60.)
        // Match native eframe's production defaults. Kittest's PREDICTABLE
        // setting overrides TextureOptions::NEAREST with forced bilinear
        // filtering, which mixes neighboring PNG pixels at fractional poses.
        .with_render_options(egui_wgpu::RendererOptions::default())
        .wgpu()
        .build_eframe(move |cc| EditorApp::new(editor, cc.wgpu_render_state.clone()))
}
// Read the authoritative full frame through the same read-only frame stream
// used by existing editor acceptance tests. Only called with the host paused.
fn frame_bytes(editor: &mut Editor) -> Vec<u8> {
    editor.sync();
    let snapshot = editor.snapshot().unwrap();
    let tick = snapshot.tick();
    let checksum = snapshot.predicted().checksum();
    let client = editor.agent_client("png-clips-frame-proof").unwrap();
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
            "matching authoritative frame bytes missing"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn observed_state(app: &EditorApp) -> (u64, Vec<(orr_ecs::Entity, orr_fp::FPVec2)>, [u32; 8]) {
    let view = app.editor.snapshot().unwrap().predicted();
    (
        view.checksum(),
        view.iter::<Position>().map(|(e, p)| (e, p.pos)).collect(),
        view.singleton::<Score>().kills,
    )
}
fn choose(h: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    use egui::accesskit::Role;
    h.get_by_role_and_label(Role::ComboBox, label)
        .scroll_to_me();
    h.run_steps(2);
    h.get_by_role_and_label(Role::ComboBox, label).click();
    h.run_steps(2);
    h.get_by_role_and_label(Role::Button, value).click();
    settle(h);
}
fn expected_source(time: u64, moving: bool) -> usize {
    if moving {
        if (70..200).contains(&time) {
            2
        } else {
            1
        }
    } else if time % 250 < 100 {
        0
    } else {
        2
    }
}
fn source_pixel(frame: usize, x: usize, y: usize) -> [u8; 4] {
    let (ox, oy) = OFFSETS[frame];
    let (w, h) = SIZES[frame];
    if x >= ox && x < ox + w && y >= oy && y < oy + h {
        SOURCES[frame][(y - oy) * w + x - ox]
    } else {
        Z
    }
}
fn matches_source(samples: &[[u8; 3]], source: usize) -> bool {
    (0..5).all(|y| {
        (0..6).all(|x| {
            let expected = source_pixel(source, x, y);
            expected[3] != 255
                || samples[y * 6 + x]
                    .iter()
                    .zip(expected)
                    .all(|(a, e)| a.abs_diff(e) <= 3)
        })
    })
}
fn capture(
    h: &mut Harness<'_, EditorApp>,
    evidence: &Path,
    label: &str,
    expected: usize,
    records: &mut Vec<serde_json::Value>,
) -> Vec<[u8; 3]> {
    // This consumes the complete egui FullOutput and its sprite meshes, not
    // GpuViewport::read_rgba8 (which cannot see SpritePanel's overlay at all).
    let image = h
        .render()
        .expect("actual composed main EditorApp framebuffer");
    let (width, height) = (image.width(), image.height());
    let rgba = image.into_raw();
    let rect = h.state().ui.viewport_rect.unwrap();
    let body = body_position(h.state(), HERO);
    let pixels = h.state().ui.viewport_px;
    let mut samples = Vec::new();
    let mut probe_pixels = Vec::new();
    for y in 0..5 {
        for x in 0..6 {
            // Literal common canvas 6x5, scale2, source pixel centers. Neither UVs,
            // generated regions, sampled region IDs nor earlier captures are used.
            let world = [
                body[0] + (x as f32 - 2.5) * 2.,
                body[1] + (2. - y as f32) * 2.,
            ];
            let at = h.state().editor.camera.world_to_screen(world, pixels);
            let sx = (rect.min.x + at[0] * rect.width() / pixels.0 as f32).floor() as u32;
            let sy = (rect.min.y + at[1] * rect.height() / pixels.1 as f32).floor() as u32;
            assert!(
                sx < width && sy < height,
                "sprite must be inside actual framebuffer"
            );
            probe_pixels.push([sx, sy]);
            let offset = ((sy * width + sx) * 4) as usize;
            samples.push(rgba[offset..offset + 3].try_into().unwrap());
        }
    }
    let file = format!("{label}.png");
    png(&evidence.join(&file), width, height, &rgba);
    assert!(
        matches_source(&samples, expected),
        "{label}: source-known composed PNG colors differ for source frame {expected}: {samples:?}"
    );
    records.push(json!({"file":file,"width":width,"height":height,"source_frame":expected,"probe_pixels":probe_pixels,
        "tick":h.state().editor.timeline().map(|t|t.tick),"elapsed_ms":h.state().sprites.playback.state(HERO).elapsed_ms,
        "moving":moving(h.state(),HERO),"viewport":[rect.min.x,rect.min.y,rect.max.x,rect.max.y],
        "checksum":format!("0x{:016x}",h.state().editor.snapshot().unwrap().predicted().checksum())}));
    samples
}

#[test]
fn generated_named_clips_drive_composed_editor_locomotion_after_reopen() {
    let Some(binary) = std::env::var_os("ORR_PNG_SEQUENCE_PACKAGE_BIN") else {
        assert!(
            std::env::var_os("ORR_REQUIRE_GPU").is_none(),
            "required PNG producer CLI missing"
        );
        eprintln!("SKIP: ORR_PNG_SEQUENCE_PACKAGE_BIN not set");
        return;
    };
    let gpu = match Wgpu::headless(WgpuOptions::default()) {
        Ok(gpu) => gpu,
        Err(error) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required GPU: {error}"
            );
            eprintln!("SKIP: {error}");
            return;
        }
    };
    let adapter = gpu.adapter_name().to_owned();
    let info = gpu.adapter().get_info();
    let backend = format!("{:?}", info.backend);
    let device_type = format!("{:?}", info.device_type);
    assert_eq!(
        backend, "Vulkan",
        "acceptance requires real Vulkan composition"
    );
    drop(gpu);
    let root = tempfile::tempdir().unwrap();
    let authoring = tempfile::tempdir().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let evidence = std::env::var_os("ORR_PNG_CLIPS_EVIDENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| temp.path().join("evidence"))
        .join("editor");
    fs::create_dir_all(&evidence).unwrap();
    assert!(
        !evidence.join("receipt.json").exists(),
        "refuse stale receipt"
    );
    let producer = authoring.path().join("producer");
    fs::copy(Path::new(&binary).canonicalize().unwrap(), &producer).unwrap();
    let names = ["z-first.png", "a-second.png", "m-shared.png"];
    for i in 0..3 {
        png(
            &authoring.path().join(names[i]),
            SIZES[i].0 as u32,
            SIZES[i].1 as u32,
            &SOURCES[i].iter().flatten().copied().collect::<Vec<_>>(),
        );
    }
    fs::write(
        authoring.path().join("license.txt"),
        "CC0-1.0 test-owned PNGs\n",
    )
    .unwrap();
    let recipe = json!({"schema":3,"name":PACKAGE,"version":"1.0.0","license":"license.txt","columns":2,"canvas":{"width":6,"height":5},
        "frames":(0..3).map(|i|json!({"png":names[i],"offset":{"x":OFFSETS[i].0,"y":OFFSETS[i].1}})).collect::<Vec<_>>(),
        "clips":[{"id":"idle","mode":"loop","frames":[{"frame":0,"duration_ms":100},{"frame":2,"duration_ms":150}]},
            {"id":"walk","mode":"once","frames":[{"frame":1,"duration_ms":70},{"frame":2,"duration_ms":130},{"frame":1,"duration_ms":100}]}]});
    let recipe_path = authoring.path().join("recipe.json");
    fs::write(&recipe_path, serde_json::to_vec(&recipe).unwrap()).unwrap();
    let output = authoring.path().join("output");
    let result = Command::new(&producer)
        .current_dir(authoring.path())
        .arg("--recipe")
        .arg(&recipe_path)
        .arg("--out")
        .arg(&output)
        .output()
        .unwrap();
    fs::write(
        evidence.join("producer.log"),
        [result.stdout.as_slice(), result.stderr.as_slice()].concat(),
    )
    .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    orr_package::Project::open_for_install(
        root.path(),
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap()
    .install(&[output])
    .unwrap();
    authoring.close().unwrap();
    assert!(!producer.exists() && !recipe_path.exists());
    let project =
        orr_package::Project::open(root.path(), orr_sample::project_runtime::compiled_runtime())
            .unwrap();
    project.verify().unwrap();
    assert!(project
        .verify_package(PACKAGE)
        .unwrap()
        .verified_packages
        .contains_key(PACKAGE));
    let installed = bytes(&root.path().join(".orr"));
    let scene = root.path().join("arena.scene.yaml");
    let sidecar = root.path().join("arena.sprites.json");
    fs::write(&scene, SCENE).unwrap();
    let mut h = app(&scene);
    settle(&mut h);
    assert!(h.state().has_gpu());
    let authored_checksum = h.state().editor.checksum();
    let history = h.state().editor.history().entries.len();
    let initial = orr_remote::sample::arena_doc(SCENE)
        .unwrap()
        .frame()
        .clone();
    assert_eq!(initial.checksum(), authored_checksum);
    click(&mut h, "hero");
    click(&mut h, "Sprite bindings (view only)");
    {
        let panel = &mut h.state_mut().sprites;
        panel.path = sidecar.to_string_lossy().into_owned();
        panel.scene = "arena.scene.yaml".into();
        panel.project = ".".into();
    }
    click(&mut h, "Create bindings");
    no_error(&h);
    {
        let panel = &mut h.state_mut().sprites;
        panel.package = PACKAGE.into();
        panel.document = "sprites.json".into();
        panel.scale = 2.;
    }
    click(&mut h, "Load sprite document");
    no_error(&h);
    choose(&mut h, "Sprite source", "Idle / walk");
    // Deliberate actual-widget negative: swapping clip mapping visibly fails
    // the correct idle oracle, then correcting widgets restores the pixels.
    choose(&mut h, "Idle clip", "walk");
    choose(&mut h, "Walk clip", "idle");
    click(&mut h, "Assign sprite to selection");
    no_error(&h);
    h.event(egui::Event::PointerMoved(egui::pos2(2., 2.)));
    settle(&mut h);
    let mut records = Vec::new();
    let wrong = capture(&mut h, &evidence, "wrong-mapping", 1, &mut records);
    assert!(
        !matches_source(&wrong, 0),
        "wrong actual-widget mapping must fail the correct idle pixel oracle"
    );
    choose(&mut h, "Idle clip", "idle");
    choose(&mut h, "Walk clip", "walk");
    click(&mut h, "Assign sprite to selection");
    click(&mut h, "Follow selected entity");
    click(&mut h, "Save bindings");
    no_error(&h);
    assert_eq!(
        document(&h).bindings[HERO].source,
        Source::Locomotion {
            idle: "idle".into(),
            walk: "walk".into()
        }
    );
    assert_eq!(document(&h).bindings[HERO].document, "sprites.json");
    assert_eq!(document(&h).bindings.len(), 1);
    let authored = document(&h).clone();
    let saved = fs::read(&sidecar).unwrap();
    h.event(egui::Event::PointerMoved(egui::pos2(2., 2.)));
    settle(&mut h);
    capture(&mut h, &evidence, "authored-idle", 0, &mut records);
    assert_eq!(h.state().editor.checksum(), authored_checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);
    drop(h);
    // Tear down the host/app/asset cache and reopen from only installed bytes.
    let mut h = app(&scene);
    settle(&mut h);
    click(&mut h, "hero");
    click(&mut h, "Sprite bindings (view only)");
    h.state_mut().sprites.path = sidecar.to_string_lossy().into_owned();
    click(&mut h, "Open bindings");
    no_error(&h);
    assert_eq!(document(&h), &authored);
    assert!(!h.state().sprites.bindings.as_ref().unwrap().dirty());
    h.event(egui::Event::PointerMoved(egui::pos2(2., 2.)));
    settle(&mut h);
    capture(&mut h, &evidence, "reopened-idle", 0, &mut records);
    click(&mut h, LBL_PLAY);
    wait_for(&mut h, "real Arena Play", |app| {
        app.editor.can_take_control()
    });
    assert_eq!(h.state().editor.mode(), Mode::Play);
    click(&mut h, "Take control");
    wait_for(&mut h, "real managed keyboard control", |app| {
        app.editor.input_phase() == Phase::Active
    });
    let start = body_position(h.state(), HERO);
    key(&h, egui::Key::ArrowRight, true);
    wait_for(
        &mut h,
        "physical keyboard movement selects generated walk clip",
        |app| moving(app, HERO) && body_position(app, HERO)[0] > start[0],
    );
    // Let the Once clip reach its last frame while actual input is still held.
    wait_for(
        &mut h,
        "generated Once clip reaches end during real movement",
        |app| moving(app, HERO) && app.sprites.playback.state(HERO).elapsed_ms >= 350,
    );
    assert_eq!(
        h.state().editor.camera.center,
        body_position(h.state(), HERO)
    );
    let mut observed = BTreeMap::new();
    let snapshot = h.state().editor.snapshot().unwrap();
    observed.insert(snapshot.tick(), observed_state(h.state()));
    let state = h.state().sprites.playback.state(HERO);
    let walk = capture(
        &mut h,
        &evidence,
        "controlled-walk",
        expected_source(state.elapsed_ms, true),
        &mut records,
    );
    assert!(
        !matches_source(&walk, 0),
        "real movement pixels must reject idle source frame zero"
    );
    key(&h, egui::Key::ArrowRight, false);
    // Full-frame software GPU readback can exceed the presenter's eight-tick
    // discontinuity bound. A reset alone is not proof that key-up took effect:
    // require three coherent stationary host ticks after the actual event.
    wait_for(&mut h, "released key selects generated idle clip", |app| {
        !moving(app, HERO) && app.sprites.playback.state(HERO).elapsed_ms >= 50
    });
    let stopped_at = body_position(h.state(), HERO);
    let state = h.state().sprites.playback.state(HERO);
    capture(
        &mut h,
        &evidence,
        "released-idle",
        expected_source(state.elapsed_ms, false),
        &mut records,
    );
    // A real timer keeps advancing idle Loop; pose must hold while frames wrap.
    wait_for(&mut h, "idle loop wraps after key release", |app| {
        !moving(app, HERO) && app.sprites.playback.state(HERO).elapsed_ms >= 300
    });
    assert_eq!(body_position(h.state(), HERO), stopped_at);
    let snapshot = h.state().editor.snapshot().unwrap();
    observed.insert(snapshot.tick(), observed_state(h.state()));
    click(&mut h, LBL_PAUSE);
    let live = frame_bytes(&mut h.state_mut().editor);
    let live_checksum = h.state().editor.snapshot().unwrap().predicted().checksum();
    observed.insert(
        h.state().editor.snapshot().unwrap().tick(),
        observed_state(h.state()),
    );
    click(&mut h, LBL_STOP);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    let stopped = h.state().editor.last_stopped().unwrap().clone();
    // Replay actual admitted GUI input samples, not guessed wall-clock timings,
    // into a separate simulation that has never seen any presentation bindings.
    let reader = orr_session::ReplayReader::<Arena>::parse(&stopped.replay).unwrap();
    let mut control = Simulation::<Arena>::from_frame(
        &initial,
        reader.header.tick_rate,
        orr_remote::default_build_id("Arena"),
    )
    .unwrap();
    let mut saw_move = false;
    let mut saw_release = false;
    let mut compared = 0;
    for tick in 1..=reader.last_tick() {
        let (inputs, commands) = reader.tick(tick).unwrap();
        assert!(
            commands.is_empty(),
            "no fire commands in movement-only test"
        );
        saw_move |= inputs[0].axis_x == orr_fp::FP::ONE;
        saw_release |= saw_move && inputs[0].axis_x == orr_fp::FP::ZERO;
        let mut samples = TickInputs::new(tick, reader.header.player_count);
        for (slot, input) in inputs.iter().enumerate() {
            samples.set_input(PlayerSlot(slot as u8), *input);
        }
        control.step(&samples);
        if let Some(expected) = observed.get(&tick) {
            assert_eq!(
                &(
                    control.frame().checksum(),
                    control
                        .frame()
                        .dense::<Position>()
                        .0
                        .iter()
                        .copied()
                        .zip(control.frame().dense::<Position>().1.iter().map(|p| p.pos))
                        .collect::<Vec<_>>(),
                    control.frame().singleton::<Score>().kills
                ),
                expected,
                "composed GUI state diverged from presentation-free control at {tick}"
            );
            compared += 1;
        }
        if let Some((_, checksum)) = reader.checksums.iter().find(|(at, _)| *at == tick) {
            assert_eq!(
                control.frame().checksum(),
                *checksum,
                "recorded checksum mismatch at {tick}"
            );
        }
    }
    assert!(saw_move && saw_release && compared >= 2);
    assert_eq!(control.frame().to_bytes(), live);
    assert_eq!(control.frame().checksum(), live_checksum);
    assert_eq!(control.frame().checksum(), stopped.checksum);
    assert_eq!(h.state().editor.checksum(), authored_checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);
    assert_eq!(document(&h), &authored);
    assert_eq!(fs::read(&sidecar).unwrap(), saved);
    assert_eq!(fs::read_to_string(&scene).unwrap(), SCENE);
    assert_eq!(
        bytes(&root.path().join(".orr")),
        installed,
        "installed package changed during authoring/play"
    );
    assert_eq!(records.len(), 5);
    fs::write(evidence.join("receipt.json"),serde_json::to_vec_pretty(&json!({"schema":1,"test":"generated_named_clips_drive_composed_editor_locomotion_after_reopen",
        "adapter":adapter,"backend":backend,"device_type":device_type,"capture_count":records.len(),"captures":records,
        "main_editor_app":true,"source_hidden":true,"installed_bytes_preserved":true,"real_clip_widgets":true,"saved_and_reopened":true,"real_keyboard_move_and_release":true,
        "presentation_free_state_and_checksum_equal":true,"compared_live_frames":compared,"recorded_ticks":reader.last_tick()})).unwrap()).unwrap();
    eprintln!("PNG named clips main EditorApp: 5 composed Vulkan captures, real clip widgets/save/reopen/Play/Take control/movement/release, independent source-pixel oracle, presentation-free recorded-input state and checksum parity");
}
