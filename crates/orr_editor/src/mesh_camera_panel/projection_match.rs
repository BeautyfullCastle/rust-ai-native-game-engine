//! Pure projection matching at the camera target plane and an explicit aspect.
use crate::mesh_camera::{Document, Projection};

pub(super) fn match_projection(source: &Document, size: (u32, u32)) -> Result<Document, String> {
    // Reuse the actual consumer admission, including bounded nonzero viewport.
    source.camera(&source.orbit(), size)?;
    let aspect = f64::from(size.0) / f64::from(size.1);
    let portrait = aspect.min(1.0);
    let mut result = source.clone();
    match source.projection {
        Projection::Perspective => {
            let half_height = f64::from(source.distance)
                * (f64::from(source.fov_y_degrees).to_radians() * 0.5).tan()
                * portrait;
            result = result.with_projection(
                Projection::Orthographic {
                    half_height: half_height as f32,
                },
                source.near,
                source.far,
            )?;
        }
        Projection::Orthographic { half_height } => {
            result.fov_y_degrees = (2.0
                * (f64::from(half_height) / portrait / f64::from(source.distance)).atan())
            .to_degrees() as f32;
            result = result.with_projection(Projection::Perspective, source.near, source.far)?;
        }
    }
    result.camera(&result.orbit(), size)?;
    Ok(result)
}

#[cfg(test)]
#[path = "projection_match/tests.rs"]
mod tests;
