use super::*;
use orr_remote::{Auth, ServerConfig};

fn editor() -> Editor {
    let mut editor = Editor::open(&default_scene_path()).expect("open PhysGame fixture");
    editor.sync();
    editor
}

#[test]
fn clean_physgame_returns_authoritative_source_stamp() {
    let mut editor = editor();
    let (path, checksum) = editor
        .windows_project_source_stamp()
        .expect("clean source stamp");

    assert_eq!(path, default_scene_path());
    assert_eq!(checksum, editor.checksum());
}

#[test]
fn dirty_physgame_cannot_be_used_as_a_project_source() {
    let mut editor = editor();
    editor.select_named("body_05");
    editor.sync();
    assert!(editor.set_field(
        &Owner::Component("orr_physics::Body".into()),
        "pos.x",
        Value::Fixed(FP::from_int(19)),
    ));

    assert!(editor.windows_project_source_stamp().is_err());
}

#[test]
fn play_mode_cannot_be_used_as_a_project_source() {
    let mut editor = editor();
    assert!(editor.start_play());
    editor.sync();

    assert!(editor.windows_project_source_stamp().is_err());
}

#[test]
fn open_transaction_cannot_be_used_as_a_project_source() {
    let mut editor = editor();
    editor
        .call("tx.begin", json!({"label": "project source guard"}))
        .unwrap();
    editor.sync();

    assert!(editor.windows_project_source_stamp().is_err());

    editor.call("tx.rollback", J::Null).unwrap();
}

#[test]
fn unsupported_games_cannot_be_used_as_a_project_source() {
    let mut editor = editor();
    editor.backend.game = EditorGame::Yard3D;

    assert!(editor.windows_project_source_stamp().is_err());
}

#[test]
fn erp_listener_cannot_be_replaced_by_project_open() {
    let spec = HostSpec::Local {
        scene: default_scene_path(),
        listen: Some(ServerConfig::new(Auth::DevNoAuth)),
        debug_hooks: false,
    };
    let mut editor = Editor::start(&spec).expect("start ERP-listening fixture");
    editor.sync();
    assert!(editor.erp_status().is_some());

    assert!(editor.windows_project_source_stamp().is_err());
}

#[test]
fn open_proposals_cannot_be_used_as_a_project_source() {
    let mut editor = editor();
    editor
        .call("proposal.begin", json!({"label": "project source guard"}))
        .unwrap();

    assert!(editor.windows_project_source_stamp().is_err());
}
