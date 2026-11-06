//! Opt-in physical gameplay bindings; fixed safety keys remain owned by the HUD.
use crate::defense_audio_settings::{Outcome, StorageProfile, Store};
use egui::{Event, Key, RawInput};
use orr_games::coop_defense_game as game;
use orr_input::{Action, ActionMap, Button, Key as InputKey};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub(crate) const ACTIONS: [&str; 4] = ["summon", "upgrade", "invest", "restart"];
macro_rules! keys { ($($key:ident),*) => { &[$((InputKey::$key, Key::$key)),*] }; }
const KEYS: &[(InputKey, Key)] = keys!(
    A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, ArrowLeft,
    ArrowRight, ArrowUp, ArrowDown, Space, Escape, Enter, Tab
);
fn physical(key: InputKey) -> Key {
    KEYS.iter().find(|(a, _)| *a == key).unwrap().1
}
fn input_key(key: Key) -> Option<InputKey> {
    KEYS.iter().find(|(_, b)| *b == key).map(|(a, _)| *a)
}
pub(crate) fn event_key(key: Key, physical_key: Option<Key>) -> Key {
    physical_key.unwrap_or(key)
}
fn reserved(key: InputKey) -> bool {
    matches!(key, InputKey::P | InputKey::Escape | InputKey::Enter)
}
pub(crate) fn default_map() -> ActionMap {
    ActionMap {
        version: 1,
        actions: ACTIONS
            .into_iter()
            .zip([InputKey::Space, InputKey::U, InputKey::I, InputKey::R])
            .map(|(name, key)| Action {
                name: name.into(),
                bindings: vec![Button::Keyboard { key }],
            })
            .collect(),
    }
}
pub(crate) fn validate(map: &ActionMap) -> Result<(), String> {
    map.validate()?;
    if map.actions.len() != 4
        || map
            .actions
            .iter()
            .map(|a| a.name.as_str())
            .collect::<BTreeSet<_>>()
            != ACTIONS.into_iter().collect()
    {
        return Err("Controls require exactly summon, upgrade, invest and restart".into());
    }
    for action in &map.actions {
        match action.bindings.as_slice() {
            [Button::Keyboard { key }]
                if !reserved(*key) && KEYS.iter().any(|(known, _)| known == key) => {}
            _ => {
                return Err(
                    "Use one keyboard key per action; P, Escape and Enter are reserved".into(),
                )
            }
        }
    }
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Document {
    schema: u32,
    profile: String,
    pub(crate) map: ActionMap,
}
impl Document {
    pub(crate) fn new(map: ActionMap) -> Result<Self, String> {
        validate(&map)?;
        Ok(Self {
            schema: 1,
            profile: "stormglass-controls-v1".into(),
            map,
        })
    }
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 8192 {
            return Err("Controls settings exceed 8 KiB".into());
        }
        let value: Self =
            serde_json::from_slice(bytes).map_err(|_| "Malformed controls settings")?;
        if value.schema != 1 || value.profile != "stormglass-controls-v1" {
            return Err("Unsupported controls settings".into());
        }
        validate(&value.map)?;
        Ok(value)
    }
}
pub(crate) struct Controls {
    pub(crate) active: ActionMap,
    pub(crate) draft: ActionMap,
    pub(crate) open: bool,
    pub(crate) waiting: Option<usize>,
    held: BTreeSet<Key>,
    blocked: BTreeSet<Key>,
    fresh: Vec<Key>,
    pub(crate) store: Option<Store>,
    pub(crate) writable: bool,
    pub(crate) save_requested: bool,
    pub(crate) status: String,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            active: default_map(),
            draft: default_map(),
            open: false,
            waiting: None,
            held: BTreeSet::new(),
            blocked: BTreeSet::new(),
            fresh: Vec::new(),
            store: None,
            writable: false,
            save_requested: false,
            status: "Session controls; saving is not configured".into(),
        }
    }
}
impl Controls {
    pub(crate) fn load(path: &Path, forbidden: &[PathBuf]) -> Result<Self, String> {
        let (store, bytes) =
            Store::open_profile(path, forbidden, StorageProfile::Controls).map_err(context)?;
        let mut controls = Self::default();
        if let Some(bytes) = bytes {
            controls.active = Document::decode(&bytes)?.map;
            controls.draft = controls.active.clone();
            controls.status = "Saved controls restored".into();
        } else if store.writable() {
            controls.status = "Default controls; SAVE to remember changes".into();
        } else {
            controls.status = "Controls unreadable or unsupported; defaults, Save disabled".into();
        }
        controls.writable = store.writable();
        controls.store = Some(store);
        Ok(controls)
    }
    // Track raw physical ownership before UI decisions. Modified presses still count as held.
    pub(crate) fn observe(&mut self, input: &RawInput) {
        self.fresh.clear();
        let lost = !input.focused
            || input
                .events
                .iter()
                .any(|e| matches!(e, Event::WindowFocused(false)));
        if lost {
            self.blocked.extend(self.held.iter().copied());
            self.held.clear();
            if self.waiting.take().is_some() {
                self.status = "Focus lost; select an action again".into();
            }
        }
        for event in &input.events {
            if let Event::Key {
                key,
                physical_key,
                pressed,
                repeat,
                modifiers,
            } = event
            {
                let key = event_key(*key, *physical_key);
                if !pressed {
                    self.held.remove(&key);
                    self.blocked.remove(&key);
                    continue;
                }
                if lost {
                    self.blocked.insert(key);
                    continue;
                }
                let new = self.held.insert(key);
                if new
                    && !repeat
                    && !self.blocked.contains(&key)
                    && !modifiers.ctrl
                    && !modifiers.alt
                    && !modifiers.command
                    && !modifiers.mac_cmd
                {
                    self.fresh.push(key);
                }
            }
        }
    }
    fn neutralize(&mut self) {
        self.blocked.extend(self.held.iter().copied());
        self.fresh.clear();
    }
    pub(crate) fn open_panel(&mut self) {
        self.open = true;
        self.waiting = None;
        self.draft = self.active.clone();
        self.neutralize();
    }
    pub(crate) fn close_panel(&mut self) {
        self.open = false;
        self.waiting = None;
        self.draft = self.active.clone();
        self.neutralize();
    }
    pub(crate) fn arm(&mut self, index: usize) {
        if index >= 4 {
            return;
        }
        self.waiting = Some(index);
        self.neutralize();
        self.status = "Press a fresh key. Esc cancels capture; P and Enter are reserved".into();
    }
    pub(crate) fn capture(&mut self) {
        let Some(index) = self.waiting else {
            return;
        };
        let Some(key) = self.fresh.first().copied() else {
            return;
        };
        if key == Key::Escape {
            self.waiting = None;
            self.neutralize();
            self.status = "Capture canceled".into();
            return;
        }
        let Some(key) = input_key(key) else {
            self.status = "Unsupported key; choose a letter, arrow, Space or Tab".into();
            return;
        };
        let mut candidate = self.draft.clone();
        candidate
            .actions
            .iter_mut()
            .find(|a| a.name == ACTIONS[index])
            .unwrap()
            .bindings = vec![Button::Keyboard { key }];
        match validate(&candidate) {
            Ok(()) => {
                self.draft = candidate;
                self.waiting = None;
                self.neutralize();
                self.status = "Draft changed; APPLY for this session or SAVE to remember".into();
            }
            Err(e) => self.status = e,
        }
    }
    pub(crate) fn apply(&mut self) -> Result<(), String> {
        validate(&self.draft)?;
        self.active = self.draft.clone();
        self.waiting = None;
        self.neutralize();
        self.status = "Controls applied for this session".into();
        Ok(())
    }
    pub(crate) fn reset_draft(&mut self) {
        self.draft = default_map();
        self.waiting = None;
        self.neutralize();
        self.status = "Default draft; APPLY or SAVE to use it".into();
    }
    pub(crate) fn escaped(&self) -> bool {
        self.waiting.is_none() && self.fresh.contains(&Key::Escape)
    }
    pub(crate) fn key(&self, action: u32) -> Key {
        let index = match action {
            game::SUMMON => 0,
            game::UPGRADE => 1,
            game::INVEST => 2,
            _ => 3,
        };
        map_key(&self.active, index)
    }
    pub(crate) fn hint(&self, index: usize, draft: bool) -> String {
        format!(
            "{:?}",
            map_key(if draft { &self.draft } else { &self.active }, index)
        )
        .to_uppercase()
    }
    pub(crate) fn blocked(&self, key: Key) -> bool {
        self.blocked.contains(&key)
    }
    pub(crate) fn blocked_keys(&self) -> BTreeSet<Key> {
        self.blocked.clone()
    }
}
fn map_key(map: &ActionMap, index: usize) -> Key {
    match map
        .actions
        .iter()
        .find(|a| a.name == ACTIONS[index])
        .unwrap()
        .bindings[0]
    {
        Button::Keyboard { key } => physical(key),
        _ => unreachable!("validated keyboard map"),
    }
}
fn context(error: String) -> String {
    error.replace("audio settings", "controls settings")
}
pub(crate) fn process_save(controls: &mut Controls) {
    if !std::mem::take(&mut controls.save_requested) {
        return;
    }
    if !controls.writable || controls.store.is_none() {
        return;
    }
    if let Err(e) = controls.apply() {
        controls.status = e;
        return;
    }
    let bytes = match Document::new(controls.active.clone())
        .and_then(|d| serde_json::to_vec(&d).map_err(|e| e.to_string()))
    {
        Ok(bytes) => bytes,
        Err(e) => {
            controls.status = e;
            return;
        }
    };
    let store = controls.store.as_mut().unwrap();
    controls.status = match store.save_bytes(bytes) {
        Outcome::Saved => "Controls applied and saved".into(),
        Outcome::NotSaved(e) => format!("Session applied; not saved: {}", context(e)),
        Outcome::PublishedUncertain(e) => {
            format!("Controls published; durability uncertain: {}", context(e))
        }
        #[cfg(windows)]
        Outcome::Uncertain(e) => {
            format!("Save outcome uncertain; restart to inspect: {}", context(e))
        }
    }
    .chars()
    .take(240)
    .map(|c| if c.is_control() { ' ' } else { c })
    .collect();
    controls.writable = store.writable();
}

#[cfg(test)]
#[path = "defense_controls_tests.rs"]
mod tests;
