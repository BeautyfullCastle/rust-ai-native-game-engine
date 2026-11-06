#![cfg(feature = "models")]
#![allow(clippy::float_arithmetic)]
use orr_editor::blockout::{Face, Recipe};

#[test]
fn portable_v1_palette_and_serialization_remain_unchanged() {
    let recipe = Recipe::default();
    let bytes = recipe.to_bytes().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 4);
    assert!(value.get("material_colors").is_none());
    assert_eq!(Recipe::from_bytes(&bytes).unwrap(), recipe);
    let model = recipe.bake_model(&bytes).unwrap();
    assert_eq!(model.source().materials[0].base_color, [1.0; 4]);
    assert_eq!(
        model.source().materials[1].base_color,
        [0.3, 0.65, 1.0, 1.0]
    );
}
#[test]
fn portable_v2_palette_roundtrip_bakes_both_opaque_materials() {
    let mut recipe = Recipe::default();
    recipe.version = 2;
    recipe.material_colors = Some([[0.8, 0.2, 0.1], [0.1, 0.8, 0.3]]);
    recipe.faces[Face::PosX.index()].material = 1;
    let bytes = recipe.to_bytes().unwrap();
    assert_eq!(Recipe::from_bytes(&bytes).unwrap(), recipe);
    let model = recipe.bake_model(&bytes).unwrap();
    assert_eq!(model.source().materials[0].base_color, [0.8, 0.2, 0.1, 1.0]);
    assert_eq!(model.source().materials[1].base_color, [0.1, 0.8, 0.3, 1.0]);
    assert_eq!(model.source().primitives[Face::PosX.index()].material, 1);
    assert_eq!(
        model.source().images[0].rgba8,
        orr_editor::blockout::checker().rgba8
    );
    assert_eq!(model.source().primitives.len(), 6);
}
#[test]
fn portable_colors_reject_schema_null_duplicates_and_invalid_rgb() {
    let mut recipe = Recipe::default();
    recipe.version = 2;
    assert!(recipe.validate().is_err());
    recipe.version = 1;
    recipe.material_colors = Some([[1.0; 3]; 2]);
    assert!(recipe.validate().is_err());
    recipe.version = 2;
    for value in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
        recipe.material_colors = Some([[value, 0.0, 0.0], [1.0; 3]]);
        assert!(recipe.to_bytes().is_err());
    }
    let text = String::from_utf8(Recipe::default().to_bytes().unwrap()).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
    value["material_colors"] = serde_json::Value::Null;
    assert!(Recipe::from_bytes(&serde_json::to_vec(&value).unwrap()).is_err());
    let duplicate = text.replacen("\"version\": 1,", "\"version\": 2, \"material_colors\": [[0,0,0],[1,1,1]], \"material_colors\": [[1,1,1],[0,0,0]],", 1);
    assert!(Recipe::from_bytes(duplicate.as_bytes()).is_err());
    value["version"] = serde_json::json!(2);
    value["material_colors"] = serde_json::json!([[1, 1, 1]]);
    assert!(Recipe::from_bytes(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn color_edits_are_atomic_undoable_and_save_reopen_without_losing_v1_bytes() {
    use orr_editor::blockout::Blockout;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut document = Blockout::create(&root, "colors.blockout.json").unwrap();
    document.save().unwrap();
    let original = std::fs::read(document.path()).unwrap();
    document.set_material_color(0, [1.0; 3]).unwrap();
    assert!(!document.dirty());
    assert_eq!(document.history_len(), (0, 0));
    document.set_material_color(0, [0.8, 0.2, 0.1]).unwrap();
    let edited = document.recipe().clone();
    let history = document.history_len();
    for slot in [2, 255] {
        assert!(document.set_material_color(slot, [0.0; 3]).is_err());
    }
    assert!(document
        .set_material_color(0, [f32::NAN, 0.0, 0.0])
        .is_err());
    assert_eq!(document.recipe(), &edited);
    assert_eq!(document.history_len(), history);
    document.undo();
    assert_eq!(document.recipe().version, 1);
    assert!(!document.dirty());
    document.save().unwrap();
    assert_eq!(std::fs::read(document.path()).unwrap(), original);
    document.redo();
    document.save().unwrap();
    let reopened = Blockout::open(&root, "colors.blockout.json").unwrap();
    assert_eq!(reopened.recipe(), &edited);
    assert_eq!(
        std::fs::read(document.path()).unwrap(),
        edited.to_bytes().unwrap()
    );
}
