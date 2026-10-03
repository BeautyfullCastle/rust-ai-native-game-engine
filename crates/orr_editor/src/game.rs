//! Closed, compiled game adapters. Gameplay ownership lives in the editor ERP state machine.
use orr_bridge::{Bridge, BridgeEvent, FrameView, ViewUpdate};
use orr_reflect::TypeRegistry;
use orr_remote::{RemoteBridge, RemoteConfig, RemoteViewDelivery, RpcError, Transport};
use orr_sample::arena_game::Arena;
use orr_sample::physics_game::PhysGame;
pub use orr_sample::editor_view::Drawable;

/// Games with a compiled frame decoder, reflection and viewport mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorGame {
    PhysGame,
    Arena,
}

impl EditorGame {
    /// Missing or unknown identities never select a fallback decoder.
    pub fn from_name(name: &str) -> Result<Self, String> {
        match name {
            "PhysGame" => Ok(Self::PhysGame),
            "Arena" => Ok(Self::Arena),
            _ => Err(format!("unsupported editor game '{name}'; expected explicit PhysGame or Arena")),
        }
    }

    pub fn name(self) -> &'static str {
        match self { Self::PhysGame => "PhysGame", Self::Arena => "Arena" }
    }

    pub fn types(self) -> TypeRegistry {
        let mut types = TypeRegistry::new();
        match self {
            Self::PhysGame => orr_sample::physics_game::register_reflect(&mut types),
            Self::Arena => orr_sample::arena_game::register_reflect(&mut types),
        }
        types
    }

    pub fn drawables(self, frame: FrameView<'_>) -> Vec<Drawable> {
        match self {
            Self::PhysGame => orr_sample::physics_view::body_views(frame),
            Self::Arena => orr_sample::arena_view::editor_drawables(frame),
        }
    }

    pub fn position_component(self) -> &'static str {
        match self { Self::PhysGame => "orr_physics::Body", Self::Arena => "Position" }
    }
}

/// Concrete decoders stay here; the rest of the editor consumes common snapshots.
pub enum EditorStream {
    Phys(RemoteBridge<PhysGame>),
    Arena(RemoteBridge<Arena>),
}

impl EditorStream {
    pub fn connect(game: EditorGame, cfg: RemoteConfig) -> Result<Self, String> {
        match game {
            EditorGame::PhysGame => RemoteBridge::connect(cfg).map(Self::Phys),
            EditorGame::Arena => RemoteBridge::connect(cfg).map(Self::Arena),
        }
    }

    pub fn connect_transport(game: EditorGame, transport: Box<dyn Transport>, cfg: RemoteConfig) -> Result<Self, String> {
        match game {
            EditorGame::PhysGame => RemoteBridge::connect_transport(transport, cfg).map(Self::Phys),
            EditorGame::Arena => RemoteBridge::connect_transport(transport, cfg).map(Self::Arena),
        }
    }

    pub fn poll_view(&mut self) -> ViewUpdate<()> {
        match self { Self::Phys(s) => normalize(s.poll_view()), Self::Arena(s) => normalize(s.poll_view()) }
    }

    #[cfg(test)]
    pub fn snapshot(&self) -> Option<orr_bridge::Snapshot> {
        match self { Self::Phys(s) => s.snapshot(), Self::Arena(s) => s.snapshot() }
    }

    #[cfg(test)]
    pub fn request(&self, method: &str, params: serde_json::Value) -> Result<(), orr_bridge::BridgeError> {
        match self { Self::Phys(s) => s.request(method, params), Self::Arena(s) => s.request(method, params) }
    }

    pub fn take_errors(&self) -> Vec<RpcError> {
        match self { Self::Phys(s) => s.take_errors(), Self::Arena(s) => s.take_errors() }
    }

    pub fn is_alive(&self) -> bool {
        match self { Self::Phys(s) => s.is_alive(), Self::Arena(s) => s.is_alive() }
    }

    pub fn view_delivery(&self) -> RemoteViewDelivery {
        match self { Self::Phys(s) => s.view_delivery(), Self::Arena(s) => s.view_delivery() }
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
