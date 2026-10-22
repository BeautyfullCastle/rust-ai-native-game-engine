//! Private native acceptance game. Never registered by a production executable.
use bytemuck::{Pod, Zeroable};
use orr_bridge::FrameView;
use orr_ecs::{ComponentRegistryBuilder, Entity, Frame};
use orr_fp::{FPVec2, FP};
use orr_input::{Action, Button, HeldActionProfile, HeldActions, Key};
use orr_reflect::{Reflect, TypeRegistry};
use orr_sim::{Game, PlayerSlot, SimContext, System};
use orr_static_starter::{
    game_contract::{HeldKeys, Starter},
    presentation::StandaloneGame,
    NoCommand,
};
use orr_view::{RenderItem, Shape, Style, Transform2};
use serde_json::{json, Value};

pub const MARKER_X: [f32; 4] = [-3.0, -1.0, 1.0, 3.0];
pub const MARKER_Y: f32 = 2.0;
pub const RADIUS: f32 = 0.45;
pub const COLORS: [[f32; 4]; 4] = [
    [1.0, 0.1, 0.1, 1.0],
    [0.1, 1.0, 0.1, 1.0],
    [0.1, 0.2, 1.0, 1.0],
    [1.0, 0.9, 0.1, 1.0],
];
pub const OFF: [f32; 4] = [0.15, 0.15, 0.15, 1.0];
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct Controls {
    pub dx: i32,
    pub dy: i32,
    pub actions: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable, Reflect)]
pub struct State {
    pub x: i32,
    pub y: i32,
    pub jump: u32,
    pub fire: u32,
    pub interact: u32,
    pub shield: u32,
    pub held: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable, Reflect)]
pub struct Actor {
    pub position: FPVec2,
}
pub struct Fixture;
impl Game for Fixture {
    type Input = Controls;
    type Command = NoCommand;
    type Event = u32;
    type Config = ();
    fn register(b: &mut ComponentRegistryBuilder) {
        b.register_singleton::<State>("State");
        b.register_component::<Actor>("Actor");
    }
    fn setup(f: &mut Frame, _: &()) {
        f.set_singleton(State::default());
        let e = f.spawn();
        f.add(e, Actor::default());
    }
    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(step as fn(&mut SimContext<Self>))]
    }
}
fn step(c: &mut SimContext<Fixture>) {
    let i = *c.inputs.input(PlayerSlot(0));
    let mut s = *c.frame.singleton::<State>();
    s.x = s.x.saturating_add(i.dx);
    s.y = s.y.saturating_add(i.dy);
    s.jump = s.jump.saturating_add(i.actions & 1);
    s.fire = s.fire.saturating_add((i.actions >> 1) & 1);
    s.interact = s.interact.saturating_add((i.actions >> 2) & 1);
    s.shield = s.shield.saturating_add((i.actions >> 3) & 1);
    s.held = i.actions;
    c.frame.set_singleton(s);
}
impl Starter for Fixture {
    const ID: &'static str = "PrivateNativeActions";
    const ALIAS: &'static str = "private-native-actions";
    const INPUT: &'static str = "Controls";
    const BUILD: u64 = 0x4d414354494f4e01;
    const SCENE:&'static str="schema: orr.scene/1\nsingletons:\n  State: {x: 0, y: 0, jump: 0, fire: 0, interact: 0, shield: 0, held: 0}\nentities:\n  e_00000001:\n    name: actor\n    Actor: {position: [0, 0]}\n";
    fn types() -> TypeRegistry {
        let mut t = TypeRegistry::new();
        t.register_singleton::<State>("State");
        t.register_component::<Actor>("Actor");
        t
    }
    fn validate(f: &Frame) -> bool {
        let s = f.singleton::<State>();
        f.dense::<Actor>().0.len() == 1
            && [s.x, s.y] == [0, 0]
            && [s.jump, s.fire, s.interact, s.shield, s.held] == [0; 5]
    }
    fn valid_input(i: &Controls) -> bool {
        (-1..=1).contains(&i.dx) && (-1..=1).contains(&i.dy) && i.actions < 16
    }
    fn won(_: &Frame) -> bool {
        false
    }
}
#[allow(
    clippy::float_arithmetic,
    reason = "Private fixture presentation geometry only"
)]
fn circle(pos: [f32; 2], color: [f32; 4], size: f32) -> RenderItem {
    RenderItem {
        entity: Entity::NONE,
        transform: Transform2 {
            pos: orr_view::fp_to_vec2(FPVec2::new(
                FP::from_raw((pos[0] * 65536.0) as i64),
                FP::from_raw((pos[1] * 65536.0) as i64),
            )),
            rot: 0.0,
        },
        style: Style {
            shape: Shape::Circle,
            size,
            half_y: 0.0,
            color,
        },
    }
}
impl StandaloneGame for Fixture {
    fn held_input(k: HeldKeys) -> Controls {
        Controls {
            dx: i32::from(k.x),
            dy: i32::from(k.y),
            actions: u32::from(k.action),
        }
    }
    fn action_profile() -> Option<HeldActionProfile> {
        Some(
            HeldActionProfile::new(
                [
                    ("jump", Key::Space),
                    ("fire", Key::F),
                    ("interact", Key::E),
                    ("shield", Key::Q),
                ]
                .into_iter()
                .map(|(name, key)| Action {
                    name: name.into(),
                    bindings: vec![Button::Keyboard { key }],
                })
                .collect(),
            )
            .unwrap(),
        )
    }
    fn held_actions_input(h: HeldActions) -> Controls {
        Controls {
            dx: i32::from(h.x),
            dy: i32::from(h.y),
            actions: u32::from(h.actions),
        }
    }
    fn input_value(i: &Controls) -> Value {
        json!({"dx":i.dx,"dy":i.dy,"actions":i.actions})
    }
    fn state_value(f: FrameView<'_>) -> Value {
        let s = f.singleton::<State>();
        json!({"x":s.x,"y":s.y,"counts":[s.jump,s.fire,s.interact,s.shield],"held":s.held})
    }
    #[allow(
        clippy::float_arithmetic,
        reason = "Private fixture presentation geometry only"
    )]
    fn extract(f: FrameView<'_>, out: &mut Vec<RenderItem>) {
        let s = f.singleton::<State>();
        out.clear();
        for (n, x) in MARKER_X.into_iter().enumerate() {
            out.push(circle(
                [x, MARKER_Y],
                if s.held & (1 << n) != 0 {
                    COLORS[n]
                } else {
                    OFF
                },
                RADIUS,
            ));
        }
        // Fixed scene anchors bound fit_camera independently of accumulated input.
        for x in [-3.0, 3.0] {
            out.push(circle([x, -2.0], [0.7, 0.7, 0.7, 1.0], RADIUS));
        }
        out.push(circle(
            [
                s.x.clamp(-100, 100) as f32 / 50.0,
                s.y.clamp(-50, 50) as f32 / 50.0,
            ],
            [0.1, 1.0, 1.0, 1.0],
            0.25,
        ));
    }
}

#[cfg(test)]
mod contract_tests {
    use super::*;
    #[test]
    fn private_scene_and_all_action_states_are_admitted() {
        let admission =
            orr_static_starter::game_contract::admit::<Fixture>(Fixture::SCENE).unwrap();
        let session = admission.session().unwrap();
        assert_eq!(
            Fixture::state_value(FrameView::of(session.frame())),
            json!({"x":0,"y":0,"counts":[0,0,0,0],"held":0})
        );
        let profile = Fixture::action_profile().unwrap();
        let frozen =
            orr_static_starter::presentation::FrozenActionInputs::<Fixture>::admit(profile.clone())
                .unwrap();
        assert_eq!(profile.state_count(), 144);
        for state in profile.states() {
            let input = frozen.get(state).unwrap();
            assert_eq!(
                input,
                Controls {
                    dx: i32::from(state.x),
                    dy: i32::from(state.y),
                    actions: u32::from(state.actions)
                }
            );
        }
    }
}
