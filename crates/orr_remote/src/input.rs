//! Opt-in, game-typed authoring input. Kept on the server rather than adding
//! fields to public host/config structs that embedders construct literally.

use std::sync::Arc;

use orr_reflect::{Reflect, TypeDesc, TypeRegistry};
use orr_sim::{Game, PlayerSlot};
use serde_json::{json, Value as J};

use crate::{json::json_to_value, methods, RpcError};

type DeriveCommands<G> =
    dyn Fn(PlayerSlot, &<G as Game>::Input) -> Vec<<G as Game>::Command> + Send + Sync;

pub(crate) struct StructuredInput<G: Game> {
    desc: TypeDesc,
    pub(crate) descriptor: J,
    pub(crate) max_players: u8,
    pub(crate) commands: Arc<DeriveCommands<G>>,
}

impl<G: Game> StructuredInput<G> {
    pub(crate) fn new(
        name: &'static str,
        max_players: u8,
        commands: impl Fn(PlayerSlot, &G::Input) -> Vec<G::Command> + Send + Sync + 'static,
    ) -> Self
    where
        G::Input: Reflect,
    {
        // This temporary registry generates only the input descriptor; input
        // is not a scene component and is never added to the game's registry.
        let mut types = TypeRegistry::new();
        types.register_component::<G::Input>(name);
        let schema: J =
            serde_json::from_str(&types.type_json_schema(name).expect("registered input"))
                .expect("reflection produces valid JSON Schema");
        Self {
            desc: G::Input::describe(),
            descriptor: json!({"schema": schema, "value_format": methods::VALUE_FORMAT, "max_players": max_players}),
            max_players,
            commands: Arc::new(commands),
        }
    }

    pub(crate) fn decode(&self, value: &J) -> Result<G::Input, RpcError> {
        let value = json_to_value(&self.desc, value, false).map_err(RpcError::params)?;
        // Padding/hidden fields are deterministic zeros, never caller bytes.
        let mut input = <G::Input as bytemuck::Zeroable>::zeroed();
        self.desc
            .set(bytemuck::bytes_of_mut(&mut input), &[], value)
            .map_err(|e| RpcError::params(e.to_string()))?;
        Ok(input)
    }
}

/// Wall-clock ownership is deliberately outside recorded simulation state.
#[derive(Default)]
pub(crate) struct ManagedHeld {
    pub(crate) enabled: bool,
    next_grant: u64,
    generation: u64,
    leases: std::collections::BTreeMap<u8, Lease>,
}

struct Lease {
    connection: u64,
    grant: u64,
    generation: u64,
    sequence: u64,
    deadline: std::time::Instant,
}

/// Optional correlation only, never authority to write input or proof of consumption.
/// Kept closed and bounded so activity can log it without retaining game payloads.
pub(crate) fn command_correlation<'a>(method: &str, params: &'a J) -> Result<Option<&'a J>, RpcError> {
    let Some(command) = params.get("command") else { return Ok(None); };
    let decimal = |value: &J, nonzero: bool| {
        value.as_str().is_some_and(|text| {
            !text.is_empty() && text.len() <= 20
                && text.bytes().all(|byte| byte.is_ascii_digit())
                && (text == "0" || !text.starts_with('0'))
                && text.parse::<u64>().is_ok_and(|number| !nonzero || number != 0)
        })
    };
    let managed = method == "sim.input_value"
        && ["grant", "generation", "sequence"].iter().all(|key| params.get(key).is_some());
    let valid = command.as_object().is_some_and(|fields| {
        fields.len() == 4 && ["id", "intent", "scene_epoch", "play_epoch"].iter().all(|key| fields.contains_key(*key))
            && decimal(&command["id"], true)
            && decimal(&command["scene_epoch"], false)
            && decimal(&command["play_epoch"], false)
            && command["intent"].as_str().is_some_and(|intent| {
                !intent.is_empty() && intent.len() <= 96
                    && intent.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b':'))
            })
    });
    if !managed || !valid {
        return Err(RpcError::params("command correlation requires managed sim.input_value and exactly bounded id/intent/scene_epoch/play_epoch fields"));
    }
    Ok(Some(command))
}

impl ManagedHeld {
    const TTL: std::time::Duration = std::time::Duration::from_secs(2);

    pub(crate) fn descriptor() -> J {
        json!({"version": 1, "lease_ms": 2000, "heartbeat_ms": 500})
    }

    pub(crate) fn status(&self, connection: u64) -> J {
        json!({"generation": self.generation.to_string(), "slots": self.leases.iter().map(|(&player, lease)| {
            json!({"player":player,"grant":lease.grant.to_string(),"generation":lease.generation.to_string(),"owned_by_you":lease.connection == connection})
        }).collect::<Vec<_>>()})
    }

    fn neutralize<G: Game>(target: &mut crate::ErpTarget<'_, G>, slot: u8) {
        if let Some(play) = target.play.as_mut() {
            play.session_mut()
                .set_input(PlayerSlot(slot), G::Input::default());
        }
    }

    pub(crate) fn expire<G: Game>(
        &mut self,
        target: &mut crate::ErpTarget<'_, G>,
        now: std::time::Instant,
    ) {
        self.leases.retain(|&slot, lease| {
            if now >= lease.deadline {
                Self::neutralize(target, slot);
                false
            } else {
                true
            }
        });
    }

    pub(crate) fn disconnect<G: Game>(
        &mut self,
        target: &mut crate::ErpTarget<'_, G>,
        connection: u64,
    ) {
        self.leases.retain(|&slot, lease| {
            if lease.connection == connection {
                Self::neutralize(target, slot);
                false
            } else {
                true
            }
        });
    }

    pub(crate) fn invalidate<G: Game>(&mut self, target: &mut crate::ErpTarget<'_, G>) {
        for &slot in self.leases.keys() {
            Self::neutralize(target, slot);
        }
        self.leases.clear();
        self.generation = self
            .generation
            .checked_add(1)
            .expect("input generation exhausted");
    }

    /// Intercepts only ownership-related calls. Validates completely before
    /// replacing held input or refreshing a deadline/sequence.
    pub(crate) fn handle<G: Game>(
        &mut self,
        target: &mut crate::ErpTarget<'_, G>,
        adapter: Option<&StructuredInput<G>>,
        caller: (u64, std::time::Instant),
        method: &str,
        params: &J,
    ) -> Option<Result<J, RpcError>> {
        let (connection, now) = caller;
        let ownership = matches!(
            method,
            "sim.input_claim" | "sim.input_renew" | "sim.input_release"
        );
        let input_write = matches!(method, "sim.input" | "sim.input_value");
        if !ownership && !input_write {
            self.expire(target, now);
            return None;
        }
        // Malformed correlation never falls through as legacy input, refreshes
        // a lease, or consumes a transport sequence (including disabled hosts).
        let correlation = match command_correlation(method, params) {
            Ok(value) => value,
            Err(error) => return Some(Err(error)),
        };
        self.expire(target, now);
        let tagged = ["grant", "generation", "sequence"]
            .iter()
            .any(|key| params.get(key).is_some());
        // Even disabled hosts must never interpret a lease-bearing write as legacy.
        if !self.enabled {
            return (ownership || tagged).then(|| {
                Err(RpcError::state(
                    "input_unavailable",
                    "managed held input is not enabled",
                ))
            });
        }
        let result = (|| {
            let obj = params
                .as_object()
                .ok_or_else(|| RpcError::params("params must be an object"))?;
            let p = crate::dispatch::P(obj);
            crate::dispatch::require_writable_play(target)?;
            let slot = crate::dispatch::slot_of(target, &p, true)?.0;
            let accepted_head_tick = target
                .play
                .as_ref()
                .expect("validated play")
                .session()
                .head_tick();
            if !ownership && !tagged && !self.leases.contains_key(&slot) {
                return Ok(None);
            }
            let adapter = adapter.ok_or_else(|| {
                RpcError::state(
                    "input_unavailable",
                    "managed held input requires a structured adapter",
                )
            })?;
            if method == "sim.input_claim" {
                if p.opt_bool("replace_held")? != Some(true) {
                    return Err(RpcError::params(
                        "claim requires explicit replace_held: true",
                    ));
                }
                if self.leases.contains_key(&slot) {
                    return Err(RpcError::state(
                        "input_owned",
                        "player already has a managed held-input owner",
                    ));
                }
                let grant = self
                    .next_grant
                    .checked_add(1)
                    .ok_or_else(|| RpcError::state("input_exhausted", "input grant exhausted"))?;
                self.next_grant = grant;
                Self::neutralize(target, slot);
                self.leases.insert(
                    slot,
                    Lease {
                        connection,
                        grant,
                        generation: self.generation,
                        sequence: 0,
                        deadline: now + Self::TTL,
                    },
                );
                return Ok(Some(
                    json!({"player": slot, "grant": grant.to_string(), "generation": self.generation.to_string(), "lease_ms": 2000, "accepted_head_tick": accepted_head_tick}),
                ));
            }
            if method == "sim.input" {
                return Err(RpcError::state(
                    "input_owned",
                    "raw input cannot write a managed slot or carry a grant",
                ));
            }
            let parse = |name: &str| -> Result<u64, RpcError> {
                p.str(name)?
                    .parse()
                    .map_err(|_| RpcError::params(format!("'{name}' must be a decimal u64 string")))
            };
            let (grant, generation, sequence) =
                (parse("grant")?, parse("generation")?, parse("sequence")?);
            let lease = self.leases.get(&slot).ok_or_else(|| {
                RpcError::state("input_stale", "held-input grant is no longer active")
            })?;
            if lease.connection != connection
                || lease.grant != grant
                || lease.generation != generation
            {
                return Err(RpcError::state(
                    "input_stale",
                    "held-input grant does not belong to this connection and session",
                ));
            }
            if sequence <= lease.sequence {
                return Err(RpcError::state(
                    "input_sequence",
                    "held-input sequence must increase",
                ));
            }
            let value = if method == "sim.input_value" {
                Some(adapter.decode(p.req_raw("value")?)?)
            } else {
                None
            };
            if method == "sim.input_release" {
                self.leases.remove(&slot);
                Self::neutralize(target, slot);
            } else {
                if let Some(value) = value {
                    target
                        .play
                        .as_mut()
                        .expect("validated play")
                        .session_mut()
                        .set_input(PlayerSlot(slot), value);
                }
                let lease = self.leases.get_mut(&slot).expect("validated grant");
                lease.sequence = sequence;
                lease.deadline = now + Self::TTL;
            }
            let mut acknowledgement = json!({"ok": true, "player": slot, "grant": grant.to_string(), "generation": generation.to_string(), "sequence": sequence.to_string(), "accepted_head_tick": accepted_head_tick});
            if let Some(command) = correlation { acknowledgement["command"] = command.clone(); }
            Ok(Some(acknowledgement))
        })();
        match result {
            Ok(None) => None,
            Ok(Some(value)) => Some(Ok(value)),
            Err(error) => Some(Err(error)),
        }
    }
}

#[cfg(test)]
mod managed_tests {
    use super::*;
    use crate::{dispatch, Caps, ErpTarget, HostLimits};
    use orr_edit::{EditorDoc, PlayController};
    use orr_sim::Simulation;
    use orr_testgame::{Arena, SpawnBulletCmd, FIRE};
    use std::time::{Duration, Instant};

    struct Rig {
        doc: EditorDoc,
        play: Option<PlayController<Arena>>,
        held: ManagedHeld,
        adapter: StructuredInput<Arena>,
        now: Instant,
    }
    impl Rig {
        fn new() -> Self {
            let mut types = TypeRegistry::new();
            orr_testgame::register_reflect(&mut types);
            let doc = EditorDoc::from_yaml("schema: orr.scene/1\nentities:\n  e_00000001:\n    Position: { pos: [-300,0] }\n    PlayerTag: { slot: 0 }\n  e_00000002:\n    Position: { pos: [300,0] }\n    PlayerTag: { slot: 1 }\n", types, Simulation::<Arena>::build_registry(), 42).unwrap();
            let adapter =
                StructuredInput::new("ArenaInput", 8, |slot, input: &orr_testgame::ArenaInput| {
                    if input.buttons & FIRE != 0 {
                        vec![SpawnBulletCmd {
                            owner: u32::from(slot.0),
                        }]
                    } else {
                        vec![]
                    }
                });
            let mut rig = Self {
                doc,
                play: None,
                held: ManagedHeld::default(),
                adapter,
                now: Instant::now(),
            };
            rig.held.enabled = true;
            rig.call(1, "sim.start", json!({})).unwrap();
            rig
        }
        fn call(&mut self, connection: u64, method: &str, params: J) -> Result<J, RpcError> {
            let target = &mut ErpTarget {
                doc: &mut self.doc,
                play: &mut self.play,
            };
            self.held.expire(target, self.now);
            let result = if let Some(result) = self.held.handle(
                target,
                Some(&self.adapter),
                (connection, self.now),
                method,
                &params,
            ) {
                result
            } else {
                dispatch::call(
                    target,
                    &HostLimits::default(),
                    Some(&self.adapter),
                    &dispatch::CallCtx {
                        client: "same-label",
                        caps: Caps::ALL,
                        last_play: None,
                        tx_check: None,
                    },
                    &mut dispatch::Effects::default(),
                    method,
                    &params,
                )
            };
            if result.is_ok()
                && matches!(
                    method,
                    "sim.start" | "sim.stop" | "sim.pause" | "sim.seek" | "sim.branch"
                )
            {
                self.held.invalidate(target);
            }
            result
        }
        fn claim(&mut self, conn: u64, player: u8) -> J {
            self.call(
                conn,
                "sim.input_claim",
                json!({"player":player,"replace_held":true}),
            )
            .unwrap()
        }
        fn disconnect(&mut self, conn: u64) {
            self.held.disconnect(
                &mut ErpTarget {
                    doc: &mut self.doc,
                    play: &mut self.play,
                },
                conn,
            );
        }
        fn step(&mut self) -> Vec<i64> {
            self.call(1, "sim.step", json!({})).unwrap();
            ["e_00000001", "e_00000002"]
                .into_iter()
                .map(|entity| {
                    self.call(
                        1,
                        "world.get",
                        json!({"entity":entity,"component":"Position","path":"pos"}),
                    )
                    .unwrap()["value"][0]
                        .as_i64()
                        .unwrap()
                })
                .collect()
        }
    }
    fn value(player: u8, axis: i32) -> J {
        json!({"player":player,"value":{"axis_x":axis,"axis_y":0,"buttons":[]}})
    }
    fn tagged(grant: &J, sequence: u64, axis: i32) -> J {
        let mut p = value(grant["player"].as_u64().unwrap() as u8, axis);
        p["grant"] = grant["grant"].clone();
        p["generation"] = grant["generation"].clone();
        p["sequence"] = json!(sequence.to_string());
        p
    }

    fn correlation() -> J {
        json!({"id":"1","intent":"2game:move_left","scene_epoch":"0","play_epoch":"3"})
    }

    #[test]
    fn command_correlation_is_closed_bounded_and_metadata_only() {
        let tagged = json!({"grant":"1","generation":"2","sequence":"3","command":correlation()});
        assert_eq!(command_correlation("sim.input_value", &tagged).unwrap(), Some(&tagged["command"]));
        for (key, invalid) in [
            ("id", json!("0")), ("id", json!("01")), ("id", json!("+1")),
            ("id", json!("18446744073709551616")), ("id", json!(1)),
            ("scene_epoch", json!("-1")), ("play_epoch", json!("00")),
            ("intent", json!("")), ("intent", json!("x".repeat(97))),
            ("intent", json!("move\nsecret")), ("intent", json!("move.path")),
            ("intent", json!("move/secret")), ("intent", json!("이동")),
        ] {
            let mut params = tagged.clone();
            params["command"][key] = invalid;
            assert!(command_correlation("sim.input_value", &params).is_err(), "{key}: {params}");
        }
        let mut params = tagged.clone();
        params["command"]["token"] = json!("never log me");
        assert!(command_correlation("sim.input_value", &params).is_err());
        params["command"] = J::Null;
        assert!(command_correlation("sim.input_value", &params).is_err());
        params["command"] = correlation();
        params["command"]["id"] = json!(u64::MAX.to_string());
        params["command"]["scene_epoch"] = json!(u64::MAX.to_string());
        params["command"]["intent"] = json!("x".repeat(96));
        assert!(command_correlation("sim.input_value", &params).is_ok());
        params.as_object_mut().unwrap().remove("grant");
        assert!(command_correlation("sim.input_value", &params).is_err());
    }

    #[test]
    fn command_correlation_echo_preserves_lease_authority_and_rejected_state() {
        let mut r = Rig::new();
        let grant = r.claim(2, 0);
        let mut params = tagged(&grant, 1, 1);
        params["command"] = correlation();
        let before = r.play.as_ref().unwrap().session().frame().checksum();
        let deadline = r.held.leases[&0].deadline;
        r.now += Duration::from_millis(1000);
        let mut malformed = params.clone();
        malformed["command"]["receipt"] = json!({"token":"must not survive"});
        assert!(r.call(2, "sim.input_value", malformed).is_err());
        assert_eq!(r.held.leases[&0].sequence, 0);
        assert_eq!(r.held.leases[&0].deadline, deadline);
        assert_eq!(r.play.as_ref().unwrap().session().frame().checksum(), before);
        assert!(r.call(3, "sim.input_value", params.clone()).is_err());
        assert_eq!(r.held.leases[&0].sequence, 0);
        let ack = r.call(2, "sim.input_value", params.clone()).unwrap();
        assert_eq!(ack["command"], correlation());
        assert_eq!(ack["sequence"], "1");
        assert_eq!(ack["accepted_head_tick"], 0);
        assert!(ack.get("consumed").is_none());
        assert!(r.call(2, "sim.input_value", params).is_err());
        assert_eq!(r.step(), [-294, 300]);
        r.call(2, "sim.pause", json!({})).unwrap();
        let mut stale = tagged(&grant, 2, 1);
        stale["command"] = correlation();
        assert!(r.call(2, "sim.input_value", stale).is_err());
    }

    #[test]
    fn command_correlation_never_falls_through_legacy_or_ownership_paths() {
        let mut r = Rig::new();
        let grant = r.claim(1, 0);
        let mut params = tagged(&grant, 1, 1);
        params["command"] = correlation();
        for method in ["sim.input", "sim.input_claim", "sim.input_renew", "sim.input_release"] {
            assert!(r.call(1, method, params.clone()).is_err());
            assert_eq!(r.held.leases[&0].sequence, 0);
        }
        let mut legacy = value(1, 1);
        legacy["command"] = correlation();
        assert!(r.call(1, "sim.input_value", legacy.clone()).is_err());
        r.held.enabled = false;
        assert!(r.call(1, "sim.input_value", legacy).is_err());
        assert!(r.call(1, "sim.input_value", params).is_err());
    }

    #[test]
    fn managed_claim_is_explicit_and_neutralizes_only_one_slot() {
        let mut r = Rig::new();
        r.call(1, "sim.input_value", value(0, 1)).unwrap();
        r.call(1, "sim.input_value", value(1, -1)).unwrap();
        r.disconnect(1); // Legacy persists, including on a managed-enabled host.
        assert_eq!(r.step(), [-294, 294]);
        assert!(r.call(2, "sim.input_claim", json!({"player":0})).is_err());
        assert!(r
            .call(
                2,
                "sim.input_claim",
                json!({"player":2,"replace_held":true})
            )
            .is_err());
        let grant = r.claim(2, 0);
        assert_eq!(r.step(), [-294, 288]);
        assert!(r
            .call(
                3,
                "sim.input_claim",
                json!({"player":0,"replace_held":true})
            )
            .is_err());
        assert!(r.call(2, "sim.input_value", value(0, 1)).is_err());
        assert!(r.call(3, "sim.input_value", tagged(&grant, 1, 1)).is_err());
        let raw =
            crate::codec::hex_encode(bytemuck::bytes_of(&orr_testgame::ArenaInput::default()));
        assert!(r
            .call(2, "sim.input", json!({"player":0,"input":raw}))
            .is_err());
        r.call(2, "sim.input_value", tagged(&grant, 1, 1)).unwrap();
        assert!(r.call(2, "sim.input_value", tagged(&grant, 1, -1)).is_err());
        assert!(r.call(2, "sim.input_value", tagged(&grant, 2, 5)).is_err());
        assert_eq!(r.step(), [-288, 282]);
        r.call(2, "sim.input_release", tagged(&grant, 2, 0))
            .unwrap();
        assert_eq!(r.step(), [-288, 276]);
    }

    #[test]
    fn managed_expiry_renew_disconnect_and_stale_cleanup_are_fenced() {
        let mut r = Rig::new();
        let first = r.claim(2, 0);
        r.call(2, "sim.input_value", tagged(&first, 1, 1)).unwrap();
        r.now += Duration::from_millis(1500);
        r.call(2, "sim.input_renew", tagged(&first, 2, 0)).unwrap();
        r.now += Duration::from_millis(1500);
        assert_eq!(r.step(), [-294, 300]);
        // Rejected updates do not extend the deadline or consume a sequence.
        assert!(r.call(2, "sim.input_value", tagged(&first, 3, 9)).is_err());
        r.now += Duration::from_millis(500);
        assert_eq!(r.step(), [-294, 300]);
        assert!(r.call(2, "sim.input_value", tagged(&first, 3, 1)).is_err());
        let next = r.claim(3, 0);
        assert_ne!(next["grant"], first["grant"]);
        r.call(3, "sim.input_value", tagged(&next, 1, -1)).unwrap();
        assert!(r
            .call(2, "sim.input_release", tagged(&first, 4, 0))
            .is_err());
        r.disconnect(2);
        assert_eq!(r.step(), [-300, 300]);
        r.disconnect(3);
        assert_eq!(r.step(), [-300, 300]);
    }

    #[test]
    fn managed_cleanup_preserves_accepted_commands_and_disabled_tags_fail() {
        use orr_sim::SimCommand;
        let mut r = Rig::new();
        let grant = r.claim(1, 0);
        let mut bytes = Vec::new();
        SpawnBulletCmd { owner: 0 }.encode(&mut bytes);
        r.call(
            1,
            "sim.command",
            json!({"player":0,"command":crate::codec::hex_encode(&bytes)}),
        )
        .unwrap();
        r.call(1, "sim.input_release", tagged(&grant, 1, 0))
            .unwrap();
        r.step();
        let replay = r.play.as_ref().unwrap().session().save_replay();
        let reader = orr_session::ReplayReader::<Arena>::parse(&replay).unwrap();
        assert_eq!(
            reader.tick(1).unwrap().1,
            vec![(PlayerSlot(0), SpawnBulletCmd { owner: 0 })]
        );
        r.held.enabled = false;
        assert!(r.call(1, "sim.input_value", tagged(&grant, 2, 1)).is_err());
        assert!(r
            .call(
                1,
                "sim.input_claim",
                json!({"player":0,"replace_held":true})
            )
            .is_err());
        r.call(1, "sim.input_value", value(0, 1)).unwrap();
    }

    #[test]
    fn managed_lifecycle_and_viewer_require_fresh_grants() {
        let mut r = Rig::new();
        for (method, params) in [
            ("sim.pause", json!({})),
            ("sim.seek", json!({"tick":0})),
            ("sim.branch", json!({})),
        ] {
            let grant = r.claim(1, 0);
            r.call(1, "sim.input_value", tagged(&grant, 1, 1)).unwrap();
            r.call(1, method, params).unwrap();
            assert!(r.call(1, "sim.input_value", tagged(&grant, 2, 1)).is_err());
            assert!(r.held.leases.is_empty());
        }
        let old = r.claim(1, 0);
        let replay = r.play.as_ref().unwrap().session().save_replay();
        r.held.invalidate(&mut ErpTarget {
            doc: &mut r.doc,
            play: &mut r.play,
        });
        // Replay viewers reject all input ownership without changing the timeline.
        *r.play.as_mut().unwrap().session_mut() =
            orr_session::PlaySession::<Arena>::open_replay(&replay, Default::default(), 0).unwrap();
        let checksum = r.play.as_ref().unwrap().session().frame().checksum();
        assert!(r
            .call(
                1,
                "sim.input_claim",
                json!({"player":0,"replace_held":true})
            )
            .is_err());
        assert!(r.call(1, "sim.input_value", tagged(&old, 1, 1)).is_err());
        assert_eq!(
            r.play.as_ref().unwrap().session().frame().checksum(),
            checksum
        );
        r.call(1, "sim.branch", json!({})).unwrap();
        let fresh = r.claim(1, 0);
        assert_ne!(fresh["generation"], old["generation"]);
        r.call(1, "sim.stop", json!({})).unwrap();
        r.call(1, "sim.start", json!({})).unwrap();
        assert!(r.call(1, "sim.input_value", tagged(&fresh, 1, 1)).is_err());
    }
}
