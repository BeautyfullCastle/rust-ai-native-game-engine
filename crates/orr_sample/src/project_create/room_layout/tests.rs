use super::*;
use serde_json::json;
fn recipe() -> serde_json::Value {
    json!({"schema":1,"player":["-3","0"],"key":["-2","0"],"exit":["3","0"],"walls":[{"center":["0","0"],"half_extents":["0.25","1"]}]})
}
#[test]
fn strict_recipe_and_authoritative_admission() {
    let valid = recipe();
    assert!(authored_scene("test", &serde_json::to_vec(&valid).unwrap()).is_ok());
    for text in [
        "",
        " ",
        "+1",
        ".5",
        "1.",
        "1e0",
        "NaN",
        "1 0",
        "0x1",
        "9999999999999999999999999999999999999999999999999",
    ] {
        assert!(decimal(text).is_err(), "{text}");
    }
    for (path, value) in [
        ("schema", json!(2)),
        (
            "walls",
            json!([{"center":["-3","0"],"half_extents":["1","1"]}]),
        ),
        (
            "walls",
            json!([{"center":["0","0"],"half_extents":["0","1"]}]),
        ),
        ("player", json!(["99", "0"])),
        ("key", json!([2, 0])),
        ("extra", json!(1)),
    ] {
        let mut bad = valid.clone();
        bad[path] = value;
        assert!(
            authored_scene("test", &serde_json::to_vec(&bad).unwrap()).is_err(),
            "{bad}"
        );
    }
    let mut nested = valid.clone();
    nested["walls"][0]["unknown"] = json!(0);
    assert!(authored_scene("test", &serde_json::to_vec(&nested).unwrap()).is_err());
    for count in [0, 64] {
        let mut bounded = valid.clone();
        bounded["walls"] = json!(vec![valid["walls"][0].clone(); count]);
        assert!(authored_scene("test", &serde_json::to_vec(&bounded).unwrap()).is_ok());
    }
    let encoded = serde_json::to_string(&valid).unwrap();
    for bad in [
        encoded.replacen("\"schema\":1", "\"schema\":1,\"schema\":1", 1),
        encoded.replacen(
            "\"center\":[\"0\",\"0\"]",
            "\"center\":[\"0\",\"0\"],\"center\":[\"0\",\"0\"]",
            1,
        ),
    ] {
        assert_ne!(bad, encoded);
        assert!(authored_scene("test", bad.as_bytes()).is_err());
    }
    let mut many = valid.clone();
    many["walls"] = json!(vec![valid["walls"][0].clone(); 65]);
    assert!(authored_scene("test", &serde_json::to_vec(&many).unwrap()).is_err());
}
#[test]
fn deterministic_layout_and_report_match_saved_scene() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let input = root.join("layout.json");
    fs::write(&input, serde_json::to_vec(&recipe()).unwrap()).unwrap();
    let options = |name: &str| CreateOptions {
        output: root.join(name),
        template: ROOM_TEMPLATE.into(),
        seed: "same".into(),
    };
    let first = create_room_layout(&options("one"), &input).unwrap();
    let second = create_room_layout(&options("two"), &input).unwrap();
    assert_eq!(first.entity_guids, second.entity_guids);
    assert_eq!(first.initial_checksum, second.initial_checksum);
    let saved = fs::read_to_string(first.output.join("room.scene.yaml")).unwrap();
    let scene = Scene::parse(&saved, &crate::room_project::types()).unwrap();
    assert_eq!(
        first.entity_guids,
        scene.entities.keys().cloned().collect::<Vec<_>>()
    );
    assert_eq!(first.entity_guids.len(), 4);
    assert_eq!(
        first.initial_checksum,
        crate::room_project::PreparedScene::parse(&saved)
            .unwrap()
            .frame()
            .checksum()
    );
    for path in [
        "room.scene.yaml",
        "room.models.json",
        "room.camera.json",
        "orr.project.json",
        "orr.packages.lock.json",
        "README.md",
    ] {
        assert_eq!(
            fs::read(first.output.join(path)).unwrap(),
            fs::read(second.output.join(path)).unwrap(),
            "{path}"
        );
    }
    let base = create(&options("default")).unwrap();
    for path in [
        "room.models.json",
        "room.camera.json",
        "orr.project.json",
        "orr.packages.lock.json",
    ] {
        assert_eq!(
            fs::read(first.output.join(path)).unwrap(),
            fs::read(base.output.join(path)).unwrap(),
            "{path}"
        );
    }
    let mut other = options("other-seed");
    other.seed = "different".into();
    let other = create_room_layout(&other, &input).unwrap();
    assert_eq!(first.initial_checksum, other.initial_checksum);
    assert_ne!(first.entity_guids, other.entity_guids);
    let before = fs::read(first.output.join("room.scene.yaml")).unwrap();
    assert!(create_room_layout(&options("one"), &input).is_err());
    assert_eq!(
        before,
        fs::read(first.output.join("room.scene.yaml")).unwrap()
    );
}
#[test]
fn invalid_input_never_publishes_and_special_files_fail() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let input = root.join("layout.json");
    let options = CreateOptions {
        output: root.join("new"),
        template: ROOM_TEMPLATE.into(),
        seed: "test".into(),
    };
    for bytes in [b"{}".to_vec(), vec![b' '; 65537]] {
        fs::write(&input, bytes).unwrap();
        assert!(create_room_layout(&options, &input).is_err());
        assert!(!options.output.exists());
    }
    let link = root.join("link");
    std::os::unix::fs::symlink(&input, &link).unwrap();
    assert!(create_room_layout(&options, &link).is_err());
    assert!(create_room_layout(&options, &root).is_err());
    let fifo = root.join("fifo");
    rustix::fs::mknodat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::RUSR,
        0,
    )
    .unwrap();
    assert!(create_room_layout(&options, &fifo).is_err());
    assert!(!options.output.exists());
}
