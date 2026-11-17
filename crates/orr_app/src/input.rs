//! What the player is holding right now, for the game's input function.

use winit::keyboard::KeyCode;

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
    fn releasing_all_keys_clears_the_state() {
        let mut ctx = InputContext::default();
        ctx.set(KeyCode::Space, true);
        ctx.set(KeyCode::KeyW, true);
        ctx.release_all();
        assert!(!ctx.any_held(&[KeyCode::Space, KeyCode::KeyW]));
    }
}
