use std::any::TypeId;
use std::io::{self, Write};

use bytemuck::{Pod, Zeroable};
use orr_edit::{Op, Origin};
use orr_games::mesh_navigation_game::{
    mission::{self, MeshMission, MeshMissionRun, MISSION_NAME, RUN_NAME},
    MeshNavigation, MeshScenePin, MESH_MISSION_ENABLED, PIN_NAME,
};
use orr_navigation::{mesh::StaticMeshGraph, SearchBudget};
use orr_reflect::{TypeInfo, TypeKind, Value};
use orr_sim::Game;
use serde_json::{json, Value as J};

use crate::{
    caps::Cap,
    dispatch::{decode_value, own_tx, require_edit_mode, CallCtx, ErpTarget, HostLimits, P},
    error::{RpcError, INVALID_VALUE, LIMIT_EXCEEDED, NOT_FOUND},
    json::value_to_json,
    wire::checksum_text,
};

const MAX_PARAMS_BYTES: usize = 1 << 20;

pub(super) fn call<G: Game>(
    target: &mut ErpTarget<'_, G>,
    limits: &HostLimits,
    context: &CallCtx<'_>,
    params: &P<'_>,
    origin: Origin,
) -> Result<J, RpcError> {
    require_edit_mode(target, "mesh.geometry.reproject")?;
    if !context.caps.has(Cap::SceneEdit) {
        return Err(RpcError::new(
            crate::error::PERMISSION_DENIED,
            "permission_denied",
            "method 'mesh.geometry.reproject' needs the 'scene_edit' capability",
        ));
    }
    own_tx(target, context)?;
    if target.doc.in_tx() {
        return Err(RpcError::state(
            "tx_open",
            "mesh.geometry.reproject cannot run while a transaction is open",
        ));
    }
    if TypeId::of::<G>() != TypeId::of::<MeshNavigation>()
        || limits.game.name != "MeshNavigation"
        || limits.build_id != crate::mesh_navigation::MESH_NAVIGATION_BUILD_ID
        || !MESH_MISSION_ENABLED
    {
        return Err(RpcError::state(
            "profile_mismatch",
            "mesh.geometry.reproject requires the matching MeshNavigation mission host profile",
        ));
    }

    check_params_size(params)?;
    if params.0.len() != 3
        || !params.0.contains_key("expected_checksum")
        || !params.0.contains_key("expected_graph_revision")
        || !params.0.contains_key("pin")
    {
        return Err(RpcError::params(
            "parameters must contain only expected_checksum, expected_graph_revision and pin",
        ));
    }

    let expected_checksum = parse_checksum(params.raw("expected_checksum"))?;
    let expected_graph_revision = parse_revision(params.raw("expected_graph_revision"))?;
    if target.doc.checksum() != expected_checksum {
        return Err(RpcError::state(
            "stale_checksum",
            format!(
                "document checksum changed (expected {}, current {})",
                checksum_text(expected_checksum),
                checksum_text(target.doc.checksum())
            ),
        ));
    }

    let types = target.doc.types();
    let pin_type = singleton_descriptor(types.get(PIN_NAME), PIN_NAME)?;
    let mission_type = singleton_descriptor(types.get(MISSION_NAME), MISSION_NAME)?;
    if pin_type.size() != core::mem::size_of::<MeshScenePin>()
        || mission_type.size() != core::mem::size_of::<MeshMission>()
        || singleton_descriptor(types.get(RUN_NAME), RUN_NAME)?.size()
            != core::mem::size_of::<MeshMissionRun>()
    {
        return Err(RpcError::new(
            crate::error::INTERNAL_ERROR,
            "reflection_layout",
            "MeshNavigation mission reflection layout does not match the active game profile",
        ));
    }
    let frame = target.doc.frame();
    if frame.registry().singleton_id::<MeshScenePin>().is_none()
        || frame.registry().singleton_id::<MeshMission>().is_none()
        || frame.registry().singleton_id::<MeshMissionRun>().is_none()
    {
        return Err(RpcError::new(
            NOT_FOUND,
            "wrong_registry",
            "MeshNavigation mission singleton registry is incomplete",
        ));
    }

    let old_pin = *frame.singleton::<MeshScenePin>();
    let old_mission = *frame.singleton::<MeshMission>();
    let old_run = *frame.singleton::<MeshMissionRun>();
    if old_pin.is_empty() {
        return Err(RpcError::state(
            "mesh_not_admitted",
            "mesh.geometry.reproject requires a saved, admitted MeshNavigation graph",
        ));
    }
    let old_graph = old_pin.graph().map_err(invalid_mesh_value)?;
    if old_graph.revision() != expected_graph_revision {
        return Err(RpcError::state(
            "stale_graph_revision",
            "the current mesh graph revision differs from expected_graph_revision",
        ));
    }
    let canonical_old_pin =
        MeshScenePin::new(&old_graph, old_pin.spec()).map_err(invalid_mesh_value)?;
    if canonical_old_pin != old_pin {
        return Err(invalid_mesh_value("current MeshScenePin is not canonical"));
    }
    if old_mission.disabled() {
        return Err(RpcError::state(
            "mission_disabled",
            "mesh.geometry.reproject requires an enabled MeshMission",
        ));
    }
    mission::validate_config(&old_mission, &old_graph, old_pin.max_vertical)
        .map_err(invalid_mesh_value)?;
    mission::validate_run(&old_mission, &old_run).map_err(invalid_mesh_value)?;
    if old_run != MeshMissionRun::zeroed() {
        return Err(RpcError::state(
            "mission_progressed",
            "mesh.geometry.reproject requires the authored, zero-progress mission state",
        ));
    }

    let proposed_pin: MeshScenePin =
        decode_pod(target, pin_type, params.raw("pin").expect("checked"))?;
    let new_graph = proposed_pin.graph().map_err(invalid_mesh_value)?;
    if new_graph.revision() == old_graph.revision() {
        return Err(RpcError::params(
            "pin must contain a different canonical graph revision",
        ));
    }
    let expected_start = project_position(&new_graph, old_pin.start, proposed_pin.max_vertical)?;
    let expected_goal = project_position(&new_graph, old_pin.goal, proposed_pin.max_vertical)?;
    if proposed_pin.distance_per_tick != old_pin.distance_per_tick
        || proposed_pin.start != expected_start
        || proposed_pin.goal != expected_goal
    {
        return Err(RpcError::new(
            INVALID_VALUE,
            "invalid_value",
            "new pin must preserve speed and project the old start and goal onto the new graph",
        ));
    }
    let canonical_new_pin = MeshScenePin::new(
        &new_graph,
        orr_navigation_runtime::mesh::AgentSpec {
            start: expected_start,
            goal: expected_goal,
            max_vertical: proposed_pin.max_vertical,
            distance_per_tick: old_pin.distance_per_tick,
        },
    )
    .map_err(invalid_mesh_value)?;
    if canonical_new_pin != proposed_pin {
        return Err(invalid_mesh_value(
            "proposed MeshScenePin is not the canonical pin for its graph and preserved route",
        ));
    }

    let mut proposed_mission = old_mission;
    for point in proposed_mission
        .points
        .iter_mut()
        .take(old_mission.count as usize)
    {
        point.position = project_position(&new_graph, point.position, proposed_pin.max_vertical)?;
    }
    proposed_mission.exit.position = project_position(
        &new_graph,
        old_mission.exit.position,
        proposed_pin.max_vertical,
    )?;
    proposed_mission.graph_revision = new_graph.revision();
    mission::validate_config(&proposed_mission, &new_graph, proposed_pin.max_vertical)
        .map_err(invalid_mesh_value)?;
    mission::validate_run(&proposed_mission, &old_run).map_err(invalid_mesh_value)?;
    let route_budget = SearchBudget {
        max_expansions: u32::try_from(new_graph.triangles().len())
            .map_err(|_| invalid_mesh_value("new mesh graph exceeds the route search budget"))?,
    };
    for point in proposed_mission
        .points
        .iter()
        .take(proposed_mission.count as usize)
        .chain(std::iter::once(&proposed_mission.exit))
    {
        new_graph
            .find_path(
                proposed_pin.start,
                point.position,
                proposed_pin.max_vertical,
                route_budget,
            )
            .map_err(invalid_mesh_value)?;
    }

    let pin_value = reflected_value(pin_type, &proposed_pin)?;
    let disabled_mission_value = reflected_value(mission_type, &MeshMission::zeroed())?;
    let mission_value = reflected_value(mission_type, &proposed_mission)?;
    let projected_mission = value_to_json(&mission_value);
    let applied = target.doc.apply_atomic_batch(
        "reproject mesh mission geometry",
        vec![
            Op::SetSingletonField {
                singleton: MISSION_NAME.into(),
                path: String::new(),
                value: disabled_mission_value,
            },
            Op::SetSingletonField {
                singleton: PIN_NAME.into(),
                path: String::new(),
                value: pin_value,
            },
            Op::SetSingletonField {
                singleton: MISSION_NAME.into(),
                path: String::new(),
                value: mission_value,
            },
        ],
        origin,
    )?;
    let changed = applied.iter().any(|entry| entry.changed);
    if !changed {
        return Err(RpcError::state(
            "unchanged",
            "mesh.geometry.reproject did not change the admitted document",
        ));
    }

    Ok(json!({
        "changed": true,
        "checksum": checksum_text(target.doc.checksum()),
        "graph_revision": revision_text(&new_graph.revision()),
        "mission": projected_mission,
    }))
}

fn singleton_descriptor<'a>(
    descriptor: Option<&'a TypeInfo>,
    name: &str,
) -> Result<&'a TypeInfo, RpcError> {
    descriptor
        .filter(|info| info.kind() == TypeKind::Singleton)
        .ok_or_else(|| {
            RpcError::new(
                NOT_FOUND,
                "wrong_registry",
                format!("required MeshNavigation singleton '{name}' is unavailable"),
            )
        })
}

struct ParamSizeWriter {
    bytes: usize,
    too_large: bool,
}

impl Write for ParamSizeWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let Some(total) = self.bytes.checked_add(buffer.len()) else {
            self.too_large = true;
            return Err(io::Error::other("serialized params exceed the limit"));
        };
        if total > MAX_PARAMS_BYTES {
            self.too_large = true;
            return Err(io::Error::other("serialized params exceed the limit"));
        }
        self.bytes = total;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn check_params_size(params: &P<'_>) -> Result<(), RpcError> {
    let mut writer = ParamSizeWriter {
        bytes: 0,
        too_large: false,
    };
    if let Err(error) = serde_json::to_writer(&mut writer, params.0) {
        if writer.too_large {
            return Err(RpcError::new(
                LIMIT_EXCEEDED,
                "limit_exceeded",
                "mesh.geometry.reproject parameters exceed 1 MiB",
            ));
        }
        return Err(RpcError::params(format!(
            "cannot serialize mesh.geometry.reproject parameters: {error}"
        )));
    }
    Ok(())
}

fn decode_pod<G: Game, T: Pod>(
    target: &ErpTarget<'_, G>,
    descriptor: &TypeInfo,
    raw: &J,
) -> Result<T, RpcError> {
    let reflected = decode_value(target, descriptor, "", raw)?;
    let mut bytes = descriptor.default_bytes();
    descriptor
        .set(&mut bytes, "", reflected)
        .map_err(|error| invalid_mesh_value(format!("{}: {error}", descriptor.name())))?;
    if bytes.len() != core::mem::size_of::<T>() {
        return Err(RpcError::new(
            crate::error::INTERNAL_ERROR,
            "reflection_layout",
            format!(
                "{} reflection layout does not match its runtime type",
                descriptor.name()
            ),
        ));
    }
    let value: T = bytemuck::pod_read_unaligned(&bytes);
    let canonical = value_to_json(&descriptor.read(bytemuck::bytes_of(&value)));
    if &canonical != raw {
        return Err(invalid_mesh_value(format!(
            "{} must be a complete canonical reflected object",
            descriptor.name()
        )));
    }
    Ok(value)
}

fn reflected_value<T: Pod>(descriptor: &TypeInfo, value: &T) -> Result<Value, RpcError> {
    let bytes = bytemuck::bytes_of(value);
    if bytes.len() != descriptor.size() {
        return Err(RpcError::new(
            crate::error::INTERNAL_ERROR,
            "reflection_layout",
            format!(
                "{} reflection layout does not match its runtime type",
                descriptor.name()
            ),
        ));
    }
    Ok(descriptor.read(bytes))
}

fn project_position(
    graph: &StaticMeshGraph,
    position: [orr_fp::FP; 3],
    max_vertical: orr_fp::FP,
) -> Result<[orr_fp::FP; 3], RpcError> {
    graph
        .project(position, max_vertical)
        .map(|projection| projection.position)
        .map_err(invalid_mesh_value)
}

fn parse_checksum(raw: Option<&J>) -> Result<u64, RpcError> {
    let text = raw
        .and_then(J::as_str)
        .ok_or_else(|| RpcError::params("'expected_checksum' must be a checksum string"))?;
    let digits = text
        .strip_prefix("0x")
        .filter(|digits| digits.len() == 16)
        .ok_or_else(|| {
            RpcError::params(
                "'expected_checksum' must be 0x followed by 16 lowercase hexadecimal digits",
            )
        })?;
    if !digits
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(RpcError::params(
            "'expected_checksum' must be 0x followed by 16 lowercase hexadecimal digits",
        ));
    }
    u64::from_str_radix(digits, 16).map_err(|_| RpcError::params("'expected_checksum' is invalid"))
}

fn parse_revision(raw: Option<&J>) -> Result<[u8; 32], RpcError> {
    let text = raw.and_then(J::as_str).ok_or_else(|| {
        RpcError::params("'expected_graph_revision' must be 64 lowercase hex digits")
    })?;
    if text.len() != 64
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(RpcError::params(
            "'expected_graph_revision' must be 64 lowercase hex digits",
        ));
    }
    let mut revision = [0; 32];
    for (index, byte) in revision.iter_mut().enumerate() {
        let start = index * 2;
        *byte = u8::from_str_radix(&text[start..start + 2], 16)
            .map_err(|_| RpcError::params("'expected_graph_revision' is invalid"))?;
    }
    Ok(revision)
}

fn revision_text(revision: &[u8; 32]) -> String {
    let mut text = String::with_capacity(64);
    for byte in revision {
        use core::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

fn invalid_mesh_value(error: impl ToString) -> RpcError {
    RpcError::new(INVALID_VALUE, "invalid_value", error.to_string())
}

#[cfg(test)]
#[path = "geometry_mission/tests.rs"]
mod tests;
