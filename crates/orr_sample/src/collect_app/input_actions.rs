//! Physical keyboard bindings for the standalone Collect player.
//! The optional file adapter only translates bindings; tick delivery, focus and
//! capture remain owned by the player's existing input lifecycle.
use std::collections::BTreeSet;
use winit::keyboard::KeyCode;

#[cfg(feature = "input-actions")]
use orr_input::{Action, ActionMap, Button, Key};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectAction {
    Left,
    Right,
    Up,
    Down,
    Restart,
    Quit,
}

const ACTIONS: [CollectAction; 6] = [
    CollectAction::Left,
    CollectAction::Right,
    CollectAction::Up,
    CollectAction::Down,
    CollectAction::Restart,
    CollectAction::Quit,
];

impl CollectAction {
    #[cfg(feature = "input-actions")]
    fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
            Self::Restart => "restart",
            Self::Quit => "quit",
        }
    }
}

/// Validated, immutable keyboard bindings. No event or simulation state lives here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectBindings {
    keys: [Vec<KeyCode>; 6],
}

impl Default for CollectBindings {
    fn default() -> Self {
        Self {
            keys: [
                vec![KeyCode::KeyA, KeyCode::ArrowLeft],
                vec![KeyCode::KeyD, KeyCode::ArrowRight],
                vec![KeyCode::KeyW, KeyCode::ArrowUp],
                vec![KeyCode::KeyS, KeyCode::ArrowDown],
                vec![KeyCode::Space],
                vec![KeyCode::Escape],
            ],
        }
    }
}

impl CollectBindings {
    /// Reuse the shared format's version, count, size and conflict rules before
    /// accepting Collect's exact semantic set and keyboard-only adapter support.
    #[cfg(feature = "input-actions")]
    pub fn from_map(map: &ActionMap) -> Result<Self, String> {
        map.validate()?;
        if map.actions.len() != ACTIONS.len()
            || map
                .actions
                .iter()
                .any(|a| !ACTIONS.iter().any(|expected| expected.name() == a.name))
        {
            return Err("Collect requires exactly left/right/up/down/restart/quit actions".into());
        }
        let mut keys: [Vec<KeyCode>; 6] = Default::default();
        for action in &map.actions {
            let index = ACTIONS
                .iter()
                .position(|expected| expected.name() == action.name)
                .expect("validated Collect action");
            for binding in &action.bindings {
                let Button::Keyboard { key } = binding else {
                    return Err("Collect supports keyboard bindings only".into());
                };
                keys[index].push(key_code(*key));
            }
        }
        Ok(Self { keys })
    }

    /// The shared loader rejects non-regular files, direct symlinks and files
    /// over 64 KiB, and bounds the reader again before JSON decoding.
    #[cfg(feature = "input-actions")]
    pub fn load_file(path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        Self::from_map(&ActionMap::load_file(path)?)
    }

    pub fn action_for_key(&self, code: KeyCode) -> Option<CollectAction> {
        ACTIONS
            .into_iter()
            .find(|&action| self.keys[action as usize].contains(&code))
    }

    /// Multiple bindings to an action are ORed; opposing directions remain the
    /// responsibility of the caller's canonical axis conversion.
    pub fn is_held(&self, action: CollectAction, pressed: &BTreeSet<KeyCode>) -> bool {
        self.keys[action as usize]
            .iter()
            .any(|key| pressed.contains(key))
    }

    pub fn hint(&self, action: CollectAction) -> String {
        let keys = &self.keys[action as usize];
        if keys.is_empty() {
            return "Unbound".into();
        }
        keys.iter()
            .map(|&key| key_hint(key))
            .collect::<Vec<_>>()
            .join("/")
    }

    pub fn controls_hint(&self) -> String {
        if *self == Self::default() {
            return crate::collect_view::DEFAULT_CONTROLS_HINT.into();
        }
        format!(
            "Left {} | Right {} | Up {} | Down {} | Restart {} | Quit {}",
            self.hint(CollectAction::Left),
            self.hint(CollectAction::Right),
            self.hint(CollectAction::Up),
            self.hint(CollectAction::Down),
            self.hint(CollectAction::Restart),
            self.hint(CollectAction::Quit),
        )
    }
}

fn key_hint(code: KeyCode) -> String {
    match code {
        KeyCode::ArrowLeft => "Left".into(),
        KeyCode::ArrowRight => "Right".into(),
        KeyCode::ArrowUp => "Up".into(),
        KeyCode::ArrowDown => "Down".into(),
        KeyCode::Escape => "Esc".into(),
        _ => {
            let name = format!("{code:?}");
            name.strip_prefix("Key").unwrap_or(&name).into()
        }
    }
}

/// The existing version-1 `orr_input` format, with Collect's default controls.
#[cfg(feature = "input-actions")]
pub fn default_map() -> ActionMap {
    let keys = [
        vec![Key::A, Key::ArrowLeft],
        vec![Key::D, Key::ArrowRight],
        vec![Key::W, Key::ArrowUp],
        vec![Key::S, Key::ArrowDown],
        vec![Key::Space],
        vec![Key::Escape],
    ];
    ActionMap {
        version: 1,
        actions: ACTIONS
            .into_iter()
            .zip(keys)
            .map(|(action, keys)| Action {
                name: action.name().into(),
                bindings: keys
                    .into_iter()
                    .map(|key| Button::Keyboard { key })
                    .collect(),
            })
            .collect(),
    }
}

#[cfg(feature = "input-actions")]
fn key_code(key: Key) -> KeyCode {
    match key {
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
    }
}

#[cfg(test)]
#[path = "input_actions_tests.rs"]
mod tests;
