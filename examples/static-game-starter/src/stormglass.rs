//! Project-owned registration of Stormglass's authored starting conditions.
//! This is a readonly presenter and command codec, not another simulation or
//! wave authoring format. All authoritative gameplay remains in DefenseGame.
use orr_bridge::{Bridge, BridgeEvent, FrameView, ViewUpdate};
use orr_editor::static_adapter::{
    CodecOutput, CommandContext, CommandContract, Consumption, EmbeddedGameplayView,
    GameplayContracts, GameplayDescriptor, GameplaySchema, GameplayViewContext, GameplayViewPhase,
    InputCapability, IntentRequest, IntentSchema, PresentationCapability, StaticFrameStream,
    StaticHostRequest,
};
use orr_games::coop_defense_game::{
    investment_cost, scenario_types, upgrade_cost, Campaign, DefenseEnemy, DefenseGame,
    DefenseInput, DefenseRun, CELLS_PER_BOARD, INVEST, LOST, MAX_INVESTMENT, MAX_UPGRADE,
    MOVE_OR_MERGE, PLAYING, RESTART, SUMMON, SUMMON_COST, UPGRADE, WON,
};
use orr_reflect::{Reflect, TypeRegistry};
use orr_remote::{
    coop_defense, ExpectedInputDescriptor, LocalHost, RemoteBridge, RemoteConfig, RemoteIdentity,
    RemoteViewDelivery, RpcError, Transport,
};
use serde_json::{json, Value as J};
use std::sync::OnceLock;

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable, Reflect)]
struct DefenseAction {
    #[reflect(range = "1..=5")]
    action: u32,
    #[reflect(range = "0..=11")]
    source_cell: u32,
    #[reflect(range = "0..=11")]
    target_cell: u32,
}
fn intent_types() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    types.register_component::<DefenseAction>("DefenseAction");
    types
}
fn input_types() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    types.register_component::<DefenseInput>("DefenseInput");
    types
}
fn neutral() -> J {
    json!({"action":0,"source_cell":0,"target_cell":0,"sequence":0})
}
fn expected_input() -> Result<ExpectedInputDescriptor, String> {
    let schema = input_types()
        .type_json_schema("DefenseInput")
        .ok_or("missing DefenseInput schema")?;
    ExpectedInputDescriptor::new(
        serde_json::from_str(&schema).map_err(|e| e.to_string())?,
        orr_remote::methods::VALUE_FORMAT,
        1,
    )
}
fn host(request: StaticHostRequest) -> Result<LocalHost, String> {
    if request.expected_input != expected_input()? || request.neutral_value != neutral() {
        return Err("Stormglass typed input contract mismatch".into());
    }
    coop_defense::spawn_host(
        request.scene_text,
        Some(request.scene_path),
        request.config,
        Some(request.work_status),
    )
}
struct Stream(RemoteBridge<DefenseGame>);
impl StaticFrameStream for Stream {
    fn poll_view(&mut self) -> ViewUpdate<()> {
        let update = self.0.poll_view();
        ViewUpdate {
            snapshot: update.snapshot,
            resync: update.resync,
            events: update
                .events
                .into_iter()
                .filter_map(|event| match event {
                    BridgeEvent::Lifecycle(event) => Some(BridgeEvent::Lifecycle(event)),
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
        RemoteBridge::<DefenseGame>::connect_transport_with_input(transport, config, input)?,
    )))
}
fn sequence(context: CommandContext) -> Result<u32, String> {
    let sequence = u32::try_from(context.sequence)
        .map_err(|_| "Stormglass sequence exceeds u32".to_string())?;
    if context.player != 0 || sequence == 0 {
        return Err("Stormglass requires player zero and a nonzero sequence".into());
    }
    Ok(sequence)
}
fn encode(
    name: &str,
    payload: &J,
    context: CommandContext,
    frame: FrameView<'_>,
) -> Result<CodecOutput, String> {
    let sequence = sequence(context)?;
    if name != "act" || sequence <= frame.singleton::<DefenseRun>().last_sequence {
        return Err("Stormglass requires a fresh act command".into());
    }
    // Admission validates the reflected payload; also reject malformed direct
    // callback invocations without narrowing or silently defaulting integers.
    let action = payload["action"].as_u64().filter(|v| (1..=5).contains(v));
    let source = payload["source_cell"]
        .as_u64()
        .filter(|v| *v < u64::from(CELLS_PER_BOARD));
    let target = payload["target_cell"]
        .as_u64()
        .filter(|v| *v < u64::from(CELLS_PER_BOARD));
    let (Some(action), Some(source), Some(target)) = (action, source, target) else {
        return Err("invalid Stormglass action or cell".into());
    };
    Ok(CodecOutput {
        input: json!({"action":action,"source_cell":source,"target_cell":target,"sequence":sequence}),
        receipt: json!({"player":context.player,"sequence":sequence}),
    })
}
fn observe(
    frame: FrameView<'_>,
    receipt: &J,
    context: CommandContext,
) -> Result<Consumption, String> {
    let Ok(sequence) = sequence(context) else {
        return Ok(Consumption::Invalid);
    };
    if receipt["player"] != context.player || receipt["sequence"] != sequence {
        return Ok(Consumption::Invalid);
    }
    // Consumption is NOT success: insufficient gold, an occupied target, or a
    // terminal run can consume a well-formed command without changing gameplay.
    Ok(
        match frame.singleton::<DefenseRun>().last_sequence.cmp(&sequence) {
            std::cmp::Ordering::Less => Consumption::Pending,
            std::cmp::Ordering::Equal => Consumption::Consumed,
            std::cmp::Ordering::Greater => Consumption::Invalid,
        },
    )
}

// Snapshot-only economic advice, not action eligibility or a dispatch gate.
fn cost_hint(label: &str, gold: u32, cost: Option<u32>) -> String {
    match cost {
        None => format!("{label}: maximum reached"),
        Some(cost) if gold < cost => {
            format!("{label}: {cost} gold (need {} more)", cost - gold)
        }
        Some(cost) => format!("{label}: {cost} gold"),
    }
}
fn cost_hints(run: &DefenseRun) -> [String; 3] {
    [
        cost_hint("Summon", run.gold[0], Some(SUMMON_COST)),
        cost_hint(
            "Upgrade",
            run.gold[0],
            (run.upgrades[0] < MAX_UPGRADE).then(|| upgrade_cost(run.upgrades[0])),
        ),
        cost_hint(
            "Invest",
            run.gold[0],
            (run.investment[0] < MAX_INVESTMENT).then(|| investment_cost(run.investment[0])),
        ),
    ]
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum PickCell {
    Source,
    #[default]
    Target,
}

#[derive(Default)]
struct StormglassView {
    source: u32,
    target: u32,
    selected: Option<u32>,
    selected_source: Option<u32>,
    pick: PickCell,
    generation: Option<u32>,
}
impl EmbeddedGameplayView for StormglassView {
    fn show(
        &mut self,
        ui: &mut egui::Ui,
        frame: FrameView<'_>,
        context: GameplayViewContext,
    ) -> Result<Option<IntentRequest>, String> {
        let run = frame.singleton::<DefenseRun>();
        if self.generation != Some(run.generation) {
            self.reset();
            self.generation = Some(run.generation);
        }
        ui.horizontal_wrapped(|ui| {
            ui.strong("Stormglass");
            ui.label(match run.phase {
                PLAYING => "Defending",
                WON => "Victory",
                LOST => "Defeat",
                _ => "Unknown phase",
            });
            ui.label(format!(
                "Wave {}/{} · Reactor {} · Gold {} / {} · Kills {} · Leaks {}",
                run.wave,
                frame.singleton::<Campaign>().last_wave,
                run.reactor_hp,
                run.gold[0],
                run.gold[1],
                run.kills,
                run.leaks,
            ));
        });
        let ready = context.phase == GameplayViewPhase::Live && context.input_ready;
        if !ready {
            ui.label("Play and Take control to send commands");
        }
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.pick, PickCell::Source, "Pick source");
            ui.selectable_value(&mut self.pick, PickCell::Target, "Pick target");
            ui.label("Source cell");
            if ui
                .add(egui::DragValue::new(&mut self.source).range(0..=11))
                .changed()
            {
                self.selected_source = Some(self.source);
            }
            ui.label("Target cell");
            if ui
                .add(egui::DragValue::new(&mut self.target).range(0..=11))
                .changed()
            {
                self.selected = Some(self.target);
            }
        });
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            for (label, id) in [
                ("Summon", SUMMON),
                ("Move / merge", MOVE_OR_MERGE),
                ("Upgrade", UPGRADE),
                ("Invest", INVEST),
                ("Restart", RESTART),
            ] {
                if ui.add_enabled(ready, egui::Button::new(label)).clicked() {
                    action = Some(id);
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            for hint in cost_hints(run) {
                ui.small(hint).on_hover_text(
                    "Snapshot cost only; slot availability, phase and later state can still reject an action.",
                );
            }
        });
        ui.collapsing("Details", |ui| {
            // Project help is bounded; generic command status and errors remain
            // in the editor chrome and are never hidden by this disclosure.
            egui::ScrollArea::vertical()
                .id_salt("stormglass-details")
                .max_height(100.0)
                .show(ui, |ui| {
                    ui.label("Edit Scenario resources / Campaign last wave (1–10) before Play");
                    ui.label(
                        "Completion means consumed, not necessarily a successful purchase or move.",
                    );
                    ui.label("Check gold, units and the board to confirm the action result.");
                    ui.label("Costs use the displayed snapshot, not an execution guarantee. Buttons still use the normal control-readiness rules.");
                    ui.label(
                        "Pick source (orange) or target (teal), then click your left board. Move / merge sends the action.",
                    );
                    ui.label("Summon uses the first empty cell, not the selected target.");
                    ui.label("Active enemies (authoritative snapshot)");
                    for enemy in frame
                        .dense::<DefenseEnemy>()
                        .1
                        .iter()
                        .filter(|e| e.active != 0)
                    {
                        ui.label(format!(
                            "#{} lane {} · HP {}/{} · progress {}",
                            enemy.id, enemy.lane, enemy.hp, enemy.max_hp, enemy.progress
                        ));
                    }
                });
        });
        let available = ui.available_size().max(egui::vec2(1.0, 1.0));
        let (rect, response) = ui.allocate_exact_size(available, egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Stormglass board")
        });
        let clip = rect.intersect(ui.clip_rect());
        let layout = orr_sample::defense_view::Layout::new(rect);
        if response.clicked() {
            if let Some(cell) = response
                .interact_pointer_pos()
                .and_then(|point| layout.hit_cell_clipped(clip, point))
            {
                match self.pick {
                    PickCell::Source => {
                        self.source = cell;
                        self.selected_source = Some(cell);
                    }
                    PickCell::Target => {
                        self.target = cell;
                        self.selected = Some(cell);
                    }
                }
            }
        }
        orr_sample::defense_view::paint_view(
            &ui.painter().with_clip_rect(clip),
            layout,
            frame,
            self.selected,
        );
        // Source uses an inner orange outline, distinct even when both picks
        // name the same cell. The existing outer teal outline remains target.
        if let Some(cell) = self.selected_source {
            ui.painter().with_clip_rect(clip).rect_stroke(
                layout.cell(0, cell).shrink(3.0 * layout.scale),
                2.0 * layout.scale,
                egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 165, 64)),
                egui::StrokeKind::Inside,
            );
        }
        action
            .map(|action| {
                IntentRequest::new(
                    "act",
                    json!({
                        "action":action,"source_cell":self.source,"target_cell":self.target
                    }),
                )
            })
            .transpose()
    }
    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Separate descriptor: never sent through the two-game Delivery/Switchboard
/// callback branch. The Scenario reflection registry matches the host.
pub fn descriptor() -> GameplayDescriptor {
    GameplayDescriptor {
        identity: RemoteIdentity {
            game: coop_defense::GAME.into(),
            build_id: format!("0x{:016x}", orr_games::coop_defense_game::SCENARIO_BUILD_ID),
            schema: serde_json::from_str(&scenario_types().legacy_json_schema())
                .expect("Scenario scene schema"),
        },
        local_alias: "stormglass",
        types: scenario_types,
        host,
        stream,
        presentation: PresentationCapability::Embedded {
            create: || Box::<StormglassView>::default(),
        },
        input: InputCapability::Commands(CommandContract {
            input: GameplaySchema {
                type_name: "DefenseInput",
                types: input_types,
            },
            max_players: 1,
            neutral: neutral(),
            max_sequence: u64::from(u32::MAX),
            intents: vec![IntentSchema {
                name: "act",
                payload: GameplaySchema {
                    type_name: "DefenseAction",
                    types: intent_types,
                },
            }],
            encode,
            observe,
        }),
    }
}

/// Application-selected opt-in registry; existing game descriptors stay intact.
pub fn registry() -> &'static GameplayContracts {
    static REGISTRY: OnceLock<GameplayContracts> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut descriptors = crate::gameplay::descriptors();
        descriptors.push(descriptor());
        GameplayContracts::new(descriptors, None).expect("compiled Stormglass contracts")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_editor::static_adapter::CommandScope;
    use orr_games::coop_defense_game::{DefenseConfig, DefenseUnit, TICK_RATE};
    use orr_session::{LocalInputSource, Session, SessionConfig};
    use orr_sim::PlayerSlot;

    fn context(sequence: u64) -> CommandContext {
        CommandContext {
            sequence,
            player: 0,
            scope: CommandScope {
                scene_epoch: 1,
                play_epoch: 1,
            },
        }
    }
    fn session() -> Session<DefenseGame, LocalInputSource> {
        let config = DefenseConfig::default();
        Session::new(
            config,
            SessionConfig::new(1, PlayerSlot(0), u64::from(config.seed), TICK_RATE),
            LocalInputSource,
        )
    }
    #[test]
    fn registration_is_separate_and_rejects_invalid_intents() {
        let game = registry().get("stormglass").unwrap();
        assert_eq!(game.identity().game, coop_defense::GAME);
        assert_eq!(game.neutral_value(), &neutral());
        assert!(registry().get("delivery").is_some());
        assert!(registry().get("switchboard").is_some());
        for action in 1..=5 {
            assert!(game
                .validate_intent(
                    IntentRequest::new(
                        "act",
                        json!({
                            "action":action,"source_cell":0,"target_cell":11
                        })
                    )
                    .unwrap()
                )
                .is_ok());
        }
        for bad in [
            json!({"action":0,"source_cell":0,"target_cell":0}),
            json!({"action":6,"source_cell":0,"target_cell":0}),
            json!({"action":1,"source_cell":12,"target_cell":0}),
            json!({"action":1,"source_cell":0,"target_cell":12}),
            json!({"action":1,"source_cell":0,"target_cell":0,"sequence":1}),
            J::Null,
        ] {
            assert!(game
                .validate_intent(IntentRequest::new("act", bad).unwrap())
                .is_err());
        }
    }
    #[test]
    fn codec_bounds_and_consumption_do_not_claim_action_success() {
        let mut session = session();
        let payload = json!({"action":MOVE_OR_MERGE,"source_cell":0,"target_cell":1});
        let view = FrameView::of(session.predicted_frame());
        let encoded = encode("act", &payload, context(1), view).unwrap();
        assert_eq!(
            observe(view, &encoded.receipt, context(1)).unwrap(),
            Consumption::Pending
        );
        assert!(encode("act", &payload, context(0), view).is_err());
        assert!(encode("act", &payload, context(u64::from(u32::MAX) + 1), view).is_err());
        assert_eq!(
            encode("act", &payload, context(u64::from(u32::MAX)), view)
                .unwrap()
                .input["sequence"],
            u32::MAX
        );
        let mut wrong_player = context(1);
        wrong_player.player = 1;
        assert!(encode("act", &payload, wrong_player, view).is_err());
        // Moving an empty cell is a game-level no-op, but the input is consumed.
        session.advance(
            DefenseInput {
                action: MOVE_OR_MERGE,
                source_cell: 0,
                target_cell: 1,
                sequence: 1,
            },
            vec![],
        );
        // SessionConfig defaults to delayed local input: advance submits at
        // head + 1 + input_delay, rather than consuming this action immediately.
        for _ in 0..session.config().input_delay {
            assert_eq!(
                observe(
                    FrameView::of(session.predicted_frame()),
                    &encoded.receipt,
                    context(1)
                )
                .unwrap(),
                Consumption::Pending
            );
            session.advance(DefenseInput::default(), vec![]);
        }
        let view = FrameView::of(session.predicted_frame());
        assert_eq!(view.singleton::<DefenseRun>().merges, 0);
        assert_eq!(
            observe(view, &encoded.receipt, context(1)).unwrap(),
            Consumption::Consumed
        );
        assert!(encode("act", &payload, context(1), view).is_err());
        assert_eq!(
            observe(view, &encoded.receipt, context(2)).unwrap(),
            Consumption::Invalid
        );
        session.advance(
            DefenseInput {
                action: RESTART,
                sequence: 2,
                ..Default::default()
            },
            vec![],
        );
        for _ in 0..session.config().input_delay {
            session.advance(DefenseInput::default(), vec![]);
        }
        // A repeated held/predicted sequence must not restart twice.
        for _ in 0..=session.config().input_delay {
            session.advance(
                DefenseInput {
                    action: RESTART,
                    sequence: 2,
                    ..Default::default()
                },
                vec![],
            );
        }
        let view = FrameView::of(session.predicted_frame());
        assert_eq!(view.singleton::<DefenseRun>().generation, 1);
        assert_eq!(view.singleton::<DefenseRun>().last_sequence, 2);
        assert_eq!(
            observe(view, &encoded.receipt, context(1)).unwrap(),
            Consumption::Invalid
        );
    }
    #[test]
    fn action_cost_hints_cover_gold_edges_and_shared_costs() {
        let mut run = *session().predicted_frame().singleton::<DefenseRun>();
        for level in 0..MAX_UPGRADE {
            run.upgrades[0] = level;
            let cost = upgrade_cost(level);
            for gold in [0, cost - 1, cost, cost + 1, u32::MAX] {
                run.gold[0] = gold;
                let expected = if gold < cost {
                    format!("Upgrade: {cost} gold (need {} more)", cost - gold)
                } else {
                    format!("Upgrade: {cost} gold")
                };
                assert_eq!(cost_hints(&run)[1], expected);
            }
        }
        for level in 0..MAX_INVESTMENT {
            run.investment[0] = level;
            let cost = investment_cost(level);
            for gold in [cost - 1, cost, cost + 1] {
                run.gold[0] = gold;
                assert_eq!(cost_hints(&run)[2], cost_hint("Invest", gold, Some(cost)));
            }
        }
        for gold in [0, SUMMON_COST - 1, SUMMON_COST, u32::MAX] {
            run.gold[0] = gold;
            assert_eq!(
                cost_hints(&run)[0],
                cost_hint("Summon", gold, Some(SUMMON_COST))
            );
        }
        assert_eq!(
            cost_hint("Summon", 24, Some(SUMMON_COST)),
            "Summon: 25 gold (need 1 more)"
        );
        assert_eq!(
            cost_hint("Invest", 59, Some(investment_cost(0))),
            "Invest: 60 gold (need 1 more)"
        );
    }

    #[test]
    fn action_cost_hints_caps_override_gold_and_ignore_remote_player() {
        let mut run = *session().predicted_frame().singleton::<DefenseRun>();
        for gold in [0, u32::MAX] {
            run.gold = [gold, 777];
            for level in [MAX_UPGRADE, MAX_UPGRADE + 1, u32::MAX] {
                run.upgrades[0] = level;
                assert_eq!(cost_hints(&run)[1], "Upgrade: maximum reached");
            }
            for level in [MAX_INVESTMENT, MAX_INVESTMENT + 1, u32::MAX] {
                run.investment[0] = level;
                assert_eq!(cost_hints(&run)[2], "Invest: maximum reached");
            }
        }
        run.gold[0] = 24;
        run.upgrades = [0, MAX_UPGRADE];
        run.investment = [0, MAX_INVESTMENT];
        assert_eq!(
            cost_hints(&run),
            [
                "Summon: 25 gold (need 1 more)",
                "Upgrade: 60 gold (need 36 more)",
                "Invest: 60 gold (need 36 more)",
            ]
        );
    }

    #[cfg(feature = "api-acceptance")]
    #[test]
    fn action_cost_hints_do_not_gate_or_change_command_payloads() {
        use egui_kittest::{kittest::Queryable, Harness};
        for (gold, upgrade, investment) in [
            (0, 0, 0),
            (150, 0, 0),
            (u32::MAX, MAX_UPGRADE, MAX_INVESTMENT),
        ] {
            let mut frame = session().predicted_frame().clone();
            let run = frame.singleton_mut::<DefenseRun>();
            run.gold[0] = gold;
            run.upgrades[0] = upgrade;
            run.investment[0] = investment;
            let expected_hints = cost_hints(run);
            let before = frame.to_bytes();
            let view = FrameView::of(&frame);
            let presenter = StormglassView {
                source: 3,
                target: 7,
                generation: Some(view.singleton::<DefenseRun>().generation),
                ..Default::default()
            };
            let mut h = Harness::builder().with_size([960.0, 650.0]).build_ui_state(
                |ui, state: &mut (StormglassView, Vec<IntentRequest>)| {
                    if let Some(intent) = state
                        .0
                        .show(
                            ui,
                            view,
                            GameplayViewContext {
                                phase: GameplayViewPhase::Live,
                                input_ready: true,
                            },
                        )
                        .unwrap()
                    {
                        state.1.push(intent);
                    }
                },
                (presenter, Vec::new()),
            );
            h.run_steps(3);
            assert!(h.state().1.is_empty());
            for hint in expected_hints {
                assert!(h.query_by_label(&hint).is_some(), "missing {hint}");
            }
            for (label, action) in [("Summon", SUMMON), ("Upgrade", UPGRADE), ("Invest", INVEST)] {
                h.get_by_label(label).click();
                h.run_steps(3);
                assert_eq!(h.state().1.len(), 1);
                let game = registry().get("stormglass").unwrap();
                let valid = game
                    .validate_intent(h.state_mut().1.pop().unwrap())
                    .unwrap();
                assert_eq!(
                    game.encode(&valid, context(1), view).unwrap().input(),
                    &json!({"action":action,"source_cell":3,"target_cell":7,"sequence":1})
                );
            }
            assert_eq!(frame.to_bytes(), before);
        }
    }

    #[cfg(feature = "api-acceptance")]
    #[test]
    fn board_pick_source_target_emit_only_explicit_move_payload() {
        use egui_kittest::{kittest::Queryable, Harness};
        let session = session();
        let frame = FrameView::of(session.predicted_frame());
        let before = session.predicted_frame().to_bytes();
        let mut h = Harness::builder().with_size([960.0, 650.0]).build_ui_state(
            |ui, state: &mut (StormglassView, Vec<IntentRequest>)| {
                if let Some(intent) = state
                    .0
                    .show(
                        ui,
                        frame,
                        GameplayViewContext {
                            phase: GameplayViewPhase::Live,
                            input_ready: true,
                        },
                    )
                    .unwrap()
                {
                    state.1.push(intent);
                }
            },
            (StormglassView::default(), Vec::new()),
        );
        h.run_steps(3);
        let click = |h: &mut Harness<'_, (StormglassView, Vec<IntentRequest>)>, point| {
            h.event(egui::Event::PointerMoved(point));
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            h.run_steps(3);
        };
        h.get_by_label("Pick source").click();
        h.run_steps(3);
        let layout =
            orr_sample::defense_view::Layout::new(h.get_by_label("Stormglass board").rect());
        click(&mut h, layout.cell(0, 3).center());
        assert_eq!(
            (h.state().0.source, h.state().0.selected_source),
            (3, Some(3))
        );
        h.get_by_label("Pick target").click();
        h.run_steps(3);
        for _ in 0..3 {
            click(&mut h, layout.cell(0, 7).center());
        }
        assert_eq!(
            (h.state().0.source, h.state().0.target, h.state().0.selected),
            (3, 7, Some(7))
        );
        // Remote board, background and points outside the allocated canvas are not picks.
        click(&mut h, layout.cell(1, 2).center());
        click(&mut h, layout.p(600.0, 30.0));
        click(&mut h, egui::pos2(-10.0, -10.0));
        assert_eq!((h.state().0.source, h.state().0.target), (3, 7));
        assert!(h.state().1.is_empty());
        h.get_by_label("Move / merge").click();
        h.run_steps(3);
        assert_eq!(h.state().1.len(), 1);
        let intent = h.state_mut().1.pop().unwrap();
        let game = registry().get("stormglass").unwrap();
        let valid = game.validate_intent(intent).unwrap();
        assert_eq!(
            game.encode(&valid, context(1), frame).unwrap().input(),
            &json!({"action":MOVE_OR_MERGE,"source_cell":3,"target_cell":7,"sequence":1})
        );
        assert_eq!(session.predicted_frame().to_bytes(), before);
    }

    #[test]
    fn board_pick_generation_reset_clears_both_outlines_and_mode() {
        let session = session();
        let frame = FrameView::of(session.predicted_frame());
        let mut presenter = StormglassView {
            source: 3,
            target: 7,
            selected: Some(7),
            selected_source: Some(3),
            pick: PickCell::Source,
            generation: Some(u32::MAX),
        };
        let context = egui::Context::default();
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            assert!(presenter
                .show(
                    ui,
                    frame,
                    GameplayViewContext {
                        phase: GameplayViewPhase::Live,
                        input_ready: true,
                    }
                )
                .unwrap()
                .is_none());
        });
        output.textures_delta.clear();
        assert_eq!((presenter.source, presenter.target), (0, 0));
        assert_eq!(
            (presenter.selected_source, presenter.selected),
            (None, None)
        );
        assert_eq!(presenter.pick, PickCell::Target);
        assert_eq!(
            presenter.generation,
            Some(frame.singleton::<DefenseRun>().generation)
        );
    }

    #[cfg(feature = "api-acceptance")]
    #[test]
    fn compact_hud_details_preserve_selection_and_emit_no_commands() {
        use egui_kittest::{kittest::Queryable, Harness};
        let session = session();
        let frame = FrameView::of(session.predicted_frame());
        let before = session.predicted_frame().to_bytes();
        let presenter = StormglassView {
            source: 3,
            target: 7,
            selected: Some(7),
            generation: Some(frame.singleton::<DefenseRun>().generation),
            ..Default::default()
        };
        let mut h = Harness::builder().with_size([960.0, 650.0]).build_ui_state(
            |ui, presenter: &mut StormglassView| {
                assert!(presenter
                    .show(
                        ui,
                        frame,
                        GameplayViewContext {
                            phase: GameplayViewPhase::Live,
                            input_ready: true,
                        }
                    )
                    .unwrap()
                    .is_none());
            },
            presenter,
        );
        h.run_steps(3);
        for _ in 0..3 {
            h.get_by_label("Details").click();
            h.run_steps(3);
            for label in ["Summon", "Move / merge", "Upgrade", "Invest", "Restart"] {
                assert!(h.query_by_label(label).is_some(), "missing action {label}");
            }
            assert_eq!(
                (h.state().source, h.state().target, h.state().selected),
                (3, 7, Some(7))
            );
            h.get_by_label("Details").click();
            h.run_steps(3);
            assert_eq!(
                (h.state().source, h.state().target, h.state().selected),
                (3, 7, Some(7))
            );
        }
        assert_eq!(session.predicted_frame().to_bytes(), before);
    }

    #[cfg(feature = "api-acceptance")]
    #[test]
    fn compact_hud_unavailable_controls_and_readable_phase_remain_visible() {
        use egui_kittest::{kittest::Queryable, Harness};
        let session = session();
        let frame = FrameView::of(session.predicted_frame());
        let mut h = Harness::builder().with_size([390.0, 650.0]).build_ui_state(
            |ui, presenter: &mut StormglassView| {
                assert!(presenter
                    .show(
                        ui,
                        frame,
                        GameplayViewContext {
                            phase: GameplayViewPhase::Edit,
                            input_ready: false,
                        }
                    )
                    .unwrap()
                    .is_none());
            },
            StormglassView::default(),
        );
        h.run_steps(3);
        assert!(h.query_by_label("Defending").is_some());
        assert!(h
            .query_by_label("Play and Take control to send commands")
            .is_some());
        for label in [
            "Source cell",
            "Target cell",
            "Summon",
            "Move / merge",
            "Upgrade",
            "Invest",
            "Restart",
            "Details",
        ] {
            assert!(h.query_by_label(label).is_some(), "missing control {label}");
        }
    }

    #[test]
    fn presenter_reads_snapshot_without_mutating_simulation() {
        let session = session();
        let before = session.predicted_frame().to_bytes();
        let context = egui::Context::default();
        let mut presenter = StormglassView::default();
        for phase in [
            GameplayViewPhase::Edit,
            GameplayViewPhase::Paused,
            GameplayViewPhase::Live,
            GameplayViewPhase::Viewer,
        ] {
            let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                assert!(presenter
                    .show(
                        ui,
                        FrameView::of(session.predicted_frame()),
                        GameplayViewContext {
                            phase,
                            input_ready: false
                        }
                    )
                    .unwrap()
                    .is_none());
            });
            // This CPU-only presenter test has no renderer to apply font
            // texture uploads. Explicitly discard them; do not claim GPU QA.
            output.textures_delta.clear();
        }
        assert_eq!(session.predicted_frame().to_bytes(), before);
        presenter.source = 11;
        presenter.target = 10;
        presenter.selected = Some(10);
        presenter.reset();
        assert_eq!((presenter.source, presenter.target), (0, 0));
        assert_eq!(presenter.selected, None);
        assert_eq!(presenter.generation, None);
    }
    #[test]
    fn presenter_generation_change_clears_local_selection_without_commands() {
        let session = session();
        let frame = FrameView::of(session.predicted_frame());
        let before = frame.checksum();
        let mut presenter = StormglassView {
            source: 11,
            target: 10,
            selected: Some(10),
            generation: Some(u32::MAX),
            ..Default::default()
        };
        let context = egui::Context::default();
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            assert!(presenter
                .show(
                    ui,
                    frame,
                    GameplayViewContext {
                        phase: GameplayViewPhase::Live,
                        input_ready: true,
                    }
                )
                .unwrap()
                .is_none());
        });
        output.textures_delta.clear();
        assert_eq!(
            (presenter.source, presenter.target, presenter.selected),
            (0, 0, None)
        );
        assert_eq!(
            presenter.generation,
            Some(frame.singleton::<DefenseRun>().generation)
        );
        assert_eq!(frame.checksum(), before);
    }
    #[allow(
        clippy::disallowed_types,
        reason = "Wall-clock deadlines bound editor acceptance tests, not simulation"
    )]
    fn wait_editor(
        editor: &mut orr_editor::Editor,
        mut done: impl FnMut(&orr_editor::Editor) -> bool,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
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
                std::time::Instant::now() < deadline,
                "Stormglass editor progress deadline"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn saved_project_authored_reopen_summon_neutral_stop_and_admission() {
        use orr_editor::editor::input::CommandStatus;
        use orr_editor::{registered_project::PreparedRegisteredProject, Editor, Mode};
        let directory = tempfile::tempdir().unwrap();
        let scene_path = directory.path().join("stormglass.scene.yaml");
        let manifest_path = directory.path().join("project.json");
        let manifest = include_bytes!("../scenes/stormglass.project.json").to_vec();
        let parsed: J = serde_json::from_slice(&manifest).unwrap();
        assert_eq!(parsed["game"], descriptor().identity.game);
        assert_eq!(parsed["build_id"], descriptor().identity.build_id);
        assert_eq!(
            include_str!("../scenes/stormglass.scene.yaml"),
            coop_defense::INITIAL_SCENE
        );
        assert!(manifest.len() < 65536);
        std::fs::write(&manifest_path, &manifest).unwrap();
        std::fs::write(&scene_path, coop_defense::INITIAL_SCENE).unwrap();
        let mut old_manifest = parsed.clone();
        for old_id in ["0x7b2a3ab98f619dca", "0x04ee32c4873f6941"] {
            old_manifest["build_id"] = json!(old_id);
            std::fs::write(&manifest_path, serde_json::to_vec(&old_manifest).unwrap()).unwrap();
            assert!(PreparedRegisteredProject::open(&manifest_path, registry()).is_err());
        }
        std::fs::write(&manifest_path, &manifest).unwrap();
        let prepared = PreparedRegisteredProject::open(&manifest_path, registry()).unwrap();
        assert_eq!(prepared.game(), registry().handle("stormglass").unwrap());
        // Registered project startup is in-process and has no listener config.
        let mut editor = Editor::open_registered_project(prepared).unwrap();
        editor.sync();
        editor
            .host_call(
                "world.singleton.patch",
                json!({
                    "name":"CoopDefense::Scenario", "path":"",
                    "value":{"starting_gold":250,"reactor_hp":80}
                }),
            )
            .unwrap();
        editor
            .host_call(
                "world.singleton.patch",
                json!({
                    "name":"CoopDefense::Campaign", "path":"last_wave", "value":3
                }),
            )
            .unwrap();
        editor
            .host_call(
                "world.singleton.patch",
                json!({
                    "name":"CoopDefense::WavePacing", "path":"spawn_interval_ticks", "value":70
                }),
            )
            .unwrap();
        editor
            .host_call("scene.save", json!({"write":true}))
            .unwrap();
        let authored_source = std::fs::read_to_string(&scene_path).unwrap();
        drop(editor);
        let reopened = PreparedRegisteredProject::open(&manifest_path, registry()).unwrap();
        let mut editor = Editor::open_registered_project(reopened).unwrap();
        editor.sync();
        let original = editor.host_call("scene.save", J::Null).unwrap();
        let history = editor.host_call("history.list", J::Null).unwrap();
        let summon = || {
            IntentRequest::new(
                "act",
                json!({"action":SUMMON,"source_cell":0,"target_cell":0}),
            )
            .unwrap()
        };
        assert!(editor.submit_gameplay_intent(summon()).is_err());
        editor.play();
        wait_editor(&mut editor, |editor| {
            editor.sim().playing
                && editor
                    .gameplay_frame_evidence()
                    .is_some_and(|e| e.snapshot().tick() > 0)
        });
        editor.claim_gameplay(0).unwrap();
        wait_editor(&mut editor, Editor::can_submit_gameplay);
        let before = *editor
            .gameplay_frame_evidence()
            .unwrap()
            .snapshot()
            .predicted()
            .singleton::<DefenseRun>();
        assert_eq!(
            editor
                .gameplay_frame_evidence()
                .unwrap()
                .snapshot()
                .predicted()
                .singleton::<Campaign>()
                .last_wave,
            3
        );
        assert_eq!(
            editor
                .gameplay_frame_evidence()
                .unwrap()
                .snapshot()
                .predicted()
                .singleton::<orr_games::coop_defense_game::WavePacing>()
                .spawn_interval_ticks,
            70
        );
        assert_eq!(before.reactor_hp, 80);
        assert!(before.gold[0] >= 250);
        let command = editor.submit_gameplay_intent(summon()).unwrap();
        assert!(editor.submit_gameplay_intent(summon()).is_err());
        wait_editor(&mut editor, |editor| {
            matches!(
                editor.gameplay_command_status(command),
                Some(CommandStatus::Completed { .. })
            )
        });
        let Some(CommandStatus::Completed { consumed, neutral }) =
            editor.gameplay_command_status(command)
        else {
            panic!("missing completion evidence")
        };
        assert_eq!(consumed.scope, neutral.scope);
        assert!(neutral.tick > consumed.tick);
        let evidence = editor.gameplay_frame_evidence().unwrap();
        let frame = evidence.snapshot().predicted();
        let run = frame.singleton::<DefenseRun>();
        assert!(run.last_sequence > before.last_sequence);
        assert!(run.summons > before.summons);
        assert!(
            run.gold[0] < before.gold[0],
            "summon must spend player gold"
        );
        assert_eq!(
            frame
                .dense::<DefenseUnit>()
                .1
                .iter()
                .filter(|u| u.board == 0 && u.active != 0)
                .count(),
            1
        );
        let _ = editor.stop();
        wait_editor(&mut editor, |editor| editor.mode() == Mode::Edit);
        editor.sync();
        assert_eq!(editor.host_call("scene.save", J::Null).unwrap(), original);
        assert_eq!(editor.host_call("history.list", J::Null).unwrap(), history);
        assert_eq!(std::fs::read(&manifest_path).unwrap(), manifest);
        assert_eq!(
            std::fs::read_to_string(&scene_path).unwrap(),
            authored_source
        );
        assert!(editor.submit_gameplay_intent(summon()).is_err());
        drop(editor);
        // Reflection permits a component-free entity to parse, but the
        // reused preset host must reject it during authoritative bake admission.
        let nonempty = "schema: orr.scene/1\nentities: {e_00000001: {name: injected}}\n";
        std::fs::write(&scene_path, nonempty).unwrap();
        let prepared = PreparedRegisteredProject::open(&manifest_path, registry()).unwrap();
        let error = match Editor::open_registered_project(prepared) {
            Ok(_) => panic!("nonempty Stormglass scene started"),
            Err(error) => error,
        };
        assert!(error.contains("no entities or prefabs"), "{error}");
        assert_eq!(std::fs::read_to_string(&scene_path).unwrap(), nonempty);
        assert_eq!(std::fs::read(&manifest_path).unwrap(), manifest);
    }

    #[test]
    fn host_rejects_contract_mismatch_before_startup() {
        for wrong_neutral in [false, true] {
            let (work_status, _owner) = orr_remote::work_status::WorkStatusService::pair();
            let mut config = orr_remote::ServerConfig::new(orr_remote::Auth::DevNoAuth);
            config.listen = false;
            let expected = if wrong_neutral {
                expected_input().unwrap()
            } else {
                ExpectedInputDescriptor::new(json!({}), orr_remote::methods::VALUE_FORMAT, 1)
                    .unwrap()
            };
            assert!(host(StaticHostRequest {
                scene_text: coop_defense::INITIAL_SCENE.into(),
                scene_path: "unused.scene.yaml".into(),
                config,
                work_status,
                neutral_value: if wrong_neutral { json!({}) } else { neutral() },
                expected_input: expected,
            })
            .is_err());
        }
    }
}
