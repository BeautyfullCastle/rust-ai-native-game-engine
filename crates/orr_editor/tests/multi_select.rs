//! Public multi-selection and batch-move behavior against a real Arena ERP host.
#![allow(clippy::disallowed_types)]

mod common;

use common::{
    arena::{proposal, ArenaHost, SCENE as ARENA_SCENE},
    target_named,
};
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use orr_editor::game::EditorGame;
use orr_editor::{Editor, EditorApp, Mode, Target};
use orr_fp::{FPVec2, FP};
use orr_remote::ErpClient;
use serde_json::{json, Value as J};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

fn guid_text(target: &Target) -> String {
    target.param()
}

fn position_of(client: &mut ErpClient, guid: &str) -> J {
    client
        .call(
            "world.get",
            json!({"entity":guid,"component":"Position","path":"pos"}),
        )
        .unwrap()["value"]
        .clone()
}

fn wait_for_down(editor: &mut Editor) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while editor.down().is_none() {
        assert!(
            Instant::now() < deadline,
            "host disconnect was not observed"
        );
        editor.pump();
        thread::sleep(Duration::from_millis(2));
    }
}

fn wait_for_ui(
    harness: &mut Harness<'_, EditorApp>,
    label: &str,
    mut ready: impl FnMut(&Editor) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !ready(&harness.state().editor) {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {label}: {:?}",
            harness.state().editor.status()
        );
        harness.run_steps(1);
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn selected_guids_nudge_as_one_history_entry_and_undo_atomically() {
    let host = ArenaHost::start(false);
    let mut observer = host.client();
    let mut editor = Editor::attach(&host.url, None).unwrap();
    editor.sync();
    let hero = target_named(&editor, "hero");
    let target = target_named(&editor, "target");
    let hero_guid = guid_text(&hero);
    let target_guid = guid_text(&target);
    let original_checksum = editor.checksum();
    let original_hero = position_of(&mut observer, &hero_guid);
    let original_target = position_of(&mut observer, &target_guid);

    editor.select(Some(hero.clone()));
    assert!(editor.toggle_selection(target.clone()));
    assert_eq!(
        editor.selection(),
        Some(&target),
        "Ctrl-toggle makes the newly added GUID the primary for the old single-selection API"
    );
    assert_eq!(
        editor.selected_guids(),
        &[hero_guid_value(&hero), hero_guid_value(&target)]
    );
    assert!(editor.is_selected(&hero));
    assert!(editor.is_selected(&target));
    assert!(editor.can_nudge_selection());

    let history_before = editor.history().entries.len();
    assert!(editor.nudge_selected(FPVec2::new(FP::from_int(5), FP::from_int(7))));
    editor.sync();
    assert_eq!(position_of(&mut observer, &hero_guid), json!([-295, 7]));
    assert_eq!(position_of(&mut observer, &target_guid), json!([305, 7]));
    assert_eq!(
        editor.history().entries.len(),
        history_before + 1,
        "one batch is one undo entry"
    );
    assert_eq!(
        editor.history().entries.last().unwrap().label,
        "move selected entities"
    );
    assert!(editor.undo());
    editor.sync();
    assert_eq!(position_of(&mut observer, &hero_guid), original_hero);
    assert_eq!(position_of(&mut observer, &target_guid), original_target);
    assert_eq!(editor.checksum(), original_checksum);
    assert_eq!(
        editor.history().entries.len(),
        history_before + 1,
        "undo preserves the accepted entry for redo"
    );
    assert!(editor.history().entries.last().unwrap().undone);
    assert!(editor.history().can_redo);
}

#[test]
fn zero_nudge_and_selection_operations_preserve_expected_history() {
    let host = ArenaHost::start(false);
    let mut editor = Editor::attach(&host.url, None).unwrap();
    editor.sync();
    let hero = target_named(&editor, "hero");
    let target = target_named(&editor, "target");
    editor.select(Some(hero.clone()));
    assert!(editor.toggle_selection(target.clone()));
    let selected = editor.selected_guids().to_vec();
    let history = editor.history().entries.clone();
    let checksum = editor.checksum();

    assert!(!editor.nudge_selected(FPVec2::ZERO));
    assert_eq!(
        editor.history().entries,
        history,
        "zero displacement is not an undoable edit"
    );
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.selected_guids(), selected.as_slice());

    assert!(editor.toggle_selection(target.clone()));
    assert_eq!(editor.selected_guids(), &[hero_guid_value(&hero)]);
    assert!(editor.toggle_selection(hero.clone()));
    assert!(editor.selected_guids().is_empty());
    assert!(editor.selection().is_none());
    editor.select(Some(target.clone()));
    assert_eq!(editor.selected_guids(), &[hero_guid_value(&target)]);
    editor.select(None);
    assert!(editor.selected_guids().is_empty());
    assert!(editor.selection().is_none());
    assert_eq!(
        editor.history().entries,
        history,
        "selection never changes document history"
    );
}

fn hero_guid_value(target: &Target) -> orr_reflect::Guid {
    match target {
        Target::Guid(guid) => guid.clone(),
        Target::Entity(_) => panic!("Arena fixture GUID"),
    }
}

#[test]
fn scene_replacement_clears_selection_and_modes_refuse_batch_edits() {
    let host = ArenaHost::start(false);
    let mut observer = host.client();
    let mut editor = Editor::attach(&host.url, None).unwrap();
    editor.sync();
    let hero = target_named(&editor, "hero");
    let target = target_named(&editor, "target");
    editor.select(Some(hero.clone()));
    assert!(editor.toggle_selection(target.clone()));

    let preview = proposal(&mut observer, -200, 10);
    editor.sync();
    assert!(editor.set_preview(Some(preview)));
    editor.sync();
    assert!(!editor.can_nudge_selection());
    assert!(!editor.nudge_selected(FPVec2::new(FP::from_int(1), FP::ZERO)));
    assert!(editor.set_preview(None));

    assert!(editor.start_play());
    assert_eq!(editor.mode(), Mode::Play);
    assert!(!editor.can_nudge_selection());
    assert!(!editor.nudge_selected(FPVec2::new(FP::from_int(1), FP::ZERO)));
    editor.pause();
    editor.sync();
    assert_eq!(
        editor.mode(),
        Mode::Play,
        "paused play is still not document-edit mode"
    );
    assert!(!editor.can_nudge_selection());
    assert!(!editor.nudge_selected(FPVec2::new(FP::from_int(1), FP::ZERO)));
    assert!(editor.stop().is_some());
    editor.sync();
    assert_eq!(editor.mode(), Mode::Edit);

    observer
        .call("tx.begin", json!({"label":"foreign edit"}))
        .unwrap();
    editor.sync();
    assert!(
        !editor.can_nudge_selection(),
        "a foreign open transaction fences batch edits"
    );
    assert!(!editor.nudge_selected(FPVec2::new(FP::from_int(1), FP::ZERO)));
    observer.call("tx.rollback", json!({})).unwrap();
    editor.sync();

    observer
        .call(
            "scene.load",
            json!({"text":include_str!("../../../scenes/arena_blank.scene.yaml")}),
        )
        .unwrap();
    editor.sync();
    assert!(
        editor.selected_guids().is_empty(),
        "authoritative scene replacement clears GUID selection"
    );
    assert!(editor.selection().is_none());
    assert!(!editor.can_nudge_selection());

    observer
        .call("scene.load", json!({"text":ARENA_SCENE}))
        .unwrap();
    editor.sync();
    let hero = target_named(&editor, "hero");
    let target = target_named(&editor, "target");
    editor.select(Some(hero));
    assert!(editor.toggle_selection(target));
    let scene_path =
        std::env::temp_dir().join(format!("orr_multi_select_{}.yaml", std::process::id()));
    std::fs::write(
        &scene_path,
        include_str!("../../../scenes/arena_blank.scene.yaml"),
    )
    .unwrap();
    assert!(editor.open_path(&scene_path));
    assert!(
        editor.selected_guids().is_empty(),
        "opening a replacement scene through the editor clears the old document selection"
    );
    assert!(editor.selection().is_none());
    let _ = std::fs::remove_file(scene_path);
}

#[test]
fn viewer_host_is_read_only_and_fixed_point_overflow_has_no_partial_effect() {
    let host = ArenaHost::start(false);
    let mut observer = host.client();
    let mut editor = Editor::attach(&host.url, None).unwrap();
    editor.sync();
    let hero = target_named(&editor, "hero");
    let target = target_named(&editor, "target");
    let hero_guid = guid_text(&hero);
    let target_guid = guid_text(&target);
    editor.select(Some(hero));
    assert!(editor.toggle_selection(target));
    let checksum = editor.checksum();
    let history = editor.history().entries.clone();
    let before_hero = position_of(&mut observer, &hero_guid);
    let before_target = position_of(&mut observer, &target_guid);

    assert!(!editor.nudge_selected(FPVec2::new(FP::from_raw(i64::MAX), FP::ZERO)));
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.history().entries, history);
    assert_eq!(position_of(&mut observer, &hero_guid), before_hero);
    assert_eq!(position_of(&mut observer, &target_guid), before_target);

    let viewer = ArenaHost::start(true);
    let mut replay_editor = Editor::attach(&viewer.url, None).unwrap();
    replay_editor.sync();
    let viewer_hero = target_named(&replay_editor, "hero");
    let viewer_target = target_named(&replay_editor, "target");
    replay_editor.select(Some(viewer_hero));
    assert!(replay_editor.toggle_selection(viewer_target));
    assert!(!replay_editor.can_nudge_selection());
    assert!(!replay_editor.nudge_selected(FPVec2::new(FP::from_int(1), FP::ZERO)));
}

#[test]
fn disconnected_host_refuses_mutation_without_fabricating_history() {
    let mut host = ArenaHost::start(false);
    let mut editor = Editor::attach(&host.url, None).unwrap();
    editor.sync();
    let hero = target_named(&editor, "hero");
    let target = target_named(&editor, "target");
    editor.select(Some(hero));
    assert!(editor.toggle_selection(target));
    let history = editor.history().entries.clone();
    host.stop();
    wait_for_down(&mut editor);
    assert!(!editor.can_nudge_selection());
    assert!(!editor.nudge_selected(FPVec2::new(FP::from_int(1), FP::ZERO)));
    assert_eq!(editor.history().entries, history);
}

struct UiScene(PathBuf);

impl UiScene {
    fn arena() -> Self {
        let path = std::env::temp_dir().join(format!(
            "orr_multi_select_ui_{}.scene.yaml",
            std::process::id()
        ));
        std::fs::write(&path, ARENA_SCENE).expect("write Arena UI fixture");
        Self(path)
    }
}

impl Drop for UiScene {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn hierarchy_ctrl_click_and_escape_drive_multi_move_and_one_undo() {
    let scene = UiScene::arena();
    let editor =
        Editor::open_game(&scene.0, EditorGame::Arena).expect("start local Arena UI fixture");
    let mut harness = Harness::builder()
        .with_size([1400.0, 850.0])
        .build_eframe(|_| EditorApp::new(editor, None));
    harness.run_steps(3);

    harness.get_by_label("hero").click();
    harness.run_steps(2);
    assert_eq!(harness.state().editor.selected_guids().len(), 1);
    let hero = target_named(&harness.state().editor, "hero");
    let target = target_named(&harness.state().editor, "target");
    harness
        .get_by_label("target")
        .click_modifiers(Modifiers::CTRL);
    harness.run_steps(2);
    assert!(harness.state().editor.is_selected(&hero));
    assert!(harness.state().editor.is_selected(&target));
    assert_eq!(harness.state().editor.selected_guids().len(), 2);
    assert_eq!(harness.state().editor.selection(), Some(&target));

    harness.key_press_modifiers(Modifiers::NONE, Key::Escape);
    harness.run_steps(2);
    assert!(
        harness.state().editor.selected_guids().is_empty(),
        "Escape clears the GUID set"
    );
    harness.get_by_label("hero").click();
    harness.run_steps(2);
    harness
        .get_by_label("target")
        .click_modifiers(Modifiers::CTRL);
    harness.run_steps(2);
    let before_history = harness.state().editor.history().entries.len();
    harness.get_by_label("Move right").click();
    wait_for_ui(&mut harness, "one multi-move history entry", |editor| {
        editor.history().entries.len() == before_history + 1
    });
    let pos = |editor: &mut Editor, guid: &Target| {
        editor
            .host_call(
                "world.get",
                json!({"entity":guid.param(),"component":"Position","path":"pos"}),
            )
            .unwrap()["value"]
            .clone()
    };
    assert_eq!(
        pos(&mut harness.state_mut().editor, &hero),
        json!([-299, 0])
    );
    assert_eq!(
        pos(&mut harness.state_mut().editor, &target),
        json!([301, 0])
    );
    assert_eq!(
        harness.state().editor.history().entries.len(),
        before_history + 1
    );

    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    wait_for_ui(&mut harness, "undo of multi-move", |editor| {
        editor
            .history()
            .entries
            .last()
            .is_some_and(|entry| entry.undone)
    });
    assert_eq!(
        pos(&mut harness.state_mut().editor, &hero),
        json!([-300, 0])
    );
    assert_eq!(
        pos(&mut harness.state_mut().editor, &target),
        json!([300, 0])
    );
    assert_eq!(
        harness.state().editor.history().entries.len(),
        before_history + 1
    );
    assert!(
        harness
            .state()
            .editor
            .history()
            .entries
            .last()
            .unwrap()
            .undone
    );
}
