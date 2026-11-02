//! Pure panel/document operations: no widget, renderer, disk Save, or Play claim.
use super::*;
use crate::{
    mesh_prop_prefab::Template,
    mesh_prop_shading::{Settings, Slot},
};
const REVISION: [u8; 32] = [0x53; 32];
fn art() -> Snapshot {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/imported_scene_demo")
        .canonicalize()
        .unwrap();
    Snapshot::from_package(&root, "foreground.glb", "LICENSE.txt", REVISION).unwrap()
}
fn settings(art: &Snapshot, metallic: f32) -> Settings {
    Settings::new(
        vec![Slot {
            material: art.source_model_arc().source().primitives[0].material,
            metallic,
            roughness: 0.25,
        }],
        art.source_model_arc(),
    )
    .unwrap()
}
fn panel(count: usize) -> Panel {
    let art = art();
    let doc = Document {
        props: (0..count)
            .map(|i| Prop {
                id: i as u64 + 1,
                name: format!("Existing {i}"),
                art: art.clone(),
                shading: Some(settings(&art, i as f32 / 10.0)),
            })
            .collect(),
    };
    let mut panel = Panel {
        graph_revision: Some(REVISION),
        ..Default::default()
    };
    panel.reset_history(doc, Some(0));
    panel.selected = Some(1);
    panel.copy_selection_fields();
    panel
}
fn template(art: &Snapshot) -> Template {
    Template::capture("Shaded prefab", art)
        .unwrap()
        .with_shading(Some(settings(art, 0.9)))
        .unwrap()
}
fn state(panel: &Panel) -> Vec<(u64, String, [u8; 32], Option<Settings>)> {
    panel
        .document
        .props
        .iter()
        .map(|p| {
            (
                p.id,
                p.name.clone(),
                p.art.render_identity(),
                p.shading.clone(),
            )
        })
        .collect()
}
#[test]
fn shaded_candidate_discard_then_explicit_update_undo_redo_preserve_appearance() {
    let mut panel = panel(1);
    let before = state(&panel);
    let template = template(&panel.document.props[0].art);
    let wanted = template.shading().cloned();
    panel.stage_prefab(template.clone()).unwrap();
    assert_eq!(state(&panel), before);
    assert_eq!(panel.history.len(), 1);
    assert!(panel.pending_prefab());
    assert_eq!(
        panel
            .shading
            .settings(template.art().source_model_arc())
            .unwrap(),
        wanted
    );
    panel.copy_selection_fields(); // Same operation as candidate Discard.
    assert!(!panel.pending_prefab());
    assert_eq!(state(&panel), before);
    assert_eq!(
        panel
            .shading
            .settings(panel.selected_prop().unwrap().art.source_model_arc())
            .unwrap(),
        before[0].3
    );
    panel.stage_prefab(template).unwrap();
    panel.update_selected();
    assert!(panel.error.is_none(), "{:?}", panel.error);
    assert_eq!(panel.document.props[0].shading, wanted);
    assert_eq!(panel.document.props[0].art.render_identity(), before[0].2);
    assert_eq!(panel.history.len(), 2);
    let after = state(&panel);
    panel.undo();
    assert_eq!(state(&panel), before);
    panel.redo();
    assert_eq!(state(&panel), after);
}
#[test]
fn explicit_add_and_duplicate_preserve_shaded_template_without_mutating_original() {
    let mut panel = panel(1);
    let before = state(&panel);
    let template = template(&panel.document.props[0].art);
    let wanted = template.shading().cloned();
    panel.stage_prefab(template).unwrap();
    assert_eq!(state(&panel), before);
    panel.add_prop();
    assert!(panel.error.is_none(), "{:?}", panel.error);
    assert_eq!(panel.document.props.len(), 2);
    assert_eq!(state(&panel)[0], before[0]);
    assert_eq!(panel.document.props[1].shading, wanted);
    panel.duplicate_selected_prop();
    assert!(panel.error.is_none(), "{:?}", panel.error);
    assert_eq!(panel.document.props.len(), 3);
    assert_eq!(panel.document.props[2].shading, wanted);
    assert_eq!(
        panel.document.props[2].art.render_identity(),
        panel.document.props[1].art.render_identity()
    );
    let after = state(&panel);
    panel.undo();
    assert_eq!(panel.document.props.len(), 2);
    panel.undo();
    assert_eq!(state(&panel), before);
    panel.redo();
    panel.redo();
    assert_eq!(state(&panel), after);
}
#[test]
fn seventh_shaded_appearance_add_and_update_fail_atomically_with_candidate_retained() {
    let mut panel = panel(6);
    // Duplicate first appearance: replacing it leaves its old appearance alive,
    // so Update as well as Add would require seven distinct appearances.
    panel.duplicate_selected_prop();
    assert!(panel.error.is_none(), "{:?}", panel.error);
    let before = state(&panel);
    let history = panel.history.len();
    let cursor = panel.cursor;
    let template = template(&panel.document.props[0].art);
    panel.stage_prefab(template.clone()).unwrap();
    panel.add_prop();
    assert!(panel.error.as_ref().unwrap().contains("six-appearance"));
    assert_eq!(state(&panel), before);
    assert_eq!((panel.history.len(), panel.cursor), (history, cursor));
    assert!(panel.pending_prefab());
    panel.stage_prefab(template).unwrap();
    panel.update_selected();
    assert!(panel.error.as_ref().unwrap().contains("six-appearance"));
    assert_eq!(state(&panel), before);
    assert_eq!((panel.history.len(), panel.cursor), (history, cursor));
    assert!(panel.pending_prefab());
}
