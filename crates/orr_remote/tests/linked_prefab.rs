#![cfg(all(feature = "collect-dodge", feature = "linked-prefabs"))]

use orr_edit::{EditorDoc, HistoryEntry, PlayController};
use orr_reflect::Scene;
use orr_remote::{call_local, collect_dodge, Cap, Caps, ErpTarget, HostLimits, RpcError};
use orr_sample::collect_game::CollectDodgeV1;
use serde_json::{json, Value};

const INITIAL: &str = include_str!("../../../scenes/collect_dodge_v1.scene.yaml");
const SOURCE: &str = "actors.prefab.yaml";

struct Host {
    doc: EditorDoc,
    play: Option<PlayController<CollectDodgeV1>>,
    limits: HostLimits,
}

impl Host {
    fn new() -> Self {
        let mut limits = HostLimits::default();
        collect_dodge::configure(&mut limits, None);
        Self {
            doc: collect_dodge::document(INITIAL).unwrap(),
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
            "linked-test",
            caps,
            method,
            &params,
        )
    }

    fn seed(&mut self) -> (String, Value) {
        let captured = self
            .call(
                Caps::of(&[Cap::Read]),
                "prefab.capture",
                json!({"selected":["e_00000002"]}),
            )
            .unwrap();
        let text = captured["text"].as_str().unwrap().to_owned();
        let revision = self.doc.revision();
        let result = self
            .call(
                Caps::ALL,
                "prefab.instantiate",
                json!({"source":SOURCE,"text":text,"expected_revision":revision}),
            )
            .unwrap();
        (text, result["instances"][0].clone())
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

fn requests(instance: &Value, text: &str, revision: u64) -> Vec<(&'static str, Value)> {
    vec![
        (
            "prefab.instantiate",
            json!({"source":SOURCE,"text":text,"expected_revision":revision}),
        ),
        (
            "prefab.position",
            json!({"instance":instance["instance"],"source_guid":"e_00000002","value":[12,0],"expected_revision":revision}),
        ),
        (
            "prefab.revert",
            json!({"instance":instance["instance"],"source_guid":"e_00000002","expected_revision":revision}),
        ),
        (
            "prefab.update",
            json!({"instance":instance["instance"],"source":SOURCE,"digest":instance["digest"],"text":text,"expected_revision":revision}),
        ),
    ]
}

fn kind(error: &RpcError) -> Option<&str> {
    error.data.as_ref().and_then(|value| value["kind"].as_str())
}

#[test]
fn read_capability_can_inspect_but_cannot_invoke_any_linked_mutator() {
    let mut host = Host::new();
    let (text, instance) = host.seed();
    let read = Caps::of(&[Cap::Read]);
    let listed = host.call(read, "prefab.list", json!({})).unwrap();
    assert_eq!(listed["instances"][0], instance);
    let before = snapshot(&host);
    for (method, params) in requests(&instance, &text, host.doc.revision()) {
        let error = host.call(read, method, params).unwrap_err();
        assert_eq!(
            error.code,
            orr_remote::PERMISSION_DENIED,
            "{method}: {error:?}"
        );
        assert_eq!(snapshot(&host), before);
    }
}

#[test]
fn all_mutators_require_exact_revision_and_reject_unknown_parameters() {
    let mut host = Host::new();
    let (text, instance) = host.seed();
    let revision = host.doc.revision();
    let before = snapshot(&host);
    for (method, params) in requests(&instance, &text, revision) {
        let mut stale = params.clone();
        stale["expected_revision"] = json!(revision + 1);
        let error = host.call(Caps::ALL, method, stale).unwrap_err();
        assert_eq!(kind(&error), Some("stale_revision"), "{method}: {error:?}");
        assert_eq!(snapshot(&host), before);

        let mut missing = params.clone();
        missing.as_object_mut().unwrap().remove("expected_revision");
        assert_eq!(
            host.call(Caps::ALL, method, missing).unwrap_err().code,
            orr_remote::INVALID_PARAMS
        );
        assert_eq!(snapshot(&host), before);

        let mut unknown = params;
        unknown["ignore_revision"] = json!(true);
        assert_eq!(
            host.call(Caps::ALL, method, unknown).unwrap_err().code,
            orr_remote::INVALID_PARAMS
        );
        assert_eq!(snapshot(&host), before);
    }
    assert_eq!(
        host.call(Caps::ALL, "prefab.list", json!({"extra":true}))
            .unwrap_err()
            .code,
        orr_remote::INVALID_PARAMS
    );
    assert_eq!(
        host.call(
            Caps::ALL,
            "prefab.capture",
            json!({"selected":["e_00000002"],"extra":true})
        )
        .unwrap_err()
        .code,
        orr_remote::INVALID_PARAMS
    );
}

#[test]
fn linked_mutations_reject_play_mode_and_open_transactions_without_document_changes() {
    let mut host = Host::new();
    let (text, instance) = host.seed();
    host.call(Caps::ALL, "sim.start", json!({})).unwrap();
    let before = snapshot(&host);
    for (method, params) in requests(&instance, &text, host.doc.revision()) {
        let error = host.call(Caps::ALL, method, params).unwrap_err();
        assert_eq!(kind(&error), Some("sim_running"));
        assert_eq!(snapshot(&host), before);
    }
    host.call(Caps::ALL, "sim.stop", json!({})).unwrap();
    host.doc
        .begin_tx("owner transaction", orr_edit::Origin::User)
        .unwrap();
    let before = snapshot(&host);
    for (method, params) in requests(&instance, &text, host.doc.revision()) {
        let error = host.call(Caps::ALL, method, params).unwrap_err();
        assert_eq!(kind(&error), Some("tx_open"));
        assert_eq!(snapshot(&host), before);
    }
    host.doc.rollback_tx().unwrap();
}

#[test]
fn source_identity_and_baseline_digest_are_independent_update_guards() {
    let mut host = Host::new();
    let (text, instance) = host.seed();
    let before = snapshot(&host);
    let valid = requests(&instance, &text, host.doc.revision())
        .pop()
        .unwrap()
        .1;
    for (key, wrong) in [
        ("source", "other.prefab.yaml"),
        ("digest", "stale-baseline"),
    ] {
        let mut params = valid.clone();
        params[key] = json!(wrong);
        assert!(host.call(Caps::ALL, "prefab.update", params).is_err());
        assert_eq!(snapshot(&host), before);
    }
    host.call(Caps::ALL, "prefab.update", valid).unwrap();
    assert_eq!(snapshot(&host), before, "identical baseline is a no-op");
}

struct TempRoot(std::path::PathBuf);
impl TempRoot {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        for _ in 0..1024 {
            let nonce = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("orr-linked-save-{}-{nonce}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("temporary directory creation failed: {e}"),
            }
        }
        panic!("temporary test directory budget exhausted")
    }
}
impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn failed_scene_save_rename_preserves_dirty_history_metadata_and_checksum() {
    let root = TempRoot::new();
    let destination = root.0.join("level.scene.yaml");
    std::fs::create_dir(&destination).unwrap();
    let mut host = Host::new();
    host.limits.scene_path = Some(destination.clone());
    host.seed();
    assert!(host.doc.is_dirty());
    let before = snapshot(&host);
    let error = host
        .call(Caps::ALL, "scene.save", json!({"write":true}))
        .unwrap_err();
    assert_eq!(kind(&error), Some("io"));
    assert!(destination.is_dir());
    assert_eq!(snapshot(&host), before);
    // Proves failure did not replace the prior clean source or clean history ID.
    host.doc.undo().unwrap();
    assert!(!host.doc.is_dirty());
    assert_eq!(host.doc.to_yaml(), INITIAL);
}

#[test]
fn enabled_host_advertises_the_exact_linked_schema() {
    let mut host = Host::new();
    let actual = host
        .call(Caps::of(&[Cap::Read]), "registry.schema", json!({}))
        .unwrap();
    let expected: Value = serde_json::from_str(&host.doc.view().json_schema()).unwrap();
    assert_eq!(actual["schema"], expected);
    assert_eq!(
        actual["schema"]["properties"]["schema"]["enum"],
        json!(["orr.scene/1", "orr.scene/2"])
    );
    assert!(actual["schema"]["properties"].get("prefabs").is_some());
}
