use crate::{editor::Editor, game::EditorGame};
use orr_bridge::FrameView;
use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use orr_remote::mesh_navigation::{
    mesh_doc_from_yaml, mesh_from_view, mesh_navigation_types, mission, AgentSpec, MeshMission,
    MeshScenePin, MissionPoint, MISSION_NAME, PIN_NAME,
};
use std::{io::Write, path::Path};

const MAX_SCENE_BYTES: usize = 1024 * 1024;
const MAX_SPEED_RAW: i64 = FP::SCALE / 2;
const MISSION_RADIUS: FP = fp!(0.5);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Settings {
    pub width: u32,
    pub depth: u32,
    pub collectible_count: u32,
    pub distance_per_tick: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            width: 8,
            depth: 6,
            collectible_count: 2,
            distance_per_tick: "0.125".into(),
        }
    }
}

pub(crate) fn prepare(path: &Path, settings: &Settings) -> Result<Editor, String> {
    let path = super::checked_scene_path(path, false)?;
    super::refuse_existing_sidecar(&path)?;
    let yaml = scene_yaml(settings)?;
    let parent = path
        .parent()
        .ok_or("mesh game scene parent directory is missing")?;
    let mut staged = tempfile::Builder::new()
        .prefix(".orr-mesh-game-")
        .tempfile_in(parent)
        .map_err(|error| format!("create mesh game staging file: {error}"))?;
    staged
        .write_all(yaml.as_bytes())
        .and_then(|()| staged.as_file().sync_all())
        .map_err(|error| format!("write mesh game staging file: {error}"))?;
    let staged_bytes = super::read_scene(staged.path())?;
    if staged_bytes != yaml.as_bytes() {
        return Err("staged mesh game scene changed before admission".into());
    }
    let staged_text = std::str::from_utf8(&staged_bytes)
        .map_err(|error| format!("staged mesh game scene is not UTF-8: {error}"))?;
    let staged_document = mesh_doc_from_yaml(staged_text)
        .map_err(|error| format!("staged mesh game admission failed: {error}"))?;
    let staged_frame = staged_document.frame();
    let staged_view = FrameView::of(staged_frame);
    let mesh_view = mesh_from_view(staged_view)?;
    let mission_view =
        mission::from_view(staged_view)?.ok_or("staged mesh game has no enabled mission")?;
    mission::validate_config(
        &mission_view.config,
        &mesh_view.graph,
        mesh_view.pin.max_vertical,
    )?;

    if super::checked_scene_path(&path, false)? != path {
        return Err("mesh game scene destination changed before publication".into());
    }
    super::refuse_existing_sidecar(&path)?;
    staged
        .persist_noclobber(&path)
        .map_err(|error| format!("publish new mesh game scene without replacement: {error}"))?;
    if super::read_scene(&path)? != yaml.as_bytes() {
        return Err("published mesh game scene differs from its validated bytes".into());
    }
    let editor = super::prepare(&path, super::Intent::Open)?;
    if editor.game() != EditorGame::MeshNavigation {
        return Err("new mesh game opened in an unexpected game".into());
    }
    Ok(editor)
}

fn scene_yaml(settings: &Settings) -> Result<String, String> {
    let (_, pin, config) = configured_scene(settings)?;
    let types = mesh_navigation_types();
    let pin_descriptor = types
        .get(PIN_NAME)
        .ok_or("MeshScenePin reflection is missing")?;
    let mission_descriptor = types
        .get(MISSION_NAME)
        .ok_or("MeshMission reflection is missing")?;
    let pin_value = orr_remote::json::value_to_json(
        &pin_descriptor.read(orr_reflect::bytemuck::bytes_of(&pin)),
    );
    let mission_value = orr_remote::json::value_to_json(
        &mission_descriptor.read(orr_reflect::bytemuck::bytes_of(&config)),
    );
    let mut singletons = serde_json::Map::new();
    singletons.insert(PIN_NAME.into(), pin_value);
    singletons.insert(MISSION_NAME.into(), mission_value);
    let json_yaml = serde_json::json!({
        "schema": "orr.scene/1",
        "singletons": singletons,
        "entities": {},
    })
    .to_string();
    let mut generated = mesh_doc_from_yaml(&json_yaml)
        .map_err(|error| format!("generated mesh game admission failed: {error}"))?;
    let source_checksum = generated.frame().checksum();
    let yaml = generated.save_yaml();
    if yaml.len() > MAX_SCENE_BYTES {
        return Err("generated mesh game scene exceeds 1 MiB".into());
    }
    let reopened = mesh_doc_from_yaml(&yaml)
        .map_err(|error| format!("generated mesh game admission failed: {error}"))?;
    if reopened.frame().checksum() != source_checksum {
        return Err("generated mesh game does not restore its admitted Frame".into());
    }
    let reopened_view = FrameView::of(reopened.frame());
    let mesh_view = mesh_from_view(reopened_view)?;
    let mission_view = mission::from_view(reopened_view)?
        .ok_or("generated mesh game mission did not survive serialization")?;
    if mission_view.config != config
        || mission_view.run != Default::default()
        || mesh_view.graph.revision() != config.graph_revision
        || mesh_view.pin.spec() != pin.spec()
    {
        return Err("generated mesh game changed during canonical serialization".into());
    }
    Ok(yaml)
}

pub(crate) fn configured_scene(
    settings: &Settings,
) -> Result<(StaticMeshGraph, MeshScenePin, MeshMission), String> {
    if !(6..=64).contains(&settings.width) || !(4..=64).contains(&settings.depth) {
        return Err("mesh game width must be 6..=64 and depth must be 4..=64".into());
    }
    if !(1..=8).contains(&settings.collectible_count) {
        return Err("mesh game collectible count must be 1..=8".into());
    }
    if settings.distance_per_tick.len() > 48 {
        return Err("mesh game speed must be at most 48 bytes".into());
    }
    let distance_per_tick = orr_reflect::decimal::parse_fp(settings.distance_per_tick.trim())
        .map_err(|error| format!("invalid mesh game speed: {error:?}"))?;
    if !(1..=MAX_SPEED_RAW).contains(&distance_per_tick.raw()) {
        return Err("mesh game speed must be positive and at most 0.5 units per tick".into());
    }
    let width_raw = i64::from(settings.width)
        .checked_mul(FP::SCALE)
        .ok_or("mesh game width overflow")?;
    let depth_raw = i64::from(settings.depth)
        .checked_mul(FP::SCALE)
        .ok_or("mesh game depth overflow")?;
    let width = FP::from_raw(width_raw);
    let depth = FP::from_raw(depth_raw);
    let graph = StaticMeshGraph::bake(
        &[
            [FP::ZERO, FP::ZERO, FP::ZERO],
            [width, FP::ZERO, FP::ZERO],
            [FP::ZERO, FP::ZERO, depth],
            [width, FP::ZERO, depth],
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .map_err(|error| format!("build walkable mesh game floor: {error}"))?;
    let start = [FP::ONE, FP::ZERO, FP::from_raw(depth_raw / 2)];
    let goal = [FP::from_raw(width_raw - FP::SCALE), FP::ZERO, start[2]];
    let spec = AgentSpec {
        start,
        goal,
        max_vertical: FP::ZERO,
        distance_per_tick,
    };
    let mut config = MeshMission {
        enabled: 1,
        count: settings.collectible_count,
        graph_revision: graph.revision(),
        ..Default::default()
    };
    let count = i64::from(settings.collectible_count);
    let span = width_raw - 2 * FP::SCALE;
    for index in 0..settings.collectible_count as usize {
        let numerator = i64::try_from(index + 1).map_err(|_| "mission point index overflow")?;
        let x_offset = span
            .checked_mul(numerator)
            .ok_or("mission point coordinate overflow")?
            / (count + 1);
        config.points[index] = MissionPoint {
            position: [FP::from_raw(FP::SCALE + x_offset), FP::ZERO, start[2]],
            radius: MISSION_RADIUS,
        };
    }
    config.exit = MissionPoint {
        position: goal,
        radius: MISSION_RADIUS,
    };
    mission::validate_config(&config, &graph, spec.max_vertical)?;
    let pin = MeshScenePin::new(&graph, spec)?;
    Ok((graph, pin, config))
}

#[cfg(test)]
#[path = "mission/tests.rs"]
mod tests;
