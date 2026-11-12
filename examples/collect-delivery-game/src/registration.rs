//! Application-owned registration on the existing static gameplay contracts.
use std::sync::OnceLock;

use bytemuck::{Pod, Zeroable};
use orr_bridge::{Bridge, BridgeEvent, FrameView, ViewUpdate};
use orr_editor::static_adapter::{
    CodecOutput, CommandContext, CommandContract, Consumption, GameplayContracts,
    GameplayDescriptor, GameplaySchema, InputCapability, IntentSchema, PresentationCapability,
    StaticFrameStream, StaticHostRequest, StaticPosition,
};
use orr_fp::{FP, FPVec2};
use orr_reflect::{Reflect, TypeRegistry};
use orr_remote::{
    ExpectedInputDescriptor, GameHooks, LocalHost, RemoteBridge, RemoteConfig, RemoteIdentity,
    RemoteViewDelivery, RpcError, Transport,
};
use orr_sim::Game;
use orr_static_starter::game_contract::Starter;
use serde_json::{Value as J, json};

use crate::game::{
    CollectDeliveryGame, CollectDeliveryState, Courier, MODE_NAVIGATION_PULSE, MODE_PRIMARY,
};

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable, Reflect)]
struct Act {
    #[reflect(range = "1..=1")]
    press: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable, Reflect)]
struct Move {
    #[reflect(range = "-1..=1")]
    dx: i32,
    #[reflect(range = "-1..=1")]
    dy: i32,
}

fn input_types() -> TypeRegistry {
    orr_static_starter::game_contract::inputs::<CollectDeliveryGame>()
}

fn act_types() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    types.register_component::<Act>("Act");
    types
}

fn move_types() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    types.register_component::<Move>("Move");
    types
}

fn host(request: StaticHostRequest) -> Result<LocalHost, String> {
    let input_schema = input_types()
        .type_json_schema(CollectDeliveryGame::INPUT)
        .ok_or("missing compiled input schema")?;
    let input_schema = serde_json::from_str(&input_schema).map_err(|error| error.to_string())?;
    let expected =
        ExpectedInputDescriptor::new(input_schema, orr_remote::methods::VALUE_FORMAT, 1)?;
    if request.expected_input != expected {
        return Err("collect-delivery input contract mismatch".into());
    }

    let mut config = request.config;
    let scene_text = request.scene_text;
    let scene_path = request.scene_path;
    LocalHost::spawn_with_input::<CollectDeliveryGame>(
        move || {
            let document =
                orr_static_starter::game_contract::document::<CollectDeliveryGame>(&scene_text)?;
            config.limits.game = GameHooks::new(CollectDeliveryGame::ID);
            config.limits.build_id = CollectDeliveryGame::BUILD;
            config.limits.player_count = 1;
            config.limits.tick_rate = orr_static_starter::TICK_RATE;
            config.limits.scene_path = Some(scene_path);
            config.limits.view_stream = None;
            Ok((document, config))
        },
        CollectDeliveryGame::INPUT,
        1,
        |_, _| Vec::new(),
        request.neutral_value,
        Some(request.work_status),
    )
}

struct Stream<G: Game>(RemoteBridge<G>);

impl<G: Game> StaticFrameStream for Stream<G> {
    fn poll_view(&mut self) -> ViewUpdate<()> {
        let update = self.0.poll_view();
        ViewUpdate {
            snapshot: update.snapshot,
            resync: update.resync,
            events: update
                .events
                .into_iter()
                .filter_map(|event| match event {
                    BridgeEvent::Lifecycle(lifecycle) => Some(BridgeEvent::Lifecycle(lifecycle)),
                    _ => None,
                })
                .collect(),
        }
    }

    fn take_errors(&self) -> Vec<RpcError> {
        self.0.take_errors()
    }

    fn is_alive(&self) -> bool {
        self.0.is_alive()
    }

    fn view_delivery(&self) -> RemoteViewDelivery {
        self.0.view_delivery()
    }

    fn frame_evidence(&self) -> Option<orr_remote::RemoteFrameEvidence> {
        self.0.frame_evidence()
    }
}

fn stream(
    transport: Box<dyn Transport>,
    config: RemoteConfig,
    input: ExpectedInputDescriptor,
) -> Result<Box<dyn StaticFrameStream>, String> {
    Ok(Box::new(Stream(
        RemoteBridge::<CollectDeliveryGame>::connect_transport_with_input(
            transport, config, input,
        )?,
    )))
}

fn draw(frame: FrameView<'_>) -> Vec<orr_editor::game::Drawable> {
    use orr_static_starter::presentation::StandaloneGame;

    let mut items = Vec::new();
    CollectDeliveryGame::extract(frame, &mut items);
    items
        .into_iter()
        .map(|item| orr_editor::game::Drawable {
            entity: item.entity,
            pos: [item.transform.pos.x, item.transform.pos.y],
            angle: item.transform.rot,
            turn: 0.0,
            style: item.style,
            outline: orr_sample::editor_view::Outline::Circle {
                radius: item.style.size,
            },
        })
        .collect()
}

fn state_json(position: FPVec2, state: &CollectDeliveryState) -> J {
    let pair = |point: FPVec2| [point.x.raw(), point.y.raw()];
    json!({
        "position": pair(position),
        "start": pair(state.start),
        "pickup": pair(state.pickup),
        "destination": pair(state.destination),
        "hazard": pair(state.hazard),
        "item_available": state.item_available,
        "carrying": state.carrying,
        "won": state.won,
        "lost": state.lost,
        "reserved": state.reserved,
        "action_held": state.action_held
    })
}

fn exact_fields(value: &J, fields: &[&str]) -> bool {
    value.as_object().is_some_and(|object| {
        object.len() == fields.len() && fields.iter().all(|field| object.contains_key(*field))
    })
}

fn number(value: &J) -> Option<i64> {
    if !value.is_number() {
        return None;
    }
    match orr_remote::json::json_to_value(&i64::describe(), value, false).ok()? {
        orr_reflect::Value::Int(value) => i64::try_from(value).ok(),
        _ => None,
    }
}

fn axis(value: &J) -> Result<i32, String> {
    number(value)
        .filter(|value| (-1..=1).contains(value))
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| "movement requires integer axes in -1..=1".into())
}

fn advance(position: FPVec2, dx: i32, dy: i32) -> Result<FPVec2, String> {
    let x = position.x.raw().checked_add(i64::from(dx) * (1 << 16));
    let y = position.y.raw().checked_add(i64::from(dy) * (1 << 16));
    let (Some(x), Some(y)) = (x, y) else {
        return Err("movement exceeds the fixed-point coordinate range".into());
    };
    Ok(FPVec2::new(FP::from_raw(x), FP::from_raw(y)))
}

fn encode(
    name: &str,
    payload: &J,
    context: CommandContext,
    frame: FrameView<'_>,
) -> Result<CodecOutput, String> {
    let (actors, couriers) = frame.dense::<Courier>();
    if context.player != 0 || couriers.len() != 1 {
        return Err("collect-delivery commands require the player-zero courier".into());
    }
    let mut state = *frame.singleton::<CollectDeliveryState>();
    if state.action_held != 0 {
        return Err("release the previous action before another command".into());
    }
    let position = couriers[0].position;
    let before = state_json(position, &state);
    let mut after_position = position;
    let input = match name {
        "act" if exact_fields(payload, &["press"]) && number(&payload["press"]) == Some(1) => {
            state.action_held = MODE_PRIMARY;
            if state.won != 0 || state.lost != 0 {
                after_position = state.start;
                state.item_available = 1;
                state.carrying = 0;
                state.won = 0;
                state.lost = 0;
            } else if position == state.hazard {
                state.lost = 1;
            } else if state.item_available != 0 && position == state.pickup {
                state.item_available = 0;
                state.carrying = 1;
            } else if state.carrying != 0 && position == state.destination {
                state.won = 1;
            }
            json!({"dx":0,"dy":0,"action":MODE_PRIMARY})
        }
        "move" if exact_fields(payload, &["dx", "dy"]) => {
            let dx = axis(&payload["dx"])?;
            let dy = axis(&payload["dy"])?;
            if dx == 0 && dy == 0 || state.won != 0 || state.lost != 0 {
                return Err("move requires a nonzero direction before a terminal outcome".into());
            }
            after_position = advance(position, dx, dy)?;
            state.action_held = MODE_NAVIGATION_PULSE;
            if after_position == state.hazard {
                state.lost = 1;
            }
            json!({"dx":dx,"dy":dy,"action":MODE_NAVIGATION_PULSE})
        }
        _ => return Err("invalid collect-delivery intent or payload".into()),
    };
    let expected = state_json(after_position, &state);
    Ok(CodecOutput {
        input,
        receipt: json!({
            "kind": name,
            "player": context.player,
            "actor": [actors[0].index, actors[0].version],
            "scope": [context.scope.scene_epoch, context.scope.play_epoch],
            "sequence": context.sequence,
            "before": before,
            "expected": expected
        }),
    })
}

fn valid_state(value: &J) -> bool {
    let fields = [
        "position",
        "start",
        "pickup",
        "destination",
        "hazard",
        "item_available",
        "carrying",
        "won",
        "lost",
        "reserved",
        "action_held",
    ];
    exact_fields(value, &fields)
        && ["position", "start", "pickup", "destination", "hazard"]
            .iter()
            .all(|key| {
                value[*key]
                    .as_array()
                    .is_some_and(|pair| pair.len() == 2 && pair.iter().all(J::is_i64))
            })
        && [
            "item_available",
            "carrying",
            "won",
            "lost",
            "reserved",
            "action_held",
        ]
        .iter()
        .all(|key| value[*key].as_u64().is_some_and(|v| v <= 2))
        && value["reserved"].as_u64() == Some(0)
}

fn observe(
    frame: FrameView<'_>,
    receipt: &J,
    context: CommandContext,
) -> Result<Consumption, String> {
    let (actors, couriers) = frame.dense::<Courier>();
    if couriers.len() != 1
        || !exact_fields(
            receipt,
            &[
                "kind", "player", "actor", "scope", "sequence", "before", "expected",
            ],
        )
        || receipt["player"].as_u64() != Some(u64::from(context.player))
        || receipt["sequence"].as_u64() != Some(context.sequence)
        || receipt["scope"] != json!([context.scope.scene_epoch, context.scope.play_epoch])
        || receipt["actor"] != json!([actors[0].index, actors[0].version])
        || !valid_state(&receipt["before"])
        || !valid_state(&receipt["expected"])
    {
        return Ok(Consumption::Invalid);
    }
    let state = frame.singleton::<CollectDeliveryState>();
    let current = state_json(couriers[0].position, state);
    if state.action_held != 0 && current == receipt["expected"] {
        Ok(Consumption::Consumed)
    } else if state.action_held == 0 && current == receipt["before"] {
        Ok(Consumption::Pending)
    } else {
        Ok(Consumption::Invalid)
    }
}

pub fn descriptor() -> GameplayDescriptor {
    let schema: J = serde_json::from_str(&CollectDeliveryGame::types().legacy_json_schema())
        .expect("compiled collect-delivery scene schema");
    GameplayDescriptor {
        identity: RemoteIdentity {
            game: CollectDeliveryGame::ID.into(),
            build_id: format!("0x{:016x}", CollectDeliveryGame::BUILD),
            schema,
        },
        local_alias: CollectDeliveryGame::ALIAS,
        types: CollectDeliveryGame::types,
        host,
        stream,
        presentation: PresentationCapability::Spatial2d {
            position: StaticPosition {
                component: "Courier",
                field: "position",
            },
            draw,
        },
        input: InputCapability::Commands(CommandContract {
            input: GameplaySchema {
                type_name: CollectDeliveryGame::INPUT,
                types: input_types,
            },
            max_players: 1,
            neutral: json!({"dx":0,"dy":0,"action":0}),
            max_sequence: u64::MAX,
            intents: vec![
                IntentSchema {
                    name: "act",
                    payload: GameplaySchema {
                        type_name: "Act",
                        types: act_types,
                    },
                },
                IntentSchema {
                    name: "move",
                    payload: GameplaySchema {
                        type_name: "Move",
                        types: move_types,
                    },
                },
            ],
            encode,
            observe,
        }),
    }
}

pub fn registry() -> &'static GameplayContracts {
    static REGISTRY: OnceLock<GameplayContracts> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        GameplayContracts::new(vec![descriptor()], None)
            .expect("compiled collect-delivery gameplay contracts")
    })
}
