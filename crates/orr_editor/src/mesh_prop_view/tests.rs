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
        model: model(),
        instance: StaticInstance {
            translation,
            rotation: IDENTITY_ROT,
            scale: [1.0; 3],
        },
    }
}

#[test]
fn bounds_include_node_vertices_and_instance_transform() {
    let mut source = model().source().clone();
    source.primitives[0].transform[3][0] = 1.0;
    let model = SceneModel {
        model: Arc::new(StaticModel::new(source).unwrap()),
        instance: StaticInstance {
            translation: [3.0, 2.0, -4.0],
            rotation: IDENTITY_ROT,
            scale: [1.0; 3],
        },
    };
    let result = bounds(&model).unwrap();
    assert_eq!(result.min, [3.0, 1.0, -5.0]);
    assert_eq!(result.max, [5.0, 3.0, -3.0]);
}

#[test]
fn bounds_reject_invalid_or_excessive_instance_geometry() {
    let mut model = scene_model([0.0; 3]);
    model.instance.scale[0] = f32::NAN;
    assert!(bounds(&model).is_err());
    model.instance.scale = [1.0; 3];
    model.instance.translation[0] = 2.0e6;
    assert!(bounds(&model).is_err());
}

#[test]
fn ray_slab_returns_nearest_front_hit_and_rejects_parallel_behind_or_invalid_rays() {
    let bounds = Bounds {
        min: [-1.0; 3],
        max: [1.0; 3],
    };
    assert_eq!(
        ray_hit(bounds, [0.0, 0.0, 3.0], [0.0, 0.0, -1.0]),
        Some(2.0)
    );
    assert_eq!(ray_hit(bounds, [2.0, 0.0, 3.0], [0.0, 0.0, -1.0]), None);
    assert_eq!(ray_hit(bounds, [0.0, 0.0, 3.0], [0.0, 0.0, 1.0]), None);
    assert_eq!(ray_hit(bounds, [0.0; 3], [0.0; 3]), None);
    assert_eq!(
        ray_hit(bounds, [f32::NAN, 0.0, 3.0], [0.0, 0.0, -1.0]),
        None
    );
}

#[test]
fn pick_uses_nearest_hit_and_lowest_id_for_exact_ties() {
    let models = vec![
        (9, scene_model([0.0, 0.0, 0.0])),
        (7, scene_model([0.0, 0.0, 0.0])),
        (1, scene_model([0.0, 0.0, -5.0])),
    ];
    assert_eq!(
        pick(&models, [0.0, 0.0, 5.0], [0.0, 0.0, -1.0]).unwrap(),
        Some(7)
    );
}

#[test]
fn drag_keeps_grab_offset_and_height_and_applies_only_valid_xz_snap() {
    let camera = Camera3D::perspective([0.0, 10.0, 10.0], [0.0, 0.0, 0.0], 45.0);
    let viewport = (800, 600);
    let pixel = [400.0, 300.0];
    let position = [2.0, 1.0, 3.0];
    let drag = Drag::new(&camera, pixel, viewport, position).unwrap();
    let unchanged = drag.position(&camera, pixel, viewport, None).unwrap();
    assert!((unchanged[0] - position[0]).abs() < 1e-4);
    assert_eq!(unchanged[1], position[1]);
    assert!((unchanged[2] - position[2]).abs() < 1e-4);
    assert!(drag
        .position(&camera, [450.0, 300.0], viewport, Some(0.5))
        .is_some());
    assert!(drag.position(&camera, pixel, viewport, Some(0.0)).is_none());
    assert!(Drag::new(&camera, [f32::INFINITY, 0.0], viewport, position).is_none());
}
