//! No implicit skips: CLI and GPU acceptances have separate mandatory inventories.
#![cfg(feature = "input-map-editor")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
#[path = "../src/input_map_authoring.rs"]
mod input_map_authoring;
use input_map_authoring::Panel;
use egui::accesskit::Role;
use egui_kittest::{kittest::{NodeT, Queryable}, Harness};
use orr_bridge::{Bridge, InProc, PlayerSlot};
use orr_input::{ActionMap, Button, Key};
use orr_sample::arena_input::{default_map, validate_map, ArenaControls};
use orr_sample::arena_view::{arena_bridge_config, loopback_pair, Keys, Loopback};
use std::{fs, path::Path, time::Duration};

fn harness() -> Harness<'static, Panel> {
    let mut h = Harness::builder().with_size([900.0, 760.0]).with_max_steps(120).build_ui_state(
        |ui, panel: &mut Panel| { egui::ScrollArea::vertical().show(ui, |ui| panel.show(ui)); }, Panel::default());
    settle(&mut h, "initial layout");
    h
}
// AccessKit scrolling animates: wait for bounded repaint/layout convergence before
// using a node's center for a genuine pointer click. Never mutate the Panel directly.
fn settle(h: &mut Harness<'_, Panel>, context: &str) {
    h.try_run().unwrap_or_else(|error| panic!("layout did not settle after {context}: {error}"));
}
fn click(h: &mut Harness<'_, Panel>, label: &str) {
    h.get_by_label(label).scroll_to_me(); settle(h, label);
    h.get_by_label(label).click(); settle(h, label);
}
fn open_combo(h: &mut Harness<'_, Panel>, label: &str) {
    h.get_by_role_and_label(Role::ComboBox, label).scroll_to_me(); settle(h, label);
    h.get_by_role_and_label(Role::ComboBox, label).click(); settle(h, label);
}
fn assert_combo(h: &Harness<'_, Panel>, label: &str, expected: &str) {
    let actual = h.get_by_role_and_label(Role::ComboBox, label).accesskit_node().value();
    assert_eq!(actual.as_deref(), Some(expected),
        "observed {label} must match requested value; accepted map: {:?}", h.state().document());
}
fn action(h: &mut Harness<'_, Panel>, name: &str) {
    open_combo(h, "Action"); click(h, name);
    assert_combo(h, "Action", name);
}
fn set_action(h: &mut Harness<'_, Panel>, name: &str, binding: &str) {
    action(h, name);
    click(h, "Clear action draft"); click(h, "Add binding");
    open_combo(h, "Binding 1"); click(h, binding);
    assert_combo(h, "Action", name);
    assert_combo(h, "Binding 1", binding);
    click(h, "Apply action");
    assert!(h.state().message().starts_with("Action applied"),
        "requested action={name}, binding={binding}; result={}; accepted map={:?}",
        h.state().message(), h.state().document());
}
fn authored() -> Harness<'static, Panel> {
    let mut h = harness();
    for (name, binding) in [("left", "Key I"), ("right", "Key L"), ("up", "Key U"),
        ("down", "Key K"), ("fire", "Mouse 0"), ("pause", "Key O"), ("quit", "Key Q")] {
        set_action(&mut h, name, binding);
    }
    h
}
fn path(h: &mut Harness<'_, Panel>, value: &Path) {
    h.get_by_role(Role::TextInput).focus(); settle(h, "path focus");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A); settle(h, "select path");
    h.get_by_role(Role::TextInput).type_text(value.to_str().unwrap()); settle(h, "path input");
}
fn save_widgets(h: &mut Harness<'_, Panel>, value: &Path) {
    path(h, value); click(h, "Save As new file");
    assert!(h.state().message().starts_with("Saved."), "{}", h.state().message());
}
fn key(key: Key) -> Button { Button::Keyboard { key } }

#[test]
fn seven_action_widgets_save_reopen_and_exact_undo_redo() {
    let dir = tempfile::tempdir().unwrap(); let file = dir.path().join("bindings.json");
    let mut h = authored(); let expected = h.state().document().clone();
    validate_map(&expected).unwrap();
    for _ in 0..7 { click(&mut h, "Undo map edit"); }
    assert_eq!(h.state().document(), &default_map()); assert!(!h.state().can_undo());
    for _ in 0..7 { click(&mut h, "Redo map edit"); }
    assert_eq!(h.state().document(), &expected); assert!(!h.state().can_redo());
    save_widgets(&mut h, &file);
    assert_eq!(ActionMap::load_file(&file).unwrap(), expected);
    let before = fs::read(&file).unwrap();
    click(&mut h, "Save As new file");
    assert!(!h.state().message().starts_with("Saved.")); assert_eq!(fs::read(&file).unwrap(), before);
    click(&mut h, "Use Arena defaults"); click(&mut h, "Open map (undoable)");
    assert_eq!(h.state().document(), &expected); click(&mut h, "Undo map edit");
    assert_eq!(h.state().document(), &default_map()); click(&mut h, "Redo map edit");
    assert_eq!(h.state().document(), &expected);

    // A loaded map may order actions differently from default_map(). Defaults
    // must follow the selected name, never its index in either action array.
    let mut reordered = expected.clone(); reordered.actions.reverse();
    let reordered_file = dir.path().join("reordered.json"); reordered.save_new(&reordered_file).unwrap();
    let reordered_bytes = fs::read(&reordered_file).unwrap();
    path(&mut h, &reordered_file); click(&mut h, "Open map (undoable)");
    assert_eq!(h.state().document(), &reordered);
    action(&mut h, "right"); assert_combo(&h, "Binding 1", "Key L");
    let history = (h.state().can_undo(), h.state().can_redo());
    click(&mut h, "Restore action defaults");
    assert_combo(&h, "Binding 1", "Key D"); assert_combo(&h, "Binding 2", "Key ArrowRight");
    assert_eq!(h.state().document(), &reordered);
    assert_eq!((h.state().can_undo(), h.state().can_redo()), history);
    assert_eq!(fs::read(&file).unwrap(), before);
    assert_eq!(fs::read(&reordered_file).unwrap(), reordered_bytes);
    let draft_save = dir.path().join("pending-defaults.json"); save_widgets(&mut h, &draft_save);
    assert_eq!(ActionMap::load_file(&draft_save).unwrap(), reordered, "Save ignores the pending restored draft");
    // Restore itself added no history entry: Undo reaches the preceding Open.
    click(&mut h, "Undo map edit"); assert_eq!(h.state().document(), &expected);
    click(&mut h, "Redo map edit"); assert_eq!(h.state().document(), &reordered);
    assert_combo(&h, "Binding 1", "Key L");
    click(&mut h, "Restore action defaults"); click(&mut h, "Apply action");
    assert!(h.state().message().starts_with("Action applied"));
    let mut restored = reordered.clone();
    restored.actions.iter_mut().find(|a|a.name=="right").unwrap().bindings = vec![key(Key::D), key(Key::ArrowRight)];
    assert_eq!(h.state().document(), &restored, "Other actions and their order must remain exact");
    click(&mut h, "Undo map edit"); assert_eq!(h.state().document(), &reordered);
    click(&mut h, "Redo map edit"); assert_eq!(h.state().document(), &restored);
    let restored_file = dir.path().join("restored.json"); save_widgets(&mut h, &restored_file);
    let mut reopened = harness(); path(&mut reopened, &restored_file); click(&mut reopened, "Open map (undoable)");
    assert_eq!(reopened.state().document(), &restored);
    assert_eq!(fs::read(&file).unwrap(), before);
    assert_eq!(fs::read(&reordered_file).unwrap(), reordered_bytes);

    // Keep the Save-dependent defaults contract in the existing Save/reopen
    // case, so the separate widget-conflict case can remain memory-only.
    let mut h = harness();
    set_action(&mut h, "right", "Key L"); set_action(&mut h, "left", "Key D");
    let accepted = h.state().document().clone();
    let dir = tempfile::tempdir().unwrap(); let file = dir.path().join("before-restore.json");
    save_widgets(&mut h, &file); let bytes = fs::read(&file).unwrap();
    set_action(&mut h, "quit", "Key Q"); let redo = h.state().document().clone();
    click(&mut h, "Undo map edit"); assert_eq!(h.state().document(), &accepted); assert!(h.state().can_redo());
    action(&mut h, "right"); click(&mut h, "Restore action defaults");
    assert_combo(&h, "Binding 1", "Key D"); assert_combo(&h, "Binding 2", "Key ArrowRight");
    assert_eq!(h.state().document(), &accepted); assert!(h.state().can_redo());
    click(&mut h, "Apply action");
    assert!(h.state().message().contains("duplicate physical binding"));
    assert_eq!(h.state().document(), &accepted); assert!(h.state().can_redo());
    assert_eq!(fs::read(&file).unwrap(), bytes);
    click(&mut h, "Redo map edit"); assert_eq!(h.state().document(), &redo);
    click(&mut h, "Undo map edit"); assert_eq!(h.state().document(), &accepted);
    // Pause already has its defaults. Restoring and applying is a true no-op,
    // preserving both the accepted document and the previously available redo.
    action(&mut h, "pause"); click(&mut h, "Restore action defaults");
    assert_combo(&h, "Binding 1", "Key P"); click(&mut h, "Apply action");
    assert!(h.state().message().starts_with("Action applied"));
    assert_eq!(h.state().document(), &accepted); assert!(h.state().can_redo());
    click(&mut h, "Redo map edit"); assert_eq!(h.state().document(), &redo);
    click(&mut h, "Undo map edit"); click(&mut h, "Undo map edit"); click(&mut h, "Undo map edit");
    assert_eq!(h.state().document(), &default_map()); assert!(!h.state().can_undo());
    assert_eq!(fs::read(&file).unwrap(), bytes);
}

#[test]
fn widget_conflicts_pending_drafts_and_empty_actions_preserve_contract() {
    let mut h = harness(); let initial = h.state().document().clone();
    assert!(h.get_by_label("Discard action draft").accesskit_node().is_disabled());
    action(&mut h, "right"); click(&mut h, "Restore action defaults");
    let command_status = h.state().message().to_owned(); assert!(!command_status.is_empty());
    click(&mut h, "Clear action draft"); click(&mut h, "Add binding");
    open_combo(&mut h, "Binding 1"); click(&mut h, "Key A");
    assert!(h.state().draft_error().unwrap().contains("duplicate physical binding"));
    h.state_mut().apply_document(initial.clone()).unwrap();
    assert!(h.state().draft_error().is_none(), "public document transition invalidates preview immediately");
    click(&mut h, "Clear action draft"); click(&mut h, "Add binding");
    open_combo(&mut h, "Binding 1"); click(&mut h, "Key A");
    h.get_by_label(&format!("Draft preview: {}", h.state().draft_error().unwrap()));
    assert_eq!(h.state().message(), command_status);
    assert_eq!(h.state().document(), &initial); assert!(!h.state().can_undo());
    open_combo(&mut h, "Binding 1"); click(&mut h, "Key L");
    assert!(h.state().draft_error().is_none()); assert_eq!(h.state().message(), command_status);
    open_combo(&mut h, "Binding 1"); click(&mut h, "Key A");
    assert_combo(&h, "Action", "right"); assert_combo(&h, "Binding 1", "Key A");
    click(&mut h, "Apply action");
    assert!(h.state().message().contains("duplicate physical binding"));
    assert_eq!(h.state().document(), &initial); assert!(!h.state().can_undo());
    let history = (h.state().can_undo(), h.state().can_redo());
    assert!(!h.get_by_label("Discard action draft").accesskit_node().is_disabled());
    click(&mut h, "Discard action draft");
    assert_combo(&h, "Action", "right");
    assert_combo(&h, "Binding 1", "Key D"); assert_combo(&h, "Binding 2", "Key ArrowRight");
    assert_eq!(h.state().document(), &initial);
    assert_eq!((h.state().can_undo(), h.state().can_redo()), history);
    assert!(h.state().message().is_empty());
    assert!(h.state().draft_error().is_none());
    assert!(h.get_by_label("Discard action draft").accesskit_node().is_disabled());
    // Recreate the original conflicting draft after the independent Discard
    // check: changing actions must itself discard a failed, pending draft.
    click(&mut h, "Clear action draft"); click(&mut h, "Add binding");
    open_combo(&mut h, "Binding 1"); click(&mut h, "Key A");
    assert_combo(&h, "Action", "right"); assert_combo(&h, "Binding 1", "Key A");
    click(&mut h, "Apply action");
    assert!(h.state().message().contains("duplicate physical binding"));
    assert_eq!(h.state().document(), &initial); assert!(!h.state().can_undo());
    action(&mut h, "left"); action(&mut h, "right");
    assert_combo(&h, "Binding 1", "Key D"); assert_combo(&h, "Binding 2", "Key ArrowRight");
    click(&mut h, "Apply action"); assert_eq!(h.state().document(), &initial);
    click(&mut h, "Clear action draft");
    assert!(h.state().draft_error().is_none()); assert_eq!(h.state().document(), &initial);
    click(&mut h, "Apply action");
    assert!(h.state().document().actions.iter().find(|a| a.name=="right").unwrap().bindings.is_empty());
    click(&mut h, "Undo map edit"); assert_eq!(h.state().document(), &initial);

    // Restore defaults creates a draft, but Discard must return to the current
    // accepted custom binding, preserving selection and an exact redo entry.
    set_action(&mut h, "right", "Key L");
    let accepted = h.state().document().clone();
    set_action(&mut h, "quit", "Key Q"); let redo = h.state().document().clone();
    click(&mut h, "Undo map edit"); assert_eq!(h.state().document(), &accepted); assert!(h.state().can_redo());
    action(&mut h, "right"); click(&mut h, "Restore action defaults");
    assert_combo(&h, "Binding 1", "Key D"); assert_combo(&h, "Binding 2", "Key ArrowRight");
    assert_eq!(h.state().document(), &accepted); assert!(h.state().can_redo());
    assert!(!h.get_by_label("Discard action draft").accesskit_node().is_disabled());
    click(&mut h, "Discard action draft");
    assert_combo(&h, "Action", "right"); assert_combo(&h, "Binding 1", "Key L");
    assert_eq!(h.state().document(), &accepted); assert!(h.state().can_redo());
    assert!(h.get_by_label("Discard action draft").accesskit_node().is_disabled());
    // Applying the discarded draft is a no-op, not a hidden extra edit.
    click(&mut h, "Apply action");
    assert_eq!(h.state().document(), &accepted); assert!(h.state().can_redo());
    click(&mut h, "Clear action draft"); click(&mut h, "Add binding");
    assert_combo(&h, "Binding 1", "Key Z");
    click(&mut h, "Discard action draft");
    assert_combo(&h, "Binding 1", "Key L");
    assert_eq!(h.state().document(), &accepted); assert!(h.state().can_redo());
    action(&mut h, "pause"); click(&mut h, "Restore action defaults");
    assert_combo(&h, "Binding 1", "Key P");
    assert!(h.get_by_label("Discard action draft").accesskit_node().is_disabled());
    assert_combo(&h, "Action", "pause");
    assert_eq!(h.state().document(), &accepted); assert!(h.state().can_redo());
    click(&mut h, "Redo map edit"); assert_eq!(h.state().document(), &redo);
    click(&mut h, "Undo map edit"); assert_eq!(h.state().document(), &accepted);
    click(&mut h, "Undo map edit");
    assert_eq!(h.state().document(), &default_map()); assert!(!h.state().can_undo());
}

#[test]
fn malformed_limits_and_failed_open_leave_document_and_redo_unchanged() {
    let mut p = Panel::default(); let original = p.document().clone();
    let mut changed = original.clone(); changed.actions[0].bindings=vec![key(Key::I)];
    p.apply_document(changed.clone()).unwrap(); assert!(p.undo());
    let mut invalid = Vec::new();
    let mut c=original.clone(); c.version=2; invalid.push(c);
    let mut c=original.clone(); c.actions.pop(); invalid.push(c);
    let mut c=original.clone(); c.actions[0].name="jump".into(); invalid.push(c);
    let mut c=original.clone(); c.actions[0].bindings=vec![Button::Gamepad{device_id:0,button:0}]; invalid.push(c);
    let mut c=original.clone(); c.actions[0].bindings=vec![Button::Mouse{button:8}]; invalid.push(c);
    let mut c=original.clone(); c.actions[0].bindings=vec![key(Key::Z);9]; invalid.push(c);
    for c in invalid { assert!(p.apply_document(c).is_err()); assert_eq!(p.document(), &original); assert!(p.can_redo()); }
    let dir=tempfile::tempdir().unwrap(); let bad=dir.path().join("bad.json");
    for bytes in [b"{".to_vec(),vec![b' ';orr_input::MAX_FILE_BYTES+1]] {
        fs::write(&bad,bytes).unwrap(); assert!(p.open(&bad).is_err()); assert_eq!(p.document(),&original); assert!(p.can_redo());
    }
    assert!(p.redo()); assert_eq!(p.document(),&changed);
    let mut too_many = original.clone(); too_many.actions[0].bindings=(0..8).map(|button|Button::Mouse{button}).collect();
    p.apply_document(too_many).unwrap(); // Exactly eight distinct supported bindings is legal.
    for n in 0..50 { let mut c=p.document().clone(); c.actions[0].bindings=vec![key(if n%2==0{Key::I}else{Key::J})]; p.apply_document(c).unwrap(); }
    let mut count=0; while p.undo(){count+=1;} assert_eq!(count,32);
    let mut count=0; while p.redo(){count+=1;} assert_eq!(count,32);

    // Independent CPU widget coverage: this regular input fixture tests Open,
    // not Save As publication. The original Save/roundtrip assertions above
    // remain mandatory and separate; this does not replace a failing Save test.
    let mut reordered = default_map(); reordered.actions.reverse();
    reordered.actions.iter_mut().find(|a|a.name=="right").unwrap().bindings = vec![key(Key::L)];
    reordered.actions.iter_mut().find(|a|a.name=="left").unwrap().bindings = vec![key(Key::I)];
    validate_map(&reordered).unwrap();
    let fixture = dir.path().join("reordered-input-fixture.json");
    let mut bytes = Vec::new(); reordered.save(&mut bytes).unwrap(); fs::write(&fixture, &bytes).unwrap();
    let mut h = harness(); path(&mut h, &fixture); click(&mut h, "Open map (undoable)");
    assert_eq!(h.state().document(), &reordered);
    action(&mut h, "right"); click(&mut h, "Restore action defaults");
    assert_combo(&h, "Binding 1", "Key D"); assert_combo(&h, "Binding 2", "Key ArrowRight");
    assert_eq!(h.state().document(), &reordered); assert!(!h.state().can_redo());
    assert_eq!(fs::read(&fixture).unwrap(), bytes);
    click(&mut h, "Undo map edit"); assert_eq!(h.state().document(), &default_map()); assert!(!h.state().can_undo());
    click(&mut h, "Redo map edit"); assert_eq!(h.state().document(), &reordered);
    assert_combo(&h, "Binding 1", "Key L");
    click(&mut h, "Restore action defaults"); click(&mut h, "Apply action");
    let mut restored = reordered.clone();
    restored.actions.iter_mut().find(|a|a.name=="right").unwrap().bindings = vec![key(Key::D), key(Key::ArrowRight)];
    assert_eq!(h.state().document(), &restored);
    click(&mut h, "Undo map edit"); assert_eq!(h.state().document(), &reordered);
    click(&mut h, "Redo map edit"); assert_eq!(h.state().document(), &restored);
    click(&mut h, "Undo map edit");
    set_action(&mut h, "left", "Key D");
    let conflicting = h.state().document().clone();
    set_action(&mut h, "quit", "Key Q"); let redo = h.state().document().clone();
    click(&mut h, "Undo map edit"); assert_eq!(h.state().document(), &conflicting); assert!(h.state().can_redo());
    action(&mut h, "right"); click(&mut h, "Restore action defaults");
    assert_combo(&h, "Binding 1", "Key D"); assert_combo(&h, "Binding 2", "Key ArrowRight");
    assert_eq!(h.state().document(), &conflicting); assert!(h.state().can_redo());
    click(&mut h, "Apply action"); assert!(h.state().message().contains("duplicate physical binding"));
    assert_eq!(h.state().document(), &conflicting); assert!(h.state().can_redo());
    action(&mut h, "pause"); click(&mut h, "Restore action defaults");
    assert_combo(&h, "Binding 1", "Key P"); click(&mut h, "Apply action");
    assert_eq!(h.state().document(), &conflicting); assert!(h.state().can_redo());
    click(&mut h, "Redo map edit"); assert_eq!(h.state().document(), &redo);
    assert_eq!(fs::read(&fixture).unwrap(), bytes);
}

#[cfg(unix)]
#[test]
fn symlinks_and_existing_destinations_are_rejected_without_replacement() {
    let dir=tempfile::tempdir().unwrap(); let file=dir.path().join("map.json");
    let p=Panel::default(); p.save_as(&file).unwrap(); let before=fs::read(&file).unwrap();
    let link=dir.path().join("link.json"); std::os::unix::fs::symlink(&file,&link).unwrap();
    let mut p=Panel::default(); assert!(p.open(&link).is_err()); assert!(p.save_as(&link).is_err());
    assert!(p.save_as(&file).is_err()); assert_eq!(fs::read(&file).unwrap(),before);
}

#[test]
fn authored_saved_actions_drive_production_controls_and_real_frame_trace() {
    let dir=tempfile::tempdir().unwrap(); let file=dir.path().join("map.json"); let mut h=authored();
    save_widgets(&mut h,&file); let map=ActionMap::load_file(&file).unwrap();
    // Every authored action crosses the production adapter; pause/quit are local control events.
    for (button, expected) in [(Key::I,Keys{left:true,..Keys::default()}),(Key::L,Keys{right:true,..Keys::default()}),
        (Key::U,Keys{up:true,..Keys::default()}),(Key::K,Keys{down:true,..Keys::default()})] {
        let mut c=ArenaControls::new(map.clone()).unwrap(); c.button(key(button),true,false,false); assert_eq!(c.keys(),expected);
    }
    let mut c=ArenaControls::new(map.clone()).unwrap(); c.button(Button::Mouse{button:0},true,false,false); assert!(c.keys().fire);
    assert!(c.button(key(Key::O),true,false,false).pause_changed); assert!(c.paused()); assert_eq!(c.keys(),Keys::default());
    c.button(key(Key::O),false,false,false); c.button(key(Key::O),true,false,false); assert!(!c.paused());
    assert!(c.button(key(Key::Q),true,false,false).quit);
    c.set_focused(false); c.set_focused(true); assert_eq!(c.keys(),Keys::default());
    c.button(key(Key::L),true,true,false); assert!(!c.keys().right);
    c.button(key(Key::L),true,false,true); assert!(!c.keys().right);
    c.set_ui_capture(true); c.button(key(Key::L),true,false,false); c.set_ui_capture(false); assert!(!c.keys().right);
    let make=||InProc::new(loopback_pair(Loopback::default()),arena_bridge_config());
    let (mut mapped,mut canonical,mut neutral)=(make(),make(),make());
    let mut controls=ArenaControls::new(map).unwrap(); let mut moved=false;
    for tick in 0..90 {
        if tick==0{controls.button(key(Key::L),true,false,false);}
        if tick==40{controls.set_focused(false);}
        controls.publish(&mut mapped).unwrap();
        canonical.set_input(PlayerSlot(0),Keys{right:tick<40,..Keys::default()}.to_input()).unwrap();
        for b in [&mut mapped,&mut canonical,&mut neutral]{b.update(Duration::from_nanos(16_666_667));}
        let a=mapped.poll_view(); let b=canonical.poll_view(); let n=neutral.poll_view();
        let checksum=a.snapshot.as_ref().unwrap().predicted().checksum();
        assert_eq!(checksum,b.snapshot.as_ref().unwrap().predicted().checksum());
        moved |= checksum!=n.snapshot.as_ref().unwrap().predicted().checksum();
    }
    assert!(moved,"remapped right action must actually change the Arena Frame trace");
}

#[test]
#[ignore = "mandatory acceptance: separately frozen input-actions Arena binary and evidence directory"]
fn authored_saved_map_is_accepted_by_actual_arena_cli() {
    use std::process::Command;
    let bin=std::path::PathBuf::from(std::env::var_os("ORR_INPUT_MAP_ARENA_BIN").expect("exact input-actions Arena producer required"));
    assert!(bin.is_absolute() && bin.is_file());
    let evidence=std::path::PathBuf::from(std::env::var_os("ORR_INPUT_MAP_EVIDENCE").expect("external retained evidence required"));
    assert!(evidence.is_absolute() && evidence.is_dir());
    let dir=tempfile::tempdir().unwrap(); let file=dir.path().join("authored.json"); let copy=dir.path().join("loaded.json");
    let mut h=authored(); save_widgets(&mut h,&file);
    let output=Command::new(bin).arg("--input-bindings").arg(&file).arg("--save-input-bindings").arg(&copy).output().unwrap();
    fs::write(evidence.join("arena-loader.stdout"),&output.stdout).unwrap(); fs::write(evidence.join("arena-loader.stderr"),&output.stderr).unwrap();
    fs::copy(&file,evidence.join("authored-map.json")).unwrap();
    assert!(output.status.success(),"{}\n{}\n{}",output.status,String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
    assert_eq!(ActionMap::load_file(&copy).unwrap(),*h.state().document()); assert_eq!(fs::read(&file).unwrap(),fs::read(copy).unwrap());
}

#[test]
#[ignore = "mandatory actual GUI GPU rendering with retained PNGs; no skip fallback"]
fn authored_input_map_widgets_have_real_gpu_pixels() {
    let evidence=std::path::PathBuf::from(std::env::var_os("ORR_INPUT_MAP_EVIDENCE").expect("external retained GPU evidence required"));
    assert!(evidence.is_absolute() && evidence.is_dir());
    let mut h=harness(); let before=h.render().expect("mandatory baseline GUI GPU readback");
    before.save(evidence.join("input-map-default.png")).unwrap();
    set_action(&mut h,"right","Key L"); let after=h.render().expect("mandatory authored GUI GPU readback");
    after.save(evidence.join("input-map-authored.png")).unwrap();
    assert_eq!(before.dimensions(),after.dimensions()); assert_ne!(before.as_raw(),after.as_raw());
    assert!(after.as_raw().as_chunks::<4>().0.iter().any(|p| p != &after.as_raw()[..4]),"nonblank GUI pixels required");
    assert_eq!(h.state().document().actions.iter().find(|a|a.name=="right").unwrap().bindings,vec![key(Key::L)]);
}
