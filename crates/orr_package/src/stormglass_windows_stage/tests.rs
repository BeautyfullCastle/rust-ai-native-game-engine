use super::*;
use crate::stormglass_windows_snapshot::ReservedInputs;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, process::Command, sync::OnceLock};

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn retained_root() -> PathBuf {
    tempfile::Builder::new()
        .prefix("stormglass-smoke-tests-")
        .tempdir()
        .unwrap()
        .keep()
}
fn probe() -> &'static Path {
    static PROBE: OnceLock<PathBuf> = OnceLock::new();
    PROBE.get_or_init(|| {
        let root = retained_root();
        let source = root.join("probe.rs");
        fs::write(&source, include_str!("probe.rs")).unwrap();
        let output = root.join("probe.exe");
        let build = Command::new("rustc")
            .args(["--edition", "2021", "-D", "warnings"])
            .arg(&source)
            .arg("-o")
            .arg(&output)
            .output()
            .unwrap();
        fs::write(root.join("build.stdout"), &build.stdout).unwrap();
        fs::write(root.join("build.stderr"), &build.stderr).unwrap();
        assert!(
            build.status.success(),
            "{}",
            String::from_utf8_lossy(&build.stderr)
        );
        eprintln!("retained trusted probe: {}", root.display());
        output
    })
}
fn process_case(name: &str, mode: &str) -> PathBuf {
    let root = retained_root().join(name);
    fs::create_dir(&root).unwrap();
    fs::copy(probe(), root.join("coop_defense.exe")).unwrap();
    fs::write(root.join("game.scene.yaml"), mode).unwrap();
    root
}
fn input_case() -> (PathBuf, PathBuf, String) {
    let root = retained_root();
    let input = root.join("inputs");
    fs::create_dir(&input).unwrap();
    let bytes = fs::read(probe()).unwrap();
    fs::write(input.join("coop_defense.exe"), &bytes).unwrap();
    fs::write(
        input.join("game.scene.yaml"),
        b"schema: orr.scene/1\nentities: {}\n",
    )
    .unwrap();
    (root, input, hash(&bytes))
}
fn output(text: &str) -> process::Output {
    process::Output {
        exit_code: 0,
        stdout: text.as_bytes().to_vec(),
        stderr: vec![],
    }
}
fn reserved(path: &Path) {
    assert!(fs::OpenOptions::new()
        .write(true)
        .open(path.join("game.scene.yaml"))
        .is_err());
}

#[test]
fn trust_and_precheck_refuse_before_execution() {
    let (root, input, pin) = input_case();
    let inputs = ReservedInputs::open(&input).unwrap();
    let mut stage = UnvalidatedStage::prepare(&inputs, &pin, &root).unwrap();
    let e = stage
        .check_with(false, |_| panic!("must not execute"))
        .unwrap_err();
    assert!(e.contains("trusted-runtime") && e.contains(&stage.path().display().to_string()));
    fs::write(stage.path().join("foreign"), b"retain").unwrap();
    let e = stage
        .check_with(true, |_| panic!("must not execute"))
        .unwrap_err();
    assert!(e.contains("closure"));
    reserved(&input);
    reserved(stage.path());
}

#[test]
fn postcheck_runs_after_success_and_failure_without_releasing_reservations() {
    for succeed in [false, true] {
        let (root, input, pin) = input_case();
        let inputs = ReservedInputs::open(&input).unwrap();
        let mut stage = UnvalidatedStage::prepare(&inputs, &pin, &root).unwrap();
        let e = stage
            .check_with(true, |path| {
                reserved(&input);
                reserved(path);
                fs::write(path.join("foreign"), b"retain").unwrap();
                if succeed {
                    Ok(output("wrong summary"))
                } else {
                    Err("injected run failure".into())
                }
            })
            .unwrap_err();
        assert!(e.contains("post-smoke stage recheck"));
        if !succeed {
            assert!(e.contains("injected run failure"));
        }
        reserved(&input);
        reserved(stage.path());
        assert_eq!(fs::read(stage.path().join("foreign")).unwrap(), b"retain");
    }
}

#[test]
fn matched_summary_leaves_stage_reserved_and_unpublished() {
    let (root, input, pin) = input_case();
    let inputs = ReservedInputs::open(&input).unwrap();
    let mut stage = UnvalidatedStage::prepare(&inputs, &pin, &root).unwrap();
    let expected = expected(*inputs.admit(&pin).unwrap().config());
    let before = stage.path().to_owned();
    assert_eq!(
        stage
            .check_with(true, |path| {
                reserved(path);
                Ok(output(&(expected.clone() + "\n")))
            })
            .unwrap(),
        expected
    );
    assert_eq!(stage.path(), before);
    reserved(&input);
    reserved(stage.path());
    assert_eq!(fs::read_dir(stage.path()).unwrap().count(), 3);
}

#[test]
fn summary_parser_is_strict_about_output_and_exit() {
    let expected = expected(game::DefenseConfig::default());
    assert!(verify_output(&output(&(expected.clone() + "\n")), &expected).is_ok());
    let optional =
        format!("Stormglass pixelfx active=0 started=0 dropped=0 canceled=0\r\n{expected}\r\n");
    assert!(verify_output(&output(&optional), &expected).is_ok());
    for bad in [
        format!("{expected}\nextra\n"),
        format!("{expected}\n\n"),
        expected.replace("tick=60", "tick=59"),
        format!("Stormglass pixelfx active=+0 started=0 dropped=0 canceled=0\n{expected}\n"),
        format!(
            "Stormglass pixelfx active=0 started=0 dropped=0 canceled=4294967296\n{expected}\n"
        ),
        String::new(),
    ] {
        assert!(verify_output(&output(&bad), &expected).is_err());
    }
    let mut invalid = output(&expected);
    invalid.stdout = vec![255];
    assert!(verify_output(&invalid, &expected).is_err());
    let mut stderr = output(&expected);
    stderr.stderr = b"diagnostic".to_vec();
    assert!(verify_output(&stderr, &expected)
        .unwrap_err()
        .contains("stderr"));
    let mut exit = output(&expected);
    exit.exit_code = 23;
    assert!(verify_output(&exit, &expected).unwrap_err().contains("23"));
    let mut changed = game::DefenseConfig::default();
    changed.scenario.reactor_hp += 1;
    assert_ne!(super::expected(changed), expected);
}

#[test]
fn supervisor_covers_environment_exit_stderr_deadline_cap_and_cleanup() {
    let root = process_case("Unicode space [한글] & args", "shape");
    let value = process::run(&root, Duration::from_secs(15), 65536).unwrap();
    assert_eq!(value.exit_code, 0);
    assert_eq!(value.stdout, b"shape checked\n");
    assert!(value.stderr.is_empty());
    let root = process_case("nonzero", "nonzero");
    let value = process::run(&root, Duration::from_secs(15), 65536).unwrap();
    assert_eq!(value.exit_code, 23);
    assert!(verify_output(&value, "").unwrap_err().contains("23"));
    let root = process_case("stderr", "stderr");
    let value = process::run(&root, Duration::from_secs(15), 65536).unwrap();
    assert_eq!(value.stderr, b"expected probe stderr\n");
    assert!(verify_output(&value, "").unwrap_err().contains("stderr"));
    for (name, mode, time, cap, expected) in [
        (
            "timeout",
            "timeout",
            Duration::from_millis(100),
            65536,
            "timed out",
        ),
        (
            "combined limit",
            "overflow",
            Duration::from_secs(15),
            64,
            "exceeds limit",
        ),
    ] {
        let root = process_case(name, mode);
        let now = process::query_clock::Clock::now();
        let e = process::run(&root, time, cap).err().unwrap();
        assert!(
            e.contains(expected)
                && e.contains("cleanup confirmed")
                && !e.contains("cleanup failure"),
            "{e}"
        );
        assert!(now.elapsed() < Duration::from_secs(7));
    }
    let root = process_case("descendant", "descendant");
    let value = process::run(&root, Duration::from_secs(15), 65536).unwrap();
    assert_eq!(value.exit_code, 0);
    assert!(String::from_utf8(value.stdout)
        .unwrap()
        .starts_with("descendant pid="));
    std::thread::sleep(Duration::from_millis(600));
    assert!(!root.join("descendant-escaped").exists());
    let root = process_case("abort suspended", "marker");
    let e = process::abort_suspended(&root).err().unwrap();
    assert!(
        e.contains("before resume")
            && e.contains("cleanup confirmed")
            && !e.contains("cleanup failure"),
        "{e}"
    );
    assert!(!root.join("executed").exists());
}

#[test]
#[ignore = "executes only an explicitly supplied independently trusted local runtime"]
fn real_trusted_runtime_keeps_reserved_stage() {
    assert_eq!(std::env::var("STORMGLASS_SMOKE_TRUSTED").unwrap(), "yes");
    let runtime = std::env::var_os("STORMGLASS_SMOKE_RUNTIME").unwrap();
    let scene = std::env::var_os("STORMGLASS_SMOKE_SCENE").unwrap();
    let pin = std::env::var("STORMGLASS_SMOKE_SHA256").unwrap();
    let root = PathBuf::from(std::env::var_os("STORMGLASS_SMOKE_EVIDENCE").unwrap());
    fs::create_dir(&root).unwrap();
    let input = root.join("inputs");
    fs::create_dir(&input).unwrap();
    let runtime = fs::read(runtime).unwrap();
    assert_eq!(hash(&runtime), pin.to_ascii_lowercase());
    fs::write(input.join("coop_defense.exe"), runtime).unwrap();
    fs::copy(scene, input.join("game.scene.yaml")).unwrap();
    let inputs = ReservedInputs::open(&input).unwrap();
    let mut stage = UnvalidatedStage::prepare(&inputs, &pin, &root).unwrap();
    let path = stage.path().to_owned();
    let summary = stage.check_trusted_runtime(true).unwrap();
    reserved(&input);
    reserved(stage.path());
    stage.recheck().unwrap();
    assert_eq!(fs::read_dir(stage.path()).unwrap().count(), 3);
    fs::write(
        root.join("result.txt"),
        format!(
            "stage: {}\n{summary}\nsource and stage still reserved; no publication\n",
            path.display()
        ),
    )
    .unwrap();
    eprintln!("{}\n{summary}", path.display());
}
