//! The terrain mesh built from a heightmap image.
#![cfg(feature = "models")]
#![allow(clippy::float_arithmetic)]

use orr_gfx_view::terrain_mesh::{read_heightmap, terrain_model, MAX_TERRAIN_GRID};
use orr_gfx_view::TerrainRef;

/// A 16-bit grayscale PNG of `width` x `height` with the given values.
fn png16(width: u32, height: u32, values: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Sixteen);
    let mut writer = encoder.write_header().unwrap();
    let data: Vec<u8> = values.iter().flat_map(|v| v.to_be_bytes()).collect();
    writer.write_image_data(&data).unwrap();
    drop(writer);
    bytes
}

fn terrain(resolution: u32) -> TerrainRef {
    TerrainRef {
        entity: orr_ecs::Entity::NONE,
        heightmap: "valley.png".into(),
        center: [0.0; 3],
        size: [100.0, 50.0],
        height: 20.0,
        resolution,
        cast_shadows: true,
    }
}

#[test]
fn a_heightmap_becomes_a_grid_with_its_heights_size_and_smooth_normals() {
    // A ramp rising along x: black on the left, white on the right.
    let (w, h) = (3, 2);
    let bytes = png16(w, h, &[0, 32768, 65535, 0, 32768, 65535]);
    let (width, height, heights) = read_heightmap(&bytes).unwrap();
    assert_eq!((width, height), (3, 2));
    assert!((heights[1] - 0.5).abs() < 1e-4);
    let model = terrain_model(&terrain(8), width, height, &heights).unwrap();
    let p = &model.source().primitives[0];
    // The grid is no finer than the image.
    assert_eq!(p.vertices.len(), 9);
    let first = p.vertices[0].position;
    let last = p.vertices[8].position;
    assert_eq!((first[0], first[2], first[1]), (-50.0, -25.0, 0.0));
    assert_eq!((last[0], last[2]), (50.0, 25.0));
    assert!((last[1] - 20.0).abs() < 1e-3, "white is the full height: {last:?}");
    // The ramp leans every normal toward -x, and the uv spans the terrain.
    assert!(p.vertices.iter().all(|v| v.normal[0] < 0.0 && v.normal[1] > 0.9));
    assert_eq!((p.vertices[0].uv, p.vertices[8].uv), ([0.0, 0.0], [1.0, 1.0]));
}

#[test]
fn a_large_heightmap_is_sampled_down_to_the_grid_limit() {
    let side = 400;
    let bytes = png16(side, side, &vec![1000; (side * side) as usize]);
    let (w, h, heights) = read_heightmap(&bytes).unwrap();
    let model = terrain_model(&terrain(9999), w, h, &heights).unwrap();
    assert_eq!(model.source().primitives[0].vertices.len(), (MAX_TERRAIN_GRID * MAX_TERRAIN_GRID) as usize);
}

#[test]
fn images_that_are_not_heightmaps_are_refused() {
    assert!(read_heightmap(b"not a png").unwrap_err().contains("not a PNG"));
    assert!(read_heightmap(&png16(1, 1, &[0])).unwrap_err().contains("2 to 8192"));
}
