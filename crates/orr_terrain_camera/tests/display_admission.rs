//! Independent admission and immutable-snapshot tests through public APIs only.
#![allow(clippy::float_arithmetic)]

use orr_terrain::Terrain;
use orr_terrain_camera::{
    BoomQuery, BoomResolution, CameraBoomError, TerrainCameraObstacles, TriangleId, MAX_TRIANGLES,
    ROOT_REFINEMENTS,
};

const Q16: i64 = 65_536;

#[allow(clippy::too_many_arguments)]
fn encoded(
    id: &str,
    width: u32,
    depth: u32,
    origin: [i64; 2],
    spacing: i64,
    heights: &[i64],
    holes: &[bool],
) -> Vec<u8> {
    let mut bytes = b"ORRTHF\x01\0".to_vec();
    bytes.extend((id.len() as u16).to_le_bytes());
    bytes.extend(id.as_bytes());
    bytes.extend(width.to_le_bytes());
    bytes.extend(depth.to_le_bytes());
    for value in [origin[0], origin[1], spacing] {
        bytes.extend(value.to_le_bytes());
    }
    for value in heights {
        bytes.extend(value.to_le_bytes());
    }
    bytes.extend(holes.iter().map(|hole| u8::from(*hole)));
    bytes
}

fn source(origin: [i64; 2], spacing: i64, heights: [i64; 4], hole: bool) -> Terrain {
    Terrain::load(&encoded(
        "tests/display.orrt",
        2,
        2,
        origin,
        spacing,
        &heights,
        &[hole],
    ))
    .unwrap()
}

fn query(obstacles: &TerrainCameraObstacles) -> BoomQuery {
    BoomQuery {
        expected_revision: obstacles.revision(),
        pivot: [0.5, 3.0, 0.5],
        desired_eye: [0.5, -3.0, 0.5],
        radius: 0.1,
        margin: 0.01,
        min_distance: 0.001,
    }
}

#[test]
fn canonical_export_matches_independent_topology_and_display_vertices() {
    // The missing cell is in the middle of traversal, so identities cannot be
    // reconstructed from a compacted triangle index without respecting holes.
    let heights = [
        0,
        Q16,
        2 * Q16,
        3 * Q16,
        4 * Q16,
        5 * Q16,
        6 * Q16,
        7 * Q16,
        8 * Q16,
    ];
    let source = Terrain::load(&encoded(
        "tests/nonflat.orrt",
        3,
        3,
        [-Q16, -2 * Q16],
        Q16,
        &heights,
        &[false, true, false, false],
    ))
    .unwrap();
    let obstacles = TerrainCameraObstacles::new(&source).unwrap();
    let display = orr_terrain_view::to_static_model(&source, &[])
        .unwrap()
        .unwrap();
    let primitive = &display.source().primitives[0];
    let cells = [[0, 0], [0, 1], [1, 1]];
    assert_eq!(obstacles.triangle_count(), 6);
    assert_eq!(primitive.indices.len(), 18);
    for (cell_number, cell) in cells.into_iter().enumerate() {
        let a = cell[1] * 3 + cell[0];
        for (local, indices) in [[a, a + 3, a + 4], [a, a + 4, a + 1]]
            .into_iter()
            .enumerate()
        {
            let index = cell_number * 2 + local;
            let (id, points) = obstacles.triangle(index).unwrap();
            assert_eq!(
                id,
                TriangleId {
                    cell,
                    triangle: local as u8
                }
            );
            let expected = indices.map(|vertex| {
                [
                    f64::from(vertex % 3) - 1.0,
                    heights[vertex as usize] as f64 / 65_536.0,
                    f64::from(vertex / 3) - 2.0,
                ]
            });
            assert_eq!(points, expected);
            for (corner, point) in points.into_iter().enumerate() {
                let vertex = primitive.indices[index * 3 + corner] as usize;
                assert_eq!(point, primitive.vertices[vertex].position.map(f64::from));
            }
        }
    }
    assert!(obstacles.triangle(6).is_none());
    assert!(obstacles.triangle(usize::MAX).is_none());
    assert_eq!(obstacles.revision(), source.revision());
}

#[test]
fn display_precision_and_coordinate_rejections_are_preserved() {
    let rejected = [
        // Q16 has a half-f32-ULP at magnitude 256; rounding is forbidden.
        source([0; 2], Q16, [256 * Q16 + 1, 0, 0, 0], false),
        source([256 * Q16 + 1, 0], Q16, [0; 4], false),
        source([0, 256 * Q16 + 1], Q16, [0; 4], false),
        source([0; 2], Q16, [1_000_001 * Q16, 0, 0, 0], false),
        source([-1_000_001 * Q16, 0], Q16, [0; 4], false),
        source([0, 1_000_000 * Q16], Q16, [0; 4], false),
    ];
    for source in rejected {
        assert!(orr_terrain_view::to_static_model(&source, &[]).is_err());
        assert!(matches!(
            TerrainCameraObstacles::new(&source),
            Err(CameraBoomError::Display(_))
        ));
    }
}

#[test]
fn exact_display_boundary_coordinates_are_accepted_without_downcasting_queries() {
    let source = source([999_999 * Q16, -1_000_000 * Q16], Q16, [0; 4], false);
    let display = orr_terrain_view::to_static_model(&source, &[])
        .unwrap()
        .unwrap();
    let obstacles = TerrainCameraObstacles::new(&source).unwrap();
    assert_eq!(obstacles.triangle_count(), 2);
    assert_eq!(
        obstacles.triangle(0).unwrap().1[0],
        [999_999.0, 0.0, -1_000_000.0]
    );
    assert!(display.source().primitives[0]
        .vertices
        .iter()
        .any(|vertex| vertex.position[0] == 1_000_000.0));
    let mut query = query(&obstacles);
    query.pivot = [999_999.123_456_789, 3.0, -999_999.876_543_211];
    query.desired_eye = [query.pivot[0], -3.0, query.pivot[2]];
    let BoomResolution::Retracted { pose, .. } = obstacles.resolve(query).unwrap() else {
        panic!("expected bounded vertical contact")
    };
    assert_eq!(pose.eye[0], query.pivot[0]);
    assert_eq!(pose.eye[2], query.pivot[2]);
    assert_ne!(pose.eye[0], f64::from(pose.eye[0] as f32));
}

#[test]
fn camera_minimum_spacing_is_an_explicit_additional_admission_rule() {
    for spacing in [1, 128, 255] {
        let source = source([0; 2], spacing, [0; 4], false);
        assert!(orr_terrain_view::to_static_model(&source, &[]).is_ok());
        assert!(matches!(
            TerrainCameraObstacles::new(&source),
            Err(CameraBoomError::UnsupportedGeometry(_))
        ));
    }
    let minimum = source([0; 2], 256, [0; 4], false);
    assert!(TerrainCameraObstacles::new(&minimum).is_ok());
}

#[test]
fn unused_hole_vertices_do_not_acquire_stricter_display_rules() {
    // Invalid display coordinates occur only at vertices owned exclusively by
    // the removed left cell. The right cell remains valid drawable geometry.
    let source = Terrain::load(&encoded(
        "tests/unused.orrt",
        3,
        2,
        [0; 2],
        Q16,
        &[i64::MAX, 0, 0, i64::MIN, 0, 0],
        &[true, false],
    ))
    .unwrap();
    assert!(orr_terrain_view::to_static_model(&source, &[])
        .unwrap()
        .is_some());
    let obstacles = TerrainCameraObstacles::new(&source).unwrap();
    assert_eq!(obstacles.triangle_count(), 2);
    assert_eq!(
        obstacles.triangle(0).unwrap().0,
        TriangleId {
            cell: [1, 0],
            triangle: 0
        }
    );
}

#[test]
fn empty_display_admits_no_fake_geometry_even_with_unused_extreme_values() {
    let source = source([2_000_000 * Q16; 2], Q16, [i64::MIN, i64::MAX, 0, 0], true);
    assert!(orr_terrain_view::to_static_model(&source, &[])
        .unwrap()
        .is_none());
    let obstacles = TerrainCameraObstacles::new(&source).unwrap();
    assert_eq!(obstacles.triangle_count(), 0);
    assert!(obstacles.triangle(0).is_none());
    assert!(matches!(
        obstacles.resolve(query(&obstacles)),
        Ok(BoomResolution::Clear(_))
    ));
}

#[test]
fn constructing_and_resolving_preserve_canonical_bytes_revision_and_storage() {
    let mut original = encoded("tests/immutable.orrt", 2, 2, [0; 2], Q16, &[0; 4], &[false]);
    let source = Terrain::load(&original).unwrap();
    let before = source.cook();
    let revision = source.revision();
    let obstacles = TerrainCameraObstacles::new(&source).unwrap();
    let exported = obstacles.triangle(0);
    let mut edited_export = exported.unwrap();
    edited_export.1[0][1] = 99.0;
    assert_ne!(Some(edited_export), obstacles.triangle(0));
    let storage = obstacles.storage_bytes();
    original.fill(0xff);
    for _ in 0..8 {
        assert!(matches!(
            obstacles.resolve(query(&obstacles)),
            Ok(BoomResolution::Retracted { .. })
        ));
        assert_eq!(obstacles.triangle(0), exported);
        assert_eq!(obstacles.storage_bytes(), storage);
        assert_eq!(obstacles.revision(), revision);
        assert_eq!(source.cook(), before);
        assert_eq!(source.revision(), revision);
    }
    let reloaded = Terrain::load(&before).unwrap();
    let reloaded_obstacles = TerrainCameraObstacles::new(&reloaded).unwrap();
    assert_eq!(reloaded_obstacles.revision(), revision);
    assert_eq!(reloaded_obstacles.triangle(0), exported);
    assert_eq!(
        reloaded_obstacles
            .resolve(query(&reloaded_obstacles))
            .unwrap(),
        obstacles.resolve(query(&obstacles)).unwrap()
    );
}

#[test]
fn content_or_identity_changes_require_explicit_snapshot_rebuild() {
    let old_source = source([0; 2], Q16, [0; 4], false);
    let old = TerrainCameraObstacles::new(&old_source).unwrap();
    let old_query = query(&old);
    let old_result = old.resolve(old_query).unwrap();
    let raised = source([0; 2], Q16, [Q16; 4], false);
    let raised_obstacles = TerrainCameraObstacles::new(&raised).unwrap();
    assert_ne!(old.revision(), raised_obstacles.revision());
    assert!(matches!(
        raised_obstacles.resolve(old_query),
        Err(CameraBoomError::RevisionMismatch)
    ));
    assert_eq!(old.resolve(old_query).unwrap(), old_result);
    assert_ne!(
        raised_obstacles.resolve(query(&raised_obstacles)).unwrap(),
        old_result
    );
    let renamed = Terrain::load(&encoded(
        "tests/renamed.orrt",
        2,
        2,
        [0; 2],
        Q16,
        &[0; 4],
        &[false],
    ))
    .unwrap();
    let renamed_obstacles = TerrainCameraObstacles::new(&renamed).unwrap();
    assert_ne!(old.revision(), renamed_obstacles.revision());
    assert_eq!(old.triangle(0), renamed_obstacles.triangle(0));
    assert!(matches!(
        renamed_obstacles.resolve(old_query),
        Err(CameraBoomError::RevisionMismatch)
    ));
}

#[test]
fn maximum_side_admission_and_full_query_work_remain_bounded() {
    let heights = vec![0; 129 * 129];
    let holes = vec![false; 128 * 128];
    let source = Terrain::load(&encoded(
        "tests/maximum.orrt",
        129,
        129,
        [0; 2],
        Q16,
        &heights,
        &holes,
    ))
    .unwrap();
    let obstacles = TerrainCameraObstacles::new(&source).unwrap();
    assert_eq!(MAX_TRIANGLES, 32_768);
    assert_eq!(obstacles.triangle_count(), MAX_TRIANGLES);
    assert!(obstacles.storage_bytes() <= 16 * 1024 * 1024);
    assert_eq!(
        obstacles.triangle(MAX_TRIANGLES - 1).unwrap().0,
        TriangleId {
            cell: [127, 127],
            triangle: 1
        },
    );
    let mut query = query(&obstacles);
    query.pivot = [-1.0, 100.0, -1.0];
    query.desired_eye = [-2.0, 100.0, -2.0];
    let (resolution, work) = obstacles.resolve_with_work(query).unwrap();
    assert!(matches!(resolution, BoomResolution::Clear(_)));
    assert_eq!(work.triangles, MAX_TRIANGLES);
    assert!(work.features <= MAX_TRIANGLES * 7);
    assert!(work.root_refinements <= MAX_TRIANGLES * 6 * ROOT_REFINEMENTS);
}
