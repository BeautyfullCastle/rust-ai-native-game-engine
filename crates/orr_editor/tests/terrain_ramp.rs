//! Independent raw-integer candidate and real document transaction contract.
#![cfg(feature = "terrain")]
use orr_editor::{
    terrain_document::TerrainSession,
    terrain_ramp::{RampCandidate, RampSpec},
};
use orr_fp::FP;
use orr_terrain::{Edit, Terrain, TerrainDocument};

fn terrain(side: u32, heights: Vec<i64>, holes: Vec<bool>) -> Terrain {
    Terrain::new(
        "independent/ramp".into(),
        side,
        side,
        [FP::from_raw(-12345), FP::from_raw(54321)],
        FP::from_raw(98304),
        heights.into_iter().map(FP::from_raw).collect(),
        holes,
    )
    .unwrap()
}
fn raw(t: &Terrain) -> Vec<i64> {
    t.heights().iter().map(|v| v.raw()).collect()
}
fn apply(t: &Terrain, s: RampSpec) -> Terrain {
    let c = RampCandidate::build(t, s).unwrap();
    let mut d = TerrainDocument::new(t.clone());
    d.apply(c.edits()).unwrap();
    d.terrain().clone()
}

#[test]
fn grouped_fixed_arrays_axes_signs_fractional_ties_reversal_and_preservation() {
    // Each row is an independently specified exact rational interpolation, not
    // an oracle that calls production sampling or candidate helpers.
    let cases: &[(i64, i64, [i64; 5])] = &[
        (0, 8, [0, 2, 4, 6, 8]),
        (8, 0, [8, 6, 4, 2, 0]),
        (-8, 0, [-8, -6, -4, -2, 0]),
        (0, -8, [0, -2, -4, -6, -8]),
        (-3, 3, [-3, -2, 0, 2, 3]),
        (1, 3, [1, 2, 2, 3, 3]),
        (-1, -3, [-1, -2, -2, -3, -3]),
        (65537, 196611, [65537, 98306, 131074, 163843, 196611]),
        (
            i64::MIN,
            i64::MAX,
            [
                i64::MIN,
                -4611686018427387904,
                -1,
                4611686018427387903,
                i64::MAX,
            ],
        ),
    ];
    for &(a, b, line) in cases {
        for axis in 0..2 {
            for width in [1, 2] {
                let mut heights = vec![91; 49];
                let point = |v: u32| if axis == 0 { [v, 3] } else { [3, v] };
                let start = point(1);
                let end = point(5);
                heights[(start[1] * 7 + start[0]) as usize] = a;
                heights[(end[1] * 7 + end[0]) as usize] = b;
                let original = terrain(7, heights.clone(), vec![false; 36]);
                let spec = RampSpec {
                    start,
                    end,
                    half_width: width,
                };
                let result = apply(&original, spec);
                let mut expected = heights;
                for along in 1..=5 {
                    for across in 3 - width..=3 + width {
                        let p = if axis == 0 {
                            [along, across]
                        } else {
                            [across, along]
                        };
                        expected[(p[1] * 7 + p[0]) as usize] = line[(along - 1) as usize];
                    }
                }
                assert_eq!(
                    raw(&result),
                    expected,
                    "axis={axis}, width={width}, a={a}, b={b}"
                );
                assert_eq!(
                    result,
                    apply(
                        &original,
                        RampSpec {
                            start: end,
                            end: start,
                            half_width: width
                        }
                    )
                );
                assert_eq!(result.origin(), original.origin());
                assert_eq!(result.spacing(), original.spacing());
                assert_eq!(result.asset_id(), original.asset_id());
                assert_eq!(result.holes(), original.holes());
                assert_eq!(
                    original.cook(),
                    terrain(7, raw(&original), vec![false; 36]).cook()
                );
            }
        }
    }
}

#[test]
fn grouped_edges_invalid_shapes_hole_halo_limits_and_noop_preserve_source() {
    let base = terrain(9, vec![0; 81], vec![false; 64]);
    let good = RampSpec {
        start: [0, 4],
        end: [8, 4],
        half_width: 1,
    };
    assert_eq!(
        RampCandidate::build(&base, good)
            .unwrap()
            .changed_vertices(),
        0
    );
    assert_eq!(apply(&base, good), base);
    for spec in [
        RampSpec {
            start: [4, 4],
            end: [4, 4],
            half_width: 1,
        },
        RampSpec {
            start: [1, 1],
            end: [7, 7],
            half_width: 1,
        },
        RampSpec {
            start: [0, 0],
            end: [8, 0],
            half_width: 1,
        },
        RampSpec {
            start: [0, 4],
            end: [9, 4],
            half_width: 1,
        },
        RampSpec {
            half_width: 0,
            ..good
        },
        RampSpec {
            half_width: u32::MAX,
            ..good
        },
    ] {
        let bytes = base.cook();
        assert!(RampCandidate::build(&base, spec).is_err());
        assert_eq!(base.cook(), bytes);
    }
    // Full vertex strip z=3..5 touches cell rows 2..5, including both outward halos.
    for z in 0..8 {
        for x in 0..8 {
            let mut holes = vec![false; 64];
            holes[(z * 8 + x) as usize] = true;
            let t = terrain(9, vec![0; 81], holes);
            let before = t.cook();
            assert_eq!(
                RampCandidate::build(&t, good).is_err(),
                (2..=5).contains(&z),
                "hole {x},{z}"
            );
            assert_eq!(t.cook(), before);
        }
    }
    for x in [0, 1, 6, 7] {
        let mut holes = vec![false; 64];
        holes[4 * 8 + x] = true;
        let t = terrain(9, vec![0; 81], holes);
        assert_eq!(
            RampCandidate::build(
                &t,
                RampSpec {
                    start: [2, 4],
                    end: [6, 4],
                    half_width: 1
                }
            )
            .is_err(),
            x == 1 || x == 6
        );
    }
    let mut document = TerrainDocument::new(base.clone());
    document
        .apply(&[Edit::SetHeight {
            x: 0,
            z: 0,
            height: FP::ONE,
        }])
        .unwrap();
    assert!(document.undo());
    let noop = RampCandidate::build(document.terrain(), good).unwrap();
    document.apply(noop.edits()).unwrap();
    assert!(document.redo());
    assert_eq!(document.terrain().heights()[0], FP::ONE);
    let mut heights = vec![7; 129 * 129];
    heights[64 * 129] = 0;
    heights[64 * 129 + 128] = 0;
    let large = terrain(129, heights, vec![false; 128 * 128]);
    assert!(RampCandidate::build(
        &large,
        RampSpec {
            start: [0, 64],
            end: [128, 64],
            half_width: 64
        }
    )
    .is_err());
    // A full-surface no-op is not incorrectly rejected by the changed-edit cap.
    let flat = terrain(129, vec![0; 129 * 129], vec![false; 128 * 128]);
    assert_eq!(
        RampCandidate::build(
            &flat,
            RampSpec {
                start: [0, 64],
                end: [128, 64],
                half_width: 64
            }
        )
        .unwrap()
        .changed_vertices(),
        0
    );
}

#[test]
fn actual_session_single_transaction_noop_undo_redo_failure_save_and_fresh_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let scene = dir.path().join("scene.yaml");
    std::fs::write(&scene, "fixture").unwrap();
    let mut heights = vec![65536; 81];
    heights[4 * 9 + 1] = 0;
    heights[4 * 9 + 7] = 196608;
    let original = terrain(9, heights, vec![false; 64]);
    std::fs::write(dir.path().join("ramp.orrt"), original.cook()).unwrap();
    let mut s = TerrainSession::default();
    s.set_scene(Some(&scene)).unwrap();
    s.open_local("ramp.orrt").unwrap();
    let candidate = RampCandidate::build(
        s.terrain().unwrap(),
        RampSpec {
            start: [1, 4],
            end: [7, 4],
            half_width: 1,
        },
    )
    .unwrap();
    let before = s.bytes().unwrap().to_vec();
    let model = s.model().unwrap().clone();
    assert!(!s.can_undo());
    assert!(!s.dirty());
    assert_eq!(s.bytes().unwrap(), before);
    s.set_render_admission_error(Some("deliberate renderer refusal".into()));
    assert!(s.apply(candidate.edits()).is_err());
    assert_eq!(s.bytes().unwrap(), before);
    assert!(!s.can_undo());
    assert!(!s.dirty());
    assert!(std::sync::Arc::ptr_eq(s.model().unwrap(), &model));
    s.set_render_admission_error(None);
    s.apply(candidate.edits()).unwrap();
    let after = s.bytes().unwrap().to_vec();
    assert_ne!(after, before);
    assert!(!candidate.is_current(s.terrain().unwrap()));
    let noop = RampCandidate::build(s.terrain().unwrap(), candidate.spec()).unwrap();
    assert!(noop.edits().is_empty());
    s.apply(noop.edits()).unwrap();
    assert!(s.undo().unwrap());
    assert_eq!(s.bytes().unwrap(), before);
    assert!(!s.undo().unwrap());
    assert!(s.redo().unwrap());
    assert_eq!(s.bytes().unwrap(), after);
    assert!(!s.redo().unwrap());
    assert!(s
        .apply(&[Edit::SetHeight {
            x: 99,
            z: 0,
            height: FP::ZERO
        }])
        .is_err());
    assert_eq!(s.bytes().unwrap(), after);
    s.save().unwrap();
    assert!(!s.dirty());
    assert_eq!(std::fs::read(dir.path().join("ramp.orrt")).unwrap(), after);
    let mut reopened = TerrainSession::default();
    reopened.set_scene(Some(&scene)).unwrap();
    reopened.open_local("ramp.orrt").unwrap();
    assert_eq!(reopened.bytes().unwrap(), after);
    assert!(!reopened.can_undo());
    for z in 0..9 {
        for x in 0..9 {
            let expected = if (3..=5).contains(&z) && (1..=7).contains(&x) {
                (x - 1) * 32768
            } else {
                original.heights()[(z * 9 + x) as usize].raw()
            };
            assert_eq!(
                reopened.terrain().unwrap().heights()[(z * 9 + x) as usize].raw(),
                expected
            );
        }
    }
}
