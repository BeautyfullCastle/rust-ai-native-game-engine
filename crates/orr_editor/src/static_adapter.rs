//! Opt-in, statically linked games. Registration is immutable and explicit.
//!
//! The executable owns a registry for its startup lifetime (for example in a
//! `OnceLock`). No process-global registry is maintained by the editor. These
//! callbacks are trusted native code, not sandboxed plugins. The editor retains
//! ERP ownership, input grants and scene lifecycle; adapters supply only the
//! typed host, frame bridge and view conversion.
use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;

use orr_bridge::{FrameView, ViewUpdate};
use orr_reflect::{TypeRegistry, Value};
use orr_remote::{
    ExpectedInputDescriptor, LocalHost, RemoteConfig, RemoteIdentity, RemoteViewDelivery, RpcError,
    ServerConfig, Transport,
};
use serde_json::Value as J;

use crate::game::Drawable;

/// Owned snapshots only. A borrowed frame must never be retained by an adapter.
pub trait StaticFrameStream {
    fn poll_view(&mut self) -> ViewUpdate<()>;
    fn take_errors(&self) -> Vec<RpcError>;
    fn is_alive(&self) -> bool;
    fn view_delivery(&self) -> RemoteViewDelivery;
}

/// Finite held input. This is not an event queue: `action` remains pressed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StaticHeldKeys {
    pub x: i8,
    pub y: i8,
    pub action: bool,
}
impl StaticHeldKeys {
    fn index(self) -> Option<usize> {
        if !(-1..=1).contains(&self.x) || !(-1..=1).contains(&self.y) {
            return None;
        }
        Some(((self.x + 1) as usize * 3 + (self.y + 1) as usize) * 2 + usize::from(self.action))
    }
}

/// Stage one exposes one player and the editor's WASD/arrows/Space controls.
pub struct StaticInputDescriptor {
    pub type_name: &'static str,
    pub types: fn() -> TypeRegistry,
    pub max_players: u8,
    pub hint: &'static str,
    pub held: fn(StaticHeldKeys) -> J,
}

/// A reflected vec2 position used by ordinary viewport picking and dragging.
#[derive(Clone, Copy, Debug)]
pub struct StaticPosition {
    pub component: &'static str,
    pub field: &'static str,
}

/// Captured scene and host settings. Preserve the supplied settings/services.
/// The typed startup helper must validate this exact neutral value against
/// `G::Input::default()` before publishing a connector. A hash is not trust.
pub struct StaticHostRequest {
    pub scene_text: String,
    pub scene_path: PathBuf,
    pub config: ServerConfig,
    pub work_status: orr_remote::work_status::WorkStatusService,
    pub neutral_value: J,
    pub expected_input: ExpectedInputDescriptor,
}

type HostFactory = fn(StaticHostRequest) -> Result<LocalHost, String>;
type StreamFactory = fn(
    Box<dyn Transport>,
    RemoteConfig,
    ExpectedInputDescriptor,
) -> Result<Box<dyn StaticFrameStream>, String>;

/// Trusted, statically linked callbacks. All schema/input output is frozen once
/// by registry admission, before any host or frame decoder starts.
pub struct StaticGameDescriptor {
    pub game_id: &'static str,
    pub local_alias: &'static str,
    pub build_id: u64,
    pub types: fn() -> TypeRegistry,
    pub host: HostFactory,
    pub stream: StreamFactory,
    pub draw: for<'a> fn(FrameView<'a>) -> Vec<Drawable>,
    pub position: StaticPosition,
    pub input: StaticInputDescriptor,
}

struct Entry {
    descriptor: StaticGameDescriptor,
    types: TypeRegistry,
    identity: RemoteIdentity,
    input: ExpectedInputDescriptor,
    held: Vec<J>,
}

/// At most 32 immutable registrations. No handles can be manufactured directly.
pub struct StaticGameRegistry {
    entries: Vec<Entry>,
}

// Include disabled builtins: installing a feature must never change ownership
// of an already admitted static game's name or alias.
const RESERVED: &[&str] = &[
    "PhysGame",
    "physics",
    "Arena",
    "arena",
    "CollectDodgeV1",
    "collect-dodge-v1",
    "RoomEscapeV1",
    "room-escape-v1",
    "Yard3D",
    "yard3d",
    "TerrainYard3D",
    "terrain-yard3d",
    "NavigationYard3D",
    "navigation-yard3d",
    "MeshNavigation",
    "mesh-navigation",
    "MeshHazardsV1",
    "mesh-hazards-v1",
];
fn name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-:".contains(&c))
}
fn bounded_json(value: &J, limit: usize) -> Result<(), String> {
    if serde_json::to_vec(value).map_err(|e| e.to_string())?.len() > limit {
        return Err("static adapter JSON exceeds its admission bound".into());
    }
    Ok(())
}

impl StaticGameRegistry {
    pub fn new(descriptors: Vec<StaticGameDescriptor>) -> Result<Self, String> {
        if descriptors.is_empty() || descriptors.len() > 32 {
            return Err("static registry requires 1..=32 games".into());
        }
        let mut names = BTreeSet::new();
        // Validate the complete namespace before invoking any callback.
        for d in &descriptors {
            for id in [d.game_id, d.local_alias] {
                if !name(id) || RESERVED.contains(&id) || !names.insert(id) {
                    return Err(format!(
                        "invalid, duplicate or reserved static game name: {id}"
                    ));
                }
            }
        }
        let mut entries = Vec::with_capacity(descriptors.len());
        for descriptor in descriptors {
            let types = (descriptor.types)();
            let schema_text = if cfg!(feature = "linked-prefabs") {
                types.json_schema()
            } else {
                types.legacy_json_schema()
            };
            let schema: J = serde_json::from_str(&schema_text).map_err(|e| e.to_string())?;
            bounded_json(&schema, 256 * 1024)?;
            let position = types
                .get(descriptor.position.component)
                .ok_or("static position component is not reflected")?;
            if position.kind() != orr_reflect::TypeKind::Component
                || !matches!(
                    position.get(&position.default_bytes(), descriptor.position.field),
                    Ok(Value::Vec2(_))
                )
            {
                return Err("static position must name a reflected vec2 component field".into());
            }
            let input = &descriptor.input;
            if input.max_players != 1
                || input.hint.is_empty()
                || input.hint.len() > 256
                || input.hint.chars().any(char::is_control)
            {
                return Err(
                    "static input requires one player and a bounded single-line hint".into(),
                );
            }
            let input_types = (input.types)();
            if input_types.types().count() != 1 {
                return Err("static input registry must contain exactly its input type".into());
            }
            let info = input_types
                .get(input.type_name)
                .ok_or("static input type is not reflected")?;
            if info.kind() != orr_reflect::TypeKind::Component {
                return Err("static input type must be registered as a component".into());
            }
            if info.size() > 64 * 1024 {
                return Err("static input type is too large".into());
            }
            let input_schema: J = serde_json::from_str(
                &input_types
                    .type_json_schema(input.type_name)
                    .ok_or("missing input schema")?,
            )
            .map_err(|e| e.to_string())?;
            bounded_json(&input_schema, 64 * 1024)?;
            let expected = ExpectedInputDescriptor::new(
                input_schema,
                orr_remote::methods::VALUE_FORMAT,
                input.max_players,
            )?;
            let mut held = Vec::with_capacity(18);
            for x in -1..=1 {
                for y in -1..=1 {
                    for action in [false, true] {
                        let value = (input.held)(StaticHeldKeys { x, y, action });
                        bounded_json(&value, 64 * 1024)?;
                        let decoded = orr_remote::json::json_to_value(info.desc(), &value, false)?;
                        let mut bytes = info.default_bytes();
                        info.set(&mut bytes, "", decoded)
                            .map_err(|e| e.to_string())?;
                        // Keep the exact admitted JSON, including exact fixed decimals.
                        held.push(value);
                    }
                }
            }
            let identity = RemoteIdentity {
                game: descriptor.game_id.into(),
                build_id: format!("0x{:016x}", descriptor.build_id),
                schema,
            };
            entries.push(Entry {
                descriptor,
                types,
                identity,
                input: expected,
                held,
            });
        }
        Ok(Self { entries })
    }

    /// The application supplies a stable startup lifetime, e.g. a OnceLock.
    pub fn handle(&'static self, id: &str) -> Result<RegisteredGame, String> {
        self.entries
            .iter()
            .position(|e| e.descriptor.game_id == id || e.descriptor.local_alias == id)
            .map(|index| RegisteredGame {
                registry: self,
                index,
            })
            .ok_or_else(|| format!("unregistered static game: {id}"))
    }
}

/// Equality is registry-instance identity plus validated index, never callbacks.
#[derive(Clone, Copy)]
pub struct RegisteredGame {
    registry: &'static StaticGameRegistry,
    index: usize,
}
impl PartialEq for RegisteredGame {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.registry, other.registry) && self.index == other.index
    }
}
impl Eq for RegisteredGame {}
impl fmt::Debug for RegisteredGame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("RegisteredGame").field(&self.name()).finish()
    }
}
impl RegisteredGame {
    fn entry(self) -> &'static Entry {
        &self.registry.entries[self.index]
    }
    pub fn name(self) -> &'static str {
        self.entry().descriptor.game_id
    }
    pub fn local_alias(self) -> &'static str {
        self.entry().descriptor.local_alias
    }
    pub fn expected_identity(self) -> RemoteIdentity {
        self.entry().identity.clone()
    }
    pub fn expected_input(self) -> ExpectedInputDescriptor {
        self.entry().input.clone()
    }
    pub fn neutral_value(self) -> J {
        self.entry().held[8].clone()
    }
    pub fn input_hint(self) -> &'static str {
        self.entry().descriptor.input.hint
    }
    pub(crate) fn types(self) -> TypeRegistry {
        self.entry().types.clone()
    }
    pub(crate) fn position(self) -> StaticPosition {
        self.entry().descriptor.position
    }
    pub(crate) fn held_value(self, keys: StaticHeldKeys) -> Option<J> {
        keys.index().map(|i| self.entry().held[i].clone())
    }
    pub(crate) fn draw(self, frame: FrameView<'_>) -> Vec<Drawable> {
        (self.entry().descriptor.draw)(frame)
    }
    pub(crate) fn spawn(self, request: StaticHostRequest) -> Result<LocalHost, String> {
        (self.entry().descriptor.host)(request)
    }
    pub(crate) fn stream(
        self,
        transport: Box<dyn Transport>,
        config: RemoteConfig,
    ) -> Result<Box<dyn StaticFrameStream>, String> {
        (self.entry().descriptor.stream)(transport, config, self.expected_input())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn scene_types() -> TypeRegistry {
        let mut types = TypeRegistry::new();
        types.register_component::<orr_sample::arena_game::Position>("Position");
        types
    }
    fn input_types() -> TypeRegistry {
        let mut types = TypeRegistry::new();
        types.register_component::<orr_sample::arena_game::ArenaInput>("FixtureInput");
        types
    }
    fn held(k: StaticHeldKeys) -> J {
        json!({"axis_x":k.x,"axis_y":k.y,"buttons":if k.action {vec!["fire"]} else {vec![]}})
    }
    fn no_host(_: StaticHostRequest) -> Result<LocalHost, String> {
        panic!("admission must not start a host")
    }
    fn no_stream(
        _: Box<dyn Transport>,
        _: RemoteConfig,
        _: ExpectedInputDescriptor,
    ) -> Result<Box<dyn StaticFrameStream>, String> {
        panic!("admission must not connect a stream")
    }
    fn no_draw(_: FrameView<'_>) -> Vec<Drawable> {
        Vec::new()
    }
    fn descriptor(id: &'static str, alias: &'static str) -> StaticGameDescriptor {
        StaticGameDescriptor {
            game_id: id,
            local_alias: alias,
            build_id: 0x1234,
            types: scene_types,
            host: no_host,
            stream: no_stream,
            draw: no_draw,
            position: StaticPosition {
                component: "Position",
                field: "pos",
            },
            input: StaticInputDescriptor {
                type_name: "FixtureInput",
                types: input_types,
                max_players: 1,
                hint: "WASD/arrows move; hold Space for action; Escape releases",
                held,
            },
        }
    }
    pub(crate) fn registry() -> &'static StaticGameRegistry {
        Box::leak(Box::new(
            StaticGameRegistry::new(vec![descriptor("FixtureGame", "fixture-game")]).unwrap(),
        ))
    }

    #[test]
    fn rejects_all_reserved_names_even_with_builtin_features_disabled() {
        for reserved in RESERVED {
            assert!(StaticGameRegistry::new(vec![descriptor(reserved, "fixture-game")]).is_err());
            assert!(StaticGameRegistry::new(vec![descriptor("FixtureGame", reserved)]).is_err());
        }
    }
    #[test]
    fn rejects_duplicate_names_and_aliases_before_invoking_factories() {
        fn forbidden() -> TypeRegistry {
            panic!("invalid namespace must fail before callbacks")
        }
        let mut first = descriptor("FixtureGame", "fixture-game");
        first.types = forbidden;
        assert!(
            StaticGameRegistry::new(vec![first, descriptor("OtherGame", "FixtureGame")]).is_err()
        );
        assert!(StaticGameRegistry::new(vec![
            descriptor("Same", "same"),
            descriptor("Same", "other")
        ])
        .is_err());
        assert!(StaticGameRegistry::new(vec![
            descriptor("Same", "same"),
            descriptor("Other", "same")
        ])
        .is_err());
        assert!(StaticGameRegistry::new(vec![descriptor("", "alias")]).is_err());
        assert!(StaticGameRegistry::new(Vec::new()).is_err());
    }
    #[test]
    fn validated_handle_identity_is_registry_and_index_not_callbacks() {
        let a = registry();
        let b = registry();
        assert_eq!(
            a.handle("FixtureGame").unwrap(),
            a.handle("fixture-game").unwrap()
        );
        assert_ne!(
            a.handle("FixtureGame").unwrap(),
            b.handle("FixtureGame").unwrap()
        );
        assert!(a.handle("missing").is_err());
        assert_eq!(
            a.handle("FixtureGame")
                .unwrap()
                .expected_identity()
                .build_id,
            "0x0000000000001234"
        );
    }
    #[test]
    fn freezes_all_input_values_and_reflection_once() {
        static TYPES: AtomicUsize = AtomicUsize::new(0);
        static VALUES: AtomicUsize = AtomicUsize::new(0);
        fn types() -> TypeRegistry {
            TYPES.fetch_add(1, Ordering::SeqCst);
            scene_types()
        }
        fn values(k: StaticHeldKeys) -> J {
            VALUES.fetch_add(1, Ordering::SeqCst);
            held(k)
        }
        let mut d = descriptor("Frozen", "frozen");
        d.types = types;
        d.input.held = values;
        let r = Box::leak(Box::new(StaticGameRegistry::new(vec![d]).unwrap()));
        let game = r.handle("Frozen").unwrap();
        for _ in 0..3 {
            let _ = game.types();
            for x in -1..=1 {
                for y in -1..=1 {
                    for action in [false, true] {
                        let k = StaticHeldKeys { x, y, action };
                        assert_eq!(game.held_value(k), Some(held(k)));
                    }
                }
            }
        }
        assert_eq!(TYPES.load(Ordering::SeqCst), 1);
        assert_eq!(VALUES.load(Ordering::SeqCst), 18);
        assert_eq!(game.neutral_value(), held(StaticHeldKeys::default()));
        assert!(game
            .held_value(StaticHeldKeys {
                x: 2,
                y: 0,
                action: false
            })
            .is_none());
    }
    #[test]
    fn malformed_or_out_of_range_input_and_position_fail_admission() {
        let mut d = descriptor("BadValue", "bad-value");
        d.input.held = |_| json!({"axis_x":2,"axis_y":0,"buttons":[]});
        assert!(StaticGameRegistry::new(vec![d]).is_err());
        let mut d = descriptor("BadShape", "bad-shape");
        d.input.held = |_| json!({"method":"sim.start"});
        assert!(StaticGameRegistry::new(vec![d]).is_err());
        let mut d = descriptor("BadPosition", "bad-position");
        d.position.field = "absent";
        assert!(StaticGameRegistry::new(vec![d]).is_err());
        let mut d = descriptor("BadPlayers", "bad-players");
        d.input.max_players = 2;
        assert!(StaticGameRegistry::new(vec![d]).is_err());
    }
    #[test]
    fn host_spec_path_changes_retain_the_exact_registered_handle() {
        let game = registry().handle("FixtureGame").unwrap();
        let mut spec = crate::backend::HostSpec::local_game(
            "before.scene.yaml",
            crate::game::EditorGame::Registered(game),
        );
        assert!(spec.set_local_scene_path(PathBuf::from("after.scene.yaml")));
        match spec.clone() {
            crate::backend::HostSpec::LocalGame {
                scene,
                game: selected,
                ..
            } => {
                assert_eq!(scene, PathBuf::from("after.scene.yaml"));
                assert_eq!(selected, crate::game::EditorGame::Registered(game));
            }
            _ => panic!("registered host identity was replaced"),
        }
    }
}
