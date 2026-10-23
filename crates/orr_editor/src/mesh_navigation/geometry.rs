use orr_fp::FP;
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};

const MAX_COORDINATE_RAW: i64 = 256 * FP::SCALE;
const HEADER_BYTES: usize = 8 + 4 * 8;
const MAX_FACES: usize = 32_768;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quad {
    pub origin: [FP; 3],
    pub width: FP,
    pub depth: FP,
    pub rise: FP,
}

pub fn append_quad(
    graph: Option<&StaticMeshGraph>,
    profile: AgentProfile,
    quad: Quad,
) -> Result<StaticMeshGraph, String> {
    if graph.is_some_and(|current| current.profile() != profile) {
        return Err("quad profile differs from the current mesh".into());
    }
    if quad.width.raw() <= 0 || quad.depth.raw() <= 0 {
        return Err("quad width and depth must be positive".into());
    }
    let origin = quad.origin;
    let far_x = [
        checked_coordinate(origin[0], quad.width)?,
        origin[1],
        origin[2],
    ];
    let far_z = [
        origin[0],
        checked_coordinate(origin[1], quad.rise)?,
        checked_coordinate(origin[2], quad.depth)?,
    ];
    let far_corner = [far_x[0], far_z[1], far_z[2]];
    let mut vertices = graph.map_or_else(Vec::new, |current| current.vertices().to_vec());
    let mut faces = graph.map(faces).transpose()?.unwrap_or_default();
    let start = u32::try_from(vertices.len()).map_err(|_| "mesh vertex limit exceeded")?;
    let end = start.checked_add(4).ok_or("mesh vertex limit exceeded")?;
    if end > 65_536 {
        return Err("mesh vertex limit exceeded".into());
    }
    for point in [origin, far_x, far_z, far_corner] {
        validate_coordinate(point)?;
        vertices.push(point);
    }
    faces.push([start, start + 2, start + 3]);
    faces.push([start, start + 3, start + 1]);
    bake(&vertices, &faces, profile)
}

pub fn move_vertex(
    graph: &StaticMeshGraph,
    index: u32,
    position: [FP; 3],
) -> Result<StaticMeshGraph, String> {
    let mut vertices = graph.vertices().to_vec();
    let vertex = vertices
        .get_mut(index as usize)
        .ok_or_else(|| "vertex index is outside the current mesh".to_owned())?;
    validate_coordinate(position)?;
    if *vertex == position {
        return Err("vertex move does not change the mesh".into());
    }
    *vertex = position;
    let faces = faces(graph)?;
    bake(&vertices, &faces, graph.profile())
}

pub fn remove_face(graph: &StaticMeshGraph, index: u32) -> Result<StaticMeshGraph, String> {
    let mut faces = faces(graph)?;
    if index as usize >= faces.len() {
        return Err("face index is outside the current mesh".into());
    }
    faces.remove(index as usize);
    bake(graph.vertices(), &faces, graph.profile())
}

pub(crate) fn faces(graph: &StaticMeshGraph) -> Result<Vec<[u32; 3]>, String> {
    let bytes = graph.to_bytes();
    if bytes.len() > 2_200_000 || bytes.get(..8) != Some(b"ORRMESH\x01".as_slice()) {
        return Err("current mesh serialization is not supported".into());
    }
    let decoded = StaticMeshGraph::from_bytes(&bytes).map_err(|error| error.to_string())?;
    if decoded != *graph || decoded.revision() != graph.revision() || decoded.to_bytes() != bytes {
        return Err("current mesh failed canonical serialization verification".into());
    }
    let vertex_count = read_u32(&bytes, HEADER_BYTES)? as usize;
    let face_count_offset = HEADER_BYTES
        .checked_add(4)
        .and_then(|offset| {
            vertex_count
                .checked_mul(24)
                .and_then(|size| offset.checked_add(size))
        })
        .ok_or("mesh serialization length overflow")?;
    let face_count = read_u32(&bytes, face_count_offset)? as usize;
    if face_count > MAX_FACES {
        return Err("mesh face limit exceeded".into());
    }
    let face_data = face_count_offset
        .checked_add(4)
        .ok_or("mesh serialization length overflow")?;
    let expected_length = face_data
        .checked_add(
            face_count
                .checked_mul(12)
                .ok_or("mesh serialization length overflow")?,
        )
        .ok_or("mesh serialization length overflow")?;
    if expected_length != bytes.len() {
        return Err("current mesh serialization has an invalid length".into());
    }
    (0..face_count)
        .map(|face_index| {
            let offset = face_data + face_index * 12;
            Ok([
                read_u32(&bytes, offset)?,
                read_u32(&bytes, offset + 4)?,
                read_u32(&bytes, offset + 8)?,
            ])
        })
        .collect()
}

fn bake(
    vertices: &[[FP; 3]],
    faces: &[[u32; 3]],
    profile: AgentProfile,
) -> Result<StaticMeshGraph, String> {
    if vertices
        .iter()
        .copied()
        .any(|point| validate_coordinate(point).is_err())
    {
        return Err("mesh coordinates must remain within ±256 units".into());
    }
    StaticMeshGraph::bake(vertices, faces, profile).map_err(|error| error.to_string())
}

fn checked_coordinate(origin: FP, delta: FP) -> Result<FP, String> {
    origin
        .raw()
        .checked_add(delta.raw())
        .map(FP::from_raw)
        .ok_or_else(|| "quad coordinate overflow".into())
}

fn validate_coordinate(point: [FP; 3]) -> Result<(), String> {
    if point
        .iter()
        .any(|coordinate| i128::from(coordinate.raw()).abs() > i128::from(MAX_COORDINATE_RAW))
    {
        return Err("mesh coordinates must remain within ±256 units".into());
    }
    Ok(())
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let end = offset
        .checked_add(4)
        .ok_or("mesh serialization offset overflow")?;
    let raw: [u8; 4] = bytes
        .get(offset..end)
        .ok_or_else(|| "truncated mesh serialization".to_owned())?
        .try_into()
        .map_err(|_| "truncated mesh serialization")?;
    Ok(u32::from_le_bytes(raw))
}

#[path = "geometry/tests.rs"]
#[cfg(test)]
mod tests;
