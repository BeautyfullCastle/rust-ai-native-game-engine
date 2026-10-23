//! Camera isolation and actual GPU/native-texture acceptance for animated previews.
#![cfg(feature = "animated-models")]
#![allow(clippy::float_arithmetic)]

use std::sync::Arc;

use orr_editor::animated_preview::{AnimatedPreviewGpu, GpuAnimatedPreview, PreviewCamera};
use orr_model::{animation::AnimatedModel, animation_import};
use orr_render::orr_rhi::{Rhi, Wgpu, WgpuOptions};

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

fn required_gpu() -> Option<Wgpu> {
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(rhi) => {
            eprintln!(
                "animated camera preview adapter: {} (software: {})",
                rhi.adapter_name(),
                rhi.is_software()
            );
            Some(rhi)
        }
        Err(error) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required animated camera GPU unavailable: {error}"
            );
            eprintln!("SKIP: animated camera GPU unavailable: {error}");
            None
        }
    }
}

fn assert_view_change_and_reset(
    preview: &mut AnimatedPreviewGpu,
    model: &Arc<AnimatedModel>,
    pose: &orr_model::animation::Pose,
    baseline: &[u8],
    gesture: impl FnOnce(&mut PreviewCamera) -> bool,
) {
    let initial_camera = *preview.camera();
    assert!(
        gesture(preview.camera_mut()),
        "valid camera gesture is applied"
    );
    assert!(
        preview
            .camera()
            .angles()
            .into_iter()
            .chain(preview.camera().offset())
            .chain([preview.camera().zoom_factor()])
            .all(f32::is_finite),
        "camera state remains finite after a valid gesture"
    );
    preview.draw(model, pose, (128, 128)).unwrap();
    let changed_pixels = preview.read_rgba8();
    assert_ne!(changed_pixels.as_slice(), baseline);
    assert!(
        preview.camera_mut().reset(),
        "reset restores the default view"
    );
    assert_eq!(preview.camera(), &initial_camera);
    preview.draw(model, pose, (128, 128)).unwrap();
    assert_eq!(
        preview.read_rgba8().as_slice(),
        baseline,
        "reset reproduces the exact default-view pixels"
    );
}

#[test]
fn preview_camera_rejects_nonfinite_inputs_and_invalid_viewports_without_mutation() {
    let mut camera = PreviewCamera::default();
    let initial = camera;

    assert!(!camera.orbit([f32::NAN, 1.0]));
    assert_eq!(camera, initial);
    assert!(!camera.orbit([1.0, f32::INFINITY]));
    assert_eq!(camera, initial);
    assert!(!camera.pan([f32::NAN, 1.0], [100.0, 100.0]));
    assert_eq!(camera, initial);
    assert!(!camera.pan([1.0, f32::INFINITY], [100.0, 100.0]));
    assert_eq!(camera, initial);
    assert!(!camera.pan([1.0, 1.0], [0.0, 100.0]));
    assert_eq!(camera, initial);
    assert!(!camera.pan([1.0, 1.0], [100.0, 0.0]));
    assert_eq!(camera, initial);
    assert!(!camera.pan([1.0, 1.0], [-1.0, 100.0]));
    assert_eq!(camera, initial);
    assert!(!camera.pan([1.0, 1.0], [f32::INFINITY, 100.0]));
    assert_eq!(camera, initial);
    assert!(!camera.pan([1.0, 1.0], [f32::NAN, 100.0]));
    assert_eq!(camera, initial);
    assert!(!camera.zoom(f32::NAN));
    assert_eq!(camera, initial);
    assert!(!camera.zoom(f32::NEG_INFINITY));
    assert_eq!(camera, initial);

    assert!(camera.orbit([20.0, 10.0]));
    assert!(camera.pan([8.0, -5.0], [128.0, 128.0]));
    assert!(camera.zoom(0.15));
    assert!(
        camera
            .angles()
            .into_iter()
            .chain(camera.offset())
            .chain([camera.zoom_factor()])
            .all(f32::is_finite),
        "valid gesture state remains finite"
    );
    assert_ne!(camera, initial);
    assert!(camera.reset());
    assert_eq!(camera, initial);
    assert!(!camera.reset());

    assert!(camera.orbit([f32::MAX, -f32::MAX]));
    assert!(camera.pan([f32::MAX, -f32::MAX], [1.0, 1.0]));
    assert!(camera.zoom(f32::MAX));
    assert!(
        camera
            .angles()
            .into_iter()
            .chain(camera.offset())
            .chain([camera.zoom_factor()])
            .all(f32::is_finite),
        "large finite gestures remain bounded to finite camera state"
    );
}

#[test]
fn pan_tracks_the_same_screen_distance_across_zoom_and_aspect() {
    let mut wide = PreviewCamera::default();
    assert!(wide.pan([0.0, 10.0], [200.0, 100.0]));
    let wide_offset = wide.offset()[1];

    let mut zoomed = PreviewCamera::default();
    assert!(zoomed.zoom(std::f32::consts::LN_2 / 0.15));
    assert!(zoomed.pan([0.0, 10.0], [200.0, 100.0]));
    assert!((zoomed.offset()[1] * 2.0 - wide_offset).abs() < 0.00001);

    let mut tall = PreviewCamera::default();
    assert!(tall.pan([0.0, 10.0], [50.0, 100.0]));
    assert!((tall.offset()[1] - wide_offset * 2.0).abs() < 0.00001);
    assert!(tall.orbit([20.0, 10.0]));
    let moved = tall;
    assert!(!tall.orbit([0.0, 0.0]));
    assert_eq!(tall, moved);
    assert!(tall.pan([f32::MAX, f32::MAX], [f32::MIN_POSITIVE, f32::MAX]));
    assert!(tall.offset().into_iter().all(f32::is_finite));
}

#[test]
fn actual_gpu_camera_gestures_change_pixels_reset_exactly_and_reload_resets_view() {
    let Some(rhi) = required_gpu() else { return };
    let model = fixture();
    let model_bytes = model.to_bytes().expect("serialize fixture model");
    let pose = model.sample_clip(0, 0.5).unwrap();
    let original_pose = pose.clone();
    let mut preview = AnimatedPreviewGpu::new(&rhi, (128, 128));

    preview.draw(&model, &pose, (128, 128)).unwrap();
    let default_camera = *preview.camera();
    let baseline = preview.read_rgba8();
    assert_eq!(baseline.len(), 128 * 128 * 4);

    assert_view_change_and_reset(&mut preview, &model, &pose, &baseline, |camera| {
        camera.orbit([20.0, 10.0])
    });
    assert_view_change_and_reset(&mut preview, &model, &pose, &baseline, |camera| {
        camera.pan([8.0, -5.0], [128.0, 128.0])
    });
    assert_view_change_and_reset(&mut preview, &model, &pose, &baseline, |camera| {
        camera.zoom(0.15)
    });

    let reloaded = fixture();
    assert!(!Arc::ptr_eq(&model, &reloaded));
    assert_eq!(model.to_bytes().unwrap(), model_bytes);
    assert_eq!(pose, original_pose);
    assert!(preview.camera_mut().orbit([20.0, 10.0]));
    preview.draw(&model, &pose, (128, 128)).unwrap();
    assert_ne!(preview.camera(), &default_camera);

    let reloaded_pose = reloaded.sample_clip(0, 0.5).unwrap();
    assert_eq!(reloaded_pose, pose);
    preview.draw(&reloaded, &reloaded_pose, (128, 128)).unwrap();
    assert_eq!(preview.camera(), &default_camera);
    assert_eq!(preview.read_rgba8(), baseline);
    assert_eq!(model.to_bytes().unwrap(), model_bytes);
    assert_eq!(pose, original_pose);
}

struct CameraTextureApp {
    state: egui_wgpu::RenderState,
    preview: Option<GpuAnimatedPreview>,
    model: Arc<AnimatedModel>,
    pose: orr_model::animation::Pose,
    target_size: (u32, u32),
    last_id: Option<egui::TextureId>,
}

impl CameraTextureApp {
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

impl eframe::App for CameraTextureApp {
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
fn native_egui_texture_reflects_camera_gestures_and_releases_after_close() {
    let Some(probe) = required_gpu() else { return };
    drop(probe);
    let model = fixture();
    let pose = model.sample_clip(0, 0.5).unwrap();
    let mut harness = egui_kittest::Harness::builder()
        .with_size([600.0, 400.0])
        .wgpu()
        .build_eframe(move |cc| CameraTextureApp::new(cc, model, pose));
    harness.run_steps(2);
    let initial_id = harness.state().last_id.expect("preview registered texture");
    let initial_frame = harness
        .render()
        .expect("egui samples the native preview texture")
        .as_raw()
        .clone();
    assert!(harness
        .state()
        .state
        .renderer
        .read()
        .texture(&initial_id)
        .is_some());

    assert!(harness
        .state_mut()
        .preview
        .as_mut()
        .unwrap()
        .camera_mut()
        .orbit([20.0, 10.0]));
    harness.run_steps(2);
    let changed_frame = harness
        .render()
        .expect("egui composes the camera-adjusted preview texture")
        .as_raw()
        .clone();
    assert_ne!(initial_frame, changed_frame);

    assert!(harness
        .state_mut()
        .preview
        .as_mut()
        .unwrap()
        .camera_mut()
        .reset());
    harness.run_steps(2);
    let reset_frame = harness
        .render()
        .expect("egui redraws the reset preview texture")
        .as_raw()
        .clone();
    assert_eq!(initial_frame, reset_frame);

    harness.state_mut().target_size = (128, 80);
    harness.run_steps(2);
    let resized_id = harness.state().last_id.unwrap();
    assert_eq!(resized_id, initial_id);
    assert_eq!(harness.state().preview.as_ref().unwrap().size(), (128, 80));
    assert!(harness
        .state()
        .state
        .renderer
        .read()
        .texture(&initial_id)
        .is_some());

    drop(harness.state_mut().preview.take());
    assert!(harness
        .state()
        .state
        .renderer
        .read()
        .texture(&initial_id)
        .is_none());
}
