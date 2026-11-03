//! Conservative import-only conversion, not a runtime/schema decomposition.
use crate::{animation::Trs, determinant, invalid, Error};

// These bounds admit f32 serialization roundoff, not general shear repair.
const ORTHOGONAL_EPSILON: f64 = 8.0 * f32::EPSILON as f64;
const RECONSTRUCTION_EPSILON: f64 = 32.0 * f32::EPSILON as f64;

/// The caller first applies Node::transform, including normal_matrix's existing
/// affine/component/inverse/determinant limits and matrix/TRS exclusivity.
pub(super) fn decompose(matrix: [[f32; 4]; 4]) -> Result<Trs, Error> {
    if determinant(matrix) <= 0.0 {
        return Err(invalid("mirrored matrix nodes unsupported"));
    }
    let lengths: [f64; 3] = std::array::from_fn(|c| {
        (0..3)
            .map(|r| f64::from(matrix[c][r]).powi(2))
            .sum::<f64>()
            .sqrt()
    });
    // Widen the existing f32 scale endpoints, not decimal f64 approximations.
    let scale_range = f64::from(1.0e-6_f32)..=f64::from(1.0e6_f32);
    if lengths
        .iter()
        .any(|s| !s.is_finite() || !scale_range.contains(s))
    {
        return Err(invalid("matrix node scale out of range"));
    }
    let rotation: [[f64; 3]; 3] =
        std::array::from_fn(|c| std::array::from_fn(|r| f64::from(matrix[c][r]) / lengths[c]));
    for (a, b) in [(0, 1), (0, 2), (1, 2)] {
        let dot: f64 = (0..3).map(|r| rotation[a][r] * rotation[b][r]).sum();
        if dot.abs() > ORTHOGONAL_EPSILON {
            return Err(invalid("matrix node has unsupported shear"));
        }
    }
    let mut quaternion = quaternion(rotation);
    let norm = quaternion.iter().map(|v| v * v).sum::<f64>().sqrt();
    if !norm.is_finite() || norm == 0.0 {
        return Err(invalid("matrix node rotation is invalid"));
    }
    for value in &mut quaternion {
        *value /= norm;
    }
    let rotation = canonical_rotation(quaternion);
    let rest = Trs {
        translation: [matrix[3][0], matrix[3][1], matrix[3][2]],
        rotation,
        scale: lengths.map(|s| s as f32),
    };
    // Validate the actual stored f32 TRS, not only the f64 intermediates.
    if !rest.rotation.iter().all(|v| v.is_finite())
        || !(0.99..=1.01).contains(&rest.rotation.iter().map(|v| v * v).sum::<f32>())
        || !rest
            .scale
            .iter()
            .all(|v| v.is_finite() && (1.0e-6..=1.0e6).contains(v))
    {
        return Err(invalid("matrix node TRS is out of range"));
    }
    validate_reconstruction(matrix, rest, lengths)?;
    Ok(rest)
}

fn canonical_rotation(quaternion: [f64; 4]) -> [f32; 4] {
    let mut rotation = quaternion.map(|v| v as f32);
    // Canonicalize AFTER casting: a tiny leading f64 component may round to
    // zero. The first nonzero in (w,x,y,z) is positive; all zeros are +0.
    if [3, 0, 1, 2]
        .into_iter()
        .find_map(|i| (rotation[i] != 0.0).then_some(rotation[i]))
        .is_some_and(|v| v < 0.0)
    {
        rotation = rotation.map(|v| -v);
    }
    rotation.map(|v| if v == 0.0 { 0.0 } else { v })
}

fn validate_reconstruction(
    matrix: [[f32; 4]; 4],
    rest: Trs,
    lengths: [f64; 3],
) -> Result<(), Error> {
    // Use the consumer's real f32 quaternion normalization and matrix builder.
    // Each column has its own scale-relative budget; large translation or an
    // unrelated large scale must never mask an error in a small column.
    let reconstructed = rest.matrix();
    for c in 0..3 {
        for r in 0..3 {
            if (f64::from(reconstructed[c][r]) - f64::from(matrix[c][r])).abs()
                > RECONSTRUCTION_EPSILON * lengths[c]
            {
                return Err(invalid("matrix node cannot round-trip through TRS"));
            }
        }
    }
    if reconstructed[3] != matrix[3] || (0..3).any(|c| reconstructed[c][3] != matrix[c][3]) {
        return Err(invalid("matrix node affine terms cannot round-trip"));
    }
    Ok(())
}

/// Select the largest quaternion component to avoid cancellation near 180°.
/// `m` is column-major, and the returned quaternion is (x,y,z,w).
fn quaternion(m: [[f64; 3]; 3]) -> [f64; 4] {
    let candidates = [
        1.0 + m[0][0] - m[1][1] - m[2][2],
        1.0 - m[0][0] + m[1][1] - m[2][2],
        1.0 - m[0][0] - m[1][1] + m[2][2],
        1.0 + m[0][0] + m[1][1] + m[2][2],
    ];
    let mut largest = 0;
    for i in 1..4 {
        if candidates[i] > candidates[largest] {
            largest = i;
        }
    }
    let component = candidates[largest].sqrt() * 0.5;
    let divisor = 4.0 * component;
    match largest {
        0 => [
            component,
            (m[1][0] + m[0][1]) / divisor,
            (m[2][0] + m[0][2]) / divisor,
            (m[1][2] - m[2][1]) / divisor,
        ],
        1 => [
            (m[1][0] + m[0][1]) / divisor,
            component,
            (m[2][1] + m[1][2]) / divisor,
            (m[2][0] - m[0][2]) / divisor,
        ],
        2 => [
            (m[2][0] + m[0][2]) / divisor,
            (m[2][1] + m[1][2]) / divisor,
            component,
            (m[0][1] - m[1][0]) / divisor,
        ],
        _ => [
            (m[1][2] - m[2][1]) / divisor,
            (m[2][0] - m[0][2]) / divisor,
            (m[0][1] - m[1][0]) / divisor,
            component,
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_sign_is_chosen_after_f32_underflow_and_zeros_are_positive() {
        assert_eq!(
            canonical_rotation([-1.0, -0.0, 0.0, f64::MIN_POSITIVE]).map(f32::to_bits),
            [1.0_f32.to_bits(), 0, 0, 0]
        );
        assert_eq!(
            canonical_rotation([0.0, -1.0, -0.0, -0.0]).map(f32::to_bits),
            [0, 1.0_f32.to_bits(), 0, 0]
        );
    }

    #[test]
    fn reconstruction_boundary_uses_each_source_column_not_translation_or_other_scales() {
        // Exercise the independent round-trip gate without the preceding
        // orthogonality gate: both are required, even if a sample fails earlier.
        for scale in [1.0e-6_f32, 1.0, 1.0e6] {
            let rest = Trs {
                translation: [1.0e6, -1.0e6, 0.0],
                scale: [scale, 1.0, 1.0e6],
                ..Trs::default()
            };
            let lengths = [f64::from(scale), 1.0, 1.0e6];
            let boundary = (RECONSTRUCTION_EPSILON * f64::from(scale)) as f32;
            // Stay one f32 step on each side of the exact f64 bound.
            let inside = f32::from_bits(boundary.to_bits() - 1);
            let outside = f32::from_bits(boundary.to_bits() + 1);
            for (error, accepted) in [(inside, true), (outside, false)] {
                let mut source = rest.matrix();
                source[0][1] = error;
                assert_eq!(
                    validate_reconstruction(source, rest, lengths).is_ok(),
                    accepted
                );
            }
            let mut source = rest.matrix();
            source[3][0] = f32::from_bits(source[3][0].to_bits() - 1);
            assert!(validate_reconstruction(source, rest, lengths).is_err());
            let mut source = rest.matrix();
            source[0][3] = f32::from_bits(1);
            assert!(validate_reconstruction(source, rest, lengths).is_err());
        }
    }
}
