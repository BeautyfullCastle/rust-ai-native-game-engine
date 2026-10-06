//! Optional action-map adapter. Produces the existing canonical `Keys`/ArenaInput.
//! Arena's Bridge consumes latest held state: it does not acknowledge tick edges.
//! Short taps between updates are therefore not promised as one-shot commands.
use crate::arena_view::Keys;
use orr_input::{Action, ActionMap, Button, Input, Key};
use std::collections::BTreeSet;

const ACTIONS: [&str; 7] = ["left", "right", "up", "down", "fire", "pause", "quit"];
pub fn default_map() -> ActionMap {
    let bindings = [
        ("left", vec![Key::A, Key::ArrowLeft]),
        ("right", vec![Key::D, Key::ArrowRight]),
        ("up", vec![Key::W, Key::ArrowUp]),
        ("down", vec![Key::S, Key::ArrowDown]),
        ("fire", vec![Key::Space]),
        ("pause", vec![Key::P]),
        ("quit", vec![Key::Escape]),
    ];
    ActionMap {
        version: 1,
        actions: bindings
            .into_iter()
            .map(|(name, keys)| Action {
                name: name.into(),
                bindings: keys
                    .into_iter()
                    .map(|key| Button::Keyboard { key })
                    .collect(),
            })
            .collect(),
    }
}
/// The sample has keyboard and mouse adapters, not a native gamepad backend.
pub fn validate_map(map: &ActionMap) -> Result<(), String> {
    map.validate()?;
    let names: BTreeSet<_> = map.actions.iter().map(|a| a.name.as_str()).collect();
    if names != ACTIONS.into_iter().collect() {
        return Err("Arena requires exactly left/right/up/down/fire/pause/quit actions".into());
    }
    if map
        .actions
        .iter()
        .flat_map(|a| &a.bindings)
        .any(|b| matches!(b, Button::Gamepad { .. }))
    {
        return Err("Arena has no native gamepad adapter; use keyboard or mouse bindings".into());
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControlEvent {
    pub quit: bool,
    pub pause_changed: bool,
}
pub struct ArenaControls {
    system: Input,
    gameplay: Input,
    paused: bool,
    ui: bool,
}
impl ArenaControls {
    pub fn new(map: ActionMap) -> Result<Self, String> {
        validate_map(&map)?;
        Ok(Self {
            system: Input::new(map.clone())?,
            gameplay: Input::new(map)?,
            paused: false,
            ui: false,
        })
    }
    pub fn paused(&self) -> bool {
        self.paused
    }
    pub fn set_ui_capture(&mut self, captured: bool) {
        self.ui = captured;
        self.system.set_blocked(captured);
        self.gameplay.set_blocked(captured || self.paused);
    }
    pub fn set_focused(&mut self, focused: bool) {
        self.system.set_focused(focused);
        self.gameplay.set_focused(focused);
    }
    pub fn button(
        &mut self,
        button: Button,
        pressed: bool,
        repeat: bool,
        consumed: bool,
    ) -> ControlEvent {
        let before_pause = self.system.state("pause").held;
        let before_quit = self.system.state("quit").held;
        self.system
            .button(button, pressed, repeat, consumed || self.ui);
        let pause_changed = !before_pause && self.system.state("pause").held;
        let quit = !before_quit && self.system.state("quit").held;
        if pause_changed {
            self.paused = !self.paused;
            self.gameplay.set_blocked(self.paused || self.ui);
        }
        // A resume click/key is still a control event, not a gameplay press.
        self.gameplay.button(
            button,
            pressed,
            repeat,
            consumed || self.ui || pause_changed || quit,
        );
        ControlEvent {
            quit,
            pause_changed,
        }
    }
    /// Publish current held state immediately; no render frame or tick-edge drain.
    pub fn publish<B: orr_bridge::Bridge<orr_testgame::Arena>>(
        &self,
        bridge: &mut B,
    ) -> Result<(), orr_bridge::BridgeError> {
        bridge.set_input(bridge.local_slot(), self.keys().to_input())
    }
    pub fn keys(&self) -> Keys {
        Keys {
            left: self.gameplay.state("left").held,
            right: self.gameplay.state("right").held,
            up: self.gameplay.state("up").held,
            down: self.gameplay.state("down").held,
            fire: self.gameplay.state("fire").held,
        }
    }
}
pub fn keyboard(code: winit::keyboard::KeyCode) -> Option<Button> {
    use winit::keyboard::KeyCode as W;
    let key = match code {
        W::KeyA => Key::A,
        W::KeyB => Key::B,
        W::KeyC => Key::C,
        W::KeyD => Key::D,
        W::KeyE => Key::E,
        W::KeyF => Key::F,
        W::KeyG => Key::G,
        W::KeyH => Key::H,
        W::KeyI => Key::I,
        W::KeyJ => Key::J,
        W::KeyK => Key::K,
        W::KeyL => Key::L,
        W::KeyM => Key::M,
        W::KeyN => Key::N,
        W::KeyO => Key::O,
        W::KeyP => Key::P,
        W::KeyQ => Key::Q,
        W::KeyR => Key::R,
        W::KeyS => Key::S,
        W::KeyT => Key::T,
        W::KeyU => Key::U,
        W::KeyV => Key::V,
        W::KeyW => Key::W,
        W::KeyX => Key::X,
        W::KeyY => Key::Y,
        W::KeyZ => Key::Z,
        W::ArrowLeft => Key::ArrowLeft,
        W::ArrowRight => Key::ArrowRight,
        W::ArrowUp => Key::ArrowUp,
        W::ArrowDown => Key::ArrowDown,
        W::Space => Key::Space,
        W::Escape => Key::Escape,
        W::Enter => Key::Enter,
        W::Tab => Key::Tab,
        _ => return None,
    };
    Some(Button::Keyboard { key })
}
pub fn mouse(button: winit::event::MouseButton) -> Option<Button> {
    use winit::event::MouseButton as M;
    let button = match button {
        M::Left => 0,
        M::Right => 1,
        M::Middle => 2,
        M::Back => 3,
        M::Forward => 4,
        M::Other(n @ 5..=7) => n as u8,
        _ => return None,
    };
    Some(Button::Mouse { button })
}
