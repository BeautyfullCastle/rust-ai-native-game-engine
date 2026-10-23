//! Session-only Collect keyboard translation. The editor input lease owns the
//! bounded restart queue and tick acknowledgments; this module never queues ticks.
use std::collections::BTreeSet;

use egui::{Event, Key};
use orr_sample::collect_app::input_actions::{CollectAction, CollectBindings, KeyCode};

use crate::editor::input::Keys;

#[derive(Clone, Default)]
pub(super) struct CollectControls {
    pub(super) path: String,
    pub(super) error: Option<String>,
    bindings: CollectBindings,
    session: Option<(u64, bool)>,
    down: BTreeSet<KeyCode>,
    blocked: BTreeSet<KeyCode>,
    pressed: BTreeSet<KeyCode>,
    enabled: bool,
}

#[derive(Debug, Default)]
pub(super) struct CollectFrame {
    pub(super) keys: Keys,
    /// Ordered pulses for the existing editor restart edge collector. Each fresh
    /// alias gets its own rising edge, even if another restart alias is held.
    pub(super) restart_transitions: Vec<bool>,
    pub(super) release: bool,
    /// Sorted, unique indices in the original event slice, matched by physical
    /// key identity and emitted once during ordered enumeration.
    pub(super) consumed: Vec<usize>,
}

impl CollectControls {
    pub(super) fn sync_session(&mut self, epoch: u64, is_collect: bool) {
        let session = (epoch, is_collect);
        if self.session != Some(session) {
            self.session = Some(session);
            self.bindings = CollectBindings::default();
            self.path.clear();
            self.error = None;
            self.invalidate();
        }
    }

    fn invalidate(&mut self) {
        self.blocked.extend(self.down.iter().copied());
        self.pressed.clear();
        self.enabled = false;
    }

    pub(super) fn load(&mut self) -> bool {
        let result = CollectBindings::load_file(&self.path).and_then(|bindings| {
            match bindings.action_for_key(KeyCode::Escape) {
                Some(CollectAction::Quit) | None => Ok(bindings),
                _ => Err("Escape is reserved for releasing editor control".into()),
            }
        });
        match result {
            Ok(bindings) => {
                self.bindings = bindings;
                self.error = None;
                self.invalidate();
                true
            }
            Err(error) => {
                self.error = Some(error);
                false
            }
        }
    }

    pub(super) fn use_defaults(&mut self) {
        self.bindings = CollectBindings::default();
        self.path.clear();
        self.error = None;
        self.invalidate();
    }

    pub(super) fn hint(&self) -> String {
        format!(
            "Left {} · Right {} · Up {} · Down {} · Restart {} · Release {} / Esc",
            self.bindings.hint(CollectAction::Left),
            self.bindings.hint(CollectAction::Right),
            self.bindings.hint(CollectAction::Up),
            self.bindings.hint(CollectAction::Down),
            self.bindings.hint(CollectAction::Restart),
            self.bindings.hint(CollectAction::Quit)
        )
    }

    /// Inspect release intent before polling the host. A copied translator keeps
    /// this check non-consuming while sharing the exact ordered-event quarantine
    /// rules with the eventual viewport dispatch.
    pub(super) fn release_requested(&self, events: &[Event], enabled: bool) -> bool {
        self.clone().route(events, enabled).release
    }

    /// Observe every Collect viewport frame, including frames with no lease or
    /// focus. Existing held keys must be released before a new session can use
    /// them. Gameplay never samples egui's logical `keys_down` set.
    pub(super) fn route(&mut self, events: &[Event], enabled: bool) -> CollectFrame {
        if !enabled || !self.enabled {
            self.invalidate();
        }
        self.enabled = enabled;
        let mut frame = CollectFrame::default();
        for (index, event) in events.iter().enumerate() {
            let Event::Key {
                key,
                physical_key,
                pressed,
                repeat,
                ..
            } = event
            else {
                continue;
            };
            let Some(code) = physical_code(physical_key.unwrap_or(*key)) else {
                continue;
            };
            let action = self.bindings.action_for_key(code);
            let release_key = code == KeyCode::Escape
                || (code == KeyCode::Tab && action.is_none())
                || action == Some(CollectAction::Quit);
            if enabled && (action.is_some() || release_key) {
                frame.consumed.push(index);
            }
            if !pressed {
                self.down.remove(&code);
                self.blocked.remove(&code);
                self.pressed.remove(&code);
                continue;
            }
            let fresh = self.down.insert(code);
            if !enabled || *repeat {
                if !enabled || fresh {
                    self.blocked.insert(code);
                }
                continue;
            }
            if !fresh || self.blocked.contains(&code) {
                continue;
            }
            if release_key {
                frame.release = true;
            } else if action == Some(CollectAction::Restart) {
                frame.restart_transitions.extend([false, true]);
            } else if action.is_some() {
                self.pressed.insert(code);
            }
        }
        if frame.release || !enabled {
            self.invalidate();
            frame.restart_transitions.clear();
            return frame;
        }
        let held = |action| i8::from(self.bindings.is_held(action, &self.pressed));
        frame.keys = Keys {
            x: held(CollectAction::Right) - held(CollectAction::Left),
            y: held(CollectAction::Up) - held(CollectAction::Down),
            fire: false,
        };
        frame
    }
}

/// Exactly the version-1 Collect map's keyboard vocabulary. Prefer physical
/// identity; logical fallback is only for platforms without physical events.
/// With `physical_key: None`, the logical key supplies both gameplay identity
/// and release intent; physical identity cannot be reconstructed on that platform.
fn physical_code(key: Key) -> Option<KeyCode> {
    Some(match key {
        Key::A => KeyCode::KeyA,
        Key::B => KeyCode::KeyB,
        Key::C => KeyCode::KeyC,
        Key::D => KeyCode::KeyD,
        Key::E => KeyCode::KeyE,
        Key::F => KeyCode::KeyF,
        Key::G => KeyCode::KeyG,
        Key::H => KeyCode::KeyH,
        Key::I => KeyCode::KeyI,
        Key::J => KeyCode::KeyJ,
        Key::K => KeyCode::KeyK,
        Key::L => KeyCode::KeyL,
        Key::M => KeyCode::KeyM,
        Key::N => KeyCode::KeyN,
        Key::O => KeyCode::KeyO,
        Key::P => KeyCode::KeyP,
        Key::Q => KeyCode::KeyQ,
        Key::R => KeyCode::KeyR,
        Key::S => KeyCode::KeyS,
        Key::T => KeyCode::KeyT,
        Key::U => KeyCode::KeyU,
        Key::V => KeyCode::KeyV,
        Key::W => KeyCode::KeyW,
        Key::X => KeyCode::KeyX,
        Key::Y => KeyCode::KeyY,
        Key::Z => KeyCode::KeyZ,
        Key::ArrowLeft => KeyCode::ArrowLeft,
        Key::ArrowRight => KeyCode::ArrowRight,
        Key::ArrowUp => KeyCode::ArrowUp,
        Key::ArrowDown => KeyCode::ArrowDown,
        Key::Space => KeyCode::Space,
        Key::Escape => KeyCode::Escape,
        Key::Enter => KeyCode::Enter,
        Key::Tab => KeyCode::Tab,
        _ => return None,
    })
}

#[cfg(test)]
#[path = "collect_controls/tests.rs"]
mod tests;
