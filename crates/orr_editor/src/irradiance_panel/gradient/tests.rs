use super::*;

#[test]
fn gradient_all_axes_dimensions_endpoints_and_transverse_nodes() {
    for dimensions in [[2, 2, 2], [3, 3, 3], [4, 4, 4], [2, 3, 4]] {
        for axis in 0..3 {
            let source = IrradianceGrid {
                dimensions,
                coefficients: vec![
                    constant_irradiance([9.0; 3]).unwrap();
                    dimensions.iter().product::<u32>() as usize
                ],
                provenance: IrradianceProvenance::Imported,
                ..IrradianceGrid::default()
            };
            let before = source.clone();
            let result = build_gradient(
                &source,
                GradientSpec {
                    axis,
                    start: [0.0, 3.0, 6.0],
                    end: [6.0, 0.0, 3.0],
                },
            )
            .unwrap();
            for z in 0..dimensions[2] {
                for y in 0..dimensions[1] {
                    for x in 0..dimensions[0] {
                        let node = [x, y, z];
                        let t = node[axis] as f32 / (dimensions[axis] - 1) as f32;
                        let expected =
                            constant_irradiance([6.0 * t, 3.0 * (1.0 - t), 6.0 - 3.0 * t]).unwrap();
                        let got = result.coefficients[result.node_index(node).unwrap()];
                        for i in 0..9 {
                            for c in 0..3 {
                                assert!((got[i][c] - expected[i][c]).abs() < 0.00001);
                            }
                        }
                    }
                }
            }
            assert_eq!(source, before);
            assert_eq!(result.provenance, IrradianceProvenance::Authored);
            assert_eq!(result.origin, source.origin);
            assert_eq!(result.spacing, source.spacing);
            assert_eq!(result.dimensions, source.dimensions);
            assert_eq!(result.enabled, source.enabled);
            assert_eq!(result.version, source.version);
        }
    }
}

#[test]
fn gradient_invalid_axis_endpoints_or_grid_leave_source_unchanged() {
    let source = IrradianceGrid::default();
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.01, 2821.0] {
        for channel in 0..3 {
            for end in [false, true] {
                let mut spec = GradientSpec {
                    axis: 0,
                    start: [0.0; 3],
                    end: [1.0; 3],
                };
                if end {
                    spec.end[channel] = bad;
                } else {
                    spec.start[channel] = bad;
                }
                assert!(build_gradient(&source, spec).is_err());
            }
        }
    }
    assert!(
        build_gradient(
            &source,
            GradientSpec {
                axis: 3,
                start: [0.0; 3],
                end: [1.0; 3]
            }
        )
        .is_err()
    );
    let mut invalid = source.clone();
    invalid.dimensions[0] = 1;
    assert!(
        build_gradient(
            &invalid,
            GradientSpec {
                axis: 0,
                start: [0.0; 3],
                end: [1.0; 3]
            }
        )
        .is_err()
    );
    assert_eq!(source, IrradianceGrid::default());
}

#[test]
fn gradient_equal_endpoints_reproduce_existing_constant_authoring() {
    let source = IrradianceGrid::default();
    for color in [[0.0; 3], [1.0, 2.0, 3.0], [2820.0; 3]] {
        let result = build_gradient(
            &source,
            GradientSpec {
                axis: 2,
                start: color,
                end: color,
            },
        )
        .unwrap();
        assert!(
            result
                .coefficients
                .iter()
                .all(|c| *c == constant_irradiance(color).unwrap())
        );
    }
}
