//! The drawn shape of an [`orr_gfx::Terrain`]: a grid mesh from a grayscale
//! heightmap, with smooth normals and texture coordinates from 0 to 1 across
//! the terrain. It is an ordinary static model, so the terrain casts and
//! receives shadows, takes scene lights and is drawn by the surface shader of
//! a `Material` on its entity like any other model.

use crate::TerrainRef;
use orr_model::{Dependency, Image, Material, ModelSource, Primitive, StaticModel, Vertex, Wrap, IDENTITY};

/// Most grid points per side: a larger grid would pass the model index limit.
pub const MAX_TERRAIN_GRID: u32 = 352;

/// Heights from 0 (black) to 1 (white) of a grayscale PNG, row by row from
/// the top (the far, -z edge). 8 and 16 bit images work; of a color image the
/// first channel is used.
pub fn read_heightmap(bytes: &[u8]) -> Result<(u32, u32, Vec<f32>), String> {
    let mut decoder = png::Decoder::new(bytes);
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info().map_err(|e| format!("not a PNG image: {e}"))?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).map_err(|e| format!("the PNG image cannot be read: {e}"))?;
    let channels = info.color_type.samples();
    let (width, height) = (info.width, info.height);
    if width < 2 || height < 2 || width > 8192 || height > 8192 {
        return Err(format!("a heightmap must be 2 to 8192 pixels per side, not {width} x {height}"));
    }
    if info.color_type == png::ColorType::Indexed {
        return Err("a heightmap must be a grayscale image, not a palette image".into());
    }
    let heights = match info.bit_depth {
        png::BitDepth::Eight => buffer[..info.buffer_size()].chunks_exact(channels).map(|p| f32::from(p[0]) / 255.0).collect(),
        png::BitDepth::Sixteen => buffer[..info.buffer_size()].chunks_exact(channels * 2).map(|p| f32::from(u16::from_be_bytes([p[0], p[1]])) / 65535.0).collect(),
        other => return Err(format!("a heightmap must have 8 or 16 bits per channel, not {other:?}")),
    };
    Ok((width, height, heights))
}

/// 64 hex digits that change with the heights: the identity of the mesh source.
fn fingerprint(heights: &[f32]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for h in heights {
        for byte in h.to_bits().to_le_bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{hash:016x}").repeat(4)
}

/// Height (0..1) at `(u, v)` in the image, both 0..1, sampled bilinearly.
fn sample(width: u32, height: u32, heights: &[f32], u: f32, v: f32) -> f32 {
    let x = u * (width - 1) as f32;
    let y = v * (height - 1) as f32;
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
    let (tx, ty) = (x - x0 as f32, y - y0 as f32);
    let at = |x: u32, y: u32| heights[(y * width + x) as usize];
    let top = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
    let bottom = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
    top * (1.0 - ty) + bottom * ty
}

/// The grid the terrain is drawn on: points per side and the 0..1 heights at
/// them, row by row from the -z edge. Whatever stands on the terrain should
/// use these heights, so it meets the drawn ground.
pub fn terrain_grid(terrain: &TerrainRef, width: u32, height: u32, heights: &[f32]) -> (u32, Vec<f32>) {
    let grid = terrain.resolution.clamp(8, MAX_TERRAIN_GRID).min(width.max(height)).max(2);
    let mut out = Vec::with_capacity((grid * grid) as usize);
    for j in 0..grid {
        for i in 0..grid {
            out.push(sample(width, height, heights, i as f32 / (grid - 1) as f32, j as f32 / (grid - 1) as f32));
        }
    }
    (grid, out)
}

/// The mesh of `terrain` from a heightmap of `width` x `height` pixels. The
/// mesh is centered on the terrain's own origin; the entity's position places it.
pub fn terrain_model(terrain: &TerrainRef, width: u32, height: u32, heights: &[f32]) -> Result<StaticModel, String> {
    let (grid, samples) = terrain_grid(terrain, width, height, heights);
    let [size_x, size_z] = terrain.size;
    let step = [size_x / (grid - 1) as f32, size_z / (grid - 1) as f32];
    let point = |i: u32, j: u32| -> [f32; 3] {
        let (u, v) = (i as f32 / (grid - 1) as f32, j as f32 / (grid - 1) as f32);
        [(u - 0.5) * size_x, samples[(j * grid + i) as usize] * terrain.height, (v - 0.5) * size_z]
    };
    let mut vertices = Vec::with_capacity((grid * grid) as usize);
    for j in 0..grid {
        for i in 0..grid {
            // Central differences of the height give a smooth normal.
            let left = point(i.saturating_sub(1), j)[1];
            let right = point((i + 1).min(grid - 1), j)[1];
            let near = point(i, j.saturating_sub(1))[1];
            let far = point(i, (j + 1).min(grid - 1))[1];
            let dx = (right - left) / (step[0] * (i.min(1) + (grid - 1 - i).min(1)) as f32);
            let dz = (far - near) / (step[1] * (j.min(1) + (grid - 1 - j).min(1)) as f32);
            let n = [-dx, 1.0, -dz];
            let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            vertices.push(Vertex {
                position: point(i, j),
                normal: n.map(|c| c / length),
                uv: [i as f32 / (grid - 1) as f32, j as f32 / (grid - 1) as f32],
            });
        }
    }
    let mut indices = Vec::with_capacity(((grid - 1) * (grid - 1) * 6) as usize);
    for j in 0..grid - 1 {
        for i in 0..grid - 1 {
            let a = j * grid + i;
            let (b, c, d) = (a + 1, a + grid, a + grid + 1);
            // Counter-clockwise seen from above.
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    let asset_id = format!("terrain/{}", terrain.heightmap);
    StaticModel::new(ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: asset_id.clone(),
        dependencies: vec![Dependency { uri: "$source".into(), sha256: fingerprint(heights) }],
        primitives: vec![Primitive { id: format!("{asset_id}#node=terrain"), vertices, indices, material: 0, transform: IDENTITY }],
        materials: vec![Material { base_color: [0.32, 0.42, 0.2, 1.0], image: 0, linear_filter: false, wrap_s: Wrap::Clamp, wrap_t: Wrap::Clamp }],
        images: vec![Image { width: 1, height: 1, rgba8: vec![255, 255, 255, 255] }],
    })
    .map_err(|e| format!("the terrain mesh is not valid: {e}"))
}
