//! Per-GUID playback and real offscreen GPU coverage for the animated preview.
#![cfg(feature = "animated-models")]
#![allow(clippy::float_arithmetic)]

use std::sync::Arc;

use orr_editor::animated_bindings::{
    self, Binding, Bindings, PlaybackMode as BindingMode, PlaybackSettings,
};
use orr_editor::animated_preview::{AnimatedPreviewGpu, GpuAnimatedPreview, PreviewPlayers};
use orr_model::{
    animation::{AnimatedModel, PlaybackMode, PlaybackState},
    animation_import,
};
use orr_package::{Project, Runtime};
use orr_reflect::Guid;
use orr_render::orr_rhi::{Rhi, Wgpu, WgpuOptions};
use std::io::Write;
use std::path::Path;

fn fixture_bytes() -> Vec<u8> {
    let imported = animation_import::import_with_resolver(
        "fixtures/animated_strip.glb",
        include_bytes!("../../orr_model/tests/fixtures/animated_strip.glb"),
        |_| panic!("the animated-strip GLB embeds all dependencies"),
    )
    .expect("import the real GLB animation fixture");
    imported.to_bytes().expect("cook imported animation")
}

fn fixture() -> Arc<AnimatedModel> {
    AnimatedModel::from_bytes(&fixture_bytes())
        .expect("reload the cooked animation")
        .into()
}

fn fixture_with_two_clips() -> Arc<AnimatedModel> {
    let mut source = fixture().source().clone();
    if source.clips.len() == 1 {
        let mut second = source.clips[0].clone();
        second.name = "duplicate test clip".into();
        source.clips.push(second);
    }
    Arc::new(AnimatedModel::new(source).expect("valid model with two preview clips"))
}

fn install_animation_package(project_root: &Path) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/animation_demo")
        .canonicalize()
        .unwrap();
    Project::open_for_install(project_root, Runtime::content_only().engine_version)
        .unwrap()
        .install(&[source])
        .unwrap();
}

fn guid(text: &str) -> Guid {
    Guid::parse(text).unwrap()
}

fn capture(name: &str, size: (u32, u32), rgba: &[u8]) {
    let Some(directory) = std::env::var_os("ORR_ANIMATION_CAPTURE_DIR") else {
        return;
    };
    let directory = Path::new(&directory);
    std::fs::create_dir_all(directory).expect("create animation capture directory");
    let mut file = std::fs::File::create(directory.join(format!("{name}.ppm")))
        .expect("create PPM animation capture");
    write!(file, "P6\n{} {}\n255\n", size.0, size.1).unwrap();
    for pixel in rgba.chunks_exact(4) {
        file.write_all(&pixel[..3]).unwrap();
    }
}

#[test]
fn guids_keep_independent_players_and_cover_controls_reload_and_close() {
    let model = fixture_with_two_clips();
    assert!(model.source().clips.len() >= 2);
    let duration = model.source().clips[0].duration();
    assert!(duration > 0.0);
    let a = guid("e_00000001");
    let b = guid("e_00000002");
    let mut previews = PreviewPlayers::default();

    previews
        .bind(&a, model.clone(), 0, PlaybackMode::Once, 1.0)
        .unwrap();
    previews
        .bind(&b, model.clone(), 1, PlaybackMode::Loop, 2.0)
        .unwrap();
    assert_eq!(previews.player(&a).unwrap().state(), PlaybackState::Paused);
    assert_eq!(previews.player(&b).unwrap().state(), PlaybackState::Paused);
    assert!(!previews.is_playing());
    assert_eq!(previews.player(&b).unwrap().clip(), Some(1));
    previews.play(&a).unwrap();
    previews.play(&b).unwrap();
    previews.tick(0.2).unwrap();
    assert!((previews.player(&a).unwrap().time() - 0.2).abs() < 1e-5);
    assert!((previews.player(&b).unwrap().time() - 0.4).abs() < 1e-5);
    assert_eq!(previews.player(&a).unwrap().state(), PlaybackState::Playing);
    assert_eq!(previews.player(&b).unwrap().state(), PlaybackState::Playing);

    // Pausing one GUID never freezes another GUID's player.
    previews.pause(&a).unwrap();
    assert!(previews.is_playing(), "the other GUID continues playing");
    previews.tick(0.2).unwrap();
    assert!((previews.player(&a).unwrap().time() - 0.2).abs() < 1e-5);
    assert!((previews.player(&b).unwrap().time() - 0.8).abs() < 1e-5);
    assert_eq!(previews.player(&a).unwrap().state(), PlaybackState::Paused);
    previews.play(&a).unwrap();
    previews.tick(0.1).unwrap();
    assert!((previews.player(&a).unwrap().time() - 0.3).abs() < 1e-5);

    // Switching clips is one transient rebind and restarts at a paused zero time.
    previews
        .bind(&b, model.clone(), 0, PlaybackMode::Loop, 2.0)
        .unwrap();
    assert_eq!(previews.player(&b).unwrap().clip(), Some(0));
    assert_eq!(previews.player(&b).unwrap().time(), 0.0);
    assert_eq!(previews.player(&b).unwrap().state(), PlaybackState::Paused);
    previews.play(&b).unwrap();

    // Once holds its terminal pose; loop wraps at the clip duration.
    previews.seek(&a, duration + 0.5).unwrap();
    assert_eq!(
        previews.player(&a).unwrap().state(),
        PlaybackState::Finished
    );
    assert!((previews.player(&a).unwrap().time() - duration).abs() < 1e-5);
    previews.seek(&b, duration + 0.25).unwrap();
    assert!(previews.player(&b).unwrap().time() < duration);
    assert!((previews.player(&b).unwrap().time() - 0.25).abs() < 1e-5);
    assert_eq!(previews.player(&b).unwrap().state(), PlaybackState::Playing);

    // Stop restores rest; seeking after stop selects the clip but stays paused.
    previews.stop(&a).unwrap();
    assert_eq!(previews.player(&a).unwrap().state(), PlaybackState::Stopped);
    assert!(previews.seek(&a, f32::NAN).is_err());
    assert!(previews.seek(&a, -1.0).is_err());
    assert_eq!(previews.player(&a).unwrap().state(), PlaybackState::Stopped);
    assert_eq!(
        previews.pose(&a).unwrap().unwrap(),
        model.rest_pose().unwrap()
    );
    previews.seek(&a, 0.35).unwrap();
    assert_eq!(previews.player(&a).unwrap().state(), PlaybackState::Paused);
    assert!((previews.player(&a).unwrap().time() - 0.35).abs() < 1e-5);
    previews.play(&a).unwrap();
    assert_eq!(previews.player(&a).unwrap().time(), 0.35);

    // Replacing the immutable asset Arc resets the player to the new identity.
    let reloaded = fixture();
    previews
        .bind(&a, reloaded.clone(), 0, PlaybackMode::Once, 1.0)
        .unwrap();
    assert_eq!(previews.player(&a).unwrap().time(), 0.0);
    assert_eq!(previews.player(&a).unwrap().state(), PlaybackState::Paused);
    assert!(Arc::ptr_eq(previews.model(&a).unwrap(), &reloaded));
    assert!(!Arc::ptr_eq(previews.model(&a).unwrap(), &model));

    assert!(previews.remove(&a));
    assert!(previews.player(&a).is_none());
    let keep = [b.clone()].into_iter().collect();
    previews.retain(&keep);
    assert_eq!(previews.len(), 1);
    previews.clear();
    assert!(previews.is_empty());
    assert!(!previews.is_playing());
}

fn gpu() -> Option<Wgpu> {
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(rhi) => {
            eprintln!(
                "animated editor-preview adapter: {} (software: {})",
                rhi.adapter_name(),
                rhi.is_software()
            );
            Some(rhi)
        }
        Err(error) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required animated preview GPU unavailable: {error}"
            );
            eprintln!("SKIP: animated preview GPU unavailable: {error}");
            None
        }
    }
}

#[test]
fn actual_skinned_preview_readback_changes_rest_midpoint_and_key_and_resizes() {
    let Some(rhi) = gpu() else { return };
    let model = fixture();
    let mut preview = AnimatedPreviewGpu::new(&rhi, (96, 96));
    let rest = model.rest_pose().unwrap();
    preview.draw(&model, &rest, (96, 96)).unwrap();
    let rest_pixels = preview.read_rgba8();
    assert_eq!(rest_pixels.len(), 96 * 96 * 4);
    let background = &rest_pixels[..3];
    let foreground_pixels = rest_pixels
        .chunks_exact(4)
        .filter(|pixel| pixel[..3] != *background)
        .count();
    assert!(
        foreground_pixels > 100,
        "rest-pose draw covers foreground pixels: {foreground_pixels}"
    );
    capture("animated-preview-rest", (96, 96), &rest_pixels);

    let midpoint = model.sample_clip(0, 0.5).unwrap();
    preview.draw(&model, &midpoint, (96, 96)).unwrap();
    let midpoint_pixels = preview.read_rgba8();
    assert_ne!(
        rest_pixels, midpoint_pixels,
        "animated midpoint deforms the actual GPU image"
    );
    capture("animated-preview-midpoint", (96, 96), &midpoint_pixels);

    let key = model.sample_clip(0, 1.0).unwrap();
    preview.draw(&model, &key, (96, 96)).unwrap();
    let key_pixels = preview.read_rgba8();
    assert_ne!(
        midpoint_pixels, key_pixels,
        "known key pose differs from the midpoint readback"
    );
    assert_ne!(
        rest_pixels, key_pixels,
        "known key pose differs from rest readback"
    );
    capture("animated-preview-key", (96, 96), &key_pixels);

    let generation = preview.generation();
    preview.draw(&model, &key, (144, 104)).unwrap();
    assert_eq!(preview.size(), (144, 104));
    assert_eq!(
        preview.generation(),
        generation + 1,
        "resize replaced the offscreen target once"
    );
    assert_eq!(preview.read_rgba8().len(), 144 * 104 * 4);
    let same_size_generation = preview.generation();
    preview.draw(&model, &midpoint, (144, 104)).unwrap();
    assert_eq!(
        preview.generation(),
        same_size_generation,
        "same-size redraw reuses the target"
    );

    // A cooked-model reload makes the GPU renderer rebuild for the new model identity.
    let reloaded = fixture();
    let reloaded_pose = reloaded.sample_clip(0, 0.5).unwrap();
    preview.draw(&reloaded, &reloaded_pose, (144, 104)).unwrap();
    assert_eq!(preview.size(), (144, 104));
    assert_eq!(preview.read_rgba8().len(), 144 * 104 * 4);
}

#[test]
fn installed_package_binding_roundtrip_reaches_the_same_gpu_preview_path() {
    let Some(rhi) = gpu() else { return };
    let temp = tempfile::tempdir().unwrap();
    install_animation_package(temp.path());
    let loaded = animated_bindings::load_asset(temp.path(), "sample-animation", "animated.glb")
        .expect("installed package imports and cooks its animated model");
    assert!(!loaded.clips().is_empty());

    let id = guid("e_00000001");
    let path = temp.path().join("fixture.animations.json");
    let mut bindings = Bindings::create(path.clone(), "scene.yaml".into(), ".".into()).unwrap();
    let binding = Binding::from_asset(
        loaded.package().to_owned(),
        loaded.asset().to_owned(),
        &loaded,
        loaded.clips()[0].index,
        PlaybackSettings {
            mode: BindingMode::Once,
            speed: 1.0,
        },
    )
    .unwrap();
    bindings
        .assign_validated(std::slice::from_ref(&id), &binding, &loaded)
        .unwrap();
    bindings.save().unwrap();
    let reopened = Bindings::open(path).unwrap();
    assert_eq!(
        reopened.document().bindings.get(&id.to_string()),
        Some(&binding)
    );

    let resolved = animated_bindings::load_binding(temp.path(), &binding)
        .expect("reopened binding resolves the exact installed package content");
    assert_eq!(resolved.package_digest(), loaded.package_digest());
    assert_eq!(resolved.source_hash(), loaded.source_hash());

    let mut players = PreviewPlayers::default();
    players
        .bind(
            &id,
            resolved.model().clone(),
            binding.clip_index,
            PlaybackMode::Once,
            1.0,
        )
        .unwrap();
    assert_eq!(players.player(&id).unwrap().state(), PlaybackState::Paused);
    let rest = players.pose(&id).unwrap().unwrap();
    let mut preview = AnimatedPreviewGpu::new(&rhi, (112, 112));
    preview.draw(resolved.model(), &rest, (112, 112)).unwrap();
    let rest_pixels = preview.read_rgba8();

    players.play(&id).unwrap();
    players.seek(&id, 0.5).unwrap();
    let midpoint = players.pose(&id).unwrap().unwrap();
    preview
        .draw(resolved.model(), &midpoint, (112, 112))
        .unwrap();
    assert_ne!(
        rest_pixels,
        preview.read_rgba8(),
        "installed and reopened clip reaches the GPU preview"
    );
}

struct PreviewTextureApp {
    state: egui_wgpu::RenderState,
    preview: Option<GpuAnimatedPreview>,
    model: Arc<AnimatedModel>,
    pose: orr_model::animation::Pose,
    target_size: (u32, u32),
    last_id: Option<egui::TextureId>,
}

impl PreviewTextureApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        model: Arc<AnimatedModel>,
        pose: orr_model::animation::Pose,
    ) -> Self {
        let state = cc
            .wgpu_render_state
            .as_ref()
            .expect("Harness initialized eframe's wgpu renderer")
            .clone();
        let preview = GpuAnimatedPreview::new(&state);
        Self {
            state,
            preview: Some(preview),
            model,
            pose,
            target_size: (96, 72),
            last_id: None,
        }
    }
}

impl eframe::App for PreviewTextureApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let id = self
            .preview
            .as_mut()
            .expect("preview remains open during draw")
            .draw(&self.model, &self.pose, self.target_size)
            .expect("native texture preview render");
        self.last_id = Some(id);
        ui.image((id, egui::vec2(240.0, 180.0)));
    }
}

#[test]
fn native_egui_texture_updates_on_resize_and_is_freed_on_close() {
    let Some(probe) = gpu() else { return };
    drop(probe);
    let model = fixture();
    let midpoint = model.sample_clip(0, 0.5).unwrap();
    let key = model.sample_clip(0, 1.0).unwrap();
    let mut harness = egui_kittest::Harness::builder()
        .with_size([600.0, 400.0])
        .wgpu()
        .build_eframe(move |cc| PreviewTextureApp::new(cc, model, midpoint));
    harness.run_steps(2);
    let initial = harness
        .state()
        .last_id
        .expect("preview registered a texture");
    assert_eq!(harness.state().preview.as_ref().unwrap().size(), (96, 72));
    assert!(harness
        .state()
        .state
        .renderer
        .read()
        .texture(&initial)
        .is_some());
    let frame = harness
        .render()
        .expect("egui samples the native preview texture");
    assert!(frame
        .pixels()
        .any(|pixel| pixel.0[0] > 40 || pixel.0[1] > 40 || pixel.0[2] > 40));
    let midpoint_frame = frame.as_raw().clone();
    capture(
        "animated-preview-egui-midpoint",
        (frame.width(), frame.height()),
        frame.as_raw(),
    );

    harness.state_mut().pose = key;
    harness.run_steps(2);
    let key_frame = harness
        .render()
        .expect("egui redraws the native texture after animation pose change");
    assert_ne!(
        &midpoint_frame,
        key_frame.as_raw(),
        "the composed egui image reflects the changed skinned pose"
    );
    capture(
        "animated-preview-egui-key",
        (key_frame.width(), key_frame.height()),
        key_frame.as_raw(),
    );

    harness.state_mut().target_size = (128, 80);
    harness.run_steps(2);
    let resized = harness.state().last_id.unwrap();
    assert_eq!(
        resized, initial,
        "egui texture identity stays stable across resize"
    );
    assert_eq!(harness.state().preview.as_ref().unwrap().size(), (128, 80));
    assert!(harness
        .state()
        .state
        .renderer
        .read()
        .texture(&initial)
        .is_some());
    let _ = harness.render().expect("resized native preview is sampled");

    // Closing the pane drops the wrapper, which must release the native egui ID.
    harness.state_mut().preview.take();
    assert!(harness
        .state()
        .state
        .renderer
        .read()
        .texture(&initial)
        .is_none());
}
