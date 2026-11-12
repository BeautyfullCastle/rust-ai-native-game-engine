//! Closed, compiled game adapters. Gameplay ownership lives in the editor ERP state machine.
use orr_bridge::{Bridge, BridgeEvent, FrameView, ViewUpdate};
use orr_reflect::TypeRegistry;
use orr_remote::{RemoteBridge, RemoteConfig, RemoteViewDelivery, RpcError, Transport};
use orr_sample::arena_game::Arena;
use orr_sample::physics_game::PhysGame;
use std::path::PathBuf;
pub use orr_sample::editor_view::Drawable;

/// Existing spatial fields for picking; absent for embedded gameplay views.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditorSpatialPosition {
    pub component: &'static str,
    pub field: &'static str,
}

/// Games with a compiled frame decoder, reflection and viewport mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorGame {
    #[cfg(feature = "static-gameplay")]
    Gameplay(crate::static_adapter::RegisteredGameplay),
    #[cfg(feature = "static-games")]
    Registered(crate::static_adapter::RegisteredGame),
    PhysGame,
    Arena,
    #[cfg(feature = "collect-dodge")]
    CollectDodge,
    #[cfg(feature = "room-project")]
    RoomEscape,
    Yard3D,
    #[cfg(feature = "terrain-physics")]
    TerrainYard3D,
    #[cfg(feature = "navigation")]
    NavigationYard3D,
    #[cfg(feature = "mesh-navigation")]
    MeshNavigation,
    #[cfg(feature = "mesh-hazards")]
    MeshHazardsV1,
}

impl EditorGame {
    #[cfg(feature = "static-gameplay")]
    pub fn gameplay(self) -> Option<crate::static_adapter::RegisteredGameplay> {
        if let Self::Gameplay(game) = self { Some(game) } else { None }
    }
    pub fn is_gameplay(self) -> bool {
        #[cfg(feature = "static-gameplay")]
        { self.gameplay().is_some() }
        #[cfg(not(feature = "static-gameplay"))]
        { false }
    }
    pub fn has_embedded_gameplay(self) -> bool {
        #[cfg(feature = "static-gameplay")]
        { self.gameplay().is_some_and(|game| game.spatial().is_none()) }
        #[cfg(not(feature = "static-gameplay"))]
        { false }
    }
    /// Embedded capabilities have no synthetic picking component or position.
    pub fn spatial_position(self) -> Option<EditorSpatialPosition> {
        #[cfg(feature = "static-gameplay")]
        if let Some(game) = self.gameplay() {
            return game.spatial().map(|p| EditorSpatialPosition { component:p.component, field:p.field });
        }
        Some(EditorSpatialPosition {
            component: self.position_component(), field: self.position_field(),
        })
    }
    #[cfg(feature = "static-games")]
    pub fn registered(self) -> Option<crate::static_adapter::RegisteredGame> {
        if let Self::Registered(game) = self { Some(game) } else { None }
    }
    pub fn is_registered(self) -> bool {
        #[cfg(feature = "static-games")]
        { self.registered().is_some() }
        #[cfg(not(feature = "static-games"))]
        { false }
    }
    /// A registered game supplies its own controls hint, not an Arena fallback.
    pub fn registered_input_hint(self) -> Option<&'static str> {
        #[cfg(feature = "static-games")]
        { self.registered().map(|game| game.input_hint()) }
        #[cfg(not(feature = "static-games"))]
        { None }
    }

    /// Missing or unknown identities never select a fallback decoder.
    pub fn from_local_name(name: &str) -> Result<Self, String> {
        match name {
            "physics" => Ok(Self::PhysGame),
            "arena" => Ok(Self::Arena),
            #[cfg(feature = "collect-dodge")]
            "collect-dodge-v1" => Ok(Self::CollectDodge),
            #[cfg(feature = "room-project")]
            "room-escape-v1" => Ok(Self::RoomEscape),
            "yard3d" => Ok(Self::Yard3D),
            #[cfg(feature = "terrain-physics")]
            "terrain-yard3d" => Ok(Self::TerrainYard3D),
            #[cfg(feature = "navigation")]
            "navigation-yard3d" => Ok(Self::NavigationYard3D),
            #[cfg(feature = "mesh-navigation")]
            "mesh-navigation" => Ok(Self::MeshNavigation),
            #[cfg(feature = "mesh-hazards")]
            "mesh-hazards-v1" => Ok(Self::MeshHazardsV1),
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
            #[cfg(feature = "room-project")]
            "RoomEscapeV1" => Ok(Self::RoomEscape),
            "Yard3D" => Ok(Self::Yard3D),
            #[cfg(feature = "terrain-physics")]
            "TerrainYard3D" => Ok(Self::TerrainYard3D),
            #[cfg(feature = "navigation")]
            "NavigationYard3D" => Ok(Self::NavigationYard3D),
            #[cfg(feature = "mesh-navigation")]
            "MeshNavigation" => Ok(Self::MeshNavigation),
            #[cfg(feature = "mesh-hazards")]
            "MeshHazardsV1" => Ok(Self::MeshHazardsV1),
            _ => Err(format!("unsupported editor game '{name}'; expected explicit PhysGame, Arena or Yard3D")),
        }
    }

    pub fn name(self) -> &'static str {
        match self { #[cfg(feature = "static-gameplay")] Self::Gameplay(game) => game.name(), #[cfg(feature = "static-games")] Self::Registered(game) => game.name(), Self::PhysGame => "PhysGame", Self::Arena => "Arena", #[cfg(feature = "collect-dodge")] Self::CollectDodge => "CollectDodgeV1", #[cfg(feature = "room-project")] Self::RoomEscape => "RoomEscapeV1", Self::Yard3D => "Yard3D", #[cfg(feature = "terrain-physics")] Self::TerrainYard3D => "TerrainYard3D", #[cfg(feature = "navigation")] Self::NavigationYard3D => "NavigationYard3D", #[cfg(feature = "mesh-navigation")] Self::MeshNavigation => "MeshNavigation", #[cfg(feature = "mesh-hazards")] Self::MeshHazardsV1 => "MeshHazardsV1" }
    }

    pub fn is_collect(self) -> bool {
        #[cfg(feature = "collect-dodge")] { self == Self::CollectDodge }
        #[cfg(not(feature = "collect-dodge"))] { false }
    }
    pub fn is_room(self) -> bool {
        #[cfg(feature = "room-project")] { self == Self::RoomEscape }
        #[cfg(not(feature = "room-project"))] { false }
    }
    pub fn has_keyboard(self) -> bool { self == Self::Arena || self.is_collect() || self.is_room() || self.is_registered() }
    /// Spatial-only legacy API. Use `spatial_position` for optional capabilities.
    pub fn position_field(self) -> &'static str {
        #[cfg(feature = "static-gameplay")]
        if let Some(game) = self.gameplay() {
            return game.spatial().expect("embedded gameplay has no spatial position").field;
        }
        #[cfg(feature = "static-games")]
        if let Some(game) = self.registered() { return game.position().field; }
        if self.is_collect() { "position" } else { "pos" } }
    pub fn is_3d(self) -> bool {
        self == Self::Yard3D || self.is_room() || self.is_terrain() || self.is_navigation() || self.has_mesh_view()
    }

    pub fn is_mesh_navigation(self) -> bool {
        #[cfg(feature = "mesh-navigation")]
        { self == Self::MeshNavigation }
        #[cfg(not(feature = "mesh-navigation"))]
        { false }
    }

    /// Shared rendering/input capability; never an export or workspace admission check.
    pub fn has_mesh_view(self) -> bool {
        self.is_mesh_navigation() || self.is_mesh_hazards()
    }

    pub fn is_mesh_hazards(self) -> bool {
        #[cfg(feature = "mesh-hazards")]
        { self == Self::MeshHazardsV1 }
        #[cfg(not(feature = "mesh-hazards"))]
        { false }
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
            #[cfg(feature = "static-gameplay")]
            Self::Gameplay(game) => return game.types(),
            #[cfg(feature = "static-games")]
            Self::Registered(game) => return game.types(),
            Self::PhysGame => orr_sample::physics_game::register_reflect(&mut types),
            Self::Arena => orr_sample::arena_game::register_reflect(&mut types),
            #[cfg(feature = "collect-dodge")]
            Self::CollectDodge => orr_sample::collect_game::register_reflect(&mut types),
            #[cfg(feature = "room-project")]
            Self::RoomEscape => orr_sample::room_game::register_reflect(&mut types),
            Self::Yard3D => orr_sample::yard3d_game::register_reflect(&mut types),
            #[cfg(feature = "terrain-physics")]
            Self::TerrainYard3D => return orr_remote::terrain_yard3d::terrain_yard3d_types(),
            #[cfg(feature = "navigation")]
            Self::NavigationYard3D => return orr_remote::navigation_yard3d::navigation_yard3d_types(),
            #[cfg(feature = "mesh-navigation")]
            Self::MeshNavigation => return orr_remote::mesh_navigation::mesh_navigation_types(),
            #[cfg(feature = "mesh-hazards")]
            Self::MeshHazardsV1 => return orr_remote::mesh_navigation::hazards::hazards_types(),
        }
        types
    }

    pub fn drawables(self, frame: FrameView<'_>) -> Vec<Drawable> {
        match self {
            #[cfg(feature = "static-gameplay")]
            Self::Gameplay(game) => game.draw(frame),
            #[cfg(feature = "static-games")]
            Self::Registered(game) => game.draw(frame),
            Self::PhysGame => orr_sample::physics_view::body_views(frame),
            Self::Arena => orr_sample::arena_view::editor_drawables(frame),
            #[cfg(feature = "collect-dodge")]
            Self::CollectDodge => orr_sample::collect_view::editor_drawables(frame),
            #[cfg(feature = "room-project")]
            Self::RoomEscape => Vec::new(),
            Self::Yard3D => Vec::new(),
            #[cfg(feature = "terrain-physics")]
            Self::TerrainYard3D => Vec::new(),
            #[cfg(feature = "navigation")]
            Self::NavigationYard3D => Vec::new(),
            #[cfg(feature = "mesh-navigation")]
            Self::MeshNavigation => Vec::new(),
            #[cfg(feature = "mesh-hazards")]
            Self::MeshHazardsV1 => Vec::new(),
        }
    }

    /// Spatial-only legacy API. Use `spatial_position` for optional capabilities.
    pub fn position_component(self) -> &'static str {
        match self { #[cfg(feature = "static-gameplay")] Self::Gameplay(game) => game.spatial().expect("embedded gameplay has no spatial position").component, #[cfg(feature = "static-games")] Self::Registered(game) => game.position().component, Self::PhysGame => "orr_physics::Body", Self::Arena => "Position", #[cfg(feature = "collect-dodge")] Self::CollectDodge => "CollectDodgeV1::Actor", #[cfg(feature = "room-project")] Self::RoomEscape => "orr_physics3d::Body", Self::Yard3D => "orr_physics3d::Body", #[cfg(feature = "terrain-physics")] Self::TerrainYard3D => "orr_physics3d::Body", #[cfg(feature = "navigation")] Self::NavigationYard3D => "NavigationAgent", #[cfg(feature = "mesh-navigation")] Self::MeshNavigation => "MeshAgent", #[cfg(feature = "mesh-hazards")] Self::MeshHazardsV1 => "MeshAgent" }
    }

    /// Default scene for a locally started game, preferring the current
    /// working tree and falling back to the scene shipped with the editor.
    pub fn default_scene_path(self) -> PathBuf {
        #[cfg(feature = "static-gameplay")]
        if self.is_gameplay() { return PathBuf::new(); }
        // Registered startup requires an explicit existing scene; no cwd/sample fallback.
        #[cfg(feature = "static-games")]
        if self.is_registered() { return PathBuf::new(); }
        let filename = match self {
            #[cfg(feature = "static-gameplay")]
            Self::Gameplay(_) => unreachable!("gameplay scenes require explicit project paths"),
            #[cfg(feature = "static-games")]
            Self::Registered(_) => unreachable!("registered scenes require explicit paths"),
            Self::PhysGame => "physics_demo.scene.yaml",
            Self::Arena => "arena_blank.scene.yaml",
            #[cfg(feature = "collect-dodge")]
            Self::CollectDodge => "collect_dodge_v1.scene.yaml",
            #[cfg(feature = "room-project")]
            Self::RoomEscape => "room_escape_v1.scene.yaml",
            Self::Yard3D => "yard3d_authoring.scene.yaml",
            #[cfg(feature = "terrain-physics")]
            Self::TerrainYard3D => "terrain_sphere.scene.yaml",
            #[cfg(feature = "navigation")]
            Self::NavigationYard3D => "navigation_blank.scene.yaml",
            #[cfg(feature = "mesh-navigation")]
            Self::MeshNavigation => "mesh_navigation_blank.scene.yaml",
            #[cfg(feature = "mesh-hazards")]
            Self::MeshHazardsV1 => "mesh_navigation_blank.scene.yaml",
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
    #[cfg(feature = "static-gameplay")]
    Gameplay(Box<dyn crate::static_adapter::StaticFrameStream>),
    #[cfg(feature = "static-games")]
    Registered(Box<dyn crate::static_adapter::StaticFrameStream>),
    Phys(RemoteBridge<PhysGame>),
    Arena(RemoteBridge<Arena>),
    #[cfg(feature = "collect-dodge")]
    CollectDodge(RemoteBridge<orr_sample::collect_game::CollectDodgeV1>),
    #[cfg(feature = "room-project")]
    RoomEscape(RemoteBridge<orr_sample::room_game::RoomEscapeV1>),
    Yard3D(RemoteBridge<orr_sample::yard3d_game::Yard3D>),
    #[cfg(feature = "terrain-physics")]
    TerrainYard3D(RemoteBridge<orr_remote::terrain_yard3d::TerrainYard3D>),
    #[cfg(feature = "navigation")]
    NavigationYard3D(RemoteBridge<orr_remote::navigation_yard3d::NavigationYard3D>),
    #[cfg(feature = "mesh-navigation")]
    MeshNavigation(RemoteBridge<orr_remote::mesh_navigation::MeshNavigation>),
    #[cfg(feature = "mesh-hazards")]
    MeshHazards(RemoteBridge<orr_remote::mesh_navigation::hazards::MeshHazardNavigation>),
}

impl EditorStream {
    pub fn connect(game: EditorGame, cfg: RemoteConfig) -> Result<Self, String> {
        match game {
            #[cfg(feature = "static-gameplay")]
            EditorGame::Gameplay(_) => Err("gameplay registrations require explicit local startup".into()),
            #[cfg(feature = "static-games")]
            EditorGame::Registered(_) => Err("registered games require explicit local startup".into()),
            EditorGame::PhysGame => RemoteBridge::connect(cfg).map(Self::Phys),
            EditorGame::Arena => RemoteBridge::connect(cfg).map(Self::Arena),
            #[cfg(feature = "collect-dodge")]
            EditorGame::CollectDodge => RemoteBridge::connect(cfg).map(Self::CollectDodge),
            #[cfg(feature = "room-project")]
            EditorGame::RoomEscape => RemoteBridge::connect(cfg).map(Self::RoomEscape),
            EditorGame::Yard3D => RemoteBridge::connect(cfg).map(Self::Yard3D),
            #[cfg(feature = "terrain-physics")]
            EditorGame::TerrainYard3D => RemoteBridge::connect(cfg).map(Self::TerrainYard3D),
            #[cfg(feature = "navigation")]
            EditorGame::NavigationYard3D => RemoteBridge::connect(cfg).map(Self::NavigationYard3D),
            #[cfg(feature = "mesh-navigation")]
            EditorGame::MeshNavigation => RemoteBridge::connect(cfg).map(Self::MeshNavigation),
            #[cfg(feature = "mesh-hazards")]
            EditorGame::MeshHazardsV1 => RemoteBridge::connect(cfg).map(Self::MeshHazards),
        }
    }

    pub fn connect_transport(game: EditorGame, transport: Box<dyn Transport>, cfg: RemoteConfig) -> Result<Self, String> {
        match game {
            #[cfg(feature = "static-gameplay")]
            EditorGame::Gameplay(game) => game.stream(transport, cfg).map(Self::Gameplay),
            #[cfg(feature = "static-games")]
            EditorGame::Registered(game) => game.stream(transport, cfg).map(Self::Registered),
            EditorGame::PhysGame => RemoteBridge::connect_transport(transport, cfg).map(Self::Phys),
            EditorGame::Arena => RemoteBridge::connect_transport(transport, cfg).map(Self::Arena),
            #[cfg(feature = "collect-dodge")]
            EditorGame::CollectDodge => RemoteBridge::connect_transport(transport, cfg).map(Self::CollectDodge),
            #[cfg(feature = "room-project")]
            EditorGame::RoomEscape => RemoteBridge::connect_transport(transport, cfg).map(Self::RoomEscape),
            EditorGame::Yard3D => RemoteBridge::connect_transport(transport, cfg).map(Self::Yard3D),
            #[cfg(feature = "terrain-physics")]
            EditorGame::TerrainYard3D => RemoteBridge::connect_transport(transport, cfg).map(Self::TerrainYard3D),
            #[cfg(feature = "navigation")]
            EditorGame::NavigationYard3D => RemoteBridge::connect_transport(transport, cfg).map(Self::NavigationYard3D),
            #[cfg(feature = "mesh-navigation")]
            EditorGame::MeshNavigation => RemoteBridge::connect_transport(transport, cfg).map(Self::MeshNavigation),
            #[cfg(feature = "mesh-hazards")]
            EditorGame::MeshHazardsV1 => RemoteBridge::connect_transport(transport, cfg).map(Self::MeshHazards),
        }
    }

    pub fn poll_view(&mut self) -> ViewUpdate<()> {
        match self { #[cfg(feature = "static-gameplay")] Self::Gameplay(s) => s.poll_view(), #[cfg(feature = "static-games")] Self::Registered(s) => s.poll_view(), Self::Phys(s) => normalize(s.poll_view()), Self::Arena(s) => normalize(s.poll_view()), #[cfg(feature = "collect-dodge")] Self::CollectDodge(s) => normalize(s.poll_view()), #[cfg(feature = "room-project")] Self::RoomEscape(s) => normalize(s.poll_view()), Self::Yard3D(s) => normalize(s.poll_view()), #[cfg(feature = "terrain-physics")] Self::TerrainYard3D(s) => normalize(s.poll_view()), #[cfg(feature = "navigation")] Self::NavigationYard3D(s) => normalize(s.poll_view()), #[cfg(feature = "mesh-navigation")] Self::MeshNavigation(s) => normalize(s.poll_view()), #[cfg(feature = "mesh-hazards")] Self::MeshHazards(s) => normalize(s.poll_view()) }
    }

    #[cfg(test)]
    pub fn snapshot(&self) -> Option<orr_bridge::Snapshot> {
        match self { #[cfg(feature = "static-gameplay")] Self::Gameplay(_) => None, #[cfg(feature = "static-games")] Self::Registered(_) => None, Self::Phys(s) => s.snapshot(), Self::Arena(s) => s.snapshot(), #[cfg(feature = "collect-dodge")] Self::CollectDodge(s) => s.snapshot(), #[cfg(feature = "room-project")] Self::RoomEscape(s) => s.snapshot(), Self::Yard3D(s) => s.snapshot(), #[cfg(feature = "terrain-physics")] Self::TerrainYard3D(s) => s.snapshot(), #[cfg(feature = "navigation")] Self::NavigationYard3D(s) => s.snapshot(), #[cfg(feature = "mesh-navigation")] Self::MeshNavigation(s) => s.snapshot(), #[cfg(feature = "mesh-hazards")] Self::MeshHazards(s) => s.snapshot() }
    }

    #[cfg(test)]
    pub fn request(&self, method: &str, params: serde_json::Value) -> Result<(), orr_bridge::BridgeError> {
        match self { #[cfg(feature = "static-gameplay")] Self::Gameplay(_) => Err(orr_bridge::BridgeError::Disconnected), #[cfg(feature = "static-games")] Self::Registered(_) => Err(orr_bridge::BridgeError::Disconnected), Self::Phys(s) => s.request(method, params), Self::Arena(s) => s.request(method, params), #[cfg(feature = "collect-dodge")] Self::CollectDodge(s) => s.request(method, params), #[cfg(feature = "room-project")] Self::RoomEscape(s) => s.request(method, params), Self::Yard3D(s) => s.request(method, params), #[cfg(feature = "terrain-physics")] Self::TerrainYard3D(s) => s.request(method, params), #[cfg(feature = "navigation")] Self::NavigationYard3D(s) => s.request(method, params), #[cfg(feature = "mesh-navigation")] Self::MeshNavigation(s) => s.request(method, params), #[cfg(feature = "mesh-hazards")] Self::MeshHazards(s) => s.request(method, params) }
    }

    pub fn take_errors(&self) -> Vec<RpcError> {
        match self { #[cfg(feature = "static-gameplay")] Self::Gameplay(s) => s.take_errors(), #[cfg(feature = "static-games")] Self::Registered(s) => s.take_errors(), Self::Phys(s) => s.take_errors(), Self::Arena(s) => s.take_errors(), #[cfg(feature = "collect-dodge")] Self::CollectDodge(s) => s.take_errors(), #[cfg(feature = "room-project")] Self::RoomEscape(s) => s.take_errors(), Self::Yard3D(s) => s.take_errors(), #[cfg(feature = "terrain-physics")] Self::TerrainYard3D(s) => s.take_errors(), #[cfg(feature = "navigation")] Self::NavigationYard3D(s) => s.take_errors(), #[cfg(feature = "mesh-navigation")] Self::MeshNavigation(s) => s.take_errors(), #[cfg(feature = "mesh-hazards")] Self::MeshHazards(s) => s.take_errors() }
    }

    pub fn is_alive(&self) -> bool {
        match self { #[cfg(feature = "static-gameplay")] Self::Gameplay(s) => s.is_alive(), #[cfg(feature = "static-games")] Self::Registered(s) => s.is_alive(), Self::Phys(s) => s.is_alive(), Self::Arena(s) => s.is_alive(), #[cfg(feature = "collect-dodge")] Self::CollectDodge(s) => s.is_alive(), #[cfg(feature = "room-project")] Self::RoomEscape(s) => s.is_alive(), Self::Yard3D(s) => s.is_alive(), #[cfg(feature = "terrain-physics")] Self::TerrainYard3D(s) => s.is_alive(), #[cfg(feature = "navigation")] Self::NavigationYard3D(s) => s.is_alive(), #[cfg(feature = "mesh-navigation")] Self::MeshNavigation(s) => s.is_alive(), #[cfg(feature = "mesh-hazards")] Self::MeshHazards(s) => s.is_alive() }
    }

    pub fn view_delivery(&self) -> RemoteViewDelivery {
        match self { #[cfg(feature = "static-gameplay")] Self::Gameplay(s) => s.view_delivery(), #[cfg(feature = "static-games")] Self::Registered(s) => s.view_delivery(), Self::Phys(s) => s.view_delivery(), Self::Arena(s) => s.view_delivery(), #[cfg(feature = "collect-dodge")] Self::CollectDodge(s) => s.view_delivery(), #[cfg(feature = "room-project")] Self::RoomEscape(s) => s.view_delivery(), Self::Yard3D(s) => s.view_delivery(), #[cfg(feature = "terrain-physics")] Self::TerrainYard3D(s) => s.view_delivery(), #[cfg(feature = "navigation")] Self::NavigationYard3D(s) => s.view_delivery(), #[cfg(feature = "mesh-navigation")] Self::MeshNavigation(s) => s.view_delivery(), #[cfg(feature = "mesh-hazards")] Self::MeshHazards(s) => s.view_delivery() }
    }

    #[cfg(feature = "static-gameplay")]
    pub fn frame_evidence(&self) -> Option<orr_remote::RemoteFrameEvidence> {
        match self { #[cfg(feature = "static-gameplay")] Self::Gameplay(s) => s.frame_evidence(), #[cfg(feature = "static-games")] Self::Registered(s) => s.frame_evidence(), Self::Phys(s) => s.frame_evidence(), Self::Arena(s) => s.frame_evidence(), #[cfg(feature = "collect-dodge")] Self::CollectDodge(s) => s.frame_evidence(), #[cfg(feature = "room-project")] Self::RoomEscape(s) => s.frame_evidence(), Self::Yard3D(s) => s.frame_evidence(), #[cfg(feature = "terrain-physics")] Self::TerrainYard3D(s) => s.frame_evidence(), #[cfg(feature = "navigation")] Self::NavigationYard3D(s) => s.frame_evidence(), #[cfg(feature = "mesh-navigation")] Self::MeshNavigation(s) => s.frame_evidence(), #[cfg(feature = "mesh-hazards")] Self::MeshHazards(s) => s.frame_evidence() }
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

#[cfg(all(test, feature = "mesh-hazards"))]
mod mesh_hazards_identity_tests {
    use super::*;

    #[test]
    fn explicit_hazard_identity_never_changes_legacy_identity() {
        let hazards = EditorGame::MeshHazardsV1;
        assert_eq!(EditorGame::from_local_name("mesh-hazards-v1").unwrap(), hazards);
        assert_eq!(EditorGame::from_name("MeshHazardsV1").unwrap(), hazards);
        assert_eq!(hazards.name(), "MeshHazardsV1");
        assert!(hazards.has_mesh_view() && hazards.is_3d() && hazards.is_mesh_hazards());
        assert!(!hazards.is_mesh_navigation());
        assert_eq!(EditorGame::from_local_name("mesh-navigation").unwrap(), EditorGame::MeshNavigation);
        assert_eq!(EditorGame::from_name("MeshNavigation").unwrap(), EditorGame::MeshNavigation);
        assert!(EditorGame::MeshNavigation.has_mesh_view());
        assert!(!EditorGame::MeshNavigation.is_mesh_hazards());
        assert!(!EditorGame::PhysGame.has_mesh_view());
        assert!(EditorGame::from_name("mesh-hazards-v1").is_err());
    }
}
