//! Compatibility exports plus the optional editor-only integration.
pub use crate::game_contract::{document, player, trace, Starter};
#[cfg(feature = "editor")]
mod editor {
    use super::*;
    use crate::{
        delivery::DeliveryGame,
        game_contract::{inputs, HeldKeys},
        presentation::StandaloneGame,
        switchboard::SwitchGame,
    };
    use orr_bridge::{Bridge, BridgeEvent, FrameView, ViewUpdate};
    use orr_editor::static_adapter::*;
    use orr_reflect::Reflect;
    use orr_remote::{
        ExpectedInputDescriptor, GameHooks, LocalHost, RemoteBridge, RemoteConfig,
        RemoteViewDelivery, RpcError, Transport,
    };
    use orr_sim::Game;
    use std::sync::OnceLock;
    fn host<G: Starter>(r: StaticHostRequest) -> Result<LocalHost, String>
    where
        G::Input: Reflect,
    {
        let schema = serde_json::from_str(
            &inputs::<G>()
                .type_json_schema(G::INPUT)
                .ok_or("missing input schema")?,
        )
        .map_err(|e| format!("{e}"))?;
        if r.expected_input
            != ExpectedInputDescriptor::new(schema, orr_remote::methods::VALUE_FORMAT, 1)?
        {
            return Err("typed starter input contract mismatch".into());
        }
        let mut cfg = r.config;
        LocalHost::spawn_with_input::<G>(
            move || {
                let doc = document::<G>(&r.scene_text)?;
                cfg.limits.game = GameHooks::new(G::ID);
                cfg.limits.build_id = G::BUILD;
                cfg.limits.player_count = 1;
                cfg.limits.tick_rate = crate::TICK_RATE;
                cfg.limits.scene_path = Some(r.scene_path);
                cfg.limits.view_stream = None;
                Ok((doc, cfg))
            },
            G::INPUT,
            1,
            |_, _| vec![],
            r.neutral_value,
            Some(r.work_status),
        )
    }
    struct Stream<G: Game>(RemoteBridge<G>);
    impl<G: Game> StaticFrameStream for Stream<G> {
        fn poll_view(&mut self) -> ViewUpdate<()> {
            let u = self.0.poll_view();
            ViewUpdate {
                snapshot: u.snapshot,
                resync: u.resync,
                events: u
                    .events
                    .into_iter()
                    .filter_map(|e| match e {
                        BridgeEvent::Lifecycle(l) => Some(BridgeEvent::Lifecycle(l)),
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
        #[cfg(feature = "gameplay")]
        fn frame_evidence(&self) -> Option<orr_remote::RemoteFrameEvidence> {
            self.0.frame_evidence()
        }
    }
    fn stream<G: Starter>(
        t: Box<dyn Transport>,
        cfg: RemoteConfig,
        input: ExpectedInputDescriptor,
    ) -> Result<Box<dyn StaticFrameStream>, String> {
        Ok(Box::new(Stream(
            RemoteBridge::<G>::connect_transport_with_input(t, cfg, input)?,
        )))
    }
    fn draw<G: crate::presentation::StandaloneGame>(
        frame: FrameView<'_>,
    ) -> Vec<orr_editor::game::Drawable> {
        let mut items = Vec::new();
        G::extract(frame, &mut items);
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
    pub fn descriptors() -> Vec<StaticGameDescriptor> {
        vec![StaticGameDescriptor {
        game_id: DeliveryGame::ID, local_alias: DeliveryGame::ALIAS, build_id: DeliveryGame::BUILD,
        types: DeliveryGame::types, host: host::<DeliveryGame>, stream: stream::<DeliveryGame>, draw: draw::<DeliveryGame>,
        position: StaticPosition { component: "Courier", field: "position" },
        input: StaticInputDescriptor { type_name: "DeliveryInput", types: inputs::<DeliveryGame>, max_players: 1,
            hint: "WASD/arrows move; Space delivers at yellow goal; release then Space restarts",
            held: |k| DeliveryGame::input_value(&DeliveryGame::held_input(HeldKeys { x:k.x, y:k.y, action:k.action })) },
    }, StaticGameDescriptor {
        game_id: SwitchGame::ID, local_alias: SwitchGame::ALIAS, build_id: SwitchGame::BUILD,
        types: SwitchGame::types, host: host::<SwitchGame>, stream: stream::<SwitchGame>, draw: draw::<SwitchGame>,
        position: StaticPosition { component: "Cursor", field: "position" },
        input: StaticInputDescriptor { type_name: "SwitchInput", types: inputs::<SwitchGame>, max_players: 1,
            hint: "Left/right selects a switch; Space toggles; match target bits (default 0101); Space restarts",
            held: |k| SwitchGame::input_value(&SwitchGame::held_input(HeldKeys { x:k.x, y:k.y, action:k.action })) },
    }]
    }
    pub fn registry() -> &'static StaticGameRegistry {
        static REGISTRY: OnceLock<StaticGameRegistry> = OnceLock::new();
        REGISTRY.get_or_init(|| {
            StaticGameRegistry::new(descriptors()).expect("compiled starter descriptors")
        })
    }
}
#[cfg(feature = "editor")]
pub use editor::{descriptors, registry};

/// Optional companion for application-owned games using the shared typed hooks.
/// The registry admits and freezes these samples; no game callback runs per frame.
#[cfg(all(feature = "editor", feature = "static-game-actions"))]
pub fn action_descriptor<G: crate::presentation::StandaloneGame>(
) -> Option<orr_editor::static_adapter::StaticActionDescriptor> {
    G::action_profile().map(
        |profile| orr_editor::static_adapter::StaticActionDescriptor {
            game_id: G::ID,
            profile,
            held: |held, types| {
                crate::presentation::admit_action_sample::<G>(held, types)
                    .map(|(_, value, _)| value)
            },
        },
    )
}
