//! Backend-neutral, bounded Room authored physical-key controls.
use orr_games::room_escape_game::{RoomInput, INTERACT};
use orr_input::{ActionMap, Button, Key};
use std::collections::BTreeMap;

const REQUIRED: [&str; 5] = [
    "move_left",
    "move_right",
    "move_forward",
    "move_backward",
    "interact",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomControls {
    map: BTreeMap<String, Vec<Key>>,
}

impl RoomControls {
    pub fn load(bytes: &[u8]) -> Result<Self, String> {
        let action_map = ActionMap::load(bytes)?;
        Self::from_map(&action_map)
    }

    pub fn from_map(map: &ActionMap) -> Result<Self, String> {
        map.validate()?;
        if map.actions.len() != REQUIRED.len() {
            return Err("Room controls require exactly five actions".into());
        }
        let mut controls = BTreeMap::new();
        for action in &map.actions {
            if !REQUIRED.contains(&action.name.as_str()) {
                return Err(format!("unknown Room control action {}", action.name));
            }
            if action.bindings.is_empty() {
                return Err(format!(
                    "Room control {} requires a keyboard binding",
                    action.name
                ));
            }
            let keys = action
                .bindings
                .iter()
                .map(|button| match button {
                    Button::Keyboard { key } => Ok(*key),
                    _ => Err(format!(
                        "Room control {} accepts keyboard bindings only",
                        action.name
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;
            if keys.iter().any(|key| matches!(key, Key::R | Key::Escape)) {
                return Err("R and Escape are reserved for restart and exit".into());
            }
            controls.insert(action.name.clone(), keys);
        }
        for name in REQUIRED {
            if !controls.contains_key(name) {
                return Err(format!("missing Room control action {name}"));
            }
        }
        Ok(Self { map: controls })
    }

    pub fn input_from_keys(&self, held: impl IntoIterator<Item = Key>) -> RoomInput {
        let held: std::collections::BTreeSet<Key> = held.into_iter().collect();
        let is_held = |name: &str| self.map[name].iter().any(|key| held.contains(key));
        RoomInput {
            move_x: i8::from(is_held("move_right")) - i8::from(is_held("move_left")),
            move_z: i8::from(is_held("move_backward")) - i8::from(is_held("move_forward")),
            buttons: if is_held("interact") { INTERACT } else { 0 },
            reserved: 0,
        }
    }

    /// Translate a platform-neutral held set into the already-admitted RoomInput.
    pub fn input_from_held(&self, held: impl IntoIterator<Item = Key>) -> RoomInput {
        self.input_from_keys(held)
    }

    /// Physical key names from the admitted map, including every authored alias.
    pub fn movement_hint(&self) -> String {
        [
            ("move_left", "left"),
            ("move_right", "right"),
            ("move_forward", "forward"),
            ("move_backward", "backward"),
            ("interact", "interact"),
        ]
        .into_iter()
        .map(|(action, label)| {
            let keys = self.map[action]
                .iter()
                .map(|key| format!("{key:?}"))
                .collect::<Vec<_>>()
                .join("/");
            format!("{keys} {label}")
        })
        .collect::<Vec<_>>()
        .join(", ")
    }

    pub fn default_map() -> ActionMap {
        use orr_input::Action;
        ActionMap {
            version: 1,
            actions: vec![
                Action {
                    name: "move_left".into(),
                    bindings: vec![
                        Button::Keyboard { key: Key::A },
                        Button::Keyboard {
                            key: Key::ArrowLeft,
                        },
                    ],
                },
                Action {
                    name: "move_right".into(),
                    bindings: vec![
                        Button::Keyboard { key: Key::D },
                        Button::Keyboard {
                            key: Key::ArrowRight,
                        },
                    ],
                },
                Action {
                    name: "move_forward".into(),
                    bindings: vec![
                        Button::Keyboard { key: Key::W },
                        Button::Keyboard { key: Key::ArrowUp },
                    ],
                },
                Action {
                    name: "move_backward".into(),
                    bindings: vec![
                        Button::Keyboard { key: Key::S },
                        Button::Keyboard {
                            key: Key::ArrowDown,
                        },
                    ],
                },
                Action {
                    name: "interact".into(),
                    bindings: vec![Button::Keyboard { key: Key::E }],
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(map: &ActionMap) -> Vec<u8> {
        serde_json::to_vec(map).unwrap()
    }

    #[test]
    fn default_aliases_cancel_and_interact_is_held() {
        let controls = RoomControls::from_map(&RoomControls::default_map()).unwrap();
        assert_eq!(
            controls.input_from_keys([Key::A, Key::ArrowLeft, Key::D]),
            RoomInput {
                move_x: 0,
                move_z: 0,
                buttons: 0,
                reserved: 0
            }
        );
        assert_eq!(
            controls
                .input_from_keys([Key::ArrowUp, Key::W, Key::E])
                .move_z,
            -1
        );
        assert_ne!(controls.input_from_keys([Key::E]).buttons & INTERACT, 0);
    }
    #[test]
    fn reserved_keys_are_rejected() {
        let mut map = RoomControls::default_map();
        map.actions[0].bindings = vec![Button::Keyboard { key: Key::R }];
        assert!(RoomControls::from_map(&map)
            .unwrap_err()
            .contains("reserved"));
    }

    #[test]
    fn rejects_invalid_action_contracts() {
        let valid = RoomControls::default_map();
        let mut cases = Vec::new();
        let mut map = valid.clone();
        map.actions.pop();
        cases.push(map);
        let mut map = valid.clone();
        map.actions[0].name = "unknown".into();
        cases.push(map);
        let mut map = valid.clone();
        map.actions[1].name = map.actions[0].name.clone();
        cases.push(map);
        let mut map = valid.clone();
        map.actions.push(orr_input::Action {
            name: "extra".into(),
            bindings: vec![Button::Keyboard { key: Key::Q }],
        });
        cases.push(map);
        let mut map = valid.clone();
        map.actions[0].bindings.clear();
        cases.push(map);
        let mut map = valid.clone();
        map.actions[0].bindings = vec![Button::Keyboard { key: Key::D }];
        cases.push(map);
        let mut map = valid.clone();
        map.version = 2;
        cases.push(map);
        let mut map = valid.clone();
        map.actions[0].bindings = vec![Button::Mouse { button: 1 }];
        cases.push(map);
        let mut map = valid.clone();
        map.actions[0].bindings = vec![Button::Keyboard { key: Key::R }];
        cases.push(map);
        let mut map = valid;
        map.actions[0].bindings = vec![Button::Keyboard { key: Key::Escape }];
        cases.push(map);
        for map in cases {
            assert!(RoomControls::load(&encoded(&map)).is_err());
        }
        assert!(RoomControls::load(&vec![b' '; orr_input::MAX_FILE_BYTES + 1]).is_err());
        assert!(RoomControls::load(b"{").is_err());
    }

    #[test]
    fn rejects_duplicate_physical_keys_across_actions() {
        let mut map = RoomControls::default_map();
        map.actions[0].bindings = vec![Button::Keyboard { key: Key::A }];
        map.actions[1].bindings = vec![Button::Keyboard { key: Key::A }];
        assert!(RoomControls::from_map(&map).is_err());
    }
}
