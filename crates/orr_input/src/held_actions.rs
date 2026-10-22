//! Bounded, opt-in keyboard controls for statically registered games.
//!
//! Names and bit positions are immutable for the lifetime of a profile. This is
//! view-side admission metadata, not a simulation input or replay format.
use crate::{Action, ActionMap, Button, Input, Key};

pub const MAX_HELD_ACTIONS: usize = 4;
pub const MAX_HELD_STATES: usize = 9 * (1 << MAX_HELD_ACTIONS);

/// A normalized held sample. Bits correspond to `HeldActionProfile::actions`
/// in declaration order. Axes must be in -1..=1; undeclared bits are invalid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeldActions {
    pub x: i8,
    pub y: i8,
    pub actions: u8,
}

/// Validated, immutable finite input domain. Zero actions allows movement only.
/// Declaration order is stable bit identity. Equality compares the exact ordered
/// action names and ordered physical bindings, not merely equivalent controls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldActionProfile {
    actions: Vec<Action>,
}
impl HeldActionProfile {
    pub fn new(actions: Vec<Action>) -> Result<Self, String> {
        if actions.len() > MAX_HELD_ACTIONS {
            return Err("held profile supports at most four actions".into());
        }
        // Reuse the ordinary name, count and physical-binding validation. Empty
        // profiles are deliberately permitted here, unlike ordinary action maps.
        if !actions.is_empty() {
            ActionMap {
                version: 1,
                actions: actions.clone(),
            }
            .validate()?;
        }
        for action in &actions {
            if action.bindings.is_empty() {
                return Err("held actions require explicit keyboard bindings".into());
            }
            for binding in &action.bindings {
                match binding {
                    Button::Keyboard { key } if !reserved_key(*key) => {}
                    _ => return Err("held action binding is not an available keyboard key".into()),
                }
            }
        }
        Ok(Self { actions })
    }
    /// Inspect admitted metadata without permitting mutation.
    ///
    /// ```compile_fail
    /// use orr_input::HeldActionProfile;
    /// let profile = HeldActionProfile::new(vec![]).unwrap();
    /// profile.actions().push(todo!());
    /// ```
    pub fn actions(&self) -> &[Action] {
        &self.actions
    }
    pub fn state_count(&self) -> usize {
        9 * (1 << self.actions.len())
    }
    /// Keys owned by this gameplay profile, excluding reserved host controls.
    pub fn contains_key(&self, key: Key) -> bool {
        matches!(
            key,
            Key::W
                | Key::A
                | Key::S
                | Key::D
                | Key::ArrowLeft
                | Key::ArrowRight
                | Key::ArrowUp
                | Key::ArrowDown
        ) || self
            .actions
            .iter()
            .any(|action| action.bindings.contains(&keyboard(key)))
    }
    /// Stable index for admission-time frozen values; rejects rather than clamps.
    pub fn index(&self, held: HeldActions) -> Option<usize> {
        let masks = 1usize << self.actions.len();
        if !(-1..=1).contains(&held.x)
            || !(-1..=1).contains(&held.y)
            || usize::from(held.actions) >= masks
        {
            return None;
        }
        Some(
            ((held.x + 1) as usize * 3 + (held.y + 1) as usize) * masks + usize::from(held.actions),
        )
    }
    /// Enumerates every admitted state exactly once, in `index` order.
    pub fn states(&self) -> impl Iterator<Item = HeldActions> + '_ {
        let masks = 1u8 << self.actions.len();
        (-1..=1).flat_map(move |x| {
            (-1..=1).flat_map(move |y| (0..masks).map(move |actions| HeldActions { x, y, actions }))
        })
    }
}
fn reserved_key(key: Key) -> bool {
    matches!(
        key,
        Key::Escape
            | Key::Enter
            | Key::Tab
            | Key::W
            | Key::A
            | Key::S
            | Key::D
            | Key::ArrowLeft
            | Key::ArrowRight
            | Key::ArrowUp
            | Key::ArrowDown
    )
}
const ACTION_NAMES: [&str; MAX_HELD_ACTIONS] = ["action_0", "action_1", "action_2", "action_3"];
fn keyboard(key: Key) -> Button {
    Button::Keyboard { key }
}

/// Existing input backend with a finite held projection. Focus/UI transitions
/// discard held controls; repeats cannot resurrect them without a fresh press.
#[derive(Clone, Debug)]
pub struct HeldActionInput {
    profile: HeldActionProfile,
    input: Input,
}
impl HeldActionInput {
    pub fn new(profile: HeldActionProfile) -> Self {
        let mut actions = vec![
            Action {
                name: "left".into(),
                bindings: vec![keyboard(Key::A), keyboard(Key::ArrowLeft)],
            },
            Action {
                name: "right".into(),
                bindings: vec![keyboard(Key::D), keyboard(Key::ArrowRight)],
            },
            Action {
                name: "up".into(),
                bindings: vec![keyboard(Key::W), keyboard(Key::ArrowUp)],
            },
            Action {
                name: "down".into(),
                bindings: vec![keyboard(Key::S), keyboard(Key::ArrowDown)],
            },
        ];
        // Internal names cannot collide with user names: the latter are never
        // inserted into this runtime map. The profile retains their stable names.
        for (index, action) in profile.actions.iter().enumerate() {
            actions.push(Action {
                name: ACTION_NAMES[index].into(),
                bindings: action.bindings.clone(),
            });
        }
        let input = Input::new(ActionMap {
            version: 1,
            actions,
        })
        .expect("validated profile and disjoint reserved movement bindings");
        Self { profile, input }
    }
    pub fn profile(&self) -> &HeldActionProfile {
        &self.profile
    }
    pub fn key(&mut self, key: Key, pressed: bool, repeat: bool, consumed: bool) {
        self.input.button(keyboard(key), pressed, repeat, consumed);
    }
    pub fn set_focused(&mut self, focused: bool) {
        self.input.set_focused(focused);
    }
    pub fn set_blocked(&mut self, blocked: bool) {
        self.input.set_blocked(blocked);
    }
    pub fn held(&self) -> HeldActions {
        let mut actions = 0;
        for (index, name) in ACTION_NAMES
            .iter()
            .take(self.profile.actions.len())
            .enumerate()
        {
            if self.input.state(name).held {
                actions |= 1 << index;
            }
        }
        HeldActions {
            x: self.input.axis("left", "right"),
            y: self.input.axis("down", "up"),
            actions,
        }
    }
}
