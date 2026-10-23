use super::{add_octahedron, build_view_model, point_to_f32, VIEW_ASSET_ID};
use orr_model::{Material, ModelSource, Primitive, StaticModel, Wrap, IDENTITY};
use orr_remote::mesh_navigation::{mission::MissionView, MeshView};

pub(super) fn build(view: &MeshView, mission: Option<&MissionView>) -> Result<StaticModel, String> {
    let base = build_view_model(&view.graph, &view.agent.path, view.agent.position)?;
    let Some(mission) = mission else {
        return Ok(base);
    };
    if mission.config.enabled != 1
        || !(1..=8).contains(&mission.config.count)
        || mission.config.graph_revision != view.graph.revision()
    {
        return Err("Mission markers require the current admitted configuration".into());
    }
    let complete_mask = (1u32 << mission.config.count) - 1;
    if mission.run.collected & !complete_mask != 0
        || mission.run.won > 1
        || (mission.run.won == 1 && mission.run.collected != complete_mask)
    {
        return Err("Mission markers reject malformed runtime progress".into());
    }
    let mut source = base.source().clone();
    let mut collectibles = primitive(&source, source.materials.len());
    for (index, point) in mission
        .config
        .points
        .iter()
        .take(mission.config.count as usize)
        .enumerate()
    {
        if mission.run.collected & (1u32 << index) != 0 {
            continue;
        }
        let mut position = point_to_f32(point.position)?;
        position[1] += 0.3;
        add_octahedron(&mut collectibles, position, 0.18);
    }
    if !collectibles.indices.is_empty() {
        source.materials.push(material([255, 175, 20, 255]));
        source.primitives.push(collectibles);
    }
    let mut exit = primitive(&source, source.materials.len());
    let mut position = point_to_f32(mission.config.exit.position)?;
    position[1] += 0.35;
    add_octahedron(&mut exit, position, 0.3);
    let color = if mission.run.won == 1 {
        [45, 165, 255, 255]
    } else if mission.run.collected == complete_mask {
        [35, 230, 115, 255]
    } else {
        [245, 55, 70, 255]
    };
    source.materials.push(material(color));
    source.primitives.push(exit);
    StaticModel::new(source).map_err(|error| error.to_string())
}

fn primitive(source: &ModelSource, material_index: usize) -> Primitive {
    Primitive {
        id: format!("{VIEW_ASSET_ID}#node={}", source.primitives.len()),
        vertices: Vec::new(),
        indices: Vec::new(),
        material: material_index as u32,
        transform: IDENTITY,
    }
}

fn material(rgba: [u8; 4]) -> Material {
    Material {
        base_color: rgba.map(|channel| f32::from(channel) / 255.0),
        image: 0,
        linear_filter: false,
        wrap_s: Wrap::Clamp,
        wrap_t: Wrap::Clamp,
    }
}

#[cfg(test)]
#[path = "mission_view/tests.rs"]
mod tests;
