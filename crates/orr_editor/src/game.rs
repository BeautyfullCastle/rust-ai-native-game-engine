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
    #[cfg(feature = "collect-dodge")]
    CollectDodge,
    Yard3D,
    #[cfg(feature = "terrain-physics")]
    TerrainYard3D,
    #[cfg(feature = "navigation")]
    NavigationYard3D,
}

impl EditorGame {
    /// Missing or unknown identities never select a fallback decoder.
    pub fn from_local_name(name: &str) -> Result<Self, String> {
        match name {
            "physics" => Ok(Self::PhysGame),
            "arena" => Ok(Self::Arena),
            #[cfg(feature = "collect-dodge")]
            "collect-dodge-v1" => Ok(Self::CollectDodge),
            "yard3d" => Ok(Self::Yard3D),
            #[cfg(feature = "terrain-physics")]
            "terrain-yard3d" => Ok(Self::TerrainYard3D),
            #[cfg(feature = "navigation")]
            "navigation-yard3d" => Ok(Self::NavigationYard3D),
            _ => Err(format!("unsupported local game '{name}'; expected physics, arena or yard3d")),
        }
    }

    /// Missing or unknown identities never select a fallback decoder.
    pub fn from_name(name: &str) -> Result<Self, String> {
        match name {
            "PhysGame" => Ok(Self::PhysGame),
            "Arena" => Ok(Self::Arena),
            #[cfg(feature = "collect-dodge")]
            "CollectDodgeV1" => Ok(Self::CollectDodge),
            "Yard3D" => Ok(Self::Yard3D),
            #[cfg(feature = "terrain-physics")]
            "TerrainYard3D" => Ok(Self::TerrainYard3D),
            #[cfg(feature = "navigation")]
            "NavigationYard3D" => Ok(Self::NavigationYard3D),
            _ => Err(format!("unsupported editor game '{name}'; expected explicit PhysGame, Arena or Yard3D")),
        }
    }

    pub fn name(self) -> &'static str {
        match self { Self::PhysGame => "PhysGame", Self::Arena => "Arena", #[cfg(feature = "collect-dodge")] Self::CollectDodge => "CollectDodgeV1", Self::Yard3D => "Yard3D", #[cfg(feature = "terrain-physics")] Self::TerrainYard3D => "TerrainYard3D", #[cfg(feature = "navigation")] Self::NavigationYard3D => "NavigationYard3D" }
    }

    pub fn is_collect(self) -> bool {
        #[cfg(feature = "collect-dodge")] { self == Self::CollectDodge }
        #[cfg(not(feature = "collect-dodge"))] { false }
    }
    pub fn has_keyboard(self) -> bool { self == Self::Arena || self.is_collect() }
    pub fn position_field(self) -> &'static str { if self.is_collect() { "position" } else { "pos" } }
    pub fn is_3d(self) -> bool {
        self == Self::Yard3D || self.is_terrain() || self.is_navigation()
    }

    pub fn is_terrain(self) -> bool {
        #[cfg(feature = "terrain-physics")]
        { self == Self::TerrainYard3D }
        #[cfg(not(feature = "terrain-physics"))]
        { false }
    }

    pub fn is_navigation(self) -> bool {
        #[cfg(feature = "navigation")]
        { self == Self::NavigationYard3D }
        #[cfg(not(feature = "navigation"))]
        { false }
    }

    pub fn has_pinned_scene(self) -> bool {
        self.is_terrain() || self.is_navigation()
    }

    pub fn types(self) -> TypeRegistry {
        let mut types = TypeRegistry::new();
        match self {
            Self::PhysGame => orr_sample::physics_game::register_reflect(&mut types),
            Self::Arena => orr_sample::arena_game::register_reflect(&mut types),
            #[cfg(feature = "collect-dodge")]
            Self::CollectDodge => orr_sample::collect_game::register_reflect(&mut types),
            Self::Yard3D => orr_sample::yard3d_game::register_reflect(&mut types),
            #[cfg(feature = "terrain-physics")]
            Self::TerrainYard3D => return orr_remote::terrain_yard3d::terrain_yard3d_types(),
            #[cfg(feature = "navigation")]
            Self::NavigationYard3D => return orr_remote::navigation_yard3d::navigation_yard3d_types(),
        }
        types
    }

    pub fn drawables(self, frame: FrameView<'_>) -> Vec<Drawable> {
        match self {
            Self::PhysGame => orr_sample::physics_view::body_views(frame),
            Self::Arena => orr_sample::arena_view::editor_drawables(frame),
            #[cfg(feature = "collect-dodge")]
            Self::CollectDodge => orr_sample::collect_view::editor_drawables(frame),
            Self::Yard3D => Vec::new(),
            #[cfg(feature = "terrain-physics")]
            Self::TerrainYard3D => Vec::new(),
            #[cfg(feature = "navigation")]
            Self::NavigationYard3D => Vec::new(),
        }
    }

    pub fn position_component(self) -> &'static str {
        match self { Self::PhysGame => "orr_physics::Body", Self::Arena => "Position", #[cfg(feature = "collect-dodge")] Self::CollectDodge => "CollectDodgeV1::Actor", Self::Yard3D => "orr_physics3d::Body", #[cfg(feature = "terrain-physics")] Self::TerrainYard3D => "orr_physics3d::Body", #[cfg(feature = "navigation")] Self::NavigationYard3D => "NavigationAgent" }
    }

    /// Default scene for a locally started game, preferring the current
    /// working tree and falling back to the scene shipped with the editor.
    pub fn default_scene_path(self) -> PathBuf {
        let filename = match self {
            Self::PhysGame => "physics_demo.scene.yaml",
            Self::Arena => "arena_blank.scene.yaml",
            #[cfg(feature = "collect-dodge")]
            Self::CollectDodge => "collect_dodge_v1.scene.yaml",
            Self::Yard3D => "yard3d_authoring.scene.yaml",
            #[cfg(feature = "terrain-physics")]
            Self::TerrainYard3D => "terrain_sphere.scene.yaml",
            #[cfg(feature = "navigation")]
            Self::NavigationYard3D => "navigation_blank.scene.yaml",
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
    #[cfg(feature = "collect-dodge")]
    CollectDodge(RemoteBridge<orr_sample::collect_game::CollectDodgeV1>),
    Yard3D(RemoteBridge<orr_sample::yard3d_game::Yard3D>),
    #[cfg(feature = "terrain-physics")]
    TerrainYard3D(RemoteBridge<orr_remote::terrain_yard3d::TerrainYard3D>),
    #[cfg(feature = "navigation")]
    NavigationYard3D(RemoteBridge<orr_remote::navigation_yard3d::NavigationYard3D>),
}

impl EditorStream {
    pub fn connect(game: EditorGame, cfg: RemoteConfig) -> Result<Self, String> {
        match game {
            EditorGame::PhysGame => RemoteBridge::connect(cfg).map(Self::Phys),
            EditorGame::Arena => RemoteBridge::connect(cfg).map(Self::Arena),
            #[cfg(feature = "collect-dodge")]
            EditorGame::CollectDodge => RemoteBridge::connect(cfg).map(Self::CollectDodge),
            EditorGame::Yard3D => RemoteBridge::connect(cfg).map(Self::Yard3D),
            #[cfg(feature = "terrain-physics")]
            EditorGame::TerrainYard3D => RemoteBridge::connect(cfg).map(Self::TerrainYard3D),
            #[cfg(feature = "navigation")]
            EditorGame::NavigationYard3D => RemoteBridge::connect(cfg).map(Self::NavigationYard3D),
        }
    }

    pub fn connect_transport(game: EditorGame, transport: Box<dyn Transport>, cfg: RemoteConfig) -> Result<Self, String> {
        match game {
            EditorGame::PhysGame => RemoteBridge::connect_transport(transport, cfg).map(Self::Phys),
            EditorGame::Arena => RemoteBridge::connect_transport(transport, cfg).map(Self::Arena),
            #[cfg(feature = "collect-dodge")]
            EditorGame::CollectDodge => RemoteBridge::connect_transport(transport, cfg).map(Self::CollectDodge),
            EditorGame::Yard3D => RemoteBridge::connect_transport(transport, cfg).map(Self::Yard3D),
            #[cfg(feature = "terrain-physics")]
            EditorGame::TerrainYard3D => RemoteBridge::connect_transport(transport, cfg).map(Self::TerrainYard3D),
            #[cfg(feature = "navigation")]
            EditorGame::NavigationYard3D => RemoteBridge::connect_transport(transport, cfg).map(Self::NavigationYard3D),
        }
    }

    pub fn poll_view(&mut self) -> ViewUpdate<()> {
        match self { Self::Phys(s) => normalize(s.poll_view()), Self::Arena(s) => normalize(s.poll_view()), #[cfg(feature = "collect-dodge")] Self::CollectDodge(s) => normalize(s.poll_view()), Self::Yard3D(s) => normalize(s.poll_view()), #[cfg(feature = "terrain-physics")] Self::TerrainYard3D(s) => normalize(s.poll_view()), #[cfg(feature = "navigation")] Self::NavigationYard3D(s) => normalize(s.poll_view()) }
    }

    #[cfg(test)]
    pub fn snapshot(&self) -> Option<orr_bridge::Snapshot> {
        match self { Self::Phys(s) => s.snapshot(), Self::Arena(s) => s.snapshot(), #[cfg(feature = "collect-dodge")] Self::CollectDodge(s) => s.snapshot(), Self::Yard3D(s) => s.snapshot(), #[cfg(feature = "terrain-physics")] Self::TerrainYard3D(s) => s.snapshot(), #[cfg(feature = "navigation")] Self::NavigationYard3D(s) => s.snapshot() }
    }

    #[cfg(test)]
    pub fn request(&self, method: &str, params: serde_json::Value) -> Result<(), orr_bridge::BridgeError> {
        match self { Self::Phys(s) => s.request(method, params), Self::Arena(s) => s.request(method, params), #[cfg(feature = "collect-dodge")] Self::CollectDodge(s) => s.request(method, params), Self::Yard3D(s) => s.request(method, params), #[cfg(feature = "terrain-physics")] Self::TerrainYard3D(s) => s.request(method, params), #[cfg(feature = "navigation")] Self::NavigationYard3D(s) => s.request(method, params) }
    }

    pub fn take_errors(&self) -> Vec<RpcError> {
        match self { Self::Phys(s) => s.take_errors(), Self::Arena(s) => s.take_errors(), #[cfg(feature = "collect-dodge")] Self::CollectDodge(s) => s.take_errors(), Self::Yard3D(s) => s.take_errors(), #[cfg(feature = "terrain-physics")] Self::TerrainYard3D(s) => s.take_errors(), #[cfg(feature = "navigation")] Self::NavigationYard3D(s) => s.take_errors() }
    }

    pub fn is_alive(&self) -> bool {
        match self { Self::Phys(s) => s.is_alive(), Self::Arena(s) => s.is_alive(), #[cfg(feature = "collect-dodge")] Self::CollectDodge(s) => s.is_alive(), Self::Yard3D(s) => s.is_alive(), #[cfg(feature = "terrain-physics")] Self::TerrainYard3D(s) => s.is_alive(), #[cfg(feature = "navigation")] Self::NavigationYard3D(s) => s.is_alive() }
    }

    pub fn view_delivery(&self) -> RemoteViewDelivery {
        match self { Self::Phys(s) => s.view_delivery(), Self::Arena(s) => s.view_delivery(), #[cfg(feature = "collect-dodge")] Self::CollectDodge(s) => s.view_delivery(), Self::Yard3D(s) => s.view_delivery(), #[cfg(feature = "terrain-physics")] Self::TerrainYard3D(s) => s.view_delivery(), #[cfg(feature = "navigation")] Self::NavigationYard3D(s) => s.view_delivery() }
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
