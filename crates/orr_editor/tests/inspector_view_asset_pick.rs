//! A number field that is the id of an asset (`orr_gfx::ViewRef`): a combo box of the view assets
//! of the scene's folder, by name, instead of a raw number.
use std::path::PathBuf;

use egui::{accesskit::Role, Id};
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::inspector::{asset_id_choices, asset_id_label, set_asset_root, show_component, InspEvent};
use orr_reflect::{asset_id, FieldDesc, IntKind, TypeDesc, Value};

struct State {
    root: PathBuf,
    asset: u64,
    edits: Vec<InspEvent>,
}

/// A folder like a project: two view assets in a sub folder, and files that are not view assets.
fn project(name: &str, with_assets: bool) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("orr_inspector_view_assets_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("avatars")).unwrap();
    if with_assets {
        for file in ["avatars/blue.orrview", "avatars/red.orrview"] {
            std::fs::write(dir.join(file), r#"{"version":1,"model":"a.glb"}"#).unwrap();
        }
    }
    std::fs::write(dir.join("notes.txt"), "not an asset").unwrap();
    dir
}

fn harness(root: PathBuf, asset: u64) -> Harness<'static, State> {
    Harness::builder().with_size([500.0, 300.0]).build_ui_state(
        |ui, state: &mut State| {
            set_asset_root(ui.ctx(), Some(&state.root));
            let int = TypeDesc::int(IntKind::U64).with_asset_id("view");
            let desc = TypeDesc::structure(8, vec![FieldDesc::new("asset", 0, int)]);
            let value = Value::Struct(vec![("asset".into(), Value::Int(i128::from(state.asset)))]);
            let mut events = Vec::new();
            show_component(ui, Id::new("view_ref"), &desc, &value, &mut events);
            for event in &events {
                if let InspEvent::Set { value: Value::Int(n), .. } = event {
                    state.asset = u64::try_from(*n).unwrap();
                }
            }
            state.edits.extend(events);
        },
        State { root, asset, edits: Vec::new() },
    )
}

fn combo(h: &Harness<'_, State>) -> String {
    h.get_by_role(Role::ComboBox).value().unwrap_or_default()
}

#[test]
fn the_choices_are_the_files_by_name_with_the_id_of_their_path() {
    let files = vec!["avatars/blue.orrview".to_string(), "knight.orrview".to_string()];
    let choices = asset_id_choices(&files);
    assert_eq!(choices[0], (asset_id("avatars/blue.orrview"), "avatars/blue".to_string(), "avatars/blue.orrview".to_string()));
    assert_eq!(choices[1].1, "knight");
    assert_eq!(asset_id_label(&choices, 0), "(none)");
    assert_eq!(asset_id_label(&choices, i128::from(choices[1].0)), "knight");
    assert_eq!(asset_id_label(&choices, 12345), "missing asset (12345)", "an id with no file is shown, not hidden");
}

#[test]
fn the_field_shows_the_name_of_the_asset_and_not_its_number() {
    let root = project("shows", true);
    let mut h = harness(root.clone(), asset_id("avatars/red.orrview"));
    h.run();
    assert_eq!(combo(&h), "avatars/red");

    let mut none = harness(root.clone(), 0);
    none.run();
    assert_eq!(combo(&none), "(none)");

    let mut moved = harness(root.clone(), asset_id("avatars/gone.orrview"));
    moved.run();
    assert!(combo(&moved).starts_with("missing asset ("), "{}", combo(&moved));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn picking_a_view_asset_sets_the_id_of_its_path_and_one_pick_is_one_edit() {
    let root = project("picks", true);
    let mut h = harness(root.clone(), 0);
    h.run();
    h.get_by_role(Role::ComboBox).click();
    h.run();
    // Only the view assets are offered, by name.
    assert!(h.query_by_label("avatars/blue").is_some());
    assert!(h.query_by_label("notes").is_none() && h.query_by_label("notes.txt").is_none());
    h.get_by_label("avatars/red").click();
    h.run();

    assert_eq!(h.state().asset, asset_id("avatars/red.orrview"));
    assert_eq!(h.state().edits, vec![InspEvent::Set { path: "asset".into(), value: Value::Int(i128::from(asset_id("avatars/red.orrview"))) }]);
    assert_eq!(combo(&h), "avatars/red");

    // Back to none, and picking what is already set changes nothing.
    h.get_by_role(Role::ComboBox).click();
    h.run();
    h.get_by_label("(none)").click();
    h.run();
    assert_eq!(h.state().asset, 0);
    let before = h.state().edits.len();
    h.get_by_role(Role::ComboBox).click();
    h.run();
    h.get_by_label("(none)").click();
    h.run();
    assert_eq!(h.state().edits.len(), before);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_folder_without_view_assets_says_so() {
    let root = project("empty", false);
    let mut h = harness(root.clone(), 0);
    h.run();
    h.get_by_role(Role::ComboBox).click();
    h.run();
    assert!(h.query_by_label_contains("No .orrview files").is_some());
    let _ = std::fs::remove_dir_all(root);
}

/// With `ORR_SHOT_DIR` set, the open picker is also written as a picture there.
#[test]
fn the_open_picker_can_be_saved_as_a_picture() {
    let Ok(dir) = std::env::var("ORR_SHOT_DIR") else { return };
    let root = project("shot", true);
    let mut h = harness(root.clone(), asset_id("avatars/blue.orrview"));
    h.run();
    h.get_by_role(Role::ComboBox).click();
    h.run();
    let image = h.render().expect("the harness renders on the GPU");
    image.save(PathBuf::from(dir).join("inspector_view_asset_picker.png")).expect("the picture is saved");
    let _ = std::fs::remove_dir_all(root);
}
