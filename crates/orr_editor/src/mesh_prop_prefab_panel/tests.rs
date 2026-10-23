use super::{context_matches, Completion, Context, Operation, Panel, RequestStamp};
use crate::mesh_art::Snapshot;
use orr_package::Manifest;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};

const STATIC_GLB: &[u8] = include_bytes!("../../../orr_model/tests/fixtures/model.glb");

fn context() -> Context {
    Context {
        scene: PathBuf::from("C:/project/mesh.scene.yaml"),
        scene_epoch: 4,
        graph_revision: [7; 32],
    }
}

fn static_package(root: &Path) -> PathBuf {
    let package_root = root.join("verified-static-package");
    fs::create_dir(&package_root).unwrap();
    fs::write(package_root.join("model.glb"), STATIC_GLB).unwrap();
    fs::write(package_root.join("LICENSE.txt"), b"Example license text\n").unwrap();
    let runtime = orr_package::Runtime::content_only();
    let manifest = Manifest {
        schema: 1,
        name: "verified-static".into(),
        version: "1.0.0".into(),
        engine: format!("={}", runtime.engine_version),
        capabilities: BTreeSet::from(["models".into()]),
        dependencies: BTreeMap::new(),
        files: BTreeSet::from(["LICENSE.txt".into(), "model.glb".into()]),
    };
    fs::write(
        package_root.join("orr.package.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    package_root
}

fn snapshot(root: &Path) -> Snapshot {
    Snapshot::from_package(&static_package(root), "model.glb", "LICENSE.txt", [7; 32]).unwrap()
}

fn poll_until_done(
    panel: &mut Panel,
    current: Option<Context>,
) -> Option<crate::mesh_prop_prefab::Template> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let candidate = panel.poll(current.clone());
        if candidate.is_some() || !panel.busy() {
            return candidate;
        }
        assert!(
            Instant::now() < deadline,
            "prefab worker did not finish promptly"
        );
        std::thread::yield_now();
    }
}

#[test]
fn worker_context_requires_exact_scene_epoch_and_graph_revision() {
    let stamp = RequestStamp {
        context: context(),
        path: PathBuf::from("C:/assets/bridge.mesh-prop-prefab.json"),
        input_generation: 3,
        operation: Operation::Load,
    };
    assert!(context_matches(
        &stamp,
        Some(&context()),
        Path::new("C:/assets/bridge.mesh-prop-prefab.json"),
        3,
        true,
    ));
    assert!(!context_matches(
        &stamp,
        None,
        Path::new("C:/assets/bridge.mesh-prop-prefab.json"),
        3,
        true,
    ));
    let mut changed = context();
    changed.scene_epoch += 1;
    assert!(!context_matches(
        &stamp,
        Some(&changed),
        Path::new("C:/assets/bridge.mesh-prop-prefab.json"),
        3,
        true,
    ));
    changed = context();
    changed.graph_revision = [8; 32];
    assert!(!context_matches(
        &stamp,
        Some(&changed),
        Path::new("C:/assets/bridge.mesh-prop-prefab.json"),
        3,
        true,
    ));
    assert!(!context_matches(
        &stamp,
        Some(&context()),
        Path::new("C:/assets/other.mesh-prop-prefab.json"),
        3,
        true,
    ));
    assert!(!context_matches(
        &stamp,
        Some(&context()),
        Path::new("C:/assets/bridge.mesh-prop-prefab.json"),
        4,
        true,
    ));
    assert!(!context_matches(
        &stamp,
        Some(&context()),
        Path::new("C:/assets/bridge.mesh-prop-prefab.json"),
        3,
        false,
    ));
}

#[test]
fn save_worker_captures_applied_snapshot_and_never_overwrites_existing_file() {
    let temporary = tempfile::tempdir().unwrap();
    let applied = snapshot(temporary.path());
    let mut panel = Panel {
        path: temporary
            .path()
            .join("bridge.mesh-prop-prefab.json")
            .display()
            .to_string(),
        ..Panel::default()
    };
    let destination = PathBuf::from(panel.path.trim());
    assert!(panel.start_save(context(), "Bridge", &applied));
    assert!(panel.busy());
    let candidate = poll_until_done(&mut panel, Some(context()));
    assert!(!panel.busy());
    assert!(candidate.is_none());
    assert!(panel
        .status
        .starts_with("Saved prop prefab asset. SHA-256: "));
    let bytes_before = fs::read(&destination).unwrap();
    assert!(!bytes_before.is_empty());

    assert!(panel.start_save(context(), "Changed", &applied));
    assert!(poll_until_done(&mut panel, Some(context())).is_none());
    assert!(!panel.busy());
    assert!(panel
        .status
        .starts_with("Prop prefab Save failed: publish mesh prop prefab without replacement:"));
    assert_eq!(fs::read(destination).unwrap(), bytes_before);
}

#[test]
fn load_requires_rights_ack_and_stale_completion_never_returns_candidate() {
    let temporary = tempfile::tempdir().unwrap();
    let art = snapshot(temporary.path());
    let template = crate::mesh_prop_prefab::Template::capture("Bridge", &art).unwrap();
    let bytes = template.encode().unwrap();
    let prefab_path = temporary.path().join("bridge.mesh-prop-prefab.json");
    crate::mesh_prop_prefab_store::save_new(&prefab_path, &bytes).unwrap();
    let mut panel = Panel {
        path: prefab_path.display().to_string(),
        ..Panel::default()
    };
    assert!(!panel.start_load(context()));
    assert!(panel.status.contains("acknowledge rights"));
    panel.rights_acknowledged = true;
    assert!(panel.start_load(context()));
    let candidate = poll_until_done(&mut panel, Some(context())).unwrap();
    assert_eq!(candidate.name(), "Bridge");
    assert!(panel.status.contains("no prop was added"));

    let (sender, receiver) = mpsc::channel();
    sender
        .send(Completion {
            stamp: RequestStamp {
                context: context(),
                path: prefab_path.clone(),
                input_generation: 0,
                operation: Operation::Load,
            },
            result: Ok((candidate, "0".repeat(64))),
        })
        .unwrap();
    drop(sender);
    panel.pending = Some(receiver);
    panel.input_generation = 1;
    assert!(panel.poll(Some(context())).is_none());
    assert!(!panel.busy());
    assert!(panel.status.contains("discarded"));
}

#[test]
fn stale_save_completion_reports_asset_without_returning_candidate() {
    let temporary = tempfile::tempdir().unwrap();
    let template =
        crate::mesh_prop_prefab::Template::capture("Bridge", &snapshot(temporary.path())).unwrap();
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Completion {
            stamp: RequestStamp {
                context: context(),
                path: PathBuf::from("C:/assets/bridge.mesh-prop-prefab.json"),
                input_generation: 0,
                operation: Operation::Save,
            },
            result: Ok((template, "a".repeat(64))),
        })
        .unwrap();
    drop(sender);
    let mut panel = Panel {
        path: "C:/assets/other.mesh-prop-prefab.json".into(),
        pending: Some(receiver),
        ..Panel::default()
    };

    assert!(panel.poll(Some(context())).is_none());
    assert!(!panel.busy());
    assert!(panel.status.contains("Saved prop prefab asset at"));
    assert!(panel.status.contains(&"a".repeat(64)));
}

#[test]
fn disconnected_worker_releases_busy_state_without_candidate() {
    let (sender, receiver) = mpsc::channel();
    drop(sender);
    let mut panel = Panel {
        pending: Some(receiver),
        ..Panel::default()
    };
    assert!(panel.busy());
    assert!(panel.poll(Some(context())).is_none());
    assert!(!panel.busy());
    assert!(panel.status.contains("disconnected"));
}
