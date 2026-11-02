//! Bounded authoring only: isotropic linear RGB gradient, not baked transport.
use orr_render::irradiance::{IrradianceGrid, IrradianceProvenance, constant_irradiance};

pub(super) struct GradientSpec {
    pub axis: usize,
    pub start: [f32; 3],
    pub end: [f32; 3],
}

pub(super) fn build_gradient(
    source: &IrradianceGrid,
    spec: GradientSpec,
) -> Result<IrradianceGrid, String> {
    source.validate()?;
    if spec.axis >= 3 {
        return Err("Gradient axis must be X, Y or Z".into());
    }
    // Admission matches existing constant authoring. Never clamp invalid endpoints.
    constant_irradiance(spec.start)?;
    constant_irradiance(spec.end)?;
    let mut grid = source.clone();
    for z in 0..grid.dimensions[2] {
        for y in 0..grid.dimensions[1] {
            for x in 0..grid.dimensions[0] {
                let node = [x, y, z];
                let step = node[spec.axis];
                let last = grid.dimensions[spec.axis] - 1;
                let color = if step == 0 {
                    spec.start
                } else if step == last {
                    spec.end
                } else {
                    let t = f64::from(step) / f64::from(last);
                    std::array::from_fn(|channel| {
                        (f64::from(spec.start[channel]) * (1.0 - t)
                            + f64::from(spec.end[channel]) * t) as f32
                    })
                };
                let index = grid.node_index(node).ok_or("Invalid gradient node")?;
                grid.coefficients[index] = constant_irradiance(color)?;
            }
        }
    }
    grid.provenance = IrradianceProvenance::Authored;
    grid.validate()?;
    Ok(grid)
}

#[cfg(test)]
#[path = "gradient/tests.rs"]
mod tests;
