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

#[test]
fn tint_errors_identify_every_channel_and_preserve_all_reader_validation() {
    let model = fixture_model();
    for (channel, label) in ["R", "G", "B", "A"].into_iter().enumerate() {
        for (text, reason, explanation) in [
            (
                "not-a-number",
                TintFieldReason::InvalidNumber,
                "enter a number in 0..=1",
            ),
            (
                "",
                TintFieldReason::InvalidNumber,
                "enter a number in 0..=1",
            ),
            (
                " 0.5",
                TintFieldReason::InvalidNumber,
                "enter a number in 0..=1",
            ),
            (
                "NaN",
                TintFieldReason::NonFinite,
                "value must be finite and within 0..=1",
            ),
            (
                "inf",
                TintFieldReason::NonFinite,
                "value must be finite and within 0..=1",
            ),
            (
                "-inf",
                TintFieldReason::NonFinite,
                "value must be finite and within 0..=1",
            ),
            (
                "1e100",
                TintFieldReason::NonFinite,
                "value must be finite and within 0..=1",
            ),
            (
                "-0.01",
                TintFieldReason::OutOfRange,
                "value must be within 0..=1",
            ),
            (
                "1.01",
                TintFieldReason::OutOfRange,
                "value must be within 0..=1",
            ),
        ] {
            let mut panel = Panel::default();
            panel.load(&model, &[]).unwrap();
            panel.fields[1].tint[channel] = text.into();
            let expected = TintFieldError {
                material: 1,
                channel,
                reason,
            };
            assert_eq!(panel.fields[1].settings().unwrap_err(), expected, "{text}");
            assert_eq!(panel.invalid_tint(), Some(expected));
            let message = format!("Material slot 1 tint {label}: {explanation}");
            assert_eq!(expected.to_string(), message);
            assert_eq!(panel.current_overrides(&model).unwrap_err(), message);
            assert_eq!(panel.current_all_slots().unwrap_err(), message);
            assert!(panel.current_cached().is_none());
            assert!(panel.dirty());
            assert!(panel.staged.is_none());
        }
    }
}

#[test]
fn tint_error_order_is_material_then_channel_not_selection_or_field_order() {
    let model = fixture_model();
    let mut panel = Panel::default();
    panel.load(&model, &[]).unwrap();
    panel.fields[1].tint[0] = "bad".into();
    panel.fields[0].tint[3] = "inf".into();
    panel.fields[0].tint[1] = "2".into();
    panel.selected = Some(1);
    panel.fields.reverse();
    assert_eq!(
        panel.invalid_tint(),
        Some(TintFieldError {
            material: 0,
            channel: 1,
            reason: TintFieldReason::OutOfRange,
        })
    );
    let field = panel
        .fields
        .iter_mut()
        .find(|field| field.material == 0)
        .unwrap();
    field.tint[1] = "1".into();
    assert_eq!(
        panel.invalid_tint(),
        Some(TintFieldError {
            material: 0,
            channel: 3,
            reason: TintFieldReason::NonFinite,
        })
    );
    panel
        .fields
        .iter_mut()
        .find(|field| field.material == 0)
        .unwrap()
        .tint[3] = "1".into();
    assert_eq!(
        panel.invalid_tint(),
        Some(TintFieldError {
            material: 1,
            channel: 0,
            reason: TintFieldReason::InvalidNumber,
        })
    );
}

#[test]
fn valid_tint_boundaries_and_numeric_spellings_keep_sparse_semantics() {
    let model = fixture_model();
    let mut panel = Panel::default();
    panel.load(&model, &[]).unwrap();
    panel.fields[1].tint = ["1.0", "1e0", "+1", "1"].map(str::to_owned);
    assert!(panel.invalid_tint().is_none());
    assert!(panel.current_overrides(&model).unwrap().is_empty());
    panel.fields[1].tint = ["0", "-0", "5e-1", "1"].map(str::to_owned);
    assert!(panel.invalid_tint().is_none());
    let overrides = panel.current_overrides(&model).unwrap();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].material, 1);
    assert_eq!(
        overrides[0].tint.map(f32::to_bits),
        [0.0_f32, -0.0, 0.5, 1.0].map(f32::to_bits)
    );
    assert!(same_settings(
        &panel.current_all_slots().unwrap(),
        &panel.current_cached().unwrap()
    ));
}

#[test]
fn hidden_invalid_slot_button_only_selects_then_repair_enables_sparse_stage() {
    use egui_kittest::{
        kittest::{NodeT, Queryable},
        Harness,
    };

    let model = fixture_model();
    let source = model.source().clone();
    let mut panel = Panel::default();
    panel.load(&model, &[]).unwrap();
    let baseline = panel.baseline.clone();
    panel.staged = Some(Staged {
        overrides: Vec::new(),
        all_slots: baseline.clone(),
    });
    panel.fields[1].tint[2] = "NaN".into();
    assert_eq!(panel.selected, Some(0));
    let shown_model = Arc::clone(&model);
    let mut harness = Harness::builder().with_size([900.0, 800.0]).build_ui_state(
        move |ui, state: &mut (Panel, Vec<Vec<crate::mesh_material_slots::Settings>>)| {
            if let Some(staged) = state.0.show(ui, &shown_model) {
                state.1.push(staged);
            }
        },
        (panel, Vec::new()),
    );
    harness.run_steps(2);
    assert!(harness
        .query_by_label("Material slot 1 tint B: value must be finite and within 0..=1")
        .is_some());
    assert!(harness
        .get_by_label("Stage material overrides")
        .accesskit_node()
        .is_disabled());
    harness
        .get_by_label("Stage material overrides")
        .click_accesskit();
    harness.run_steps(2);
    assert!(harness.state().1.is_empty());
    harness.get_by_label("Show invalid material slot").click();
    harness.run_steps(2);
    let state = harness.state();
    assert_eq!(state.0.selected, Some(1));
    assert_eq!(state.0.fields[1].tint[2], "NaN");
    assert!(state.1.is_empty());
    assert!(same_settings(&state.0.baseline, &baseline));
    assert!(state.0.baseline_overrides.is_empty());
    assert!(state.0.staged.as_ref().unwrap().overrides.is_empty());
    assert!(same_settings(
        &state.0.staged.as_ref().unwrap().all_slots,
        &baseline
    ));
    assert!(!state.0.staged_matches_fields());
    assert!(state.0.model_matches(&model));
    assert_eq!(model.source(), &source);

    harness.state_mut().0.fields[1].tint[2] = "0.25".into();
    harness.run_steps(2);
    assert!(harness
        .query_by_label("Show invalid material slot")
        .is_none());
    assert!(!harness
        .get_by_label("Stage material overrides")
        .accesskit_node()
        .is_disabled());
    assert!(harness.state().0.error.is_none());
    harness.get_by_label("Stage material overrides").click();
    harness.run_steps(2);
    let state = harness.state();
    assert_eq!(state.1.len(), 1);
    let overrides = &state.1[0];
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].material, 1);
    assert_eq!(overrides[0].tint, [1.0, 1.0, 0.25, 1.0]);
    assert!(state.0.staged_matches_fields());
    assert!(same_settings(&state.0.baseline, &baseline));
    assert!(state.0.baseline_overrides.is_empty());
    assert_eq!(model.source(), &source);
}
