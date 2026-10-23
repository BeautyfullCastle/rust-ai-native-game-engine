use super::*;
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

#[test]
fn loaded_defaults_are_clean_and_stage_only_the_changed_used_slot() {
    let model = fixture_model();
    let mut panel = Panel::default();
    panel.load(&model, &[]).unwrap();
    assert!(!panel.dirty());
    assert_eq!(panel.fields.len(), 2);
    assert_eq!(panel.fields[0].wrap_s, Wrap::Clamp);
    assert_eq!(panel.fields[1].wrap_s, Wrap::Mirror);

    panel.fields[0].tint[0] = "0.25".into();
    let overrides = panel.current_overrides(&model).unwrap();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].material, 0);
    assert_eq!(overrides[0].tint, [0.25, 1.0, 1.0, 1.0]);
    assert!(!overrides[0].linear_filter);
    assert_eq!(overrides[0].wrap_s, Wrap::Clamp);
    assert_eq!(overrides[0].wrap_t, Wrap::Repeat);
    assert!(panel.dirty());

    panel.staged = Some(Staged {
        overrides: overrides.clone(),
        all_slots: panel.current_all_slots().unwrap(),
    });
    panel.acknowledge();
    assert!(!panel.dirty());
}

#[test]
fn sampler_changes_are_sparse_and_invalid_values_remain_dirty() {
    let model = fixture_model();
    let mut panel = Panel::default();
    panel.load(&model, &[]).unwrap();
    panel.fields[1].linear_filter = false;
    panel.fields[1].wrap_t = Wrap::Mirror;
    let overrides = panel.current_overrides(&model).unwrap();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].material, 1);
    assert!(!overrides[0].linear_filter);
    assert_eq!(overrides[0].wrap_t, Wrap::Mirror);

    panel.fields[0].tint[2] = "NaN".into();
    assert!(panel.current_overrides(&model).is_err());
    assert!(panel.dirty());
}

#[test]
fn reset_stages_loaded_applied_values_until_parent_apply_acknowledges_them() {
    let model = fixture_model();
    let applied = crate::mesh_material_slots::Settings {
        material: 1,
        tint: [0.5, 0.75, 1.0, 1.0],
        linear_filter: false,
        wrap_s: Wrap::Repeat,
        wrap_t: Wrap::Mirror,
    };
    let mut panel = Panel::default();
    panel.load(&model, std::slice::from_ref(&applied)).unwrap();
    panel.fields[1].tint[1] = "0.2".into();
    assert!(panel.dirty());

    let staged = panel.reset_to_baseline().unwrap();
    assert_eq!(staged.len(), 1);
    assert_eq!(staged[0].material, applied.material);
    assert_eq!(staged[0].tint, applied.tint);
    assert_eq!(staged[0].linear_filter, applied.linear_filter);
    assert_eq!(staged[0].wrap_s, applied.wrap_s);
    assert_eq!(staged[0].wrap_t, applied.wrap_t);
    assert!(panel.dirty());
    assert_eq!(panel.fields[1].tint, ["0.5", "0.75", "1", "1"]);

    panel.acknowledge();
    assert!(!panel.dirty());
}

#[test]
fn changes_after_stage_require_an_explicit_restage_before_parent_apply() {
    let model = fixture_model();
    let mut panel = Panel::default();
    panel.load(&model, &[]).unwrap();
    panel.fields[0].tint[0] = "0.4".into();
    let first_stage = panel.current_overrides(&model).unwrap();
    panel.staged = Some(Staged {
        overrides: first_stage,
        all_slots: panel.current_all_slots().unwrap(),
    });
    assert!(panel.staged_matches_fields());

    panel.fields[0].tint[0] = "0.6".into();
    assert!(!panel.staged_matches_fields());
    let second_stage = panel.current_overrides(&model).unwrap();
    panel.staged = Some(Staged {
        overrides: second_stage,
        all_slots: panel.current_all_slots().unwrap(),
    });
    assert!(panel.staged_matches_fields());
}

#[test]
fn show_rejects_a_different_model_arc_even_when_used_indices_match() {
    let loaded_model = fixture_model();
    let replacement_model = fixture_model();
    let mut panel = Panel::default();
    panel.load(&loaded_model, &[]).unwrap();
    let original_fields = panel.fields.clone();
    let context = egui::Context::default();
    let _ = context.run_ui(egui::RawInput::default(), |ui| {
        assert!(panel.show(ui, &replacement_model).is_none());
    });
    assert_eq!(
        panel.error.as_deref(),
        Some("The admitted model changed; reload this panel")
    );
    assert!(panel
        .fields
        .iter()
        .zip(original_fields)
        .all(|(left, right)| {
            left.material == right.material
                && left.tint == right.tint
                && left.linear_filter == right.linear_filter
                && left.wrap_s == right.wrap_s
                && left.wrap_t == right.wrap_t
        }));
    assert!(!panel.model_matches(&replacement_model));
    assert!(panel.model_matches(&loaded_model));
}

#[test]
fn duplicate_unused_and_excessive_overrides_are_rejected_before_load_mutates() {
    let model = fixture_model();
    let mut panel = Panel::default();
    panel.load(&model, &[]).unwrap();
    let original_fields = panel.fields.clone();
    let settings = crate::mesh_material_slots::Settings {
        material: 0,
        tint: [0.5, 1.0, 1.0, 1.0],
        linear_filter: false,
        wrap_s: Wrap::Clamp,
        wrap_t: Wrap::Repeat,
    };
    assert!(panel.load(&model, &[settings, settings]).is_err());
    let unused = crate::mesh_material_slots::Settings {
        material: 2,
        ..settings
    };
    assert!(panel.load(&model, &[unused]).is_err());
    assert!(panel.load(&model, &vec![settings; 257]).is_err());
    assert_eq!(panel.fields.len(), original_fields.len());
    assert!(panel
        .fields
        .iter()
        .zip(original_fields)
        .all(|(left, right)| {
            left.material == right.material
                && left.tint == right.tint
                && left.linear_filter == right.linear_filter
                && left.wrap_s == right.wrap_s
                && left.wrap_t == right.wrap_t
        }));
}
