//! Application-owned simulation, admission and typed transport boundary.
use crate::{
    delivery::{Courier, Delivery, DeliveryGame, DeliveryInput},
    switchboard::{Board, Cursor, SwitchGame, SwitchInput},
};
use orr_bridge::{Bridge, BridgeEvent, FrameView, ViewUpdate};
use orr_edit::{BakeAdmission, EditError, EditorDoc};
use orr_editor::static_adapter::*;
use orr_reflect::{Reflect, Scene, SceneIndex, TypeRegistry};
use orr_remote::{
    ExpectedInputDescriptor, GameHooks, LocalHost, RemoteBridge, RemoteConfig, RemoteViewDelivery,
    RpcError, Transport,
};
use orr_sim::{Game, Simulation};
use serde_json::json;
use std::sync::{Arc, OnceLock};

pub trait Starter: Game<Config = (), Event = u32, Command = crate::NoCommand> {
    const ID: &'static str;
    const ALIAS: &'static str;
    const INPUT: &'static str;
    const BUILD: u64;
    const SCENE: &'static str;
    fn types() -> TypeRegistry;
    fn validate(frame: &orr_ecs::Frame) -> bool;
    fn valid_input(input: &Self::Input) -> bool;
    fn won(frame: &orr_ecs::Frame) -> bool;
}
impl Starter for DeliveryGame {
    const ID: &'static str = "StarterDelivery";
    const ALIAS: &'static str = "delivery";
    const INPUT: &'static str = "DeliveryInput";
    const BUILD: u64 = crate::delivery::BUILD;
    const SCENE: &'static str = crate::delivery::SCENE;
    fn types() -> TypeRegistry {
        crate::delivery::types()
    }
    fn valid_input(i: &DeliveryInput) -> bool {
        (-1..=1).contains(&i.dx) && (-1..=1).contains(&i.dy) && i.deliver <= 1
    }
    fn won(f: &orr_ecs::Frame) -> bool {
        f.singleton::<Delivery>().won != 0
    }
    fn validate(f: &orr_ecs::Frame) -> bool {
        let d = f.singleton::<Delivery>();
        f.dense::<Courier>().0.len() == 1 && d.won == 0 && d.action_held == 0
    }
}
impl Starter for SwitchGame {
    const ID: &'static str = "StarterSwitchboard";
    const ALIAS: &'static str = "switchboard";
    const INPUT: &'static str = "SwitchInput";
    const BUILD: u64 = crate::switchboard::BUILD;
    const SCENE: &'static str = crate::switchboard::SCENE;
    fn types() -> TypeRegistry {
        crate::switchboard::types()
    }
    fn valid_input(i: &SwitchInput) -> bool {
        [orr_fp::FP::ZERO, orr_fp::FP::ONE, -orr_fp::FP::ONE].contains(&i.selection)
            && i.toggle <= 1
    }
    fn won(f: &orr_ecs::Frame) -> bool {
        f.singleton::<Board>().solved != 0
    }
    fn validate(f: &orr_ecs::Frame) -> bool {
        let b = f.singleton::<Board>();
        let cursors = f.dense::<Cursor>().1;
        cursors.len() == 1
            && cursors[0].selected < 4
            && cursors[0].held == 0
            && b.bits < 16
            && (1..16).contains(&b.target)
            && b.solved == 0
            && b.moves == 0
    }
}
struct Admission<G>(std::marker::PhantomData<fn() -> G>);
impl<G: Starter> BakeAdmission for Admission<G> {
    fn admit(
        &self,
        _: &Scene,
        frame: &mut orr_ecs::Frame,
        _: &SceneIndex,
    ) -> Result<(), EditError> {
        if G::validate(frame) {
            Ok(())
        } else {
            Err(EditError::Invalid(
                "starter requires one valid actor and initial-only state".into(),
            ))
        }
    }
    fn allow_play_edits(&self) -> bool {
        false
    }
}
/// Both editor host and source player call this exact loader/admission.
pub fn document<G: Starter>(text: &str) -> Result<EditorDoc, String> {
    EditorDoc::from_yaml_with_admission(
        text,
        G::types(),
        Simulation::<G>::build_registry(),
        crate::SEED,
        Some(Arc::new(Admission::<G>(std::marker::PhantomData))),
    )
    .map_err(|e| e.to_string())
}
pub fn player<G: Starter>(text: &str) -> Result<orr_session::PlaySession<G>, String> {
    let doc = document::<G>(text)?;
    let mut config = doc.play_config(1, crate::TICK_RATE);
    config.build_id = G::BUILD;
    orr_session::PlaySession::from_frame(config, doc.frame()).map_err(|e| e.to_string())
}
fn inputs<G: Starter>() -> TypeRegistry
where
    G::Input: Reflect,
{
    let mut t = TypeRegistry::new();
    t.register_component::<G::Input>(G::INPUT);
    t
}
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
fn circle(
    entity: orr_ecs::Entity,
    p: orr_fp::FPVec2,
    color: [f32; 4],
) -> orr_editor::game::Drawable {
    orr_editor::game::Drawable {
        entity,
        pos: [orr_view::fp_to_f32(p.x), orr_view::fp_to_f32(p.y)],
        angle: 0.0,
        turn: 0.0,
        style: orr_view::Style {
            shape: orr_view::Shape::Circle,
            size: 0.4,
            half_y: 0.0,
            color,
        },
        outline: orr_sample::editor_view::Outline::Circle { radius: 0.4 },
    }
}
fn draw_delivery(f: FrameView<'_>) -> Vec<orr_editor::game::Drawable> {
    let d = f.singleton::<Delivery>();
    let mut v: Vec<_> = f
        .iter::<Courier>()
        .map(|(e, c)| {
            circle(
                e,
                c.position,
                if d.won != 0 {
                    [0.2, 1.0, 0.2, 1.0]
                } else {
                    [0.3, 0.6, 1.0, 1.0]
                },
            )
        })
        .collect();
    v.push(circle(orr_ecs::Entity::NONE, d.goal, [1.0, 0.8, 0.2, 1.0]));
    v
}
fn draw_switch(f: FrameView<'_>) -> Vec<orr_editor::game::Drawable> {
    let b = f.singleton::<Board>();
    let mut v: Vec<_> = f
        .iter::<Cursor>()
        .map(|(e, c)| circle(e, c.position, [0.3, 0.6, 1.0, 1.0]))
        .collect();
    for i in 0..4 {
        v.push(circle(
            orr_ecs::Entity::NONE,
            orr_fp::FPVec2::new(
                orr_fp::FP::from_raw(i * (2 << 16)),
                orr_fp::FP::from_raw(2 << 16),
            ),
            if b.bits & (1 << i) != 0 {
                [0.2, 1.0, 0.2, 1.0]
            } else {
                [0.3, 0.3, 0.3, 1.0]
            },
        ));
    }
    v
}
pub fn descriptors() -> Vec<StaticGameDescriptor> {
    vec![StaticGameDescriptor {
        game_id: DeliveryGame::ID, local_alias: DeliveryGame::ALIAS, build_id: DeliveryGame::BUILD,
        types: DeliveryGame::types, host: host::<DeliveryGame>, stream: stream::<DeliveryGame>, draw: draw_delivery,
        position: StaticPosition { component: "Courier", field: "position" },
        input: StaticInputDescriptor { type_name: "DeliveryInput", types: inputs::<DeliveryGame>, max_players: 1,
            hint: "WASD/arrows move; Space delivers at yellow goal; release then Space restarts",
            held: |k| json!({"dx":k.x,"dy":k.y,"deliver":u32::from(k.action)}) },
    }, StaticGameDescriptor {
        game_id: SwitchGame::ID, local_alias: SwitchGame::ALIAS, build_id: SwitchGame::BUILD,
        types: SwitchGame::types, host: host::<SwitchGame>, stream: stream::<SwitchGame>, draw: draw_switch,
        position: StaticPosition { component: "Cursor", field: "position" },
        input: StaticInputDescriptor { type_name: "SwitchInput", types: inputs::<SwitchGame>, max_players: 1,
            hint: "Left/right selects a switch; Space toggles; match target bits (default 0101); Space restarts",
            held: |k| json!({"selection":k.x,"toggle":u32::from(k.action)}) },
    }]
}
pub fn registry() -> &'static StaticGameRegistry {
    static REGISTRY: OnceLock<StaticGameRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        StaticGameRegistry::new(descriptors()).expect("compiled starter descriptors")
    })
}

/// Finite data, not a script. Validate the entire trace before any tick runs.
/// Each input is decoded using the exact reflected type used by ERP admission.
pub fn trace<G: Starter>(bytes: &[u8]) -> Result<Vec<(u32, G::Input)>, String>
where
    G::Input: Reflect,
{
    if bytes.len() > 65_536 {
        return Err("trace exceeds 64 KiB".into());
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let steps = value.as_array().ok_or("trace must be an array")?;
    if steps.is_empty() || steps.len() > 64 {
        return Err("trace requires 1..=64 steps".into());
    }
    let input_registry = inputs::<G>();
    let info = input_registry.get(G::INPUT).ok_or("missing typed input")?;
    let mut total = 0;
    let mut out = Vec::with_capacity(steps.len());
    for step in steps {
        let object = step.as_object().ok_or("trace step must be an object")?;
        if object.len() != 2 || !object.contains_key("ticks") || !object.contains_key("input") {
            return Err("trace step requires exactly ticks and input".into());
        }
        let ticks = object["ticks"]
            .as_u64()
            .filter(|n| (1..=600).contains(n))
            .ok_or("ticks must be an integer in 1..=600")? as u32;
        total += ticks;
        if total > 600 {
            return Err("trace exceeds 600 total ticks".into());
        }
        let value = orr_remote::json::json_to_value(info.desc(), &object["input"], false)?;
        let mut bytes = info.default_bytes();
        info.set(&mut bytes, "", value).map_err(|e| e.to_string())?;
        let input =
            bytemuck::try_pod_read_unaligned::<G::Input>(&bytes).map_err(|e| e.to_string())?;
        if !G::valid_input(&input) {
            return Err("trace input is outside the finite held-key domain".into());
        }
        out.push((ticks, input));
    }
    Ok(out)
}
