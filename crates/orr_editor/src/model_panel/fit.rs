//! Presentation-only fitting; never changes a collider or physics frame.
use super::{LoadedAsset, LocalTransform, ModelKind};

pub(super) fn loaded(asset: &LoadedAsset, half: [f64; 3]) -> Result<LocalTransform, String> {
    match asset.kind() {
        ModelKind::Static => {
            let model = asset
                .static_model()
                .ok_or("Fitting requires a static model")?;
            points(
                model.source().primitives.iter().flat_map(|primitive| {
                    primitive.vertices.iter().map(|vertex| {
                        std::array::from_fn(|axis| {
                            (0..3)
                                .map(|column| {
                                    f64::from(primitive.transform[column][axis])
                                        * f64::from(vertex.position[column])
                                })
                                .sum::<f64>()
                                + f64::from(primitive.transform[3][axis])
                        })
                    })
                }),
                half,
            )
        }
        ModelKind::Animated => {
            #[cfg(feature = "animated-models")]
            {
                let model = asset
                    .animated_model()
                    .ok_or("Fitting requires an animated model")?;
                let pose = model.rest_pose().map_err(|error| error.to_string())?;
                let bounds = model.bounds(&pose).map_err(|error| error.to_string())?;
                points([bounds.min.map(f64::from), bounds.max.map(f64::from)], half)
            }
            #[cfg(not(feature = "animated-models"))]
            Err("Animated fitting requires animated-models support".into())
        }
    }
}

fn points(
    points: impl IntoIterator<Item = [f64; 3]>,
    half: [f64; 3],
) -> Result<LocalTransform, String> {
    if half.iter().any(|value| !value.is_finite() || *value <= 0.0) {
        return Err("Fitting requires positive finite box half extents".into());
    }
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    let mut count = 0;
    for point in points {
        count += 1;
        if count > orr_model::MAX_VERTICES
            || point.iter().any(|v| !v.is_finite() || v.abs() > 1.0e9)
        {
            return Err("Fitting requires bounded finite model geometry".into());
        }
        for axis in 0..3 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    if count == 0 {
        return Err("Model has no fitting geometry".into());
    }
    let mut scale = f64::INFINITY;
    for axis in 0..3 {
        let width = max[axis] - min[axis];
        if width > 0.0 {
            scale = scale.min(2.0 * half[axis] / width);
        }
    }
    // Leave a small margin and reject point geometry rather than guessing units.
    scale *= 0.99;
    if !scale.is_finite() {
        return Err("Model needs a nonzero extent for fitting".into());
    }
    let result = LocalTransform {
        translation: std::array::from_fn(|axis| (-0.5 * (min[axis] + max[axis]) * scale) as f32),
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [scale as f32; 3],
    };
    result.validate()?;
    // Reject cancellation after conversion to the stored f32 authoring values.
    // A valid LocalTransform can still lose a small box's entire fitting margin.
    for axis in 0..3 {
        let low = min[axis] as f32 * result.scale[axis] + result.translation[axis];
        let high = max[axis] as f32 * result.scale[axis] + result.translation[axis];
        let rounding_budget = 4.0
            * f64::from(f32::EPSILON)
            * (min[axis].abs().max(max[axis].abs()) * f64::from(result.scale[axis])
                + f64::from(result.translation[axis]).abs());
        if !low.is_finite()
            || !high.is_finite()
            || f64::from(low.abs().max(high.abs())) + rounding_budget > half[axis]
        {
            return Err("Model origin is too far from this box size for precise fitting".into());
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn off_center_asymmetric_bounds_fit_uniformly_without_stretching() {
        let transform = points([[10.0, -2.0, 4.0], [14.0, 0.0, 6.0]], [1.0, 2.0, 3.0]).unwrap();
        assert_eq!(transform.scale, [0.495; 3]);
        assert_eq!(transform.rotation, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(transform.translation, [-5.94, 0.495, -2.475]);
    }
    #[test]
    fn flat_geometry_fits_its_nonzero_axes() {
        let transform = points([[-1.0, 8.0, -2.0], [1.0, 8.0, 2.0]], [1.0, 1.0, 1.0]).unwrap();
        assert_eq!(transform.scale, [0.495; 3]);
        assert_eq!(transform.translation, [0.0, -3.96, 0.0]);
    }
    #[test]
    fn empty_point_nonfinite_and_extreme_geometry_are_rejected() {
        assert!(points([], [1.0; 3]).is_err());
        assert!(points([[0.0; 3]], [1.0; 3]).is_err());
        for bad in [f64::NAN, f64::INFINITY, 1.0e10] {
            assert!(points([[bad, 0.0, 0.0], [1.0; 3]], [1.0; 3]).is_err());
        }
    }
    #[test]
    fn distant_narrow_model_rejects_float_cancellation() {
        assert!(points(
            [[100000.0, 0.0, 0.0], [100000.125, 0.125, 0.125]],
            [0.03; 3]
        )
        .is_err());
    }
    #[test]
    fn invalid_boxes_and_out_of_contract_scale_are_rejected() {
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(points([[0.0; 3], [1.0; 3]], [bad, 1.0, 1.0]).is_err());
        }
        assert!(points([[0.0; 3], [1.0e6; 3]], [1.0; 3]).is_err());
        assert!(points([[0.0; 3], [1.0e-6; 3]], [1.0; 3]).is_err());
    }
}
