use super::geometry;
use orr_fp::FP;
use orr_model::{
    Dependency, Image, Material, ModelSource, Primitive, StaticModel, Vertex, Wrap, IDENTITY,
};
use orr_navigation::{mesh::StaticMeshGraph, MAX_TRIANGLES};
use std::{collections::BTreeSet, sync::Arc};

const MAX_GRAPH_BYTES: usize = 8_192;
const MAX_SOURCE_VERTICES: usize = 1_024;
const MAX_SOURCE_FACES: usize = 512;
const MAX_DISPLAY_RAW: i128 = 256 * FP::SCALE as i128;
const PREVIEW_ASSET_ID: &str = "mesh/navigation/staged-geometry-preview.orrmodel";

pub(super) fn build(graph: &StaticMeshGraph) -> Result<Arc<StaticModel>, String> {
    let graph_bytes = graph.to_bytes();
    if graph_bytes.len() > MAX_GRAPH_BYTES {
        return Err("staged mesh exceeds the 8,192-byte preview limit".into());
    }
    let decoded = StaticMeshGraph::from_bytes(&graph_bytes).map_err(|error| error.to_string())?;
    if decoded != *graph || decoded.to_bytes() != graph_bytes {
        return Err("staged mesh failed canonical serialization verification".into());
    }
    if graph.vertices().len() > MAX_SOURCE_VERTICES
        || graph.triangles().len() > MAX_TRIANGLES as usize
    {
        return Err("staged mesh exceeds the preview geometry admission limit".into());
    }
    let faces = geometry::faces(graph)?;
    if faces.is_empty() || faces.len() > MAX_SOURCE_FACES {
        return Err("staged mesh must contain 1..=512 source faces for preview".into());
    }
    let walkable_keys: BTreeSet<_> = graph
        .triangles()
        .iter()
        .map(|triangle| triangle.key)
        .collect();
    if walkable_keys.len() != graph.triangles().len()
        || graph.triangles().iter().any(|triangle| {
            faces
                .get(triangle.key as usize)
                .is_none_or(|face| *face != triangle.vertices)
        })
    {
        return Err("staged mesh walkable-face keys are inconsistent".into());
    }

    let mut walkable = preview_primitive(0);
    let mut rejected = preview_primitive(1);
    for (face_index, face) in faces.iter().enumerate() {
        let target = if walkable_keys.contains(&(face_index as u32)) {
            &mut walkable
        } else {
            &mut rejected
        };
        let points = [
            graph
                .vertices()
                .get(face[0] as usize)
                .copied()
                .ok_or_else(|| "staged mesh face references a missing vertex".to_owned())?,
            graph
                .vertices()
                .get(face[1] as usize)
                .copied()
                .ok_or_else(|| "staged mesh face references a missing vertex".to_owned())?,
            graph
                .vertices()
                .get(face[2] as usize)
                .copied()
                .ok_or_else(|| "staged mesh face references a missing vertex".to_owned())?,
        ];
        let positions = [
            point_to_f32(points[0])?,
            point_to_f32(points[1])?,
            point_to_f32(points[2])?,
        ];
        let normal = face_normal(positions)?;
        let base = u32::try_from(target.vertices.len())
            .map_err(|_| "staged mesh preview vertex index overflow".to_owned())?;
        for position in positions {
            target.vertices.push(Vertex {
                position,
                normal,
                uv: [0.0, 0.0],
            });
        }
        target.indices.extend([base, base + 1, base + 2]);
    }

    let mut primitives = Vec::with_capacity(2);
    if !walkable.indices.is_empty() {
        primitives.push(walkable);
    }
    if !rejected.indices.is_empty() {
        primitives.push(rejected);
    }
    if primitives.is_empty()
        || primitives.iter().any(|primitive| {
            primitive.vertices.len() > orr_model::MAX_VERTICES
                || primitive.indices.len() > orr_model::MAX_INDICES
        })
    {
        return Err("staged mesh preview exceeds renderer geometry limits".into());
    }

    let source = ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: PREVIEW_ASSET_ID.into(),
        dependencies: vec![Dependency {
            uri: "$source".into(),
            sha256: hex_revision(graph.revision()),
        }],
        primitives,
        materials: vec![
            preview_material([0.22, 0.65, 0.92, 1.0]),
            preview_material([0.95, 0.42, 0.18, 1.0]),
        ],
        images: vec![Image {
            width: 1,
            height: 1,
            rgba8: vec![255; 4],
        }],
    };
    let model = StaticModel::new(source).map_err(|error| error.to_string())?;
    let model_bytes = model.to_bytes().map_err(|error| error.to_string())?;
    if model_bytes.len() > MAX_GRAPH_BYTES * 64 {
        return Err("staged mesh preview exceeds the cooked model byte limit".into());
    }
    Ok(Arc::new(model))
}

fn preview_primitive(material: u32) -> Primitive {
    let slot = if material == 0 {
        "walkable"
    } else {
        "slope-rejected"
    };
    Primitive {
        id: format!("{PREVIEW_ASSET_ID}#node={slot}"),
        vertices: Vec::new(),
        indices: Vec::new(),
        material,
        transform: IDENTITY,
    }
}

fn preview_material(base_color: [f32; 4]) -> Material {
    Material {
        base_color,
        image: 0,
        linear_filter: false,
        wrap_s: Wrap::Clamp,
        wrap_t: Wrap::Clamp,
    }
}

fn point_to_f32(point: [FP; 3]) -> Result<[f32; 3], String> {
    let mut converted = [0.0; 3];
    for axis in 0..3 {
        if i128::from(point[axis].raw()).abs() > MAX_DISPLAY_RAW {
            return Err("staged mesh preview coordinates exceed ±256".into());
        }
        let value = point[axis].to_f32();
        if !value.is_finite() || f64::from(value) * FP::SCALE as f64 != point[axis].raw() as f64 {
            return Err("staged mesh preview requires exact finite f32 coordinates".into());
        }
        converted[axis] = value;
    }
    Ok(converted)
}

fn face_normal(points: [[f32; 3]; 3]) -> Result<[f32; 3], String> {
    let first_edge = [
        f64::from(points[1][0]) - f64::from(points[0][0]),
        f64::from(points[1][1]) - f64::from(points[0][1]),
        f64::from(points[1][2]) - f64::from(points[0][2]),
    ];
    let second_edge = [
        f64::from(points[2][0]) - f64::from(points[0][0]),
        f64::from(points[2][1]) - f64::from(points[0][1]),
        f64::from(points[2][2]) - f64::from(points[0][2]),
    ];
    let cross = [
        first_edge[1] * second_edge[2] - first_edge[2] * second_edge[1],
        first_edge[2] * second_edge[0] - first_edge[0] * second_edge[2],
        first_edge[0] * second_edge[1] - first_edge[1] * second_edge[0],
    ];
    let length = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
    if !length.is_finite() || length <= f64::EPSILON {
        return Err("staged mesh preview contains a degenerate face normal".into());
    }
    let normal = cross.map(|component| (component / length) as f32);
    if !normal.iter().all(|component| component.is_finite()) {
        return Err("staged mesh preview contains a nonfinite face normal".into());
    }
    Ok(normal)
}

fn hex_revision(revision: [u8; 32]) -> String {
    let mut encoded = String::with_capacity(64);
    for byte in revision {
        use std::fmt::Write;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

#[path = "geometry_preview/tests.rs"]
#[cfg(test)]
mod tests;
