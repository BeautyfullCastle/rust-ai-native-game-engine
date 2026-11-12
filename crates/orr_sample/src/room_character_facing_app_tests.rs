//! CPU production App/native placement acceptance; no GPU/window claim.
#![allow(clippy::float_arithmetic, clippy::disallowed_types)]
use super::*;
use crate::room_character::{ForwardAxis, Motion, PlaybackRates};
use orr_model::animation::{Matrix4, Trs, multiply};
use orr_model_bindings::model_bindings::LocalTransform;

fn local() -> LocalTransform {
    LocalTransform {
        translation: [0.25, 0.125, -0.5],
        rotation: [0.0, 0.0, 1.0, 0.0],
        scale: [0.75, 1.25, 0.5],
    }
}
fn fixture(root: &Path, axis: ForwardAxis) -> PreparedProject {
    crate::project_create::create(&crate::project_create::CreateOptions {
        output: root.into(),
        template: crate::project_create::ROOM_CHARACTER_TEMPLATE.into(),
        seed: "facing-consumer".into(),
    })
    .unwrap();
    let path = root.join("room.character.json");
    let mut document =
        crate::room_character::Document::parse(&std::fs::read(&path).unwrap()).unwrap();
    document.schema = 7;
    document.speeds = Some(PlaybackRates::default());
    document.crossfade_ticks = Some(0);
    document.motion = Some(Motion {
        searching_idle: document.searching,
        carrying_idle: document.carrying,
        escaped_once: false,
        pickup: None,
        facing: Some(axis),
    });
    std::fs::write(path, document.to_bytes().unwrap()).unwrap();
    let path = root.join("room.models.json");
    let mut bindings: orr_model_bindings::model_bindings::Document =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    bindings
        .bindings
        .get_mut(&document.player)
        .unwrap()
        .transform = local();
    std::fs::write(path, serde_json::to_vec_pretty(&bindings).unwrap()).unwrap();
    PreparedProject::open_with_capabilities(
        root,
        crate::room_project::RoomProjectCapabilities {
            lighting: false,
            character: true,
        },
    )
    .unwrap()
}
fn axes() -> [ForwardAxis; 4] {
    [
        ForwardAxis::PositiveZ,
        ForwardAxis::NegativeZ,
        ForwardAxis::PositiveX,
        ForwardAxis::NegativeX,
    ]
}
fn right_yaw(axis: ForwardAxis) -> f32 {
    match axis {
        ForwardAxis::PositiveZ => std::f32::consts::FRAC_PI_2,
        ForwardAxis::NegativeZ => -std::f32::consts::FRAC_PI_2,
        ForwardAxis::PositiveX => 0.0,
        ForwardAxis::NegativeX => std::f32::consts::PI,
    }
}
fn forward_yaw(axis: ForwardAxis) -> f32 {
    match axis {
        ForwardAxis::PositiveZ => 0.0,
        ForwardAxis::NegativeZ => -std::f32::consts::PI,
        ForwardAxis::PositiveX => -std::f32::consts::FRAC_PI_2,
        ForwardAxis::NegativeX => std::f32::consts::FRAC_PI_2,
    }
}
fn oracle(frame: &orr_sim::Frame, project: &PreparedProject, yaw: f32) -> Matrix4 {
    let character = &project.character().unwrap().document;
    let entity = project
        .scene()
        .index()
        .entity(&orr_reflect::Guid::parse(&character.player).unwrap())
        .unwrap();
    let body = frame.get::<orr_physics3d::Body>(entity).unwrap();
    let body = orr_view::fp_to_transform3(body.pos, body.rot);
    let body = Trs {
        translation: body.pos.to_array(),
        rotation: body.rot.to_array(),
        scale: [1.0; 3],
    }
    .matrix();
    // Independent explicit Y rotation matrix, rather than the production helper.
    let (sin, cos) = yaw.sin_cos();
    let rotation = [
        [cos, 0.0, -sin, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [sin, 0.0, cos, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let local = local();
    multiply(
        multiply(body, rotation),
        Trs {
            translation: local.translation,
            rotation: local.rotation,
            scale: local.scale,
        }
        .matrix(),
    )
}
fn assert_matrix(actual: Matrix4, expected: Matrix4) {
    for (a, b) in actual.iter().flatten().zip(expected.iter().flatten()) {
        assert!((a - b).abs() < 0.00001, "{actual:?} != {expected:?}");
    }
}
fn actual(app: &App) -> Matrix4 {
    room_view::observed_character_placement(
        FrameView::of(app.sim.frame()),
        app.project.scene().index(),
        app.project.models(),
        &app.character_playback,
    )
    .unwrap()
    .unwrap()
    .transform
}
#[test]
fn facing_actual_app_moves_holds_idle_and_restarts_without_sources_for_all_authored_axes() {
    for (index, axis) in axes().into_iter().enumerate() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp
            .path()
            .canonicalize()
            .unwrap()
            .join(format!("facing-{index}"));
        let project = fixture(&root, axis);
        let mut baseline = project.scene().simulation().unwrap();
        let mut app = App::new(project).unwrap();
        let initial = app.sim.frame().to_bytes();
        let initial_matrix = actual(&app);
        assert_matrix(initial_matrix, oracle(app.sim.frame(), &app.project, 0.0));
        let saved_models = std::fs::read(root.join("room.models.json")).unwrap();
        std::fs::rename(&root, root.with_extension("hidden")).unwrap();
        for _cycle in 0..2 {
            for (input, yaw) in [
                (
                    RoomInput {
                        move_x: 1,
                        ..Default::default()
                    },
                    right_yaw(axis),
                ),
                (RoomInput::default(), right_yaw(axis)),
                (
                    RoomInput {
                        move_z: 1,
                        ..Default::default()
                    },
                    forward_yaw(axis),
                ),
                (RoomInput::default(), forward_yaw(axis)),
            ] {
                app.step_authoritative(input).unwrap();
                step(&mut baseline, input);
                assert_eq!(app.sim.frame().to_bytes(), baseline.frame().to_bytes());
                let before = app.sim.frame().to_bytes();
                let placed = actual(&app);
                assert_matrix(placed, oracle(app.sim.frame(), &app.project, yaw));
                app.observe_character().unwrap();
                assert_eq!(actual(&app), placed);
                assert_eq!(app.sim.frame().to_bytes(), before);
            }
            app.restart().unwrap();
            baseline = app.project.scene().simulation().unwrap();
            assert_eq!(app.sim.frame().to_bytes(), initial);
            assert_eq!(actual(&app), initial_matrix);
        }
        assert_eq!(
            std::fs::read(root.with_extension("hidden").join("room.models.json")).unwrap(),
            saved_models
        );
        assert_eq!(
            app.project.models().document.bindings
                [&app.project.character().unwrap().document.player]
                .transform,
            local()
        );
    }
}
#[cfg(all(feature = "project-export", target_arch = "x86_64"))]
#[test]
#[ignore = "explicit source-hidden native CPU acceptance requires ORR_ROOM_FACING_RUNTIME"]
fn facing_saved_axes_source_hidden_native_matrices_match_frame_oracle_and_restart() {
    use sha2::{Digest, Sha256};
    let built = PathBuf::from(
        std::env::var_os("ORR_ROOM_FACING_RUNTIME").expect("explicit built room runtime required"),
    );
    for (index, axis) in axes().into_iter().enumerate() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().canonicalize().unwrap();
        let root = parent.join("authored-room");
        let _prepared = fixture(&root, axis);
        let original_models = std::fs::read(root.join("room.models.json")).unwrap();
        let original_character = std::fs::read(root.join("room.character.json")).unwrap();
        let runtime = parent.join("trusted runtime");
        std::fs::copy(&built, &runtime).unwrap();
        let hash: String = Sha256::digest(std::fs::read(&runtime).unwrap())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let output = parent.join("deployed-room");
        crate::project_export::export_room(&crate::project_export::ExportOptions {
            project: root.clone(),
            runtime: runtime.clone(),
            runtime_sha256: hash,
            output: output.clone(),
            trusted_runtime: true,
            source_revision: None,
        })
        .unwrap();
        std::fs::rename(&root, root.with_extension("hidden")).unwrap();
        std::fs::rename(&runtime, runtime.with_extension("hidden")).unwrap();
        let prepared = PreparedProject::open_with_capabilities(
            &output.join("project"),
            crate::room_project::RoomProjectCapabilities {
                lighting: false,
                character: true,
            },
        )
        .unwrap();
        for (hold, input, yaw) in [
            (
                "right",
                RoomInput {
                    move_x: 1,
                    ..Default::default()
                },
                right_yaw(axis),
            ),
            (
                "down",
                RoomInput {
                    move_z: 1,
                    ..Default::default()
                },
                forward_yaw(axis),
            ),
        ] {
            let mut baseline = prepared.scene().simulation().unwrap();
            for _ in 0..2 {
                step(&mut baseline, input);
            }
            let expected = oracle(baseline.frame(), &prepared, yaw);
            let mut previous = None;
            for _restart in 0..2 {
                let receipt = std::process::Command::new(output.join("run-room-escape"))
                    .current_dir(&parent)
                    .args([
                        "--headless",
                        "--ticks",
                        "2",
                        "--hold",
                        hold,
                        "--check-model-poses",
                    ])
                    .env_remove("DISPLAY")
                    .env_remove("WAYLAND_DISPLAY")
                    .output()
                    .unwrap();
                assert!(
                    receipt.status.success(),
                    "axis {index}: {}",
                    String::from_utf8_lossy(&receipt.stderr)
                );
                let text = String::from_utf8(receipt.stdout).unwrap();
                assert!(
                    text.contains(&format!(
                        "room tick: 2 checksum: 0x{:016x}",
                        baseline.frame().checksum()
                    )),
                    "{text}"
                );
                assert!(text.contains("room key: 0 won: 0"));
                assert!(text.contains("room model poses: 3 frames checked;"));
                let matrix: Matrix4 = serde_json::from_str(
                    text.lines()
                        .find_map(|line| line.strip_prefix("room character observed instance: "))
                        .expect("facing runtime must report actual instance"),
                )
                .unwrap();
                assert_matrix(matrix, expected);
                if let Some(old) = previous {
                    assert_eq!(text, old);
                }
                previous = Some(text);
            }
        }
        assert_eq!(
            std::fs::read(output.join("project/room.models.json")).unwrap(),
            original_models
        );
        assert_eq!(
            std::fs::read(output.join("project/room.character.json")).unwrap(),
            original_character
        );
    }
}
