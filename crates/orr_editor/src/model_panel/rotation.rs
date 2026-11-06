//! Model-local draft rotations; no scene or collision mutation.
use crate::model_bindings::LocalTransform;
use orr_view::Quat;

pub(super) fn quarter_turn(
    rotation: [f32; 4],
    axis: usize,
    direction: i8,
) -> Result<[f32; 4], String> {
    if axis > 2 || !matches!(direction, -1 | 1) {
        return Err("Quarter turn requires axis 0..2 and direction -1 or 1".into());
    }
    LocalTransform {
        rotation,
        ..Default::default()
    }
    .validate()?;
    let mut turn = [0.0; 4];
    turn[axis] = std::f32::consts::FRAC_1_SQRT_2 * f32::from(direction);
    turn[3] = std::f32::consts::FRAC_1_SQRT_2;
    let q = (Quat::new(rotation[0], rotation[1], rotation[2], rotation[3])
        * Quat::new(turn[0], turn[1], turn[2], turn[3]))
    .normalize();
    let next = q.to_array();
    LocalTransform {
        rotation: next,
        ..Default::default()
    }
    .validate()?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn vector(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
        Quat::new(q[0], q[1], q[2], q[3])
            .rotate(orr_view::Vec3::new(v[0], v[1], v[2]))
            .to_array()
    }
    fn near(actual: [f32; 3], expected: [f32; 3]) {
        for axis in 0..3 {
            assert!(
                (actual[axis] - expected[axis]).abs() < 1e-5,
                "{actual:?} != {expected:?}"
            );
        }
    }
    #[test]
    fn positive_and_negative_axes_follow_right_hand_rotation() {
        let identity = [0.0, 0.0, 0.0, 1.0];
        near(
            vector(quarter_turn(identity, 0, 1).unwrap(), [0.0, 1.0, 0.0]),
            [0.0, 0.0, 1.0],
        );
        near(
            vector(quarter_turn(identity, 1, 1).unwrap(), [0.0, 0.0, 1.0]),
            [1.0, 0.0, 0.0],
        );
        near(
            vector(quarter_turn(identity, 2, -1).unwrap(), [1.0, 0.0, 0.0]),
            [0.0, -1.0, 0.0],
        );
    }
    #[test]
    fn composition_uses_current_local_axes_and_four_turns_restore_orientation() {
        let y = quarter_turn([0.0, 0.0, 0.0, 1.0], 1, 1).unwrap();
        let local_x = quarter_turn(y, 0, 1).unwrap();
        near(vector(local_x, [0.0, 1.0, 0.0]), [1.0, 0.0, 0.0]);
        let mut q = y;
        for _ in 0..4 {
            q = quarter_turn(q, 2, 1).unwrap();
        }
        near(vector(q, [0.0, 0.0, 1.0]), [1.0, 0.0, 0.0]);
        for _ in 0..1000 {
            q = quarter_turn(q, 0, -1).unwrap();
        }
        LocalTransform {
            rotation: q,
            ..Default::default()
        }
        .validate()
        .unwrap();
    }
    #[test]
    fn invalid_controls_and_bad_quaternions_are_rejected() {
        for (axis, direction) in [(3, 1), (usize::MAX, 1), (0, 0), (0, 2), (0, -2)] {
            assert!(quarter_turn([0.0, 0.0, 0.0, 1.0], axis, direction).is_err());
        }
        for q in [
            [0.0; 4],
            [f32::NAN, 0.0, 0.0, 1.0],
            [f32::INFINITY, 0.0, 0.0, 1.0],
            [1.0; 4],
        ] {
            assert!(quarter_turn(q, 0, 1).is_err());
        }
    }
}
