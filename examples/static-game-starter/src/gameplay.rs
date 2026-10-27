//! Project-owned command capabilities for two existing, unchanged games.
//!
//! This CPU example exercises the shared router. It does not add a manifest
//! resolver or claim that the native editor viewport consumes these descriptors.
use orr_bridge::FrameView;
use orr_editor::static_adapter::{
    CodecOutput, CommandContext, CommandContract, Consumption, GameplayDescriptor, GameplaySchema,
    InputCapability, IntentSchema, PresentationCapability,
};
use orr_reflect::{Reflect, TypeRegistry};
use orr_remote::RemoteIdentity;
use serde_json::{json, Value as J};

use crate::{
    delivery::{Delivery, DeliveryGame},
    game_contract::{inputs, Starter},
    switchboard::{Board, Cursor},
};

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable, Reflect)]
struct Act {
    #[reflect(range = "1..=1")]
    press: u32,
}
fn intent_types() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    types.register_component::<Act>("Act");
    types
}

fn delivery_encode(
    _: &str,
    _: &J,
    context: CommandContext,
    frame: FrameView<'_>,
) -> Result<CodecOutput, String> {
    if context.player != 0 || frame.singleton::<Delivery>().action_held != 0 {
        return Err("delivery action requires the neutral player-zero frame".into());
    }
    Ok(CodecOutput {
        input: json!({"dx":0,"dy":0,"deliver":1}),
        receipt: json!({"player":context.player}),
    })
}
fn delivery_observe(
    frame: FrameView<'_>,
    receipt: &J,
    context: CommandContext,
) -> Result<Consumption, String> {
    if receipt["player"] != context.player || context.player != 0 {
        return Ok(Consumption::Invalid);
    }
    Ok(if frame.singleton::<Delivery>().action_held == 1 {
        Consumption::Consumed
    } else {
        Consumption::Pending
    })
}
fn switch_encode(
    _: &str,
    _: &J,
    context: CommandContext,
    frame: FrameView<'_>,
) -> Result<CodecOutput, String> {
    let cursors = frame.dense::<Cursor>().1;
    if context.player != 0 || cursors.len() != 1 || cursors[0].held != 0 {
        return Err("switch action requires the neutral player-zero frame".into());
    }
    let board = frame.singleton::<Board>();
    Ok(CodecOutput {
        input: json!({"selection":0,"toggle":1}),
        receipt: json!({"player":context.player,"bits":board.bits,"moves":board.moves,"solved":board.solved}),
    })
}
fn switch_observe(
    frame: FrameView<'_>,
    receipt: &J,
    context: CommandContext,
) -> Result<Consumption, String> {
    if receipt["player"] != context.player || context.player != 0 {
        return Ok(Consumption::Invalid);
    }
    let cursors = frame.dense::<Cursor>().1;
    let board = frame.singleton::<Board>();
    let changed = receipt["bits"] != board.bits
        || receipt["moves"] != board.moves
        || receipt["solved"] != board.solved;
    Ok(if cursors.len() == 1 && cursors[0].held == 1 && changed {
        Consumption::Consumed
    } else {
        Consumption::Pending
    })
}

/// The application supplies callbacks and exact compiled identities; the engine
/// contains no game-name switch. Existing typed host/stream/draw callbacks are
/// reused rather than copied, including their input and scene admission checks.
pub fn descriptors() -> Vec<GameplayDescriptor> {
    crate::integration::descriptors()
        .into_iter()
        .map(|existing| {
            let delivery = existing.game_id == DeliveryGame::ID;
            GameplayDescriptor {
                identity: RemoteIdentity {
                    game: existing.game_id.into(),
                    build_id: format!("0x{:016x}", existing.build_id),
                    schema: serde_json::from_str(&(existing.types)().legacy_json_schema())
                        .expect("compiled starter scene schema"),
                },
                local_alias: existing.local_alias,
                types: existing.types,
                host: existing.host,
                stream: existing.stream,
                presentation: PresentationCapability::Spatial2d {
                    position: existing.position,
                    draw: existing.draw,
                },
                input: InputCapability::Commands(CommandContract {
                    input: GameplaySchema {
                        type_name: existing.input.type_name,
                        types: if delivery {
                            inputs::<DeliveryGame>
                        } else {
                            inputs::<crate::switchboard::SwitchGame>
                        },
                    },
                    max_players: 1,
                    neutral: (existing.input.held)(Default::default()),
                    max_sequence: u64::MAX,
                    intents: vec![IntentSchema {
                        name: "act",
                        payload: GameplaySchema {
                            type_name: "Act",
                            types: intent_types,
                        },
                    }],
                    encode: if delivery {
                        delivery_encode
                    } else {
                        switch_encode
                    },
                    observe: if delivery {
                        delivery_observe
                    } else {
                        switch_observe
                    },
                }),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_editor::editor::input::{CommandStatus, GameplayRouter};
    use orr_editor::static_adapter::{GameplayContracts, IntentRequest};
    use orr_remote::{
        Auth, Caps, ErpClient, LocalHost, RemoteConfig, ServerConfig, ViewDeliveryMode,
    };
    use std::time::{Duration, Instant};

    struct Fixture {
        _dir: tempfile::TempDir,
        _host: LocalHost,
        observer: ErpClient,
        router: GameplayRouter,
    }
    fn fixture(alias: &str, scene: &str) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.scene.yaml");
        std::fs::write(&path, scene).unwrap();
        let contract = GameplayContracts::new(descriptors(), None)
            .unwrap()
            .get(alias)
            .unwrap();
        let mut config = ServerConfig::new(Auth::DevNoAuth);
        config.listen = false;
        let (work_status, _owner) = orr_remote::work_status::WorkStatusService::pair();
        let host = (contract.host_factory())(orr_editor::static_adapter::StaticHostRequest {
            scene_text: scene.into(),
            scene_path: path,
            config,
            work_status,
            neutral_value: contract.neutral_value().clone(),
            expected_input: contract.expected_input().clone(),
        })
        .unwrap();
        let connector = host.connector();
        let mut observer = ErpClient::with_transport(Box::new(
            connector
                .connect("observer", Caps::parse("read").unwrap())
                .unwrap(),
        ));
        observer.call_timeout = Duration::from_secs(1);
        let mut remote = RemoteConfig::new("in-process");
        remote.source = "view".into();
        remote.view_delivery = ViewDeliveryMode::RequireFenced;
        let router =
            GameplayRouter::connect_local(contract, &host, "shared-ui-ai", Caps::ALL, remote)
                .unwrap();
        Fixture {
            _dir: dir,
            _host: host,
            observer,
            router,
        }
    }
    fn wait(router: &mut GameplayRouter, mut done: impl FnMut(&GameplayRouter) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            router.poll(Instant::now()).unwrap();
            if done(router) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "router progress deadline: state={}, error={:?}",
                router.state(),
                router.error()
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn act() -> IntentRequest {
        IntentRequest::new("act", json!({"press":1})).unwrap()
    }

    #[test]
    fn sticky_failure_discards_late_start_ack_without_sending_play() {
        let Fixture {
            _dir,
            _host,
            mut observer,
            mut router,
        } = fixture("delivery", crate::delivery::SCENE);
        let now = Instant::now();
        router.play().unwrap();
        router.poll(now).unwrap();
        // The real host admits sim.start, but this client deliberately does not
        // collect its ACK. This leaves the StartPlay continuation in flight.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let state = observer.call("sim.state", J::Null).unwrap();
            if state["mode"] == "play" {
                assert_eq!(state["playing"], false);
                assert_eq!(state["head_tick"], 0);
                break;
            }
            assert!(Instant::now() < deadline, "host start admission deadline");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(router.poll(now - Duration::from_millis(1)).is_err());
        assert_eq!(router.error(), Some("clock_regressed"));
        assert!(router.play().is_err());
        assert!(router.start().is_err());
        assert!(router.step().is_err());
        assert!(router.pause().is_err());
        assert!(router.stop().is_err());
        assert!(router.claim(0).is_err());
        assert!(router.submit(act()).is_err());
        for offset in [1, 100] {
            assert!(router.poll(now + Duration::from_millis(offset)).is_err());
            assert_eq!(router.error(), Some("clock_regressed"));
        }
        let state = observer.call("sim.state", J::Null).unwrap();
        assert_eq!(state["playing"], false);
        assert_eq!(state["head_tick"], 0);
        assert!(!router.can_submit());
        assert!(router.frame_evidence().is_none());
        let activity = observer
            .call("activity.list", json!({"include_reads":false}))
            .unwrap();
        let entries = activity["entries"].as_array().unwrap();
        assert!(entries
            .iter()
            .any(|entry| entry["method"] == "sim.start" && entry["ok"] == true));
        assert!(!entries.iter().any(|entry| entry["method"] == "sim.play"));
    }

    #[test]
    fn two_project_owned_games_share_actual_router_play_commands_and_stop() {
        // Only project scene data changes. Both games execute their existing
        // compiled systems, host and typed stream; no test simulation is copied.
        let delivery = crate::delivery::SCENE.replace("goal: [4, 0]", "goal: [0, 0]");
        let switchboard = crate::switchboard::SCENE.replace("target: 5", "target: 1");
        for (alias, scene) in [("delivery", delivery), ("switchboard", switchboard)] {
            let Fixture {
                _dir,
                _host,
                mut observer,
                mut router,
            } = fixture(alias, &scene);
            let original = observer.call("scene.save", J::Null).unwrap();
            let history = observer.call("history.list", J::Null).unwrap();
            assert!(router.submit(act()).is_err());
            router.play().unwrap();
            wait(&mut router, |router| {
                router.state()["playing"] == true
                    && router
                        .frame_evidence()
                        .is_some_and(|e| e.snapshot().tick() > 0)
                    && observer.call("sim.state", J::Null).unwrap()["playing"] == true
            });
            router.claim(0).unwrap();
            wait(&mut router, GameplayRouter::can_submit);
            assert!(router
                .submit(IntentRequest::new("act", json!({"press":0})).unwrap())
                .is_err());
            let first = router.submit(act()).unwrap();
            assert!(router.submit(act()).is_err(), "one pending command only");
            wait(&mut router, |r| {
                matches!(r.status(first), Some(CommandStatus::Completed { .. }))
            });
            let Some(CommandStatus::Completed { consumed, neutral }) = router.status(first) else {
                panic!("completed consumption and neutral evidence required");
            };
            assert_eq!(consumed.scope, neutral.scope);
            assert!(neutral.tick > consumed.tick);
            let evidence = router.frame_evidence().unwrap();
            let view = evidence.snapshot().predicted();
            if alias == "delivery" {
                assert_eq!(view.singleton::<Delivery>().won, 1);
                assert_eq!(view.singleton::<Delivery>().action_held, 0);
            } else {
                assert_eq!(view.singleton::<Board>().solved, 1);
                assert_eq!(view.singleton::<Board>().moves, 1);
                assert_eq!(view.dense::<Cursor>().1[0].held, 0);
            }
            // A second pulse must see the neutral tick and produce one restart.
            let second = router.submit(act()).unwrap();
            assert_ne!(first, second);
            wait(&mut router, |r| {
                matches!(r.status(second), Some(CommandStatus::Completed { .. }))
            });
            let evidence = router.frame_evidence().unwrap();
            let view = evidence.snapshot().predicted();
            if alias == "delivery" {
                assert_eq!(view.singleton::<Delivery>().won, 0);
            } else {
                assert_eq!(view.singleton::<Board>().solved, 0);
                assert_eq!(view.singleton::<Board>().moves, 0);
            }

            let activity = observer
                .call("activity.list", json!({"include_reads":false}))
                .unwrap();
            for id in [first, second] {
                assert!(activity["entries"].as_array().unwrap().iter().any(|entry| {
                    entry["method"] == "sim.input_value"
                        && entry["ok"] == true
                        && entry["summary"].as_str().is_some_and(|s| {
                            s.contains(&format!("command={}", id.0))
                                && s.contains("intent=act")
                                && s.contains("admission=accepted")
                        })
                }));
            }
            router.pause().unwrap();
            wait(&mut router, |r| {
                r.state()["mode"] == "play"
                    && r.state()["playing"] == false
                    && observer.call("sim.state", J::Null).unwrap()["playing"] == false
            });
            let paused = observer.call("sim.state", J::Null).unwrap();
            for _ in 0..10 {
                router.poll(Instant::now()).unwrap();
            }
            assert_eq!(
                observer.call("sim.state", J::Null).unwrap()["head_tick"],
                paused["head_tick"]
            );
            assert!(router.submit(act()).is_err());
            router.step().unwrap();
            let tick = paused["head_tick"].as_u64().unwrap();
            wait(&mut router, |r| {
                r.state()["head_tick"] == tick + 1
                    && observer.call("sim.state", J::Null).unwrap()["head_tick"] == tick + 1
            });
            router.stop().unwrap();
            wait(&mut router, |r| {
                r.state()["mode"] == "edit"
                    && observer.call("sim.state", J::Null).unwrap()["mode"] == "edit"
            });
            assert_eq!(observer.call("scene.save", J::Null).unwrap(), original);
            assert_eq!(observer.call("history.list", J::Null).unwrap(), history);
            assert_eq!(
                std::fs::read_to_string(_dir.path().join("project.scene.yaml")).unwrap(),
                scene
            );
            assert!(router.submit(act()).is_err());
            router.start().unwrap();
            wait(&mut router, |r| {
                r.state()["mode"] == "play"
                    && observer.call("sim.state", J::Null).unwrap()["mode"] == "play"
            });
            let restarted = observer.call("sim.state", J::Null).unwrap();
            assert_eq!(restarted["head_tick"], 0);
            assert_eq!(restarted["playing"], false);
            assert_eq!(restarted["checksum"], original["checksum"]);
            router.stop().unwrap();
            wait(&mut router, |r| {
                r.state()["mode"] == "edit"
                    && observer.call("sim.state", J::Null).unwrap()["mode"] == "edit"
            });
        }
    }

    #[test]
    fn command_capabilities_preserve_both_compiled_scene_and_input_identities() {
        let contracts = GameplayContracts::new(descriptors(), None).unwrap();
        for original in crate::integration::descriptors() {
            let admitted = contracts.get(original.game_id).unwrap();
            assert_eq!(admitted.identity().game, original.game_id);
            assert_eq!(
                admitted.identity().build_id,
                format!("0x{:016x}", original.build_id)
            );
            assert_eq!(
                admitted.identity().schema,
                serde_json::from_str::<J>(&(original.types)().legacy_json_schema()).unwrap()
            );
            assert_eq!(
                admitted.neutral_value(),
                &(original.input.held)(Default::default())
            );
            assert_eq!(admitted.intent_catalog().next().unwrap().id, "act");
            assert!(admitted
                .validate_intent(IntentRequest::new("act", json!({"press":1})).unwrap())
                .is_ok());
            for bad in [json!({"press":0}), json!({"press":2}), json!({}), J::Null] {
                assert!(admitted
                    .validate_intent(IntentRequest::new("act", bad).unwrap())
                    .is_err());
            }
            assert!(admitted
                .validate_intent(IntentRequest::new("unregistered", json!({"press":1})).unwrap())
                .is_err());
        }
    }
}
