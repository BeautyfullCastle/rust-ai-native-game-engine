//! Project-owned semantic command capabilities for two starter games.
//!
//! One compiled registry serves saved projects and the shared editor/router.
//! The project owns its readonly presenter and intent payloads; the engine has
//! no game-name branches and never copies these games' simulation rules.
use orr_bridge::FrameView;
use orr_editor::static_adapter::{
    CodecOutput, CommandContext, CommandContract, CommandScope, Consumption, EmbeddedGameplayView,
    GameplayContracts, GameplayDescriptor, GameplaySchema, GameplayViewContext, GameplayViewPhase,
    InputCapability, IntentRequest, IntentSchema, PresentationCapability,
};
use orr_reflect::{Reflect, TypeRegistry};
use orr_remote::RemoteIdentity;
use serde_json::{json, Value as J};
use std::{marker::PhantomData, sync::OnceLock};

use crate::{
    delivery::{Courier, Delivery, DeliveryGame},
    game_contract::{inputs, Starter},
    presentation::StandaloneGame,
    switchboard::{Board, Cursor, SwitchGame},
};

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable, Reflect)]
struct Act {
    #[reflect(range = "1..=1")]
    press: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable, Reflect)]
struct Move {
    #[reflect(range = "-1..=1")]
    dx: i32,
    #[reflect(range = "-1..=1")]
    dy: i32,
}
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable, Reflect)]
struct Select {
    #[reflect(range = "-1..=1")]
    delta: i32,
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
fn select_types() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    types.register_component::<Select>("Select");
    types
}

fn controls(delivery: bool) -> Vec<(&'static str, &'static str, J)> {
    let mut controls = vec![("Act", "act", json!({"press":1}))];
    if delivery {
        controls.extend([
            ("Left", "move", json!({"dx":-1,"dy":0})),
            ("Right", "move", json!({"dx":1,"dy":0})),
            ("Up", "move", json!({"dx":0,"dy":1})),
            ("Down", "move", json!({"dx":0,"dy":-1})),
        ]);
    } else {
        controls.extend([
            ("Previous", "select", json!({"delta":-1})),
            ("Next", "select", json!({"delta":1})),
        ]);
    }
    controls
}

struct StarterView<G>(PhantomData<fn() -> G>);
impl<G: StandaloneGame> EmbeddedGameplayView for StarterView<G> {
    #[allow(clippy::float_arithmetic)] // View-only pixel layout; no simulation writes.
    fn show(
        &mut self,
        ui: &mut egui::Ui,
        frame: FrameView<'_>,
        context: GameplayViewContext,
    ) -> Result<Option<IntentRequest>, String> {
        let delivery = G::ID == DeliveryGame::ID;
        let actor_count = if delivery {
            frame.count::<Courier>()
        } else {
            frame.count::<Cursor>()
        };
        ui.heading(G::ID);
        if actor_count == 1 {
            let state = G::state_value(frame);
            ui.label(if state["won"] != 0 {
                "Victory: Act restarts"
            } else if delivery {
                "Move to the goal, then Act to deliver"
            } else {
                "Select a switch, then Act to toggle it"
            });
            ui.monospace(state.to_string());
        } else {
            ui.label("Controls require exactly one actor");
        }
        if let Some(reachability) = G::reachability(frame) {
            for warning in reachability.warnings() {
                ui.add(egui::Label::new(warning).wrap());
            }
        }
        let ready = context.phase == GameplayViewPhase::Live && context.input_ready;
        let mut intent = None;
        ui.horizontal_wrapped(|ui| {
            for (label, name, payload) in controls(delivery) {
                // The same read-only preflight gates both human buttons and AI
                // commands, including held latches, victory, and raw overflow.
                let command = CommandContext {
                    sequence: 1,
                    scope: CommandScope {
                        scene_epoch: 0,
                        play_epoch: 0,
                    },
                    player: 0,
                };
                let enabled = ready
                    && if delivery {
                        delivery_encode(name, &payload, command, frame).is_ok()
                    } else {
                        switch_encode(name, &payload, command, frame).is_ok()
                    };
                let response = ui.add_enabled(enabled, egui::Button::new(label));
                if response.clicked() && intent.is_none() {
                    intent = Some(IntentRequest::new(name, payload));
                }
            }
        });
        if !ready {
            ui.label("Play and Take control to use the project commands");
        }

        let available = ui.available_size();
        let size = egui::vec2(available.x.max(1.0), available.y.max(1.0));
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(18, 23, 32));
        let mut items = Vec::new();
        G::extract(frame, &mut items);
        // The existing project extractor is the only source of world positions,
        // colors and visual state. This is a view, not another game simulation.
        let scale = (rect.width() / 12.0).min(rect.height() / 8.0).max(1.0);
        let origin = egui::pos2(rect.center().x - 2.0 * scale, rect.center().y + scale);
        for item in items {
            let point = origin + egui::vec2(item.transform.pos.x, -item.transform.pos.y) * scale;
            let color: egui::Color32 = egui::Rgba::from_rgba_unmultiplied(
                item.style.color[0],
                item.style.color[1],
                item.style.color[2],
                item.style.color[3],
            )
            .into();
            painter.circle_filled(point, (item.style.size * scale).clamp(3.0, 32.0), color);
        }
        intent.transpose()
    }
    fn reset(&mut self) {}
}
fn view<G: StandaloneGame>() -> Box<dyn EmbeddedGameplayView> {
    Box::new(StarterView::<G>(PhantomData))
}

fn exact_fields(value: &J, fields: &[&str]) -> bool {
    value.as_object().is_some_and(|object| {
        object.len() == fields.len() && fields.iter().all(|field| object.contains_key(*field))
    })
}
fn integer_number(value: &J) -> Option<i64> {
    if !value.is_number() {
        return None;
    }
    // Match the reflected integer-value contract: clients may spell 1 as 1.0.
    // The existing parser uses decimal text, never a floating-point conversion.
    match orr_remote::json::json_to_value(&i64::describe(), value, false).ok()? {
        orr_reflect::Value::Int(value) => i64::try_from(value).ok(),
        _ => None,
    }
}
fn axis(value: &J) -> Result<i64, String> {
    integer_number(value)
        .filter(|value| (-1..=1).contains(value))
        .ok_or_else(|| "navigation requires integer axes in -1..=1".into())
}
fn primary_payload(payload: &J) -> bool {
    exact_fields(payload, &["press"]) && integer_number(&payload["press"]) == Some(1)
}
fn delivery_state(courier: &Courier, state: &Delivery) -> J {
    json!({"position":[courier.position.x.raw(),courier.position.y.raw()],
        "goal":[state.goal.x.raw(),state.goal.y.raw()],"won":state.won})
}
fn switch_state(cursor: &Cursor, board: &Board) -> J {
    json!({"position":[cursor.position.x.raw(),cursor.position.y.raw()],
        "selected":cursor.selected,"bits":board.bits,"target":board.target,
        "solved":board.solved,"moves":board.moves})
}
fn delivery_encode(
    name: &str,
    payload: &J,
    context: CommandContext,
    frame: FrameView<'_>,
) -> Result<CodecOutput, String> {
    let (actors, couriers) = frame.dense::<Courier>();
    let state = frame.singleton::<Delivery>();
    if context.player != 0 || couriers.len() != 1 || state.action_held != 0 {
        return Err("delivery command requires one released player-zero courier".into());
    }
    let courier = &couriers[0];
    let before = delivery_state(courier, state);
    let mut expected = before.clone();
    let input = match name {
        "act" if primary_payload(payload) => {
            if state.won != 0 {
                expected["position"] = json!([0, 0]);
                expected["won"] = json!(0);
            } else if courier.position == state.goal {
                expected["won"] = json!(1);
            }
            json!({"dx":0,"dy":0,"deliver":crate::delivery::MODE_PRIMARY})
        }
        "move" if exact_fields(payload, &["dx", "dy"]) => {
            let dx = axis(&payload["dx"])?;
            let dy = axis(&payload["dy"])?;
            if (dx == 0 && dy == 0) || state.won != 0 {
                return Err("move requires a nonzero direction before victory".into());
            }
            let x = courier.position.x.raw().checked_add(dx * (1 << 16));
            let y = courier.position.y.raw().checked_add(dy * (1 << 16));
            let (Some(x), Some(y)) = (x, y) else {
                return Err("move exceeds the fixed-point position range".into());
            };
            expected["position"] = json!([x, y]);
            json!({"dx":dx,"dy":dy,"deliver":crate::delivery::MODE_NAVIGATION_PULSE})
        }
        _ => return Err("invalid delivery intent or payload".into()),
    };
    Ok(CodecOutput {
        input,
        receipt: json!({"kind":name,"player":context.player,
            "actor":[actors[0].index,actors[0].version],"before":before,"expected":expected}),
    })
}
fn switch_encode(
    name: &str,
    payload: &J,
    context: CommandContext,
    frame: FrameView<'_>,
) -> Result<CodecOutput, String> {
    let (actors, cursors) = frame.dense::<Cursor>();
    if context.player != 0 || cursors.len() != 1 || cursors[0].held != 0 {
        return Err("switch command requires one released player-zero cursor".into());
    }
    let cursor = &cursors[0];
    let board = frame.singleton::<Board>();
    let before = switch_state(cursor, board);
    let mut expected = before.clone();
    let input = match name {
        "act" if primary_payload(payload) && cursor.selected < 4 => {
            expected["position"][0] = json!(i64::from(cursor.selected) * (2 << 16));
            if board.solved != 0 {
                expected["bits"] = json!(0);
                expected["solved"] = json!(0);
                expected["moves"] = json!(0);
            } else {
                let bits = board.bits ^ (1 << cursor.selected);
                expected["bits"] = json!(bits);
                expected["moves"] = json!(board.moves.saturating_add(1));
                expected["solved"] = json!(u32::from(bits == board.target));
            }
            json!({"selection":0,"toggle":crate::switchboard::MODE_PRIMARY})
        }
        "select" if exact_fields(payload, &["delta"]) => {
            let delta = axis(&payload["delta"])?;
            if delta == 0 {
                return Err("select requires a nonzero direction".into());
            }
            let selected = (i64::from(cursor.selected) + delta).rem_euclid(4);
            expected["selected"] = json!(selected);
            expected["position"][0] = json!(selected * (2 << 16));
            json!({"selection":delta,"toggle":crate::switchboard::MODE_NAVIGATION_PULSE})
        }
        _ => return Err("invalid switch intent or payload".into()),
    };
    Ok(CodecOutput {
        input,
        receipt: json!({"kind":name,"player":context.player,
            "actor":[actors[0].index,actors[0].version],"before":before,"expected":expected}),
    })
}

fn raw_pair(value: &J) -> bool {
    value
        .as_array()
        .is_some_and(|pair| pair.len() == 2 && pair.iter().all(|v| v.as_i64().is_some()))
}
fn counter(value: &J) -> bool {
    value.as_u64().is_some_and(|v| u32::try_from(v).is_ok())
}
fn delivery_state_valid(value: &J) -> bool {
    exact_fields(value, &["position", "goal", "won"])
        && raw_pair(&value["position"])
        && raw_pair(&value["goal"])
        && counter(&value["won"])
}
fn switch_state_valid(value: &J) -> bool {
    exact_fields(
        value,
        &["position", "selected", "bits", "target", "solved", "moves"],
    ) && raw_pair(&value["position"])
        && ["selected", "bits", "target", "solved", "moves"]
            .iter()
            .all(|key| counter(&value[*key]))
}
fn observe_receipt(
    receipt: &J,
    context: CommandContext,
    kinds: &[&str],
    actor: orr_ecs::Entity,
    held: u32,
    current: J,
    valid: fn(&J) -> bool,
) -> Consumption {
    if !exact_fields(receipt, &["kind", "player", "actor", "before", "expected"])
        || context.player != 0
        || receipt["player"].as_u64() != Some(u64::from(context.player))
        || !receipt["kind"]
            .as_str()
            .is_some_and(|kind| kinds.contains(&kind))
        || receipt["actor"] != json!([actor.index, actor.version])
        || !valid(&receipt["before"])
        || !valid(&receipt["expected"])
    {
        return Consumption::Invalid;
    }
    match held {
        1 if current == receipt["expected"] => Consumption::Consumed,
        0 if current == receipt["before"] => Consumption::Pending,
        // A held but contradictory frame is not evidence that this command ran.
        // A release after lost observation likewise cannot manufacture proof.
        _ => Consumption::Invalid,
    }
}
fn delivery_observe(
    frame: FrameView<'_>,
    receipt: &J,
    context: CommandContext,
) -> Result<Consumption, String> {
    let (actors, couriers) = frame.dense::<Courier>();
    if couriers.len() != 1 {
        return Ok(Consumption::Invalid);
    }
    let state = frame.singleton::<Delivery>();
    Ok(observe_receipt(
        receipt,
        context,
        &["act", "move"],
        actors[0],
        state.action_held,
        delivery_state(&couriers[0], state),
        delivery_state_valid,
    ))
}
fn switch_observe(
    frame: FrameView<'_>,
    receipt: &J,
    context: CommandContext,
) -> Result<Consumption, String> {
    let (actors, cursors) = frame.dense::<Cursor>();
    if cursors.len() != 1 {
        return Ok(Consumption::Invalid);
    }
    Ok(observe_receipt(
        receipt,
        context,
        &["act", "select"],
        actors[0],
        cursors[0].held,
        switch_state(&cursors[0], frame.singleton::<Board>()),
        switch_state_valid,
    ))
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
                presentation: PresentationCapability::Embedded {
                    create: if delivery {
                        view::<DeliveryGame>
                    } else {
                        view::<SwitchGame>
                    },
                },
                input: InputCapability::Commands(CommandContract {
                    input: GameplaySchema {
                        type_name: existing.input.type_name,
                        types: if delivery {
                            inputs::<DeliveryGame>
                        } else {
                            inputs::<SwitchGame>
                        },
                    },
                    max_players: 1,
                    neutral: (existing.input.held)(Default::default()),
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
                            name: if delivery { "move" } else { "select" },
                            payload: GameplaySchema {
                                type_name: if delivery { "Move" } else { "Select" },
                                types: if delivery { move_types } else { select_types },
                            },
                        },
                    ],
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

/// The executable constructs its trusted compiled registry once, then projects
/// select immutable handles from it. JSON never introduces callback code.
pub fn registry() -> &'static GameplayContracts {
    static GAMES: OnceLock<GameplayContracts> = OnceLock::new();
    GAMES.get_or_init(|| {
        GameplayContracts::new(descriptors(), None).expect("compiled gameplay contracts")
    })
}

#[cfg(test)]
#[path = "gameplay_cpu_api_acceptance.rs"]
mod cpu_api_acceptance;
#[cfg(all(test, feature = "api-acceptance"))]
#[path = "gameplay_api_acceptance.rs"]
mod gpu_api_acceptance;
#[cfg(test)]
#[path = "gameplay_ws_acceptance.rs"]
#[allow(
    clippy::disallowed_types,
    reason = "Wall-clock deadlines bound host transport tests, not simulation"
)]
mod ws_acceptance;

#[cfg(test)]
#[path = "gameplay_navigation_tests.rs"]
mod navigation_tests;

#[cfg(test)]
#[allow(
    clippy::disallowed_types,
    reason = "Wall-clock deadlines bound host lifecycle tests, not simulation"
)]
mod tests {
    use super::*;
    use orr_editor::editor::input::{CommandStatus, GameplayRouter};
    use orr_editor::static_adapter::{GameplayContracts, IntentRequest};
    use orr_remote::{
        Auth, Caps, ErpClient, LocalHost, RemoteConfig, ServerConfig, ViewDeliveryMode,
    };
    use std::time::{Duration, Instant};

    fn saved_project(alias: &str, scene: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let (manifest, filename) = match alias {
            "delivery" => (
                include_str!("../scenes/delivery.project.json"),
                "delivery.scene.yaml",
            ),
            "switchboard" => (
                include_str!("../scenes/switchboard.project.json"),
                "switchboard.scene.yaml",
            ),
            _ => panic!("unknown fixture"),
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.json");
        std::fs::write(&path, manifest).unwrap();
        std::fs::write(dir.path().join(filename), scene).unwrap();
        (dir, path)
    }
    fn wait_editor(
        editor: &mut orr_editor::Editor,
        mut done: impl FnMut(&orr_editor::Editor) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            editor.pump();
            assert!(editor.down().is_none(), "editor host disconnected");
            assert!(
                editor.gameplay_error().is_none(),
                "{:?}",
                editor.gameplay_error()
            );
            if done(editor) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "editor progress deadline: mode={:?}, tick={}, command error={:?}",
                editor.mode(),
                editor.sim().head_tick,
                editor.gameplay_error()
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn saved_manifests_select_compiled_capabilities_and_reject_unsafe_inputs() {
        use orr_editor::registered_project::PreparedRegisteredProject;
        for (alias, scene) in [
            ("delivery", crate::delivery::SCENE),
            ("switchboard", crate::switchboard::SCENE),
        ] {
            let (dir, path) = saved_project(alias, scene);
            let source = std::fs::read(&path).unwrap();
            let prepared = PreparedRegisteredProject::open(&path, registry()).unwrap();
            let handle = registry().handle(alias).unwrap();
            assert_eq!(prepared.game(), handle);
            assert_eq!(prepared.scene_text(), scene);
            assert!(!format!("{prepared:?}").contains("entities:"));
            let base: J = serde_json::from_slice(&source).unwrap();
            for (field, bad) in [
                ("game", json!("NotCompiled")),
                ("game", json!(alias)),
                ("build_id", json!("0x0000000000000001")),
                ("build_id", json!("0x1")),
                ("input", json!("held")),
                ("input", json!("downloaded-code")),
                ("scene", json!("../outside.yaml")),
                ("scene", json!("C:/outside.yaml")),
                ("scene", json!("game.yaml:stream")),
                ("scene", json!("CON.yaml")),
                ("scene", json!("https://example.com/game.yaml")),
                ("code", json!("untrusted.dll")),
            ] {
                let mut invalid = base.clone();
                invalid[field] = bad;
                std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
                assert!(
                    PreparedRegisteredProject::open(&path, registry()).is_err(),
                    "accepted field {field}"
                );
            }
            std::fs::write(&path, vec![b' '; 64 * 1024 + 1]).unwrap();
            assert!(PreparedRegisteredProject::open(&path, registry())
                .unwrap_err()
                .contains("65536"));
            std::fs::write(&path, &source).unwrap();
            let scene_path = prepared.scene_path();
            std::fs::write(scene_path, vec![b' '; 256 * 1024 + 1]).unwrap();
            assert!(PreparedRegisteredProject::open(&path, registry())
                .unwrap_err()
                .contains("262144"));
            std::fs::write(
                scene_path,
                "schema: orr.scene/1\nentities: {e_00000001: {AlienType: {}}}\n",
            )
            .unwrap();
            assert!(PreparedRegisteredProject::open(&path, registry()).is_err());
            std::fs::write(scene_path, scene).unwrap();
            #[cfg(unix)]
            {
                let target = dir.path().join("real.scene.yaml");
                std::fs::rename(scene_path, &target).unwrap();
                std::os::unix::fs::symlink(&target, scene_path).unwrap();
                assert!(PreparedRegisteredProject::open(&path, registry()).is_err());
            }
            #[cfg(not(unix))]
            let _ = dir;
        }
    }

    #[test]
    fn saved_projects_drive_the_existing_editor_lifecycle_and_preserve_source() {
        use orr_editor::{registered_project::PreparedRegisteredProject, Editor, Mode};
        // CPU-only win fixtures change scene data. Shipped native projects keep
        // the original delivery goal [4,0] and switch target 5.
        for (alias, scene) in [
            (
                "delivery",
                crate::delivery::SCENE.replace("goal: [4, 0]", "goal: [0, 0]"),
            ),
            (
                "switchboard",
                crate::switchboard::SCENE.replace("target: 5", "target: 1"),
            ),
        ] {
            let (_dir, path) = saved_project(alias, &scene);
            let manifest_bytes = std::fs::read(&path).unwrap();
            let prepared = PreparedRegisteredProject::open(&path, registry()).unwrap();
            let handle = prepared.game();
            let scene_path = prepared.scene_path().to_path_buf();
            // Admission owns the exact scene bytes. First startup must succeed
            // even after the on-disk scene becomes invalid, without rereading it.
            std::fs::write(&scene_path, "invalid after admission").unwrap();
            let mut editor = Editor::open_registered_project(prepared).unwrap();
            editor.sync();
            assert_eq!(editor.game().gameplay(), Some(handle));
            std::fs::write(&scene_path, &scene).unwrap();
            let original = editor.host_call("scene.save", J::Null).unwrap();
            let history = editor.host_call("history.list", J::Null).unwrap();
            assert!(editor.submit_gameplay_intent(act()).is_err());
            editor.play();
            wait_editor(&mut editor, |e| {
                e.sim().playing
                    && e.gameplay_frame_evidence()
                        .is_some_and(|v| v.snapshot().tick() > 0)
            });
            editor.claim_gameplay(0).unwrap();
            wait_editor(&mut editor, Editor::can_submit_gameplay);
            let id = editor.submit_gameplay_intent(act()).unwrap();
            assert!(editor.submit_gameplay_intent(act()).is_err());
            wait_editor(&mut editor, |e| {
                matches!(
                    e.gameplay_command_status(id),
                    Some(CommandStatus::Completed { .. })
                )
            });
            let frame = editor
                .gameplay_frame_evidence()
                .unwrap()
                .snapshot()
                .predicted();
            if alias == "delivery" {
                assert_eq!(frame.singleton::<Delivery>().won, 1);
            } else {
                assert_eq!(frame.singleton::<Board>().solved, 1);
                assert_eq!(frame.singleton::<Board>().moves, 1);
            }
            let next = editor.submit_gameplay_intent(act()).unwrap();
            wait_editor(&mut editor, |e| {
                matches!(
                    e.gameplay_command_status(next),
                    Some(CommandStatus::Completed { .. })
                )
            });
            let frame = editor
                .gameplay_frame_evidence()
                .unwrap()
                .snapshot()
                .predicted();
            if alias == "delivery" {
                assert_eq!(frame.singleton::<Delivery>().won, 0);
            } else {
                assert_eq!(frame.singleton::<Board>().solved, 0);
                assert_eq!(frame.singleton::<Board>().moves, 0);
            }
            editor.pause();
            wait_editor(&mut editor, |e| {
                !e.sim().playing && e.input_phase() == orr_editor::editor::input::Phase::Off
            });
            let paused = editor.sim().head_tick;
            for _ in 0..10 {
                editor.pump();
            }
            assert_eq!(editor.sim().head_tick, paused);
            assert!(editor.submit_gameplay_intent(act()).is_err());
            editor.step(1);
            wait_editor(&mut editor, |e| {
                e.sim().head_tick == paused + 1 && !e.sim().playing
            });
            let _ = editor.stop();
            wait_editor(&mut editor, |e| e.mode() == Mode::Edit);
            editor.sync();
            assert_eq!(editor.host_call("scene.save", J::Null).unwrap(), original);
            assert_eq!(editor.host_call("history.list", J::Null).unwrap(), history);
            assert_eq!(std::fs::read_to_string(&scene_path).unwrap(), scene);
            assert_eq!(std::fs::read(&path).unwrap(), manifest_bytes);
            assert!(editor.start_play());
            editor.sync();
            assert_eq!(editor.sim().head_tick, 0);
            assert!(!editor.sim().playing);
            assert_eq!(
                editor.host_call("sim.state", J::Null).unwrap()["checksum"],
                original["checksum"]
            );
            let _ = editor.stop();
            wait_editor(&mut editor, |e| e.mode() == Mode::Edit);
            assert!(editor.restart());
            editor.sync();
            assert_eq!(editor.game().gameplay(), Some(handle));
            assert_eq!(editor.host_call("scene.save", J::Null).unwrap(), original);
            assert_eq!(editor.mode(), Mode::Edit);
            assert_eq!(editor.sim().head_tick, 0);
            assert!(!editor.sim().playing);
            assert_eq!(editor.input_phase(), orr_editor::editor::input::Phase::Off);
            assert!(!editor.can_submit_gameplay());
            assert!(editor.gameplay_command_status(id).is_none());
            assert!(editor.gameplay_command_status(next).is_none());
            editor.play();
            wait_editor(&mut editor, |e| {
                e.sim().playing
                    && e.gameplay_frame_evidence()
                        .is_some_and(|v| v.snapshot().tick() > 0)
            });
            editor.claim_gameplay(0).unwrap();
            wait_editor(&mut editor, Editor::can_submit_gameplay);
            let after_restart = editor.submit_gameplay_intent(act()).unwrap();
            assert!(after_restart.0 > next.0);
            assert_ne!(after_restart, id);
            assert_ne!(after_restart, next);
            assert!(editor.gameplay_command_status(id).is_none());
            assert!(editor.gameplay_command_status(next).is_none());
            wait_editor(&mut editor, |e| {
                matches!(
                    e.gameplay_command_status(after_restart),
                    Some(CommandStatus::Completed { .. })
                )
            });
            let _ = editor.stop();
            wait_editor(&mut editor, |e| e.mode() == Mode::Edit);
            assert_eq!(editor.host_call("scene.save", J::Null).unwrap(), original);
            assert_eq!(editor.host_call("history.list", J::Null).unwrap(), history);
            drop(editor);
            let reopened = PreparedRegisteredProject::open(&path, registry()).unwrap();
            let mut editor = Editor::open_registered_project(reopened).unwrap();
            editor.sync();
            assert_eq!(editor.host_call("scene.save", J::Null).unwrap(), original);
            assert_eq!(editor.host_call("history.list", J::Null).unwrap(), history);
        }
    }

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
    include!("gameplay_stock_controls_tests.rs");
}
