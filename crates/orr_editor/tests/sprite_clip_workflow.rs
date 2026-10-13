//! Real authoring -> package admission -> EditorApp Frame -> export acceptance.
//! Prepared source only until the exact gated commands are executed.
#![cfg(all(feature = "sprite-clip-editor", target_os = "linux"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
#[path = "common/arena_sprites.rs"]
mod arena;
#[path = "common/sprite_clip_export.rs"]
mod export;

use arena::{Fixture, HERO, PACKAGE};
use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::{NodeT, Queryable}};
use orr_editor::{EditorApp, Mode, app::{LBL_PLAY, LBL_PAUSE, LBL_STEP, LBL_STOP},
    sprite_clip_panel::{ClipPanel, SaveOutcome}, sprite_bindings::Source};
use orr_package::{Project, Runtime};
use orr_sprite::{PlaybackMode, SpriteSource};
use std::{collections::BTreeMap, path::PathBuf};

fn clip_settle(h: &mut Harness<'_, ClipPanel>) { h.try_run().expect("bounded clip layout"); }
fn clip_click(h: &mut Harness<'_, ClipPanel>, label: &str) {
    h.get_by_label(label).scroll_to_me(); clip_settle(h);
    h.get_by_label(label).click(); clip_settle(h);
}
fn clip_text(h: &mut Harness<'_, ClipPanel>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).scroll_to_me(); clip_settle(h);
    h.get_by_role_and_label(Role::TextInput, label).focus(); clip_settle(h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A); clip_settle(h);
    h.get_by_role_and_label(Role::TextInput, label).type_text(value); clip_settle(h);
}
struct Authored {
    fixture: Fixture,
    _save_root: tempfile::TempDir,
    saved: PathBuf,
    source: SpriteSource,
    original_assets: BTreeMap<String, Vec<u8>>,
}
fn author() -> Authored {
    let fixture = Fixture::new();
    let mut runtime = Runtime::content_only();
    runtime.capabilities.insert("sprite".into()); // guaranteed by this target's feature gate
    let project = Project::open(fixture.root.path(), runtime).unwrap();
    let before_lock = std::fs::read(fixture.root.path().join("orr.packages.lock.json")).unwrap();
    let original = project.verify().unwrap().packages[PACKAGE].clone();
    let original_assets: BTreeMap<_, _> = original.files.keys().map(|p|
        (p.clone(), project.read_asset(PACKAGE, p).unwrap())).collect();
    let panel = ClipPanel::open_installed(fixture.root.path(), PACKAGE, "sprites.json", "walk", "1.0.1").unwrap();
    let original_source = panel.source().clone();
    let mut h = Harness::builder().with_size([1100.0, 1000.0]).with_max_steps(120)
        .build_ui_state(|ui, panel: &mut ClipPanel| { egui::ScrollArea::vertical().show(ui, |ui| panel.show(ui)); }, panel);
    clip_settle(&mut h);
    clip_text(&mut h, "Duration ms", "125");
    h.get_by_role_and_label(Role::ComboBox, "Playback").click(); clip_settle(&mut h);
    clip_click(&mut h, "Once");
    assert_eq!(h.get_by_role_and_label(Role::ComboBox, "Playback").accesskit_node().value().as_deref(), Some("Once"));
    clip_click(&mut h, "Apply frame draft");
    clip_click(&mut h, "Move frame later");
    let expected = h.state().source().clone();
    let walk = expected.clips.iter().find(|c| c.id == "walk").unwrap();
    assert_eq!(walk.mode, PlaybackMode::Once);
    assert_eq!(walk.frames.iter().map(|f| (f.region, f.duration_ms)).collect::<Vec<_>>(), [(21, 140), (20, 125)]);
    clip_click(&mut h, "Undo clip edit"); clip_click(&mut h, "Undo clip edit");
    assert_eq!(h.state().source(), &original_source);
    clip_click(&mut h, "Redo clip edit"); clip_click(&mut h, "Redo clip edit");
    assert_eq!(h.state().source(), &expected);
    let save_root = tempfile::tempdir().unwrap();
    let saved = save_root.path().canonicalize().unwrap().join("authored-package");
    clip_text(&mut h, "New package directory", saved.to_str().unwrap());
    clip_click(&mut h, "Save As new package");
    assert!(matches!(h.state().save_outcome(), SaveOutcome::Published { destination } if destination == &saved));
    assert_eq!(std::fs::read(fixture.root.path().join("orr.packages.lock.json")).unwrap(), before_lock);
    assert_eq!(project.verify().unwrap().packages[PACKAGE], original);
    for (p, bytes) in &original_assets {
        assert_eq!(project.read_asset(PACKAGE, p).unwrap(), *bytes);
        if p != "sprites.json" { assert_eq!(std::fs::read(saved.join(p)).unwrap(), *bytes); }
    }
    assert_eq!(expected.atlas, original_source.atlas);
    assert_eq!(expected.regions, original_source.regions);
    for clip in original_source.clips.iter().filter(|c| c.id != "walk") {
        assert_eq!(expected.clips.iter().find(|c| c.id == clip.id), Some(clip));
    }
    let saved_bytes = std::fs::read(saved.join("sprites.json")).unwrap();
    assert_eq!(serde_json::from_slice::<SpriteSource>(&saved_bytes).unwrap(), expected);
    clip_click(&mut h, "Save As new package");
    assert!(matches!(h.state().save_outcome(), SaveOutcome::Failed { published: false, .. }));
    assert_eq!(std::fs::read(saved.join("sprites.json")).unwrap(), saved_bytes);
    let installed = project.install(std::slice::from_ref(&saved)).unwrap();
    assert_eq!(installed.packages[PACKAGE].manifest.version, "1.0.1");
    assert_ne!(installed.packages[PACKAGE].digest, original.digest);
    eprintln!("authored package version={} old_digest={} new_digest={}", installed.packages[PACKAGE].manifest.version, original.digest, installed.packages[PACKAGE].digest);
    let reopened = ClipPanel::open_installed(fixture.root.path(), PACKAGE, "sprites.json", "walk", "1.0.2").unwrap();
    assert_eq!(reopened.source(), &expected);
    Authored { fixture, _save_root: save_root, saved, source: expected, original_assets }
}
fn settle(h: &mut Harness<'_, EditorApp>) { arena::settle(h); }
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).scroll_to_me();
    for _ in 0..20 { h.run_steps(1); }
    let a = h.get_by_role_and_label(Role::Button, label).rect(); h.run_steps(1);
    assert_eq!(h.get_by_role_and_label(Role::Button, label).rect(), a, "stable pointer target {label}");
    h.get_by_role_and_label(Role::Button, label).click(); settle(h);
}
fn assign_and_reopen(h: &mut Harness<'_, EditorApp>, f: &Fixture) {
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().entries.len();
    let scene = std::fs::read(&f.scene).unwrap();
    // Fixture-only path/scale setup; clip choice and assignment are actual controls.
    arena::configure(h, f);
    h.get_by_role_and_label(Role::ComboBox, "Sprite source").scroll_to_me(); settle(h);
    h.get_by_role_and_label(Role::ComboBox, "Sprite source").click(); settle(h);
    h.get_by_label("Clip walk").click(); settle(h);
    assert_eq!(h.state().sprites.source, Source::Clip("walk".into()));
    click(h, "Assign sprite to selection"); arena::no_error(h);
    click(h, "Save bindings"); arena::no_error(h);
    let expected = arena::document(h).clone();
    let sidecar = std::fs::read(&f.sidecar).unwrap();
    click(h, "Close bindings"); click(h, "Open bindings"); arena::no_error(h);
    assert_eq!(arena::document(h), &expected);
    assert_eq!(expected.bindings[HERO].source, Source::Clip("walk".into()));
    assert_eq!(arena::region(h.state(), HERO), Some(21));
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);
    assert_eq!(std::fs::read(&f.scene).unwrap(), scene);
    assert_eq!(std::fs::read(&f.sidecar).unwrap(), sidecar);
}
fn reopen_saved(h: &mut Harness<'_, EditorApp>, f: &Fixture) {
    settle(h);
    click(h, "hero");
    click(h, "Sprite bindings (view only)");
    h.state_mut().sprites.path = f.sidecar.to_string_lossy().into_owned(); // fixture path only
    click(h, "Open bindings"); arena::no_error(h);
    assert_eq!(arena::document(h).bindings[HERO].source, Source::Clip("walk".into()));
    assert_eq!(arena::region(h.state(), HERO), Some(21));
    eprintln!("fresh EditorApp/host reopened saved scene and authored clip binding");
}
fn frame_at(h: &mut Harness<'_, EditorApp>, tick: u64, expected: u32) {
    assert_eq!(h.state().editor.timeline().unwrap().tick, tick);
    assert_eq!(h.state().editor.snapshot().unwrap().tick_rate(), 60, "review fixture sampling if native tick rate changes");
    assert_eq!(arena::region(h.state(), HERO), Some(expected));
    eprintln!("authoritative Frame tick={tick} tick_rate=60 region={expected}");
}
fn begin_paused(h: &mut Harness<'_, EditorApp>) {
    click(h, LBL_PLAY);
    arena::wait_for(h, "actual Play frame", |app| app.editor.timeline().is_some_and(|t| t.playing));
    click(h, LBL_PAUSE);
    click(h, "⏮");
    frame_at(h, 0, 21);
}
fn step_to(h: &mut Harness<'_, EditorApp>, tick: u64) {
    while h.state().editor.timeline().unwrap().tick < tick {
        let previous = h.state().editor.timeline().unwrap().tick;
        click(h, LBL_STEP);
        assert_eq!(h.state().editor.timeline().unwrap().tick, previous + 1);
    }
}
#[test]
fn authored_package_widgets_install_assign_reopen_and_native_frames() {
    let authored = author();
    let mut h = authored.fixture.headless();
    assign_and_reopen(&mut h, &authored.fixture);
    begin_paused(&mut h);
    step_to(&mut h, 8); frame_at(&mut h, 8, 21);
    step_to(&mut h, 9); frame_at(&mut h, 9, 20);
    step_to(&mut h, 16); frame_at(&mut h, 16, 20);
    let paused = h.state().editor.timeline().unwrap().tick;
    h.run_steps(30); frame_at(&mut h, paused, 20);
    click(&mut h, LBL_STOP);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    assert_eq!(arena::region(h.state(), HERO), Some(21));
    assert_eq!(std::fs::read(authored.saved.join("lantern_keeper.png")).unwrap(), authored.original_assets["lantern_keeper.png"]);
    assert_eq!(authored.source.clips.iter().find(|c| c.id == "walk").unwrap().mode, PlaybackMode::Once);
    drop(h);
    let mut reopened = authored.fixture.headless();
    reopen_saved(&mut reopened, &authored.fixture);
}
fn capture(h: &mut Harness<'_, EditorApp>, name: &str) -> Vec<[u8; 3]> {
    let image = h.render().expect("actual EditorApp composed GPU output");
    let directory = PathBuf::from(std::env::var_os("ORR_SPRITE_CLIP_CAPTURE_DIR").expect("mandatory capture directory"));
    assert!(directory.is_dir());
    let output = directory.join(format!("{name}.png"));
    let file = std::fs::OpenOptions::new().create_new(true).write(true).open(output).unwrap();
    let mut encoder = png::Encoder::new(file, image.width(), image.height());
    encoder.set_color(png::ColorType::Rgba); encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(image.as_raw()).unwrap();
    let rect = h.state().ui.viewport_rect.unwrap();
    let body = arena::body_position(h.state(), HERO);
    let scale = arena::document(h).bindings[HERO].units_per_pixel;
    let mut pixels = Vec::new();
    for row in 0..16 { for col in 0..16 {
        let world = [body[0] + (col as f32 - 7.5) * scale, body[1] + (7.5 - row as f32) * scale];
        let at = h.state().editor.camera.world_to_screen(world, h.state().ui.viewport_px);
        let x = (rect.min.x + at[0]).floor() as u32;
        let y = (rect.min.y + at[1]).floor() as u32;
        assert!(x < image.width() && y < image.height());
        let rgba = image.get_pixel(x, y).0; pixels.push([rgba[0], rgba[1], rgba[2]]);
    }}
    assert!(pixels.iter().any(|p| p.iter().any(|v| *v > 40)));
    pixels
}
fn assert_atlas_region(pixels: &[[u8; 3]], authored: &Authored, region_id: u32) {
    let atlas = &authored.source.atlas;
    let rgba = orr_sample::project_sprites::decode_atlas(
        &authored.original_assets[&atlas.image], atlas.width, atlas.height).unwrap();
    let region = authored.source.regions.iter().find(|r| r.id == region_id).unwrap();
    assert_eq!((region.width, region.height), (16, 16));
    let mut opaque = 0;
    for y in 0..region.height { for x in 0..region.width {
        let at = (((region.y + y) * atlas.width + region.x + x) * 4) as usize;
        if rgba[at + 3] == 255 {
            opaque += 1;
            let actual = pixels[(y * region.width + x) as usize];
            assert!(actual.iter().zip(&rgba[at..at+3]).all(|(a,b)| a.abs_diff(*b) <= 3),
                "actual composed sprite texel must match PNG region {region_id}, ({x},{y}): {actual:?}");
        }
    }}
    assert!(opaque >= 32, "real atlas must supply a meaningful opaque-pixel oracle");
}
#[test]
#[ignore = "mandatory explicit GPU and pinned runtime/exporter acceptance"]
fn authored_clip_composed_gpu_and_source_hidden_readonly_export() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let authored = author();
    let editor = authored.fixture.editor();
    let mut h = Harness::builder().with_size([1500.0, 1400.0]).with_step_dt(1.0 / 60.0).wgpu()
        .build_eframe(move |cc| {
            eprintln!("actual EditorApp adapter: {:?}", cc.wgpu_render_state.as_ref().expect("WGPU renderer").adapter.get_info());
            EditorApp::new(editor, cc.wgpu_render_state.clone())
        });
    settle(&mut h); assert!(h.state().has_gpu());
    assign_and_reopen(&mut h, &authored.fixture);
    begin_paused(&mut h);
    let first = capture(&mut h, "clip-01-frame21");
    assert_atlas_region(&first, &authored, 21);
    step_to(&mut h, 9); frame_at(&mut h, 9, 20);
    let second = capture(&mut h, "clip-02-frame20");
    assert_atlas_region(&second, &authored, 20);
    assert!(first.iter().zip(&second).filter(|(a,b)| a != b).count() > 8, "authored frame changes actual sprite pixels");
    step_to(&mut h, 16); frame_at(&mut h, 16, 20);
    assert_eq!(capture(&mut h, "clip-03-once-end"), second);
    click(&mut h, LBL_STOP);
    assert_eq!(capture(&mut h, "clip-04-stop-restored"), first);
    drop(h);
    let editor = authored.fixture.editor();
    let mut reopened = Harness::builder().with_size([1500.0, 1400.0]).with_step_dt(1.0 / 60.0).wgpu()
        .build_eframe(move |cc| EditorApp::new(editor, cc.wgpu_render_state.clone()));
    reopen_saved(&mut reopened, &authored.fixture);
    let reopened_pixels = capture(&mut reopened, "clip-05-reopened-frame21");
    assert_atlas_region(&reopened_pixels, &authored, 21);
    assert_eq!(reopened_pixels, first);
    drop(reopened);
    std::fs::write(authored.fixture.root.path().join("orr.project.json"), br#"{"schema":2,"engine":"^0.0.1","entry":{"game":"arena","scene":"arena.scene.yaml","sprites":"arena.sprites.json"}}"#).unwrap();
    let evidence = export::assert_authored_export(authored.fixture.root.path(), &[
        export::CaptureSample { tick: 0, region: 21 },
        export::CaptureSample { tick: 8, region: 21 },
        export::CaptureSample { tick: 9, region: 20 },
        export::CaptureSample { tick: 15, region: 20 },
        export::CaptureSample { tick: 16, region: 20 },
        export::CaptureSample { tick: 60, region: 20 },
    ]);
    assert_eq!(evidence.tick_rate, 60);
    assert!(evidence.directory.is_dir());
}
