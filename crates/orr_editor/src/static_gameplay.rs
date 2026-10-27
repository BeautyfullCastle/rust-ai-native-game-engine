//! Validated, application-owned capabilities. This is not an editor handle,
//! command router, plugin sandbox or hot-reload mechanism.
use super::*;
use orr_reflect::{TypeInfo, TypeKind};
use std::sync::Arc;

const SCHEMA_BYTES: usize = 256 * 1024;
const VALUE_BYTES: usize = 64 * 1024;
const RECEIPT_BYTES: usize = 4096;
const CONTRACT_BYTES: usize = 4 * 1024 * 1024;

/// A bounded request, not an ERP envelope or an accepted command.
pub struct IntentRequest {
    name: String,
    value: J,
}
impl IntentRequest {
    pub fn new(name: impl Into<String>, value: J) -> Result<Self, String> {
        let name = name.into();
        if !super::name(&name) {
            return Err("invalid intent name".into());
        }
        bounded_json(&value, VALUE_BYTES)?;
        Ok(Self { name, value })
    }
}

/// Presentation receives only a borrowed view and may request one bounded intent.
/// This trusted native callback is not a security sandbox against arbitrary I/O.
pub trait EmbeddedGameplayView {
    fn show(
        &mut self,
        ui: &mut egui::Ui,
        frame: FrameView<'_>,
        context: GameplayViewContext,
    ) -> Result<Option<IntentRequest>, String>;
    fn reset(&mut self);
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameplayViewPhase {
    Edit,
    Paused,
    Live,
    Viewer,
}
#[derive(Clone, Copy, Debug)]
pub struct GameplayViewContext {
    pub phase: GameplayViewPhase,
    pub input_ready: bool,
}

pub type GameplayDraw = for<'a> fn(FrameView<'a>) -> Vec<Drawable>;

pub enum PresentationCapability {
    Spatial2d {
        position: StaticPosition,
        draw: GameplayDraw,
    },
    Embedded {
        create: fn() -> Box<dyn EmbeddedGameplayView>,
    },
}

/// An exact reflected input/intent type. Its factory runs once during admission.
pub struct GameplaySchema {
    pub type_name: &'static str,
    pub types: fn() -> TypeRegistry,
}
pub struct IntentSchema {
    pub name: &'static str,
    pub payload: GameplaySchema,
}

/// Correlation and lifecycle scope are supplied by the future router, not the codec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandScope {
    pub scene_epoch: u64,
    pub play_epoch: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct CommandContext {
    pub sequence: u64,
    pub scope: CommandScope,
    pub player: u8,
}
pub struct CodecOutput {
    pub input: J,
    pub receipt: J,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Consumption {
    Pending,
    Consumed,
    Invalid,
}

pub struct CommandContract {
    pub input: GameplaySchema,
    pub max_players: u8,
    pub neutral: J,
    pub max_sequence: u64,
    pub intents: Vec<IntentSchema>,
    pub encode: for<'a> fn(&str, &J, CommandContext, FrameView<'a>) -> Result<CodecOutput, String>,
    pub observe: for<'a> fn(FrameView<'a>, &J, CommandContext) -> Result<Consumption, String>,
}
pub enum InputCapability {
    Held(StaticInputDescriptor),
    Commands(CommandContract),
}

/// Host/stream callbacks use the existing typed startup contract. Admission does
/// not call them or the view/codec callbacks. Full scene identity is mandatory.
pub struct GameplayDescriptor {
    pub identity: RemoteIdentity,
    pub local_alias: &'static str,
    pub types: fn() -> TypeRegistry,
    pub host: HostFactory,
    pub stream: StreamFactory,
    pub presentation: PresentationCapability,
    pub input: InputCapability,
}

struct FrozenCommand {
    info: TypeInfo,
    intents: Vec<FrozenIntent>,
    contract: CommandContract,
}
struct FrozenIntent {
    id: String,
    info: TypeInfo,
    payload_schema: J,
}

/// Borrowed discovery metadata from the already validated, frozen catalog.
#[derive(Clone, Copy, Debug)]
pub struct IntentMetadata<'a> {
    pub id: &'a str,
    pub payload_schema: &'a J,
}
enum FrozenInput {
    Held {
        descriptor: StaticInputDescriptor,
        values: Vec<J>,
    },
    Commands(Box<FrozenCommand>),
}
struct Contract {
    identity: RemoteIdentity,
    alias: &'static str,
    types: TypeRegistry,
    host: HostFactory,
    stream: StreamFactory,
    presentation: PresentationCapability,
    expected_input: ExpectedInputDescriptor,
    neutral: J,
    input: FrozenInput,
}

/// Immutable admission result, deliberately distinct from `RegisteredGame`.
#[derive(Clone)]
pub struct ValidatedGameplay(Arc<Contract>);
pub struct ValidatedIntent {
    owner: Arc<Contract>,
    name: String,
    value: J,
}
pub struct PreparedGameplayCommand {
    owner: Arc<Contract>,
    context: CommandContext,
    queued_tick: u64,
    input: J,
    receipt: J,
}
impl PreparedGameplayCommand {
    pub fn context(&self) -> CommandContext {
        self.context
    }
    pub fn input(&self) -> &J {
        &self.input
    }
}

/// Namespace admission can include an existing legacy registry without changing it.
/// New entries cannot be opened as editor games until the common integration lands.
pub struct GameplayContracts {
    entries: Vec<ValidatedGameplay>,
}
impl GameplayContracts {
    pub fn new(
        descriptors: Vec<GameplayDescriptor>,
        legacy: Option<&StaticGameRegistry>,
    ) -> Result<Self, String> {
        let legacy_count = legacy.map_or(0, |registry| registry.entries.len());
        if descriptors.is_empty() || descriptors.len() + legacy_count > 32 {
            return Err("gameplay registry requires 1..=32 combined games".into());
        }
        let mut names = BTreeSet::new();
        if let Some(registry) = legacy {
            for entry in &registry.entries {
                names.insert(entry.descriptor.game_id.to_string());
                names.insert(entry.descriptor.local_alias.to_string());
            }
        }
        // Reject the complete namespace/identity syntax before invoking factories.
        for descriptor in &descriptors {
            if !descriptor.identity.schema.is_object() {
                return Err("gameplay scene schema must be an object".into());
            }
            bounded_json(&descriptor.identity.schema, SCHEMA_BYTES)?;
            for id in [descriptor.identity.game.as_str(), descriptor.local_alias] {
                if !name(id) || RESERVED.contains(&id) || !names.insert(id.to_string()) {
                    return Err("invalid, duplicate or reserved gameplay name".into());
                }
            }
            let build = descriptor.identity.build_id.strip_prefix("0x");
            if !build.is_some_and(|hex| {
                hex.len() == 16
                    && hex.bytes().all(|c| c.is_ascii_hexdigit())
                    && u64::from_str_radix(hex, 16).is_ok_and(|id| id != 0)
            }) {
                return Err("gameplay build identity must be a nonzero 64-bit hex value".into());
            }
        }
        let mut total = 0;
        let mut entries = Vec::new();
        for descriptor in descriptors {
            let types = (descriptor.types)();
            for info in types.types() {
                info.desc()
                    .check_layout(info.size())
                    .map_err(|error| error.to_string())?;
            }
            let schema = scene_schema(&types)?;
            account(&schema, SCHEMA_BYTES, &mut total)?;
            if descriptor.identity.schema != schema {
                return Err("gameplay full scene schema mismatch".into());
            }
            if let PresentationCapability::Spatial2d { position, .. } = &descriptor.presentation {
                let info = types
                    .get(position.component)
                    .ok_or("spatial position is not reflected")?;
                if info.kind() != TypeKind::Component
                    || !matches!(
                        info.get(&info.default_bytes(), position.field),
                        Ok(Value::Vec2(_))
                    )
                {
                    return Err("spatial position must be a reflected vec2 component field".into());
                }
            }
            let (input, expected_input, neutral) = match descriptor.input {
                InputCapability::Held(input) => {
                    if input.max_players != 1
                        || input.hint.is_empty()
                        || input.hint.len() > 256
                        || input.hint.chars().any(char::is_control)
                    {
                        return Err("invalid held input player count or hint".into());
                    }
                    let (info, schema) = freeze_schema(&GameplaySchema {
                        type_name: input.type_name,
                        types: input.types,
                    })?;
                    account(&schema, VALUE_BYTES, &mut total)?;
                    let expected =
                        ExpectedInputDescriptor::new(schema, orr_remote::methods::VALUE_FORMAT, 1)?;
                    let mut values = Vec::with_capacity(18);
                    for x in -1..=1 {
                        for y in -1..=1 {
                            for action in [false, true] {
                                let value = (input.held)(StaticHeldKeys { x, y, action });
                                check_value(&info, &value)?;
                                account(&value, VALUE_BYTES, &mut total)?;
                                values.push(value);
                            }
                        }
                    }
                    let neutral = values[8].clone();
                    (
                        FrozenInput::Held {
                            descriptor: input,
                            values,
                        },
                        expected,
                        neutral,
                    )
                }
                InputCapability::Commands(contract) => {
                    if contract.max_sequence == 0
                        || contract.intents.is_empty()
                        || contract.intents.len() > 32
                    {
                        return Err("commands require bounded sequence and 1..=32 intents".into());
                    }
                    let mut intent_names = BTreeSet::new();
                    for intent in &contract.intents {
                        if !name(intent.name) || !intent_names.insert(intent.name) {
                            return Err("invalid or duplicate intent name".into());
                        }
                    }
                    let (info, schema) = freeze_schema(&contract.input)?;
                    account(&schema, VALUE_BYTES, &mut total)?;
                    let expected = ExpectedInputDescriptor::new(
                        schema,
                        orr_remote::methods::VALUE_FORMAT,
                        contract.max_players,
                    )?;
                    account(&contract.neutral, VALUE_BYTES, &mut total)?;
                    check_value(&info, &contract.neutral)?;
                    let neutral = contract.neutral.clone();
                    let mut intents = Vec::new();
                    for intent in &contract.intents {
                        let (info, schema) = freeze_schema(&intent.payload)?;
                        account(&schema, VALUE_BYTES, &mut total)?;
                        account(&J::String(intent.name.into()), VALUE_BYTES, &mut total)?;
                        intents.push(FrozenIntent {
                            id: intent.name.to_string(),
                            info,
                            payload_schema: schema,
                        });
                    }
                    (
                        FrozenInput::Commands(Box::new(FrozenCommand {
                            info,
                            intents,
                            contract,
                        })),
                        expected,
                        neutral,
                    )
                }
            };
            account(
                &serde_json::json!({"game":descriptor.identity.game,"build":descriptor.identity.build_id,"alias":descriptor.local_alias}),
                VALUE_BYTES,
                &mut total,
            )?;
            entries.push(ValidatedGameplay(Arc::new(Contract {
                identity: descriptor.identity,
                alias: descriptor.local_alias,
                types,
                host: descriptor.host,
                stream: descriptor.stream,
                presentation: descriptor.presentation,
                expected_input,
                neutral,
                input,
            })));
        }
        Ok(Self { entries })
    }
    pub fn get(&self, id: &str) -> Option<ValidatedGameplay> {
        self.entries
            .iter()
            .find(|entry| entry.0.identity.game == id || entry.0.alias == id)
            .cloned()
    }
}

fn scene_schema(types: &TypeRegistry) -> Result<J, String> {
    let text = if cfg!(feature = "linked-prefabs") {
        types.json_schema()
    } else {
        types.legacy_json_schema()
    };
    serde_json::from_str(&text).map_err(|error| error.to_string())
}
fn freeze_schema(schema: &GameplaySchema) -> Result<(TypeInfo, J), String> {
    if !name(schema.type_name) {
        return Err("invalid reflected type name".into());
    }
    let types = (schema.types)();
    if types.types().count() != 1 {
        return Err("input/intent registry must contain exactly its type".into());
    }
    let info = types
        .get(schema.type_name)
        .ok_or("missing reflected input/intent type")?;
    if info.kind() != TypeKind::Component || info.size() > VALUE_BYTES {
        return Err("invalid input/intent layout".into());
    }
    info.desc()
        .check_layout(info.size())
        .map_err(|error| error.to_string())?;
    let value = types
        .type_json_schema(schema.type_name)
        .ok_or("missing typed schema")?;
    Ok((
        info.clone(),
        serde_json::from_str(&value).map_err(|error| error.to_string())?,
    ))
}
fn account(value: &J, limit: usize, total: &mut usize) -> Result<(), String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| error.to_string())?
        .len();
    *total = total.checked_add(bytes).ok_or("gameplay budget overflow")?;
    if bytes > limit || *total > CONTRACT_BYTES {
        return Err("gameplay JSON budget exceeded".into());
    }
    Ok(())
}
fn decode(info: &TypeInfo, value: &J) -> Result<Vec<u8>, String> {
    bounded_json(value, VALUE_BYTES)?;
    let decoded = orr_remote::json::json_to_value(info.desc(), value, false)?;
    let mut bytes = info.default_bytes();
    info.set(&mut bytes, "", decoded)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}
fn check_value(info: &TypeInfo, value: &J) -> Result<(), String> {
    decode(info, value).map(|_| ())
}

impl ValidatedGameplay {
    pub fn identity(&self) -> &RemoteIdentity {
        &self.0.identity
    }
    pub fn types(&self) -> &TypeRegistry {
        &self.0.types
    }
    pub fn expected_input(&self) -> &ExpectedInputDescriptor {
        &self.0.expected_input
    }
    pub fn neutral_value(&self) -> &J {
        &self.0.neutral
    }
    pub fn host_factory(&self) -> HostFactory {
        self.0.host
    }
    pub fn stream_factory(&self) -> StreamFactory {
        self.0.stream
    }
    pub fn spatial(&self) -> Option<(StaticPosition, GameplayDraw)> {
        match self.0.presentation {
            PresentationCapability::Spatial2d { position, draw } => Some((position, draw)),
            PresentationCapability::Embedded { .. } => None,
        }
    }
    /// Explicit later operation; this factory never runs during admission.
    pub fn create_view(&self) -> Option<Box<dyn EmbeddedGameplayView>> {
        match self.0.presentation {
            PresentationCapability::Embedded { create } => Some(create()),
            PresentationCapability::Spatial2d { .. } => None,
        }
    }
    pub fn held_value(&self, keys: StaticHeldKeys) -> Option<&J> {
        match &self.0.input {
            FrozenInput::Held { values, .. } => keys.index().map(|index| &values[index]),
            FrozenInput::Commands(_) => None,
        }
    }
    pub fn input_hint(&self) -> Option<&'static str> {
        match &self.0.input {
            FrozenInput::Held { descriptor, .. } => Some(descriptor.hint),
            FrozenInput::Commands(_) => None,
        }
    }
    /// Does not allocate, regenerate schemas, or execute application callbacks.
    pub fn intent_catalog(&self) -> impl ExactSizeIterator<Item = IntentMetadata<'_>> {
        let intents: &[FrozenIntent] = match &self.0.input {
            FrozenInput::Held { .. } => &[],
            FrozenInput::Commands(command) => &command.intents,
        };
        intents.iter().map(|intent| IntentMetadata {
            id: &intent.id,
            payload_schema: &intent.payload_schema,
        })
    }
    pub fn command_max_sequence(&self) -> Option<u64> {
        match &self.0.input {
            FrozenInput::Held { .. } => None,
            FrozenInput::Commands(command) => Some(command.contract.max_sequence),
        }
    }
    pub fn validate_intent(&self, request: IntentRequest) -> Result<ValidatedIntent, String> {
        let FrozenInput::Commands(command) = &self.0.input else {
            return Err("this game has no command capability".into());
        };
        let intent = command
            .intents
            .iter()
            .find(|intent| intent.id == request.name)
            .ok_or("unknown command intent")?;
        check_value(&intent.info, &request.value)?;
        Ok(ValidatedIntent {
            owner: self.0.clone(),
            name: request.name,
            value: request.value,
        })
    }
    pub fn encode(
        &self,
        intent: &ValidatedIntent,
        context: CommandContext,
        frame: FrameView<'_>,
    ) -> Result<PreparedGameplayCommand, String> {
        if !Arc::ptr_eq(&self.0, &intent.owner) {
            return Err("intent belongs to a different validated contract".into());
        }
        let FrozenInput::Commands(command) = &self.0.input else {
            return Err("this game has no command capability".into());
        };
        if context.sequence == 0
            || context.sequence > command.contract.max_sequence
            || context.player >= command.contract.max_players
        {
            return Err("command sequence or player is out of bounds".into());
        }
        let output = (command.contract.encode)(&intent.name, &intent.value, context, frame)?;
        check_value(&command.info, &output.input)?;
        bounded_json(&output.receipt, RECEIPT_BYTES)?;
        Ok(PreparedGameplayCommand {
            owner: self.0.clone(),
            context,
            queued_tick: frame.tick(),
            input: output.input,
            receipt: output.receipt,
        })
    }
    /// The caller must first validate the actual managed-input ACK. This helper
    /// checks receipt scope/tick ordering; it does not authenticate an RPC ACK.
    pub fn observe(
        &self,
        command: &PreparedGameplayCommand,
        scope: CommandScope,
        accepted_head_tick: u64,
        frame: FrameView<'_>,
    ) -> Result<Consumption, String> {
        if !Arc::ptr_eq(&self.0, &command.owner) {
            return Err("command belongs to a different validated contract".into());
        }
        if command.context.scope != scope || accepted_head_tick < command.queued_tick {
            return Ok(Consumption::Invalid);
        }
        if frame.tick() <= accepted_head_tick {
            return Ok(Consumption::Pending);
        }
        let FrozenInput::Commands(codec) = &self.0.input else {
            return Err("this game has no command capability".into());
        };
        (codec.contract.observe)(frame, &command.receipt, command.context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::{Pod, Zeroable};
    use orr_reflect::Reflect;
    use serde_json::json;

    #[repr(C)]
    #[derive(Clone, Copy, Default, Pod, Zeroable, Reflect)]
    struct Input {
        #[reflect(range = "0..=10")]
        value: u32,
        sequence: u32,
    }
    #[repr(C)]
    #[derive(Clone, Copy, Default, Pod, Zeroable, Reflect)]
    struct Payload {
        #[reflect(range = "0..=10")]
        value: u32,
    }
    fn input_types() -> TypeRegistry {
        let mut types = TypeRegistry::new();
        types.register_component::<Input>("Input");
        types
    }
    fn payload_types() -> TypeRegistry {
        let mut types = TypeRegistry::new();
        types.register_component::<Payload>("Payload");
        types
    }
    fn empty_types() -> TypeRegistry {
        TypeRegistry::new()
    }
    fn forbidden_types() -> TypeRegistry {
        panic!("invalid namespace must fail before schema callbacks")
    }
    fn no_host(_: StaticHostRequest) -> Result<LocalHost, String> {
        panic!("admission must not call host")
    }
    fn no_stream(
        _: Box<dyn Transport>,
        _: RemoteConfig,
        _: ExpectedInputDescriptor,
    ) -> Result<Box<dyn StaticFrameStream>, String> {
        panic!("admission must not call stream")
    }
    fn no_view() -> Box<dyn EmbeddedGameplayView> {
        panic!("admission must not create view")
    }
    fn no_draw(_: FrameView<'_>) -> Vec<Drawable> {
        panic!("admission must not draw")
    }
    fn encode(
        _: &str,
        value: &J,
        context: CommandContext,
        _: FrameView<'_>,
    ) -> Result<CodecOutput, String> {
        Ok(CodecOutput {
            input: json!({"value":value["value"],"sequence":context.sequence}),
            receipt: json!({"expected":value["value"],"player":context.player}),
        })
    }
    fn observe(
        _: FrameView<'_>,
        receipt: &J,
        context: CommandContext,
    ) -> Result<Consumption, String> {
        if receipt["expected"] == 3 && receipt["player"] == context.player && context.player == 1 {
            Ok(Consumption::Consumed)
        } else {
            Ok(Consumption::Pending)
        }
    }
    fn descriptor(id: &str, alias: &'static str) -> GameplayDescriptor {
        GameplayDescriptor {
            identity: RemoteIdentity {
                game: id.into(),
                build_id: "0x0000000000001234".into(),
                schema: scene_schema(&empty_types()).unwrap(),
            },
            local_alias: alias,
            types: empty_types,
            host: no_host,
            stream: no_stream,
            presentation: PresentationCapability::Embedded { create: no_view },
            input: InputCapability::Commands(CommandContract {
                input: GameplaySchema {
                    type_name: "Input",
                    types: input_types,
                },
                max_players: 2,
                neutral: json!({"value":0,"sequence":0}),
                max_sequence: u64::from(u32::MAX),
                intents: vec![IntentSchema {
                    name: "set",
                    payload: GameplaySchema {
                        type_name: "Payload",
                        types: payload_types,
                    },
                }],
                encode,
                observe,
            }),
        }
    }
    fn fixture_game() -> ValidatedGameplay {
        GameplayContracts::new(vec![descriptor("Fixture", "fixture")], None)
            .unwrap()
            .get("fixture")
            .unwrap()
    }
    fn frame(tick: u64) -> orr_ecs::Frame {
        let mut frame = orr_ecs::Frame::new(orr_ecs::ComponentRegistryBuilder::new().build());
        frame.set_tick(tick);
        frame
    }
    fn context() -> CommandContext {
        CommandContext {
            sequence: 1,
            scope: CommandScope {
                scene_epoch: 2,
                play_epoch: 3,
            },
            player: 1,
        }
    }

    #[test]
    fn embedded_without_fake_position_freezes_identity_and_neutral_without_startup() {
        fn forbidden_encode(
            _: &str,
            _: &J,
            _: CommandContext,
            _: FrameView<'_>,
        ) -> Result<CodecOutput, String> {
            panic!("admission must not encode")
        }
        fn forbidden_observe(
            _: FrameView<'_>,
            _: &J,
            _: CommandContext,
        ) -> Result<Consumption, String> {
            panic!("admission must not observe")
        }
        let mut value = descriptor("Fixture", "fixture");
        let InputCapability::Commands(command) = &mut value.input else {
            unreachable!()
        };
        command.encode = forbidden_encode;
        command.observe = forbidden_observe;
        let game = GameplayContracts::new(vec![value], None)
            .unwrap()
            .get("fixture")
            .unwrap();
        assert_eq!(game.identity().game, "Fixture");
        assert!(game.spatial().is_none());
        assert_eq!(game.neutral_value(), &json!({"value":0,"sequence":0}));
        assert_eq!(game.expected_input().max_players(), 2);
        let _host = game.host_factory();
        let _stream = game.stream_factory();
        assert_eq!(game.types().types().count(), 0);
    }

    #[test]
    fn namespace_and_build_identity_fail_before_any_schema_factory() {
        for (id, alias, build) in [
            ("Fixture", "fixture", "0x0000000000000000"),
            ("Arena", "fixture", "0x0000000000001234"),
            ("Fixture", "Fixture", "0x0000000000001234"),
            ("Fixture", "fixture", "1234"),
        ] {
            let mut value = descriptor(id, alias);
            value.types = forbidden_types;
            value.identity.build_id = build.into();
            assert!(GameplayContracts::new(vec![value], None).is_err());
        }
        let mut a = descriptor("Fixture", "fixture");
        a.types = forbidden_types;
        assert!(GameplayContracts::new(vec![a, descriptor("Other", "fixture")], None).is_err());
        let legacy = Box::leak(Box::new(
            StaticGameRegistry::new(vec![super::super::tests::descriptor("Legacy", "legacy")])
                .unwrap(),
        ));
        let mut value = descriptor("Legacy", "fixture");
        value.types = forbidden_types;
        assert!(GameplayContracts::new(vec![value], Some(legacy)).is_err());
        assert_eq!(legacy.handle("legacy").unwrap().name(), "Legacy");
    }

    #[test]
    fn scene_schema_mismatch_and_spatial_missing_or_non_vec2_fail_closed() {
        let mut value = descriptor("Fixture", "fixture");
        value.identity.schema = json!({"wrong":true});
        assert!(GameplayContracts::new(vec![value], None).is_err());
        let mut value = descriptor("Fixture", "fixture");
        value.presentation = PresentationCapability::Spatial2d {
            position: StaticPosition {
                component: "Missing",
                field: "pos",
            },
            draw: no_draw,
        };
        assert!(GameplayContracts::new(vec![value], None).is_err());
        let mut value = descriptor("Fixture", "fixture");
        value.types = payload_types;
        value.identity.schema = scene_schema(&payload_types()).unwrap();
        value.presentation = PresentationCapability::Spatial2d {
            position: StaticPosition {
                component: "Payload",
                field: "value",
            },
            draw: no_draw,
        };
        assert!(GameplayContracts::new(vec![value], None).is_err());
    }

    #[test]
    fn spatial_and_held_values_keep_the_legacy_contract() {
        let legacy = super::super::tests::descriptor("Legacy", "legacy");
        let schema = scene_schema(&(legacy.types)()).unwrap();
        let game = GameplayContracts::new(
            vec![GameplayDescriptor {
                identity: RemoteIdentity {
                    game: "Spatial".into(),
                    build_id: "0x0000000000001234".into(),
                    schema,
                },
                local_alias: "spatial",
                types: legacy.types,
                host: legacy.host,
                stream: legacy.stream,
                presentation: PresentationCapability::Spatial2d {
                    position: legacy.position,
                    draw: legacy.draw,
                },
                input: InputCapability::Held(legacy.input),
            }],
            None,
        )
        .unwrap()
        .get("spatial")
        .unwrap();
        assert_eq!(game.spatial().unwrap().0.component, "Position");
        assert!(game.create_view().is_none());
        assert_eq!(game.intent_catalog().len(), 0);
        assert_eq!(game.command_max_sequence(), None);
        assert_eq!(
            game.held_value(StaticHeldKeys::default()),
            Some(game.neutral_value())
        );
        assert!(game
            .held_value(StaticHeldKeys {
                x: 2,
                ..Default::default()
            })
            .is_none());
        assert!(game
            .validate_intent(IntentRequest::new("set", json!({"value":3})).unwrap())
            .is_err());
    }

    #[test]
    fn malformed_unknown_and_oversized_intents_are_rejected_before_codec() {
        let game = fixture_game();
        for (name, value) in [
            ("unknown", json!({"value":3})),
            ("set", json!({"value":11})),
            ("set", json!({"value":"bad"})),
            ("set", json!({"value":3,"extra":1})),
            ("set", J::Null),
        ] {
            assert!(game
                .validate_intent(IntentRequest::new(name, value).unwrap())
                .is_err());
        }
        assert!(IntentRequest::new("set", J::String("x".repeat(VALUE_BYTES))).is_err());
        assert!(IntentRequest::new("", J::Null).is_err());
    }

    #[test]
    fn command_catalog_borrows_exact_multiple_payload_schemas_and_sequence_limits() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static SCHEMAS: AtomicUsize = AtomicUsize::new(0);
        fn counted_payload() -> TypeRegistry {
            SCHEMAS.fetch_add(1, Ordering::SeqCst);
            payload_types()
        }
        fn no_encode(
            _: &str,
            _: &J,
            _: CommandContext,
            _: FrameView<'_>,
        ) -> Result<CodecOutput, String> {
            panic!("metadata queries must not encode")
        }
        fn no_observe(_: FrameView<'_>, _: &J, _: CommandContext) -> Result<Consumption, String> {
            panic!("metadata queries must not observe")
        }
        let expected_payload: J =
            serde_json::from_str(&payload_types().type_json_schema("Payload").unwrap()).unwrap();
        let expected_input: J =
            serde_json::from_str(&input_types().type_json_schema("Input").unwrap()).unwrap();
        for limit in [1, u64::MAX] {
            SCHEMAS.store(0, Ordering::SeqCst);
            let mut value = descriptor("Fixture", "fixture");
            let InputCapability::Commands(command) = &mut value.input else {
                unreachable!()
            };
            command.max_sequence = limit;
            command.intents[0].payload.types = counted_payload;
            command.intents.push(IntentSchema {
                name: "adjust",
                payload: GameplaySchema {
                    type_name: "Input",
                    types: input_types,
                },
            });
            command.encode = no_encode;
            command.observe = no_observe;
            let game = GameplayContracts::new(vec![value], None)
                .unwrap()
                .get("fixture")
                .unwrap();
            assert_eq!(SCHEMAS.load(Ordering::SeqCst), 1);
            let mut catalog = game.intent_catalog();
            assert_eq!(catalog.len(), 2);
            let first = catalog.next().unwrap();
            assert_eq!(first.id, "set");
            assert_eq!(first.payload_schema, &expected_payload);
            let second = catalog.next().unwrap();
            assert_eq!(second.id, "adjust");
            assert_eq!(second.payload_schema, &expected_input);
            assert_eq!(catalog.len(), 0);
            assert!(catalog.next().is_none());
            assert_eq!(game.command_max_sequence(), Some(limit));
            assert_eq!(game.intent_catalog().len(), 2);
            assert_eq!(SCHEMAS.load(Ordering::SeqCst), 1);
            assert!(game
                .validate_intent(
                    IntentRequest::new("adjust", json!({"value":3,"sequence":1})).unwrap()
                )
                .is_ok());
            assert!(game
                .validate_intent(IntentRequest::new("adjust", json!({"value":3})).unwrap())
                .is_err());
        }
    }

    #[test]
    fn command_sequence_player_tokens_and_runtime_output_are_validated() {
        let game = fixture_game();
        let other = fixture_game();
        let intent = game
            .validate_intent(IntentRequest::new("set", json!({"value":3})).unwrap())
            .unwrap();
        let frame = frame(5);
        assert!(other
            .encode(&intent, context(), FrameView::of(&frame))
            .is_err());
        for bad in [
            CommandContext {
                sequence: 0,
                ..context()
            },
            CommandContext {
                sequence: u64::MAX,
                ..context()
            },
            CommandContext {
                player: 2,
                ..context()
            },
        ] {
            assert!(game.encode(&intent, bad, FrameView::of(&frame)).is_err());
        }
        let prepared = game
            .encode(&intent, context(), FrameView::of(&frame))
            .unwrap();
        assert_eq!(prepared.input(), &json!({"value":3,"sequence":1}));
        assert_eq!(prepared.context().player, 1);
        assert!(other
            .observe(&prepared, context().scope, 5, FrameView::of(&frame))
            .is_err());
    }

    #[test]
    fn observation_fences_scope_and_tick_before_game_specific_receipt() {
        let game = fixture_game();
        let before = frame(5);
        let intent = game
            .validate_intent(IntentRequest::new("set", json!({"value":3})).unwrap())
            .unwrap();
        let command = game
            .encode(&intent, context(), FrameView::of(&before))
            .unwrap();
        assert_eq!(
            game.observe(&command, context().scope, 5, FrameView::of(&before))
                .unwrap(),
            Consumption::Pending
        );
        assert_eq!(
            game.observe(&command, context().scope, 4, FrameView::of(&before))
                .unwrap(),
            Consumption::Invalid
        );
        let after = frame(6);
        assert_eq!(
            game.observe(
                &command,
                CommandScope {
                    play_epoch: 4,
                    ..context().scope
                },
                5,
                FrameView::of(&after)
            )
            .unwrap(),
            Consumption::Invalid
        );
        assert_eq!(
            game.observe(&command, context().scope, 5, FrameView::of(&after))
                .unwrap(),
            Consumption::Consumed
        );
    }

    #[test]
    fn duplicate_intents_bad_schema_neutral_players_and_limits_fail_admission() {
        for change in 0..6 {
            let mut value = descriptor("Fixture", "fixture");
            let InputCapability::Commands(command) = &mut value.input else {
                unreachable!()
            };
            match change {
                0 => command.intents.push(IntentSchema {
                    name: "set",
                    payload: GameplaySchema {
                        type_name: "Payload",
                        types: payload_types,
                    },
                }),
                1 => command.input.type_name = "Missing",
                2 => command.neutral = json!({"value":11,"sequence":0}),
                3 => command.max_players = 0,
                4 => command.max_sequence = 0,
                _ => command.intents.clear(),
            }
            assert!(GameplayContracts::new(vec![value], None).is_err());
        }
    }

    #[test]
    fn malformed_or_oversized_codec_outputs_fail_before_preparation() {
        fn wrong(
            _: &str,
            _: &J,
            _: CommandContext,
            _: FrameView<'_>,
        ) -> Result<CodecOutput, String> {
            Ok(CodecOutput {
                input: json!({"value":11,"sequence":1}),
                receipt: J::Null,
            })
        }
        fn huge(
            _: &str,
            _: &J,
            _: CommandContext,
            _: FrameView<'_>,
        ) -> Result<CodecOutput, String> {
            Ok(CodecOutput {
                input: json!({"value":3,"sequence":1}),
                receipt: J::String("x".repeat(RECEIPT_BYTES)),
            })
        }
        for encode in [
            wrong
                as for<'a> fn(
                    &str,
                    &J,
                    CommandContext,
                    FrameView<'a>,
                ) -> Result<CodecOutput, String>,
            huge,
        ] {
            let mut value = descriptor("Fixture", "fixture");
            let InputCapability::Commands(command) = &mut value.input else {
                unreachable!()
            };
            command.encode = encode;
            let game = GameplayContracts::new(vec![value], None)
                .unwrap()
                .get("fixture")
                .unwrap();
            let intent = game
                .validate_intent(IntentRequest::new("set", json!({"value":3})).unwrap())
                .unwrap();
            assert!(game
                .encode(&intent, context(), FrameView::of(&frame(0)))
                .is_err());
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default, Pod, Zeroable)]
    struct LargeDoc {
        value: u32,
        sequence: u32,
    }
    impl Reflect for LargeDoc {
        fn describe() -> orr_reflect::TypeDesc {
            let mut desc = Input::describe();
            desc.doc = "x".repeat(SCHEMA_BYTES);
            desc
        }
    }
    #[repr(C)]
    #[derive(Clone, Copy, Default, Pod, Zeroable)]
    struct BadLayout {
        value: u32,
    }
    impl Reflect for BadLayout {
        fn describe() -> orr_reflect::TypeDesc {
            Input::describe()
        }
    }

    #[test]
    fn oversized_schema_is_rejected_and_registry_checks_layout_upstream() {
        fn large() -> TypeRegistry {
            let mut types = TypeRegistry::new();
            types.register_component::<LargeDoc>("Large");
            types
        }
        fn bad() -> TypeRegistry {
            let mut types = TypeRegistry::new();
            types.register_component::<BadLayout>("Bad");
            types
        }
        let mut value = descriptor("Fixture", "fixture");
        value.types = large;
        value.identity.schema = scene_schema(&large()).unwrap();
        assert!(GameplayContracts::new(vec![value], None).is_err());
        // TypeRegistry rejects invalid layouts while constructing the registry;
        // this is upstream rejection, not an admission Err from this API.
        assert!(std::panic::catch_unwind(bad).is_err());
        let mut value = descriptor("Fixture", "fixture");
        let InputCapability::Commands(command) = &mut value.input else {
            unreachable!()
        };
        command.input = GameplaySchema {
            type_name: "Large",
            types: large,
        };
        assert!(GameplayContracts::new(vec![value], None).is_err());
    }

    #[test]
    fn supplied_nonzero_neutral_is_frozen_without_claiming_typed_default_equivalence() {
        let mut value = descriptor("Fixture", "fixture");
        let InputCapability::Commands(command) = &mut value.input else {
            unreachable!()
        };
        command.neutral = json!({"value":7,"sequence":9});
        let game = GameplayContracts::new(vec![value], None)
            .unwrap()
            .get("fixture")
            .unwrap();
        assert_eq!(game.neutral_value(), &json!({"value":7,"sequence":9}));
        let mut value = descriptor("HeldGame", "held-game");
        value.input = InputCapability::Held(StaticInputDescriptor {
            type_name: "Input",
            types: input_types,
            max_players: 1,
            hint: "nonzero neutral",
            held: |_| json!({"value":7,"sequence":9}),
        });
        let game = GameplayContracts::new(vec![value], None)
            .unwrap()
            .get("held-game")
            .unwrap();
        assert_eq!(game.neutral_value(), &json!({"value":7,"sequence":9}));
    }

    #[test]
    fn declared_oversized_identity_fails_before_schema_factory() {
        let mut value = descriptor("Fixture", "fixture");
        value.types = forbidden_types;
        value.identity.schema = json!({"oversized":"x".repeat(SCHEMA_BYTES)});
        assert!(GameplayContracts::new(vec![value], None).is_err());
    }

    #[test]
    fn factory_runs_only_explicitly_and_view_context_does_not_grant_input_authority() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static CREATED: AtomicUsize = AtomicUsize::new(0);
        struct View;
        impl EmbeddedGameplayView for View {
            fn show(
                &mut self,
                _: &mut egui::Ui,
                _: FrameView<'_>,
                context: GameplayViewContext,
            ) -> Result<Option<IntentRequest>, String> {
                if context.phase == GameplayViewPhase::Live && context.input_ready {
                    Ok(Some(IntentRequest::new("set", json!({"value":3}))?))
                } else {
                    Ok(None)
                }
            }
            fn reset(&mut self) {}
        }
        fn create() -> Box<dyn EmbeddedGameplayView> {
            CREATED.fetch_add(1, Ordering::SeqCst);
            Box::new(View)
        }
        CREATED.store(0, Ordering::SeqCst);
        let mut value = descriptor("Fixture", "fixture");
        value.presentation = PresentationCapability::Embedded { create };
        let game = GameplayContracts::new(vec![value], None)
            .unwrap()
            .get("fixture")
            .unwrap();
        assert_eq!(CREATED.load(Ordering::SeqCst), 0);
        let mut view = game.create_view().unwrap();
        assert_eq!(CREATED.load(Ordering::SeqCst), 1);
        let frame = frame(0);
        let before = frame.to_bytes();
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            for phase in [
                GameplayViewPhase::Edit,
                GameplayViewPhase::Paused,
                GameplayViewPhase::Viewer,
            ] {
                assert!(view
                    .show(
                        ui,
                        FrameView::of(&frame),
                        GameplayViewContext {
                            phase,
                            input_ready: false
                        }
                    )
                    .unwrap()
                    .is_none());
            }
            let request = view
                .show(
                    ui,
                    FrameView::of(&frame),
                    GameplayViewContext {
                        phase: GameplayViewPhase::Live,
                        input_ready: true,
                    },
                )
                .unwrap()
                .unwrap();
            assert!(game.validate_intent(request).is_ok());
        });
        view.reset();
        assert_eq!(frame.to_bytes(), before);
    }

    #[test]
    fn empty_and_over_capacity_sets_fail_before_factories() {
        assert!(GameplayContracts::new(vec![], None).is_err());
        let values = (0..33)
            .map(|index| {
                let alias: &'static str = Box::leak(format!("alias_{index}").into_boxed_str());
                let mut value = descriptor(&format!("Game_{index}"), alias);
                value.types = forbidden_types;
                value
            })
            .collect();
        assert!(GameplayContracts::new(values, None).is_err());
    }
}
