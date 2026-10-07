//! View-side input only. Never serialize runtime state into simulation/replay.
//! Feed platform events, then call `sample_tick` exactly when a game tick consumes
//! input. Render frames may inspect `state` without consuming pending edges.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};

pub const MAX_FILE_BYTES: usize = 65536;
pub const MAX_ACTIONS: usize = 64;
pub const MAX_BINDINGS: usize = 8;

/// Stable physical identities, independent of a window or GPU library.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Key {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Space,
    Escape,
    Enter,
    Tab,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "device", deny_unknown_fields)]
pub enum Button {
    Keyboard {
        key: Key,
    },
    Mouse {
        button: u8,
    },
    /// An adapter must supply stable session-local device IDs; digital buttons only.
    Gamepad {
        device_id: u16,
        button: u8,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub name: String,
    pub bindings: Vec<Button>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionMap {
    pub version: u32,
    pub actions: Vec<Action>,
}
impl ActionMap {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err("unsupported input map version (expected 1)".into());
        }
        if self.actions.is_empty() || self.actions.len() > MAX_ACTIONS {
            return Err("action count outside 1..=64".into());
        }
        let mut names = BTreeSet::new();
        let mut buttons = BTreeSet::new();
        for action in &self.actions {
            if action.name.is_empty()
                || action.name.len() > 64
                || !action
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            {
                return Err(
                    "action names must be 1..=64 ASCII letters, digits or underscores".into(),
                );
            }
            if !names.insert(&action.name) {
                return Err(format!("duplicate action {}", action.name));
            }
            if action.bindings.len() > MAX_BINDINGS {
                return Err(format!("too many bindings for {}", action.name));
            }
            for button in &action.bindings {
                if matches!(button, Button::Mouse { button } if *button > 7)
                    || matches!(button, Button::Gamepad { button, .. } if *button > 31)
                {
                    return Err("unsupported button index".into());
                }
                if !buttons.insert(button) {
                    return Err(format!("duplicate physical binding for {}", action.name));
                }
            }
        }
        Ok(())
    }
    /// Bound bytes before deserialization, including readers without a known length.
    pub fn load(reader: impl Read) -> Result<Self, String> {
        let mut bytes = Vec::new();
        reader
            .take((MAX_FILE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err("input map exceeds 64 KiB".into());
        }
        let map: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        map.validate()?;
        Ok(map)
    }
    /// Load only a bounded regular file. Reject special files before opening so
    /// a pre-existing FIFO cannot block awaiting a writer. Direct symlinks are
    /// rejected too; concurrent hostile path replacement is outside this API's
    /// filesystem boundary (as with the local content package loader).
    pub fn load_file(path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let before = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !before.is_file() || before.len() > MAX_FILE_BYTES as u64 {
            return Err("input bindings require a regular file within 64 KiB".into());
        }
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let after = file.metadata().map_err(|e| e.to_string())?;
        if !after.is_file() || after.len() > MAX_FILE_BYTES as u64 {
            return Err("input bindings require a regular file within 64 KiB".into());
        }
        Self::load(file)
    }
    /// Atomically publish a new binding file without overwriting an existing path.
    /// Staging is in the same directory; filesystems without hard-link support
    /// return an error rather than falling back to a truncating write.
    pub fn save_new(&self, path: impl AsRef<std::path::Path>) -> Result<(), String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let mut bytes = Vec::new();
        self.save(&mut bytes)?;
        let path = path.as_ref();
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        let mut staging = None;
        for _ in 0..32 {
            let temp = parent.join(format!(
                ".orr-bindings-{}-{}.tmp",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
            {
                Ok(file) => {
                    staging = Some((temp, file));
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("stage bindings: {error}")),
            }
        }
        let (temp, mut file) = staging.ok_or("unable to reserve a binding staging file")?;
        let result = (|| {
            file.write_all(&bytes)?;
            file.sync_all()?;
            std::fs::hard_link(&temp, path)
        })();
        drop(file);
        let cleanup = std::fs::remove_file(&temp);
        result.map_err(|e| format!("save bindings: {e}"))?;
        cleanup.map_err(|e| format!("bindings saved, but staging cleanup failed: {e}"))
    }
    /// Validate and encode before touching the writer. File replacement policy belongs to the host.
    pub fn save(&self, mut writer: impl Write) -> Result<(), String> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err("input map exceeds 64 KiB".into());
        }
        writer.write_all(&bytes).map_err(|e| e.to_string())
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActionState {
    pub held: bool,
    /// At least one rising/falling transition since the last tick sample. Multiple
    /// taps in a tick coalesce; a complete tap reports both edges with held=false.
    pub pressed: bool,
    pub released: bool,
}
#[derive(Clone, Debug)]
pub struct Input {
    map: ActionMap,
    down: BTreeSet<Button>,
    states: BTreeMap<String, ActionState>,
    focused: bool,
    blocked: bool,
}
impl Input {
    pub fn new(map: ActionMap) -> Result<Self, String> {
        map.validate()?;
        let states = map
            .actions
            .iter()
            .map(|a| (a.name.clone(), ActionState::default()))
            .collect();
        Ok(Self {
            map,
            down: BTreeSet::new(),
            states,
            focused: true,
            blocked: false,
        })
    }
    pub fn map(&self) -> &ActionMap {
        &self.map
    }
    /// Transactional rebind. Invalid replacements preserve map and state; valid
    /// replacements release all controls and require fresh physical presses.
    pub fn replace_map(&mut self, map: ActionMap) -> Result<(), String> {
        let mut next = Self::new(map)?;
        next.focused = self.focused;
        next.blocked = self.blocked;
        *self = next;
        Ok(())
    }
    fn refresh(&mut self) {
        for action in &self.map.actions {
            let held = action.bindings.iter().any(|b| self.down.contains(b));
            let state = self.states.get_mut(&action.name).expect("validated action");
            state.pressed |= held && !state.held;
            state.released |= !held && state.held;
            state.held = held;
        }
    }
    /// Release events always clear state, even when UI consumed them. Consumed
    /// presses never enter gameplay. Repeats never create or resurrect presses.
    pub fn button(&mut self, button: Button, pressed: bool, repeat: bool, consumed: bool) {
        if !pressed || consumed {
            self.down.remove(&button);
        } else if self.focused
            && !self.blocked
            && !repeat
            && self
                .map
                .actions
                .iter()
                .any(|a| a.bindings.contains(&button))
        {
            self.down.insert(button);
        }
        self.refresh();
    }
    fn clear(&mut self) {
        self.down.clear();
        self.refresh();
        // Entering UI/focus loss cancels pending gameplay presses too.
        for state in self.states.values_mut() {
            state.pressed = false;
        }
    }
    pub fn set_focused(&mut self, focused: bool) {
        if self.focused != focused {
            self.clear();
        }
        self.focused = focused;
    }
    pub fn set_blocked(&mut self, blocked: bool) {
        if self.blocked != blocked {
            self.clear();
        }
        self.blocked = blocked;
    }
    pub fn disconnect_gamepad(&mut self, device_id: u16) {
        let removed: BTreeSet<_> = self
            .down
            .iter()
            .copied()
            .filter(|b| matches!(b, Button::Gamepad { device_id: id, .. } if *id == device_id))
            .collect();
        self.down.retain(|b| !removed.contains(b));
        self.refresh();
        for action in &self.map.actions {
            if action.bindings.iter().any(|b| removed.contains(b)) {
                let state = self.states.get_mut(&action.name).expect("validated action");
                if !state.held {
                    state.pressed = false;
                }
            }
        }
    }
    pub fn state(&self, action: &str) -> ActionState {
        self.states.get(action).copied().unwrap_or_default()
    }
    /// Digital opposing directions cancel. Multiple bindings for one direction OR.
    pub fn axis(&self, negative: &str, positive: &str) -> i8 {
        i8::from(self.state(positive).held) - i8::from(self.state(negative).held)
    }
    pub fn sample_tick(&mut self) -> BTreeMap<String, ActionState> {
        let sample = self.states.clone();
        for state in self.states.values_mut() {
            state.pressed = false;
            state.released = false;
        }
        sample
    }
}
