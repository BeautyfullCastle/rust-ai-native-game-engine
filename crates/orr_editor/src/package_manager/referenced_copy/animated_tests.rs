//! Animated upgrades create a new admitted copy; originals remain untouched.
use super::*;
use orr_model::animation::AnimatedModel;
use orr_model_bindings::model_bindings::{AnimationDescriptor, ModelKind, PlaybackMode};
pub(super) fn animated_fixture(character: bool) -> Fixture {
    let f = Fixture::new();
    let base = AnimatedModel::from_bytes(include_bytes!(
        "../../../../../assets/room_character_demo/courier.orrmodel.json"
    ))
    .unwrap();
    for (path, version, rgb) in [
        (&f.v1, "3.0.0", [1.0, 1.0, 1.0]),
        (&f.source, "4.0.0", [0.2, 0.7, 0.4]),
    ] {
        let mut source = base.source().clone();
        source.materials[0].base_color[..3].copy_from_slice(&rgb);
        fs::write(
            path.join(MODEL),
            AnimatedModel::new(source).unwrap().to_bytes().unwrap(),
        )
        .unwrap();
        let manifestpath = path.join("orr.package.json");
        let mut m: Manifest = serde_json::from_slice(&fs::read(&manifestpath).unwrap()).unwrap();
        m.version = version.into();
        m.capabilities.insert("animation".into());
        fs::write(manifestpath, serde_json::to_vec(&m).unwrap()).unwrap();
    }
    Project::open(&f.root, room_runtime())
        .unwrap()
        .install(std::slice::from_ref(&f.v1))
        .unwrap();
    let project = Project::open(&f.root, room_runtime()).unwrap();
    let asset =
        model_bindings::load_asset_from_project(&project, TARGET, MODEL, ModelKind::Animated)
            .unwrap();
    let path = f.root.join("room.models.json");
    let mut d: Document = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let prepared = room_project::PreparedScene::parse(
        &fs::read_to_string(f.root.join("room.scene.yaml")).unwrap(),
    )
    .unwrap();
    let player = prepared
        .frame()
        .entities()
        .find(|&e| {
            prepared
                .frame()
                .get::<RoomActor>(e)
                .is_some_and(|a| a.kind == PLAYER)
        })
        .unwrap();
    let guid = prepared.index().guid(player).unwrap().to_string();
    if character {
        d.bindings.retain(|id, _| id == &guid);
    }
    for b in d.bindings.values_mut() {
        *b = Binding::from_animated_asset(
            TARGET.into(),
            MODEL.into(),
            &asset,
            AnimationDescriptor {
                clip_index: 0,
                playback: PlaybackMode::Loop,
            },
            b.transform,
        )
        .unwrap();
    }
    fs::write(path, serde_json::to_vec(&d).unwrap()).unwrap();
    if character {
        let path = f.root.join("orr.project.json");
        let mut entry: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        entry["entry"]["character"] = serde_json::json!("room.character.json");
        fs::write(path, serde_json::to_vec(&entry).unwrap()).unwrap();
        fs::write(f.root.join("room.character.json"),serde_json::to_vec_pretty(&serde_json::json!({"schema":5,"player":guid,"searching":0,"carrying":1,"escaped":2,"speeds":{"searching":"1/2x","carrying":"2x","escaped":"1x"},"crossfade_ticks":3,"motion":{"searching_idle":0,"carrying_idle":1,"escaped_once":true}})).unwrap()).unwrap();
    }
    open_room_project(&f.root).unwrap();
    f
}
fn reject(f: &Fixture, name: &str) {
    let before = files(f.temp.path());
    let output = f.output(name);
    if let Ok(plan) = plan_copy(&f.root, &f.source) {
        assert!(apply_copy(&plan, &output, true).is_err());
    }
    assert!(!output.exists());
    assert_eq!(files(f.temp.path()), before);
}
#[test]
fn compatible_animated_copy_preserves_all_descriptors_transforms_and_originals() {
    let f = animated_fixture(false);
    let original = files(&f.root);
    let source = files(&f.source);
    let models: Document =
        serde_json::from_slice(&original[Path::new("room.models.json")]).unwrap();
    let plan = plan_copy(&f.root, &f.source).unwrap();
    let output = f.output("animated-copy");
    let report = apply_copy(&plan, &output, true).unwrap();
    assert_eq!(report.bindings.len(), 2);
    let copied: Document =
        serde_json::from_slice(&fs::read(output.join("room.models.json")).unwrap()).unwrap();
    for (id, old) in models.bindings {
        let new = &copied.bindings[&id];
        assert_eq!(new.kind, ModelKind::Animated);
        assert_eq!(new.animation, old.animation);
        assert_eq!(new.transform, old.transform);
        assert_eq!(new.asset, old.asset);
        assert_ne!(new.package_digest, old.package_digest);
        assert_ne!(new.source_hash, old.source_hash);
    }
    assert_eq!(files(&f.root), original);
    assert_eq!(files(&f.source), source);
    open_room_project(&output).unwrap();
    fs::rename(&f.root, f.output("original-hidden")).unwrap();
    fs::rename(&f.source, f.output("source-hidden")).unwrap();
    open_room_project(&output).unwrap();
}
#[test]
fn missing_selected_animation_clip_rejects_new_copy_atomically() {
    let f = animated_fixture(false);
    let project = Project::open(&f.root, room_runtime()).unwrap();
    let asset =
        model_bindings::load_asset_from_project(&project, TARGET, MODEL, ModelKind::Animated)
            .unwrap();
    let path = f.root.join("room.models.json");
    let mut d: Document = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for b in d.bindings.values_mut() {
        *b = Binding::from_animated_asset(
            TARGET.into(),
            MODEL.into(),
            &asset,
            AnimationDescriptor {
                clip_index: 2,
                playback: PlaybackMode::Once,
            },
            b.transform,
        )
        .unwrap();
    }
    fs::write(path, serde_json::to_vec(&d).unwrap()).unwrap();
    open_room_project(&f.root).unwrap();
    let mut s = AnimatedModel::from_bytes(&fs::read(f.source.join(MODEL)).unwrap())
        .unwrap()
        .source()
        .clone();
    s.clips.truncate(2);
    fs::write(
        f.source.join(MODEL),
        AnimatedModel::new(s).unwrap().to_bytes().unwrap(),
    )
    .unwrap();
    reject(&f, "missing-clip-copy");
}
#[test]
fn character_mapping_and_policy_are_preserved_in_reopened_source_free_copy() {
    let f = animated_fixture(true);
    let before = files(&f.root);
    let plan = plan_copy(&f.root, &f.source).unwrap();
    let output = f.output("character-copy");
    apply_copy(&plan, &output, true).unwrap();
    assert_eq!(
        fs::read(output.join("room.character.json")).unwrap(),
        before[Path::new("room.character.json")]
    );
    assert_eq!(files(&f.root), before);
    open_room_project(&output).unwrap();
    fs::rename(&f.root, f.output("original-hidden")).unwrap();
    fs::rename(&f.source, f.output("source-hidden")).unwrap();
    open_room_project(&output).unwrap();
}
#[test]
fn missing_character_state_clip_rejects_even_when_binding_clip_still_exists() {
    let f = animated_fixture(true);
    let mut s = AnimatedModel::from_bytes(&fs::read(f.source.join(MODEL)).unwrap())
        .unwrap()
        .source()
        .clone();
    s.clips.truncate(2);
    fs::write(
        f.source.join(MODEL),
        AnimatedModel::new(s).unwrap().to_bytes().unwrap(),
    )
    .unwrap();
    reject(&f, "missing-character-state");
}
#[test]
fn stale_animated_source_and_foreign_output_preserve_all_inputs() {
    let f = animated_fixture(true);
    let plan = plan_copy(&f.root, &f.source).unwrap();
    let output = f.output("foreign");
    fs::create_dir(&output).unwrap();
    fs::write(output.join("keep"), b"foreign").unwrap();
    let before = files(f.temp.path());
    assert!(apply_copy(&plan, &output, true).is_err());
    assert_eq!(files(f.temp.path()), before);
    let path = f.source.join(MODEL);
    let mut bytes = fs::read(&path).unwrap();
    bytes.push(b' ');
    fs::write(path, bytes).unwrap();
    let before = files(f.temp.path());
    assert!(apply_copy(&plan, &f.output("stale-copy"), true).is_err());
    assert_eq!(files(f.temp.path()), before);
}
