use super::super::*;
use egui_kittest::{kittest::Queryable, Harness};
use orr_model::{Dependency, Image, Material, ModelSource, Primitive, Vertex};
use std::sync::Arc;

fn fixture_model() -> Arc<StaticModel> {
    let vertex = Vertex {
        position: [0.0, 0.0, 0.0],
        normal: [0.0, 0.0, 1.0],
        uv: [0.0, 0.0],
    };
    let primitive = |material, suffix: &str| Primitive {
        id: format!("slot-model.glb#node=0/mesh=0/primitive={suffix}"),
        vertices: vec![vertex; 3],
        indices: vec![0, 1, 2],
        material,
        transform: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    Arc::new(
        StaticModel::new(ModelSource {
            format: "orr_static_model".into(),
            version: 1,
            asset_id: "slot-model.glb".into(),
            dependencies: vec![Dependency {
                uri: "$source".into(),
                sha256: "0".repeat(64),
            }],
            primitives: vec![primitive(0, "0"), primitive(1, "1")],
            materials: vec![
                Material {
                    base_color: [1.0; 4],
                    image: 0,
                    linear_filter: false,
                    wrap_s: Wrap::Clamp,
                    wrap_t: Wrap::Repeat,
                },
                Material {
                    base_color: [0.5, 0.75, 1.0, 1.0],
                    image: 0,
                    linear_filter: true,
                    wrap_s: Wrap::Mirror,
                    wrap_t: Wrap::Clamp,
                },
            ],
            images: vec![Image {
                width: 1,
                height: 1,
                rgba8: vec![255; 4],
            }],
        })
        .unwrap(),
    )
}

fn fields(panel: &Panel) -> Vec<(u32, [String; 4], bool, Wrap, Wrap)> {
    panel
        .fields
        .iter()
        .map(|s| {
            (
                s.material,
                s.tint.clone(),
                s.linear_filter,
                s.wrap_s,
                s.wrap_t,
            )
        })
        .collect()
}

#[test]
fn actual_copy_is_snapshot_paste_only_edits_destination_then_explicit_stage() {
    let model = fixture_model();
    let source = model.source().clone();
    let mut panel = Panel::default();
    panel.load(&model, &[]).unwrap();
    panel.fields[0].tint = ["0.125".into(), "0.25".into(), "0.5".into(), "0.75".into()];
    panel.fields[0].linear_filter = true;
    panel.fields[0].wrap_s = Wrap::Repeat;
    panel.fields[0].wrap_t = Wrap::Mirror;
    let copied = panel.fields[0].settings().unwrap();
    let baseline = panel.baseline.clone();
    let mut h = Harness::builder()
        .with_size([1100.0, 1200.0])
        .build_ui_state(
            |ui,
             state: &mut (
                Panel,
                Arc<StaticModel>,
                Vec<Vec<crate::mesh_material_slots::Settings>>,
            )| {
                if let Some(value) = state.0.show(ui, &state.1) {
                    state.2.push(value);
                }
            },
            (panel, model, Vec::new()),
        );
    h.run_steps(2);
    h.get_by_label("Copy material slot settings").click();
    h.run_steps(3);
    assert!(h.state().0.copied.is_some());
    assert!(h.state().2.is_empty());
    h.get_by_label("Stage material overrides").click();
    h.run_steps(3);
    assert!(h.state().0.staged_matches_fields());
    let staged = h.state().0.staged.as_ref().unwrap().all_slots.clone();
    {
        let panel = &mut h.state_mut().0;
        panel.fields[0].tint[0] = "0.875".into();
        panel.selected = Some(0);
        panel.fields[1].tint[2] = "NaN".into();
    }
    h.run_steps(3);
    assert!(h.state().0.invalid_tint().is_some());
    h.get_by_label("Show invalid material slot").click();
    h.run_steps(3);
    assert_eq!(h.state().0.selected, Some(1));
    h.get_by_label("Paste material slot settings").click();
    h.run_steps(3);
    let panel = &h.state().0;
    let destination = panel.fields[1].settings().unwrap();
    assert_eq!(destination.material, 1);
    assert_eq!(destination.tint, copied.tint);
    assert_eq!(destination.linear_filter, copied.linear_filter);
    assert_eq!(destination.wrap_s, copied.wrap_s);
    assert_eq!(destination.wrap_t, copied.wrap_t);
    assert_eq!(panel.fields[0].tint[0], "0.875");
    assert_eq!(panel.baseline, baseline);
    assert_eq!(panel.staged.as_ref().unwrap().all_slots, staged);
    assert!(!panel.staged_matches_fields());
    assert!(panel.invalid_tint().is_none());
    assert_eq!(h.state().2.len(), 1);
    assert_eq!(h.state().1.source(), &source);
    h.get_by_label("Stage material overrides").click();
    h.run_steps(3);
    assert_eq!(h.state().2.len(), 2);
    assert!(h.state().0.staged_matches_fields());
    let overrides = h.state().2.last().unwrap();
    assert_eq!(overrides.len(), 2);
    assert_eq!(overrides[1].material, 1);
    assert_eq!(overrides[1].tint, copied.tint);
    assert_eq!(h.state().1.source(), &source);
}

#[test]
fn invalid_copy_preserves_previous_clipboard_fields_and_staged_baseline() {
    let model = fixture_model();
    let mut panel = Panel::default();
    panel.load(&model, &[]).unwrap();
    panel.copy_selected(&model).unwrap();
    let copied = panel.fields[0].settings().unwrap();
    let baseline = panel.baseline.clone();
    for invalid in ["bad", "NaN", "inf", "-0.1", "1.1"] {
        panel.selected = Some(0);
        panel.fields[0].tint[0] = invalid.into();
        let before = fields(&panel);
        let dirty = panel.dirty();
        assert!(panel.copy_selected(&model).is_err());
        assert_eq!(fields(&panel), before);
        assert_eq!(panel.dirty(), dirty);
        assert_eq!(panel.baseline, baseline);
        assert!(panel.staged.is_none());
        panel.selected = Some(1);
        panel.paste_selected(&model).unwrap();
        assert_eq!(panel.fields[1].settings().unwrap().tint, copied.tint);
    }
}

#[test]
fn missing_selection_empty_clipboard_and_stale_model_refuse_without_field_edits() {
    let model = fixture_model();
    let mut panel = Panel::default();
    panel.load(&model, &[]).unwrap();
    let before = fields(&panel);
    assert!(panel.paste_selected(&model).is_err());
    assert_eq!(fields(&panel), before);
    panel.selected = None;
    assert!(panel.copy_selected(&model).is_err());
    assert_eq!(fields(&panel), before);
    panel.selected = Some(0);
    panel.copy_selected(&model).unwrap();
    let other = fixture_model();
    assert!(!Arc::ptr_eq(&model, &other));
    assert!(panel.copy_selected(&other).is_err());
    assert_eq!(fields(&panel), before);
    assert!(panel.paste_selected(&other).is_err());
    assert_eq!(fields(&panel), before);
    assert!(panel.staged.is_none());
    assert!(!panel.dirty());
}

#[test]
fn reload_and_model_replacement_clear_clipboard_without_touching_model_textures() {
    let model = fixture_model();
    let original = model.source().clone();
    let mut panel = Panel::default();
    panel.load(&model, &[]).unwrap();
    panel.copy_selected(&model).unwrap();
    assert!(panel.copied.is_some());
    panel.load(&model, &[]).unwrap();
    assert!(panel.copied.is_none());
    assert!(panel.paste_selected(&model).is_err());
    panel.copy_selected(&model).unwrap();
    let replacement = fixture_model();
    panel.load(&replacement, &[]).unwrap();
    assert!(panel.copied.is_none());
    assert!(panel.paste_selected(&replacement).is_err());
    assert_eq!(model.source(), &original);
    assert_eq!(replacement.source(), &original);
    assert_eq!(
        panel.fields.iter().map(|s| s.material).collect::<Vec<_>>(),
        vec![0, 1]
    );
}
