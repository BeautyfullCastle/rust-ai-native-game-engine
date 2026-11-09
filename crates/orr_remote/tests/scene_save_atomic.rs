use orr_edit::{EditorDoc, HistoryEntry, PlayController};
use orr_reflect::{Scene, TypeRegistry};
use orr_remote::{Cap, Caps, ErpTarget, HostLimits, RpcError, call_local};
use orr_sim::Simulation;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const INITIAL: &str = "schema: orr.scene/1\nsingletons:\n  Score: { kills: [0,0,0,0,0,0,0,0] }\nentities:\n  e_00000001:\n    Position: { pos: [0,0] }\n    PlayerTag: { slot: 0 }\n";

struct Host {
    doc: EditorDoc,
    play: Option<PlayController<orr_testgame::Arena>>,
    limits: HostLimits,
}

impl Host {
    fn new(scene_path: PathBuf) -> Self {
        let mut types = TypeRegistry::new();
        orr_testgame::register_reflect(&mut types);
        let doc = EditorDoc::from_yaml(
            INITIAL,
            types,
            Simulation::<orr_testgame::Arena>::build_registry(),
            42,
        )
        .unwrap();
        let limits = HostLimits {
            scene_path: Some(scene_path),
            ..HostLimits::default()
        };
        Self {
            doc,
            play: None,
            limits,
        }
    }

    fn call(&mut self, caps: Caps, method: &str, params: Value) -> Result<Value, RpcError> {
        call_local(
            &mut ErpTarget {
                doc: &mut self.doc,
                play: &mut self.play,
            },
            &self.limits,
            "scene-save-test",
            caps,
            method,
            &params,
        )
    }

    fn patch_position(&mut self, x: i64) {
        self.call(
            Caps::ALL,
            "world.patch",
            json!({
                "entity": "e_00000001",
                "component": "Position",
                "path": "pos",
                "value": [x, 0]
            }),
        )
        .unwrap();
    }
}

#[derive(Debug, PartialEq)]
struct Snapshot {
    scene: Scene,
    frame: Vec<u8>,
    checksum: String,
    text: String,
    revision: u64,
    history: Vec<HistoryEntry>,
    dirty: bool,
    redo: bool,
}

fn snapshot(host: &Host) -> Snapshot {
    Snapshot {
        scene: host.doc.scene().clone(),
        frame: host.doc.frame().to_bytes(),
        checksum: format!("{:?}", host.doc.checksum()),
        text: host.doc.to_yaml(),
        revision: host.doc.revision(),
        history: host.doc.history(),
        dirty: host.doc.is_dirty(),
        redo: host.doc.can_redo(),
    }
}

fn rpc_kind(error: &RpcError) -> Option<&str> {
    error.data.as_ref().and_then(|value| value["kind"].as_str())
}

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct TestRoot(PathBuf);

impl TestRoot {
    #[allow(deprecated)] // Keep test fixture code compatible with the repository's Rust MSRV.
    fn new() -> Self {
        for _ in 0..32 {
            let serial = NEXT_ROOT
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_add(1)
                })
                .expect("scene-save test root serial does not exhaust");
            let path = std::env::temp_dir().join(format!(
                "orr-scene-save-api-{}-{serial}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create scene-save test root: {error}"),
            }
        }
        panic!("scene-save API test root creation attempts exhausted")
    }

    fn scene(&self) -> PathBuf {
        self.0.join("level.scene.yaml")
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assert_no_owned_temporary_files(directory: &Path) {
    let leftovers: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_string_lossy().starts_with(".orr-scene-save-"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "owned temporary files remain: {leftovers:?}"
    );
}

#[test]
fn scene_save_preserves_predictable_regular_temp_and_reopens_saved_yaml() {
    let root = TestRoot::new();
    let scene_path = root.scene();
    let predictable_temp = scene_path.with_extension("scene.yaml.tmp");
    fs::write(&scene_path, b"old scene bytes").unwrap();
    fs::write(&predictable_temp, b"existing temporary-name occupant").unwrap();

    let mut host = Host::new(scene_path.clone());
    host.patch_position(1);
    assert!(host.doc.is_dirty());
    let saved = host
        .call(Caps::ALL, "scene.save", json!({"write": true}))
        .unwrap();

    assert_eq!(saved["dirty"], false);
    assert_eq!(saved["written"], scene_path.display().to_string());
    assert!(
        fs::symlink_metadata(&scene_path)
            .unwrap()
            .file_type()
            .is_file()
    );
    let disk_bytes = fs::read(&scene_path).unwrap();
    let saved_text = saved["text"].as_str().unwrap();
    assert_eq!(disk_bytes, saved_text.as_bytes());
    assert_eq!(
        fs::read(predictable_temp).unwrap(),
        b"existing temporary-name occupant"
    );

    let mut reopened = Host::new(scene_path);
    let loaded = reopened
        .call(
            Caps::ALL,
            "scene.load",
            json!({"text": std::str::from_utf8(&disk_bytes).unwrap()}),
        )
        .unwrap();
    assert_eq!(loaded["checksum"], saved["checksum"]);
    assert_eq!(reopened.doc.to_yaml(), saved_text);
    assert_eq!(reopened.doc.checksum(), host.doc.checksum());
    assert!(!reopened.doc.is_dirty());
    assert_no_owned_temporary_files(&root.0);
}

#[cfg(unix)]
#[test]
fn scene_save_does_not_follow_or_replace_predictable_temp_symlink() {
    let root = TestRoot::new();
    let scene_path = root.scene();
    let sentinel = root.0.join("outside-sentinel");
    let predictable_temp = scene_path.with_extension("scene.yaml.tmp");
    let sentinel_before = b"this file belongs to another path";
    fs::write(&scene_path, b"old scene bytes").unwrap();
    fs::write(&sentinel, sentinel_before).unwrap();
    std::os::unix::fs::symlink(&sentinel, &predictable_temp).unwrap();

    let mut host = Host::new(scene_path.clone());
    host.patch_position(1);
    let saved = host
        .call(Caps::ALL, "scene.save", json!({"write": true}))
        .unwrap();

    assert_eq!(saved["dirty"], false);
    assert_eq!(fs::read(&sentinel).unwrap(), sentinel_before);
    assert!(
        fs::symlink_metadata(&scene_path)
            .unwrap()
            .file_type()
            .is_file()
    );
    assert_eq!(
        fs::read(&scene_path).unwrap(),
        saved["text"].as_str().unwrap().as_bytes()
    );
    assert!(
        fs::symlink_metadata(&predictable_temp)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_link(predictable_temp).unwrap(), sentinel);
    assert_no_owned_temporary_files(&root.0);
}

#[test]
fn failed_rename_preserves_document_history_and_existing_destination() {
    let root = TestRoot::new();
    let scene_path = root.scene();
    fs::create_dir(&scene_path).unwrap();
    let witness = scene_path.join("witness");
    fs::write(&witness, b"keep existing destination").unwrap();

    let mut host = Host::new(scene_path.clone());
    host.patch_position(1);
    host.patch_position(2);
    host.doc.undo().unwrap();
    assert!(host.doc.is_dirty());
    assert!(host.doc.can_redo());
    let before = snapshot(&host);

    let error = host
        .call(Caps::ALL, "scene.save", json!({"write": true}))
        .unwrap_err();

    assert_eq!(rpc_kind(&error), Some("io"));
    assert!(scene_path.is_dir());
    assert_eq!(fs::read(witness).unwrap(), b"keep existing destination");
    assert_eq!(snapshot(&host), before);
    assert_no_owned_temporary_files(&root.0);
}

#[test]
fn read_only_scene_save_is_rejected_without_file_or_document_changes() {
    let root = TestRoot::new();
    let scene_path = root.scene();
    fs::write(&scene_path, b"do not change").unwrap();

    let mut host = Host::new(scene_path.clone());
    host.patch_position(1);
    let before = snapshot(&host);
    let error = host
        .call(Caps::of(&[Cap::Read]), "scene.save", json!({"write": true}))
        .unwrap_err();

    assert_eq!(error.code, orr_remote::PERMISSION_DENIED);
    assert_eq!(fs::read(scene_path).unwrap(), b"do not change");
    assert_eq!(snapshot(&host), before);
    assert_no_owned_temporary_files(&root.0);
}
