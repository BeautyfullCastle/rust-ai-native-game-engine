use super::*;
use orr_model::{
    Dependency, Image, Material, ModelSource, Primitive, StaticModel, Vertex, Wrap, IDENTITY,
};
use orr_render::{MaterialShading, ScalarMaterial};
use std::sync::Arc;
fn model() -> Arc<StaticModel> {
    Arc::new(
        StaticModel::new(ModelSource {
            format: "orr_static_model".into(),
            version: 1,
            asset_id: "fixture/model.glb".into(),
            dependencies: vec![Dependency {
                uri: "$source".into(),
                sha256: "0".repeat(64),
            }],
            primitives: vec![Primitive {
                id: "fixture/model.glb#node=0".into(),
                vertices: vec![
                    vertex([-1.0, -1.0, -1.0]),
                    vertex([1.0, 1.0, 1.0]),
                    vertex([-1.0, 1.0, 1.0]),
                ],
                indices: vec![0, 1, 2],
                material: 0,
                transform: IDENTITY,
            }],
            materials: vec![Material {
                base_color: [1.0; 4],
                image: 0,
                linear_filter: true,
                wrap_s: Wrap::Clamp,
                wrap_t: Wrap::Clamp,
            }],
            images: vec![Image {
                width: 1,
                height: 1,
                rgba8: vec![255; 4],
            }],
        })
        .unwrap(),
    )
}

fn vertex(position: [f32; 3]) -> Vertex {
    Vertex {
        position,
        normal: [0.0, 1.0, 0.0],
        uv: [0.0, 0.0],
    }
}

fn scene(model: &Arc<StaticModel>, value: Option<f32>) -> SceneModel {
    SceneModel {
        model: model.clone(),
        instance: Default::default(),
        shading: value.map(|metallic| {
            vec![MaterialShading::MetallicRoughness(
                ScalarMaterial::new(metallic, 0.5).unwrap(),
            )]
        }),
    }
}
#[test]
fn shared_arc_distinct_shading_groups_coexist_and_undo_keys_match_only_original() {
    let model = model();
    let original = scene(&model, Some(0.0));
    let changed = scene(&model, Some(1.0));
    let legacy = scene(&model, None);
    let groups =
        static_groups(&[], &[original, changed, legacy, scene(&model, Some(0.0))]).unwrap();
    assert_eq!(groups.len(), 3);
    assert_eq!(groups[0].1.len(), 2);
    assert!(!groups[0].0.matches(&groups[1].0));
    assert!(!groups[0].0.matches(&groups[2].0));
    let undo = static_groups(&[], &[scene(&model, Some(0.0))]).unwrap();
    assert!(groups[0].0.matches(&undo[0].0));
    let different_arc = Arc::new((*model).clone());
    assert!(!groups[0]
        .0
        .matches(&static_groups(&[], &[scene(&different_arc, Some(0.0))]).unwrap()[0].0));
}
#[test]
fn invalid_table_and_ninth_appearance_reject_before_gpu_allocation() {
    let model = model();
    let mut invalid = scene(&model, Some(0.0));
    invalid.shading = Some(vec![]);
    assert!(static_groups(&[], &[invalid]).is_err());
    let appearances: Vec<_> = (0..8)
        .map(|i| scene(&model, Some(i as f32 / 8.0)))
        .collect();
    assert_eq!(static_groups(&[], &appearances).unwrap().len(), 8);
    let mut nine = appearances;
    nine.push(scene(&model, Some(1.0)));
    assert!(static_groups(&[], &nine).err().unwrap().contains("eight"));
}
#[test]
fn entity_model_stays_legacy_and_explicit_legacy_table_is_not_silently_upgraded() {
    let model = model();
    let entity = ModelPlacement {
        entity: Entity {
            index: 0,
            version: 0,
        },
        model: model.clone(),
        instance: Default::default(),
    };
    let explicit = SceneModel {
        model: model.clone(),
        instance: Default::default(),
        shading: Some(vec![MaterialShading::LegacyDiffuse]),
    };
    let groups = static_groups(&[entity], &[scene(&model, None), explicit]).unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].1.len(), 2);
    assert!(groups[0].0.shading.is_none());
}
#[test]
#[ignore = "requires actual GPU; proves cache/table and rejection atomicity, not physical display"]
fn actual_gpu_cache_preserves_shading_identity_across_hdr_and_rejection() {
    let rhi = Wgpu::headless(Default::default()).expect("actual GPU required");
    let size = (128, 128);
    let mut viewport = Viewport3dGpu::new(&rhi, size);
    let model = model();
    let list = RenderList3D::default();
    let camera = Camera3D::orthographic([0.0, 4.0, 5.0], [0.0; 3], 3.0);
    let render = |v: &mut Viewport3dGpu, scene: &[SceneModel], hdr: bool| {
        v.render_scene(
            size,
            &list,
            &camera,
            &[],
            scene,
            #[cfg(feature = "animated-models")]
            &[],
            orr_render::PostProcessSettings {
                enabled: hdr,
                ..Default::default()
            },
            #[cfg(feature = "irradiance-probes")]
            None,
        )
    };
    for hdr in [false, true, false] {
        render(
            &mut viewport,
            &[scene(&model, Some(0.0)), scene(&model, Some(1.0))],
            hdr,
        )
        .unwrap();
        assert_eq!(viewport.models.len(), 2);
        for (i, entry) in viewport.models.iter().enumerate() {
            assert_eq!(
                entry.renderer.material_shading(),
                entry.model.shading.as_ref().unwrap()
            );
            assert_eq!(entry.model.shading, scene(&model, Some(i as f32)).shading);
        }
        let before = viewport.read_rgba8();
        let mut bad = scene(&model, Some(0.0));
        bad.shading = Some(vec![]);
        assert!(render(&mut viewport, &[bad], hdr).is_err());
        assert_eq!(viewport.models.len(), 2);
        assert_eq!(viewport.read_rgba8(), before);
        render(&mut viewport, &[scene(&model, Some(0.0))], hdr).unwrap();
        assert_eq!(viewport.models.len(), 1);
        assert_eq!(
            viewport.models[0].renderer.material_shading(),
            scene(&model, Some(0.0)).shading.as_ref().unwrap()
        );
        render(&mut viewport, &[scene(&model, None)], hdr).unwrap();
        assert_eq!(
            viewport.models[0].renderer.material_shading(),
            &[MaterialShading::LegacyDiffuse]
        );
    }
}
