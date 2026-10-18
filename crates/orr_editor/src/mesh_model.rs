use orr_fp::FP;
use orr_model::StaticModel;
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path},
};

const MAX_SOURCE_VERTICES: usize = 1_024;
const MAX_SOURCE_FACES: usize = 512;
const MAX_GRAPH_BYTES: usize = 8_192;
const MAX_MODEL_BYTES: usize = orr_model::MAX_FILE_BYTES;
const MAX_COORDINATE_RAW: i64 = 256 * FP::SCALE;

pub fn import_model(path: &Path, profile: AgentProfile) -> Result<StaticMeshGraph, String> {
    validate_path_length(path)?;
    let (parent, file_name) = validate_input_path(path)?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .ok_or("model path needs a .glb, .gltf, or .orrmodel extension")?;
    let model = if extension.eq_ignore_ascii_case("orrmodel") {
        let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
        if is_link_or_reparse(&metadata) || !metadata.is_file() {
            return Err("model file must be a regular non-link file".into());
        }
        if metadata.len() > MAX_MODEL_BYTES as u64 {
            return Err("cooked model exceeds the 64 MiB limit".into());
        }
        let mut file = File::open(path).map_err(|error| error.to_string())?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        Read::by_ref(&mut file)
            .take(MAX_MODEL_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() > MAX_MODEL_BYTES {
            return Err("cooked model exceeds the 64 MiB limit".into());
        }
        StaticModel::from_bytes(&bytes).map_err(|error| error.to_string())?
    } else if extension.eq_ignore_ascii_case("glb") || extension.eq_ignore_ascii_case("gltf") {
        let parent = parent.ok_or("model path must have a parent directory")?;
        let file_name = file_name.ok_or("model filename must be valid UTF-8")?;
        orr_model::import::import_path(parent, file_name).map_err(|error| error.to_string())?
    } else {
        return Err("model extension must be .glb, .gltf, or .orrmodel".into());
    };
    cook_model(&model, profile)
}

/// Bakes walkable triangles after quantizing model transforms and positions to Q16.
/// Negative quantized transform determinants reverse face indices to preserve authored winding.
pub fn cook_model(model: &StaticModel, profile: AgentProfile) -> Result<StaticMeshGraph, String> {
    let source = model.source();
    let mut source_vertex_count = 0usize;
    let mut source_face_count = 0usize;
    for primitive in &source.primitives {
        source_vertex_count = source_vertex_count
            .checked_add(primitive.vertices.len())
            .ok_or("source vertex count overflow")?;
        source_face_count = source_face_count
            .checked_add(primitive.indices.len() / 3)
            .ok_or("source face count overflow")?;
        if source_vertex_count > MAX_SOURCE_VERTICES {
            return Err("source model exceeds the 1,024-vertex limit".into());
        }
        if primitive.indices.len() % 3 != 0 || source_face_count > MAX_SOURCE_FACES {
            return Err(
                "source model exceeds the 512-triangle limit or has incomplete faces".into(),
            );
        }
    }
    if source_face_count == 0 {
        return Err("source model has no triangles".into());
    }

    let mut vertices = Vec::with_capacity(source_vertex_count);
    let mut faces = Vec::with_capacity(source_face_count);
    for primitive in &source.primitives {
        let transform = fixed_transform(primitive.transform)?;
        let determinant = linear_determinant(transform)?;
        if determinant == 0 {
            return Err(
                "model primitive transform is singular after fixed-point quantization".into(),
            );
        }
        let base = u32::try_from(vertices.len()).map_err(|_| "source vertex index overflow")?;
        for vertex in &primitive.vertices {
            let local = vertex
                .position
                .map(float_to_raw)
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?;
            let point = transform_point(transform, [local[0], local[1], local[2]])?;
            if point
                .iter()
                .any(|coordinate| coordinate.raw().unsigned_abs() > MAX_COORDINATE_RAW as u64)
            {
                return Err("transformed model vertex is outside +/-256".into());
            }
            vertices.push(point);
        }
        for triangle in primitive.indices.as_chunks::<3>().0 {
            let local_indices = [triangle[0], triangle[1], triangle[2]];
            if local_indices
                .iter()
                .any(|index| *index as usize >= primitive.vertices.len())
            {
                return Err("source model has an index outside its primitive".into());
            }
            let face = local_indices.map(|index| {
                base.checked_add(index)
                    .ok_or("source vertex index overflow")
            });
            let mut face = [face[0]?, face[1]?, face[2]?];
            if determinant < 0 {
                face.swap(1, 2);
            }
            let points = face.map(|index| vertices[index as usize]);
            if degenerate(points) {
                return Err("source model contains a degenerate triangle".into());
            }
            let winding = cross_xz(points[0], points[1], points[2]);
            if winding < 0 {
                faces.push(face);
            }
        }
    }
    let graph =
        StaticMeshGraph::bake(&vertices, &faces, profile).map_err(|error| error.to_string())?;
    if graph.triangles().is_empty() {
        return Err("source model has no upward walkable triangles within the slope limit".into());
    }
    if graph.to_bytes().len() > MAX_GRAPH_BYTES {
        return Err("canonical mesh exceeds the 8,192-byte limit".into());
    }
    Ok(graph)
}

pub fn save_graph(path: &Path, graph: &StaticMeshGraph) -> Result<(), String> {
    validate_path_length(path)?;
    if !path.is_absolute() {
        return Err("mesh output path must be absolute".into());
    }
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("orrmesh"))
    {
        return Err("mesh output path must use the .orrmesh extension".into());
    }
    check_existing_components(path)?;
    let parent = path
        .parent()
        .ok_or("mesh output path needs a parent directory")?;
    if !fs::metadata(parent)
        .map_err(|error| error.to_string())?
        .is_dir()
    {
        return Err("mesh output parent must be an existing directory".into());
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if is_link_or_reparse(&metadata) {
                return Err("mesh output cannot replace a link or reparse point".into());
            }
            return Err("mesh output already exists".into());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    let bytes = graph.to_bytes();
    if bytes.len() > MAX_GRAPH_BYTES {
        return Err("canonical mesh exceeds the 8,192-byte limit".into());
    }
    let canonical = StaticMeshGraph::from_bytes(&bytes).map_err(|error| error.to_string())?;
    if canonical.revision() != graph.revision() {
        return Err("canonical mesh revision changed during validation".into());
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn validate_path_length(path: &Path) -> Result<(), String> {
    if path.to_string_lossy().len() > 4_096 {
        return Err("model or mesh path exceeds the 4,096-byte limit".into());
    }
    Ok(())
}

fn validate_input_path(path: &Path) -> Result<(Option<&Path>, Option<&str>), String> {
    if !path.is_absolute() {
        return Err("model path must be absolute".into());
    }
    check_existing_components(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if is_link_or_reparse(&metadata) || !metadata.is_file() {
        return Err("model file must be a regular non-link file".into());
    }
    Ok((
        path.parent(),
        path.file_name().and_then(|name| name.to_str()),
    ))
}

pub fn check_existing_components(path: &Path) -> Result<(), String> {
    let mut current = Path::new("").to_path_buf();
    let components: Vec<_> = path.components().collect();
    for (index, component) in components.iter().enumerate() {
        match component {
            Component::Prefix(prefix) => {
                current.push(prefix.as_os_str());
                continue;
            }
            Component::RootDir => current.push(component.as_os_str()),
            Component::CurDir => continue,
            Component::ParentDir => {
                return Err("path cannot contain parent-directory segments".into());
            }
            Component::Normal(part) => current.push(part),
        }
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && index + 1 == components.len() =>
            {
                return Ok(());
            }
            Err(error) => return Err(error.to_string()),
        };
        if is_link_or_reparse(&metadata) {
            return Err("path cannot contain links or reparse points".into());
        }
    }
    Ok(())
}

fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    false
}

fn float_to_raw(value: f32) -> Result<i64, String> {
    let bits = value.to_bits();
    let negative = bits >> 31 != 0;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let fraction = bits & 0x7f_ffff;
    if exponent == 0xff {
        return Err("model contains a non-finite coordinate".into());
    }
    if exponent == 0 && fraction == 0 {
        return Ok(0);
    }
    let (significand, shift) = if exponent == 0 {
        (fraction as u128, -133)
    } else {
        (((1 << 23) | fraction) as u128, exponent - 134)
    };
    let magnitude = if shift >= 0 {
        1u128
            .checked_shl(shift as u32)
            .and_then(|factor| significand.checked_mul(factor))
            .ok_or("model coordinate overflows fixed-point range")?
    } else {
        let right_shift = (-shift) as u32;
        if right_shift >= 26 {
            0
        } else {
            let denominator = 1u128 << right_shift;
            let quotient = significand / denominator;
            let remainder = significand % denominator;
            quotient + u128::from(remainder * 2 >= denominator)
        }
    };
    let signed =
        i128::try_from(magnitude).map_err(|_| "model coordinate overflows fixed-point range")?;
    let signed = if negative { -signed } else { signed };
    i64::try_from(signed).map_err(|_| "model coordinate overflows fixed-point range".into())
}

fn fixed_transform(transform: [[f32; 4]; 4]) -> Result<[[i64; 4]; 4], String> {
    if transform.iter().flatten().any(|value| !value.is_finite())
        || transform[0][3] != 0.0
        || transform[1][3] != 0.0
        || transform[2][3] != 0.0
        || transform[3][3] != 1.0
    {
        return Err("model primitive transform must be finite and affine".into());
    }
    let mut fixed = [[0i64; 4]; 4];
    for (column, values) in transform.iter().enumerate() {
        for (row, value) in values.iter().enumerate() {
            fixed[column][row] = float_to_raw(*value)?;
        }
    }
    Ok(fixed)
}

fn transform_point(transform: [[i64; 4]; 4], point: [i64; 3]) -> Result<[FP; 3], String> {
    let mut output = [FP::ZERO; 3];
    for (row, coordinate) in output.iter_mut().enumerate() {
        let mut sum = i128::from(transform[3][row])
            .checked_mul(i128::from(FP::SCALE))
            .ok_or("transformed model vertex overflows")?;
        for (column, point_coordinate) in point.iter().enumerate() {
            let product = i128::from(transform[column][row])
                .checked_mul(i128::from(*point_coordinate))
                .ok_or("transformed model vertex overflows")?;
            sum = sum
                .checked_add(product)
                .ok_or("transformed model vertex overflows")?;
        }
        let rounded = round_q16(sum)?;
        *coordinate = FP::from_raw(rounded);
    }
    Ok(output)
}

fn linear_determinant(transform: [[i64; 4]; 4]) -> Result<i128, String> {
    let element_00 = i128::from(transform[0][0]);
    let element_10 = i128::from(transform[1][0]);
    let element_20 = i128::from(transform[2][0]);
    let element_01 = i128::from(transform[0][1]);
    let element_11 = i128::from(transform[1][1]);
    let element_21 = i128::from(transform[2][1]);
    let element_02 = i128::from(transform[0][2]);
    let element_12 = i128::from(transform[1][2]);
    let element_22 = i128::from(transform[2][2]);
    let minor_00 = element_11
        .checked_mul(element_22)
        .and_then(|value| value.checked_sub(element_21.checked_mul(element_12)?))
        .ok_or("model transform determinant overflows")?;
    let minor_01 = element_01
        .checked_mul(element_22)
        .and_then(|value| value.checked_sub(element_21.checked_mul(element_02)?))
        .ok_or("model transform determinant overflows")?;
    let minor_02 = element_01
        .checked_mul(element_12)
        .and_then(|value| value.checked_sub(element_11.checked_mul(element_02)?))
        .ok_or("model transform determinant overflows")?;
    element_00
        .checked_mul(minor_00)
        .and_then(|value| value.checked_sub(element_10.checked_mul(minor_01)?))
        .and_then(|value| value.checked_add(element_20.checked_mul(minor_02)?))
        .ok_or_else(|| "model transform determinant overflows".into())
}

fn round_q16(value: i128) -> Result<i64, String> {
    let quotient = value / i128::from(FP::SCALE);
    let remainder = value % i128::from(FP::SCALE);
    let adjustment = if remainder.unsigned_abs() * 2 >= FP::SCALE as u128 {
        if value < 0 {
            -1
        } else {
            1
        }
    } else {
        0
    };
    i64::try_from(quotient + adjustment).map_err(|_| "transformed model vertex overflows".into())
}

fn degenerate(points: [[FP; 3]; 3]) -> bool {
    let first = points[0].map(FP::raw);
    let second = points[1].map(FP::raw);
    let third = points[2].map(FP::raw);
    let first_edge = [
        i128::from(second[0]) - i128::from(first[0]),
        i128::from(second[1]) - i128::from(first[1]),
        i128::from(second[2]) - i128::from(first[2]),
    ];
    let second_edge = [
        i128::from(third[0]) - i128::from(first[0]),
        i128::from(third[1]) - i128::from(first[1]),
        i128::from(third[2]) - i128::from(first[2]),
    ];
    let cross = [
        first_edge[1] * second_edge[2] - first_edge[2] * second_edge[1],
        first_edge[2] * second_edge[0] - first_edge[0] * second_edge[2],
        first_edge[0] * second_edge[1] - first_edge[1] * second_edge[0],
    ];
    cross == [0; 3]
}

fn cross_xz(first: [FP; 3], second: [FP; 3], third: [FP; 3]) -> i128 {
    let first_x = i128::from(second[0].raw()) - i128::from(first[0].raw());
    let first_z = i128::from(second[2].raw()) - i128::from(first[2].raw());
    let second_x = i128::from(third[0].raw()) - i128::from(first[0].raw());
    let second_z = i128::from(third[2].raw()) - i128::from(first[2].raw());
    first_x * second_z - first_z * second_x
}

#[cfg(test)]
mod tests {
    use super::{fixed_transform, float_to_raw};

    #[test]
    fn converts_ieee_f32_to_q16_nearest_ties_away_from_zero() {
        assert_eq!(float_to_raw(f32::from_bits(0x3700_0000)).unwrap(), 1);
        assert_eq!(float_to_raw(f32::from_bits(0xb700_0000)).unwrap(), -1);
        assert_eq!(float_to_raw(f32::from_bits(0x3680_0000)).unwrap(), 0);
        assert!(float_to_raw(f32::INFINITY).is_err());
        assert!(float_to_raw(f32::NAN).is_err());
        assert!(float_to_raw(f32::MAX).is_err());
        assert!(float_to_raw(-f32::MAX).is_err());
    }

    #[test]
    fn rejects_non_affine_transform() {
        let mut transform = orr_model::IDENTITY;
        transform[1][3] = 1.0;
        assert!(fixed_transform(transform).is_err());

        let mut oversized = orr_model::IDENTITY;
        oversized[0][0] = f32::MAX;
        assert!(fixed_transform(oversized).is_err());
    }
}
