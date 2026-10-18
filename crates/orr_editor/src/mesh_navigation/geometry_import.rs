use crate::{mesh_art::Snapshot, viewport3d::SceneModel};
use orr_fp::FP;
use orr_model::Primitive;
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use orr_render::{math3::quat_rotate, StaticInstance};
use std::collections::BTreeMap;

const MAX_WORLD_COORDINATE: f32 = 256.0;
const MAX_SOURCE_VERTICES: usize = 4096;
const MAX_SOURCE_TRIANGLES: usize = 4096;
const MAX_GRAPH_BYTES: usize = 8192;

pub(crate) fn candidate(
    snapshot: &Snapshot,
    license_acknowledged: bool,
    profile: AgentProfile,
) -> Result<StaticMeshGraph, String> {
    if !license_acknowledged {
        return Err(
            "license acknowledgement is required before importing walkable geometry".into(),
        );
    }
    let placement = snapshot.placement([FP::ZERO; 3])?;
    candidate_model(&placement, license_acknowledged, profile)
}

fn candidate_model(
    placement: &SceneModel,
    license_acknowledged: bool,
    profile: AgentProfile,
) -> Result<StaticMeshGraph, String> {
    if !license_acknowledged {
        return Err(
            "license acknowledgement is required before importing walkable geometry".into(),
        );
    }
    let model = &placement.model;
    let (source_vertices, source_triangles) = model.source().primitives.iter().try_fold(
        (0usize, 0usize),
        |(vertex_total, triangle_total), primitive| {
            if primitive.indices.is_empty() || !primitive.indices.len().is_multiple_of(3) {
                return Err("walkable model primitives must contain indexed triangles".to_owned());
            }
            Ok((
                vertex_total
                    .checked_add(primitive.vertices.len())
                    .ok_or_else(|| "walkable model vertex count overflow".to_owned())?,
                triangle_total
                    .checked_add(primitive.indices.len() / 3)
                    .ok_or_else(|| "walkable model triangle count overflow".to_owned())?,
            ))
        },
    )?;
    if source_vertices == 0 || source_vertices > MAX_SOURCE_VERTICES {
        return Err("walkable model exceeds the 4096 source vertex limit".into());
    }
    if source_triangles == 0 || source_triangles > MAX_SOURCE_TRIANGLES {
        return Err("walkable model exceeds the 4096 source triangle limit".into());
    }

    let mut vertices = Vec::with_capacity(source_vertices.min(256));
    let mut faces = Vec::with_capacity(source_triangles.min(512));
    let mut vertex_lookup = BTreeMap::<[i64; 3], u32>::new();
    for primitive in &model.source().primitives {
        append_primitive(
            primitive,
            placement.instance,
            &mut vertices,
            &mut faces,
            &mut vertex_lookup,
        )?;
    }
    if faces.is_empty() {
        return Err("walkable model contains no usable triangles".into());
    }
    let graph = StaticMeshGraph::bake(&vertices, &faces, profile)
        .map_err(|error| format!("walkable model admission failed: {error}"))?;
    if graph.triangles().len() != faces.len() {
        return Err("walkable model contains triangles outside the agent slope limit".into());
    }
    if graph.to_bytes().len() > MAX_GRAPH_BYTES {
        return Err("walkable model exceeds the 8 KiB navigation graph limit".into());
    }
    Ok(graph)
}

fn append_primitive(
    primitive: &Primitive,
    instance: StaticInstance,
    vertices: &mut Vec<[FP; 3]>,
    faces: &mut Vec<[u32; 3]>,
    vertex_lookup: &mut BTreeMap<[i64; 3], u32>,
) -> Result<(), String> {
    let mut transformed = Vec::with_capacity(primitive.vertices.len());
    for vertex in &primitive.vertices {
        let local = vertex.position;
        let matrix = primitive.transform;
        let node_position: [f32; 3] = std::array::from_fn(|axis| {
            matrix[0][axis] * local[0]
                + matrix[1][axis] * local[1]
                + matrix[2][axis] * local[2]
                + matrix[3][axis]
        });
        if !node_position
            .iter()
            .all(|coordinate| coordinate.is_finite())
        {
            return Err("walkable model has a nonfinite transformed vertex".into());
        }
        let scaled = std::array::from_fn(|axis| node_position[axis] * instance.scale[axis]);
        let rotated = quat_rotate(instance.rotation, scaled);
        let world = std::array::from_fn(|axis| rotated[axis] + instance.translation[axis]);
        if !world
            .iter()
            .all(|coordinate| coordinate.is_finite() && coordinate.abs() <= MAX_WORLD_COORDINATE)
        {
            return Err("walkable model coordinates must remain within ±256 units".into());
        }
        transformed.push(world.map(FP::from_f32));
    }

    let mut local_to_graph = Vec::with_capacity(transformed.len());
    for point in transformed {
        let key = point.map(FP::raw);
        let index = if let Some(index) = vertex_lookup.get(&key) {
            *index
        } else {
            let index = u32::try_from(vertices.len())
                .map_err(|_| "walkable model exceeds the graph vertex limit")?;
            vertices.push(point);
            vertex_lookup.insert(key, index);
            index
        };
        local_to_graph.push(index);
    }

    for source_face in primitive.indices.chunks_exact(3) {
        let mut face = source_face
            .iter()
            .map(|index| {
                local_to_graph
                    .get(*index as usize)
                    .copied()
                    .ok_or_else(|| "walkable model has an invalid triangle index".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| "walkable model has an invalid triangle")?;
        let area = projected_area(vertices, face);
        if area == 0 {
            return Err("walkable model contains a degenerate projected triangle".into());
        }
        if area > 0 {
            face.swap(1, 2);
        }
        faces.push(face);
    }
    Ok(())
}

fn projected_area(vertices: &[[FP; 3]], face: [u32; 3]) -> i128 {
    let first = vertices[face[0] as usize];
    let second = vertices[face[1] as usize];
    let third = vertices[face[2] as usize];
    let x1 = i128::from(second[0].raw()) - i128::from(first[0].raw());
    let z1 = i128::from(second[2].raw()) - i128::from(first[2].raw());
    let x2 = i128::from(third[0].raw()) - i128::from(first[0].raw());
    let z2 = i128::from(third[2].raw()) - i128::from(first[2].raw());
    x1 * z2 - z1 * x2
}

#[path = "geometry_import/tests.rs"]
#[cfg(test)]
mod tests;
