#![cfg(feature = "room-project")]
#![allow(clippy::disallowed_types)]

#[path = "../examples/room_layout/authoring.rs"]
mod authoring;

use authoring::Options;
use orr_bridge::{FrameView, PlayerSlot};
use orr_edit::{Op, Origin};
use orr_fp::{FPVec3, FP};
use orr_reflect::{Guid, Scene, Value};
use orr_remote::{room_escape, Auth, Caps, ErpClient, ServerConfig};
use orr_sample::{
    room_game::{
        RoomActor, RoomConfig, RoomEscapeV1, RoomInput, RoomRun, EXIT, INTERACT, KEY, PLAYER, WALL,
    },
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use orr_sim::{Simulation, TickInputs};
use serde_json::{json, Value as J};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const PACKAGE: &str = "sample-imported-scene";
const PACKAGE_DIGEST: &str = "9c7708914c9e5fbe61dad5cc717dff6df0d811d0e14f3aa9a967896785522cf0";
const LOCK: &str = r#"{"schema":1,"direct":{"sample-imported-scene":"1.0.0"},"packages":{"sample-imported-scene":{"manifest":{"schema":1,"name":"sample-imported-scene","version":"1.0.0","engine":"^0.0.1","capabilities":["models"],"dependencies":{},"files":["LICENSE.txt","background.glb","foreground.glb","generate.py"]},"digest":"9c7708914c9e5fbe61dad5cc717dff6df0d811d0e14f3aa9a967896785522cf0","files":{"LICENSE.txt":"398039150c615dec7d81ca90231e4d94fa142fe2ffbec784d09ab3407c49f37c","background.glb":"f93967c23bec3fbbe3bf64445e364665e08fbebecd211429f2b16ca9e615340d","foreground.glb":"465f0e8acb9b567b575f05004113580302d57f585b18d14cee402841925652fd","generate.py":"b7bb27c9bcf2ea582a1babb9a7c9ea2475437f936c2a19d4b23f0cc31e9cc2e8"}}}}"#;
const ASSETS: &[(&str, &[u8])] = &[
    (
        "orr.package.json",
        include_bytes!("../../../assets/imported_scene_demo/orr.package.json"),
    ),
    (
        "LICENSE.txt",
        include_bytes!("../../../assets/imported_scene_demo/LICENSE.txt"),
    ),
    (
        "background.glb",
        include_bytes!("../../../assets/imported_scene_demo/background.glb"),
    ),
    (
        "foreground.glb",
        include_bytes!("../../../assets/imported_scene_demo/foreground.glb"),
    ),
    (
        "generate.py",
        include_bytes!("../../../assets/imported_scene_demo/generate.py"),
    ),
];

struct Fixture {
    base: PathBuf,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let base = std::env::temp_dir().join(format!(
            "orr-room-layout-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&base).unwrap();
        let root = base.join("admitted room");
        fs::create_dir(&root).unwrap();
        let object = root.join(".orr/packages/objects").join(PACKAGE_DIGEST);
        fs::create_dir_all(&object).unwrap();
        for (name, bytes) in ASSETS {
            fs::write(object.join(name), bytes).unwrap();
        }
        fs::write(root.join("orr.packages.lock.json"), LOCK).unwrap();

        let simulation = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
        let scene = Scene::unbake(&room_project::types(), simulation.frame(), None).unwrap();
        fs::write(root.join("room.scene.yaml"), scene.to_yaml()).unwrap();
        fs::write(
            root.join("orr.project.json"),
            serde_json::to_vec_pretty(&json!({
                "schema": 2,
                "engine": "*",
                "entry": {
                    "game": "room-escape-v1",
                    "scene": "room.scene.yaml",
                    "models": "room.models.json",
                    "camera": "room.camera.json"
                }
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            root.join("room.camera.json"),
            orr_sample::room_camera::Document::readable_default()
                .to_bytes()
                .unwrap(),
        )
        .unwrap();
        let mut bindings = serde_json::Map::new();
        for (guid, entity) in &scene.entities {
            let kind = entity
                .components
                .iter()
                .find(|(name, _)| name == room_project::ACTOR)
                .unwrap()
                .1
                .field("kind")
                .unwrap();
            if matches!(kind, Value::Int(1 | 2)) {
                bindings.insert(
                    guid.to_string(),
                    json!({
                        "kind": "static",
                        "package": PACKAGE,
                        "asset": "foreground.glb",
                        "package_digest": PACKAGE_DIGEST,
                        "source_hash": "465f0e8acb9b567b575f05004113580302d57f585b18d14cee402841925652fd",
                        "transform": {"translation":[0,0,0],"rotation":[0,0,0,1],"scale":[1,1,1]}
                    }),
                );
            }
        }
        fs::write(
            root.join("room.models.json"),
            serde_json::to_vec_pretty(&json!({
                "version": 2,
                "scene": "room.scene.yaml",
                "project": ".",
                "bindings": bindings
            }))
            .unwrap(),
        )
        .unwrap();
        PreparedProject::open(&root).expect("fixture must pass production Room admission");
        Self { base, root }
    }

    fn args(&self) -> Vec<OsString> {
        [
            OsString::from("--project"),
            self.root.clone().into_os_string(),
            "--player".into(),
            "-2".into(),
            "-1".into(),
            "--key".into(),
            "-1".into(),
            "-1".into(),
            "--exit".into(),
            "1".into(),
            "-1".into(),
            "--output".into(),
            self.base.join("room-layout.ops.json").into_os_string(),
        ]
        .into()
    }

    fn options(&self) -> Options {
        Options::parse(self.args()).unwrap()
    }

    fn text(&self) -> String {
        fs::read_to_string(self.root.join("room.scene.yaml")).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!(
                "FAILED Room layout fixture retained: {}",
                self.base.display()
            );
        } else {
            let _ = fs::remove_dir_all(&self.base);
        }
    }
}

fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(!kind.is_symlink());
            if kind.is_dir() {
                visit(root, &entry.path(), files);
            } else {
                assert!(kind.is_file());
                files.insert(
                    entry.path().strip_prefix(root).unwrap().to_owned(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

fn generate(options: &Options) -> Result<Vec<u8>, String> {
    authoring::generate(options).map(|(bytes, _)| bytes)
}

fn fixed(value: &J) -> FP {
    let text = value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_number().map(ToString::to_string))
        .unwrap();
    orr_reflect::decimal::parse_fp(&text).unwrap()
}

fn patches(bytes: &[u8]) -> Vec<(Guid, FPVec3)> {
    let operations: Vec<J> = serde_json::from_slice(bytes).expect("ops JSON");
    assert_eq!(
        operations.len(),
        3,
        "one whole-position patch for each role"
    );
    operations
        .into_iter()
        .map(|operation| {
            assert_eq!(operation["op"], "patch");
            let component = operation["component"].as_str().unwrap();
            assert_eq!(component, "orr_physics3d::Body");
            let path = operation["path"].as_str().unwrap().to_owned();
            assert_eq!(path, "pos");
            let guid = Guid::parse(operation["entity"].as_str().unwrap()).unwrap();
            let position = operation["value"].as_array().unwrap();
            assert_eq!(position.len(), 3);
            (
                guid,
                FPVec3::new(
                    fixed(&position[0]),
                    fixed(&position[1]),
                    fixed(&position[2]),
                ),
            )
        })
        .collect()
}

fn role_guids(doc: &orr_edit::EditorDoc) -> BTreeMap<u32, Guid> {
    doc.frame()
        .entities()
        .filter_map(|entity| {
            let role = doc.frame().get::<RoomActor>(entity)?.kind;
            [PLAYER, KEY, EXIT]
                .contains(&role)
                .then(|| (role, doc.index().guid(entity).unwrap().clone()))
        })
        .collect()
}

fn json_ops_to_batch(bytes: &[u8]) -> Vec<Op> {
    patches(bytes)
        .into_iter()
        .map(|(guid, position)| Op::SetField {
            guid,
            component: "orr_physics3d::Body".into(),
            path: "pos".into(),
            value: Value::Vec3(position),
        })
        .collect()
}

fn role_position(scene: &Scene, role: u32) -> (Guid, FPVec3) {
    for (guid, entity) in &scene.entities {
        let actor = entity
            .components
            .iter()
            .find(|(name, _)| name == room_project::ACTOR)
            .unwrap()
            .1
            .field("kind")
            .unwrap();
        if actor == &Value::Int(i128::from(role)) {
            let position = entity
                .components
                .iter()
                .find(|(name, _)| name == "orr_physics3d::Body")
                .unwrap()
                .1
                .field("pos")
                .unwrap();
            let Value::Vec3(position) = position else {
                panic!("Body.pos")
            };
            return (guid.clone(), *position);
        }
    }
    panic!("missing role {role}")
}

#[test]
fn room_layout_cli_rejects_duplicate_missing_and_invalid_coordinates() {
    let fixture = Fixture::new();
    let args = fixture.args();
    for count in 0..args.len() {
        assert!(Options::parse(args[..count].to_vec()).is_err());
    }
    for token in [
        "NaN", "inf", "-inf", "1e0", " 1", "1 ", "01", "+1", ".5", "5.",
    ] {
        let mut bad = args.clone();
        bad[3] = token.into();
        assert!(Options::parse(bad).is_err(), "{token}");
    }
    let mut duplicate = args.clone();
    duplicate.extend(["--player".into(), "2".into(), "2".into()]);
    assert!(Options::parse(duplicate).is_err());

    let mut exact = args;
    exact[3] = "0.0000152587890625".into();
    let options = Options::parse(exact).unwrap();
    assert_eq!(options.player[0], FP(1));
    let exact_output = patches(&generate(&options).unwrap());
    let player = role_guids(&room_escape::document(&fixture.text()).unwrap())[&PLAYER].clone();
    assert_eq!(
        exact_output
            .iter()
            .find(|(guid, _)| *guid == player)
            .unwrap()
            .1
            .x,
        FP(1)
    );

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let mut non_utf8 = fixture.args();
        non_utf8[3] = OsString::from_vec(vec![0xff]);
        assert!(Options::parse(non_utf8).is_err());
    }
}

#[test]
fn room_layout_generates_three_exact_role_position_patches_without_project_mutation() {
    let fixture = Fixture::new();
    let before = tree(&fixture.root);
    let bytes = generate(&fixture.options()).unwrap();
    let actual = patches(&bytes);
    assert_eq!(actual.len(), 3);
    let doc = room_escape::document(&fixture.text()).unwrap();
    let expected_roles = role_guids(&doc);
    for (role, position_x, position_z) in [
        (PLAYER, FP::from_int(-2), FP::from_int(-1)),
        (KEY, FP::from_int(-1), FP::from_int(-1)),
        (EXIT, FP::ONE, FP::from_int(-1)),
    ] {
        let guid = &expected_roles[&role];
        assert_eq!(
            actual
                .iter()
                .filter(|(candidate, _)| candidate == guid)
                .count(),
            1
        );
        let (_, position) = actual
            .iter()
            .find(|(candidate, _)| candidate == guid)
            .unwrap();
        assert_eq!((position.x, position.z), (position_x, position_z));
        assert_eq!(position.y, FP::HALF);
    }
    assert_eq!(tree(&fixture.root), before);
    assert!(!fixture.options().output.exists());
}

#[test]
fn room_layout_applies_as_one_undoable_batch_and_round_trips_scene_and_sidecars() {
    let fixture = Fixture::new();
    let original = fixture.text();
    let before_project = tree(&fixture.root);
    let mut doc = room_escape::document(&original).unwrap();
    let before_scene = doc.scene().clone();
    let batch = json_ops_to_batch(&generate(&fixture.options()).unwrap());
    assert_eq!(batch.len(), 3);
    doc.apply_batch("place Room actors", batch, Origin::User)
        .unwrap();
    assert_eq!(doc.history().len(), 1);
    assert_eq!(doc.history()[0].op_count, 3);

    for role in [PLAYER, KEY, EXIT] {
        let (guid, old_pos) = role_position(&before_scene, role);
        let (same_guid, new_pos) = role_position(doc.scene(), role);
        assert_eq!(same_guid, guid);
        assert_eq!(new_pos.y, old_pos.y);
        let old_entity = &before_scene.entities[&guid];
        let new_entity = &doc.scene().entities[&guid];
        assert_eq!(old_entity.name, new_entity.name);
        for (name, old_value) in &old_entity.components {
            let new_value = new_entity
                .components
                .iter()
                .find(|(candidate, _)| candidate == name)
                .unwrap()
                .1
                .clone();
            if name == "orr_physics3d::Body" {
                let (Value::Struct(old_fields), Value::Struct(new_fields)) =
                    (old_value, &new_value)
                else {
                    panic!("Body must remain a reflected struct");
                };
                assert_eq!(old_fields.len(), new_fields.len());
                for (field, old_field) in old_fields {
                    if field != "pos" {
                        let new_field = new_fields
                            .iter()
                            .find(|(candidate, _)| candidate == field)
                            .map(|(_, value)| value);
                        assert_eq!(Some(old_field), new_field, "Body.{field}");
                    }
                }
            } else {
                assert_eq!(old_value, &new_value, "{name}");
            }
        }
    }

    let adopted = doc.to_yaml();
    doc.undo().unwrap();
    assert_eq!(doc.to_yaml(), original);
    doc.redo().unwrap();
    assert_eq!(doc.to_yaml(), adopted);
    fs::write(fixture.root.join("room.scene.yaml"), &adopted).unwrap();
    let reopened = PreparedProject::open(&fixture.root).unwrap();
    assert_eq!(reopened.scene().text(), adopted);
    assert_eq!(
        room_escape::document(&adopted).unwrap().frame().to_bytes(),
        reopened.scene().frame().to_bytes()
    );
    let mut expected = before_project;
    expected.remove(Path::new("room.scene.yaml"));
    let mut actual = tree(&fixture.root);
    actual.remove(Path::new("room.scene.yaml"));
    assert_eq!(
        actual, expected,
        "model/package/camera sidecars remain byte-identical"
    );
}

#[test]
fn room_layout_erp_accept_undo_redo_save_and_reopen() {
    let fixture = Fixture::new();
    let original = fixture.text();
    let path = fixture.root.join("room.scene.yaml");
    let mut config = ServerConfig::new(Auth::DevNoAuth);
    config.listen = false;
    let host = room_escape::spawn_host(original.clone(), Some(path), config).unwrap();
    let mut client = ErpClient::with_transport(Box::new(
        host.connector()
            .connect("room-layout-acceptance", Caps::ALL)
            .unwrap(),
    ));
    let ops: J = serde_json::from_slice(&generate(&fixture.options()).unwrap()).unwrap();
    let id = client
        .call("proposal.begin", json!({"label":"author Room layout"}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    client
        .call("proposal.apply", json!({"id":id,"ops":ops}))
        .unwrap();
    assert_eq!(
        client.call("scene.save", J::Null).unwrap()["text"],
        original
    );
    let report = client
        .call(
            "proposal.verify",
            json!({
                "id": id,
                "inputs": {"kind":"idle","ticks":4},
                "sample_every": 1,
                "checks": ["entities.delta == 0"]
            }),
        )
        .unwrap();
    assert_eq!(report["passed"], true, "{report}");
    client
        .call(
            "proposal.accept_verified",
            json!({"id":id,"verified_state":report["verified_state"]}),
        )
        .unwrap();
    let adopted = client.call("scene.save", J::Null).unwrap()["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(adopted, original);
    let history = client.call("history.list", J::Null).unwrap();
    assert_eq!(history["entries"].as_array().unwrap().len(), 1);
    assert_eq!(history["entries"][0]["op_count"], 3);
    client.call("history.undo", J::Null).unwrap();
    assert_eq!(
        client.call("scene.save", J::Null).unwrap()["text"],
        original
    );
    client.call("history.redo", J::Null).unwrap();
    assert_eq!(client.call("scene.save", J::Null).unwrap()["text"], adopted);
    let saved = client.call("scene.save", json!({"write":true})).unwrap();
    assert_eq!(saved["text"], adopted);
    assert_eq!(saved["dirty"], false);
    let reopened = PreparedProject::open(&fixture.root).unwrap();
    assert_eq!(reopened.scene().text(), adopted);
    assert_eq!(
        room_escape::document(&adopted).unwrap().frame().to_bytes(),
        reopened.scene().frame().to_bytes()
    );
}

#[test]
fn room_layout_rejects_out_of_bounds_and_player_wall_overlap_without_output_or_mutation() {
    for (player, output_name) in [
        ([FP::from_int(17), FP::ZERO], "bounds.ops.json"),
        ([FP(4 * 65536 + 3 * 65536 / 4), FP::ZERO], "wall.ops.json"),
    ] {
        let fixture = Fixture::new();
        let before = tree(&fixture.root);
        let mut options = fixture.options();
        options.player = player;
        options.output = fixture.base.join(output_name);
        assert!(generate(&options).is_err());
        assert!(authoring::run(options.clone()).is_err());
        assert_eq!(tree(&fixture.root), before);
        assert!(!options.output.exists());
    }
}

#[test]
fn room_layout_rejects_key_pose_that_invalidates_its_admitted_model_placement() {
    let fixture = Fixture::new();
    let original = fixture.text();
    let key = role_guids(&room_escape::document(&original).unwrap())[&KEY]
        .as_str()
        .to_owned();
    let models_path = fixture.root.join("room.models.json");
    let mut models: J = serde_json::from_slice(&fs::read(&models_path).unwrap()).unwrap();
    models["bindings"][key.as_str()]["transform"]["translation"][0] = json!(-1_000_000.0);
    fs::write(&models_path, serde_json::to_vec_pretty(&models).unwrap()).unwrap();
    let project = PreparedProject::open(&fixture.root)
        .expect("original key pose remains within admitted model bounds");
    let before = tree(&fixture.root);

    let mut candidate_doc = room_escape::document(&original).unwrap();
    let (key_guid, old_position) = role_position(candidate_doc.scene(), KEY);
    candidate_doc
        .apply_batch(
            "move key to boundary",
            vec![Op::SetField {
                guid: key_guid,
                component: "orr_physics3d::Body".into(),
                path: "pos".into(),
                value: Value::Vec3(FPVec3::new(FP::from_int(-16), old_position.y, FP::ZERO)),
            }],
            Origin::User,
        )
        .unwrap();
    let candidate = PreparedScene::parse(&candidate_doc.to_yaml())
        .expect("the candidate remains valid under Room game admission");
    assert!(orr_sample::room_view::admit_presentation(
        FrameView::of(candidate.frame()),
        candidate.index(),
        project.models(),
    )
    .is_err());

    let mut options = fixture.options();
    options.key = [FP::from_int(-16), FP::ZERO];
    assert!(generate(&options).is_err());
    assert!(!options.output.exists());
    assert_eq!(tree(&fixture.root), before);
}

#[test]
fn room_layout_rejects_output_inside_project_and_never_overwrites_existing_output() {
    let fixture = Fixture::new();
    let mut inside = fixture.options();
    inside.output = fixture.root.join("generated.ops.json");
    assert!(authoring::run(inside.clone()).is_err());
    assert!(!inside.output.exists());

    let options = fixture.options();
    let expected = generate(&options).unwrap();
    authoring::run(options.clone()).unwrap();
    assert_eq!(fs::read(&options.output).unwrap(), expected);
    assert!(authoring::run(options.clone()).is_err());
    assert_eq!(fs::read(&options.output).unwrap(), expected);
}

fn step(simulation: &mut Simulation<RoomEscapeV1>, input: RoomInput) {
    let mut inputs = TickInputs::new(simulation.tick(), 1);
    inputs.set_input(PlayerSlot(0), input);
    simulation.step(&inputs);
}

fn reflected_position(frame: &orr_ecs::Frame, role: u32) -> FPVec3 {
    let entity = frame
        .entities()
        .find(|&entity| {
            frame
                .get::<RoomActor>(entity)
                .is_some_and(|actor| actor.kind == role)
        })
        .unwrap();
    let body = room_project::types()
        .read_component(frame, entity, "orr_physics3d::Body")
        .unwrap();
    let Value::Struct(fields) = body else {
        panic!("Body")
    };
    let Value::Vec3(position) = fields
        .iter()
        .find(|(name, _)| name == "pos")
        .map(|(_, value)| value)
        .unwrap()
    else {
        panic!("Body.pos")
    };
    *position
}

#[test]
fn room_layout_repositioned_key_can_be_collected_before_winning_at_exit() {
    let fixture = Fixture::new();
    let mut options = fixture.options();
    options.player = [FP::from_int(-3), FP::from_int(-2)];
    options.key = [FP::from_int(-1), FP::from_int(-2)];
    options.exit = [FP::from_int(2), FP::from_int(-2)];
    let mut doc = room_escape::document(&fixture.text()).unwrap();
    doc.apply_batch(
        "place Room actors",
        json_ops_to_batch(&generate(&options).unwrap()),
        Origin::User,
    )
    .unwrap();
    let prepared = PreparedScene::parse(&doc.to_yaml()).unwrap();
    let mut simulation = prepared.simulation().unwrap();
    step(
        &mut simulation,
        RoomInput {
            buttons: INTERACT,
            ..RoomInput::default()
        },
    );
    assert_eq!(simulation.frame().singleton::<RoomRun>().key_collected, 0);
    assert_eq!(simulation.frame().singleton::<RoomRun>().won, 0);
    step(&mut simulation, RoomInput::default());
    for _ in 0..30 {
        step(
            &mut simulation,
            RoomInput {
                move_x: 1,
                ..RoomInput::default()
            },
        );
    }
    step(
        &mut simulation,
        RoomInput {
            buttons: INTERACT,
            ..RoomInput::default()
        },
    );
    assert_eq!(simulation.frame().singleton::<RoomRun>().key_collected, 1);
    assert_eq!(simulation.frame().singleton::<RoomRun>().won, 0);
    step(&mut simulation, RoomInput::default());
    for _ in 0..60 {
        step(
            &mut simulation,
            RoomInput {
                move_x: 1,
                ..RoomInput::default()
            },
        );
    }
    assert_eq!(simulation.frame().singleton::<RoomRun>().won, 0);
    step(
        &mut simulation,
        RoomInput {
            buttons: INTERACT,
            ..RoomInput::default()
        },
    );
    assert_eq!(simulation.frame().singleton::<RoomRun>().won, 1);
    let player = reflected_position(simulation.frame(), PLAYER);
    assert_eq!(player.y, FP::HALF);
    assert_ne!(
        player,
        FPVec3::new(FP::from_int(-3), FP::HALF, FP::from_int(-2))
    );
    assert!((player.x - options.exit[0]).abs() <= FP::ONE);
    assert_eq!(player.z, options.exit[1]);
    let frame = simulation.frame();
    let wall_count = frame
        .entities()
        .filter(|&entity| {
            frame
                .get::<RoomActor>(entity)
                .is_some_and(|actor| actor.kind == WALL)
        })
        .count();
    assert_eq!(wall_count, 4);
}
