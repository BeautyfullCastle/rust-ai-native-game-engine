//! Match authored target-plane framing at an explicit viewport aspect.
//! This pure operation changes only projection; transient navigation is not captured.
#![allow(clippy::float_arithmetic)]
use orr_sample::room_camera::{Document, Projection};

pub(super) fn match_projection(source: &Document, size: (u32, u32)) -> Result<Document, String> {
    // The actual Room consumer owns all document, viewport and matrix admission.
    source.camera(&source.orbit(), size)?;
    let aspect = f64::from(size.0) / f64::from(size.1);
    let portrait = aspect.min(1.0);
    let mut result = source.clone();
    result.projection = match source.projection {
        Projection::Perspective { fov_y_degrees } => {
            let half_height = f64::from(source.distance)
                * (f64::from(fov_y_degrees).to_radians() * 0.5).tan()
                * portrait;
            Projection::Orthographic {
                half_height: half_height as f32,
            }
        }
        Projection::Orthographic { half_height } => {
            let fov_y_degrees = (2.0
                * (f64::from(half_height) / portrait / f64::from(source.distance)).atan())
            .to_degrees();
            Projection::Perspective {
                fov_y_degrees: fov_y_degrees as f32,
            }
        }
    };
    // Refuse unrepresentable/out-of-policy matches rather than silently clamping.
    result.camera(&result.orbit(), size)?;
    Ok(result)
}

#[cfg(test)]
#[path = "projection_match_tests.rs"]
mod tests;
