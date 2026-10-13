use orr_terrain_demo::{DEFAULT_COURSE, DEFAULT_TERRAIN};
use std::{fs, process::Command};
fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_terrain_demo"))
}

#[test]
fn actual_headless_binary_loads_builtin_and_explicit_course_without_writing() {
    let out = command().arg("--headless").output().unwrap();
    assert!(out.status.success(), "{:?}", out);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["beacons"], 4);
    assert_eq!(v["width"], 17);
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let c = root.join("course.json");
    let t = root.join("hill_course.orrt");
    fs::write(&c, DEFAULT_COURSE).unwrap();
    fs::write(&t, DEFAULT_TERRAIN).unwrap();
    let out = command()
        .arg("--headless")
        .arg("--course")
        .arg(&c)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(fs::read(&c).unwrap(), DEFAULT_COURSE);
    assert_eq!(fs::read(&t).unwrap(), DEFAULT_TERRAIN);
    assert_eq!(fs::read_dir(root).unwrap().count(), 2);
}

#[test]
fn actual_binary_rejects_bad_arguments_and_bad_snapshot_without_writes() {
    for args in [
        vec!["--unknown"],
        vec!["--course"],
        vec!["--headless", "--headless"],
        vec!["--help", "--headless"],
    ] {
        let out = command().args(args).output().unwrap();
        assert!(!out.status.success());
    }
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().canonicalize().unwrap().join("bad.json");
    fs::write(&path, b"{}").unwrap();
    let out = command()
        .arg("--headless")
        .arg("--course")
        .arg(&path)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(fs::read(path).unwrap(), b"{}");
}

#[test]
fn help_is_cpu_only_and_describes_controls() {
    let out = command().arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("WASD"));
    assert!(text.contains("--course"));
}

#[cfg(not(feature = "window"))]
#[test]
fn default_binary_explains_window_opt_in() {
    let out = command().output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8(out.stderr)
        .unwrap()
        .contains("--features window"));
}

#[cfg(unix)]
#[test]
#[allow(clippy::disallowed_types)]
fn actual_binary_rejects_fifo_before_open_without_blocking() {
    use std::{
        process::Stdio,
        thread,
        time::{Duration, Instant},
    };
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let course = root.join("course.json");
    let terrain = root.join("hill_course.orrt");
    fs::write(&course, DEFAULT_COURSE).unwrap();
    assert!(Command::new("mkfifo")
        .arg(&terrain)
        .status()
        .unwrap()
        .success());
    let mut child = command()
        .arg("--headless")
        .arg("--course")
        .arg(&course)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(!status.success());
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let _ = child.wait();
            panic!("regular-file precheck failed: FIFO input blocked the CLI");
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(fs::read(&course).unwrap(), DEFAULT_COURSE);
    assert!(!fs::symlink_metadata(&terrain).unwrap().is_file());
}
