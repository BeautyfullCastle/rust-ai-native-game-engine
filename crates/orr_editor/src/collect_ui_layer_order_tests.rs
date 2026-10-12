use super::*;

fn node(id: &str, parent: Option<&str>, kind: Kind) -> Node {
    Node { id: id.into(), parent: parent.map(str::to_owned), kind,
        screen: Screen::Title, anchor: [0, 0], offset: [0, 0], size: [100, 100] }
}
fn button(id: &str, parent: Option<&str>, action: Action) -> Node {
    node(id, parent, Kind::Button { text: id.into(), action })
}
fn fixture() -> Document {
    Document { schema: 1, nodes: vec![
        node("parent", None, Kind::Container),
        node("selected", Some("parent"), Kind::Container),
        button("outside", Some("parent"), Action::Quit),
        button("child", Some("selected"), Action::Play),
        button("last", None, Action::Restart),
    ] }
}
fn ids(document: &Document) -> Vec<&str> {
    document.nodes.iter().map(|node| node.id.as_str()).collect()
}
fn panel(document: Document) -> (tempfile::TempDir, Panel) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("collect.ui.json");
    std::fs::write(&path, document.to_bytes().unwrap()).unwrap();
    let panel = Panel::new(PreparedCollectUi { path, document,
        font: include_bytes!("../../../assets/game_ui_font/OrreryKoreanUI.otf").to_vec() });
    (dir, panel)
}

#[test]
fn front_and_back_preserve_subtree_fields_and_parent_order() {
    let original = fixture();
    let (front, selected) = reorder_subtree(&original, 1, LayerEdge::Front).unwrap();
    assert_eq!(ids(&front), ["parent", "outside", "last", "selected", "child"]);
    assert_eq!(selected, 3);
    let (back, selected) = reorder_subtree(&front, selected, LayerEdge::Back).unwrap();
    assert_eq!(ids(&back), ["parent", "selected", "child", "outside", "last"]);
    assert_eq!(selected, 1);
    for document in [&front, &back] {
        document.validate().unwrap();
        for before in &original.nodes {
            assert_eq!(document.nodes.iter().find(|node| node.id == before.id), Some(before));
        }
    }
}

#[test]
fn other_screens_and_noop_do_not_create_spurious_edits() {
    let mut original = fixture();
    let mut foreign = button("foreign", None, Action::Quit);
    foreign.screen = Screen::Menu;
    original.nodes.insert(2, foreign.clone());
    let (front, index) = reorder_subtree(&original, 1, LayerEdge::Front).unwrap();
    assert_eq!(front.nodes.iter().find(|node| node.id == "foreign"), Some(&foreign));
    assert_eq!(reorder_subtree(&front, index, LayerEdge::Front).unwrap(), (front.clone(), index));
    let only = Document { schema: 1, nodes: vec![foreign, button("only", None, Action::Play)] };
    assert_eq!(reorder_subtree(&only, 1, LayerEdge::Back).unwrap(), (only, 1));
}

#[test]
fn reorder_undo_redo_save_reopen_restore_exact_selection() {
    let original = fixture();
    let (_dir, mut panel) = panel(original.clone());
    panel.selected = Some(1);
    panel.reorder_selected(LayerEdge::Front).unwrap();
    let front = panel.document.clone();
    assert_eq!(panel.selected, Some(3));
    assert!(panel.undo());
    assert_eq!(panel.document, original);
    assert_eq!(panel.selected, Some(1));
    assert!(panel.redo());
    assert_eq!(panel.document, front);
    assert_eq!(panel.selected, Some(3));
    panel.save().unwrap();
    assert_eq!(Document::parse(&std::fs::read(&panel.path).unwrap()).unwrap(), front);
    assert!(panel.undo());
    panel.apply_document(original.clone()).unwrap();
    assert_eq!(panel.redo.len(), 1, "no-op keeps redo");
    let mut invalid = original;
    invalid.nodes[0].size[0] = 0;
    assert!(panel.apply_document(invalid).is_err());
    assert_eq!(panel.redo.len(), 1, "rejected edit keeps redo");
    panel.reorder_selected(LayerEdge::Back).unwrap();
    assert!(panel.redo.is_empty(), "new edit discards redo");
}

#[test]
fn malformed_or_missing_selection_is_atomic_and_history_bounded() {
    let original = fixture();
    assert!(reorder_subtree(&original, 99, LayerEdge::Front).is_err());
    let mut invalid = original.clone();
    invalid.nodes.swap(0, 1);
    assert!(reorder_subtree(&invalid, 0, LayerEdge::Front).is_err());
    let (_dir, mut panel) = panel(original.clone());
    assert!(panel.reorder_selected(LayerEdge::Front).is_err());
    assert_eq!(panel.document, original);
    assert!(panel.undo.is_empty());
    panel.selected = Some(1);
    for _ in 0..40 {
        panel.reorder_selected(LayerEdge::Front).unwrap();
        panel.reorder_selected(LayerEdge::Back).unwrap();
    }
    assert_eq!(panel.undo.len(), MAX_UNDO);
    for _ in 0..MAX_UNDO { assert!(panel.undo()); }
    assert!(!panel.undo());
    for _ in 0..MAX_UNDO { assert!(panel.redo()); }
    assert!(!panel.redo());
}

fn runtime_click(document: Document) -> Option<Action> {
    let mut runtime = CollectUi::new(document,
        include_bytes!("../../../assets/game_ui_font/OrreryKoreanUI.otf").to_vec()).unwrap();
    let input = |events| egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0))),
        events, ..Default::default()
    };
    let (output, _) = runtime.show(input(vec![]), Default::default());
    output.drop_without_applying_deltas();
    let point = egui::pos2(50.0, 50.0);
    let event = |pressed| egui::Event::PointerButton {
        pos: point, button: egui::PointerButton::Primary, pressed, modifiers: Default::default(),
    };
    let (output, action) = runtime.show(input(vec![egui::Event::PointerMoved(point), event(true)]), Default::default());
    output.drop_without_applying_deltas();
    assert_eq!(action, None);
    let (output, action) = runtime.show(input(vec![event(false)]), Default::default());
    output.drop_without_applying_deltas();
    action
}

#[test]
fn actual_widgets_save_reopen_and_runtime_pointer_route_follow_order() {
    use egui::accesskit::Role;
    use egui_kittest::{kittest::Queryable, Harness};
    let original = fixture();
    assert_eq!(runtime_click(original.clone()), Some(Action::Restart));
    let (_dir, panel) = panel(original.clone());
    let mut h = Harness::builder().with_size([1100.0, 1400.0])
        .build_ui_state(|ui, panel: &mut Panel| panel.show(ui, true), panel);
    h.run_steps(3);
    h.get_by_label("Collect UI document").click(); h.run_steps(3);
    h.get_by_role_and_label(Role::ComboBox, "UI node").click(); h.run_steps(3);
    h.get_by_label("selected").click(); h.run_steps(3);
    h.get_by_label("Bring subtree to front").click(); h.run_steps(3);
    let front = h.state().document.clone();
    assert_eq!(ids(&front), ["parent", "outside", "last", "selected", "child"]);
    assert_eq!(runtime_click(front.clone()), Some(Action::Play));
    h.get_by_label("Undo UI edit").click(); h.run_steps(3);
    assert_eq!(h.state().document, original);
    assert_eq!(h.state().selected, Some(1));
    h.get_by_label("Redo UI edit").click(); h.run_steps(3);
    assert_eq!(h.state().document, front);
    assert_eq!(h.state().selected, Some(3));
    h.get_by_label("Save UI document").click(); h.run_steps(3);
    assert!(h.state().error.is_none(), "{:?}", h.state().error);
    let saved = Document::parse(&std::fs::read(&h.state().path).unwrap()).unwrap();
    assert_eq!(saved, front);
    assert_eq!(runtime_click(saved), Some(Action::Play));
    h.get_by_label("Send subtree to back").click(); h.run_steps(3);
    assert_eq!(runtime_click(h.state().document.clone()), Some(Action::Restart));
}

type PanelSnapshot = (Document, Option<usize>, Vec<(Document, Option<String>)>, Vec<(Document, Option<String>)>);
fn snapshot(panel: &Panel) -> PanelSnapshot {
    let history = |entries: &[HistoryEntry]| entries.iter()
        .map(|entry| (entry.document.clone(), entry.selected.clone())).collect();
    (panel.document.clone(), panel.selected, history(&panel.undo), history(&panel.redo))
}
fn selected_id(panel: &Panel) -> Option<&str> {
    panel.selected.and_then(|index| panel.document.nodes.get(index)).map(|node| node.id.as_str())
}

#[test]
fn nested_interleaved_descendants_and_two_foreign_screens_preserve_outside_order() {
    let mut foreign1 = button("foreign1", None, Action::Quit);
    foreign1.screen = Screen::Menu;
    let mut foreign2 = button("foreign2", None, Action::Quit);
    foreign2.screen = Screen::Terminal;
    let original = Document { schema: 1, nodes: vec![
        node("parent", None, Kind::Container),
        node("selected", Some("parent"), Kind::Container),
        foreign1,
        button("outside", Some("parent"), Action::Quit),
        node("nested", Some("selected"), Kind::Container),
        foreign2,
        button("last", None, Action::Restart),
        button("deep", Some("nested"), Action::Play),
        button("child", Some("selected"), Action::Menu),
    ] };
    original.validate().unwrap();
    let (front, selected) = reorder_subtree(&original, 1, LayerEdge::Front).unwrap();
    assert_eq!(ids(&front), ["parent", "foreign1", "outside", "foreign2", "last", "selected", "nested", "deep", "child"]);
    assert_eq!(selected, 5);
    let (back, selected) = reorder_subtree(&front, selected, LayerEdge::Back).unwrap();
    assert_eq!(ids(&back), ["parent", "selected", "nested", "deep", "child", "foreign1", "outside", "foreign2", "last"]);
    assert_eq!(selected, 1);
    for document in [&front, &back] {
        document.validate().unwrap();
        for before in &original.nodes {
            assert_eq!(document.nodes.iter().find(|node| node.id == before.id), Some(before));
        }
        let outside: Vec<_> = document.nodes.iter().filter(|node| !["selected", "nested", "deep", "child"].contains(&node.id.as_str())).map(|node| node.id.as_str()).collect();
        assert_eq!(outside, ["parent", "foreign1", "outside", "foreign2", "last"]);
    }
}

#[test]
fn panel_edge_noops_and_failed_candidates_preserve_complete_history_and_selection() {
    let (_dir, mut panel) = panel(fixture());
    panel.selected = Some(1);
    panel.reorder_selected(LayerEdge::Front).unwrap();
    panel.reorder_selected(LayerEdge::Back).unwrap();
    assert!(panel.undo());
    assert!(!panel.undo.is_empty() && !panel.redo.is_empty());
    let before = snapshot(&panel);
    panel.reorder_selected(LayerEdge::Front).unwrap();
    assert_eq!(snapshot(&panel), before);
    let mut invalid = panel.document.clone();
    invalid.nodes[0].size = [0, 0];
    assert!(panel.apply_document(invalid).is_err());
    assert_eq!(snapshot(&panel), before);
    assert!(panel.redo());
    panel.reorder_selected(LayerEdge::Front).unwrap();
    assert!(panel.undo());
    let before = snapshot(&panel);
    panel.reorder_selected(LayerEdge::Back).unwrap();
    assert_eq!(snapshot(&panel), before);
}

#[test]
fn actual_add_remove_duplicate_widgets_undo_redo_restore_selected_ids() {
    use egui::accesskit::Role;
    use egui_kittest::{kittest::Queryable, Harness};
    let (_dir, panel) = panel(fixture());
    let mut h = Harness::builder().with_size([1100.0, 1400.0])
        .build_ui_state(|ui, panel: &mut Panel| panel.show(ui, true), panel);
    h.run_steps(3);
    h.get_by_label("Collect UI document").click(); h.run_steps(3);
    h.get_by_role_and_label(Role::ComboBox, "UI node").click(); h.run_steps(3);
    h.get_by_label("selected").click(); h.run_steps(3);
    let original = h.state().document.clone();
    for command in ["Add label", "Duplicate node and descendants", "Remove node and descendants"] {
        assert_eq!(h.state().document, original);
        assert_eq!(selected_id(h.state()), Some("selected"));
        h.get_by_label(command).click(); h.run_steps(3);
        assert!(h.state().error.is_none(), "{:?}", h.state().error);
        let changed = h.state().document.clone();
        let selected = selected_id(h.state()).map(str::to_owned);
        match command {
            "Add label" => assert_eq!(selected.as_deref(), Some("ui_1")),
            "Duplicate node and descendants" => assert_eq!(selected.as_deref(), Some("ui_copy_1")),
            _ => assert_eq!(selected, None),
        }
        assert_ne!(changed, original);
        h.get_by_label("Undo UI edit").click(); h.run_steps(3);
        assert_eq!(h.state().document, original);
        assert_eq!(selected_id(h.state()), Some("selected"));
        h.get_by_label("Redo UI edit").click(); h.run_steps(3);
        assert_eq!(h.state().document, changed);
        assert_eq!(selected_id(h.state()), selected.as_deref());
        h.get_by_label("Undo UI edit").click(); h.run_steps(3);
    }
}

#[test]
fn rejected_add_and_remove_widget_commits_preserve_selection_and_both_stacks() {
    use egui::accesskit::Role;
    use egui_kittest::{kittest::Queryable, Harness};
    let document = Document { schema: 1, nodes: vec![
        node("root", None, Kind::Container),
        node("level2", Some("root"), Kind::Container),
        node("level3", Some("level2"), Kind::Container),
        node("level4", Some("level3"), Kind::Container),
        button("outside", None, Action::Play),
    ] };
    let (_dir, mut panel) = panel(document);
    panel.selected = Some(3);
    panel.reorder_selected(LayerEdge::Front).unwrap();
    panel.reorder_selected(LayerEdge::Back).unwrap();
    assert!(panel.undo());
    assert!(!panel.undo.is_empty() && !panel.redo.is_empty());
    let mut h = Harness::builder().with_size([1100.0, 1400.0])
        .build_ui_state(|ui, panel: &mut Panel| panel.show(ui, true), panel);
    h.run_steps(3);
    h.get_by_label("Collect UI document").click(); h.run_steps(3);
    // Select through the actual picker even though the initial history selected it.
    h.get_by_role_and_label(Role::ComboBox, "UI node").click(); h.run_steps(3);
    h.get_by_label("level4").click(); h.run_steps(3);
    let before = snapshot(h.state());
    h.get_by_label("Add label").click(); h.run_steps(3);
    assert!(h.state().error.as_ref().is_some_and(|error| error.contains("depth")));
    assert_eq!(snapshot(h.state()), before);
    // A valid removal is monotonic for schema validation. Deliberately corrupt
    // only the test-owned font to force the existing precommit admission fence.
    // This is defensive failure injection, not a normal admitted-project state.
    h.state_mut().font.clear();
    h.get_by_label("Remove node and descendants").click(); h.run_steps(3);
    assert!(h.state().error.is_some());
    assert_eq!(snapshot(h.state()), before);
}

#[test]
fn no_selection_and_play_widgets_cannot_mutate_history_selection_or_disk() {
    use egui_kittest::{kittest::Queryable, Harness};
    struct State { panel: Panel, editable: bool }
    let (_dir, panel) = panel(fixture());
    let mut h = Harness::builder().with_size([1100.0, 1400.0])
        .build_ui_state(|ui, state: &mut State| state.panel.show(ui, state.editable),
            State { panel, editable: true });
    h.run_steps(3);
    h.get_by_label("Collect UI document").click(); h.run_steps(3);
    let before = snapshot(&h.state().panel);
    for label in ["Bring subtree to front", "Send subtree to back"] {
        h.get_by_label(label).click(); h.run_steps(3);
        assert_eq!(snapshot(&h.state().panel), before);
    }
    // Prepare both history directions before testing the read-only UI fence.
    h.state_mut().panel.selected = Some(1);
    h.state_mut().panel.reorder_selected(LayerEdge::Front).unwrap();
    h.state_mut().panel.reorder_selected(LayerEdge::Back).unwrap();
    assert!(h.state_mut().panel.undo());
    h.state_mut().editable = false; h.run_steps(3);
    let before = snapshot(&h.state().panel);
    let disk = std::fs::read(&h.state().panel.path).unwrap();
    for label in ["Bring subtree to front", "Send subtree to back", "Undo UI edit", "Redo UI edit",
        "Save UI document", "Duplicate node and descendants", "Add label", "Remove node and descendants"] {
        h.get_by_label(label).click(); h.run_steps(3);
        assert_eq!(snapshot(&h.state().panel), before);
        assert_eq!(std::fs::read(&h.state().panel.path).unwrap(), disk);
    }
}
