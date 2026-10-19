//! Pure, window-local physical-key capture. It never changes authoring documents.
use std::collections::BTreeSet;

/// Result of one input frame. Indices refer to the original input event slice.
#[derive(Debug, Default)]
pub struct CaptureOutput {
    pub captured: Option<(usize, orr_input::Key)>,
    pub cancelled: bool,
    pub message: Option<String>,
    /// Sorted, unique indices; callers can remove them in one linear retain pass.
    pub consumed_event_indices: Vec<usize>,
}

#[derive(Debug)]
struct Session {
    row: usize,
    context: u64,
    armed: bool,
}

/// Tracks held physical keys even when idle so activation cannot bind a held key.
#[derive(Debug, Default)]
pub struct CaptureState {
    down: BTreeSet<egui::Key>,
    session: Option<Session>,
}

impl CaptureState {
    /// Start after processing the activation frame. The context must change on
    /// selection, row/draft, or accepted-document changes, even if later reverted.
    pub fn begin(&mut self, row: usize, context: u64) {
        self.session = Some(Session {
            row,
            context,
            armed: false,
        });
    }

    pub fn is_listening(&self) -> bool {
        self.session.is_some()
    }

    pub fn row(&self) -> Option<usize> {
        self.session.as_ref().map(|session| session.row)
    }

    /// Explicit Cancel preserves the caller's complete draft and accepted map.
    pub fn cancel(&mut self) -> CaptureOutput {
        let cancelled = self.session.take().is_some();
        CaptureOutput {
            cancelled,
            message: cancelled.then(|| "Key capture cancelled. Draft unchanged.".into()),
            ..CaptureOutput::default()
        }
    }

    /// Call exactly once per frame, before drawing widgets (including while idle).
    /// A successful result replaces only the specified existing draft row; Apply,
    /// validation, history and saving remain entirely the caller's responsibility.
    pub fn process(
        &mut self,
        events: &[egui::Event],
        context: u64,
        focused: bool,
    ) -> CaptureOutput {
        let mut output = CaptureOutput::default();
        if self.is_listening() {
            output.consumed_event_indices = events
                .iter()
                .enumerate()
                .filter_map(|(index, event)| is_modal_keyboard_event(event).then_some(index))
                .collect();
        }

        // Preflight the complete frame: a later focus-loss event must invalidate
        // an earlier candidate, even if focus was already regained by frame end.
        if !focused
            || events
                .iter()
                .any(|event| matches!(event, egui::Event::WindowFocused(false)))
        {
            output.cancelled = self.session.take().is_some();
            if output.cancelled {
                output.message = Some(
                    "Key capture cancelled because the window lost focus. Draft unchanged.".into(),
                );
            }
            self.down.clear();
            return output;
        }

        if self
            .session
            .as_ref()
            .is_some_and(|session| session.context != context)
        {
            self.session = None;
            output.cancelled = true;
            output.message = Some("Key capture cancelled because the action draft changed.".into());
        }

        if let Some(session) = self.session.as_mut() {
            // Only arm at a frame boundary. Releasing an activation-held key in
            // this frame cannot make a subsequent down in this frame eligible.
            if !session.armed {
                session.armed = self.down.is_empty();
                output.message = Some(
                    if session.armed {
                        "Press one physical key. Escape cancels. You can also use the key dropdown."
                    } else {
                        "Release held keys before pressing a new key. Escape cancels."
                    }
                    .into(),
                );
            }
        }

        for event in events {
            let egui::Event::Key {
                physical_key,
                pressed,
                repeat,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            let Some(physical) = *physical_key else {
                if *pressed && self.is_listening() {
                    output.message = Some("Physical key identity is unavailable. Use the key dropdown, or press a key on a supported keyboard backend.".into());
                }
                continue;
            };
            if !pressed {
                self.down.remove(&physical);
                continue;
            }
            // Keep tracking even repeats, so a key first observed mid-hold is
            // never accepted by a later duplicate non-repeat event.
            let fresh = self.down.insert(physical);
            if *repeat || !fresh {
                continue;
            }
            let Some(session) = self.session.as_ref() else {
                continue;
            };
            if physical == egui::Key::Escape {
                self.session = None;
                output.cancelled = true;
                output.message = Some("Key capture cancelled. Draft unchanged.".into());
                continue;
            }
            if !session.armed {
                continue;
            }
            if modifiers.any() {
                output.message = Some("Modifier chords are unsupported. Release modifiers and press one key, or use the key dropdown.".into());
                continue;
            }
            let Some(key) = map_physical_key(physical) else {
                output.message = Some("This physical key is unsupported. Press a supported key or use the key dropdown.".into());
                continue;
            };
            output.captured = Some((session.row, key));
            output.message = Some(format!(
                "Captured physical key {key:?} in the action draft. Apply action to accept it."
            ));
            self.session = None;
        }
        output
    }
}

fn is_modal_keyboard_event(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Key { .. }
            | egui::Event::Text(_)
            | egui::Event::Paste(_)
            | egui::Event::Copy
            | egui::Event::Cut
            | egui::Event::Ime(_)
    )
}

fn map_physical_key(key: egui::Key) -> Option<orr_input::Key> {
    use orr_input::Key as Input;
    Some(match key {
        egui::Key::A => Input::A,
        egui::Key::B => Input::B,
        egui::Key::C => Input::C,
        egui::Key::D => Input::D,
        egui::Key::E => Input::E,
        egui::Key::F => Input::F,
        egui::Key::G => Input::G,
        egui::Key::H => Input::H,
        egui::Key::I => Input::I,
        egui::Key::J => Input::J,
        egui::Key::K => Input::K,
        egui::Key::L => Input::L,
        egui::Key::M => Input::M,
        egui::Key::N => Input::N,
        egui::Key::O => Input::O,
        egui::Key::P => Input::P,
        egui::Key::Q => Input::Q,
        egui::Key::R => Input::R,
        egui::Key::S => Input::S,
        egui::Key::T => Input::T,
        egui::Key::U => Input::U,
        egui::Key::V => Input::V,
        egui::Key::W => Input::W,
        egui::Key::X => Input::X,
        egui::Key::Y => Input::Y,
        egui::Key::Z => Input::Z,
        egui::Key::ArrowLeft => Input::ArrowLeft,
        egui::Key::ArrowRight => Input::ArrowRight,
        egui::Key::ArrowUp => Input::ArrowUp,
        egui::Key::ArrowDown => Input::ArrowDown,
        egui::Key::Space => Input::Space,
        egui::Key::Enter => Input::Enter,
        egui::Key::Tab => Input::Tab,
        // Escape is a cancellation command, never a captured binding. It remains
        // available in the manual dropdown alongside the other 33 input keys.
        _ => return None,
    })
}
