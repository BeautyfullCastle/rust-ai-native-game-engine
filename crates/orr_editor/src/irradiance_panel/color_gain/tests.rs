use super::*;
fn directional() -> IrradianceGrid {
    let mut grid = IrradianceGrid {
        enabled: true,
        provenance: IrradianceProvenance::Imported,
        origin: [-2.0, -3.0, -4.0],
        spacing: [4.0, 6.0, 8.0],
        ..IrradianceGrid::default()
    };
    for (index, node) in grid.coefficients.iter_mut().enumerate() {
        // Distinct nonzero signed values for every basis/channel at every node.
        let amplitude = (index + 1) as f32;
        *node = [
            [20.0 * amplitude, 24.0 * amplitude, 28.0 * amplitude],
            [0.1 * amplitude, -0.2 * amplitude, 0.3 * amplitude],
            [-0.4 * amplitude, 0.5 * amplitude, -0.6 * amplitude],
            [0.7 * amplitude, -0.8 * amplitude, 0.9 * amplitude],
            [-0.1 * amplitude, 0.2 * amplitude, -0.3 * amplitude],
            [0.4 * amplitude, -0.5 * amplitude, 0.6 * amplitude],
            [-0.7 * amplitude, 0.8 * amplitude, -0.9 * amplitude],
            [0.15 * amplitude, -0.25 * amplitude, 0.35 * amplitude],
            [-0.45 * amplitude, 0.55 * amplitude, -0.65 * amplitude],
        ];
    }
    grid.validate().unwrap();
    grid
}
#[test]
fn all_signed_coefficients_scaled_and_geometry_source_preserved() {
    // Independent literal oracle: every value and product is binary-exact.
    let mut literal = IrradianceGrid::default();
    literal.coefficients[0] = [
        [1.0, 2.0, 4.0],
        [-1.0, 2.0, -4.0],
        [2.0, -4.0, 8.0],
        [-2.0, -4.0, -8.0],
        [4.0, 8.0, -16.0],
        [-4.0, -8.0, 16.0],
        [0.5, -1.0, 2.0],
        [-0.5, 1.0, -2.0],
        [8.0, -16.0, 32.0],
    ];
    let literal_result = apply_gain(&literal, [0.5, 2.0, 1.25]).unwrap().unwrap();
    assert_eq!(
        literal_result.coefficients[0],
        [
            [0.5, 4.0, 5.0],
            [-0.5, 4.0, -5.0],
            [1.0, -8.0, 10.0],
            [-1.0, -8.0, -10.0],
            [2.0, 16.0, -20.0],
            [-2.0, -16.0, 20.0],
            [0.25, -2.0, 2.5],
            [-0.25, 2.0, -2.5],
            [4.0, -32.0, 40.0],
        ]
    );
    let source = directional();
    let original = source.to_json().unwrap();
    let gain = [0.5, 2.0, 1.25];
    let result = apply_gain(&source, gain).unwrap().unwrap();
    for (actual, old) in result.coefficients.iter().zip(&source.coefficients) {
        for (actual, old) in actual.iter().zip(old) {
            for channel in 0..3 {
                let expected = (f64::from(old[channel]) * f64::from(gain[channel])) as f32;
                assert_eq!(actual[channel].to_bits(), expected.to_bits());
                assert_eq!(
                    actual[channel].is_sign_negative(),
                    old[channel].is_sign_negative()
                );
            }
        }
    }
    let mut expected = source.clone();
    expected.coefficients = result.coefficients.clone();
    expected.provenance = IrradianceProvenance::Authored;
    assert_eq!(result, expected);
    assert_eq!(source.to_json().unwrap(), original);
    let mut disabled = source.clone();
    disabled.enabled = false;
    assert!(!apply_gain(&disabled, gain).unwrap().unwrap().enabled);
}
#[test]
fn actual_sampling_keeps_directional_and_spatial_pattern() {
    let source = directional();
    let gain = [0.5, 2.0, 1.25];
    let result = apply_gain(&source, gain).unwrap().unwrap();
    for position in [
        [0.0; 3],
        [-1.0, -1.0, -1.0],
        [1.0, 2.0, 3.0],
        [-2.0, -3.0, -4.0],
        [2.0, 3.0, 4.0],
    ] {
        for normal in [
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [1.0, 2.0, 3.0],
        ] {
            let old = source.sample(position, normal).unwrap();
            let new = result.sample(position, normal).unwrap();
            assert_eq!(old.weight, new.weight);
            for (channel, g) in gain.iter().enumerate() {
                let expected = old.irradiance[channel] * g;
                assert!(
                    (new.irradiance[channel] - expected).abs() <= expected.abs().max(1.0) * 2.0e-6
                );
            }
        }
    }
    assert_ne!(
        source.sample([0.0; 3], [1.0, 0.0, 0.0]).unwrap().irradiance,
        source
            .sample([0.0; 3], [-1.0, 0.0, 0.0])
            .unwrap()
            .irradiance
    );
}
#[test]
fn zero_identity_and_unchanged_preserve_noop_contract() {
    let mut source = directional();
    source.provenance = IrradianceProvenance::Baked;
    source.coefficients[0][0][0] = -0.0;
    assert!(apply_gain(&source, [1.0; 3]).unwrap().is_none());
    let result = apply_gain(&source, [0.0; 3]).unwrap().unwrap();
    assert!(result
        .coefficients
        .iter()
        .flatten()
        .flatten()
        .all(|v| *v == 0.0));
    assert_eq!(result.provenance, IrradianceProvenance::Authored);
    let zero = IrradianceGrid {
        provenance: IrradianceProvenance::Baked,
        ..IrradianceGrid::default()
    };
    assert!(apply_gain(&zero, [16.0, 0.5, 0.0]).unwrap().is_none());
    let mut only_red = zero;
    only_red.coefficients[0][0][0] = 1.0;
    assert!(apply_gain(&only_red, [1.0, 2.0, 4.0]).unwrap().is_none());
}
#[test]
fn invalid_gains_source_and_coefficient_overflow_are_atomic_errors() {
    let source = directional();
    let bytes = source.to_json().unwrap();
    for value in [-0.1, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 16.000002] {
        for channel in 0..3 {
            let mut gain = [1.0; 3];
            gain[channel] = value;
            assert!(apply_gain(&source, gain).is_err());
        }
    }
    assert_eq!(source.to_json().unwrap(), bytes);
    let mut invalid = source.clone();
    invalid.coefficients.clear();
    assert!(apply_gain(&invalid, [1.0; 3]).is_err());
    for sign in [-1.0, 1.0] {
        let mut bound = source.clone();
        bound.coefficients[7][8][2] = sign * MAX_COEFFICIENT;
        assert!(apply_gain(&bound, [1.0, 1.0, 1.000001]).is_err());
        assert!(apply_gain(&bound, [1.0; 3]).unwrap().is_none());
    }
    let mut exact = IrradianceGrid::default();
    exact.coefficients[0][0][0] = 625.0;
    assert_eq!(
        apply_gain(&exact, [16.0, 1.0, 1.0])
            .unwrap()
            .unwrap()
            .coefficients[0][0][0],
        MAX_COEFFICIENT
    );
}
