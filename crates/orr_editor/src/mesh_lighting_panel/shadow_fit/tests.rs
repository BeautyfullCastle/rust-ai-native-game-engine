use super::*;
use orr_model::{
    Dependency, Image, Material, ModelSource, Primitive, StaticModel, Vertex, Wrap, IDENTITY,
};
use orr_render::{StaticInstance, IDENTITY_ROT};
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

fn scene_model(translation: [f32; 3]) -> SceneModel {
    SceneModel {
        #[cfg(feature = "mesh-prop-shading")]
        shading: None,
        model: model(),
        instance: StaticInstance {
            translation,
            rotation: IDENTITY_ROT,
            scale: [1.0; 3],
        },
    }
}

fn graph() -> StaticMeshGraph {
    StaticMeshGraph::bake(
        &[
            [FP::from_raw(0); 3],
            [FP::from_raw(65536), FP::from_raw(0), FP::from_raw(0)],
            [FP::from_raw(0), FP::from_raw(0), FP::from_raw(65536)],
        ],
        &[[0, 2, 1]],
        Default::default(),
    )
    .unwrap()
}
#[test]
fn translated_scaled_and_oblique_fit_contains_actual_bounds() {
    let mut prop = scene_model([40.0, 12.0, -30.0]);
    prop.instance.scale = [-3.0, 2.0, 4.0];
    assert!(mesh_prop_view::bounds(&prop).is_err());
    prop.instance.scale = [3.0, 2.0, 4.0];
    prop.instance.rotation = [
        0.0,
        std::f32::consts::FRAC_1_SQRT_2,
        0.0,
        std::f32::consts::FRAC_1_SQRT_2,
    ];
    let mut source = prop.model.source().clone();
    source.primitives[0].transform[0][0] = -1.0;
    source.primitives[0].transform[3][0] = 2.0;
    prop.model = Arc::new(StaticModel::new(source).unwrap());
    let bounds = mesh_prop_view::bounds(&prop).unwrap();
    let lighting = Lighting {
        direction: [0.3, -0.8, 0.5],
        ..Lighting::default()
    };
    let (center, radius) = fit(&graph(), &[prop], lighting).unwrap();
    let mut fitted = lighting;
    fitted.shadow_center = center;
    fitted.shadow_radius = radius;
    for size in [512, 1024, 2048] {
        for corner in 0..8 {
            let point = std::array::from_fn(|axis| {
                if corner & (1 << axis) == 0 {
                    bounds.min[axis]
                } else {
                    bounds.max[axis]
                }
            });
            let c = light_view_proj(&fitted, size).transform_point4(point);
            assert!(c[0].abs() < c[3] && c[1].abs() < c[3] && c[2] > 0.0 && c[2] < c[3]);
        }
    }
    assert_eq!(lighting.direction, fitted.direction);
    assert_eq!(lighting.intensity, fitted.intensity);
}
#[test]
fn floor_and_extreme_bounds_are_checked() {
    let light = Lighting::default();
    assert_eq!(fit_bounds([0.0; 3], [0.0; 3], light).unwrap().1, 0.5);
    assert!(fit_bounds([f64::INFINITY; 3], [f64::INFINITY; 3], light).is_err());
    assert!(fit_bounds([1000001.0; 3], [1000002.0; 3], light).is_err());
    assert!(fit_bounds([-100000.0; 3], [100000.0; 3], light).is_err());
    let (c, r) = fit_bounds([999999.0; 3], [1000000.0; 3], light).unwrap();
    assert!(c.iter().all(|v| v.is_finite()) && r >= 0.5);
    assert!(fit_bounds(
        [0.0; 3],
        [1.0; 3],
        Lighting {
            direction: [f32::NAN, 0.0, 0.0],
            ..light
        }
    )
    .is_err());
}
#[test]
fn invalid_props_and_count_limits_reject_before_fit() {
    let mut prop = scene_model([0.0; 3]);
    prop.instance.scale[0] = f32::NAN;
    assert!(fit(&graph(), &[prop], Lighting::default()).is_err());
    let props: Vec<_> = (0..129).map(|_| scene_model([0.0; 3])).collect();
    assert!(fit(&graph(), &props, Lighting::default())
        .unwrap_err()
        .contains("128"));
    let mut source = model().source().clone();
    source.primitives[0].vertices = vec![vertex([0.0; 3]); 100001];
    let mut large = scene_model([0.0; 3]);
    large.model = Arc::new(StaticModel::new(source).unwrap());
    assert!(fit(&graph(), &[large], Lighting::default())
        .unwrap_err()
        .contains("100000 vertices"));
}
#[test]
fn aggregate_budget_counts_shared_models_per_instance() {
    let mut source = model().source().clone();
    source.primitives[0].vertices = vec![vertex([0.0; 3]); 100_000];
    let shared = Arc::new(StaticModel::new(source).unwrap());
    let props: Vec<_> = (0..11)
        .map(|_| SceneModel {
            #[cfg(feature = "mesh-prop-shading")]
            shading: None,
            model: shared.clone(),
            instance: StaticInstance {
                translation: [0.0; 3],
                rotation: IDENTITY_ROT,
                scale: [1.0; 3],
            },
        })
        .collect();
    assert!(fit(&graph(), &props, Lighting::default())
        .unwrap_err()
        .contains("1000000"));
}

#[test]
fn exact_graph_display_and_empty_geometry_contract() {
    assert!(exact_f32(FP::from_raw(256 * 65536 + 1)).is_err());
    assert!(exact_f32(FP::from_raw(256 * 65536 - 1)).is_ok());
    let empty = StaticMeshGraph::bake(&[], &[], Default::default()).unwrap();
    assert!(fit(&empty, &[], Lighting::default()).is_err());
}
