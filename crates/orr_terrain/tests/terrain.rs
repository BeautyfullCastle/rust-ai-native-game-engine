use orr_fp::FP;
use orr_terrain::{Edit, Terrain, TerrainDocument, MAX_EDITS, MAX_FILE_BYTES, MAX_HISTORY};
fn f(n: i64) -> FP {
    FP::from_raw(n * 65536)
}
fn fixture() -> Terrain {
    Terrain::new(
        "fixtures/terrain.thf".into(),
        3,
        3,
        [f(-2), f(-2)],
        f(2),
        [0, 2, 0, 4, 8, -2, 0, 2, 0].map(f).to_vec(),
        vec![false; 4],
    )
    .unwrap()
}
#[test]
fn canonical_roundtrip_identity_and_hash() {
    let t = fixture();
    let bytes = t.cook();
    let loaded = Terrain::load(&bytes).unwrap();
    assert_eq!(loaded, t);
    assert_eq!(loaded.cook(), bytes);
    assert_eq!(loaded.revision(), t.revision());
    assert_eq!(
        sha2::Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        "5979f73de3ff5403bd43ba5379b0a3a38879ba0883aad055ba471eb8624a2499"
    );
    let other = Terrain::new(
        "fixtures/renamed.thf".into(),
        3,
        3,
        t.origin(),
        t.spacing(),
        t.heights().to_vec(),
        t.holes().to_vec(),
    )
    .unwrap();
    assert_ne!(other.revision(), t.revision());
}
use sha2::Digest;
#[test]
fn malformed_truncated_oversize_unknown_version_and_masks() {
    let bytes = fixture().cook();
    for n in 0..bytes.len() {
        assert!(Terrain::load(&bytes[..n]).is_err(), "prefix {n}");
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(Terrain::load(&extra).is_err());
    for index in 0..8 {
        let mut bad = bytes.clone();
        bad[index] ^= 0xff;
        assert!(Terrain::load(&bad).is_err());
    }
    for value in [2, 127, 255] {
        let mut bad = bytes.clone();
        *bad.last_mut().unwrap() = value;
        assert!(Terrain::load(&bad).is_err());
    }
    assert!(Terrain::load(&vec![0; MAX_FILE_BYTES + 1]).is_err());
    let mut bad = bytes.clone();
    bad[8..10].copy_from_slice(&u16::MAX.to_le_bytes());
    assert!(Terrain::load(&bad).is_err());
    let mut bad = bytes.clone();
    bad[10] = 255;
    assert!(Terrain::load(&bad).is_err());
    let offset = 10 + fixture().asset_id().len();
    for side in [0, 1, 130, u32::MAX] {
        let mut bad = bytes.clone();
        bad[offset..offset + 4].copy_from_slice(&side.to_le_bytes());
        assert!(Terrain::load(&bad).is_err());
    }
    for spacing in [0i64, -1, i64::MIN] {
        let mut bad = bytes.clone();
        bad[offset + 24..offset + 32].copy_from_slice(&spacing.to_le_bytes());
        assert!(Terrain::load(&bad).is_err());
    }
}
#[test]
fn bounds_and_extreme_coordinate_arithmetic() {
    let construct = |origin, spacing| {
        Terrain::new(
            "a".into(),
            2,
            2,
            origin,
            spacing,
            vec![
                FP::from_raw(i64::MIN),
                FP::from_raw(i64::MAX),
                FP::from_raw(i64::MAX),
                FP::from_raw(i64::MIN),
            ],
            vec![false],
        )
    };
    assert!(construct([FP::from_raw(i64::MAX), f(0)], f(1)).is_err());
    assert!(construct([f(0), FP::from_raw(i64::MAX)], f(1)).is_err());
    let t = construct([FP::from_raw(i64::MIN); 2], FP::from_raw(i64::MAX)).unwrap();
    assert_eq!(
        t.sample(FP::from_raw(i64::MIN), FP::from_raw(i64::MIN)),
        Some(FP::from_raw(i64::MIN))
    );
    assert_eq!(
        t.sample(FP::from_raw(-1), FP::from_raw(-1)),
        Some(FP::from_raw(i64::MIN))
    );
    assert_eq!(
        t.sample(FP::from_raw(-1), FP::from_raw(i64::MIN)),
        Some(FP::from_raw(i64::MAX))
    );
    assert_eq!(t.sample(FP::from_raw(i64::MAX), f(0)), None);
    assert!(t
        .sample(FP::from_raw(i64::MIN / 2), FP::from_raw(i64::MIN / 2 + 1))
        .is_some());
    assert!(t.vertex_position(u32::MAX).is_none());
    for id in ["", "/a", "a/../b", "a//b", "a/./b", "a\\b", "♥"] {
        assert!(
            Terrain::new(id.into(), 2, 2, [f(0); 2], f(1), vec![f(0); 4], vec![false]).is_err()
        );
    }
    assert!(Terrain::new(
        "a".into(),
        u32::MAX,
        u32::MAX,
        [f(0); 2],
        f(1),
        vec![],
        vec![]
    )
    .is_err());
    assert!(Terrain::new(
        "a".into(),
        2,
        2,
        [f(0); 2],
        f(1),
        vec![f(0); 3],
        vec![false]
    )
    .is_err());
    assert!(Terrain::new("a".into(), 2, 2, [f(0); 2], f(1), vec![f(0); 4], vec![]).is_err());
}
// Independent oriented-area barycentric oracle. It operates on triangle vertices
// and does not use cell selection or the production interpolation formula.
fn triangle_height(points: [[FP; 3]; 3], x: FP, z: FP) -> Option<FP> {
    let p = points.map(|v| v.map(|n| i128::from(n.raw())));
    let cross = |a: [i128; 2], b: [i128; 2], c: [i128; 2]| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    let q = [i128::from(x.raw()), i128::from(z.raw())];
    let flat = p.map(|v| [v[0], v[2]]);
    let area = cross(flat[0], flat[1], flat[2]);
    let w = [
        cross(q, flat[1], flat[2]),
        cross(flat[0], q, flat[2]),
        cross(flat[0], flat[1], q),
    ];
    if w.iter().any(|a| *a != 0 && a.signum() != area.signum()) {
        return None;
    }
    let sum: i128 = (0..3).map(|i| p[i][1] * w[i]).sum();
    let (sum, area) = if area < 0 { (-sum, -area) } else { (sum, area) };
    Some(FP::from_raw(sum.div_euclid(area) as i64))
}
#[test]
fn triangles_match_independent_oracle_vertices_interiors_diagonal_seams_and_outer_edges() {
    let t = fixture();
    assert_eq!(&t.triangles()[..2], &[[0, 3, 4], [0, 4, 1]]);
    for triangle in t.triangles() {
        for i in triangle {
            let p = t.vertex_position(i).unwrap();
            assert_eq!(t.sample(p[0], p[2]), Some(p[1]));
        }
    }
    for xi in -16..=16 {
        for zi in -16..=16 {
            let x = FP::from_raw(xi * 8192);
            let z = FP::from_raw(zi * 8192);
            let oracle = t
                .triangles()
                .into_iter()
                .find_map(|ids| triangle_height(ids.map(|i| t.vertex_position(i).unwrap()), x, z))
                .unwrap();
            assert_eq!(t.sample(x, z), Some(oracle), "x={x} z={z}");
        }
    }
    // Cell a=0,b=2,c=4,d=8 center lies on a-d, height 4; bilinear gives 3.5.
    assert_eq!(t.sample(f(-1), f(-1)), Some(f(4)));
    assert_ne!(t.sample(f(-1), f(-1)), Some(FP::from_raw(229376)));
    for (x, z) in [(-131073, 0), (131073, 0), (0, -131073), (0, 131073)] {
        assert_eq!(t.sample(FP::from_raw(x), FP::from_raw(z)), None);
    }
}
#[test]
fn explicit_hole_seam_ownership_and_empty_geometry() {
    let mut doc = TerrainDocument::new(fixture());
    doc.apply(&[Edit::SetHole {
        x: 1,
        z: 0,
        hole: true,
    }])
    .unwrap();
    let t = doc.terrain();
    assert_eq!(t.triangles().len(), 6);
    assert!(t.sample(f(0), f(-1)).is_none()); // seam belongs to right cell
    assert!(t.sample(FP::from_raw(-1), f(-1)).is_some());
    assert!(t.sample(f(2), f(-1)).is_none()); // outer edge uses final cell
    assert!(t.sample(f(1), f(0)).is_some()); // +Z cell owns seam
    doc.apply(&[
        Edit::SetHole {
            x: 0,
            z: 0,
            hole: true,
        },
        Edit::SetHole {
            x: 0,
            z: 1,
            hole: true,
        },
        Edit::SetHole {
            x: 1,
            z: 1,
            hole: true,
        },
    ])
    .unwrap();
    assert!(doc.terrain().triangles().is_empty());
    for x in -2..=2 {
        for z in -2..=2 {
            assert!(doc.terrain().sample(f(x), f(z)).is_none());
        }
    }
}
#[test]
fn rejected_edit_preserves_bytes_revision_history_and_redo() {
    let mut doc = TerrainDocument::new(fixture());
    let original = doc.terrain().cook();
    let revision = doc.terrain().revision();
    doc.apply(&[Edit::SetHeight {
        x: 0,
        z: 0,
        height: f(9),
    }])
    .unwrap();
    let changed = doc.terrain().cook();
    assert!(doc.undo());
    assert!(doc
        .apply(&[
            Edit::SetHeight {
                x: 0,
                z: 0,
                height: f(11)
            },
            Edit::SetHole {
                x: u32::MAX,
                z: 0,
                hole: true
            }
        ])
        .is_err());
    assert_eq!(doc.terrain().cook(), original);
    assert_eq!(doc.terrain().revision(), revision);
    assert!(doc.redo());
    assert_eq!(doc.terrain().cook(), changed);
    assert!(doc.undo());
    assert!(doc
        .apply(&vec![
            Edit::SetHole {
                x: 0,
                z: 0,
                hole: false
            };
            MAX_EDITS + 1
        ])
        .is_err());
    assert_eq!(doc.terrain().cook(), original);
    assert!(doc.redo());
    doc.apply(&[
        Edit::SetHeight {
            x: 1,
            z: 1,
            height: f(-4),
        },
        Edit::SetHole {
            x: 0,
            z: 1,
            hole: true,
        },
    ])
    .unwrap();
    let saved = doc.terrain().cook();
    let reopened = TerrainDocument::new(Terrain::load(&saved).unwrap());
    assert_eq!(reopened.terrain().cook(), saved);
    assert_eq!(reopened.terrain().revision(), doc.terrain().revision());
    assert!(doc.undo());
    assert!(doc.redo());
    assert_eq!(doc.terrain().cook(), saved);
}
#[test]
fn history_is_bounded_and_noops_do_not_consume_history() {
    let mut doc = TerrainDocument::new(fixture());
    for n in 1..=MAX_HISTORY + 3 {
        doc.apply(&[Edit::SetHeight {
            x: 0,
            z: 0,
            height: f(n as i64),
        }])
        .unwrap();
    }
    doc.apply(&[]).unwrap();
    doc.apply(&[Edit::SetHeight {
        x: 0,
        z: 0,
        height: f((MAX_HISTORY + 3) as i64),
    }])
    .unwrap();
    for _ in 0..MAX_HISTORY {
        assert!(doc.undo());
    }
    assert!(!doc.undo());
    assert_eq!(doc.terrain().heights()[0], f(3));
    for _ in 0..MAX_HISTORY {
        assert!(doc.redo());
    }
    assert!(!doc.redo());
    assert!(doc.undo());
    doc.apply(&[Edit::SetHole {
        x: 0,
        z: 0,
        hole: true,
    }])
    .unwrap();
    assert!(!doc.redo());
}
#[test]
fn normals_slopes_and_dirty_cells_follow_the_selected_triangle() {
    let t = fixture();
    let s = t.surface(f(-1), f(-1)).unwrap();
    assert_eq!(s.height, f(4));
    assert!(s.normal[0].raw() < 0 && s.normal[1].raw() > 0 && s.normal[2].raw() < 0);
    // First triangle has rise/run gradients (2,2), hence sqrt(8).
    assert_eq!(s.slope, Some(FP::from_raw(185363)));
    let other = t.surface(FP::from_raw(-32768), f(-1)).unwrap();
    assert_ne!(other.normal, s.normal);
    assert_eq!(other.slope, Some(FP::from_raw(207243))); // sqrt(1²+3²)
    let flat = Terrain::new(
        "flat".into(),
        2,
        2,
        [f(0); 2],
        f(1),
        vec![f(3); 4],
        vec![false],
    )
    .unwrap();
    assert_eq!(flat.surface(f(0), f(0)).unwrap().normal, [f(0), f(1), f(0)]);
    assert_eq!(flat.surface(f(1), f(1)).unwrap().slope, Some(f(0)));
    let steep = Terrain::new(
        "steep".into(),
        2,
        2,
        [f(0); 2],
        FP::from_raw(1),
        vec![
            FP::from_raw(i64::MIN),
            FP::from_raw(i64::MAX),
            FP::from_raw(i64::MIN),
            FP::from_raw(i64::MAX),
        ],
        vec![false],
    )
    .unwrap();
    assert_eq!(steep.surface(f(0), f(0)).unwrap().slope, None);
    let mut doc = TerrainDocument::new(t);
    assert_eq!(doc.dirty_region(), None);
    doc.apply(&[Edit::SetHeight {
        x: 0,
        z: 0,
        height: f(1),
    }])
    .unwrap();
    assert_eq!(
        doc.dirty_region(),
        Some(orr_terrain::DirtyRegion {
            min: [0, 0],
            max: [0, 0]
        })
    );
    doc.clear_dirty();
    assert_eq!(doc.dirty_region(), None);
    doc.apply(&[Edit::SetHeight {
        x: 1,
        z: 1,
        height: f(9),
    }])
    .unwrap();
    assert_eq!(
        doc.dirty_region(),
        Some(orr_terrain::DirtyRegion {
            min: [0, 0],
            max: [1, 1]
        })
    );
    let dirty = doc.dirty_region();
    assert!(doc
        .apply(&[Edit::SetHeight {
            x: 99,
            z: 99,
            height: f(1)
        }])
        .is_err());
    assert_eq!(doc.dirty_region(), dirty);
    doc.clear_dirty();
    assert!(doc.undo());
    assert_eq!(doc.dirty_region(), dirty);
}
#[test]
fn maximum_asset_budget_and_once_only_negative_rounding() {
    let t = Terrain::new(
        "a".repeat(256),
        129,
        129,
        [f(-64); 2],
        f(1),
        vec![f(0); 129 * 129],
        vec![false; 128 * 128],
    )
    .unwrap();
    assert_eq!(t.cook().len(), MAX_FILE_BYTES);
    assert_eq!(Terrain::load(&t.cook()).unwrap(), t);
    assert_eq!(t.triangles().len(), 128 * 128 * 2);
    let tiny = Terrain::new(
        "tiny".into(),
        2,
        2,
        [FP::ZERO; 2],
        FP::from_raw(3),
        vec![FP::ZERO, FP::from_raw(-1), FP::ZERO, FP::from_raw(-1)],
        vec![false],
    )
    .unwrap();
    assert_eq!(
        tiny.sample(FP::from_raw(1), FP::from_raw(1)),
        Some(FP::from_raw(-1))
    );
    assert_eq!(
        tiny.sample(FP::from_raw(2), FP::from_raw(1)),
        Some(FP::from_raw(-1))
    );
    assert_eq!(tiny.sample(FP::ZERO, FP::from_raw(1)), Some(FP::ZERO));
}
