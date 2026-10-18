use crate::{GameHooks, HostLimits, LocalHost, ServerConfig, ViewStreamHook};
use orr_ecs::Frame;
use orr_edit::{BakeAdmission, EditError, EditorDoc};
use orr_navigation::mesh::StaticMeshGraph;
use orr_navigation_runtime::mesh::{
    decode_readback, AgentReadback, MeshRuntime, RuntimeAgent, RuntimeState,
};
use orr_reflect::{Scene, SceneIndex, TypeRegistry};
use orr_sim::Simulation;
use orr_viewstream::{FrameMeta, Schema, StreamProducer};
use std::{io::Read, path::PathBuf, sync::Arc};

pub mod authoring;

#[cfg(test)]
#[path = "mesh_play_host_tests.rs"]
mod mesh_play_host_tests;

pub use crate::yard3d::{YARD_PLAYERS, YARD_SEED, YARD_TICK_RATE};
pub use orr_games::mesh_navigation_game::{
    MeshNavigation, MeshScenePin, MeshStepStatus, MESH_SET_GOAL, PIN_NAME,
};
pub use orr_navigation_runtime::mesh::AgentSpec as RuntimeAgentSpec;
pub use orr_navigation_runtime::mesh::{decode_asset, AgentSpec};

pub const MESH_NAVIGATION_BUILD_ID: u64 = 0x4d45_5348_0000_0002;
const MAX_SCENE_YAML_BYTES: u64 = 1 << 20;

pub fn mesh_navigation_types() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    orr_games::mesh_navigation_game::register_reflect(&mut types);
    types
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MeshNavigationAdmission;

impl BakeAdmission for MeshNavigationAdmission {
    fn admit(&self, scene: &Scene, frame: &mut Frame, _: &SceneIndex) -> Result<(), EditError> {
        let fail =
            |message: String| EditError::Invalid(format!("mesh navigation admission: {message}"));
        if !scene.entities.is_empty() || frame.alive_count() != 0 {
            return Err(fail(
                "MeshNavigation rejects authored entities and admits only its derived agent".into(),
            ));
        }
        if bytemuck::bytes_of(frame.singleton::<MeshStepStatus>())
            .iter()
            .any(|byte| *byte != 0)
        {
            return Err(fail(
                "runtime failure status cannot be authored into a scene".into(),
            ));
        }

        let pin = *frame.singleton::<MeshScenePin>();
        if pin.is_empty() {
            return Ok(());
        }

        let graph = pin.graph().map_err(fail)?;
        MeshRuntime::admit(frame, &graph, pin.binding(), pin.spec())
            .map_err(|error| fail(error.to_string()))?;
        orr_games::mesh_navigation_game::validate_scene_frame(frame).map_err(fail)
    }

    fn allow_play_edits(&self) -> bool {
        false
    }
}

pub fn mesh_doc_from_yaml(yaml: &str) -> Result<EditorDoc, EditError> {
    EditorDoc::from_yaml_with_admission(
        yaml,
        mesh_navigation_types(),
        Simulation::<MeshNavigation>::build_registry(),
        YARD_SEED,
        Some(Arc::new(MeshNavigationAdmission)),
    )
}

pub fn mesh_doc_from_path(path: &std::path::Path) -> Result<EditorDoc, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.len() > MAX_SCENE_YAML_BYTES {
        return Err(format!(
            "{} must be a regular scene file of at most1 MiB",
            path.display()
        ));
    }
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    let before = file
        .metadata()
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    if !before.is_file() {
        return Err(format!("{} is not a regular scene file", path.display()));
    }
    if before.len() > MAX_SCENE_YAML_BYTES {
        return Err(format!(
            "{} exceeds the 1 MiB scene byte limit",
            path.display()
        ));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_SCENE_YAML_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if bytes.len() as u64 > MAX_SCENE_YAML_BYTES {
        return Err(format!(
            "{} exceeds the 1 MiB scene byte limit",
            path.display()
        ));
    }
    let after = file
        .metadata()
        .map_err(|error| format!("cannot recheck {}: {error}", path.display()))?;
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || bytes.len() as u64 != after.len()
    {
        return Err(format!("{} changed while being read", path.display()));
    }
    let yaml = String::from_utf8(bytes)
        .map_err(|error| format!("{} is not UTF-8: {error}", path.display()))?;
    mesh_doc_from_yaml(&yaml).map_err(|error| format!("{}: {error}", path.display()))
}

pub fn configure_mesh_navigation(limits: &mut HostLimits) {
    limits.player_count = YARD_PLAYERS;
    limits.tick_rate = YARD_TICK_RATE;
    limits.build_id = MESH_NAVIGATION_BUILD_ID;
    limits.game = GameHooks::new("MeshNavigation");
    limits.view_stream = Some(ViewStreamHook::new(MeshNavigationStream::new(
        limits.build_id,
        limits.player_count,
    )));
}

pub fn spawn_mesh_navigation_scene_host(
    path: PathBuf,
    mut config: ServerConfig,
) -> Result<LocalHost, String> {
    LocalHost::spawn_configured::<MeshNavigation>(
        move || {
            let document = mesh_doc_from_path(&path)?;
            config.limits.scene_path = Some(path);
            configure_mesh_navigation(&mut config.limits);
            Ok((document, config))
        },
        |server| {
            server.set_structured_input::<MeshNavigation>("MeshGoalInput", YARD_PLAYERS, |_, _| Vec::new());
            server.enable_managed_input();
        },
    )
}

struct MeshNavigationStream {
    yard: orr_sample::yard3d_stream::Yard3dStreamProducer,
    schema: Schema,
}

impl MeshNavigationStream {
    fn new(build_id: u64, player_count: u8) -> Self {
        let yard = orr_sample::yard3d_stream::Yard3dStreamProducer::new(build_id, player_count);
        let mut schema = yard.schema().clone();
        schema.game = "MeshNavigation".into();
        Self { yard, schema }
    }
}

impl StreamProducer for MeshNavigationStream {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn encode_frame(
        &mut self,
        current: &Frame,
        previous: Option<&Frame>,
        meta: FrameMeta,
    ) -> Vec<u8> {
        self.yard.encode_frame(current, previous, meta)
    }
}

pub struct MeshView {
    pub graph: StaticMeshGraph,
    pub agent: AgentReadback,
    pub pin: MeshScenePin,
}

pub fn mesh_from_view(frame: orr_bridge::FrameView<'_>) -> Result<MeshView, String> {
    let pin = *frame.singleton::<MeshScenePin>();
    if pin.is_empty() {
        return Err("snapshot has no admitted static mesh scene pin".into());
    }
    let status = frame.singleton::<MeshStepStatus>();
    if status.failed != 0 || status.error_len as usize > status.error.len() {
        return Err(format!(
            "MeshNavigation runtime failed: {}",
            status.message()
        ));
    }
    if bytemuck::bytes_of(status).iter().any(|byte| *byte != 0) {
        return Err("snapshot has a noncanonical successful mesh status".into());
    }

    let state = *frame.singleton::<RuntimeState>();
    if frame.alive_count() != 1
        || !state.is_active()
        || state.agent.is_none()
        || !frame.exists(state.agent)
        || frame.count::<RuntimeAgent>() != 1
    {
        return Err("snapshot has no valid admitted mesh agent".into());
    }
    let agent = frame
        .get::<RuntimeAgent>(state.agent)
        .ok_or("snapshot mesh agent is missing")?;
    let stored_bytes = frame.list(state.graph_bytes);
    if stored_bytes.is_empty() || stored_bytes.len() > orr_navigation_runtime::mesh::MAX_GRAPH_BYTES
    {
        return Err("snapshot mesh graph asset is missing or stale".into());
    }
    let graph_bytes: Vec<u8> = stored_bytes.iter().map(|byte| byte.value).collect();
    let graph = decode_asset(&graph_bytes).map_err(|error| error.to_string())?;
    let pinned_graph = pin.graph()?;
    if graph.revision() != pin.graph_revision
        || graph.revision() != state.binding.graph_revision
        || graph.to_bytes() != pinned_graph.to_bytes()
    {
        return Err("snapshot graph asset does not match the embedded scene pin".into());
    }
    let binding = pin.binding();
    let agent =
        decode_readback(&state, agent, stored_bytes, binding).map_err(|error| error.to_string())?;
    let pinned_spec = RuntimeAgentSpec {
        start: pin.start,
        goal: pin.goal,
        max_vertical: pin.max_vertical,
        distance_per_tick: pin.distance_per_tick,
    };
    if agent.spec != pinned_spec || agent.binding != binding {
        return Err("snapshot runtime agent does not match the embedded scene pin".into());
    }

    Ok(MeshView { graph, agent, pin })
}
