#![cfg(all(windows, target_arch = "x86_64", feature = "windows-projects"))]
#![allow(clippy::disallowed_types)]
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::windows_project_panel::Panel;

struct State {
    panel: Panel,
    calls: usize,
    blocked: Option<String>,
    authorize: bool,
}
fn harness() -> Harness<'static, State> {
    let mut panel = Panel::default();
    panel.open = true;
    Harness::builder()
        .with_size([1100.0, 800.0])
        .build_ui_state(
            |ui, s: &mut State| {
                s.panel.show(ui.ctx(), s.blocked.as_deref(), || {
                    s.calls += 1;
                    if s.authorize {
                        Ok(())
                    } else {
                        Err("test authority refusal".into())
                    }
                });
            },
            State {
                panel,
                calls: 0,
                blocked: None,
                authorize: false,
            },
        )
}
#[test]
fn empty_paths_never_request_authority_or_start_workers() {
    let mut h = harness();
    h.run_steps(3);
    for label in ["Create Room project", "Open Room project"] {
        h.get_by_label(label).click();
        h.run_steps(3);
    }
    assert_eq!(h.state().calls, 0);
    assert!(!h.state().panel.busy());
    assert!(h.state().panel.status.is_empty());
    h.get_by_label("Enter a new Room project directory to enable Create.");
    h.get_by_label("Enter an existing Room project directory to enable Open.");
}

#[test]
fn path_requirements_are_action_specific_and_authority_still_applies() {
    let mut h = harness();
    h.state_mut().panel.output = "C:\\new-room".into();
    h.run_steps(3);
    h.get_by_label("Open Room project").click();
    h.run_steps(2);
    assert_eq!(h.state().calls, 0);
    h.get_by_label("Create Room project").click();
    h.run_steps(2);
    assert_eq!(h.state().calls, 1);
    assert!(h.state().panel.status.contains("test authority refusal"));
    assert!(!h.state().panel.busy());
    h.state_mut().panel.output.clear();
    h.state_mut().panel.root = "C:\\existing-room".into();
    h.run_steps(3);
    h.get_by_label("Create Room project").click();
    h.run_steps(2);
    assert_eq!(h.state().calls, 1);
    h.get_by_label("Open Room project").click();
    h.run_steps(2);
    assert_eq!(h.state().calls, 2);
    assert!(!h.state().panel.busy());
}

#[test]
fn native_byte_limit_and_host_blocker_prevent_work() {
    let mut h = harness();
    h.state_mut().panel.output = "\u{00e9}".repeat(2049);
    h.state_mut().panel.root = "x".repeat(4097);
    h.run_steps(3);
    for label in ["Create Room project", "Open Room project"] {
        h.get_by_label(label).click();
        h.run_steps(2);
    }
    assert_eq!(h.state().calls, 0);
    h.state_mut().panel.output = "C:\\new-room".into();
    h.state_mut().panel.root = "C:\\old-room".into();
    h.state_mut().blocked = Some("Save current scene first".into());
    h.run_steps(3);
    for label in ["Create Room project", "Open Room project"] {
        h.get_by_label(label).click();
        h.run_steps(2);
    }
    assert_eq!(h.state().calls, 0);
    assert!(!h.state().panel.busy());
    h.state_mut().panel.open = false;
    h.run_steps(2);
    h.state_mut().panel.open = true;
    h.run_steps(3);
    assert_eq!(h.state().panel.output, "C:\\new-room");
    assert_eq!(h.state().calls, 0);
}

fn wait_worker(h: &mut Harness<'_, State>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        h.run_steps(2);
        if !h.state().panel.status.contains("background") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "worker timeout: {}",
            h.state().panel.status
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn valid_create_and_reopen_use_existing_admission_and_never_overwrite() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Room 프로젝트");
    let mut h = harness();
    h.state_mut().authorize = true;
    h.state_mut().panel.output = root.display().to_string();
    h.run_steps(3);
    h.get_by_label("Create Room project").click();
    h.run_steps(2);
    wait_worker(&mut h);
    let created = h
        .state_mut()
        .panel
        .take_ready()
        .expect("created and admitted project");
    assert_eq!(created, std::fs::canonicalize(&root).unwrap());
    let prepared = orr_sample::windows_project::open_room_project(&root).unwrap();
    let checksum = prepared.scene().frame().checksum();
    let manifest = std::fs::read(root.join("orr.project.json")).unwrap();
    h.run_steps(3);
    h.get_by_label("Create Room project").click();
    h.run_steps(2);
    wait_worker(&mut h);
    assert!(h.state_mut().panel.take_ready().is_none());
    assert!(h.state().panel.status.contains("failed"));
    assert_eq!(
        std::fs::read(root.join("orr.project.json")).unwrap(),
        manifest
    );
    assert_eq!(
        orr_sample::windows_project::open_room_project(&root)
            .unwrap()
            .scene()
            .frame()
            .checksum(),
        checksum
    );
    h.get_by_label("Open Room project").click();
    h.run_steps(2);
    wait_worker(&mut h);
    assert_eq!(h.state_mut().panel.take_ready(), Some(created));
    assert_eq!(h.state().calls, 3);
}
