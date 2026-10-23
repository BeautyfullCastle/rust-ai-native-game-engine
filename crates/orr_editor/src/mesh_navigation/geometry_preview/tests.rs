use super::build;
use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};

fn graph_with_walkable_and_rejected_faces() -> StaticMeshGraph {
    StaticMeshGraph::bake(
        &[
            [FP::ZERO, FP::ZERO, FP::ZERO],
            [fp!(2), FP::ZERO, FP::ZERO],
            [FP::ZERO, FP::ZERO, fp!(2)],
            [fp!(2), FP::ZERO, fp!(2)],
            [fp!(4), FP::ZERO, FP::ZERO],
            [fp!(6), FP::ZERO, FP::ZERO],
            [fp!(4), fp!(2), fp!(2)],
            [fp!(6), fp!(2), fp!(2)],
        ],
        &[[0, 2, 3], [0, 3, 1], [4, 6, 7], [4, 7, 5]],
        AgentProfile {
            max_slope: FP::ZERO,
            radius: FP::ZERO,
            headroom: FP::ZERO,
            max_step: FP::ZERO,
        },
    )
    .unwrap()
}

#[test]
fn preview_preserves_all_faces_and_separates_walkable_from_slope_rejected() {
    let graph = graph_with_walkable_and_rejected_faces();
    let original_bytes = graph.to_bytes();
    let original_revision = graph.revision();

    let model = build(&graph).unwrap();
    let source = model.source();
    assert_eq!(source.primitives.len(), 2);
    assert_eq!(source.materials.len(), 2);
    let walkable = source
        .primitives
        .iter()
        .find(|primitive| primitive.material == 0)
        .unwrap();
    let rejected = source
        .primitives
        .iter()
        .find(|primitive| primitive.material == 1)
        .unwrap();
    assert_eq!(walkable.indices.len() / 3, 2);
    assert_eq!(rejected.indices.len() / 3, 2);
    assert_eq!(
        source
            .primitives
            .iter()
            .map(|primitive| primitive.indices.len() / 3)
            .sum::<usize>(),
        4
    );
    assert_eq!(source.dependencies[0].sha256.len(), 64);
    assert_eq!(
        source.dependencies[0].sha256,
        super::hex_revision(original_revision)
    );
    assert_eq!(
        source.asset_id,
        "mesh/navigation/staged-geometry-preview.orrmodel"
    );
    assert_ne!(
        source.materials[0].base_color,
        source.materials[1].base_color
    );

    let source_faces = super::geometry::faces(&graph).unwrap();
    let mut expected_walkable = Vec::new();
    let mut expected_rejected = Vec::new();
    for (face_index, face) in source_faces.iter().enumerate() {
        let points = face.map(|vertex_index| {
            graph.vertices()[vertex_index as usize].map(|coordinate| coordinate.to_f32())
        });
        let signature = face_signature(points);
        if graph
            .triangles()
            .iter()
            .any(|triangle| triangle.key == face_index as u32)
        {
            expected_walkable.push(signature);
        } else {
            expected_rejected.push(signature);
        }
    }
    expected_walkable.sort();
    expected_rejected.sort();
    assert_eq!(actual_face_signatures(source, 0), expected_walkable);
    assert_eq!(actual_face_signatures(source, 1), expected_rejected);

    for vertex in source
        .primitives
        .iter()
        .flat_map(|primitive| &primitive.vertices)
    {
        let normal_length = vertex
            .normal
            .iter()
            .map(|component| component * component)
            .sum::<f32>();
        assert!((0.99..=1.01).contains(&normal_length));
        assert!(vertex.normal[1] > 0.0);
    }
    let minimum_x = source
        .primitives
        .iter()
        .flat_map(|primitive| &primitive.vertices)
        .map(|vertex| vertex.position[0])
        .fold(f32::INFINITY, f32::min);
    let maximum_x = source
        .primitives
        .iter()
        .flat_map(|primitive| &primitive.vertices)
        .map(|vertex| vertex.position[0])
        .fold(f32::NEG_INFINITY, f32::max);
    assert_eq!(minimum_x, 0.0);
    assert_eq!(maximum_x, 6.0);
    assert_eq!(graph.revision(), original_revision);
    assert_eq!(graph.to_bytes(), original_bytes);
}

fn actual_face_signatures(source: &orr_model::ModelSource, material: u32) -> Vec<[[i64; 3]; 3]> {
    let mut signatures = Vec::new();
    for primitive in source
        .primitives
        .iter()
        .filter(|primitive| primitive.material == material)
    {
        for indices in primitive.indices.as_chunks::<3>().0 {
            let points = [indices[0], indices[1], indices[2]]
                .map(|index| primitive.vertices[index as usize].position);
            signatures.push(face_signature(points));
        }
    }
    signatures.sort();
    signatures
}

fn face_signature(points: [[f32; 3]; 3]) -> [[i64; 3]; 3] {
    let mut signature =
        points.map(|point| point.map(|coordinate| (coordinate * FP::SCALE as f32) as i64));
    signature.sort();
    signature
}

#[test]
fn preview_rejects_a_graph_without_source_faces() {
    let graph = StaticMeshGraph::bake(&[], &[], AgentProfile::default()).unwrap();
    let error = build(&graph).unwrap_err();
    assert!(error.contains("source faces"));
}
