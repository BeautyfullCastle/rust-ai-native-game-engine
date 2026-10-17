#![cfg(feature = "mesh-navigation")]
#![allow(clippy::disallowed_types)]

use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use orr_remote::mesh_navigation::{
    authoring::{create_mesh_scene, create_scene_from_asset, read_graph},
    mesh_doc_from_path, AgentSpec,
};
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture {
    parent: PathBuf,
    root: PathBuf,
    asset: PathBuf,
    scene: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = std::env::temp_dir().canonicalize().unwrap();
        let root = parent.join(format!(
            "orr_mesh_authoring_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let asset = root.join("floor.orrmesh");
        let graph = graph();
        fs::write(&asset, graph.to_bytes()).unwrap();
        Self {
            parent,
            asset,
            scene: root.join("floor.scene.yaml"),
            root,
        }
    }

    fn create(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_orr_mesh"))
            .args(["new", "--asset"])
            .arg(&self.asset)
            .arg("--output")
            .arg(&self.scene)
            .args([
                "--start",
                "0.25,0,0.25",
                "--goal",
                "1.75,0,1.75",
                "--distance",
                "0.25",
            ])
            .output()
            .unwrap()
    }

    fn command(&self, command: &str, arguments: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_orr_mesh"))
            .args([command, "--scene"])
            .arg(&self.scene)
            .args(arguments)
            .output()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Ok(resolved) = self.root.canonicalize() {
            if resolved.parent() == Some(self.parent.as_path()) && resolved == self.root {
                let _ = fs::remove_dir_all(&resolved);
            }
        }
    }
}

fn graph() -> StaticMeshGraph {
    StaticMeshGraph::bake(
        &[
            [fp!(0), fp!(0), fp!(0)],
            [fp!(2), fp!(0), fp!(0)],
            [fp!(0), fp!(0), fp!(2)],
            [fp!(2), fp!(0), fp!(2)],
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap()
}

fn spec() -> AgentSpec {
    AgentSpec {
        start: [fp!(0.25), FP::ZERO, fp!(0.25)],
        goal: [fp!(1.75), FP::ZERO, fp!(1.75)],
        max_vertical: FP::ZERO,
        distance_per_tick: fp!(0.25),
    }
}

fn json(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn authoring_uses_admission_and_saves_a_self_contained_scene_without_clobbering() {
    let fixture = Fixture::new();
    let expected = create_mesh_scene(&graph(), spec()).unwrap();
    let receipt = create_scene_from_asset(&fixture.asset, &fixture.scene, spec()).unwrap();
    let saved = fs::read(&fixture.scene).unwrap();
    assert_eq!(receipt.checksum, expected.frame().checksum());
    assert_eq!(
        mesh_doc_from_path(&fixture.scene)
            .unwrap()
            .frame()
            .to_bytes(),
        expected.frame().to_bytes()
    );
    assert!(create_scene_from_asset(&fixture.asset, &fixture.scene, spec()).is_err());
    assert_eq!(fs::read(&fixture.scene).unwrap(), saved);
    let uppercase_asset = fixture.root.join("FLOOR.ORRMESH");
    let uppercase_scene = fixture.root.join("OTHER.SCENE.YAML");
    fs::write(&uppercase_asset, graph().to_bytes()).unwrap();
    assert_eq!(
        read_graph(&uppercase_asset).unwrap().to_bytes(),
        graph().to_bytes()
    );
    create_scene_from_asset(&uppercase_asset, &uppercase_scene, spec()).unwrap();
    assert_eq!(
        mesh_doc_from_path(&uppercase_scene)
            .unwrap()
            .frame()
            .to_bytes(),
        expected.frame().to_bytes()
    );
    fs::remove_file(&fixture.asset).unwrap();
    assert_eq!(
        mesh_doc_from_path(&fixture.scene)
            .unwrap()
            .frame()
            .to_bytes(),
        expected.frame().to_bytes()
    );
}

#[test]
fn invalid_asset_or_agent_never_creates_a_scene() {
    let fixture = Fixture::new();
    let mut invalid = spec();
    invalid.distance_per_tick = FP::ZERO;
    assert!(create_scene_from_asset(&fixture.asset, &fixture.scene, invalid).is_err());
    assert!(!fixture.scene.exists());
    let directory = fixture.root.join("directory.orrmesh");
    fs::create_dir(&directory).unwrap();
    assert!(read_graph(&directory).is_err());
    assert!(mesh_doc_from_path(&directory).is_err());
    fs::write(&fixture.asset, vec![0; 8_193]).unwrap();
    assert!(read_graph(&fixture.asset).is_err());
    assert!(create_scene_from_asset(&fixture.asset, &fixture.scene, spec()).is_err());
    assert!(!fixture.scene.exists());
}

#[test]
fn real_cli_creates_reopens_and_reaches_the_exact_goal_without_the_source_asset() {
    let fixture = Fixture::new();
    let asset = json(
        Command::new(env!("CARGO_BIN_EXE_orr_mesh"))
            .args(["asset", "--asset"])
            .arg(&fixture.asset)
            .output()
            .unwrap(),
    );
    assert_eq!(asset["triangles"], 2);
    assert_eq!(asset["first_face_points"].as_array().unwrap().len(), 3);
    let created = json(fixture.create());
    let before = json(fixture.command("inspect", &[]));
    assert_eq!(created["graph_revision"], before["graph_revision"]);
    assert_eq!(created["checksum"], before["checksum"]);
    let saved = fs::read(&fixture.scene).unwrap();
    fs::remove_file(&fixture.asset).unwrap();
    let finished = json(fixture.command("run", &["--ticks", "300"]));
    assert_eq!(finished["tick"], 300);
    assert_eq!(finished["status"], "Arrived");
    assert_eq!(finished["position_raw"], finished["goal_raw"]);
    assert_ne!(finished["checksum"], before["checksum"]);
    assert_eq!(json(fixture.command("run", &["--ticks", "300"])), finished);
    assert_eq!(fs::read(&fixture.scene).unwrap(), saved);
}

#[test]
fn cli_rejects_bad_options_and_non_loopback_unless_explicit_local_development() {
    let fixture = Fixture::new();
    for arguments in [
        vec!["--ticks", "6001"],
        vec!["--ticks", "1", "--ticks", "2"],
        vec!["--unknown", "value"],
    ] {
        assert_eq!(fixture.command("run", &arguments).status.code(), Some(2));
    }
    assert_eq!(fixture.command("serve", &[]).status.code(), Some(2));
    assert_eq!(
        fixture
            .command("serve", &["--dev-no-auth", "--bind", "0.0.0.0:7777"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        fixture
            .command("serve", &["--dev-no-auth", "--stop-after-ms", "0"])
            .status
            .code(),
        Some(2)
    );
    assert!(!fixture.scene.exists());
    assert!(Command::new(env!("CARGO_BIN_EXE_orr_mesh"))
        .arg("--help")
        .output()
        .unwrap()
        .status
        .success());
}

#[test]
fn real_cli_preserves_existing_scene_and_starts_a_bounded_loopback_host() {
    let fixture = Fixture::new();
    json(fixture.create());
    let before = fs::read(&fixture.scene).unwrap();
    assert_eq!(fixture.create().status.code(), Some(1));
    assert_eq!(fs::read(&fixture.scene).unwrap(), before);
    let started = json(fixture.command(
        "serve",
        &[
            "--dev-no-auth",
            "--bind",
            "127.0.0.1:0",
            "--stop-after-ms",
            "50",
        ],
    ));
    assert_eq!(started["game"], "MeshNavigation");
    assert!(started["url"]
        .as_str()
        .unwrap()
        .starts_with("ws://127.0.0.1:"));
    assert_eq!(fs::read(&fixture.scene).unwrap(), before);
}
