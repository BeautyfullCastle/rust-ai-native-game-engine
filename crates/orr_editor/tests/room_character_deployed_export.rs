//! Source-hidden Room character export acceptance, exercised as a real Cargo
//! integration target rather than editor source-module tests.
//! `orr_sample::project_export` is only exposed on Linux x86_64.
//!
//! Compile and list the integration target. This does not execute the ignored
//! CPU export acceptances:
//! ```sh
//! cargo test --release --locked --offline -p orr_editor --features windows-export,room-character,project-create --test room_character_deployed_export -- --list
//! ```
//!
//! Build the official Room CPU runtime required by those acceptance tests:
//! ```sh
//! cargo build --release --locked --offline -p orr_sample --features room-project,room-character --bin room_escape
//! ```
//!
//! Run both ignored source-hidden export acceptances. From the repository root,
//! `ROOM_RUNTIME` is normally `target/release/room_escape`; set it to the actual
//! built binary path when using a custom Cargo target directory:
//! ```sh
//! ROOM_RUNTIME=target/release/room_escape
//! ORR_ESCAPE_ONCE_RUNTIME="$ROOM_RUNTIME" ORR_MOTION_RUNTIME="$ROOM_RUNTIME" \
//!   cargo test --release --locked --offline -p orr_editor --features windows-export,room-character,project-create --test room_character_deployed_export -- --ignored --test-threads=1
//! ```

#![cfg(all(
    feature = "windows-export",
    feature = "room-character",
    feature = "project-create",
    target_os = "linux",
    target_arch = "x86_64"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use orr_editor::{room_character_panel::Panel, Editor, HostSpec, Target};
use orr_sample::{
    project_create::{create, CreateOptions, ROOM_CHARACTER_TEMPLATE},
    room_project::PreparedProject,
};
use std::{fs, path::PathBuf, time::Duration};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    editor: Editor,
    models: orr_editor::model_panel::ModelPanel,
    panel: Panel,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp
            .path()
            .canonicalize()
            .unwrap()
            .join("character project");
        create(&CreateOptions {
            output: root.clone(),
            template: ROOM_CHARACTER_TEMPLATE.into(),
            seed: "character-pair-save".into(),
        })
        .unwrap();
        let mut prepared = PreparedProject::open_with_capabilities(
            &root,
            orr_sample::room_project::RoomProjectCapabilities {
                lighting: false,
                character: true,
            },
        )
        .unwrap();
        let character = prepared.take_character().unwrap();
        let (_, path, scene, prepared_models) = prepared.into_parts();
        let mut editor = Editor::start(&HostSpec::PreparedRoom {
            scene: path,
            text: scene.text().into(),
            listen: None,
            debug_hooks: false,
        })
        .unwrap();
        editor
            .install_room_character(character.document.clone())
            .unwrap();
        let mut models = orr_editor::model_panel::ModelPanel::default();
        models.install_room(prepared_models).unwrap();
        for attempt in 0..1600 {
            editor.sync();
            if editor.yard_rows_coherent() && models.scene_matches(&editor) {
                break;
            }
            assert!(
                attempt < 1599,
                "room character panel did not become coherent"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let panel = Panel::new_for_editor(character, &editor).unwrap();
        Self {
            _temp: temp,
            root,
            editor,
            models,
            panel,
        }
    }

    fn position_room_actors_for_export(&mut self) {
        use orr_fp::{FPVec3, FP};
        use orr_sample::room_game::{RoomActor, EXIT, KEY, PLAYER};

        for kind in [PLAYER, KEY, EXIT] {
            let guid = self
                .editor
                .rows()
                .iter()
                .find(|row| {
                    self.editor
                        .snapshot()
                        .unwrap()
                        .predicted()
                        .get::<RoomActor>(row.entity)
                        .is_some_and(|actor| actor.kind == kind)
                })
                .unwrap()
                .guid
                .clone()
                .unwrap();
            self.editor.select(Some(Target::Guid(guid)));
            assert!(self.editor.set_yard_transform(
                FPVec3::new(FP::ZERO, FP::HALF, FP::ZERO),
                [FP::ZERO, FP::ZERO, FP::ZERO, FP::ONE]
            ));
            for attempt in 0..1600 {
                self.editor.sync();
                if self.editor.yard_rows_coherent() {
                    break;
                }
                assert!(
                    attempt < 1599,
                    "room actor transform did not become coherent"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        assert!(self.editor.save());
        for attempt in 0..1600 {
            self.editor.sync();
            if self.editor.yard_rows_coherent() {
                break;
            }
            assert!(
                attempt < 1599,
                "saved room actor transforms did not become coherent"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[path = "fixtures/room_motion.rs"]
mod motion_fixture;

#[cfg(feature = "windows-export")]
#[test]
#[ignore = "explicit CPU source-hidden export requires ORR_ESCAPE_ONCE_RUNTIME"]
fn once_policy_source_hidden_export_reaches_real_escape_and_holds_endpoint() {
    use orr_bridge::FrameView;
    use orr_fp::{FPVec3, FP};
    use orr_sample::{
        room_character::PlaybackCursor,
        room_game::{RoomActor, RoomInput, RoomRun, EXIT, INTERACT, KEY, PLAYER},
        room_view::{self, PresentationTime},
    };
    use orr_sim::{PlayerSlot, TickInputs};
    use sha2::{Digest, Sha256};

    let mut f = Fixture::new();
    let mut once = f.panel.document().clone();
    once.schema = 4;
    once.speeds = Some(Default::default());
    once.crossfade_ticks = Some(0);
    f.panel
        .apply_document(once.clone(), &mut f.editor, &mut f.models)
        .unwrap();
    f.panel.save_all(&f.editor, &mut f.models).unwrap();
    for kind in [PLAYER, KEY, EXIT] {
        let guid = f
            .editor
            .rows()
            .iter()
            .find(|row| {
                f.editor
                    .snapshot()
                    .unwrap()
                    .predicted()
                    .get::<RoomActor>(row.entity)
                    .is_some_and(|actor| actor.kind == kind)
            })
            .unwrap()
            .guid
            .clone()
            .unwrap();
        f.editor.select(Some(Target::Guid(guid)));
        assert!(f.editor.set_yard_transform(
            FPVec3::new(FP::ZERO, FP::HALF, FP::ZERO),
            [FP::ZERO, FP::ZERO, FP::ZERO, FP::ONE]
        ));
        for attempt in 0..1600 {
            f.editor.sync();
            if f.editor.yard_rows_coherent() {
                break;
            }
            assert!(attempt < 1599);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    assert!(f.editor.save());
    for attempt in 0..1600 {
        f.editor.sync();
        if f.editor.yard_rows_coherent() {
            break;
        }
        assert!(attempt < 1599);
        std::thread::sleep(Duration::from_millis(5));
    }
    let built = PathBuf::from(
        std::env::var_os("ORR_ESCAPE_ONCE_RUNTIME").expect("explicit built runtime required"),
    )
    .canonicalize()
    .unwrap();
    let runtime = f._temp.path().join("trusted once runtime");
    fs::copy(built, &runtime).unwrap();
    let hash: String = Sha256::digest(fs::read(&runtime).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let bundle = f._temp.path().join("deployed once game");
    orr_sample::project_export::export_room(&orr_sample::project_export::ExportOptions {
        project: f.root.clone(),
        runtime: runtime.clone(),
        runtime_sha256: hash,
        output: bundle.clone(),
        trusted_runtime: true,
        source_revision: None,
    })
    .unwrap();
    assert_eq!(
        fs::read(bundle.join("project/room.character.json")).unwrap(),
        once.to_bytes().unwrap()
    );
    fs::rename(&f.root, f.root.with_extension("hidden")).unwrap();
    fs::rename(&runtime, runtime.with_extension("hidden")).unwrap();
    let prepared = PreparedProject::open_with_capabilities(
        bundle.join("project"),
        orr_sample::room_project::RoomProjectCapabilities {
            lighting: false,
            character: true,
        },
    )
    .unwrap();
    assert_eq!(prepared.character().unwrap().document, once);
    let mut sim = prepared.scene().simulation().unwrap();
    let mut cursor = PlaybackCursor::default();
    let index = prepared.scene().index();
    let models = prepared.models();
    let binding = &models.document.bindings[&once.player];
    let model = models.assets[&(binding.package.clone(), binding.asset.clone())]
        .animated_model()
        .unwrap();
    room_view::observe_character(
        &mut cursor,
        FrameView::of(sim.frame()),
        index,
        models,
        PresentationTime::default(),
    )
    .unwrap();
    for tick in 1..=100 {
        let mut inputs = TickInputs::new(sim.tick() + 1, 1);
        inputs.set_input(
            PlayerSlot(0),
            RoomInput {
                buttons: if tick == 1 || tick == 3 { INTERACT } else { 0 },
                ..Default::default()
            },
        );
        sim.step(&inputs);
        let before = sim.frame().to_bytes();
        room_view::observe_character(
            &mut cursor,
            FrameView::of(sim.frame()),
            index,
            models,
            PresentationTime::default(),
        )
        .unwrap();
        assert_eq!(sim.frame().to_bytes(), before);
        assert_eq!(
            room_view::check_model_poses(FrameView::of(sim.frame()), index, models, Some(&cursor))
                .unwrap()
                .animated_models,
            1
        );
        let pose = cursor
            .pose(
                &once,
                model,
                FrameView::of(sim.frame()),
                orr_sample::room_character::PlaybackContext {
                    entity: index
                        .entity(&orr_reflect::Guid::parse(&once.player).unwrap())
                        .unwrap(),
                    tick_rate: 60,
                    rest: false,
                    revision: 0,
                },
            )
            .unwrap();
        if tick == 3 {
            assert_eq!(sim.frame().singleton::<RoomRun>().won, 1);
            assert_eq!(pose, &model.sample_clip(once.escaped, 0.0).unwrap());
        }
        if tick == 33 {
            assert_eq!(pose, &model.sample_clip(once.escaped, 0.5).unwrap());
        }
        if tick >= 63 {
            assert_eq!(pose, &model.sample_clip(once.escaped, 1.0).unwrap());
        }
    }
    let output = std::process::Command::new(bundle.join("run-room-escape"))
        .current_dir(f._temp.path())
        .args([
            "--headless",
            "--ticks",
            "8",
            "--hold",
            "interact",
            "--check-model-poses",
        ])
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .contains("room model poses: 9 frames checked;"));
}

mod motion_tests {
    use super::*;
    use orr_model_bindings::model_bindings::{AnimationDescriptor, ModelKind, PlaybackMode};
    use orr_sample::room_character::{Document, Motion};

    fn install(f: &mut Fixture) {
        motion_fixture::install(&f.root, &f._temp.path().join("idle source"));
        f.editor.select(Some(Target::Guid(
            orr_reflect::Guid::parse(&f.panel.document().player).unwrap(),
        )));
        f.models.package = motion_fixture::PACKAGE.into();
        f.models.asset = motion_fixture::ASSET.into();
        f.models.kind = ModelKind::Animated;
        f.models.animation = AnimationDescriptor {
            clip_index: f.panel.document().searching,
            playback: PlaybackMode::Loop,
        };
        f.models.assign(&f.editor).unwrap();
    }

    fn document(f: &Fixture) -> Document {
        Document {
            schema: 5,
            speeds: Some(Default::default()),
            crossfade_ticks: Some(0),
            motion: Some(Motion {
                searching_idle: motion_fixture::IDLE,
                carrying_idle: motion_fixture::IDLE,
                escaped_once: true,
                pickup: None,
                facing: None,
            }),
            ..f.panel.document().clone()
        }
    }

    #[cfg(feature = "windows-export")]
    #[test]
    #[ignore = "explicit CPU deployed simulation requires ORR_MOTION_RUNTIME"]
    fn motion_policy_deployed_game_switches_idle_move_carry_and_victory_without_source() {
        use orr_bridge::FrameView;
        use orr_sample::{
            room_character::{PlaybackContext, PlaybackCursor},
            room_game::{RoomInput, RoomRun, INTERACT},
            room_view::{self, PresentationTime},
        };
        use orr_sim::{PlayerSlot, TickInputs};
        use sha2::{Digest, Sha256};

        let mut f = Fixture::new();
        install(&mut f);
        let doc = document(&f);
        f.panel
            .apply_document(doc.clone(), &mut f.editor, &mut f.models)
            .unwrap();
        f.panel.save_all(&f.editor, &mut f.models).unwrap();
        f.position_room_actors_for_export();
        let built = PathBuf::from(
            std::env::var_os("ORR_MOTION_RUNTIME").expect("explicit runtime required"),
        )
        .canonicalize()
        .unwrap();
        let runtime = f._temp.path().join("trusted motion runtime");
        fs::copy(built, &runtime).unwrap();
        let hash: String = Sha256::digest(fs::read(&runtime).unwrap())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let bundle = f._temp.path().join("deployed motion game");
        orr_sample::project_export::export_room(&orr_sample::project_export::ExportOptions {
            project: f.root.clone(),
            runtime: runtime.clone(),
            runtime_sha256: hash,
            output: bundle.clone(),
            trusted_runtime: true,
            source_revision: None,
        })
        .unwrap();
        assert_eq!(
            fs::read(bundle.join("project/room.character.json")).unwrap(),
            doc.to_bytes().unwrap()
        );
        fs::rename(&f.root, f.root.with_extension("hidden")).unwrap();
        fs::rename(&runtime, runtime.with_extension("hidden")).unwrap();
        fs::rename(
            f._temp.path().join("idle source"),
            f._temp.path().join("idle source hidden"),
        )
        .unwrap();
        let prepared = PreparedProject::open_with_capabilities(
            bundle.join("project"),
            orr_sample::room_project::RoomProjectCapabilities {
                lighting: false,
                character: true,
            },
        )
        .unwrap();
        assert_eq!(prepared.character().unwrap().document, doc);
        let models = prepared.models();
        let index = prepared.scene().index();
        let binding = &models.document.bindings[&doc.player];
        let model = models.assets[&(binding.package.clone(), binding.asset.clone())]
            .animated_model()
            .unwrap();
        let entity = index
            .entity(&orr_reflect::Guid::parse(&doc.player).unwrap())
            .unwrap();
        let context = PlaybackContext {
            entity,
            tick_rate: 60,
            rest: false,
            revision: 0,
        };
        let mut sim = prepared.scene().simulation().unwrap();
        let mut cursor = PlaybackCursor::default();
        room_view::observe_character(
            &mut cursor,
            FrameView::of(sim.frame()),
            index,
            models,
            PresentationTime::default(),
        )
        .unwrap();
        assert_eq!(
            cursor
                .pose(&doc, model, FrameView::of(sim.frame()), context)
                .unwrap(),
            &model.sample_clip(motion_fixture::IDLE, 0.0).unwrap()
        );
        for tick in 1..=100 {
            let mut inputs = TickInputs::new(sim.tick() + 1, 1);
            inputs.set_input(
                PlayerSlot(0),
                RoomInput {
                    move_x: if tick == 2 || tick == 5 { 1 } else { 0 },
                    buttons: if tick == 4 || tick == 7 { INTERACT } else { 0 },
                    ..Default::default()
                },
            );
            sim.step(&inputs);
            let before = sim.frame().to_bytes();
            room_view::observe_character(
                &mut cursor,
                FrameView::of(sim.frame()),
                index,
                models,
                PresentationTime::default(),
            )
            .unwrap();
            assert_eq!(sim.frame().to_bytes(), before);
            assert_eq!(
                room_view::check_model_poses(
                    FrameView::of(sim.frame()),
                    index,
                    models,
                    Some(&cursor)
                )
                .unwrap()
                .animated_models,
                1
            );
            let pose = cursor
                .pose(&doc, model, FrameView::of(sim.frame()), context)
                .unwrap();
            if tick < 7 {
                let clip = match tick {
                    2 => doc.searching,
                    5 => doc.carrying,
                    _ => motion_fixture::IDLE,
                };
                assert_eq!(pose, &model.sample_clip(clip, tick as f32 / 60.0).unwrap());
            }
            if tick == 4 {
                assert_eq!(sim.frame().singleton::<RoomRun>().key_collected, 1);
            }
            if tick == 7 {
                assert_eq!(sim.frame().singleton::<RoomRun>().won, 1);
                assert_eq!(pose, &model.sample_clip(doc.escaped, 0.0).unwrap());
            }
            if tick >= 67 {
                assert_eq!(pose, &model.sample_clip(doc.escaped, 1.0).unwrap());
            }
        }
        let output = std::process::Command::new(bundle.join("run-room-escape"))
            .current_dir(f._temp.path())
            .args([
                "--headless",
                "--ticks",
                "8",
                "--hold",
                "right",
                "--check-model-poses",
            ])
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8(output.stdout)
            .unwrap()
            .contains("room model poses: 9 frames checked;"));
    }
}
