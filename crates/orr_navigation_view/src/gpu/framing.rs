//! Presentation-only fit for the identity-transform navigation display model.
use orr_model::{StaticModel, IDENTITY};
use orr_render::{Camera3D, Projection};

pub(crate) fn fitted_top_camera(model: &StaticModel, size: (u32, u32)) -> crate::Result<Camera3D> {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for primitive in &model.source().primitives {
        if primitive.transform != IDENTITY {
            return Err("navigation capture requires identity display transforms".into());
        }
        for vertex in &primitive.vertices {
            for axis in 0..3 {
                let value = vertex.position[axis];
                if !value.is_finite() {
                    return Err("navigation capture bounds must be finite".into());
                }
                min[axis] = min[axis].min(value);
                max[axis] = max[axis].max(value);
            }
        }
    }
    fit(min, max, size)
}

fn fit(min: [f32; 3], max: [f32; 3], size: (u32, u32)) -> crate::Result<Camera3D> {
    if size.0 == 0 || size.1 == 0 || size.0 > 8192 || size.1 > 8192 {
        return Err("capture size must be 1..8192 per axis".into());
    }
    if (0..3).any(|i| !min[i].is_finite() || !max[i].is_finite() || max[i] < min[i])
        || min[0] == max[0]
        || min[2] == max[2]
    {
        return Err("navigation capture needs finite nondegenerate XZ bounds".into());
    }
    let lo = min.map(f64::from);
    let hi = max.map(f64::from);
    let center = std::array::from_fn::<_, 3, _>(|i| (lo[i] + hi[i]) * 0.5);
    let aspect = f64::from(size.0) / f64::from(size.1);
    let half_height = ((hi[2] - lo[2]) * 0.5).max((hi[0] - lo[0]) * 0.5 / aspect) * 1.25;
    let depth_margin = ((hi[1] - lo[1]) * 0.125).max(1.0);
    let camera = Camera3D {
        eye: [
            center[0] as f32,
            (hi[1] + depth_margin) as f32,
            center[2] as f32,
        ],
        target: center.map(|v| v as f32),
        up: [0.0, 0.0, -1.0],
        projection: Projection::Orthographic {
            half_height: half_height as f32,
            near: (depth_margin * 0.5) as f32,
            far: (hi[1] - lo[1] + depth_margin * 1.5) as f32,
        },
    };
    // Validate the rounded camera actually used by the renderer. Never clamp a
    // poorly represented camera or feed presentation rounding into simulation.
    if !camera
        .eye
        .iter()
        .chain(&camera.target)
        .all(|v| v.is_finite())
        || camera.eye[1] <= max[1]
        || camera.eye[1] <= camera.target[1]
    {
        return Err("navigation capture camera is not representable".into());
    }
    let matrix = camera.view_proj(size.0 as f32 / size.1 as f32);
    if !matrix.0.iter().flatten().all(|v| v.is_finite()) {
        return Err("navigation capture matrix is not finite".into());
    }
    for x in [min[0], max[0]] {
        for y in [min[1], max[1]] {
            for z in [min[2], max[2]] {
                let p = matrix.transform_point4([x, y, z]);
                if !p.iter().all(|v| v.is_finite())
                    || p[3] != 1.0
                    || p[0].abs() >= 0.95
                    || p[1].abs() >= 0.95
                    || p[2] <= 0.0
                    || p[2] >= 1.0
                {
                    return Err("navigation capture bounds lose framing margin or depth".into());
                }
            }
        }
    }
    Ok(camera)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fits_translated_large_high_and_portrait_bounds() {
        for (min, max) in [
            ([-4., 0., -4.], [4., 0.7, 4.]),
            ([128., 1024., -96.], [192., 2048., -32.]),
            ([-1000., -900., -2000.], [1000., 900., 2000.]),
        ] {
            for size in [(1000, 750), (300, 900)] {
                let c = fit(min, max, size).unwrap();
                let m = c.view_proj(size.0 as f32 / size.1 as f32);
                for x in [min[0], max[0]] {
                    for y in [min[1], max[1]] {
                        for z in [min[2], max[2]] {
                            let p = m.transform_point4([x, y, z]);
                            assert!(
                                p[0].abs() < 0.95 && p[1].abs() < 0.95 && p[2] > 0. && p[2] < 1.
                            );
                        }
                    }
                }
            }
        }
        assert!(matches!(
            fit([-4., 0., -4.], [4., 0.7, 4.], (1000, 750))
                .unwrap()
                .projection,
            Projection::Orthographic {
                half_height: 5.0,
                ..
            }
        ));
    }
    #[test]
    fn actual_display_model_is_immutable_and_nonidentity_is_rejected() {
        let terrain = crate::fixture();
        let model = crate::to_static_model(
            &terrain,
            None,
            [orr_fp::FP::ZERO; 3],
            crate::PathDisplay::Current,
        )
        .unwrap();
        let before = model.source().clone();
        fitted_top_camera(&model, (1000, 750)).unwrap();
        assert_eq!(model.source(), &before);
        let mut changed = before;
        changed.primitives[0].transform[3][0] = 1.0;
        let transformed = StaticModel::new(changed).unwrap();
        assert!(fitted_top_camera(&transformed, (1000, 750)).is_err());
    }
    #[test]
    fn rejects_invalid_degenerate_and_unrepresentable_bounds() {
        for (min, max) in [
            ([f32::NAN, 0., 0.], [1., 1., 1.]),
            ([0., 0., 0.], [f32::INFINITY, 1., 1.]),
            ([1., 0., 0.], [0., 1., 1.]),
            ([0., 0., 0.], [0., 1., 1.]),
            ([0., f32::MAX, 0.], [1., f32::MAX, 1.]),
            ([-f32::MAX, 0., -f32::MAX], [f32::MAX, 1., f32::MAX]),
        ] {
            assert!(fit(min, max, (1000, 750)).is_err());
        }
        for size in [(0, 1), (1, 0), (8193, 1), (1, 8193)] {
            assert!(fit([0.; 3], [1.; 3], size).is_err());
        }
    }
}
