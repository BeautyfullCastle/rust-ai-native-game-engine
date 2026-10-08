#![cfg(all(
    feature = "collect-audio",
    feature = "project-export",
    target_os = "linux",
    target_arch = "x86_64"
))]
#![allow(clippy::disallowed_types)]
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            assert!(!entry.file_type().unwrap().is_symlink());
            if entry.file_type().unwrap().is_dir() {
                visit(root, &entry.path(), out)
            } else {
                out.insert(
                    entry.path().strip_prefix(root).unwrap().into(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}
fn fixture(root: &Path) {
    fs::create_dir(root).unwrap();
    fs::write(root.join("orr.project.json"),br#"{"schema":2,"engine":"*","entry":{"game":"collect-dodge-v1","scene":"scene.yaml","audio":"audio.json"}}"#).unwrap();
    fs::write(
        root.join("scene.yaml"),
        include_bytes!("../../../scenes/collect_dodge_v1.scene.yaml"),
    )
    .unwrap();
    fs::write(
        root.join("audio.json"),
        orr_sample::collect_audio::Document::default_collect()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    let mut runtime = orr_package::Runtime::content_only();
    runtime.capabilities.insert("collect-audio".into());
    orr_package::Project::open(root, runtime)
        .unwrap()
        .install(&[Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/packages/collect-audio-v1")
            .canonicalize()
            .unwrap()])
        .unwrap();
}
fn required(name: &str) -> PathBuf {
    let path =
        PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} required; no skip")));
    assert!(path.is_file(), "{name} invalid");
    path.canonicalize().unwrap()
}
#[test]
#[ignore = "mandatory explicit production exporter/source-hidden read-only runtime acceptance"]
fn actual_audio_export_relocation_readonly_owned_pcm_and_off_policy() {
    let runtime = required("ORR_COLLECT_AUDIO_RUNTIME");
    let exporter = required("ORR_COLLECT_AUDIO_EXPORTER");
    let temp = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let source = temp.path().join("source");
    fixture(&source);
    let bundle = temp.path().join("export");
    let digest = Sha256::digest(fs::read(&runtime).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let result = Command::new(exporter)
        .args(["--project"])
        .arg(&source)
        .arg("--runtime")
        .arg(&runtime)
        .args(["--runtime-sha256", &digest, "--output"])
        .arg(&bundle)
        .arg("--trusted-runtime")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "export stdout={} stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(bundle.join("orr.export.json")).unwrap()).unwrap();
    assert_eq!(
        manifest["payload"]["profile"],
        "collect-dodge-audio-linux-x86_64-v1"
    );
    assert_eq!(
        fs::read(source.join("audio.json")).unwrap(),
        fs::read(bundle.join("project/audio.json")).unwrap()
    );
    let relocated = temp.path().join("relocated");
    fs::rename(&bundle, &relocated).unwrap();
    let before = files(&relocated);
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let run = || {
        Command::new("bwrap")
            .args([
                "--die-with-parent",
                "--ro-bind",
                "/",
                "/",
                "--dev-bind",
                "/dev/null",
                "/dev/null",
                "--tmpfs",
            ])
            .arg(&repo)
            .arg("--tmpfs")
            .arg(&source)
            .args(["--chdir", "/", "--"])
            .arg(relocated.join("run-collect-dodge"))
            .args([
                "--headless",
                "--ticks",
                "180",
                "--hold",
                "right",
                "--audio",
                "off",
                "--audio-render-check",
            ])
            .output()
            .unwrap()
    };
    let result = run();
    assert!(
        result.status.success(),
        "readonly runtime stdout={} stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(
        output.contains("collect audio: offline Kira; device=none; pickups=2;"),
        "{output}"
    );
    assert_eq!(files(&relocated), before);
    println!("source-hidden read-only runtime:\n{output}");
    println!("unchanged exported closure: {} files", before.len());
    // Corruption must fail admission even with --audio off before playback/device work.
    let lock: orr_package::Lock = serde_json::from_slice(
        &fs::read(relocated.join("project/orr.packages.lock.json")).unwrap(),
    )
    .unwrap();
    let p = &lock.packages["collect-audio-v1"];
    let object = p
        .manifest
        .files
        .iter()
        .find(|s| s.contains("objects/"))
        .unwrap();
    fs::write(
        relocated.join(format!(
            "project/.orr/packages/objects/{}/{object}",
            p.digest
        )),
        b"tampered",
    )
    .unwrap();
    let result = run();
    assert!(!result.status.success());
    assert!(!String::from_utf8_lossy(&result.stdout).contains("collect tick:"));
}
