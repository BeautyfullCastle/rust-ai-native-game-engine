//! What the player is holding right now, for the game's input function.

use orr_bridge::PlayerSlot;
use winit::keyboard::KeyCode;

use crate::gamepad::{PadAxis, PadButton, PadEvent, PadState, MAX_PADS};

/// The keys of one player on a shared keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyLayout {
    pub up: KeyCode,
    pub down: KeyCode,
    pub left: KeyCode,
    pub right: KeyCode,
    pub action: KeyCode,
}

impl KeyLayout {
    /// W A S D and Space: the left hand.
    pub const WASD: KeyLayout =
        KeyLayout { up: KeyCode::KeyW, down: KeyCode::KeyS, left: KeyCode::KeyA, right: KeyCode::KeyD, action: KeyCode::Space };

    /// The arrow keys and Enter: the right hand.
    pub const ARROWS: KeyLayout =
        KeyLayout { up: KeyCode::ArrowUp, down: KeyCode::ArrowDown, left: KeyCode::ArrowLeft, right: KeyCode::ArrowRight, action: KeyCode::Enter };

    /// The default keys of a local slot: slot 0 gets [`KeyLayout::WASD`], slot 1
    /// [`KeyLayout::ARROWS`], later slots none (one keyboard has room for two).
    pub fn for_slot(slot: PlayerSlot) -> Option<KeyLayout> {
        match slot.0 {
            0 => Some(Self::WASD),
            1 => Some(Self::ARROWS),
            _ => None,
        }
    }
}

/// A [`KeyLayout`] read as a stick and a button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stick {
    /// -1 (left), 0 or 1 (right).
    pub x: i32,
    /// -1 (down), 0 or 1 (up).
    pub y: i32,
    /// The action key is down.
    pub action: bool,
}

/// Where a local player's input comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    /// The keys of a layout on the shared keyboard.
    Keyboard(KeyLayout),
    /// The pad with this index (the order of connection, below `MAX_PADS` = 4).
    Pad(u8),
}

/// Keyboard and controller state of the window. The game reads it once per frame in its
/// input function and turns it into the game's own `Input` value.
#[derive(Clone, Debug, Default)]
pub struct InputContext {
    held: Vec<KeyCode>,
    pads: [PadState; MAX_PADS],
    /// Slots that were given a device by hand (see [`InputContext::assign`]).
    assigned: Vec<(PlayerSlot, Device)>,
}

impl InputContext {
    /// `true` while `key` is down.
    pub fn held(&self, key: KeyCode) -> bool {
        self.held.contains(&key)
    }

    /// `true` while at least one of `keys` is down.
    pub fn any_held(&self, keys: &[KeyCode]) -> bool {
        keys.iter().any(|&k| self.held(k))
    }

    /// -1, 0 or 1: `positive` keys count up, `negative` keys count down, both together cancel.
    pub fn axis(&self, negative: &[KeyCode], positive: &[KeyCode]) -> i32 {
        i32::from(self.any_held(positive)) - i32::from(self.any_held(negative))
    }

    /// The keys of `layout` as a stick and a button.
    pub fn stick(&self, layout: &KeyLayout) -> Stick {
        Stick {
            x: self.axis(&[layout.left], &[layout.right]),
            y: self.axis(&[layout.down], &[layout.up]),
            action: self.held(layout.action),
        }
    }

    /// The default keys of `slot` ([`KeyLayout::for_slot`]) as a stick and a button.
    /// A slot without keys (the third local player on) always reads as idle. This reads the
    /// keyboard only; [`InputContext::stick_of`] follows the device of the slot, pads included.
    pub fn stick_for_slot(&self, slot: PlayerSlot) -> Stick {
        KeyLayout::for_slot(slot).map_or_else(Stick::default, |layout| self.stick(&layout))
    }

    /// `true` while a controller is connected as pad `pad`.
    pub fn pad_connected(&self, pad: u8) -> bool {
        self.pads.get(usize::from(pad)).is_some_and(PadState::connected)
    }

    /// `true` while `button` of pad `pad` is down.
    pub fn pad_button(&self, pad: u8, button: PadButton) -> bool {
        self.pads.get(usize::from(pad)).is_some_and(|p| p.button(button))
    }

    /// `true` while `button` is down on any pad.
    pub fn any_pad_button(&self, button: PadButton) -> bool {
        self.pads.iter().any(|p| p.button(button))
    }

    /// An axis of pad `pad` from -1.0 to 1.0, 0.0 inside the dead zone (up and right are positive).
    pub fn pad_axis(&self, pad: u8, axis: PadAxis) -> f32 {
        self.pads.get(usize::from(pad)).map_or(0.0, |p| p.axis(axis))
    }

    /// Pad `pad` as a stick (the left stick or the D-pad, one step per axis) and a button
    /// (`South`: A on an Xbox pad). A pad that is not there reads as idle.
    pub fn pad_stick(&self, pad: u8) -> Stick {
        self.pads.get(usize::from(pad)).map_or_else(Stick::default, |p| Stick { x: p.step(PadAxis::LeftX), y: p.step(PadAxis::LeftY), action: p.button(PadButton::South) })
    }

    /// Gives `slot` a device by hand, instead of the one [`InputContext::device_of`] picks.
    pub fn assign(&mut self, slot: PlayerSlot, device: Device) {
        self.assigned.retain(|(s, _)| *s != slot);
        self.assigned.push((slot, device));
    }

    /// The device that plays `slot`: the one given by [`InputContext::assign`], else slot 0 the
    /// keyboard (WASD), slot 1 the first pad (the arrow keys while no pad is connected), slot 2
    /// the second pad, and so on. `None`: nothing plays this slot.
    pub fn device_of(&self, slot: PlayerSlot) -> Option<Device> {
        if let Some((_, device)) = self.assigned.iter().find(|(s, _)| *s == slot) {
            return Some(*device);
        }
        match slot.0 {
            0 => Some(Device::Keyboard(KeyLayout::WASD)),
            n => {
                let pad = n - 1;
                if self.pad_connected(pad) {
                    Some(Device::Pad(pad))
                } else {
                    (n == 1).then_some(Device::Keyboard(KeyLayout::ARROWS))
                }
            }
        }
    }

    /// The stick and button of the device of `slot`; idle when it has none.
    pub fn stick_of(&self, slot: PlayerSlot) -> Stick {
        match self.device_of(slot) {
            Some(Device::Keyboard(layout)) => self.stick(&layout),
            Some(Device::Pad(pad)) => self.pad_stick(pad),
            None => Stick::default(),
        }
    }

    /// This state without the keys and pad buttons given. A game screen uses it to hide the
    /// press that just closed a menu from the game input function.
    pub(crate) fn without(&self, keys: &[KeyCode], buttons: &[PadButton]) -> Self {
        let mut copy = self.clone();
        copy.held.retain(|k| !keys.contains(k));
        for pad in &mut copy.pads {
            for button in buttons {
                pad.hide(*button);
            }
        }
        copy
    }

    /// Feeds one controller event (see [`PadEvent`]). The window does this for the pads it
    /// reads; an embedding program with its own device code can too.
    pub fn apply_pad(&mut self, event: PadEvent) {
        if let Some(pad) = self.pads.get_mut(usize::from(event.pad())) {
            pad.apply(event);
        }
    }

    pub(crate) fn set(&mut self, key: KeyCode, down: bool) {
        let at = self.held.iter().position(|&k| k == key);
        match (down, at) {
            (true, None) => self.held.push(key),
            (false, Some(i)) => {
                self.held.swap_remove(i);
            }
            _ => {}
        }
    }

    pub(crate) fn release_all(&mut self) {
        self.held.clear();
        for pad in &mut self.pads {
            pad.release();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_go_down_and_up() {
        let mut ctx = InputContext::default();
        ctx.set(KeyCode::KeyA, true);
        ctx.set(KeyCode::KeyA, true);
        assert!(ctx.held(KeyCode::KeyA));
        ctx.set(KeyCode::KeyA, false);
        assert!(!ctx.held(KeyCode::KeyA));
        ctx.set(KeyCode::KeyA, false);
        assert!(!ctx.any_held(&[KeyCode::KeyA, KeyCode::ArrowLeft]));
    }

    #[test]
    fn an_axis_reads_either_key_of_a_side_and_cancels_when_both_sides_are_down() {
        let mut ctx = InputContext::default();
        let (left, right) = ([KeyCode::KeyA, KeyCode::ArrowLeft], [KeyCode::KeyD, KeyCode::ArrowRight]);
        assert_eq!(ctx.axis(&left, &right), 0);
        ctx.set(KeyCode::ArrowRight, true);
        assert_eq!(ctx.axis(&left, &right), 1);
        ctx.set(KeyCode::KeyA, true);
        assert_eq!(ctx.axis(&left, &right), 0);
        ctx.set(KeyCode::ArrowRight, false);
        assert_eq!(ctx.axis(&left, &right), -1);
    }

    #[test]
    fn two_players_read_separate_keys_of_one_keyboard() {
        let mut ctx = InputContext::default();
        ctx.set(KeyCode::KeyD, true);
        ctx.set(KeyCode::Space, true);
        ctx.set(KeyCode::ArrowDown, true);
        assert_eq!(ctx.stick_for_slot(PlayerSlot(0)), Stick { x: 1, y: 0, action: true });
        assert_eq!(ctx.stick_for_slot(PlayerSlot(1)), Stick { x: 0, y: -1, action: false });
        // A slot with no default keys is idle, whatever is held.
        assert_eq!(ctx.stick_for_slot(PlayerSlot(2)), Stick::default());
        assert_eq!(ctx.stick(&KeyLayout::ARROWS), ctx.stick_for_slot(PlayerSlot(1)));
    }

    #[test]
    fn releasing_all_keys_clears_the_state() {
        let mut ctx = InputContext::default();
        ctx.set(KeyCode::Space, true);
        ctx.set(KeyCode::KeyW, true);
        ctx.release_all();
        assert!(!ctx.any_held(&[KeyCode::Space, KeyCode::KeyW]));
    }

    fn axis(pad: u8, axis: PadAxis, value: f32) -> PadEvent {
        PadEvent::Axis { pad, axis, value }
    }

    fn button(pad: u8, button: PadButton, down: bool) -> PadEvent {
        PadEvent::Button { pad, button, down }
    }

    #[test]
    fn a_pad_reads_as_a_stick_with_a_dead_zone_and_a_button() {
        let mut keys = InputContext::default();
        assert!(!keys.pad_connected(0) && keys.pad_stick(0) == Stick::default(), "no pad: idle");
        keys.apply_pad(PadEvent::Connected { pad: 0 });
        assert!(keys.pad_connected(0));
        keys.apply_pad(axis(0, PadAxis::LeftX, 0.15));
        assert_eq!((keys.pad_axis(0, PadAxis::LeftX), keys.pad_stick(0).x), (0.0, 0), "inside the dead zone");
        keys.apply_pad(axis(0, PadAxis::LeftX, 0.4));
        assert_eq!((keys.pad_axis(0, PadAxis::LeftX), keys.pad_stick(0).x), (0.4, 0), "moved, but not a step yet");
        keys.apply_pad(axis(0, PadAxis::LeftX, 0.9));
        keys.apply_pad(axis(0, PadAxis::LeftY, -1.0));
        keys.apply_pad(button(0, PadButton::South, true));
        assert_eq!(keys.pad_stick(0), Stick { x: 1, y: -1, action: true });
        keys.apply_pad(axis(0, PadAxis::LeftX, f32::NAN));
        assert_eq!(keys.pad_stick(0).x, 0, "a bad value is no value");
    }

    #[test]
    fn the_dpad_wins_over_the_stick_and_opposite_buttons_cancel() {
        let mut keys = InputContext::default();
        keys.apply_pad(axis(0, PadAxis::LeftX, -1.0));
        keys.apply_pad(button(0, PadButton::DPadRight, true));
        assert_eq!(keys.pad_stick(0).x, 1);
        keys.apply_pad(button(0, PadButton::DPadLeft, true));
        assert_eq!(keys.pad_stick(0).x, -1, "both D-pad sides cancel, the stick decides");
        keys.apply_pad(button(0, PadButton::DPadUp, true));
        assert_eq!(keys.pad_stick(0).y, 1);
    }

    #[test]
    fn a_disconnected_pad_lets_go_of_everything_and_a_lost_focus_keeps_it_connected() {
        let mut keys = InputContext::default();
        keys.apply_pad(PadEvent::Connected { pad: 1 });
        keys.apply_pad(button(1, PadButton::Start, true));
        keys.apply_pad(axis(1, PadAxis::LeftY, 1.0));
        assert!(keys.any_pad_button(PadButton::Start) && keys.pad_button(1, PadButton::Start));
        keys.release_all();
        assert!(keys.pad_connected(1) && !keys.any_pad_button(PadButton::Start) && keys.pad_axis(1, PadAxis::LeftY) == 0.0);
        keys.apply_pad(button(1, PadButton::Start, true));
        keys.apply_pad(PadEvent::Disconnected { pad: 1 });
        assert!(!keys.pad_connected(1) && !keys.pad_button(1, PadButton::Start));
        keys.apply_pad(button(9, PadButton::Start, true));
        assert!(!keys.any_pad_button(PadButton::Start), "a pad beyond the limit is ignored");
    }

    #[test]
    fn slot_zero_plays_the_keyboard_and_slot_one_the_first_pad() {
        let mut keys = InputContext::default();
        keys.set(KeyCode::KeyD, true);
        keys.set(KeyCode::ArrowUp, true);
        assert_eq!(keys.device_of(PlayerSlot(0)), Some(Device::Keyboard(KeyLayout::WASD)));
        assert_eq!(keys.device_of(PlayerSlot(1)), Some(Device::Keyboard(KeyLayout::ARROWS)), "no pad yet: the arrow keys");
        assert_eq!(keys.device_of(PlayerSlot(2)), None);
        assert_eq!((keys.stick_of(PlayerSlot(0)).x, keys.stick_of(PlayerSlot(1)).y), (1, 1));

        keys.apply_pad(PadEvent::Connected { pad: 0 });
        keys.apply_pad(axis(0, PadAxis::LeftX, -1.0));
        keys.apply_pad(PadEvent::Connected { pad: 1 });
        keys.apply_pad(button(1, PadButton::South, true));
        assert_eq!(keys.device_of(PlayerSlot(1)), Some(Device::Pad(0)));
        assert_eq!(keys.device_of(PlayerSlot(2)), Some(Device::Pad(1)));
        assert_eq!(keys.stick_of(PlayerSlot(1)), Stick { x: -1, y: 0, action: false }, "the first pad plays slot 1");
        assert_eq!(keys.stick_of(PlayerSlot(2)), Stick { x: 0, y: 0, action: true });
        assert_eq!(keys.stick_of(PlayerSlot(0)).x, 1, "slot 0 stays on the keyboard");
        assert_eq!(keys.stick_for_slot(PlayerSlot(1)).y, 1, "stick_for_slot still reads the keyboard only");
    }

    #[test]
    fn a_slot_can_be_given_another_device() {
        let mut keys = InputContext::default();
        keys.apply_pad(PadEvent::Connected { pad: 0 });
        keys.apply_pad(axis(0, PadAxis::LeftY, 1.0));
        keys.assign(PlayerSlot(0), Device::Pad(0));
        assert_eq!(keys.stick_of(PlayerSlot(0)).y, 1);
        keys.assign(PlayerSlot(0), Device::Keyboard(KeyLayout::ARROWS));
        keys.set(KeyCode::ArrowLeft, true);
        assert_eq!(keys.stick_of(PlayerSlot(0)), Stick { x: -1, y: 0, action: false }, "a later assignment replaces the earlier one");
    }
}
