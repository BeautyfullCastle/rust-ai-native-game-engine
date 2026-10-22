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
