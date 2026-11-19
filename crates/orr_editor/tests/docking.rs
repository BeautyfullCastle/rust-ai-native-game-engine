//! Single-window docking and the user's layout settings, driven through the real `EditorApp`.
//!
//! Screenshots are written only when `ORR_DOCK_SHOTS` names a file prefix, for example
//! `C:\tmp\dock_`, and a GPU is available.
#![allow(clippy::disallowed_types)]
#![allow(clippy::float_arithmetic)]

mod common;

use std::path::Path;

use common::*;
use egui::accesskit::Role;
use egui::{Event, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use orr_editor::layout::{LayoutStore, Panel};
use orr_editor::EditorApp;

const SIZE: [f32; 2] = [1280.0, 800.0];

fn shots() -> Option<std::ffi::OsString> {
    std::env::var_os("ORR_DOCK_SHOTS").filter(|_| common::gpu().is_some())
}

/// A new editor window. `settings` is the layout folder, as the real editor takes it from the user's profile.
fn app(settings: Option<&Path>) -> Harness<'static, EditorApp> {
    let dir = settings.map(Path::to_path_buf);
    let build = move |app: &mut EditorApp| {
        if let Some(dir) = &dir {
            app.ui.dock.attach(LayoutStore::at(dir));
        }
    };
    let builder = Harness::builder().with_size(SIZE).with_step_dt(1.0 / 60.0);
    let mut h = if shots().is_some() {
        builder.wgpu().build_eframe(move |cc| {
            let mut app = EditorApp::new(demo_editor(), cc.wgpu_render_state.clone());
            build(&mut app);
            app
        })
    } else {
        builder.build_eframe(move |_cc| {
            let mut app = EditorApp::new(demo_editor(), None);
            build(&mut app);
            app
        })
    };
    settle(&mut h);
    h
}

fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}

fn shot(h: &mut Harness<'_, EditorApp>, name: &str) {
    let Some(prefix) = shots() else { return };
    h.run_steps(2);
    let image = h.render().expect("render the editor window");
    let mut path = prefix;
    path.push(format!("{name}.png"));
    image.save(&path).expect("write the screenshot");
}

fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

/// Opens the Window menu. A click on a check box keeps the menu open, so look first.
fn open_window_menu(h: &mut Harness<'_, EditorApp>) {
    if !present(h, "Layout ⏵") {
        click(h, "Window");
    }
}

fn click_role(h: &mut Harness<'_, EditorApp>, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).click();
    h.run_steps(3);
}

fn rect_of(h: &Harness<'_, EditorApp>, label: &str) -> Rect {
    h.get_by_label(label).rect()
}

fn present(h: &Harness<'_, EditorApp>, label: &str) -> bool {
    h.query_all_by_label(label).next().is_some()
}

fn viewport(h: &Harness<'_, EditorApp>) -> Rect {
    h.state().ui.viewport_rect.expect("viewport")
}

fn press(h: &mut Harness<'_, EditorApp>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

/// Presses at `from` and moves to `to` in small steps. The button stays down.
fn drag_to(h: &mut Harness<'_, EditorApp>, from: Pos2, to: Pos2) {
    h.event(Event::PointerMoved(from));
    h.run_steps(2);
    press(h, from, true);
    for step in 1..=10 {
        let at = from + (to - from) * (step as f32 / 10.0);
        h.event(Event::PointerMoved(at));
        h.run_steps(2);
    }
}

fn drag_title(h: &mut Harness<'_, EditorApp>, title: &str, to: Pos2) {
    let from = rect_of(h, title).center();
    drag_to(h, from, to);
}

/// The point in the lower part of the viewport, where a drop makes a split below it.
fn below_viewport_center(h: &Harness<'_, EditorApp>) -> Pos2 {
    let rect = viewport(h);
    Pos2::new(rect.center().x, rect.min.y + rect.height() * 0.85)
}

const LAST: &str = "last.json";

/// The tree of a layout without the sizes. A reset layout has its sizes in points.
fn shape(node: &orr_editor::layout::Node) -> String {
    use orr_editor::layout::Node;
    match node {
        Node::Pane { panel, visible } => format!("{}{}", panel.key(), if *visible { "" } else { "(hidden)" }),
        Node::Split { dir, children } => format!("{dir:?}[{}]", children.iter().map(|child| shape(&child.node)).collect::<Vec<_>>().join(",")),
        Node::Tabs { children, active } => format!("Tabs{active}[{}]", children.iter().map(shape).collect::<Vec<_>>().join(",")),
    }
}

#[test]
fn default_layout_keeps_the_previous_arrangement() {
    let mut h = app(None);
    shot(&mut h, "default");
    let viewport = viewport(&h);
    let hierarchy = rect_of(&h, "Hierarchy");
    let inspector = rect_of(&h, "Inspector");
    assert!(hierarchy.max.x <= viewport.min.x + 1.0, "{hierarchy:?} {viewport:?}");
    assert!(inspector.min.x >= viewport.max.x - 1.0, "{inspector:?} {viewport:?}");
    let timeline = rect_of(&h, "Timeline");
    assert!(timeline.min.y >= viewport.max.y - 1.0, "{timeline:?} {viewport:?}");
    // The first sizes are the sizes of the fixed panels before docking: 230 and 340 points wide.
    assert!((viewport.min.x - 230.0).abs() < 8.0, "{viewport:?}");
    assert!((SIZE[0] - viewport.max.x - 340.0).abs() < 8.0, "{viewport:?}");
}

#[test]
fn a_panel_title_drag_moves_the_panel_below_the_viewport() {
    let mut h = app(None);
    let target = below_viewport_center(&h);
    drag_title(&mut h, "Hierarchy", target);
    // While the button is down the drop target is drawn, and nothing has moved yet.
    shot(&mut h, "drop_preview");
    assert!(h.query_all_by_label("Hierarchy").any(|node| node.rect().max.x <= viewport(&h).min.x + 1.0), "the panel moves only on release");
    press(&mut h, target, false);
    settle(&mut h);
    let moved = rect_of(&h, "Hierarchy");
    assert!(moved.min.y >= viewport(&h).max.y - 1.0, "the Hierarchy is below the viewport: {moved:?} {:?}", viewport(&h));
    let layout = h.state().ui.dock.layout().expect("a valid layout");
    layout.validate().expect("the layout stays valid");
    shot(&mut h, "rearranged");
}

#[test]
fn dropping_on_the_middle_of_a_panel_makes_a_tab() {
    let mut h = app(None);
    // The Inspector fills the right column. The middle of a pane, a little below its center, is the tab target.
    let row = viewport(&h);
    let target = Pos2::new((row.max.x + SIZE[0]) / 2.0, row.center().y + 12.0);
    drag_title(&mut h, "Hierarchy", target);
    press(&mut h, target, false);
    settle(&mut h);
    // Both panels share one place now: the layout holds them in one tab group.
    let layout = h.state().ui.dock.layout().unwrap().to_value().to_string();
    assert!(layout.contains("\"tabs\""), "{layout}");
    let viewport = viewport(&h);
    assert!(rect_of(&h, "Inspector").min.x >= viewport.max.x - 1.0 || rect_of(&h, "Hierarchy").min.x >= viewport.max.x - 1.0, "the grouped panels sit at the right");
    h.state().ui.dock.layout().expect("valid").validate().unwrap();
}

#[test]
fn escape_cancels_a_panel_drag_and_keeps_the_selection() {
    let mut h = app(None);
    let guid = guid_named(&h.state().editor, "body_05");
    h.state_mut().editor.select(Some(orr_editor::Target::Guid(guid.clone())));
    settle(&mut h);
    assert!(h.state().editor.selection().is_some(), "selected before");
    let before = h.state().ui.dock.layout().unwrap();
    let target = below_viewport_center(&h);
    drag_title(&mut h, "Hierarchy", target);
    assert!(h.state().editor.selection().is_some(), "selected during the drag");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().editor.selection().is_some(), "selected after Escape");
    press(&mut h, target, false);
    settle(&mut h);
    assert_eq!(h.state().ui.dock.layout().unwrap(), before, "the drag was cancelled");
    assert!(h.state().editor.selection().is_some(), "Escape for the drag does not clear the selection");
}

#[test]
fn a_panel_is_hidden_and_restored_from_the_window_menu() {
    let mut h = app(None);
    let wide = viewport(&h).width();
    open_window_menu(&mut h);
    click_role(&mut h, Role::CheckBox, "Inspector");
    settle(&mut h);
    assert!(!present(&h, "Inspector") || h.query_all_by_label("Inspector").all(|node| node.rect().min.x < viewport(&h).max.x), "the Inspector is gone");
    assert!(viewport(&h).width() > wide + 100.0, "the viewport took the room");
    assert!(!h.state().ui.dock.is_visible(Panel::Inspector));
    open_window_menu(&mut h);
    click_role(&mut h, Role::CheckBox, "Inspector");
    settle(&mut h);
    assert!(h.state().ui.dock.is_visible(Panel::Inspector));
    assert!((viewport(&h).width() - wide).abs() < 4.0, "back at its old place and size");
}

#[test]
fn the_viewport_cannot_be_hidden_or_lost() {
    let mut h = app(None);
    open_window_menu(&mut h);
    assert!(h.get_by_role_and_label(Role::CheckBox, "Viewport").accesskit_node().is_disabled(), "the Viewport entry is disabled");
    h.state_mut().ui.dock.set_visible(Panel::Viewport, false);
    settle(&mut h);
    assert!(h.state().ui.dock.is_visible(Panel::Viewport));
    // Hide everything else: the viewport fills the window and the menu still brings the rest back.
    for panel in [Panel::Hierarchy, Panel::Inspector, Panel::Timeline] {
        h.state_mut().ui.dock.set_visible(panel, false);
    }
    settle(&mut h);
    assert!(viewport(&h).width() > SIZE[0] - 20.0, "{:?}", viewport(&h));
    h.state().ui.dock.layout().unwrap().validate().unwrap();
}

#[test]
fn a_damaged_or_hidden_viewport_layout_is_never_loaded() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(LAST),
        r#"{"format":"orrery-editor-layout","version":1,"layout":{"tabs":[{"pane":"hierarchy","visible":true},{"pane":"viewport","visible":false},{"pane":"inspector","visible":true},{"pane":"timeline","visible":true}],"active":0}}"#,
    )
    .unwrap();
    let h = app(Some(dir.path()));
    assert!(h.state().ui.dock.is_visible(Panel::Viewport));
    assert!(h.state().ui.dock.notice().is_some(), "the notice says the file was not used");
}

#[test]
fn reset_brings_back_the_default_layout() {
    let mut h = app(None);
    let original = h.state().ui.dock.layout().unwrap();
    open_window_menu(&mut h);
    click_role(&mut h, Role::CheckBox, "Inspector");
    let target = below_viewport_center(&h);
    drag_title(&mut h, "Hierarchy", target);
    press(&mut h, target, false);
    settle(&mut h);
    assert_ne!(h.state().ui.dock.layout().unwrap(), original);
    open_window_menu(&mut h);
    click(&mut h, "Layout ⏵");
    click(&mut h, "Reset to default");
    settle(&mut h);
    let layout = h.state().ui.dock.layout().unwrap();
    assert_eq!(shape(&layout.root), shape(&orr_editor::layout::Layout::default().root));
    assert!(rect_of(&h, "Hierarchy").max.x <= viewport(&h).min.x + 1.0);
    assert!(present(&h, "Inspector"));
}

#[test]
fn a_saved_preset_and_the_last_layout_come_back_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let settings = dir.path().join("layouts");
    {
        let mut h = app(Some(&settings));
        // Hide the Inspector and keep that arrangement as a preset.
        open_window_menu(&mut h);
        click_role(&mut h, Role::CheckBox, "Inspector");
        settle(&mut h);
        open_window_menu(&mut h);
        click(&mut h, "Layout ⏵");
        click(&mut h, "Save as\u{2026}");
        h.event(Event::Text("Wide view".into()));
        h.run_steps(3);
        click_role(&mut h, Role::Button, "Save");
        settle(&mut h);
        assert_eq!(h.state().ui.dock.presets(), ["Wide view".to_owned()]);
        assert!(h.state().ui.dock.notice().is_none(), "{:?}", h.state().ui.dock.notice());
        // Move the Hierarchy below, so the last layout differs from the preset.
        let target = below_viewport_center(&h);
        drag_title(&mut h, "Hierarchy", target);
        press(&mut h, target, false);
        settle(&mut h);
        assert!(settings.join(LAST).is_file(), "the last layout was written");
    }
    // A new window, the same settings folder: the last layout is back.
    let mut h = app(Some(&settings));
    assert!(!h.state().ui.dock.is_visible(Panel::Inspector), "the hidden Inspector stays hidden");
    assert!(rect_of(&h, "Hierarchy").min.y >= viewport(&h).max.y - 1.0, "the moved Hierarchy stays below");
    assert_eq!(h.state().ui.dock.presets(), ["Wide view".to_owned()]);
    shot(&mut h, "preset_restored");
    // Reset, then pick the preset from the menu.
    open_window_menu(&mut h);
    click(&mut h, "Layout ⏵");
    click(&mut h, "Reset to default");
    settle(&mut h);
    assert!(h.state().ui.dock.is_visible(Panel::Inspector));
    open_window_menu(&mut h);
    click(&mut h, "Layout ⏵");
    click(&mut h, "Wide view");
    settle(&mut h);
    assert!(!h.state().ui.dock.is_visible(Panel::Inspector));
    assert!(rect_of(&h, "Hierarchy").max.x <= viewport(&h).min.x + 1.0, "the preset has the Hierarchy at the left");
}

#[test]
fn unusable_layout_files_give_the_default_layout_and_a_notice() {
    let valid_tree = r#"{"split":"vertical","children":[{"share":800,"node":{"split":"horizontal","children":[{"share":200,"node":{"pane":"hierarchy","visible":true}},{"share":600,"node":{"pane":"viewport","visible":true}},{"share":200,"node":{"pane":"inspector","visible":true}}]}},{"share":200,"node":{"pane":"timeline","visible":true}}]}"#;
    let wrap = |tree: &str| format!(r#"{{"format":"orrery-editor-layout","version":1,"layout":{tree}}}"#);
    let cases: [(&str, String); 6] = [
        ("damaged", "{ this is not json".to_owned()),
        ("unknown version", format!(r#"{{"format":"orrery-editor-layout","version":9,"layout":{valid_tree}}}"#)),
        ("missing panel", r#"{"format":"orrery-editor-layout","version":1,"layout":{"pane":"viewport","visible":true}}"#.to_owned()),
        ("unknown panel", wrap(&valid_tree.replace("inspector", "terminal"))),
        ("too large", " ".repeat(70_000)),
        ("not a layout file", "{}".to_owned()),
    ];
    for (name, text) in cases {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(LAST), &text).unwrap();
        let mut h = app(Some(dir.path()));
        assert!(h.state().ui.dock.notice().is_some(), "{name}: no notice");
        assert!(present(&h, "Dismiss"), "{name}: the notice bar is shown");
        assert_eq!(shape(&h.state().ui.dock.layout().unwrap().root), shape(&orr_editor::layout::Layout::default().root), "{name}: the default arrangement");
        // The notice does not block the window: the controls still work.
        click(&mut h, "Dismiss");
        assert!(h.state().ui.dock.notice().is_none(), "{name}");
        assert!(h.state().ui.dock.is_visible(Panel::Viewport));
    }
}

#[test]
fn a_failed_write_keeps_the_old_layout_file() {
    let dir = tempfile::tempdir().unwrap();
    let settings = dir.path().join("layouts");
    let mut h = app(Some(&settings));
    // Make the first change, so a last layout exists.
    open_window_menu(&mut h);
    click_role(&mut h, Role::CheckBox, "Timeline & tools");
    settle(&mut h);
    let written = std::fs::read(settings.join(LAST)).expect("the first change was written");
    // A folder in the place of the temporary file makes the next write fail.
    std::fs::create_dir(settings.join("last.json.tmp")).unwrap();
    open_window_menu(&mut h);
    click_role(&mut h, Role::CheckBox, "Inspector");
    settle(&mut h);
    assert_eq!(std::fs::read(settings.join(LAST)).unwrap(), written, "the old file is unchanged");
    let notice = h.state().ui.dock.notice().expect("a notice");
    assert!(notice.contains("not saved"), "{notice}");
    // The window works on in memory.
    assert!(!h.state().ui.dock.is_visible(Panel::Inspector));
    // A restart reads the old file and gives the layout of the last good write.
    let again = app(Some(&settings));
    assert!(again.state().ui.dock.is_visible(Panel::Inspector));
    assert!(!again.state().ui.dock.is_visible(Panel::Timeline));
}

#[test]
fn a_saved_layout_that_does_not_fit_the_window_gives_the_default_layout() {
    // Inspector and Hierarchy share one column of 900 shares each side: no room is left in the middle.
    let tree = r#"{"split":"horizontal","children":[{"share":1,"node":{"pane":"hierarchy","visible":true}},{"share":1,"node":{"pane":"viewport","visible":true}},{"share":900,"node":{"split":"vertical","children":[{"share":1,"node":{"pane":"inspector","visible":true}},{"share":1,"node":{"pane":"timeline","visible":true}}]}}]}"#;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(LAST), format!(r#"{{"format":"orrery-editor-layout","version":1,"layout":{tree}}}"#)).unwrap();
    let h = app(Some(dir.path()));
    let notice = h.state().ui.dock.notice().expect("a notice");
    assert!(notice.contains("does not fit"), "{notice}");
    assert!(viewport(&h).width() >= 300.0);
}

#[test]
fn arranging_panels_does_not_change_the_document() {
    let mut h = app(None);
    let before = doc_checksum(&mut h.state_mut().editor);
    let history = h.state().editor.history().clone();
    let mode = h.state().editor.mode();
    open_window_menu(&mut h);
    click_role(&mut h, Role::CheckBox, "Hierarchy");
    open_window_menu(&mut h);
    click(&mut h, "Move panel ⏵");
    click(&mut h, "Inspector ⏵");
    click(&mut h, "Dock at left edge");
    settle(&mut h);
    assert_eq!(doc_checksum(&mut h.state_mut().editor), before);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(h.state().editor.mode(), mode);
    assert!(!h.state().editor.is_dirty());
    assert!(rect_of(&h, "Inspector").max.x <= viewport(&h).min.x + 1.0, "the Inspector is at the left edge");
}

#[test]
fn the_move_panel_menu_docks_and_tabs_without_the_mouse() {
    let mut h = app(None);
    open_window_menu(&mut h);
    click(&mut h, "Move panel ⏵");
    click(&mut h, "Timeline & tools ⏵");
    click(&mut h, "Tab with Hierarchy");
    settle(&mut h);
    let layout = h.state().ui.dock.layout().unwrap();
    assert!(layout.to_value().to_string().contains("\"tabs\""));
    layout.validate().unwrap();
    // Both titles are tabs of one group now.
    // Each name shows once, on its tab, not again as a panel title.
    assert_eq!(h.query_all_by_label("Timeline & tools").count(), 1);
    assert_eq!(h.query_all_by_label("Hierarchy").count(), 1);
}

/// A line that only the unsaved work details show.
const DETAILS: &str = "This list never saves, applies, discards or closes anything. Scene Save does not save independent documents.";

#[test]
fn the_unsaved_button_shows_the_details_when_the_timeline_is_hidden() {
    let mut h = app(None);
    open_window_menu(&mut h);
    click_role(&mut h, Role::CheckBox, "Timeline & tools");
    settle(&mut h);
    assert!(!h.state().ui.dock.is_visible(Panel::Timeline));
    assert!(!present(&h, DETAILS));
    let inspector = h.state().ui.dock.is_visible(Panel::Inspector);
    click(&mut h, "Unsaved: 0");
    settle(&mut h);
    assert!(h.state().ui.dock.is_visible(Panel::Timeline), "the Timeline is shown again");
    assert!(present(&h, DETAILS), "the details are on the screen");
    assert_eq!(h.state().ui.dock.is_visible(Panel::Inspector), inspector, "the other panels stay as they were");
}

#[test]
fn the_unsaved_button_brings_a_tabbed_timeline_to_the_front() {
    let mut h = app(None);
    h.state_mut().ui.dock.tab_with(Panel::Timeline, Panel::Hierarchy);
    settle(&mut h);
    // Put the Hierarchy in front, so the Timeline tab is behind it.
    click(&mut h, "Hierarchy");
    settle(&mut h);
    assert!(h.state().ui.dock.is_visible(Panel::Timeline), "the Timeline is only behind another tab");
    assert!(!present(&h, DETAILS), "the Timeline is not on the screen");
    click(&mut h, "Unsaved: 0");
    settle(&mut h);
    assert!(present(&h, DETAILS), "the details are on the screen");
}
