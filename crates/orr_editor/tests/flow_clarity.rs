//! The making flow in the existing panels: which entity is selected, edit or
//! play, unsaved or saved, where an error is, and what to do next.
//!
//! Screenshots are written only when `ORR_FLOW_SHOTS` names a file prefix, for
//! example `C:\tmp\flow_after_`. The frame-time numbers are printed with
//! `--nocapture` (lines that start with `FLOWTIME`).
#![allow(clippy::disallowed_types)]

mod common;

use std::time::{Duration, Instant};

use common::*;
use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use orr_editor::app::LBL_PLAY;
use orr_editor::editor::Mode;
use orr_editor::EditorApp;
use orr_reflect::{decimal, Value};

fn build(editor: orr_editor::Editor, size: [f32; 2], ppp: f32, gpu: bool) -> Harness<'static, EditorApp> {
    let builder = Harness::builder().with_size(size).with_pixels_per_point(ppp).with_step_dt(1.0 / 60.0);
    let builder = if gpu { builder.wgpu() } else { builder };
    builder.build_eframe(move |_cc| EditorApp::new(editor, None))
}

fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}

fn wait_for(h: &mut Harness<'_, EditorApp>, what: &str, pred: impl Fn(&EditorApp) -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !pred(h.state()) {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn text_field<'a>(h: &'a Harness<'_, EditorApp>, value: &'a str) -> egui_kittest::Node<'a> {
    h.get_by(move |n| n.role() == Role::TextInput && n.value().as_deref() == Some(value))
}

fn type_into(h: &mut Harness<'_, EditorApp>, current: &str, text: &str) {
    text_field(h, current).focus();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    text_field(h, current).type_text(text);
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::NONE, Key::Enter);
    settle(h);
}

fn pos_x_text(h: &mut Harness<'_, EditorApp>, name: &str) -> String {
    let ed = &mut h.state_mut().editor;
    let t = target_named(ed, name);
    let Value::Fixed(x) = field(ed, &t, BODY, "pos.x") else { panic!("pos.x is fixed") };
    decimal::fp_to_decimal(x)
}

/// Writes the harness image when `ORR_FLOW_SHOTS` is set. Needs a GPU harness.
fn shot(h: &mut Harness<'_, EditorApp>, name: &str) {
    let Some(prefix) = std::env::var_os("ORR_FLOW_SHOTS") else { return };
    h.run_steps(2);
    let image = h.render().expect("render the editor window");
    let mut path = prefix;
    path.push(format!("{name}.png"));
    image.save(&path).expect("write the screenshot");
}

/// The demo scene with one entity renamed to a long Korean name.
fn korean_editor() -> (orr_editor::Editor, tempfile::TempDir) {
    let text = std::fs::read_to_string(orr_editor::editor::default_scene_path()).expect("demo scene");
    assert!(text.contains("name: body_05\n"), "the demo scene names body_05");
    let text = text.replace("name: body_05\n", "name: 상자_05_아주_긴_한국어_이름_검사\n");
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("korean.scene.yaml");
    std::fs::write(&path, text).expect("write the scene");
    let mut ed = orr_editor::Editor::open(&path).expect("open the Korean scene");
    ed.sync();
    (ed, dir)
}

#[test]
fn flow_screenshots() {
    let Some(gpu) = common::gpu() else {
        eprintln!("no GPU adapter: flow screenshots skipped");
        return;
    };
    drop(gpu);

    // 1. The scene that opens on start: nothing selected.
    let mut h = build(demo_editor(), [1500.0, 900.0], 1.0, true);
    h.run_steps(3);
    shot(&mut h, "flow_1_start");

    // 2. One entity selected.
    h.get_by_label("body_05").click();
    settle(&mut h);
    shot(&mut h, "flow_2_selected");

    // 3. An edit makes the scene unsaved.
    let shown = pos_x_text(&mut h, "body_05");
    type_into(&mut h, &shown, "3.25");
    assert!(h.state().editor.is_dirty());
    shot(&mut h, "flow_3_dirty");

    // 4. Text that is not a number is refused.
    let shown = pos_x_text(&mut h, "body_05");
    type_into(&mut h, &shown, "abc");
    shot(&mut h, "flow_4_refused");

    // 5. Play.
    h.get_by_label(LBL_PLAY).click();
    wait_for(&mut h, "play mode", |a| a.editor.mode() == Mode::Play);
    settle(&mut h);
    shot(&mut h, "flow_5_play");
    drop(h);

    // 6. The smallest window.
    let mut h = build(demo_editor(), [1024.0, 640.0], 1.0, true);
    h.run_steps(3);
    h.get_by_label("body_05").click();
    settle(&mut h);
    let shown = pos_x_text(&mut h, "body_05");
    type_into(&mut h, &shown, "3.25");
    shot(&mut h, "flow_6_min_window");
    drop(h);

    // 7. A long Korean name.
    let (ed, _dir) = korean_editor();
    let mut h = build(ed, [1280.0, 800.0], 1.0, true);
    h.run_steps(3);
    h.get_by_label("상자_05_아주_긴_한국어_이름_검사").click();
    settle(&mut h);
    shot(&mut h, "flow_7_korean");
    drop(h);

    // 8. Display scale 1.5 and 2.0.
    for (ppp, name) in [(1.5, "flow_8_dpi150"), (2.0, "flow_9_dpi200")] {
        let mut h = build(demo_editor(), [1024.0, 640.0], ppp, true);
        h.run_steps(3);
        h.get_by_label("body_05").click();
        settle(&mut h);
        shot(&mut h, name);
    }
}

/// UI frame cost with the hierarchy, inspector and bars drawn (no GPU).
#[test]
fn flow_frame_time() {
    let mut h = build(demo_editor(), [1500.0, 900.0], 1.0, false);
    settle(&mut h);
    h.get_by_label("body_05").click();
    settle(&mut h);
    for (phase, frames) in [("selected", 300)] {
        let mut ms = Vec::new();
        for _ in 0..frames {
            let t = Instant::now();
            h.step();
            ms.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let pick = |q: f64| ms[((ms.len() - 1) as f64 * q) as usize];
        eprintln!(
            "FLOWTIME {phase}: median {:.3} ms / p95 {:.3} ms / max {:.3} ms over {frames} frames",
            pick(0.5), pick(0.95), pick(1.0)
        );
    }
}

// ---- states ----

use egui::{Rect, Vec2};
use egui_kittest::Node;
use orr_editor::app::{LBL_PAUSE, LBL_STEP, LBL_STOP};
use orr_editor::flow::{next_step, Facts, NEW_PROJECT_ROUTE};

fn texts(h: &Harness<'_, EditorApp>, text: &str) -> usize {
    h.query_all_by_label(text).count()
}

fn contains(h: &Harness<'_, EditorApp>, text: &str) -> bool {
    h.query_all_by_label_contains(text).next().is_some()
}

fn inside(node: &Node<'_>, window: Rect, what: &str) {
    let rect = node.rect();
    assert!(rect.width() > 1.0 && rect.height() > 1.0, "{what} has no size: {rect:?}");
    assert!(window.contains_rect(rect), "{what} is clipped by the window: {rect:?} outside {window:?}");
}

fn select_and_edit(h: &mut Harness<'_, EditorApp>) {
    h.get_by_label("body_05").click();
    settle(h);
    let shown = pos_x_text(h, "body_05");
    type_into(h, &shown, "3.25");
}

#[test]
fn start_on_the_sample_names_the_next_action() {
    let mut h = build(demo_editor(), [1500.0, 900.0], 1.0, false);
    h.run_steps(3);
    assert!(h.state().editor.selection().is_none());
    // Status bar: one short next action, with the route to a project of the user.
    assert_eq!(texts(&h, &format!("Select an entity, or {NEW_PROJECT_ROUTE}")), 1);
    // Inspector: what to do, and why this scene is not a project.
    assert_eq!(texts(&h, "Nothing selected"), 1);
    assert!(contains(&h, "Select an entity to edit it."));
    assert!(contains(&h, "This is the built-in sample scene."));
    assert!(contains(&h, NEW_PROJECT_ROUTE));
    // Mode and dirty state: edit, nothing unsaved.
    assert_eq!(texts(&h, "EDIT"), 1);
    assert_eq!(texts(&h, "PLAY"), 0);
    assert!(!contains(&h, "Unsaved changes"));
    assert!(!h.state().editor.title().starts_with('*'));
}

#[test]
fn a_selected_entity_is_named_by_hierarchy_and_inspector() {
    let mut h = build(demo_editor(), [1500.0, 900.0], 1.0, false);
    h.run_steps(3);
    h.get_by_label("body_05").click();
    settle(&mut h);
    // Hierarchy: the count says one is selected.
    assert!(contains(&h, " entities \u{B7} 1 selected"));
    let want = target_named(&h.state().editor, "body_05");
    assert!(h.state().editor.is_selected(&want));
    // Inspector: the header holds the name and the kind, and the empty state is gone.
    assert_eq!(texts(&h, "Physics body"), 1);
    assert_eq!(texts(&h, "Nothing selected"), 0);
    assert_eq!(texts(&h, "Edit in Inspector \u{B7} Ctrl+S saves \u{B7} Play tests"), 1);
}

#[test]
fn an_edit_shows_unsaved_in_the_bar_status_and_title() {
    let mut h = build(demo_editor(), [1500.0, 900.0], 1.0, false);
    h.run_steps(3);
    select_and_edit(&mut h);
    assert!(h.state().editor.is_dirty());
    assert_eq!(texts(&h, "\u{25CF} Unsaved changes"), 1);
    assert_eq!(texts(&h, "Unsaved: 1"), 1);
    // The status bar shows the message of the edit at first. Then the next step takes its place.
    h.run_steps(500);
    assert_eq!(texts(&h, "Unsaved \u{B7} Ctrl+S saves, Ctrl+Z undoes"), 1);
    assert!(h.state().editor.title().starts_with('*'));
    // Undo makes all three cues go away.
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    settle(&mut h);
    assert!(!h.state().editor.is_dirty());
    assert_eq!(texts(&h, "\u{25CF} Unsaved changes"), 0);
    assert!(!h.state().editor.title().starts_with('*'));
}

#[test]
fn play_mode_is_marked_and_the_inspector_says_edits_are_temporary() {
    let mut h = build(demo_editor(), [1500.0, 900.0], 1.0, false);
    h.run_steps(3);
    h.get_by_label("body_05").click();
    settle(&mut h);
    assert_eq!(texts(&h, "Play mode: edits here are temporary."), 0);
    h.get_by_label(LBL_PLAY).click();
    wait_for(&mut h, "play mode", |a| a.editor.mode() == Mode::Play);
    settle(&mut h);
    h.state_mut().editor.pause();
    settle(&mut h);
    assert_eq!(texts(&h, "PLAY"), 1);
    assert_eq!(texts(&h, "EDIT"), 0);
    assert_eq!(texts(&h, "Play mode: edits here are temporary."), 1);
    assert!(contains(&h, "The scene file does not change."));
    assert!(h.state().editor.title().contains("[play]"));
    // A play edit does not make the scene unsaved: the cue is true.
    let shown = pos_x_text(&mut h, "body_05");
    type_into(&mut h, &shown, "4.5");
    assert!(!h.state().editor.is_dirty());
    assert_eq!(texts(&h, "\u{25CF} Unsaved changes"), 0);
    // Stop brings the edit state back.
    h.get_by_label(LBL_STOP).click();
    wait_for(&mut h, "edit mode", |a| a.editor.mode() == Mode::Edit);
    settle(&mut h);
    assert_eq!(texts(&h, "EDIT"), 1);
    assert_eq!(texts(&h, "PLAY"), 0);
    assert_eq!(texts(&h, "Play mode: edits here are temporary."), 0);
}

#[test]
fn a_refused_edit_stays_in_the_inspector_until_it_no_longer_applies() {
    let mut h = build(demo_editor(), [1500.0, 900.0], 1.0, false);
    h.run_steps(3);
    h.get_by_label("body_05").click();
    settle(&mut h);
    let original = scene_text(&mut h.state_mut().editor);
    let shown = pos_x_text(&mut h, "body_05");
    type_into(&mut h, &shown, "abc");
    assert_eq!(scene_text(&mut h.state_mut().editor), original, "a refused edit changes nothing");
    // The message is in the Inspector, next to the fields, and also in the status bar.
    assert!(contains(&h, "Not applied: pos.x: 'abc' is not a plain decimal number"));
    assert!(contains(&h, "The field keeps its old value."));
    assert!(h.state().editor.status().is_some_and(|m| m.error));
    // Another selection: the refusal does not belong to it.
    h.get_by_label("floor").click();
    settle(&mut h);
    assert!(!contains(&h, "Not applied:"));
    // A refusal, then a good value: the refusal goes away.
    h.get_by_label("body_05").click();
    settle(&mut h);
    assert!(!contains(&h, "Not applied:"), "an old refusal does not come back with the selection");
    let shown = pos_x_text(&mut h, "body_05");
    type_into(&mut h, &shown, "abc");
    assert!(contains(&h, "Not applied:"));
    let shown = pos_x_text(&mut h, "body_05");
    type_into(&mut h, &shown, "2.5");
    assert!(!contains(&h, "Not applied:"));
    assert!(h.state().editor.is_dirty());
}

#[test]
fn smallest_window_keeps_every_main_control_inside() {
    let window = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1024.0, 640.0));
    let mut h = build(demo_editor(), [1024.0, 640.0], 1.0, false);
    h.run_steps(3);
    select_and_edit(&mut h);
    for label in [LBL_PLAY, LBL_PAUSE, LBL_STEP, LBL_STOP, "EDIT", "File", "Edit", "Hierarchy", "Inspector", "\u{25CF} Unsaved changes", "Timeline", "Physics body"] {
        inside(&h.get_by_label(label), window, label);
    }
    let status = h.get_by_label_contains("Unsaved: ");
    inside(&status, window, "Unsaved: N");
    drop(status);
    // The viewport keeps room to work in.
    let viewport = h.state().ui.viewport_rect.expect("viewport");
    assert!(viewport.width() >= 400.0 && viewport.height() >= 250.0, "viewport {viewport:?}");
    // Play mode adds the cue and the badge; they fit too.
    h.get_by_label(LBL_PLAY).click();
    wait_for(&mut h, "play mode", |a| a.editor.mode() == Mode::Play);
    settle(&mut h);
    for label in ["PLAY", "Play mode: edits here are temporary.", LBL_STOP] {
        inside(&h.get_by_label(label), window, label);
    }
}

#[test]
fn display_scales_keep_every_main_control_inside() {
    for (ppp, size) in [(1.5_f32, [1280.0_f32, 800.0_f32]), (2.0, [1024.0, 640.0])] {
        let window = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(size[0], size[1]));
        let mut h = build(demo_editor(), size, ppp, false);
        h.run_steps(3);
        select_and_edit(&mut h);
        for label in [LBL_PLAY, LBL_STOP, "EDIT", "\u{25CF} Unsaved changes", "Inspector", "Physics body"] {
            inside(&h.get_by_label(label), window, &format!("{label} at scale {ppp}"));
        }
    }
}

#[test]
fn korean_names_have_glyphs_and_fit_the_inspector_header() {
    let (ed, _dir) = korean_editor();
    let mut h = build(ed, [1280.0, 800.0], 1.0, false);
    h.run_steps(3);
    let name = "상자_05_아주_긴_한국어_이름_검사";
    let font = egui::FontId::proportional(13.0);
    let has = h.ctx.fonts_mut(|fonts| fonts.has_glyphs(&font, name));
    assert!(has, "a system font with Hangul must back the editor font; Malgun Gothic is expected on Windows");
    h.get_by_label(name).click();
    settle(&mut h);
    let window = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 800.0));
    let rects = h.query_all_by_label(name).map(|n| n.rect()).collect::<Vec<_>>();
    assert!(!rects.is_empty());
    for rect in rects {
        assert!(window.contains_rect(rect), "{rect:?}");
    }
}

#[test]
fn next_step_follows_the_work_in_order() {
    let base = Facts::default();
    let sample = Facts { sample: true, ..base };
    assert_eq!(next_step(sample), format!("Select an entity, or {NEW_PROJECT_ROUTE}"));
    assert_eq!(next_step(base), "Select an entity in Hierarchy to edit it");
    assert!(next_step(Facts { selected: true, ..base }).starts_with("Edit in Inspector"));
    assert!(next_step(Facts { selected: true, dirty: true, ..base }).starts_with("Unsaved"));
    assert!(next_step(Facts { selected: true, dirty: true, playing: true, ..base }).starts_with("Playing"));
    assert!(next_step(Facts { playing: true, viewer: true, ..base }).starts_with("Replay viewer"));
    assert!(next_step(Facts { playing: true, host_down: true, ..base }).starts_with("Host stopped"));
}

#[test]
fn status_messages_age_into_the_next_step() {
    let mut state = orr_editor::flow::State::default();
    state.observe_start(None, 1);
    assert!(!state.status_fresh(1, false, 0.0), "the message of the start is not news");
    assert!(state.status_fresh(2, false, 1.0));
    assert!(state.status_fresh(2, false, 8.9));
    assert!(!state.status_fresh(2, false, 9.1), "a message of success gives way to the next step");
    assert!(state.status_fresh(3, false, 20.0));
    assert!(state.status_fresh(3, true, 60.0), "a refusal stays until a newer message replaces it");
}

/// WCAG contrast ratio of two opaque colors.
fn contrast(a: egui::Color32, b: egui::Color32) -> f64 {
    fn channel(c: u8) -> f64 {
        let c = f64::from(c) / 255.0;
        if c <= 0.039_28 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    }
    let l = |c: egui::Color32| 0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b());
    let (hi, lo) = (l(a).max(l(b)), l(a).min(l(b)));
    (hi + 0.05) / (lo + 0.05)
}

#[test]
fn state_colors_are_readable_on_the_panels() {
    use orr_editor::theme::*;
    for (name, color) in [("TEXT", TEXT), ("TEXT_STRONG", TEXT_STRONG), ("TEXT_WEAK", TEXT_WEAK), ("ACCENT", ACCENT), ("PLAY", PLAY), ("WARN", WARN), ("ERROR", ERROR), ("INFO", INFO)] {
        for (ground_name, ground) in [("PANEL", PANEL), ("BAR", BAR), ("CARD", CARD)] {
            let ratio = contrast(color, ground);
            assert!(ratio >= 4.5, "{name} on {ground_name}: {ratio:.2}");
        }
    }
    // The selected row: strong text over the selection fill over the panel.
    let a = f32::from(ACCENT_FILL.a()) / 255.0;
    let mix = |fill: u8, ground: u8| (f32::from(fill) + f32::from(ground) * (1.0 - a)).round() as u8;
    let selected = egui::Color32::from_rgb(mix(ACCENT_FILL.r(), PANEL.r()), mix(ACCENT_FILL.g(), PANEL.g()), mix(ACCENT_FILL.b(), PANEL.b()));
    assert!(contrast(TEXT_STRONG, selected) >= 4.5, "selected row {:.2}", contrast(TEXT_STRONG, selected));
    // The play badge: dark text on the play fill.
    assert!(contrast(BG, PLAY) >= 4.5);
    assert!(contrast(TEXT_STRONG, CARD_HEADER) >= 4.5);
}
