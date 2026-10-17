#![cfg(feature = "room-project")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use orr_model_bindings::model_bindings::Document;
#[cfg(feature = "room-animated-models")]
use orr_model_bindings::model_bindings::{
    self, AnimationDescriptor, Binding, LocalTransform, ModelKind, PlaybackMode,
};
use orr_package::{Project, Runtime};
use orr_reflect::Scene;
#[cfg(feature = "room-animated-models")]
use orr_sample::{
    room_game::{RoomActor, PLAYER},
    room_project::PreparedScene,
};
use orr_sample::{
    room_game::{RoomConfig, RoomEscapeV1, TICK_RATE},
    room_project::{self, SEED},
};
use orr_sim::Simulation;
use std::{collections::BTreeMap, fs, path::PathBuf};

#[cfg(feature = "room-animated-models")]
const ANIMATED_PACKAGE: &str = "sample-animation";
#[cfg(feature = "room-animated-models")]
const ANIMATED_ASSET: &str = "animated.glb";

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    #[cfg(feature = "room-animated-models")]
    scene: String,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("animated room");
        fs::create_dir(&root).unwrap();
        let simulation = Simulation::<RoomEscapeV1>::new(RoomConfig, TICK_RATE, SEED);
        let scene = Scene::unbake(&room_project::types(), simulation.frame(), None)
            .unwrap()
            .to_yaml();
        fs::write(root.join("room.scene.yaml"), &scene).unwrap();
        fs::write(
            root.join("orr.project.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema": 2,
                "engine": "*",
                "entry": {
                    "game": "room-escape-v1",
                    "scene": "room.scene.yaml",
                    "models": "room.models.json"
                }
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            root.join("room.models.json"),
            serde_json::to_vec_pretty(&Document {
                version: 2,
                scene: "room.scene.yaml".into(),
                project: ".".into(),
                bindings: BTreeMap::new(),
            })
            .unwrap(),
        )
        .unwrap();
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/animation_demo")
            .canonicalize()
            .unwrap();
        Project::open_for_install(&root, Runtime::content_only().engine_version)
            .unwrap()
            .install(&[source])
            .unwrap();
        Self {
            _temp: temp,
            root,
            #[cfg(feature = "room-animated-models")]
            scene,
        }
    }

    fn bytes(&self) -> Vec<(PathBuf, Vec<u8>)> {
        [
            "orr.project.json",
            "room.scene.yaml",
            "room.models.json",
            "orr.packages.lock.json",
        ]
        .into_iter()
        .map(|relative| {
            let path = self.root.join(relative);
            (path.clone(), fs::read(path).unwrap())
        })
        .collect()
    }

    #[cfg(feature = "room-animated-models")]
    fn write_animated_binding(&self, clip_index: u32) -> Binding {
        let loaded = model_bindings::load_asset_for_kind(
            &self.root,
            ANIMATED_PACKAGE,
            ANIMATED_ASSET,
            ModelKind::Animated,
        )
        .unwrap();
        let binding = Binding::from_animated_asset(
            ANIMATED_PACKAGE.into(),
            ANIMATED_ASSET.into(),
            &loaded,
            AnimationDescriptor {
                clip_index,
                playback: PlaybackMode::Loop,
            },
            LocalTransform::default(),
        )
        .unwrap_or_else(|error| {
            assert!(clip_index < 32, "unexpected fixture error: {error}");
            Binding {
                kind: ModelKind::Animated,
                package: loaded.package().into(),
                asset: loaded.asset().into(),
                package_digest: loaded.package_digest().into(),
                source_hash: loaded.source_hash().into(),
                transform: LocalTransform::default(),
                animation: Some(AnimationDescriptor {
                    clip_index,
                    playback: PlaybackMode::Loop,
                }),
            }
        });
        let prepared = PreparedScene::parse(&self.scene).unwrap();
        let player = prepared
            .frame()
            .entities()
            .find(|entity| {
                prepared
                    .frame()
                    .get::<RoomActor>(*entity)
                    .is_some_and(|actor| actor.kind == PLAYER)
            })
            .unwrap();
        let guid = prepared.index().guid(player).unwrap();
        let document = Document {
            version: 2,
            scene: "room.scene.yaml".into(),
            project: ".".into(),
            bindings: BTreeMap::from([(guid.to_string(), binding.clone())]),
        };
        fs::write(
            self.root.join("room.models.json"),
            serde_json::to_vec_pretty(&document).unwrap(),
        )
        .unwrap();
        binding
    }
}

#[cfg(not(feature = "room-animated-models"))]
#[test]
fn room_runtime_rejects_animation_capability_even_when_model_feature_is_unified() {
    let fixture = Fixture::new();
    assert!(!room_project::compiled_runtime()
        .capabilities
        .contains("animation"));
    let original = fixture.bytes();
    let error = match room_project::PreparedProject::open(&fixture.root) {
        Ok(_) => panic!("Room without explicit animation support admitted animation package"),
        Err(error) => error,
    };
    assert!(error.to_ascii_lowercase().contains("animation"), "{error}");
    assert_eq!(original, fixture.bytes());
}

#[cfg(feature = "room-animated-models")]
#[test]
fn room_animated_project_admits_and_rechecks_selected_clip_without_writes() {
    let fixture = Fixture::new();
    assert!(room_project::compiled_runtime()
        .capabilities
        .contains("animation"));
    let binding = fixture.write_animated_binding(0);
    let original = fixture.bytes();
    let prepared = room_project::PreparedProject::open(&fixture.root).unwrap();
    assert_eq!(prepared.models().assets.len(), 1);
    assert_eq!(
        prepared.models().assets.values().next().unwrap().kind(),
        ModelKind::Animated
    );
    assert_eq!(
        prepared
            .models()
            .document
            .bindings
            .values()
            .next()
            .unwrap()
            .animation
            .unwrap()
            .clip_index,
        0
    );
    assert_eq!(original, fixture.bytes());

    let asset = prepared.models().assets.values().next().unwrap();
    let instance_work = asset.animated_model().unwrap().source().nodes.len()
        + asset
            .animated_model()
            .unwrap()
            .source()
            .primitives
            .iter()
            .map(|primitive| primitive.vertices.len())
            .sum::<usize>()
        + asset
            .animated_model()
            .unwrap()
            .source()
            .skins
            .iter()
            .map(|skin| skin.joints.len())
            .sum::<usize>()
        + asset.animated_model().unwrap().source().clips[0]
            .channels
            .len();
    assert!(instance_work > 0);
    let copies = room_project::MAX_ANIMATION_WORK_PER_FRAME / instance_work + 1;
    assert!(
        room_project::validate_animation_workload(std::iter::repeat_n((&binding, asset), copies))
            .is_err()
    );

    fixture.write_animated_binding(31);
    let invalid_bytes = fixture.bytes();
    assert!(room_project::PreparedProject::open(&fixture.root).is_err());
    assert_eq!(invalid_bytes, fixture.bytes());
}
