//! What the player is holding right now, for the game's input function.

use orr_bridge::PlayerSlot;
use winit::keyboard::KeyCode;

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

/// Keyboard state of the window. The game reads it once per frame in its
/// input function and turns it into the game's own `Input` value.
#[derive(Clone, Debug, Default)]
pub struct InputContext {
    held: Vec<KeyCode>,
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
    /// A slot without keys (the third local player on) always reads as idle: give
    /// it a gamepad or your own layout. This build reads the keyboard only.
    pub fn stick_for_slot(&self, slot: PlayerSlot) -> Stick {
        KeyLayout::for_slot(slot).map_or_else(Stick::default, |layout| self.stick(&layout))
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
}
