use super::{request_matches, Completion, Panel, Request};
use crate::{game::EditorGame, mesh_workspace};
use std::{fs, path::PathBuf, sync::mpsc};

const EMPTY_SCENE: &[u8] = b"schema: orr.scene/1\nsingletons: {}\nentities: {}\n";

#[test]
fn duplicate_panel_starts_idle_and_hides_no_result_without_a_worker() {
    let panel = Panel::default();
    assert!(!panel.busy());
    assert!(!panel.visible());
    assert!(panel.status.is_empty());
}

#[test]
fn open_refuses_pending_or_cancelled_work() {
    let mut panel = Panel::default();
    assert!(panel.open());
    let (_sender, receiver) = mpsc::channel::<Completion>();
    panel.pending = Some(receiver);
    assert!(!panel.open());
    assert!(panel.busy());
    panel.pending = None;
    panel.cancel_pending = true;
    assert!(!panel.open());
    panel.cancel_pending = false;
    assert!(panel.open());
}

#[test]
fn completion_requires_the_original_source_epoch_and_destination() {
    let request = Request {
        source: PathBuf::from(r"C:\project\scene.scene.yaml"),
        checksum: 11,
        epoch: 17,
        destination: PathBuf::from(r"C:\project-copy"),
    };
    let current = (
        EditorGame::MeshNavigation,
        request.source.clone(),
        request.checksum,
    );
    assert!(request_matches(
        &request,
        &current,
        request.epoch,
        &request.destination
    ));
    assert!(!request_matches(
        &request,
        &(current.0, current.1.clone(), current.2 + 1),
        request.epoch,
        &request.destination
    ));
    assert!(!request_matches(
        &request,
        &current,
        request.epoch + 1,
        &request.destination
    ));
    assert!(!request_matches(
        &request,
        &current,
        request.epoch,
        PathBuf::from(r"C:\different-copy").as_path()
    ));
}

#[test]
fn poll_publishes_a_valid_copy_without_mutating_the_source_editor() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = root.join("source.scene.yaml");
    fs::write(&source, EMPTY_SCENE).unwrap();
    let mut editor = mesh_workspace::prepare(&source, mesh_workspace::Intent::Open).unwrap();
    editor.sync();
    let (_, source_path, checksum) = editor.mesh_workspace_source_stamp().unwrap();
    let source_checksum = editor.checksum();
    let history = editor.history().clone();
    let destination = root.join("copy");
    let plan = super::super::copy::Plan::prepare(&source_path, &destination).unwrap();
    let request = Request {
        source: source_path,
        checksum,
        epoch: editor.scene_load_epoch(),
        destination: destination.clone(),
    };
    let mut panel = Panel {
        open: true,
        destination: destination.display().to_string(),
        ..Default::default()
    };
    panel.pending = Some(ready_channel(Completion {
        request,
        result: Ok(plan),
    }));

    let copied_scene = panel.poll(&mut editor, None).unwrap();

    assert_eq!(copied_scene, destination.join("source.scene.yaml"));
    assert_eq!(fs::read(copied_scene).unwrap(), EMPTY_SCENE);
    assert_eq!(editor.checksum(), source_checksum);
    assert_eq!(editor.history(), &history);
}

#[test]
fn closed_blocked_stale_and_changed_requests_are_discarded() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = root.join("source.scene.yaml");
    fs::write(&source, EMPTY_SCENE).unwrap();
    let mut editor = mesh_workspace::prepare(&source, mesh_workspace::Intent::Open).unwrap();
    editor.sync();
    let (_, source_path, checksum) = editor.mesh_workspace_source_stamp().unwrap();
    let epoch = editor.scene_load_epoch();

    let closed_destination = root.join("closed-copy");
    let mut closed_panel =
        panel_with_completion(&source_path, checksum, epoch, &closed_destination);
    closed_panel.open = false;
    assert!(closed_panel.poll(&mut editor, None).is_none());
    assert!(!closed_destination.exists());

    let blocked_destination = root.join("blocked-copy");
    let mut blocked_panel =
        panel_with_completion(&source_path, checksum, epoch, &blocked_destination);
    assert!(blocked_panel
        .poll(&mut editor, Some("editor has pending work"))
        .is_none());
    assert!(!blocked_destination.exists());

    let stale_destination = root.join("stale-copy");
    let mut stale_panel =
        panel_with_completion(&source_path, checksum, epoch + 1, &stale_destination);
    assert!(stale_panel.poll(&mut editor, None).is_none());
    assert!(!stale_destination.exists());

    let changed_destination = root.join("changed-copy");
    let mut changed_panel =
        panel_with_completion(&source_path, checksum, epoch, &changed_destination);
    changed_panel.destination = root.join("different-copy").display().to_string();
    assert!(changed_panel.poll(&mut editor, None).is_none());
    assert!(!changed_destination.exists());
}

#[test]
fn disconnected_worker_is_reported_without_publishing() {
    let mut panel = Panel {
        open: true,
        cancel_pending: true,
        ..Default::default()
    };
    let (sender, receiver) = mpsc::channel::<Completion>();
    panel.pending = Some(receiver);
    drop(sender);
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = root.join("missing.scene.yaml");
    fs::write(&source, EMPTY_SCENE).unwrap();
    let mut editor = mesh_workspace::prepare(&source, mesh_workspace::Intent::Open).unwrap();

    assert!(panel.poll(&mut editor, None).is_none());
    assert!(!panel.busy());
    assert!(panel.status.contains("worker disconnected"));
    assert!(panel.open());
}

fn ready_channel(completion: Completion) -> mpsc::Receiver<Completion> {
    let (sender, receiver) = mpsc::channel();
    let _ = sender.send(completion);
    receiver
}

fn panel_with_completion(
    source: &std::path::Path,
    checksum: u64,
    epoch: u64,
    destination: &std::path::Path,
) -> Panel {
    let plan = super::super::copy::Plan::prepare(source, destination).unwrap();
    let request = Request {
        source: source.to_path_buf(),
        checksum,
        epoch,
        destination: destination.to_path_buf(),
    };
    let mut panel = Panel {
        open: true,
        destination: destination.display().to_string(),
        ..Default::default()
    };
    panel.pending = Some(ready_channel(Completion {
        request,
        result: Ok(plan),
    }));
    panel
}
