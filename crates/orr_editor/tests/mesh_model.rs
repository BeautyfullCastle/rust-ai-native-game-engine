#![cfg(feature = "navigation")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

#[path = "../src/mesh_model.rs"]
mod mesh_model;

use mesh_model::{cook_model, import_model, save_graph};
use orr_fp::{fp, FP};
use orr_model::{
    Dependency, Image, Material, ModelSource, Primitive, StaticModel, Vertex, Wrap, IDENTITY,
};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use std::path::Path;

fn profile(max_slope: FP) -> AgentProfile {
    AgentProfile {
        max_slope,
        radius: FP::ZERO,
        headroom: FP::ZERO,
        max_step: FP::ZERO,
    }
}

fn model(primitives: Vec<Primitive>) -> StaticModel {
    StaticModel::new(ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: "test/model.orrmodel".into(),
        dependencies: vec![Dependency {
            uri: "$source".into(),
            sha256: "a".repeat(64),
        }],
        primitives: primitives
            .into_iter()
            .enumerate()
            .map(|(index, mut primitive)| {
                primitive.id = format!("test/model.orrmodel#node=0/mesh=0/primitive={index}");
                primitive
            })
            .collect(),
        materials: vec![Material {
            base_color: [1.0; 4],
            image: 0,
            linear_filter: false,
            wrap_s: Wrap::Clamp,
            wrap_t: Wrap::Clamp,
        }],
        images: vec![Image {
            width: 1,
            height: 1,
            rgba8: vec![255; 4],
        }],
    })
    .unwrap()
}

fn primitive(positions: &[[f32; 3]], indices: &[u32], transform: [[f32; 4]; 4]) -> Primitive {
    Primitive {
        id: "test/model.orrmodel#node=0/mesh=0/primitive=0".into(),
        vertices: positions
            .iter()
            .copied()
            .map(|position| Vertex {
                position,
                normal: [0.0, 1.0, 0.0],
                uv: [0.0; 2],
            })
            .collect(),
        indices: indices.to_vec(),
        material: 0,
        transform,
    }
}

fn upward_triangle() -> Primitive {
    primitive(
        &[[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
        &[0, 1, 2],
        IDENTITY,
    )
}

#[test]
fn imports_real_glb_and_cooks_walkable_geometry() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/imported_scene_demo/foreground.glb")
        .canonicalize()
        .unwrap();
    let graph = import_model(&path, profile(fp!(4))).unwrap();
    assert!(!graph.vertices().is_empty());
    assert!(!graph.triangles().is_empty());
    assert!(graph.to_bytes().len() <= 8_192);
}

#[test]
fn imports_bounded_cooked_model_and_matches_direct_cook() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("model.orrmodel");
    let model = model(vec![upward_triangle()]);
    std::fs::write(&path, model.to_bytes().unwrap()).unwrap();
    let direct = cook_model(&model, profile(fp!(1))).unwrap();
    let loaded = import_model(&path.canonicalize().unwrap(), profile(fp!(1))).unwrap();
    assert_eq!(loaded.to_bytes(), direct.to_bytes());
}

#[test]
fn applies_affine_transform_and_preserves_authored_winding_under_reflection() {
    let mut transform = IDENTITY;
    transform[0][0] = -1.0;
    transform[3][0] = 2.0;
    transform[3][1] = 1.5;
    let reflected_upward = primitive(
        &[[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
        &[0, 1, 2],
        transform,
    );
    let graph = cook_model(&model(vec![reflected_upward]), profile(fp!(1))).unwrap();
    assert_eq!(graph.vertices().len(), 3);
    assert!(graph.vertices().contains(&[fp!(2), fp!(1.5), fp!(0)]));
    assert_eq!(graph.triangles().len(), 1);

    let reflected_downward = primitive(
        &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        &[0, 1, 2],
        transform,
    );
    assert!(cook_model(&model(vec![reflected_downward]), profile(fp!(1))).is_err());
}

#[test]
fn keeps_upward_faces_and_filters_downward_vertical_and_slope_limited_faces() {
    let mut steep = upward_triangle();
    steep.vertices[0].position[0] = 3.0;
    steep.vertices[1].position[0] = 3.0;
    steep.vertices[2].position[0] = 4.0;
    steep.vertices[1].position = [0.0, 2.0, 1.0];
    steep.vertices[1].position[0] = 3.0;
    let upward = primitive(
        &[[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
        &[0, 1, 2],
        IDENTITY,
    );
    let downward = primitive(
        &[[6.0, 0.0, 0.0], [7.0, 0.0, 0.0], [6.0, 0.0, 1.0]],
        &[0, 1, 2],
        IDENTITY,
    );
    let vertical = primitive(
        &[[9.0, 0.0, 0.0], [9.0, 1.0, 0.0], [9.0, 0.0, 1.0]],
        &[0, 1, 2],
        IDENTITY,
    );
    let graph = cook_model(
        &model(vec![upward, steep, downward, vertical]),
        profile(FP::ZERO),
    )
    .unwrap();
    assert_eq!(graph.triangles().len(), 1);
}

#[test]
fn rejects_degenerate_triangles_and_non_affine_transforms() {
    let degenerate = primitive(
        &[[0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
        &[0, 1, 2],
        IDENTITY,
    );
    assert!(cook_model(&model(vec![degenerate]), profile(fp!(1))).is_err());
}

#[test]
fn rejects_transforms_that_become_singular_after_quantization() {
    let mut transform = IDENTITY;
    transform[1][1] = 0.000001;
    let source = primitive(
        &[[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
        &[0, 1, 2],
        transform,
    );
    assert!(cook_model(&model(vec![source]), profile(fp!(1))).is_err());
}

#[test]
fn enforces_source_vertex_and_face_caps_before_filtering() {
    let many_vertices: Vec<[f32; 3]> = (0..1_025).map(|index| [index as f32, 0.0, 0.0]).collect();
    let vertices_over_limit = primitive(&many_vertices, &[0, 1, 2], IDENTITY);
    assert!(cook_model(&model(vec![vertices_over_limit]), profile(fp!(1))).is_err());

    let positions = [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]];
    let indices = [0u32, 1, 2].repeat(513);
    let faces_over_limit = primitive(&positions, &indices, IDENTITY);
    assert!(cook_model(&model(vec![faces_over_limit]), profile(fp!(1))).is_err());
}

#[test]
fn rejects_transformed_coordinates_outside_supported_domain() {
    let outside = primitive(
        &[[257.0, 0.0, 0.0], [257.0, 0.0, 1.0], [258.0, 0.0, 0.0]],
        &[0, 1, 2],
        IDENTITY,
    );
    assert!(cook_model(&model(vec![outside]), profile(fp!(1))).is_err());
}

#[test]
fn saves_canonical_graph_without_overwriting_existing_files() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("walkable.orrmesh");
    let graph = cook_model(&model(vec![upward_triangle()]), profile(fp!(1))).unwrap();
    save_graph(&path, &graph).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(
        StaticMeshGraph::from_bytes(&bytes).unwrap().to_bytes(),
        bytes
    );
    assert_eq!(
        save_graph(&path, &graph).unwrap_err(),
        "mesh output already exists"
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[cfg(unix)]
#[test]
fn refuses_symlinked_output_parent() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let real_parent = root.path().join("real");
    let linked_parent = root.path().join("linked");
    std::fs::create_dir(&real_parent).unwrap();
    symlink(&real_parent, &linked_parent).unwrap();
    let output = linked_parent.join("walkable.orrmesh");
    let graph = cook_model(&model(vec![upward_triangle()]), profile(fp!(1))).unwrap();
    assert!(save_graph(&output, &graph).is_err());
    assert!(!real_parent.join("walkable.orrmesh").exists());
}
