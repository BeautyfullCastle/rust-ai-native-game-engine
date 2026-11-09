//! One existing Room transaction, never a direct scene-file replacement.
use super::*;
use orr_sample::room_layout::{decimal, position, Recipe};

impl Editor {
    pub(crate) fn room_import_source(&mut self) -> Result<String, String> {
        self.room_authoring_ready()?;
        self.call("scene.save", json!({"write":false}))?
            .get("text")
            .and_then(J::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "Room source capture returned no document".into())
    }
    pub(crate) fn import_room_layout(
        &mut self,
        text: &str,
        expected: &(Option<PathBuf>, u64, u64),
        expected_source: &str,
    ) -> Result<(), String> {
        let recipe = Recipe::parse(text.as_bytes())?;
        self.room_authoring_ready()?;
        if expected.0.is_none()
            || (self.path(), self.scene_load_epoch(), self.checksum()) != *expected
        {
            return Err("Room import context changed; Preview the recipe again".into());
        }
        self.room_authoring_transaction("Import Room layout", |editor| {
            let state = editor.call("sim.state", J::Null)?;
            if state.get("checksum").and_then(orr_remote::wire::parse_checksum) != Some(expected.2)
                || (editor.path(), editor.scene_load_epoch()) != (expected.0.clone(), expected.1)
            {
                return Err("Room import context changed; Preview the recipe again".into());
            }
            let source = editor.call("scene.save", json!({"write":false}))?;
            if source.get("text").and_then(J::as_str) != Some(expected_source) {
                return Err("Room document changed; Preview the recipe again".into());
            }
            let actors = editor.room_layout_actors()?;
            let walls = editor.room_wall_rows()?;
            // Check every reference before the first mutation. The application also
            // fences unsaved or mismatched model-binding documents at the UI edge.
            for wall in &walls { editor.admit_saved_room_for_wall_removal(&wall.wall.guid)?; }
            // Remove old collision geometry before moving the player. Every
            // intermediate operation and the complete staged result use the host.
            for wall in walls { editor.call("world.despawn", json!({"entity":wall.target.param()}))?; }
            for ((target, previous), pos) in actors.into_iter().zip(recipe.positions()?) {
                let pos = FPVec3::new(pos.x, previous.y, pos.z);
                editor.call("world.patch", json!({"entity":target.param(),"component":BODY,"path":"pos","value":value_to_json(&Value::Vec3(pos))}))?;
            }
            for (index, wall) in recipe.walls.iter().enumerate() {
                let pos = value_to_json(&Value::Vec3(position(&wall.center)?));
                let half = value_to_json(&Value::Vec3(FPVec3::new(decimal(&wall.half_extents[0])?, FP::ONE, decimal(&wall.half_extents[1])?)));
                let spawned = editor.call("world.spawn", json!({"name":format!("Wall {}", index+1),"components":{
                    ACTOR:{"kind":orr_sample::room_game::WALL,"ordinal":0},
                    BODY:{"pos":pos,"rot":{"kind":"unit","x":0,"y":0,"z":0,"w":1},
                        "vel":[0,0,0],"omega":[0,0,0],"inv_mass":0,"inv_inertia":[0,0,0],
                        "linear_damping":0,"angular_damping":0,"kind":"static"},
                    COLLIDER:{"shape":{"kind":"box","half_extents":half},
                        "restitution":0,"friction":value_to_json(&Value::Fixed(FP::HALF)),"layer":1,"mask":0}
                }}))?;
                if spawned.get("guid").and_then(J::as_str).and_then(Target::parse)
                    .is_none_or(|target| !matches!(target, Target::Guid(_))) {
                    return Err("Imported Room wall has no persistent GUID".into());
                }
            }
            Ok(())
        })?;
        self.select(None);
        self.inspect = None;
        self.dirty.inspect = true;
        Ok(())
    }
}

#[cfg(test)]
impl Editor {
    fn import_test(
        &mut self,
        text: &str,
        expected: &(Option<PathBuf>, u64, u64),
    ) -> Result<(), String> {
        let source = self.room_import_source()?;
        self.import_room_layout(text, expected, &source)
    }
    pub(crate) fn room_import_fixture() -> (tempfile::TempDir, PathBuf, Self) {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("Room import project");
        tests::create_admitted_fixture(&root);
        let prepared = orr_sample::room_project::PreparedProject::open(&root).unwrap();
        let mut editor = Editor::start(&crate::HostSpec::PreparedRoom {
            scene: prepared.path().to_owned(),
            text: prepared.scene().text().to_owned(),
            listen: None,
            debug_hooks: false,
        })
        .unwrap();
        editor.sync();
        (temporary, root, editor)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
    use orr_sample::{
        room_game::{RoomActor, KEY, PLAYER},
        room_project::{PreparedProject, PreparedScene},
    };
    use std::{fs, path::Path};
    pub(super) fn create_admitted_fixture(root: &Path) {
        fs::create_dir(root).unwrap();
        let text = include_str!("../../tests/fixtures/room_initial.scene.yaml").to_owned();
        fs::write(root.join("room.scene.yaml"), &text).unwrap();
        fs::write(
            root.join("room.camera.json"),
            orr_sample::room_camera::Document::readable_default()
                .to_bytes()
                .unwrap(),
        )
        .unwrap();
        fs::write(root.join("orr.project.json"),serde_json::to_vec(&serde_json::json!({"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json","camera":"room.camera.json"}})).unwrap()).unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/imported_scene_demo")
            .canonicalize()
            .unwrap();
        orr_package::Project::open_for_install(
            root,
            orr_package::Runtime::content_only().engine_version,
        )
        .unwrap()
        .install(&[source])
        .unwrap();
        let loaded =
            model_bindings::load_asset(root, "sample-imported-scene", "foreground.glb").unwrap();
        let binding = Binding::from_asset(
            "sample-imported-scene".into(),
            "foreground.glb".into(),
            &loaded,
            LocalTransform::default(),
        )
        .unwrap();
        let scene = PreparedScene::parse(&text).unwrap();
        let bindings = scene
            .frame()
            .entities()
            .filter(|entity| {
                scene
                    .frame()
                    .get::<RoomActor>(*entity)
                    .is_some_and(|actor| [PLAYER, KEY].contains(&actor.kind))
            })
            .map(|entity| {
                (
                    scene.index().guid(entity).unwrap().to_string(),
                    binding.clone(),
                )
            })
            .collect();
        fs::write(
            root.join("room.models.json"),
            serde_json::to_vec(&Document {
                version: 2,
                scene: "room.scene.yaml".into(),
                project: ".".into(),
                bindings,
            })
            .unwrap(),
        )
        .unwrap();
        #[cfg(feature = "room-input-map-play")]
        {
            let mut controls = Vec::new();
            orr_sample::room_controls::RoomControls::default_map()
                .save(&mut controls)
                .unwrap();
            fs::write(root.join("room.controls.json"), controls).unwrap();
            let path = root.join("orr.project.json");
            let mut manifest: J = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            manifest["entry"]["controls"] = json!("room.controls.json");
            fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        }
    }

    const RECIPE: &str = r#"{"schema":1,"player":["-3","0"],"key":["-2","0"],"exit":["3","0"],"walls":[{"center":["0","0"],"half_extents":["0.25","1"]}]}"#;
    fn stamp(editor: &Editor) -> (Option<PathBuf>, u64, u64) {
        (editor.path(), editor.scene_load_epoch(), editor.checksum())
    }
    fn scene(editor: &mut Editor) -> String {
        editor.call("scene.save", json!({"write":false})).unwrap()["text"]
            .as_str()
            .unwrap()
            .into()
    }
    #[test]
    fn import_is_one_undoable_edit_and_save_reopens_without_sidecar_changes() {
        let (_temp, root, mut editor) = Editor::room_import_fixture();
        let before = scene(&mut editor);
        let initial = editor.checksum();
        let history = editor.history().entries.len();
        let sidecars: Vec<_> = [
            "orr.project.json",
            "orr.packages.lock.json",
            "room.camera.json",
            "room.models.json",
        ]
        .into_iter()
        .map(|name| (name, fs::read(root.join(name)).unwrap()))
        .collect();
        #[cfg(feature = "room-input-map-play")]
        let controls = fs::read(root.join("room.controls.json")).unwrap();
        let actors = editor
            .room_layout_actors()
            .unwrap()
            .map(|(target, _)| target.param());
        editor.import_test(RECIPE, &stamp(&editor)).unwrap();
        editor.sync();
        let after = scene(&mut editor);
        assert_ne!(editor.checksum(), initial);
        assert_eq!(editor.history().entries.len(), history + 1);
        assert_eq!(
            actors,
            editor
                .room_layout_actors()
                .unwrap()
                .map(|(target, _)| target.param())
        );
        assert_eq!(editor.read_room_walls().unwrap().len(), 1);
        assert_eq!(
            fs::read_to_string(root.join("room.scene.yaml")).unwrap(),
            before
        );
        assert!(editor.undo());
        editor.sync();
        assert_eq!(scene(&mut editor), before);
        assert!(editor.redo());
        editor.sync();
        assert_eq!(scene(&mut editor), after);
        assert!(editor.save());
        editor.sync();
        let reopened = PreparedProject::open(&root).unwrap();
        assert_eq!(reopened.scene().frame().checksum(), editor.checksum());
        assert_eq!(reopened.scene().text(), after);
        #[cfg(feature = "room-input-map-play")]
        {
            assert_eq!(fs::read(root.join("room.controls.json")).unwrap(), controls);
            assert_eq!(reopened.controls().unwrap().bytes, controls);
        }
        for (name, bytes) in sidecars {
            assert_eq!(fs::read(root.join(name)).unwrap(), bytes, "{name}");
        }
    }
    #[test]
    fn rejected_import_rolls_back_partial_changes_and_stale_context() {
        let (_temp, _root, mut editor) = Editor::room_import_fixture();
        let original = scene(&mut editor);
        let history = editor.history().entries.len();
        for bad in [
            RECIPE.replace("\"0.25\"", "\"0\""),
            RECIPE.replace("\"0.25\"", "\"5\""),
            RECIPE.replace("\"-3\"", "\"99\""),
            RECIPE.replace("\"schema\":1", "\"schema\":2"),
        ] {
            assert!(editor.import_test(&bad, &stamp(&editor)).is_err());
            editor.sync();
            assert_eq!(scene(&mut editor), original);
            assert_eq!(editor.history().entries.len(), history);
            assert!(!editor.history().in_tx);
        }
        for index in 0..3 {
            let mut stale = stamp(&editor);
            match index {
                0 => stale.0 = Some(PathBuf::from("other.scene.yaml")),
                1 => stale.1 += 1,
                _ => stale.2 ^= 1,
            }
            assert!(editor.import_test(RECIPE, &stale).is_err());
        }
        assert_eq!(scene(&mut editor), original);
    }
    #[test]
    fn bound_wall_and_play_mode_refuse_before_mutation() {
        let (_temp, root, mut editor) = Editor::room_import_fixture();
        let guid = editor.read_room_walls().unwrap()[0].guid.clone();
        let path = root.join("room.models.json");
        let mut document: Document = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        document
            .bindings
            .insert(guid, document.bindings.values().next().unwrap().clone());
        fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        let before = scene(&mut editor);
        let history = editor.history().entries.len();
        assert!(editor
            .import_test(RECIPE, &stamp(&editor))
            .unwrap_err()
            .contains("model binding"));
        assert_eq!(scene(&mut editor), before);
        editor.sync();
        assert_eq!(editor.history().entries.len(), history);
        editor.call("sim.start", J::Null).unwrap();
        editor.sync();
        assert!(editor.import_test(RECIPE, &stamp(&editor)).is_err());
    }
    #[test]
    fn imported_saved_puzzle_blocks_shortcut_and_replays_winning_detour() {
        use orr_sample::room_game::{RoomInput, RoomRun, INTERACT};
        let (_temp, root, mut editor) = Editor::room_import_fixture();
        editor.import_test(RECIPE, &stamp(&editor)).unwrap();
        editor.sync();
        assert!(editor.save());
        editor.sync();
        let reopened = PreparedProject::open(&root).unwrap();
        let mut player = Editor::start(&crate::HostSpec::PreparedRoom {
            scene: reopened.path().to_owned(),
            text: reopened.scene().text().to_owned(),
            listen: None,
            debug_hooks: false,
        })
        .unwrap();
        player.sync();
        let mut winning = None;
        for detour in [false, true, true] {
            player.call("sim.start", json!({})).unwrap();
            let route = if detour {
                vec![
                    (1, 0, 0, true),
                    (40, 0, 1, false),
                    (120, 1, 0, false),
                    (40, 0, -1, false),
                    (1, 0, 0, true),
                ]
            } else {
                vec![(1, 0, 0, true), (120, 1, 0, false), (1, 0, 0, true)]
            };
            for (ticks, x, z, interact) in route {
                let input = RoomInput {
                    move_x: x,
                    move_z: z,
                    reserved: 0,
                    buttons: if interact { INTERACT } else { 0 },
                };
                let encoded: String = bytemuck::bytes_of(&input)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect();
                player
                    .call("sim.input", json!({"player":0,"input":encoded}))
                    .unwrap();
                player.call("sim.step", json!({"n":ticks})).unwrap();
            }
            player.sync();
            let frame = player.snapshot().unwrap();
            let run = frame.predicted().singleton::<RoomRun>();
            assert_eq!(run.key_collected, 1);
            assert_eq!(run.won, u32::from(detour));
            let checksum = player.call("sim.checksum", J::Null).unwrap();
            if detour {
                if let Some(previous) = &winning {
                    assert_eq!(&checksum, previous);
                } else {
                    winning = Some(checksum);
                }
            }
            player.call("sim.stop", J::Null).unwrap();
            player.sync();
        }
        assert_eq!(
            fs::read_to_string(root.join("room.scene.yaml")).unwrap(),
            reopened.scene().text()
        );
    }
    #[test]
    fn same_checksum_document_rename_rejects_stale_preview() {
        let (_temp, _root, mut editor) = Editor::room_import_fixture();
        let source = editor.room_import_source().unwrap();
        let expected = stamp(&editor);
        let wall = editor.read_room_walls().unwrap()[0].guid.clone();
        editor
            .call(
                "world.rename",
                json!({"entity":wall,"name":"new wall name"}),
            )
            .unwrap();
        editor.sync();
        assert_eq!(editor.checksum(), expected.2);
        let changed = scene(&mut editor);
        let history = editor.history().entries.len();
        assert!(editor
            .import_room_layout(RECIPE, &expected, &source)
            .unwrap_err()
            .contains("document changed"));
        editor.sync();
        assert_eq!(scene(&mut editor), changed);
        assert_eq!(editor.history().entries.len(), history);
    }
}
