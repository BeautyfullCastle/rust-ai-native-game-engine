//! Physical keyboard routing for immutable registered-game profiles.
use crate::{editor::input::Phase, static_adapter::RegisteredGame};
use orr_input::{HeldActionInput, HeldActions, Key};

#[derive(Default)]
pub(super) struct Router {
    session: Option<(RegisteredGame, u64, u64)>,
    input: Option<HeldActionInput>,
    active: bool,
}
impl Router {
    pub fn route(
        &mut self,
        game: Option<RegisteredGame>,
        epoch: u64,
        intent: u64,
        enabled: bool,
        phase: Phase,
        events: &[egui::Event],
    ) -> (HeldActions, Vec<usize>, bool) {
        let game = game.filter(|g| g.action_profile().is_some());
        let session = game.map(|g| (g, epoch, intent));
        if self.session != session {
            self.input = game.map(|g| HeldActionInput::new(g.action_profile().unwrap().clone()));
            self.session = session;
            self.active = false;
        }
        let active = enabled && phase == Phase::Active;
        let was_active = self.active;
        self.active = active;
        let Some(input) = &mut self.input else {
            return (HeldActions::default(), vec![], false);
        };
        input.set_focused(enabled);
        input.set_blocked(!active);
        let mut consumed = Vec::new();
        let mut release = false;
        for (index, event) in events.iter().enumerate() {
            if let egui::Event::Key {
                key,
                physical_key,
                pressed,
                repeat,
                modifiers,
            } = event
            {
                // Releases can safely use logical fallback; presses require physical identity.
                let physical = physical_key
                    .or((!*pressed).then_some(*key))
                    .and_then(convert);
                let Some(key) = physical else {
                    continue;
                };
                if enabled
                    && matches!(phase, Phase::Claiming | Phase::Active)
                    && matches!(key, Key::Escape | Key::Tab)
                    && *pressed
                {
                    release = true;
                    consumed.push(index);
                }
                if input.profile().contains_key(key) {
                    let modified = modifiers.ctrl || modifiers.command || modifiers.alt;
                    input.key(key, *pressed, *repeat, !active || !was_active || modified);
                    if active && !modified {
                        consumed.push(index);
                    }
                }
            }
        }
        if release {
            input.set_blocked(true);
            self.active = false;
        }
        (input.held(), consumed, release)
    }
}
fn convert(key: egui::Key) -> Option<Key> {
    Some(match key {
        egui::Key::A => Key::A,
        egui::Key::B => Key::B,
        egui::Key::C => Key::C,
        egui::Key::D => Key::D,
        egui::Key::E => Key::E,
        egui::Key::F => Key::F,
        egui::Key::G => Key::G,
        egui::Key::H => Key::H,
        egui::Key::I => Key::I,
        egui::Key::J => Key::J,
        egui::Key::K => Key::K,
        egui::Key::L => Key::L,
        egui::Key::M => Key::M,
        egui::Key::N => Key::N,
        egui::Key::O => Key::O,
        egui::Key::P => Key::P,
        egui::Key::Q => Key::Q,
        egui::Key::R => Key::R,
        egui::Key::S => Key::S,
        egui::Key::T => Key::T,
        egui::Key::U => Key::U,
        egui::Key::V => Key::V,
        egui::Key::W => Key::W,
        egui::Key::X => Key::X,
        egui::Key::Y => Key::Y,
        egui::Key::Z => Key::Z,
        egui::Key::ArrowLeft => Key::ArrowLeft,
        egui::Key::ArrowRight => Key::ArrowRight,
        egui::Key::ArrowUp => Key::ArrowUp,
        egui::Key::ArrowDown => Key::ArrowDown,
        egui::Key::Space => Key::Space,
        egui::Key::Escape => Key::Escape,
        egui::Key::Enter => Key::Enter,
        egui::Key::Tab => Key::Tab,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(key: egui::Key, pressed: bool, repeat: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat,
            modifiers: egui::Modifiers::NONE,
        }
    }
    #[test]
    fn physical_actions_clear_on_focus_phase_epoch_and_claim_intent() {
        let game = Some(crate::static_adapter::actions::tests::game());
        let mut r = Router::default();
        r.route(game, 0, 1, true, Phase::Active, &[]);
        let held = r
            .route(
                game,
                0,
                1,
                true,
                Phase::Active,
                &[
                    event(egui::Key::W, true, false),
                    event(egui::Key::Space, true, false),
                    event(egui::Key::F, true, false),
                    event(egui::Key::E, true, false),
                ],
            )
            .0;
        assert_eq!(
            held,
            HeldActions {
                x: 0,
                y: 1,
                actions: 7
            }
        );
        assert_eq!(
            r.route(game, 0, 1, false, Phase::Active, &[]).0,
            HeldActions::default()
        );
        r.route(game, 0, 1, true, Phase::Active, &[]);
        assert_eq!(
            r.route(
                game,
                0,
                1,
                true,
                Phase::Active,
                &[event(egui::Key::F, true, true)]
            )
            .0
            .actions,
            0
        );
        assert_eq!(
            r.route(
                game,
                0,
                1,
                true,
                Phase::Active,
                &[event(egui::Key::F, true, false)]
            )
            .0
            .actions,
            2
        );
        assert_eq!(
            r.route(game, 1, 1, true, Phase::Active, &[]).0,
            HeldActions::default()
        );
        r.route(
            game,
            1,
            1,
            true,
            Phase::Active,
            &[event(egui::Key::E, true, false)],
        );
        assert_eq!(
            r.route(game, 1, 1, true, Phase::Off, &[]).0,
            HeldActions::default()
        );
        r.route(
            game,
            1,
            2,
            true,
            Phase::Claiming,
            &[event(egui::Key::E, true, false)],
        );
        assert_eq!(
            r.route(
                game,
                1,
                2,
                true,
                Phase::Active,
                &[event(egui::Key::E, true, true)]
            )
            .0,
            HeldActions::default()
        );
        assert_eq!(
            r.route(
                game,
                1,
                2,
                true,
                Phase::Active,
                &[event(egui::Key::E, true, false)]
            )
            .0
            .actions,
            4
        );
        // Even a reclaim that skips an observed Off frame cannot retain old bits.
        assert_eq!(
            r.route(game, 1, 3, true, Phase::Active, &[]).0,
            HeldActions::default()
        );
    }
    #[test]
    fn routing_never_owns_unfocused_or_modified_text_and_release_clears_all() {
        let game = Some(crate::static_adapter::actions::tests::game());
        let mut r = Router::default();
        assert!(r
            .route(
                game,
                0,
                1,
                false,
                Phase::Active,
                &[event(egui::Key::F, true, false)]
            )
            .1
            .is_empty());
        r.route(game, 0, 1, true, Phase::Active, &[]);
        let mut modified = event(egui::Key::F, true, false);
        if let egui::Event::Key { modifiers, .. } = &mut modified {
            *modifiers = egui::Modifiers::COMMAND;
        }
        let (held, consumed, _) = r.route(game, 0, 1, true, Phase::Active, &[modified]);
        assert_eq!(held, HeldActions::default());
        assert!(consumed.is_empty());
        r.route(
            game,
            0,
            1,
            true,
            Phase::Active,
            &[event(egui::Key::F, true, false)],
        );
        assert_eq!(
            r.route(
                game,
                0,
                1,
                true,
                Phase::Active,
                &[event(egui::Key::F, false, false)]
            )
            .0
            .actions,
            0
        );
        r.route(
            game,
            0,
            1,
            true,
            Phase::Active,
            &[event(egui::Key::E, true, false)],
        );
        let (held, _, release) = r.route(
            game,
            0,
            1,
            true,
            Phase::Active,
            &[event(egui::Key::Tab, true, false)],
        );
        assert!(release);
        assert_eq!(held, HeldActions::default());
    }
}
