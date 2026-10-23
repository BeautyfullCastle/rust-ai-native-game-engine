//! Shared read-only game presentation and finite typed key mapping.
use crate::{
    delivery::{Courier, Delivery, DeliveryGame, DeliveryInput},
    game_contract::{HeldKeys, Starter},
    switchboard::{Board, Cursor, SwitchGame, SwitchInput},
};
use orr_bridge::FrameView;
use orr_ecs::Entity;
use orr_fp::{FPVec2, FP};
use orr_view::{RenderItem, Shape, Style, Transform2};
use serde_json::{json, Value};

/// Application-owned adapter, shared by editor and native player. Not a game VM.
pub trait StandaloneGame: Starter {
    fn held_input(keys: HeldKeys) -> Self::Input;
    /// Opt-in bounded controls. Existing games retain their legacy input path.
    #[cfg(feature = "static-game-actions")]
    fn action_profile() -> Option<orr_input::HeldActionProfile> {
        None
    }
    /// Called only while admitting the complete finite input table, never per frame.
    #[cfg(feature = "static-game-actions")]
    fn held_actions_input(_held: orr_input::HeldActions) -> Self::Input {
        Self::Input::default()
    }
    fn input_value(input: &Self::Input) -> Value;
    fn extract(frame: FrameView<'_>, out: &mut Vec<RenderItem>);
    fn state_value(frame: FrameView<'_>) -> Value;
}
fn circle(entity: Entity, position: FPVec2, color: [f32; 4]) -> RenderItem {
    RenderItem {
        entity,
        transform: Transform2 {
            pos: orr_view::fp_to_vec2(position),
            rot: 0.0,
        },
        style: Style {
            shape: Shape::Circle,
            size: 0.4,
            half_y: 0.0,
            color,
        },
    }
}
impl StandaloneGame for DeliveryGame {
    fn held_input(k: HeldKeys) -> DeliveryInput {
        DeliveryInput {
            dx: i32::from(k.x.clamp(-1, 1)),
            dy: i32::from(k.y.clamp(-1, 1)),
            deliver: u32::from(k.action),
        }
    }
    fn input_value(i: &DeliveryInput) -> Value {
        json!({"dx":i.dx,"dy":i.dy,"deliver":i.deliver})
    }
    fn extract(f: FrameView<'_>, out: &mut Vec<RenderItem>) {
        out.clear();
        let d = f.singleton::<Delivery>();
        out.extend(f.iter::<Courier>().map(|(e, c)| {
            circle(
                e,
                c.position,
                if d.won != 0 {
                    [0.2, 1.0, 0.2, 1.0]
                } else {
                    [0.3, 0.6, 1.0, 1.0]
                },
            )
        }));
        out.push(circle(Entity::NONE, d.goal, [1.0, 0.8, 0.2, 1.0]));
    }
    fn state_value(f: FrameView<'_>) -> Value {
        let d = f.singleton::<Delivery>();
        let p = f
            .iter::<Courier>()
            .next()
            .expect("admitted courier")
            .1
            .position;
        json!({"position":[p.x.raw(),p.y.raw()],"goal":[d.goal.x.raw(),d.goal.y.raw()],"won":d.won,"held":d.action_held})
    }
}
impl StandaloneGame for SwitchGame {
    fn held_input(k: HeldKeys) -> SwitchInput {
        SwitchInput {
            selection: FP::from_int(i32::from(k.x.clamp(-1, 1))),
            toggle: u32::from(k.action),
            pad: 0,
        }
    }
    fn input_value(i: &SwitchInput) -> Value {
        // The admitted held domain is exactly integer -1, 0, 1; no float conversion.
        json!({"selection":i.selection.to_int(),"toggle":i.toggle})
    }
    fn extract(f: FrameView<'_>, out: &mut Vec<RenderItem>) {
        out.clear();
        let b = f.singleton::<Board>();
        out.extend(
            f.iter::<Cursor>()
                .map(|(e, c)| circle(e, c.position, [0.3, 0.6, 1.0, 1.0])),
        );
        for i in 0..4 {
            out.push(circle(
                Entity::NONE,
                FPVec2::new(FP::from_raw(i * (2 << 16)), FP::from_raw(2 << 16)),
                if b.bits & (1 << i) != 0 {
                    [0.2, 1.0, 0.2, 1.0]
                } else {
                    [0.3, 0.3, 0.3, 1.0]
                },
            ));
        }
    }
    fn state_value(f: FrameView<'_>) -> Value {
        let b = f.singleton::<Board>();
        let c = f.iter::<Cursor>().next().expect("admitted cursor").1;
        json!({"position":[c.position.x.raw(),c.position.y.raw()],"selected":c.selected,"held":c.held,"bits":b.bits,"target":b.target,"moves":b.moves,"won":b.solved})
    }
}

/// Immutable typed samples admitted once for a declared action profile.
#[cfg(feature = "static-game-actions")]
pub struct FrozenActionInputs<G: StandaloneGame> {
    profile: orr_input::HeldActionProfile,
    values: Vec<G::Input>,
}
#[cfg(feature = "static-game-actions")]
impl<G: StandaloneGame> FrozenActionInputs<G> {
    pub fn admit(profile: orr_input::HeldActionProfile) -> Result<Self, String> {
        const MAX_TOTAL_BYTES: usize = 4 * 1024 * 1024;
        let registry = crate::game_contract::inputs::<G>();
        let schema = registry
            .type_json_schema(G::INPUT)
            .ok_or("missing input schema")?;
        let descriptor = serde_json::to_vec(profile.actions()).map_err(|e| e.to_string())?;
        let identity = serde_json::to_vec(&json!({"game":G::ID,"input":G::INPUT,"build":G::BUILD}))
            .map_err(|e| e.to_string())?;
        let mut total = schema
            .len()
            .checked_add(descriptor.len())
            .and_then(|n| n.checked_add(identity.len()))
            .ok_or("input budget overflow")?;
        let mut values = Vec::with_capacity(profile.state_count());
        for held in profile.states() {
            let (input, _, encoded_len) = admit_action_sample::<G>(held, &registry)?;
            total = total
                .checked_add(encoded_len)
                .ok_or("input budget overflow")?;
            if total > MAX_TOTAL_BYTES {
                return Err("action table exceeds 4 MiB".into());
            }
            values.push(input);
        }
        Ok(Self { profile, values })
    }
    pub fn profile(&self) -> &orr_input::HeldActionProfile {
        &self.profile
    }
    /// Invalid externally supplied axes or masks are rejected, never truncated.
    pub fn get(&self, held: orr_input::HeldActions) -> Option<G::Input> {
        self.profile
            .index(held)
            .and_then(|index| self.values.get(index))
            .copied()
    }
}

/// Shared admission semantics for the native table and optional editor companion.
#[cfg(feature = "static-game-actions")]
pub(crate) fn admit_action_sample<G: StandaloneGame>(
    held: orr_input::HeldActions,
    registry: &orr_reflect::TypeRegistry,
) -> Result<(G::Input, Value, usize), String> {
    const MAX_VALUE_BYTES: usize = 64 * 1024;
    if std::mem::size_of::<G::Input>() > MAX_VALUE_BYTES {
        return Err("typed action sample exceeds 64 KiB".into());
    }
    let info = registry.get(G::INPUT).ok_or("missing typed input")?;
    let input = G::held_actions_input(held);
    if !G::valid_input(&input) {
        return Err("action sample is outside typed input domain".into());
    }
    let value = G::input_value(&input);
    let encoded_len = serde_json::to_vec(&value).map_err(|e| e.to_string())?.len();
    if encoded_len > MAX_VALUE_BYTES {
        return Err("action sample exceeds 64 KiB".into());
    }
    let reflected = orr_remote::json::json_to_value(info.desc(), &value, false)?;
    let mut bytes = info.default_bytes();
    info.set(&mut bytes, "", reflected)
        .map_err(|e| e.to_string())?;
    if bytes != bytemuck::bytes_of(&input) {
        return Err("action JSON does not round-trip to its typed input".into());
    }
    if held == orr_input::HeldActions::default()
        && bytemuck::bytes_of(&input) != bytemuck::bytes_of(&G::Input::default())
    {
        return Err("action profile neutral must equal typed default".into());
    }
    Ok((input, value, encoded_len))
}
