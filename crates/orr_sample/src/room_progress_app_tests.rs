//! Production App steps/restart with a real external store, no synthetic won flags.
//! Store injection keeps process-global XDG/HOME untouched during parallel tests.
use super::*;
use crate::game_progress::{
    room::{RoomClearKey, RoomClearStore},
    ProgressPaths,
};
use crate::room_progress::ProgressSession;
use std::{collections::BTreeMap, fs};

fn fixture() -> (tempfile::TempDir, App) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    crate::project_create::create(&crate::project_create::CreateOptions {
        output: root.clone(),
        template: crate::project_create::ROOM_TEMPLATE.into(),
        seed: "room-clear-app".into(),
    })
    .unwrap();
    let project = PreparedProject::open(&root).unwrap();
    let app = App::new(project).unwrap();
    assert!(app.progress.status().contains("no authored identity"));
    (temp, app)
}
fn key(app: &App) -> RoomClearKey {
    RoomClearKey {
        game_id: [
            0x12, 0x34, 0x56, 0x78, 0x12, 0x34, 0x42, 0x34, 0x82, 0x34, 0x12, 0x34, 0x56, 0x78,
            0x9a, 0xbc,
        ],
        challenge: crate::room_progress_digest::challenge_digest(&app.project),
    }
}
fn store(path: &Path, key: RoomClearKey) -> RoomClearStore {
    RoomClearStore::open(ProgressPaths::from_directory(path.to_owned()).unwrap(), key).unwrap()
}
fn snapshot(root: &Path) -> BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)> {
    fn collect(
        root: &Path,
        path: &Path,
        out: &mut BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)>,
    ) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    (
                        fs::read(&path).unwrap(),
                        fs::metadata(&path).unwrap().modified().unwrap(),
                    ),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    if root.exists() {
        collect(root, root, &mut result);
    }
    result
}
fn win(app: &mut App) {
    let mut baseline = app.project.scene().simulation().unwrap();
    let mut inputs = vec![RoomInput {
        buttons: INTERACT,
        ..Default::default()
    }];
    inputs.extend(std::iter::repeat_n(
        RoomInput {
            move_x: 1,
            ..Default::default()
        },
        40,
    ));
    inputs.push(RoomInput {
        buttons: INTERACT,
        ..Default::default()
    });
    for input in inputs {
        app.step_authoritative(input).unwrap();
        step(&mut baseline, input);
        assert_eq!(
            app.sim.frame().to_bytes(),
            baseline.frame().to_bytes(),
            "persistence must not modify simulation"
        );
    }
    assert_eq!(app.sim.frame().singleton::<RoomRun>().won, 1);
}
#[test]
fn authoritative_win_records_once_across_terminal_ticks_restart_and_reopen() {
    let (temp, mut app) = fixture();
    let directory = temp.path().join("external-progress");
    let project_before = snapshot(app.project.root());
    app.progress = ProgressSession::from_store(Ok(store(&directory, key(&app))));
    assert!(!directory.exists(), "open must be read-only");
    win(&mut app);
    assert_eq!(app.progress.status(), "Room cleared (saved)");
    let saved = snapshot(&directory);
    assert!(!saved.is_empty());
    assert!(store(&directory, key(&app)).cleared());
    for _ in 0..120 {
        app.step_authoritative(RoomInput::default()).unwrap();
    }
    assert_eq!(snapshot(&directory), saved);
    app.restart().unwrap();
    assert_eq!(app.sim.frame().singleton::<RoomRun>().won, 0);
    assert_eq!(app.sim.frame().singleton::<RoomRun>().key_collected, 0);
    assert_eq!(app.progress.status(), "Room cleared (saved)");
    win(&mut app);
    assert_eq!(
        snapshot(&directory),
        saved,
        "restart/re-clear cannot rewrite a clear record"
    );
    let project = PreparedProject::open(app.project.root()).unwrap();
    let mut reopened = App::new(project).unwrap();
    reopened.progress = ProgressSession::from_store(Ok(store(&directory, key(&reopened))));
    assert_eq!(reopened.progress.status(), "Room cleared (loaded)");
    assert_eq!(reopened.sim.frame().singleton::<RoomRun>().won, 0);
    win(&mut reopened);
    assert_eq!(snapshot(&directory), saved);
    assert_eq!(snapshot(app.project.root()), project_before);
}
#[test]
fn failed_commit_keeps_gameplay_and_does_not_retry_after_restart() {
    let (temp, mut app) = fixture();
    let directory = temp.path().join("external-progress");
    app.progress = ProgressSession::from_store(Ok(store(&directory, key(&app))));
    fs::write(&directory, b"blocked directory sentinel").unwrap();
    win(&mut app);
    assert!(
        app.progress.status().contains("not saved"),
        "{}",
        app.progress.status()
    );
    assert_eq!(fs::read(&directory).unwrap(), b"blocked directory sentinel");
    fs::remove_file(&directory).unwrap();
    let error = app.progress.status().to_owned();
    app.restart().unwrap();
    win(&mut app);
    assert_eq!(app.progress.status(), error);
    assert!(!directory.exists(), "no automatic retry after failure");
}
#[test]
fn incomplete_restart_and_headless_never_submit_progress() {
    let (temp, mut app) = fixture();
    let directory = temp.path().join("external-progress");
    app.progress = ProgressSession::from_store(Ok(store(&directory, key(&app))));
    app.step_authoritative(RoomInput {
        buttons: INTERACT,
        ..Default::default()
    })
    .unwrap();
    assert_eq!(app.sim.frame().singleton::<RoomRun>().key_collected, 1);
    assert_eq!(app.sim.frame().singleton::<RoomRun>().won, 0);
    app.restart().unwrap();
    assert!(!directory.exists());
    headless(
        &app.project,
        60,
        RoomInput {
            buttons: INTERACT,
            ..Default::default()
        },
        None,
    )
    .unwrap();
    assert!(!directory.exists());
    assert_eq!(app.sim.tick(), 0);
}
#[test]
fn observer_refuses_initial_terminal_nonsequential_and_invalid_won_values() {
    let (temp, app) = fixture();
    let directory = temp.path().join("external-progress");
    let mut session = ProgressSession::from_store(Ok(store(&directory, key(&app))));
    for (before_tick, before_won, after_tick, after_won) in [
        (0, 1, 1, 1),
        (1, 0, 1, 1),
        (0, 0, 2, 1),
        (u64::MAX, 0, 0, 1),
        (0, 0, 1, 2),
    ] {
        session.observe(before_tick, before_won, after_tick, after_won);
    }
    assert!(!directory.exists());
    assert_eq!(session.status(), "Room not cleared yet");
}

#[test]
fn isolated_authored_identity_native_app_save_and_relaunch() {
    let (temp, app) = fixture();
    let root = app.project.root().to_owned();
    drop(app);
    let path = root.join("orr.project.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["schema"] = 3.into();
    manifest["progress"] = serde_json::json!({"schema":1,"game_id":"12345678-1234-4234-8234-123456789abc","profile":"room-escape-clear-v1"});
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let before = snapshot(&root);
    let data = temp.path().join("data");
    let mut saved = None;
    for mode in ["win", "reload"] {
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "room_app::progress_tests::isolated_authored_identity_child",
                "--ignored",
                "--nocapture",
            ])
            .env("ORR_ROOM_PROGRESS_TEST_ROOT", &root)
            .env("ORR_ROOM_PROGRESS_TEST_MODE", mode)
            .env("XDG_DATA_HOME", &data)
            .env("HOME", temp.path().join("unused-home"))
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
        assert!(
            String::from_utf8_lossy(&child.stdout).contains("1 passed"),
            "child test must actually run"
        );
        let current = snapshot(&data);
        assert!(!current.is_empty());
        if let Some(saved) = &saved {
            assert_eq!(&current, saved);
        }
        saved = Some(current);
        assert_eq!(snapshot(&root), before);
    }
}

#[test]
#[ignore = "process-isolated child; invoked by isolated_authored_identity_native_app_save_and_relaunch"]
fn isolated_authored_identity_child() {
    let root =
        PathBuf::from(std::env::var_os("ORR_ROOM_PROGRESS_TEST_ROOT").expect("explicit test root"));
    let mode = std::env::var("ORR_ROOM_PROGRESS_TEST_MODE").expect("explicit test mode");
    assert!(matches!(mode.as_str(), "win" | "reload"));
    let data = PathBuf::from(std::env::var_os("XDG_DATA_HOME").unwrap());
    let project = PreparedProject::open_with_progress(
        root,
        crate::room_project::RoomProjectCapabilities {
            lighting: cfg!(feature = "room-lighting"),
            character: cfg!(feature = "room-character"),
        },
        crate::room_project::ProgressSupport::MetadataOnly,
    )
    .unwrap();
    let before_headless = snapshot(&data);
    headless(
        &project,
        60,
        RoomInput {
            buttons: INTERACT,
            ..Default::default()
        },
        None,
    )
    .unwrap();
    assert_eq!(
        snapshot(&data),
        before_headless,
        "metadata headless must not even open the store"
    );
    let mut app = App::new(project).unwrap();
    assert_eq!(app.sim.frame().singleton::<RoomRun>().won, 0);
    if mode == "win" {
        assert!(!data.exists());
        assert_eq!(app.progress.status(), "Room not cleared yet");
        win(&mut app);
        assert_eq!(app.progress.status(), "Room cleared (saved)");
    } else {
        assert_eq!(app.progress.status(), "Room cleared (loaded)");
        win(&mut app);
        assert_eq!(snapshot(&data), before_headless);
    }
}
