//! Actual exporter/runtime smoke and ordinary relocation. No window/GPU or sandbox claim.
#![cfg(all(
    feature = "room-progress",
    feature = "project-create",
    feature = "project-export",
    target_os = "linux",
    target_arch = "x86_64"
))]
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
fn required(name: &str) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} required")));
    assert!(
        path.is_absolute() && path.is_file(),
        "{name}: absolute actual binary required"
    );
    path
}
fn good(output: Output) -> String {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            assert!(!entry.file_type().unwrap().is_symlink());
            if path.is_dir() {
                visit(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    if root.exists() {
        visit(root, root, &mut result);
    }
    result
}
#[test]
#[ignore = "requires actual trusted feature-on/off Room binaries, exporter and evidence directory"]
fn actual_room_progress_export_relocation_and_feature_off_smoke_refusal() {
    let runtime = required("ORR_ROOM_PROGRESS_RUNTIME");
    let off = required("ORR_ROOM_PROGRESS_FEATURE_OFF_RUNTIME");
    let exporter = required("ORR_ROOM_PROGRESS_EXPORTER");
    let evidence = PathBuf::from(
        std::env::var_os("ORR_ROOM_PROGRESS_EXPORT_EVIDENCE").expect("evidence directory required"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("source");
    orr_sample::project_create::create(&orr_sample::project_create::CreateOptions {
        output: root.clone(),
        template: orr_sample::project_create::ROOM_TEMPLATE.into(),
        seed: "progress-export".into(),
    })
    .unwrap();
    let manifest_path = root.join("orr.project.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["schema"] = 3.into();
    manifest["progress"] = serde_json::json!({"schema":1,"game_id":"12345678-1234-4234-8234-123456789abc","profile":"room-escape-clear-v1"});
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let source_before = tree(&root);
    let supplied = temp.path().join("supplied-runtime");
    fs::copy(runtime, &supplied).unwrap();
    let data = temp.path().join("external-data");
    let export = |runtime: &Path, output: &Path| {
        Command::new(&exporter)
            .arg("--project")
            .arg(&root)
            .arg("--runtime")
            .arg(runtime)
            .arg("--runtime-sha256")
            .arg(
                Sha256::digest(fs::read(runtime).unwrap())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>(),
            )
            .arg("--trusted-runtime")
            .arg("--output")
            .arg(output)
            .env("XDG_DATA_HOME", &data)
            .env("HOME", temp.path().join("unused-home"))
            .output()
            .unwrap()
    };
    let refused = temp.path().join("refused");
    let bad = export(&off, &refused);
    fs::write(evidence.join("feature-off.stdout"), &bad.stdout).unwrap();
    fs::write(evidence.join("feature-off.stderr"), &bad.stderr).unwrap();
    assert!(!bad.status.success());
    let diagnostic = String::from_utf8_lossy(&bad.stderr);
    assert!(
        diagnostic.contains("progress") || diagnostic.contains("metadata"),
        "wrong refusal: {diagnostic}"
    );
    assert!(!refused.exists());
    assert!(!data.exists(), "failed smoke must not create progress");
    let output = temp.path().join("bundle");
    let exported = export(&supplied, &output);
    fs::write(evidence.join("export.stdout"), &exported.stdout).unwrap();
    fs::write(evidence.join("export.stderr"), &exported.stderr).unwrap();
    good(exported);
    assert!(!data.exists());
    assert_eq!(tree(&root), source_before);
    let bundled: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("project/orr.project.json")).unwrap())
            .unwrap();
    assert_eq!(bundled["progress"], manifest["progress"]);
    let relocated = temp.path().join("relocated");
    fs::rename(output, &relocated).unwrap();
    fs::rename(&root, temp.path().join("hidden-source")).unwrap();
    fs::rename(&supplied, temp.path().join("hidden-runtime")).unwrap();
    assert!(!root.exists() && !supplied.exists());
    let before = tree(&relocated);
    let cwd = temp.path().join("empty-cwd");
    fs::create_dir(&cwd).unwrap();
    // No store is discovered even if headless is relaunched. This is not a win test.
    for attempt in 0..2 {
        let run = Command::new(relocated.join("run-room-escape"))
            .args(["--headless", "--ticks", "60", "--hold", "right,interact"])
            .current_dir(&cwd)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .env("XDG_DATA_HOME", &data)
            .env("HOME", temp.path().join("unused-home"))
            .output()
            .unwrap();
        fs::write(
            evidence.join(format!("relocated-{attempt}.stdout")),
            &run.stdout,
        )
        .unwrap();
        fs::write(
            evidence.join(format!("relocated-{attempt}.stderr")),
            &run.stderr,
        )
        .unwrap();
        assert!(good(run).contains("room tick: 60 checksum:"));
        assert!(!data.exists());
        assert_eq!(tree(&relocated), before);
    }
}
