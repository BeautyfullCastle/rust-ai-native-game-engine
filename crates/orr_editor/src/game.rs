//! Closed, compiled game adapters. Gameplay ownership lives in the editor ERP state machine.
use orr_bridge::{Bridge, BridgeEvent, FrameView, ViewUpdate};
use orr_reflect::TypeRegistry;
use orr_remote::{RemoteBridge, RemoteConfig, RemoteViewDelivery, RpcError, Transport};
use orr_sample::arena_game::Arena;
use orr_sample::physics_game::PhysGame;
use std::path::PathBuf;
pub use orr_sample::editor_view::Drawable;

/// Games with a compiled frame decoder, reflection and viewport mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorGame {
    PhysGame,
    Arena,
    Yard3D,
}

impl EditorGame {
    /// Missing or unknown identities never select a fallback decoder.
    pub fn from_local_name(name: &str) -> Result<Self, String> {
        match name {
            "physics" => Ok(Self::PhysGame),
            "arena" => Ok(Self::Arena),
            "yard3d" => Ok(Self::Yard3D),
            _ => Err(format!("unsupported local game '{name}'; expected physics, arena or yard3d")),
        }
    }

    /// Missing or unknown identities never select a fallback decoder.
    pub fn from_name(name: &str) -> Result<Self, String> {
        match name {
            "PhysGame" => Ok(Self::PhysGame),
            "Arena" => Ok(Self::Arena),
            "Yard3D" => Ok(Self::Yard3D),
            _ => Err(format!("unsupported editor game '{name}'; expected explicit PhysGame, Arena or Yard3D")),
        }
    }

    pub fn name(self) -> &'static str {
        match self { Self::PhysGame => "PhysGame", Self::Arena => "Arena", Self::Yard3D => "Yard3D" }
    }

    pub fn types(self) -> TypeRegistry {
        let mut types = TypeRegistry::new();
        match self {
            Self::PhysGame => orr_sample::physics_game::register_reflect(&mut types),
            Self::Arena => orr_sample::arena_game::register_reflect(&mut types),
            Self::Yard3D => orr_sample::yard3d_game::register_reflect(&mut types),
        }
        types
    }

    pub fn drawables(self, frame: FrameView<'_>) -> Vec<Drawable> {
        match self {
            Self::PhysGame => orr_sample::physics_view::body_views(frame),
            Self::Arena => orr_sample::arena_view::editor_drawables(frame),
            Self::Yard3D => Vec::new(),
        }
    }

    pub fn position_component(self) -> &'static str {
        match self { Self::PhysGame => "orr_physics::Body", Self::Arena => "Position", Self::Yard3D => "orr_physics3d::Body" }
    }

    /// Default scene for a locally started game, preferring the current
    /// working tree and falling back to the scene shipped with the editor.
    pub fn default_scene_path(self) -> PathBuf {
        let filename = match self {
            Self::PhysGame => "physics_demo.scene.yaml",
            Self::Arena => "arena_blank.scene.yaml",
            Self::Yard3D => "yard3d_authoring.scene.yaml",
        };
        let local = PathBuf::from("scenes").join(filename);
        if local.exists() {
            return local;
        }
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenes/")).join(filename)
    }
}

/// Concrete decoders stay here; the rest of the editor consumes common snapshots.
pub enum EditorStream {
    Phys(RemoteBridge<PhysGame>),
    Arena(RemoteBridge<Arena>),
    Yard3D(RemoteBridge<orr_sample::yard3d_game::Yard3D>),
}

impl EditorStream {
    pub fn connect(game: EditorGame, cfg: RemoteConfig) -> Result<Self, String> {
        match game {
            EditorGame::PhysGame => RemoteBridge::connect(cfg).map(Self::Phys),
            EditorGame::Arena => RemoteBridge::connect(cfg).map(Self::Arena),
            EditorGame::Yard3D => RemoteBridge::connect(cfg).map(Self::Yard3D),
        }
    }

    pub fn connect_transport(game: EditorGame, transport: Box<dyn Transport>, cfg: RemoteConfig) -> Result<Self, String> {
        match game {
            EditorGame::PhysGame => RemoteBridge::connect_transport(transport, cfg).map(Self::Phys),
            EditorGame::Arena => RemoteBridge::connect_transport(transport, cfg).map(Self::Arena),
            EditorGame::Yard3D => RemoteBridge::connect_transport(transport, cfg).map(Self::Yard3D),
        }
    }

    pub fn poll_view(&mut self) -> ViewUpdate<()> {
        match self { Self::Phys(s) => normalize(s.poll_view()), Self::Arena(s) => normalize(s.poll_view()), Self::Yard3D(s) => normalize(s.poll_view()) }
    }

    #[cfg(test)]
    pub fn snapshot(&self) -> Option<orr_bridge::Snapshot> {
        match self { Self::Phys(s) => s.snapshot(), Self::Arena(s) => s.snapshot(), Self::Yard3D(s) => s.snapshot() }
    }

    #[cfg(test)]
    pub fn request(&self, method: &str, params: serde_json::Value) -> Result<(), orr_bridge::BridgeError> {
        match self { Self::Phys(s) => s.request(method, params), Self::Arena(s) => s.request(method, params), Self::Yard3D(s) => s.request(method, params) }
    }

    pub fn take_errors(&self) -> Vec<RpcError> {
        match self { Self::Phys(s) => s.take_errors(), Self::Arena(s) => s.take_errors(), Self::Yard3D(s) => s.take_errors() }
    }

    pub fn is_alive(&self) -> bool {
        match self { Self::Phys(s) => s.is_alive(), Self::Arena(s) => s.is_alive(), Self::Yard3D(s) => s.is_alive() }
    }

    pub fn view_delivery(&self) -> RemoteViewDelivery {
        match self { Self::Phys(s) => s.view_delivery(), Self::Arena(s) => s.view_delivery(), Self::Yard3D(s) => s.view_delivery() }
    }
}

fn normalize<E>(update: ViewUpdate<E>) -> ViewUpdate<()> {
    ViewUpdate {
        snapshot: update.snapshot,
        resync: update.resync,
        // Editor owns no gameplay effects; still drain every game's events.
        events: update.events.into_iter().filter_map(|event| match event {
            BridgeEvent::Lifecycle(life) => Some(BridgeEvent::Lifecycle(life)),
            _ => None,
        }).collect(),
    }
}
