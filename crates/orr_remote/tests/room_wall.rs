//! Closed Room wall authoring acceptance. Ordinary tests exercise the real
//! admitted document and ERP server through its in-process transport. The one
//! ignored test must run in the mandatory gate with a freshly retained CLI.
#![cfg(feature = "room-project")]
#![allow(clippy::disallowed_types)]

#[path = "../examples/room_wall/authoring.rs"]
mod authoring;

use authoring::Options;
use orr_bridge::PlayerSlot;
use orr_edit::{Op, Origin};
use orr_fp::{FPVec3, FP};
use orr_reflect::{Guid, Scene, Value};
use orr_remote::{room_escape, Auth, Caps, ErpClient, LocalHost, ServerConfig};
use orr_sample::{
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, RoomInput, RoomRun, INTERACT, PLAYER, WALL},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use orr_sim::{Simulation, TickInputs};
use serde_json::{json, Value as J};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

const GUID: &str = "e_abcdef0123456789";
const PACKAGE: &str = "sample-imported-scene";
const PACKAGE_DIGEST: &str = "9c7708914c9e5fbe61dad5cc717dff6df0d811d0e14f3aa9a967896785522cf0";
// Fixture identity is independently reproducible with the package manager's
// canonical ["orr-package-v1", manifest, sorted file hashes] encoding. Opening
// PreparedProject verifies these hashes, including every embedded source file.
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
            "orr-room-wall-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&base).unwrap();
        let base = base.canonicalize().unwrap();
        let root = base.join("admitted room");
        fs::create_dir(&root).unwrap();
        let object = root.join(".orr/packages/objects").join(PACKAGE_DIGEST);
        fs::create_dir_all(&object).unwrap();
        for (name, bytes) in ASSETS {
            fs::write(object.join(name), bytes).unwrap();
        }
        fs::write(root.join("orr.packages.lock.json"), LOCK).unwrap();
        let sim = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
        let scene = Scene::unbake(&room_project::types(), sim.frame(), None).unwrap();
        fs::write(root.join("room.scene.yaml"), scene.to_yaml()).unwrap();
        fs::write(root.join("orr.project.json"), serde_json::to_vec_pretty(&json!({"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json","camera":"room.camera.json"}})).unwrap()).unwrap();
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
                bindings.insert(guid.to_string(), json!({"kind":"static","package":PACKAGE,"asset":"foreground.glb","package_digest":PACKAGE_DIGEST,"source_hash":"465f0e8acb9b567b575f05004113580302d57f585b18d14cee402841925652fd","transform":{"translation":[0,0,0],"rotation":[0,0,0,1],"scale":[1,1,1]}}));
            }
        }
        fs::write(
            root.join("room.models.json"),
            serde_json::to_vec_pretty(
                &json!({"version":2,"scene":"room.scene.yaml","project":".","bindings":bindings}),
            )
            .unwrap(),
        )
        .unwrap();
        PreparedProject::open(&root)
            .expect("pinned fixture must pass production package/model/scene/camera admission");
        Self { base, root }
    }
    fn args(&self) -> Vec<OsString> {
        [
            OsString::from("--project"),
            self.root.clone().into_os_string(),
            "--guid".into(),
            GUID.into(),
            "--name".into(),
            "authored wall".into(),
            "--center".into(),
            "0.625".into(),
            "0".into(),
            "--half-extents".into(),
            "0.125".into(),
            "0.75".into(),
            "--output".into(),
            self.base.join("wall.ops.json").into_os_string(),
        ]
        .into()
    }
    fn options(&self) -> Options {
        Options::parse(self.args()).unwrap()
    }
    fn text(&self) -> String {
        fs::read_to_string(self.root.join("room.scene.yaml")).unwrap()
    }
    fn save(&self, doc: &orr_edit::EditorDoc) {
        fs::write(self.root.join("room.scene.yaml"), doc.to_yaml()).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("FAILED Room wall fixture retained: {}", self.base.display());
        } else {
            let _ = fs::remove_dir_all(&self.base);
        }
    }
}
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(!kind.is_symlink());
            if kind.is_dir() {
                visit(root, &entry.path(), out);
            } else {
                assert!(kind.is_file());
                out.insert(
                    entry.path().strip_prefix(root).unwrap().to_owned(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}
fn preserve_except_scene(before: &BTreeMap<PathBuf, Vec<u8>>, root: &Path) {
    let mut after = tree(root);
    let mut before = before.clone();
    before.remove(Path::new("room.scene.yaml"));
    after.remove(Path::new("room.scene.yaml"));
    assert_eq!(before, after, "model/camera/manifest/package bytes changed");
}
fn generate(options: &Options) -> Result<Vec<u8>, String> {
    authoring::generate(options).map(|(bytes, _)| bytes)
}
fn wall_count(doc: &orr_edit::EditorDoc) -> usize {
    doc.frame()
        .entities()
        .filter(|&entity| {
            doc.frame()
                .get::<RoomActor>(entity)
                .is_some_and(|actor| actor.kind == WALL)
        })
        .count()
}
fn add(doc: &mut orr_edit::EditorDoc, options: &Options) {
    doc.apply(authoring::wall_op(options).unwrap(), Origin::User)
        .unwrap();
}
fn fill(doc: &mut orr_edit::EditorDoc, fixture: &Fixture, count: usize) {
    while wall_count(doc) < count {
        let mut o = fixture.options();
        o.guid = Guid::from_u32(0x1000 + wall_count(doc) as u32);
        o.center = [FP::from_int(16); 2];
        o.half_extents = [FP(6554); 2];
        add(doc, &o);
    }
}
fn player_guid(doc: &orr_edit::EditorDoc) -> Guid {
    let entity = doc
        .frame()
        .entities()
        .find(|&entity| {
            doc.frame()
                .get::<RoomActor>(entity)
                .is_some_and(|actor| actor.kind == PLAYER)
        })
        .unwrap();
    doc.index().guid(entity).unwrap().clone()
}
fn position(sim: &Simulation<RoomEscapeV1>) -> FPVec3 {
    let scene = Scene::unbake(&room_project::types(), sim.frame(), None).unwrap();
    let entity = scene
        .entities
        .values()
        .find(|e| {
            e.components.iter().any(|(n, v)| {
                n == room_project::ACTOR && v.field("kind") == Some(&Value::Int(i128::from(PLAYER)))
            })
        })
        .unwrap();
    let body = &entity
        .components
        .iter()
        .find(|(n, _)| n == "orr_physics3d::Body")
        .unwrap()
        .1;
    let Value::Vec3(p) = body.field("pos").unwrap() else {
        panic!("Body.pos");
    };
    *p
}
fn step(sim: &mut Simulation<RoomEscapeV1>, input: RoomInput) {
    let mut inputs = TickInputs::new(sim.tick(), 1);
    inputs.set_input(PlayerSlot(0), input);
    sim.step(&inputs);
}
fn walk(x: i8, z: i8) -> RoomInput {
    RoomInput {
        move_x: x,
        move_z: z,
        ..RoomInput::default()
    }
}
fn host(text: &str, path: PathBuf) -> (LocalHost, ErpClient) {
    let mut cfg = ServerConfig::new(Auth::DevNoAuth);
    cfg.listen = false;
    let host = room_escape::spawn_host(text.into(), Some(path), cfg).unwrap();
    let client = ErpClient::with_transport(Box::new(
        host.connector()
            .connect("wall-acceptance", Caps::ALL)
            .unwrap(),
    ));
    (host, client)
}
fn scene_text(c: &mut ErpClient) -> String {
    c.call("scene.save", J::Null).unwrap()["text"]
        .as_str()
        .unwrap()
        .into()
}
fn stage(c: &mut ErpClient, ops: &J) -> String {
    let id = c
        .call("proposal.begin", json!({"label":"add authored Room wall"}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    c.call("proposal.apply", json!({"id":id,"ops":ops}))
        .unwrap();
    id
}
fn verify(c: &mut ErpClient, id: &str) -> J {
    let report = c.call("proposal.verify", json!({"id":id,"inputs":{"kind":"idle","ticks":4},"sample_every":1,"checks":["entities.delta == 1"]})).unwrap();
    assert_eq!(report["passed"], true, "{report}");
    assert_eq!(report["first_divergence"], 0);
    report
}

#[test]
fn room_wall_cli_cardinality_and_token_bounds() {
    let f = Fixture::new();
    let good = f.args();
    for count in 0..good.len() {
        assert!(Options::parse(good[..count].to_vec()).is_err());
    }
    for extra in ["--help", "--unknown", "extra", "--guid"] {
        let mut a = good.clone();
        a.push(extra.into());
        assert!(Options::parse(a).is_err());
    }
    for index in [0, 2, 4, 6, 9, 12] {
        let mut a = good.clone();
        a[index] = "--guid".into();
        if index != 2 {
            assert!(Options::parse(a).is_err());
        }
    }
    for (index, bad) in [
        (3, "e_0000000".into()),
        (3, "e_ABCDEF00".into()),
        (3, format!("e_{}", "a".repeat(33))),
        (5, String::new()),
        (5, "a".repeat(129)),
        (7, "1".repeat(49)),
        (7, "NaN".into()),
        (7, "inf".into()),
        (7, "1e0".into()),
        (7, " 1".into()),
        (7, "1 ".into()),
        (7, "1_000".into()),
        (7, "+1.25".into()),
        (7, ".5".into()),
        (7, "5.".into()),
        (7, "01".into()),
    ] {
        let mut a = good.clone();
        a[index] = bad.into();
        assert!(Options::parse(a).is_err());
    }
    for index in [1, 13] {
        let mut a = f.args();
        a[index] = "a".repeat(4097).into();
        assert!(Options::parse(a).is_err());
    }
    for index in [1, 13] {
        let mut a = f.args();
        a[index] = OsString::new();
        assert!(Options::parse(a).is_err());
    }
    let mut a = good;
    a[3] = format!("e_{}", "a".repeat(32)).into();
    a[5] = "a".repeat(128).into();
    assert!(Options::parse(a).is_ok());
    assert!(!f.options().output.exists());
}

#[test]
fn room_wall_fixed_point_envelope_boundaries() {
    let f = Fixture::new();
    let before = tree(&f.root);
    for token in [
        "16",
        "-16",
        "0.0000152587890625",
        "0.00000762939453125",
        "-0.0000152587890625",
        "1.25",
    ] {
        let mut a = f.args();
        a[7] = token.into();
        a[8] = "16".into();
        a[10] = "0.1".into();
        let o = Options::parse(a).unwrap();
        assert_eq!(o.center[0], orr_reflect::decimal::parse_fp(token).unwrap());
        assert!(generate(&o).is_ok());
    }
    for token in [
        "16.0000152587890625",
        "-16.0000152587890625",
        "140737488355327",
        "-140737488355328",
    ] {
        let mut a = f.args();
        a[7] = token.into();
        a[8] = "16".into();
        let result = Options::parse(a).and_then(|o| generate(&o));
        assert!(result.is_err(), "{token}");
    }
    for (extent, good) in [
        ("0.1", true),
        ("0.100006103515625", true),
        ("8", true),
        ("0.0999908447265625", false),
        ("8.0000152587890625", false),
        ("0", false),
        ("-1", false),
    ] {
        let mut a = f.args();
        a[7] = "16".into();
        a[8] = "16".into();
        a[10] = extent.into();
        a[11] = extent.into();
        assert_eq!(
            Options::parse(a).and_then(|o| generate(&o)).is_ok(),
            good,
            "{extent}"
        );
    }
    assert_eq!(tree(&f.root), before);
    assert!(!f.options().output.exists());
}

#[test]
fn room_wall_absent_guid_and_canonical_spawn() {
    let empty = Fixture::new();
    let mut no_walls = room_escape::document(&empty.text()).unwrap();
    let walls = no_walls
        .frame()
        .entities()
        .filter(|&entity| {
            no_walls
                .frame()
                .get::<RoomActor>(entity)
                .is_some_and(|actor| actor.kind == WALL)
        })
        .map(|entity| no_walls.index().guid(entity).unwrap().clone())
        .collect::<Vec<_>>();
    for guid in walls {
        no_walls
            .apply(Op::DespawnEntity { guid }, Origin::User)
            .unwrap();
    }
    assert_eq!(wall_count(&no_walls), 0);
    empty.save(&no_walls);
    PreparedProject::open(&empty.root).unwrap();
    generate(&empty.options()).unwrap();
    add(&mut no_walls, &empty.options());
    assert_eq!(wall_count(&no_walls), 1);
    let f = Fixture::new();
    let o = f.options();
    let bytes = generate(&o).unwrap();
    let j: J = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(j.as_array().unwrap().len(), 1);
    assert_eq!(j[0]["op"], "spawn");
    assert_eq!(j[0]["guid"], GUID);
    let keys = j[0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        keys,
        ["op", "guid", "name", "components"].into_iter().collect()
    );
    let mut doc = room_escape::document(&f.text()).unwrap();
    let before = doc.scene().clone();
    add(&mut doc, &o);
    assert_eq!(doc.scene().entities.len(), before.entities.len() + 1);
    assert_eq!(doc.scene().singletons, before.singletons);
    for (guid, entity) in before.entities {
        assert_eq!(doc.scene().entities[&guid], entity);
    }
    let wall = &doc.scene().entities[&o.guid];
    assert_eq!(wall.name.as_deref(), Some("authored wall"));
    assert_eq!(wall.components.len(), 3);
    let value = |name: &str| &wall.components.iter().find(|(n, _)| n == name).unwrap().1;
    assert_eq!(
        value(room_project::ACTOR).field("kind"),
        Some(&Value::Int(4))
    );
    assert_eq!(
        value(room_project::ACTOR).field("ordinal"),
        Some(&Value::Int(0))
    );
    assert_eq!(
        value("orr_physics3d::Body").field("pos"),
        Some(&Value::Vec3(FPVec3::new(FP(40960), FP::HALF, FP::ZERO)))
    );
    assert_eq!(
        value("orr_physics3d::Collider").field("layer"),
        Some(&Value::Int(1))
    );
    assert_eq!(
        value("orr_physics3d::Collider").field("mask"),
        Some(&Value::Int(0))
    );
    PreparedScene::parse(&doc.to_yaml()).unwrap();
    f.save(&doc);
    assert!(generate(&o).is_err());
    let mut overlap = o;
    overlap.guid = Guid::from_u32(0xdeadbeef);
    overlap.center = [FP::ZERO; 2];
    assert!(generate(&overlap).is_err());
}

#[test]
fn room_wall_inclusive_64_wall_budget() {
    let f = Fixture::new();
    let mut doc = room_escape::document(&f.text()).unwrap();
    fill(&mut doc, &f, 63);
    f.save(&doc);
    let o = f.options();
    generate(&o).unwrap();
    add(&mut doc, &o);
    assert_eq!(wall_count(&doc), 64);
    f.save(&doc);
    PreparedProject::open(&f.root).unwrap();
    let before = tree(&f.root);
    let mut next = o;
    next.guid = Guid::from_u32(0xfeedbeef);
    assert!(generate(&next).is_err());
    assert_eq!(tree(&f.root), before);
}

#[test]
fn room_wall_publication_no_clobber_and_failure_preservation() {
    for broken in [
        "room.scene.yaml",
        "orr.project.json",
        "room.models.json",
        "orr.packages.lock.json",
    ] {
        let malformed = Fixture::new();
        fs::write(malformed.root.join(broken), b"malformed: [").unwrap();
        let before = tree(&malformed.root);
        assert!(authoring::run(malformed.options()).is_err(), "{broken}");
        assert_eq!(tree(&malformed.root), before);
        assert!(!malformed.options().output.exists());
    }
    let f = Fixture::new();
    let o = f.options();
    let before = tree(&f.root);
    let expected = generate(&o).unwrap();
    authoring::run(o.clone()).unwrap();
    assert_eq!(fs::read(&o.output).unwrap(), expected);
    assert!(authoring::run(o.clone()).is_err());
    assert_eq!(fs::read(&o.output).unwrap(), expected);
    let mut bad = o.clone();
    bad.output = f.base.join("bad.ops.json");
    bad.center = [FP::ZERO; 2];
    assert!(authoring::run(bad.clone()).is_err());
    assert!(!bad.output.exists());
    bad = o.clone();
    bad.output = f.base.join("directory");
    fs::create_dir(&bad.output).unwrap();
    assert!(authoring::run(bad).is_err());
    bad = o;
    bad.output = f.root.join("new.ops.json");
    assert!(authoring::run(bad).is_err());
    assert_eq!(tree(&f.root), before);
    assert!(
        !fs::read_dir(&f.base).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".room-wall")),
        "failed publication leaked temporary sibling"
    );
}

#[test]
fn room_wall_platform_native_paths_and_unsafe_ancestry() {
    let f = Fixture::new();
    let before = tree(&f.root);
    for leaf in [
        "../escape.json",
        "a/../escape.json",
        "./escape.json",
        "a//escape.json",
        "C:escape.json",
        "CON",
        "nul.json",
        "trailing. ",
    ] {
        let mut o = f.options();
        o.output = f.base.join(leaf);
        assert!(authoring::run(o).is_err(), "{leaf}");
    }
    let mut o = f.options();
    o.project = f.base.join("absent");
    assert!(authoring::run(o).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::{ffi::OsStringExt, fs::symlink};
        let mut args = f.args();
        args[5] = OsString::from_vec(vec![255]);
        assert!(Options::parse(args).is_err());
        let mut args = f.args();
        args[2] = OsString::from_vec(vec![255]);
        assert!(Options::parse(args).is_err());
        let native = f.base.join(OsString::from_vec(b"wall-\xff.json".to_vec()));
        let mut o = f.options();
        o.output = native.clone();
        authoring::run(o).unwrap();
        assert!(native.is_file());
        let mut native_project = Fixture::new();
        let native_before = tree(&native_project.root);
        let native_root = native_project
            .base
            .join(OsString::from_vec(b"project-\xff".to_vec()));
        fs::rename(&native_project.root, &native_root).unwrap();
        native_project.root = native_root;
        let native_options = Options::parse(native_project.args()).unwrap();
        PreparedProject::open(&native_options.project).unwrap();
        authoring::run(native_options.clone()).unwrap();
        assert_eq!(
            fs::read(&native_options.output).unwrap(),
            generate(&native_options).unwrap()
        );
        assert_eq!(tree(&native_project.root), native_before);
        let victim = f.base.join("victim");
        fs::write(&victim, b"unchanged").unwrap();
        for (name, target) in [
            ("symlink", victim.clone()),
            ("dangling", f.base.join("missing")),
            ("parent-link", f.base.clone()),
        ] {
            let link = f.base.join(name);
            symlink(target, &link).unwrap();
            let mut o = f.options();
            o.output = if name == "parent-link" {
                link.join("through.json")
            } else {
                link
            };
            assert!(authoring::run(o).is_err());
        }
        let hard = f.base.join("hardlink");
        fs::hard_link(&victim, &hard).unwrap();
        let mut o = f.options();
        o.output = hard;
        assert!(authoring::run(o).is_err());
        assert_eq!(fs::read(victim).unwrap(), b"unchanged");
    }
    #[cfg(windows)]
    for path in [
        r"\\server\share\wall.json",
        r"\\?\C:\room\wall.json",
        r"\\.\pipe\wall",
        r"C:wall.json",
        r"\wall.json",
        "//server/share/wall.json",
    ] {
        let mut options = f.options();
        options.output = PathBuf::from(path);
        assert!(authoring::run(options).is_err(), "{path}");
    }
    assert_eq!(tree(&f.root), before);
}

#[test]
fn room_wall_erp_verified_adoption_undo_redo_save_reopen() {
    let f = Fixture::new();
    let before = tree(&f.root);
    let text = f.text();
    let ops = generate(&f.options()).unwrap();
    let ops_json: J = serde_json::from_slice(&ops).unwrap();
    let (_host, mut c) = host(&text, f.root.join("room.scene.yaml"));
    let id = stage(&mut c, &ops_json);
    assert_eq!(scene_text(&mut c), text);
    let report = verify(&mut c, &id);
    assert_eq!(scene_text(&mut c), text);
    c.call(
        "proposal.accept_verified",
        json!({"id":id,"verified_state":report["verified_state"]}),
    )
    .unwrap();
    let adopted = scene_text(&mut c);
    assert_ne!(adopted, text);
    let history = c.call("history.list", J::Null).unwrap();
    assert_eq!(history["entries"].as_array().unwrap().len(), 1);
    assert_eq!(history["entries"][0]["op_count"], 1);
    c.call("history.undo", J::Null).unwrap();
    assert_eq!(scene_text(&mut c), text);
    c.call("history.redo", J::Null).unwrap();
    assert_eq!(scene_text(&mut c), adopted);
    assert_eq!(
        f.text(),
        text,
        "proposal/accept/undo/redo must not save implicitly"
    );
    let saved = c.call("scene.save", json!({"write":true})).unwrap();
    assert_eq!(saved["text"], adopted);
    assert_eq!(saved["dirty"], false);
    let started = c.call("sim.start", J::Null).unwrap();
    c.call("sim.step", json!({"n":4})).unwrap();
    c.call("sim.stop", J::Null).unwrap();
    let restarted = c.call("sim.start", J::Null).unwrap();
    assert_eq!(started["checksum"], restarted["checksum"]);
    assert_eq!(restarted["head_tick"], 0);
    c.call("sim.stop", J::Null).unwrap();
    let reopened = PreparedProject::open(&f.root).unwrap();
    assert_eq!(reopened.scene().text(), adopted);
    let expected = room_escape::document(&adopted).unwrap();
    assert_eq!(
        expected.frame().to_bytes(),
        reopened.scene().frame().to_bytes()
    );
    let initial = room_escape::document(&text).unwrap();
    for (guid, entity) in &initial.scene().entities {
        assert_eq!(expected.scene().entities[guid], *entity);
    }
    assert_eq!(initial.scene().singletons, expected.scene().singletons);
    preserve_except_scene(&before, &f.root);
    if let Some(dir) = std::env::var_os("ORR_ROOM_WALL_CPU_EVIDENCE") {
        let dir = PathBuf::from(dir).join("adoption");
        fs::create_dir(&dir).unwrap();
        for (name, bytes) in [
            ("before.scene.yaml", text.as_bytes()),
            ("after.scene.yaml", adopted.as_bytes()),
            ("reopened.scene.yaml", reopened.scene().text().as_bytes()),
            ("wall.ops.json", ops.as_slice()),
        ] {
            fs::write(dir.join(name), bytes).unwrap();
        }
        fs::write(dir.join("initial.frame"), initial.frame().to_bytes()).unwrap();
        fs::write(
            dir.join("reopened.frame"),
            reopened.scene().frame().to_bytes(),
        )
        .unwrap();
        for (path, bytes) in before {
            let target = dir.join("before-project").join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, bytes).unwrap();
        }
        for (path, bytes) in tree(&f.root) {
            let target = dir.join("after-project").join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, bytes).unwrap();
        }
        fs::write(
            dir.join("verified-report.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.join("history.json"),
            serde_json::to_vec_pretty(&history).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn room_wall_live_document_stale_revalidation() {
    let f = Fixture::new();
    let ops: J = serde_json::from_slice(&generate(&f.options()).unwrap()).unwrap();
    for change in ["duplicate", "overlap", "budget"] {
        let mut live = room_escape::document(&f.text()).unwrap();
        match change {
            "duplicate" => add(&mut live, &f.options()),
            "overlap" => {
                live.apply(
                    Op::SetField {
                        guid: player_guid(&live),
                        component: "orr_physics3d::Body".into(),
                        path: "pos.x".into(),
                        value: Value::Fixed(FP(40960)),
                    },
                    Origin::User,
                )
                .unwrap();
            }
            "budget" => fill(&mut live, &f, 64),
            _ => unreachable!(),
        }
        let text = live.to_yaml();
        let (_host, mut c) = host(&text, f.root.join("room.scene.yaml"));
        let before = c.call("history.list", J::Null).unwrap();
        let id = c
            .call("proposal.begin", json!({"label":"stale offline ops"}))
            .unwrap()["id"]
            .clone();
        assert!(
            c.call("proposal.apply", json!({"id":id,"ops":ops}))
                .is_err(),
            "{change}"
        );
        assert_eq!(scene_text(&mut c), text);
        assert_eq!(c.call("history.list", J::Null).unwrap(), before);
    }
    let (_host, mut c) = host(&f.text(), f.root.join("room.scene.yaml"));
    let id = stage(&mut c, &ops);
    let report = verify(&mut c, &id);
    let player = player_guid(&room_escape::document(&f.text()).unwrap());
    c.call(
        "world.rename",
        json!({"entity":player.as_str(),"name":"live changed player"}),
    )
    .unwrap();
    assert!(c
        .call(
            "proposal.accept_verified",
            json!({"id":id,"verified_state":report["verified_state"]})
        )
        .is_err());
    let fresh = verify(&mut c, &id);
    c.call(
        "proposal.accept_verified",
        json!({"id":id,"verified_state":fresh["verified_state"]}),
    )
    .unwrap();
    assert!(scene_text(&mut c).contains("live changed player"));
}

#[test]
fn room_wall_collision_los_controls_and_restart() {
    let f = Fixture::new();
    let base = PreparedProject::open(&f.root).unwrap();
    let mut doc = room_escape::document(base.scene().text()).unwrap();
    add(&mut doc, &f.options());
    let authored = PreparedScene::parse(&doc.to_yaml()).unwrap();
    let mut a = authored.simulation().unwrap();
    let mut b = base.scene().simulation().unwrap();
    let press = RoomInput {
        buttons: INTERACT,
        ..RoomInput::default()
    };
    step(&mut a, press);
    step(&mut b, press);
    assert_eq!(
        a.frame().singleton::<RoomRun>().key_collected,
        0,
        "new wall must block key ray"
    );
    assert_eq!(
        b.frame().singleton::<RoomRun>().key_collected,
        1,
        "baseline key is reachable"
    );
    a = authored.simulation().unwrap();
    b = base.scene().simulation().unwrap();
    for _ in 0..80 {
        step(&mut a, walk(1, 0));
        step(&mut b, walk(1, 0));
    }
    assert!(position(&a).x < FP(8192));
    assert!(position(&b).x > FP::from_int(3));
    // The player can walk around the bounded wall and still collect the key.
    a = authored.simulation().unwrap();
    for _ in 0..30 {
        step(&mut a, walk(0, 1));
    }
    for _ in 0..24 {
        step(&mut a, walk(1, 0));
    }
    for _ in 0..30 {
        step(&mut a, walk(0, -1));
    }
    step(&mut a, press);
    assert_eq!(a.frame().singleton::<RoomRun>().key_collected, 1);
    let initial = authored.frame().to_bytes();
    let mut first = authored.simulation().unwrap();
    let mut second = authored.simulation().unwrap();
    let mut session = authored.session().unwrap();
    assert_eq!(session.frame().to_bytes(), initial);
    for tick in 0..96 {
        let input = if tick < 48 { walk(1, 0) } else { walk(0, 1) };
        step(&mut first, input);
        step(&mut second, input);
        session.set_input(PlayerSlot(0), input);
        assert!(session.step_now().is_some());
        assert_eq!(first.frame().to_bytes(), second.frame().to_bytes());
        assert_eq!(first.frame().to_bytes(), session.frame().to_bytes());
    }
    assert_eq!(authored.simulation().unwrap().frame().to_bytes(), initial);
    assert_eq!(authored.session().unwrap().frame().to_bytes(), initial);
    assert_ne!(initial, base.scene().frame().to_bytes());
    let mut control = f.options();
    control.center = [FP::from_int(-2), FP::ZERO];
    let mut control_doc = room_escape::document(base.scene().text()).unwrap();
    add(&mut control_doc, &control);
    let control_scene = PreparedScene::parse(&control_doc.to_yaml()).unwrap();
    let mut control_sim = control_scene.simulation().unwrap();
    step(&mut control_sim, press);
    assert_eq!(control_sim.frame().singleton::<RoomRun>().key_collected, 1);
}

#[test]
#[ignore = "mandatory gate: freshly retained Room wall CLI and CPU evidence directory required"]
fn room_wall_real_cli_ops_file_no_overwrite() {
    let binary =
        PathBuf::from(std::env::var_os("ORR_ROOM_WALL_BIN").expect("mandatory real CLI producer"));
    assert!(binary.is_absolute() && binary.is_file());
    let evidence = PathBuf::from(
        std::env::var_os("ORR_ROOM_WALL_CPU_EVIDENCE").expect("mandatory retained CPU evidence"),
    );
    assert!(evidence.is_dir());
    let f = Fixture::new();
    let before = tree(&f.root);
    let mut rejections = serde_json::Map::new();
    for name in [
        "malformed-number",
        "duplicate-option",
        "overlong-number",
        "extra-arguments",
        "missing-output",
    ] {
        let destination = f.base.join(format!("{name}.ops.json"));
        let mut args = f.args();
        args[13] = destination.clone().into_os_string();
        match name {
            "malformed-number" => args[7] = "not-a-number".into(),
            "duplicate-option" => args[6] = "--name".into(),
            "overlong-number" => args[7] = "1".repeat(49).into(),
            "extra-arguments" => args.extend((0..32).map(|_| OsString::from("extra"))),
            "missing-output" => {
                args.pop();
            }
            _ => unreachable!(),
        }
        let rejected = Command::new(&binary).args(args).output().unwrap();
        assert!(!rejected.status.success(), "{name}");
        assert!(!destination.exists());
        assert_eq!(tree(&f.root), before);
        rejections.insert(name.into(),json!({"exit":rejected.status.code().expect("CLI rejection exits normally"),"stdout":String::from_utf8(rejected.stdout).unwrap(),"stderr":String::from_utf8(rejected.stderr).unwrap()}));
    }
    let output = Command::new(&binary).args(f.args()).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = fs::read(f.options().output).unwrap();
    assert_eq!(bytes, generate(&f.options()).unwrap());
    let duplicate = Command::new(&binary).args(f.args()).output().unwrap();
    assert!(!duplicate.status.success());
    assert_eq!(fs::read(f.options().output).unwrap(), bytes);
    let (_host, mut c) = host(&f.text(), f.root.join("room.scene.yaml"));
    let id = stage(&mut c, &serde_json::from_slice(&bytes).unwrap());
    let report = verify(&mut c, &id);
    c.call(
        "proposal.accept_verified",
        json!({"id":id,"verified_state":report["verified_state"]}),
    )
    .unwrap();
    c.call("scene.save", json!({"write":true})).unwrap();
    PreparedProject::open(&f.root).unwrap();
    preserve_except_scene(&before, &f.root);
    fs::write(evidence.join("real-cli.ops.json"), bytes).unwrap();
    fs::write(evidence.join("real-cli.stdout"), output.stdout).unwrap();
    fs::write(
        evidence.join("real-cli-no-overwrite.stderr"),
        duplicate.stderr,
    )
    .unwrap();
    let hash = |bytes: &[u8]| {
        format!(
            "{:016x}",
            bytes
                .iter()
                .fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(*b))
                    .wrapping_mul(0x100000001b3))
        )
    };
    let receipt = json!({"schema":1,"status":"passed","cli_rejections":rejections,"source_fnv1a64":{"test":hash(include_bytes!("room_wall.rs")),"cli":hash(include_bytes!("../examples/room_wall.rs")),"authoring":hash(include_bytes!("../examples/room_wall/authoring.rs"))},"producer_fnv1a64":{"room-wall-cli":hash(&fs::read(&binary).unwrap())}});
    fs::write(
        evidence.join("cpu-receipt.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}
