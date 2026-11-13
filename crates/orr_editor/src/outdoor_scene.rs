//! The sky, water and scatters of the frame, in the outdoor pass's terms. A
//! water surface or scatter names its terrain entity; the terrain's heights
//! come from its heightmap, which the terrain cache already read.

use crate::terrain_models::Heights;
use crate::viewport3d::YardFrame;
use orr_ecs::Entity;
use orr_scenery::passes::{HeightField, OutdoorScatter, OutdoorScene, OutdoorSky, OutdoorWater, ScatterKind};

fn field(frame: &YardFrame, heights: &[(Entity, Heights)], terrain: Entity) -> Option<HeightField> {
    let t = frame.terrains.iter().find(|t| t.entity == terrain)?;
    let (_, (width, depth, values)) = heights.iter().find(|(e, _)| *e == terrain)?;
    Some(HeightField { center: t.center, size: t.size, height: t.height, width: *width, depth: *depth, heights: values.clone() })
}

/// The outdoor scene of `frame`. `heights` are the loaded heights of its terrains.
pub fn scene(frame: &YardFrame, heights: &[(Entity, Heights)]) -> OutdoorScene {
    OutdoorScene {
        sky: frame.sky.map(|s| OutdoorSky {
            brightness: s.brightness,
            sun_glow: s.sun_glow,
            cloud_cover: s.cloud_cover,
            cloud_speed: s.cloud_speed,
            wind_strength: s.wind_strength,
            wind_speed: s.wind_speed,
        }),
        waters: frame
            .waters
            .iter()
            .map(|w| OutdoorWater {
                center: w.center,
                size: w.size,
                color: w.color,
                clarity: w.clarity,
                ripples: w.ripples,
                flow: w.flow,
                glint: w.glint,
                ground: w.terrain.and_then(|t| field(frame, heights, t)),
            })
            .collect(),
        // A scatter without a loaded terrain has nothing to stand on.
        scatters: frame
            .scatters
            .iter()
            .filter_map(|s| {
                Some(OutdoorScatter {
                    kind: match s.kind {
                        orr_gfx::SCATTER_BROADLEAF => ScatterKind::Broadleaf,
                        orr_gfx::SCATTER_ROCK => ScatterKind::Rock,
                        orr_gfx::SCATTER_GRASS => ScatterKind::Grass,
                        _ => ScatterKind::Conifer,
                    },
                    center: s.center,
                    area: s.area,
                    heights: s.heights,
                    max_slope: s.max_slope,
                    scale: s.scale,
                    count: s.count,
                    seed: s.seed,
                    cast_shadows: s.cast_shadows,
                    ground: field(frame, heights, s.terrain?)?,
                })
            })
            .collect(),
    }
}
