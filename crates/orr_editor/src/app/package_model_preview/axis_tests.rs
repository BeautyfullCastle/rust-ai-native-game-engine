use super::*;
use crate::package_model_preview::PreviewBounds;
fn bounds() -> PreviewBounds {
    PreviewBounds {
        center: [3.0, 7.0, -2.0],
        radius: 4.0,
    }
}
#[test]
fn axis_presets_face_cardinal_directions_and_refit_zoom() {
    let directions = [
        [0.0, 0.0, -1.0],
        [-1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
    ];
    let mut controls = PreviewCamera::default();
    for (index, expected) in directions.into_iter().enumerate() {
        controls.zoom(8.0);
        assert!(controls.horizontal_view(index));
        let camera = controls.camera(bounds(), (640, 480));
        let mut fitted = PreviewCamera::default();
        fitted.horizontal_view(index);
        assert_eq!(camera, fitted.camera(bounds(), (640, 480)));
        assert_eq!(camera.target, bounds().center);
        for (actual, expected) in camera.forward().into_iter().zip(expected) {
            assert!((actual - expected).abs() < 0.00001);
        }
        assert!(!controls.horizontal_view(index));
        assert_eq!(camera, controls.camera(bounds(), (640, 480)));
    }
}
#[test]
fn presets_fit_corners_across_aspects_and_keep_extreme_matrices_finite() {
    for bounds in [
        bounds(),
        PreviewBounds {
            center: [1.0e9; 3],
            radius: 0.125,
        },
    ] {
        for size in [(512, 512), (128, 768), (768, 128)] {
            for index in 0..4 {
                let mut controls = PreviewCamera::default();
                controls.horizontal_view(index);
                let camera = controls.camera(bounds, size);
                assert_ne!(camera.eye, camera.target);
                assert!(camera
                    .view_proj(size.0 as f32 / size.1 as f32)
                    .0
                    .into_iter()
                    .flatten()
                    .all(f32::is_finite));
                if bounds.radius > 1.0 {
                    for x in [-2.0, 2.0] {
                        for y in [-2.0, 2.0] {
                            for z in [-2.0, 2.0] {
                                let point = [
                                    bounds.center[0] + x,
                                    bounds.center[1] + y,
                                    bounds.center[2] + z,
                                ];
                                let screen = camera.world_to_screen(point, size).unwrap();
                                assert!((0.0..=size.0 as f32).contains(&screen[0]));
                                assert!((0.0..=size.1 as f32).contains(&screen[1]));
                            }
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn free_orbit_zoom_reset_and_invalid_preset_remain_well_defined() {
    let mut controls = PreviewCamera::default();
    for index in 0..4 {
        controls.horizontal_view(index);
        let preset = controls;
        assert!(!controls.horizontal_view(usize::MAX));
        assert_eq!(controls, preset);
        assert!(controls.orbit([17.0, 23.0]));
        assert_ne!(controls, preset);
        assert!(controls.zoom(1.0));
        assert!(controls.reset());
        assert_eq!(controls, PreviewCamera::default());
    }
}
fn ui_frame(
    ctx: &egui::Context,
    camera: &mut PreviewCamera,
    events: Vec<egui::Event>,
) -> (bool, egui::FullOutput) {
    let mut changed = false;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 240.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            changed = horizontal_camera_controls(ui, camera);
        },
    );
    output.textures_delta.clear();
    (changed, output)
}
#[test]
fn production_buttons_select_each_camera_and_repeated_click_is_noop() {
    let ctx = egui::Context::default();
    let mut controls = PreviewCamera::default();
    for (index, label) in ["Front view", "Right view", "Back view", "Left view"]
        .into_iter()
        .enumerate()
    {
        let (_, out) = ui_frame(&ctx, &mut controls, vec![]);
        let text = out
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == label => Some(t),
                _ => None,
            })
            .unwrap();
        let pos = text.pos + text.galley.size() * 0.5;
        for repeat in 0..2 {
            ui_frame(
                &ctx,
                &mut controls,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            let (changed, _) = ui_frame(
                &ctx,
                &mut controls,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            assert_eq!(changed, repeat == 0);
            let mut expected = PreviewCamera::default();
            expected.horizontal_view(index);
            assert_eq!(controls, expected);
        }
    }
}
