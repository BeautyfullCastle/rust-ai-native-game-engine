//! Private application fixture: no third production game or native-window claim.
#![cfg(all(feature = "standalone", feature = "static-game-actions"))]
use bytemuck::{Pod, Zeroable};
use orr_bridge::FrameView;
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_input::{Action, Button, HeldActionProfile, HeldActions, Key};
use orr_reflect::{Reflect, TypeRegistry};
use orr_sim::{Game, PlayerSlot, SimContext, System};
use orr_static_starter::{
    game_contract::{admit, HeldKeys, Starter},
    presentation::{FrozenActionInputs, StandaloneGame},
    standalone::Controller,
    NoCommand,
};
use serde_json::{json, Value};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
const TICK: Duration = Duration::from_nanos(16_666_667);
static MAPPINGS: AtomicUsize = AtomicUsize::new(0);
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Pod, Zeroable, Reflect)]
struct Controls {
    x: i32,
    y: i32,
    jump: u32,
    fire: u32,
    interact: u32,
    shield: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable, Reflect)]
struct State {
    x: i32,
    y: i32,
    jump: u32,
    fire: u32,
    interact: u32,
    shield: u32,
}
struct Fixture<const BAD: u8>;
impl<const BAD: u8> Game for Fixture<BAD> {
    type Input = Controls;
    type Command = NoCommand;
    type Event = u32;
    type Config = ();
    fn register(b: &mut ComponentRegistryBuilder) {
        b.register_singleton::<State>("State");
    }
    fn setup(f: &mut Frame, _: &()) {
        f.set_singleton(State::default());
    }
    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(step::<BAD> as fn(&mut SimContext<Self>))]
    }
}
fn step<const BAD: u8>(c: &mut SimContext<Fixture<BAD>>) {
    let i = *c.inputs.input(PlayerSlot(0));
    let mut s = *c.frame.singleton::<State>();
    s.x += i.x;
    s.y += i.y;
    s.jump += i.jump;
    s.fire += i.fire;
    s.interact += i.interact;
    s.shield += i.shield;
    c.frame.set_singleton(s);
}
impl<const BAD: u8> Starter for Fixture<BAD> {
    const ID: &'static str = "PrivateActions";
    const ALIAS: &'static str = "private-actions";
    const INPUT: &'static str = "Controls";
    const BUILD: u64 = 771;
    const SCENE: &'static str = "schema: orr.scene/1\nsingletons:\n  State: {x: 0, y: 0, jump: 0, fire: 0, interact: 0, shield: 0}\nentities: {}\n";
    fn types() -> TypeRegistry {
        let mut t = TypeRegistry::new();
        t.register_singleton::<State>("State");
        t
    }
    fn validate(_: &Frame) -> bool {
        true
    }
    fn valid_input(i: &Controls) -> bool {
        (-1..=1).contains(&i.x)
            && (-1..=1).contains(&i.y)
            && [i.jump, i.fire, i.interact, i.shield]
                .iter()
                .all(|n| *n <= 1)
    }
    fn won(_: &Frame) -> bool {
        false
    }
}
impl<const BAD: u8> StandaloneGame for Fixture<BAD> {
    fn held_input(_: HeldKeys) -> Controls {
        panic!("profile must not use legacy mapper")
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
        if BAD == 0 {
            MAPPINGS.fetch_add(1, Ordering::SeqCst);
        }
        Controls {
            x: if BAD == 3 { 2 } else { i32::from(h.x) },
            y: i32::from(h.y),
            jump: if BAD == 1 {
                1
            } else {
                u32::from(h.actions & 1)
            },
            fire: u32::from((h.actions >> 1) & 1),
            interact: u32::from((h.actions >> 2) & 1),
            shield: u32::from((h.actions >> 3) & 1),
        }
    }
    fn input_value(i: &Controls) -> Value {
        if BAD == 2 {
            return json!({"oversized":"x".repeat(65537)});
        }
        json!({"x":if BAD == 4 { 0 } else { i.x },"y":i.y,"jump":i.jump,"fire":i.fire,"interact":i.interact,"shield":i.shield})
    }
    fn extract(_: FrameView<'_>, out: &mut Vec<orr_view::RenderItem>) {
        out.clear();
    }
    fn state_value(f: FrameView<'_>) -> Value {
        let s = f.singleton::<State>();
        json!([s.x, s.y, s.jump, s.fire, s.interact, s.shield])
    }
}
fn press(c: &mut Controller<Fixture<0>>, key: Key) {
    c.key(key, true, false, false).unwrap();
}
#[test]
fn frozen_domain_lifecycle_and_replay_are_exact() {
    type G = Fixture<0>;
    let profile = G::action_profile().unwrap();
    let frozen = FrozenActionInputs::<G>::admit(profile.clone()).unwrap();
    assert_eq!(profile.state_count(), 144);
    let before: Vec<_> = profile.states().map(|h| frozen.get(h).unwrap()).collect();
    let mapping_calls = MAPPINGS.load(Ordering::SeqCst);
    for _ in 0..100 {
        assert_eq!(frozen.profile().controls_legend(), profile.controls_legend());
    }
    assert_eq!(frozen.profile(), &profile);
    assert_eq!(MAPPINGS.load(Ordering::SeqCst), mapping_calls);
    assert!(profile
        .states()
        .map(|h| frozen.get(h).unwrap())
        .eq(before));
    for h in profile.states() {
        let i = frozen.get(h).unwrap();
        assert_eq!((i.x, i.y), (i32::from(h.x), i32::from(h.y)));
        assert_eq!(
            i.jump | (i.fire << 1) | (i.interact << 2) | (i.shield << 3),
            u32::from(h.actions)
        );
    }
    assert!(frozen
        .get(HeldActions {
            x: 2,
            y: 0,
            actions: 0
        })
        .is_none());
    assert!(frozen
        .get(HeldActions {
            x: 0,
            y: 0,
            actions: 16
        })
        .is_none());
    let admitted = admit::<G>(G::SCENE).unwrap();
    let mut c = Controller::<G>::new(&admitted, None).unwrap();
    let calls = MAPPINGS.load(Ordering::SeqCst);
    c.focus(true).unwrap();
    press(&mut c, Key::Enter);
    press(&mut c, Key::F); // ignored through neutral boundary
    c.update(TICK).unwrap();
    assert!(c.held() == Controls::default());
    for key in [Key::D, Key::W, Key::Space, Key::F, Key::E, Key::Q] {
        press(&mut c, key);
    }
    assert_eq!(
        G::input_value(&c.held()),
        json!({"x":1,"y":1,"jump":1,"fire":1,"interact":1,"shield":1})
    );
    c.update(TICK).unwrap();
    assert_eq!(
        G::state_value(c.snapshot().predicted()),
        json!([1, 1, 1, 1, 1, 1])
    );
    c.key(Key::F, false, true, true).unwrap(); // releases survive repeat/synthetic flags
    assert_eq!(c.held().fire, 0);
    assert_eq!(c.held().jump, 1);
    c.update(TICK).unwrap();
    assert_eq!(
        G::state_value(c.snapshot().predicted()),
        json!([2, 2, 2, 1, 2, 2])
    );
    c.focus(false).unwrap();
    assert!(c.held() == Controls::default());
    c.focus(true).unwrap();
    press(&mut c, Key::Enter);
    c.update(TICK).unwrap();
    c.key(Key::F, true, true, false).unwrap();
    c.key(Key::E, true, false, true).unwrap();
    assert!(c.held() == Controls::default());
    for key in [Key::A, Key::Space, Key::F, Key::E] {
        press(&mut c, key);
    }
    c.update(TICK).unwrap();
    press(&mut c, Key::Escape);
    assert!(c.held() == Controls::default());
    press(&mut c, Key::Enter);
    c.update(TICK).unwrap();
    assert!(c.held() == Controls::default());
    assert_eq!(
        MAPPINGS.load(Ordering::SeqCst),
        calls,
        "no per-frame mapping callback"
    );
    let checksum = c.snapshot().predicted().checksum();
    let ticks = c.snapshot().tick();
    let bytes = c.recording().unwrap();
    admitted.open_replay(&bytes).unwrap();
    let mut replay = Controller::<G>::new(&admitted, Some(&bytes)).unwrap();
    replay.focus(true).unwrap();
    replay.resume().unwrap();
    for _ in 0..ticks {
        replay.update(TICK).unwrap();
    }
    assert_eq!(replay.snapshot().predicted().checksum(), checksum);
}
#[test]
fn nonneutral_and_oversized_profiles_are_rejected_before_play() {
    assert!(
        FrozenActionInputs::<Fixture<1>>::admit(Fixture::<1>::action_profile().unwrap())
            .err()
            .unwrap()
            .contains("neutral")
    );
    assert!(
        FrozenActionInputs::<Fixture<2>>::admit(Fixture::<2>::action_profile().unwrap())
            .err()
            .unwrap()
            .contains("64 KiB")
    );
}

#[test]
fn invalid_typed_domain_and_mismatched_json_are_rejected() {
    assert!(
        FrozenActionInputs::<Fixture<3>>::admit(Fixture::<3>::action_profile().unwrap())
            .err()
            .unwrap()
            .contains("typed input domain")
    );
    assert!(
        FrozenActionInputs::<Fixture<4>>::admit(Fixture::<4>::action_profile().unwrap())
            .err()
            .unwrap()
            .contains("round-trip")
    );
}

#[cfg(feature = "editor")]
#[test]
fn editor_companion_matches_frozen_typed_admission() {
    use orr_static_starter::{game_contract::inputs, integration::action_descriptor};
    type G = Fixture<5>;
    let descriptor = action_descriptor::<G>().unwrap();
    let registry = inputs::<G>();
    let frozen = FrozenActionInputs::<G>::admit(descriptor.profile.clone()).unwrap();
    for held in descriptor.profile.states() {
        assert_eq!(
            (descriptor.held)(held, &registry).unwrap(),
            G::input_value(&frozen.get(held).unwrap())
        );
    }
    fn rejected<const BAD: u8>(held: HeldActions, expected: &str) {
        let descriptor = action_descriptor::<Fixture<BAD>>().unwrap();
        let error = (descriptor.held)(held, &inputs::<Fixture<BAD>>()).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
    rejected::<1>(HeldActions::default(), "neutral");
    rejected::<2>(HeldActions::default(), "64 KiB");
    rejected::<3>(HeldActions::default(), "typed input domain");
    rejected::<4>(
        HeldActions {
            x: 1,
            ..HeldActions::default()
        },
        "round-trip",
    );
    assert!(action_descriptor::<orr_static_starter::delivery::DeliveryGame>().is_none());
    assert!(action_descriptor::<orr_static_starter::switchboard::SwitchGame>().is_none());
}
