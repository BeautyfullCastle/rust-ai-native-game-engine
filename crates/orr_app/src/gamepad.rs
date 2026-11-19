//! Game controllers, as plain events and state without a device library.
//!
//! The window turns what `gilrs` reports into [`PadEvent`] values (`poll` below, with the
//! `gamepad` feature) and feeds them to the [`crate::InputContext`]. Everything after that
//! (button state, dead zone, sticks, which pad plays which slot) works on these values only, so
//! it runs without a window or a device.

/// The most pads the window reads. A pad is named by its index: the order of connection.
pub const MAX_PADS: usize = 4;

/// An analog value below this (of 1.0) counts as zero: a loose stick that rests off center.
pub const DEADZONE: f32 = 0.2;

/// How far a stick must go before it counts as a step (-1 or 1) of a digital [`crate::Stick`].
pub const PRESS: f32 = 0.5;

/// A pad button, by its place on the pad and not by a vendor symbol (`South` is A on an Xbox
/// pad and Cross on a PlayStation pad).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadButton {
    South,
    East,
    West,
    North,
    Start,
    Select,
    LeftBumper,
    RightBumper,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
}

impl PadButton {
    fn bit(self) -> u16 {
        1 << (self as u16)
    }
}

/// An analog axis of the left stick. `LeftY` is positive when the stick goes up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadAxis {
    LeftX,
    LeftY,
}

/// One thing that happened to a pad. `pad` is the index of the pad (below [`MAX_PADS`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PadEvent {
    Connected { pad: u8 },
    /// The pad went away: its buttons and axes are released.
    Disconnected { pad: u8 },
    Button { pad: u8, button: PadButton, down: bool },
    Axis { pad: u8, axis: PadAxis, value: f32 },
}

/// What one pad holds right now.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PadState {
    connected: bool,
    buttons: u16,
    x: f32,
    y: f32,
}

impl PadState {
    /// Lets go of every button and axis, like a window that lost the focus. The pad stays connected.
    pub(crate) fn release(&mut self) {
        *self = Self { connected: self.connected, ..Self::default() };
    }

    pub(crate) fn hide(&mut self, button: PadButton) {
        self.buttons &= !button.bit();
    }

    pub(crate) fn connected(&self) -> bool {
        self.connected
    }

    pub(crate) fn button(&self, button: PadButton) -> bool {
        self.buttons & button.bit() != 0
    }

    /// The axis with the dead zone applied: 0.0 inside it, the raw value outside.
    pub(crate) fn axis(&self, axis: PadAxis) -> f32 {
        let value = match axis {
            PadAxis::LeftX => self.x,
            PadAxis::LeftY => self.y,
        };
        if value.abs() < DEADZONE {
            0.0
        } else {
            value
        }
    }

    /// One digital step of an axis, or of the two D-pad buttons of its side (they win).
    pub(crate) fn step(&self, axis: PadAxis) -> i32 {
        let (negative, positive) = match axis {
            PadAxis::LeftX => (PadButton::DPadLeft, PadButton::DPadRight),
            PadAxis::LeftY => (PadButton::DPadDown, PadButton::DPadUp),
        };
        let pad = i32::from(self.button(positive)) - i32::from(self.button(negative));
        if pad != 0 {
            return pad;
        }
        let value = self.axis(axis);
        if value >= PRESS {
            1
        } else if value <= -PRESS {
            -1
        } else {
            0
        }
    }

    pub(crate) fn apply(&mut self, event: PadEvent) {
        match event {
            PadEvent::Connected { .. } => self.connected = true,
            PadEvent::Disconnected { .. } => *self = Self::default(),
            // A pad that was never announced is connected as soon as it speaks.
            PadEvent::Button { button, down, .. } => {
                self.connected = true;
                if down {
                    self.buttons |= button.bit();
                } else {
                    self.buttons &= !button.bit();
                }
            }
            PadEvent::Axis { axis, value, .. } => {
                self.connected = true;
                let value = if value.is_finite() { value.clamp(-1.0, 1.0) } else { 0.0 };
                match axis {
                    PadAxis::LeftX => self.x = value,
                    PadAxis::LeftY => self.y = value,
                }
            }
        }
    }
}

impl PadEvent {
    /// The pad this event is about.
    pub fn pad(&self) -> u8 {
        match *self {
            PadEvent::Connected { pad } | PadEvent::Disconnected { pad } | PadEvent::Button { pad, .. } | PadEvent::Axis { pad, .. } => pad,
        }
    }
}

#[cfg(feature = "gamepad")]
pub(crate) use device::Pads;

/// The `gilrs` side: reads the devices and names them by the order of connection.
#[cfg(feature = "gamepad")]
mod device {
    use gilrs::{Axis, Button, EventType, GamepadId, Gilrs};

    use super::{PadAxis, PadButton, PadEvent, MAX_PADS};

    /// The pads of the system. Without a working backend it reports nothing.
    pub(crate) struct Pads {
        gilrs: Option<Gilrs>,
        /// The device of each pad index, in the order they showed up.
        order: Vec<GamepadId>,
    }

    impl Pads {
        pub(crate) fn open() -> Self {
            let gilrs = match Gilrs::new() {
                Ok(gilrs) => Some(gilrs),
                Err(e) => {
                    eprintln!("gamepad: no controller input ({e}); the keyboard still works");
                    None
                }
            };
            let mut pads = Self { gilrs, order: Vec::new() };
            // The pads that are plugged in already.
            let known: Vec<GamepadId> = pads.gilrs.iter().flat_map(|g| g.gamepads().map(|(id, _)| id)).collect();
            for id in known {
                pads.index_of(id);
            }
            pads
        }

        fn index_of(&mut self, id: GamepadId) -> Option<u8> {
            let at = match self.order.iter().position(|known| *known == id) {
                Some(at) => at,
                None if self.order.len() < MAX_PADS => {
                    self.order.push(id);
                    self.order.len() - 1
                }
                None => return None,
            };
            Some(at as u8)
        }

        /// Every event since the last call, as [`PadEvent`] values.
        pub(crate) fn poll(&mut self, out: &mut Vec<PadEvent>) {
            let mut raw = Vec::new();
            if let Some(gilrs) = &mut self.gilrs {
                while let Some(event) = gilrs.next_event() {
                    raw.push((event.id, event.event));
                }
            }
            for (id, event) in raw {
                let Some(pad) = self.index_of(id) else { continue };
                if let Some(event) = translate(pad, &event) {
                    out.push(event);
                }
            }
        }
    }

    fn button(button: Button) -> Option<PadButton> {
        Some(match button {
            Button::South => PadButton::South,
            Button::East => PadButton::East,
            Button::West => PadButton::West,
            Button::North => PadButton::North,
            Button::Start => PadButton::Start,
            Button::Select => PadButton::Select,
            Button::LeftTrigger => PadButton::LeftBumper,
            Button::RightTrigger => PadButton::RightBumper,
            Button::DPadUp => PadButton::DPadUp,
            Button::DPadDown => PadButton::DPadDown,
            Button::DPadLeft => PadButton::DPadLeft,
            Button::DPadRight => PadButton::DPadRight,
            _ => return None,
        })
    }

    fn axis(axis: Axis) -> Option<PadAxis> {
        match axis {
            Axis::LeftStickX => Some(PadAxis::LeftX),
            Axis::LeftStickY => Some(PadAxis::LeftY),
            _ => None,
        }
    }

    fn translate(pad: u8, event: &EventType) -> Option<PadEvent> {
        match *event {
            EventType::Connected => Some(PadEvent::Connected { pad }),
            EventType::Disconnected => Some(PadEvent::Disconnected { pad }),
            EventType::ButtonPressed(b, _) => button(b).map(|button| PadEvent::Button { pad, button, down: true }),
            EventType::ButtonReleased(b, _) => button(b).map(|button| PadEvent::Button { pad, button, down: false }),
            EventType::AxisChanged(a, value, _) => axis(a).map(|axis| PadEvent::Axis { pad, axis, value }),
            _ => None,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn gilrs_buttons_and_axes_map_by_place_on_the_pad() {
            assert_eq!(button(Button::South), Some(PadButton::South));
            assert_eq!(button(Button::LeftTrigger), Some(PadButton::LeftBumper));
            assert_eq!(button(Button::DPadUp), Some(PadButton::DPadUp));
            assert_eq!(button(Button::Mode), None);
            assert_eq!(axis(Axis::LeftStickY), Some(PadAxis::LeftY));
            assert_eq!(axis(Axis::RightStickX), None, "only the left stick is read");
            assert_eq!(translate(2, &EventType::Disconnected), Some(PadEvent::Disconnected { pad: 2 }));
            assert_eq!(translate(1, &EventType::Connected), Some(PadEvent::Connected { pad: 1 }));
        }
    }
}
