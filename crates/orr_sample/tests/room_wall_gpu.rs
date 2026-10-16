//! Mandatory software-GPU appearance of an actual CLI-authored Room wall.
//! Missing CLI, software adapter or capture directory is a failure, never a skip.
#![cfg(feature = "room-project")]
#![allow(clippy::float_arithmetic, clippy::disallowed_types)]
use orr_reflect::{Scene, Value};
use orr_sample::{
    room_game::{RoomConfig, RoomEscapeV1},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use orr_sim::Simulation;
use serde_json::json;
use std::{
    collections::BTreeMap,
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
fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir(to).unwrap();
    for (path, bytes) in tree(from) {
        let target = to.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, bytes).unwrap();
    }
}
fn fnv(bytes: &[u8]) -> String {
    format!(
        "{:016x}",
        bytes
            .iter()
            .fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(*b))
                .wrapping_mul(0x100000001b3))
    )
}
fn producer() -> PathBuf {
    let path =
        PathBuf::from(std::env::var_os("ORR_ROOM_WALL_BIN").expect("mandatory fresh wall CLI"));
    assert!(path.is_absolute() && path.is_file());
    path
}
fn produce(root: &Path, output: &Path, center: [&str; 2], half: [&str; 2]) -> std::process::Output {
    let result = Command::new(producer())
        .arg("--project")
        .arg(root)
        .args([
            "--guid",
            GUID,
            "--name",
            "authored wall",
            "--center",
            center[0],
            center[1],
            "--half-extents",
            half[0],
            half[1],
            "--output",
        ])
        .arg(output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    result
}
// Read the ordinary emitted spawn through the production reflection parser.
// This adapter does not author a replacement wall or special operation schema.
// ERP guarded acceptance and save are independently exercised in orr_remote;
// here EditorDoc applies the same emitted op, then full project admission checks it.
fn consume(root: &Path, bytes: &[u8]) {
    let json: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(json.as_array().unwrap().len(), 1);
    let op = &json[0];
    assert_eq!(op["op"], "spawn");
    assert_eq!(op["guid"], GUID);
    assert_eq!(
        op.as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        ["op", "guid", "name", "components"].into_iter().collect()
    );
    let mut yaml = format!(
        "schema: orr.scene/1\nentities:\n  {GUID}:\n    name: {}\n",
        op["name"]
    );
    for (name, value) in op["components"].as_object().unwrap() {
        yaml.push_str(&format!(
            "    {}: {}\n",
            serde_json::to_string(name).unwrap(),
            value
        ));
    }
    let fragment = Scene::parse(&yaml, &room_project::types()).unwrap();
    let entity = fragment.entities.values().next().unwrap();
    let prepared = PreparedProject::open(root).unwrap();
    let before = tree(root);
    let mut doc = orr_edit::EditorDoc::from_yaml(
        prepared.scene().text(),
        room_project::types(),
        Simulation::<RoomEscapeV1>::build_registry(),
        SEED,
    )
    .unwrap();
    doc.apply(
        orr_edit::Op::SpawnEntity {
            guid: Some(orr_reflect::Guid::parse(GUID).unwrap()),
            name: entity.name.clone(),
            components: entity.components.clone(),
        },
        orr_edit::Origin::User,
    )
    .unwrap();
    PreparedScene::parse(&doc.to_yaml()).unwrap();
    let initial = Scene::parse(prepared.scene().text(), &room_project::types()).unwrap();
    for (guid, entity) in &initial.entities {
        assert_eq!(doc.scene().entities[guid], *entity);
    }
    assert_eq!(doc.scene().singletons, initial.singletons);
    fs::write(root.join("room.scene.yaml"), doc.to_yaml()).unwrap();
    let reopened = PreparedProject::open(root).unwrap();
    assert_eq!(reopened.scene().frame().to_bytes(), doc.frame().to_bytes());
    let mut after = tree(root);
    let mut before = before;
    before.remove(Path::new("room.scene.yaml"));
    after.remove(Path::new("room.scene.yaml"));
    assert_eq!(
        before, after,
        "wall adoption changed model/camera/package data"
    );
}
fn artifact(dir: &Path, name: &str, bytes: &[u8]) -> serde_json::Value {
    use std::io::Write;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join(name))
        .unwrap();
    f.write_all(bytes).unwrap();
    f.sync_all().unwrap();
    json!({"file":name,"bytes":bytes.len(),"fnv1a64":fnv(bytes)})
}

const SIZE: u32 = 512;
fn capture(dir: &Path, name: &str, pixels: &[u8]) -> serde_json::Value {
    assert_eq!(pixels.len(), (SIZE * SIZE * 4) as usize);
    let raw = artifact(dir, &format!("{name}.rgba"), pixels);
    let mut ppm = format!("P6\n{SIZE} {SIZE}\n255\n").into_bytes();
    for pixel in pixels.chunks_exact(4) {
        ppm.extend_from_slice(&pixel[..3]);
    }
    let image = artifact(dir, &format!("{name}.ppm"), &ppm);
    json!({"raw":raw,"image":image})
}
fn render(gpu: &orr_render::orr_rhi::Wgpu, prepared: &PreparedProject) -> Vec<u8> {
    let before = prepared.scene().frame().to_bytes();
    let mut renderer =
        orr_sample::room_view::RoomRenderer::new(gpu, (SIZE, SIZE), prepared.models()).unwrap();
    let mut camera = orr_render::Camera3D::orthographic([0.0, 20.0, 0.0], [0.0, 0.0, 0.0], 4.0);
    camera.up = [0.0, 0.0, -1.0];
    renderer
        .render(
            orr_bridge::FrameView::of(prepared.scene().frame()),
            prepared.scene().index(),
            prepared.models(),
            &camera,
        )
        .unwrap();
    assert_eq!(prepared.scene().frame().to_bytes(), before);
    renderer.read_rgba8()
}
fn in_rect(x: usize, y: usize, rect: [usize; 4]) -> bool {
    x >= rect[0] && x < rect[2] && y >= rect[1] && y < rect[3]
}

#[test]
#[ignore = "mandatory gate: fresh real wall CLI, software Vulkan GPU and fresh capture directory"]
fn room_wall_real_software_gpu_appearance_and_controls() {
    use orr_render::orr_rhi::{Wgpu, WgpuOptions};
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let capture_dir = PathBuf::from(
        std::env::var_os("ORR_ROOM_WALL_CAPTURE_DIR").expect("mandatory retained captures"),
    );
    assert!(capture_dir.is_absolute());
    fs::create_dir(&capture_dir).expect("capture directory must not exist");
    let gpu = Wgpu::headless(WgpuOptions {
        force_software: true,
        ..WgpuOptions::default()
    })
    .expect("mandatory software GPU unavailable");
    assert!(gpu.is_software());
    let info = gpu.adapter().get_info();
    let adapter = json!({"name":info.name,"backend":format!("{:?}",info.backend),"device_type":format!("{:?}",info.device_type),"vendor":info.vendor,"device":info.device,"driver":info.driver,"driver_info":info.driver_info,"software":gpu.is_software()});
    assert_eq!(adapter["backend"], "Vulkan");
    artifact(
        &capture_dir,
        "adapter.json",
        &serde_json::to_vec_pretty(&adapter).unwrap(),
    );
    let f = Fixture::new();
    let authored_root = f.base.join("authored");
    let wrong_root = f.base.join("wrong-position");
    let models_off_root = f.base.join("models-off");
    for path in [&authored_root, &wrong_root, &models_off_root] {
        copy_tree(&f.root, path);
    }
    let ops = f.base.join("wall.ops.json");
    let wrong_ops = f.base.join("wrong-position.ops.json");
    produce(&authored_root, &ops, ["2.5", "-2"], ["0.5", "0.75"]);
    produce(&wrong_root, &wrong_ops, ["2.5", "2"], ["0.5", "0.75"]);
    let ops = fs::read(ops).unwrap();
    let wrong_ops = fs::read(wrong_ops).unwrap();
    consume(&authored_root, &ops);
    consume(&wrong_root, &wrong_ops);
    let path = models_off_root.join("room.models.json");
    let mut sidecar: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for binding in sidecar["bindings"].as_object_mut().unwrap().values_mut() {
        binding["transform"]["translation"] = json!([1000, 1000, 1000]);
    }
    fs::write(path, serde_json::to_vec_pretty(&sidecar).unwrap()).unwrap();
    let baseline = PreparedProject::open(&f.root).unwrap();
    let authored = PreparedProject::open(&authored_root).unwrap();
    let wrong = PreparedProject::open(&wrong_root).unwrap();
    let off = PreparedProject::open(&models_off_root).unwrap();
    assert_eq!(baseline.models().document, authored.models().document);
    assert_eq!(
        baseline.camera().unwrap().bytes,
        authored.camera().unwrap().bytes
    );
    assert_eq!(
        baseline.scene().frame().to_bytes(),
        off.scene().frame().to_bytes()
    );
    let a = render(&gpu, &baseline);
    let b = render(&gpu, &authored);
    let c = render(&gpu, &wrong);
    let d = render(&gpu, &off);
    let frames = json!({"baseline":capture(&capture_dir,"baseline",&a),"authored":capture(&capture_dir,"authored",&b),"wrong_position":capture(&capture_dir,"wrong-position",&c),"models_off":capture(&capture_dir,"models-off",&d)});
    let rect = [384, 80, 448, 176];
    let interior = [386, 82, 446, 174];
    let expanded = [382, 78, 450, 178];
    let wrong_rect = [384, 336, 448, 432];
    let mut inside = 0;
    let mut model_pixels = 0;
    let mut wrong_pixels = 0;
    for index in 0..(SIZE * SIZE) as usize {
        let x = index % SIZE as usize;
        let y = index / SIZE as usize;
        let at = 4 * index;
        let pa = &a[at..at + 4];
        let pb = &b[at..at + 4];
        let pc = &c[at..at + 4];
        let pd = &d[at..at + 4];
        if in_rect(x, y, interior) {
            assert_ne!(pa, pb, "missing wall interior {x},{y}");
            assert_eq!(pa, pc, "wrong-position control contaminated wall interior");
            inside += 1;
        }
        if !in_rect(x, y, expanded) {
            assert_eq!(pa, pb, "wall changed exterior pixel {x},{y}");
            if pa != pd {
                assert_eq!(pb, pa);
                model_pixels += 1;
            }
        }
        if in_rect(
            x,
            y,
            [
                wrong_rect[0] + 2,
                wrong_rect[1] + 2,
                wrong_rect[2] - 2,
                wrong_rect[3] - 2,
            ],
        ) && pa != pc
        {
            wrong_pixels += 1;
        }
    }
    assert_eq!(inside, 60 * 92);
    assert!(
        model_pixels > 20,
        "existing real models must contribute unchanged visible pixels"
    );
    assert!(
        wrong_pixels > 500,
        "wrong position must change a separate region"
    );
    artifact(&capture_dir, "wall.ops.json", &ops);
    artifact(&capture_dir, "wrong-position.ops.json", &wrong_ops);
    for (name, root) in [
        ("baseline-project", &f.root),
        ("authored-project", &authored_root),
        ("wrong-position-project", &wrong_root),
        ("models-off-project", &models_off_root),
    ] {
        copy_tree(root, &capture_dir.join(name));
    }
    let receipt = json!({"schema":1,"status":"passed","width":SIZE,"height":SIZE,"adapter":adapter,"frames":frames,"wall_rect":rect,"interior_rect":interior,"exterior_exclusion_rect":expanded,"wrong_position_rect":wrong_rect,"interior_pixels":inside,"unchanged_model_pixels":model_pixels,"wrong_position_pixels":wrong_pixels,"source_fnv1a64":{"test":fnv(include_bytes!("room_wall_gpu.rs")),"cli":fnv(include_bytes!("../../orr_remote/examples/room_wall.rs")),"authoring":fnv(include_bytes!("../../orr_remote/examples/room_wall/authoring.rs"))},"producer_fnv1a64":{"room-wall-cli":fnv(&fs::read(producer()).unwrap())}});
    artifact(
        &capture_dir,
        "receipt.json",
        &serde_json::to_vec_pretty(&receipt).unwrap(),
    );
}
