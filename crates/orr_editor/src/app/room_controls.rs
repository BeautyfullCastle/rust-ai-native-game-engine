//! Explicit session-only Room controls. No scene, manifest or protocol writes.
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use egui::{Event, Key};
use orr_input::{ActionMap, Button, Key as PhysicalKey};
use orr_sample::room_controls::RoomControls;

use crate::{
    editor::input::{Keys, Phase},
    model::Mode,
    room_input_map_authoring::Panel,
};

pub(super) struct Controls {
    panel: Panel,
    router: Router,
    session: Option<(u64, bool, Option<PathBuf>)>,
    project: Option<(PathBuf, ActionMap)>,
    status: String,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            panel: Panel::default(),
            router: Router::default(),
            session: None,
            project: None,
            status: "Default Room controls for this session".into(),
        }
    }
}

impl Controls {
    pub(super) fn hint(&self) -> String {
        self.router.bindings.movement_hint()
    }

    fn sync(&mut self, epoch: u64, room: bool, path: Option<PathBuf>) {
        let session = (epoch, room, path.clone());
        if self.session.as_ref() != Some(&session) {
            self.session = Some(session);
            if !room
                || self
                    .project
                    .as_ref()
                    .is_some_and(|(scene, _)| Some(scene) != path.as_ref())
            {
                self.project = None;
            }
            let map = self
                .project
                .as_ref()
                .map(|(_, map)| map.clone())
                .unwrap_or_else(RoomControls::default_map);
            self.router.install(&map).expect("admitted Room map");
            self.status = if self.project.is_some() {
                "Admitted project controls restored; authoring document retained"
            } else {
                "Default Room controls; authoring document retained"
            }
            .into();
        }
    }

    fn install_project(&mut self, epoch: u64, scene: &Path, map: ActionMap) -> Result<(), String> {
        self.router.install(&map)?;
        let mut panel = Panel::default();
        panel.apply_document(map.clone())?;
        self.panel = panel;
        self.project = Some((scene.to_path_buf(), map));
        self.session = Some((epoch, true, Some(scene.to_path_buf())));
        self.status =
            "Admitted project controls installed; Use for Play explicitly overrides this session"
                .into();
        Ok(())
    }

    fn apply(&mut self) -> Result<(), String> {
        if self.panel.has_pending_draft() {
            return Err("Apply action or Discard draft before Use for Play".into());
        }
        self.router.install(self.panel.document())?;
        self.status = "Accepted controls applied to this editor session only".into();
        Ok(())
    }
}

impl super::EditorApp {
    /// Install the bounded controls bytes from a fully admitted Room project.
    /// The snapshot is pinned to its scene; restart never rereads the file.
    pub fn install_room_project_controls(
        &mut self,
        scene: &Path,
        bytes: Option<&[u8]>,
    ) -> Result<(), String> {
        if !self.editor.game().is_room()
            || !self.editor.spec().is_local()
            || self.editor.path().as_deref() != Some(scene)
        {
            return Err("Room controls require the matching admitted local scene".into());
        }
        let bootstrap = self.frames == 0 && self.ui.room_controls.session.is_none();
        if (!bootstrap && (self.editor.mode() != Mode::Edit || !self.editor.can_mutate()))
            || self.editor.input_phase() != Phase::Off
            || self.editor.input_cleanup_pending()
            || self.editor.previewing().is_some()
            || self.ui.dialog.is_some()
            || self.ui.room_controls.panel.has_pending_draft()
        {
            return Err("Room controls installation requires idle local Edit mode with released control and no pending draft".into());
        }
        let map = match bytes {
            Some(bytes) => ActionMap::load(bytes)?,
            None => RoomControls::default_map(),
        };
        self.ui
            .room_controls
            .install_project(self.editor.scene_load_epoch(), scene, map)
    }

    pub(super) fn show_room_controls(&mut self, ui: &mut egui::Ui) {
        self.ui.room_controls.sync(
            self.editor.scene_load_epoch(),
            self.editor.game().is_room(),
            self.editor.path(),
        );
        if !self.editor.game().is_room() {
            return;
        }
        ui.label("Room Play controls (admitted project or explicit session override)");
        ui.label(self.ui.room_controls.router.bindings.movement_hint());
        ui.collapsing("Room controls for Play", |ui| {
            ui.label(self.ui.room_controls.router.bindings.movement_hint());
            ui.small("Open or edit a map, then Use for Play. Project files are not changed. Scene reload/restart restores the admitted project snapshot (defaults for a bare scene); authoring edits are retained.");
            let editable = self.editor.spec().is_local() && self.editor.mode() == Mode::Edit
                && self.editor.can_mutate() && self.editor.previewing().is_none()
                && self.editor.input_phase() == Phase::Off && self.ui.dialog.is_none();
            ui.add_enabled_ui(editable, |ui| {
                self.ui.room_controls.panel.show(ui);
                if ui.button("Use for Play").clicked() {
                    if let Err(error) = self.ui.room_controls.apply() {
                        self.ui.room_controls.status = error;
                    }
                }
            });
            if !editable { ui.weak("Idle local Edit mode with released control required"); }
            ui.label(&self.ui.room_controls.status);
        });
    }

    /// Resolve keys before host pumping and UI shortcuts, including a late lease
    /// reply. Keep observing disabled frames so a held key cannot cross a reclaim.
    pub(super) fn route_room_controls(
        &mut self,
        ctx: &egui::Context,
        viewport_focus: bool,
    ) -> bool {
        let room = self.editor.game().is_room();
        self.ui
            .room_controls
            .sync(self.editor.scene_load_epoch(), room, self.editor.path());
        let enabled =
            room && !ctx.input(|i| {
                i.events
                    .iter()
                    .any(|event| matches!(event, Event::WindowFocused(false)))
            }) && viewport_focus
                && ctx.input(|i| i.focused)
                && self.ui.dialog.is_none()
                && self.editor.previewing().is_none()
                && self.editor.can_take_control()
                && matches!(self.editor.input_phase(), Phase::Claiming | Phase::Active);
        let routed = ctx.input(|i| self.ui.room_controls.router.route(&i.events, enabled));
        if room {
            self.editor
                .arena_keys(enabled && !routed.release, routed.keys);
            if routed.release {
                ctx.memory_mut(|m| m.surrender_focus(egui::Id::new("room_keyboard_viewport")));
            }
        }
        if enabled {
            ctx.input_mut(|i| {
                let mut index = 0;
                i.events.retain(|_| {
                    let keep = routed.consumed.binary_search(&index).is_err();
                    index += 1;
                    keep
                });
            });
        }
        enabled
    }
}

struct Router {
    bindings: RoomControls,
    bound: BTreeSet<PhysicalKey>,
    down: BTreeSet<PhysicalKey>,
    blocked: BTreeSet<PhysicalKey>,
    held: BTreeSet<PhysicalKey>,
    enabled: bool,
}

impl Default for Router {
    fn default() -> Self {
        let map = RoomControls::default_map();
        Self {
            bindings: RoomControls::from_map(&map).expect("default Room map"),
            bound: bound_keys(&map),
            down: BTreeSet::new(),
            blocked: BTreeSet::new(),
            held: BTreeSet::new(),
            enabled: false,
        }
    }
}

#[derive(Default)]
struct Routed {
    keys: Keys,
    release: bool,
    missing_identity: bool,
    consumed: Vec<usize>,
}

fn bound_keys(map: &ActionMap) -> BTreeSet<PhysicalKey> {
    map.actions
        .iter()
        .flat_map(|a| &a.bindings)
        .filter_map(|b| match b {
            Button::Keyboard { key } => Some(*key),
            _ => None,
        })
        .collect()
}

impl Router {
    fn invalidate(&mut self) {
        self.blocked.extend(self.down.iter().copied());
        self.held.clear();
        self.enabled = false;
    }

    fn install(&mut self, map: &ActionMap) -> Result<(), String> {
        let bindings = RoomControls::from_map(map)?;
        map.save(Vec::new())?;
        self.bindings = bindings;
        self.bound = bound_keys(map);
        self.invalidate();
        Ok(())
    }

    fn route(&mut self, events: &[Event], enabled: bool) -> Routed {
        if !enabled || !self.enabled {
            self.invalidate();
        }
        self.enabled = enabled;
        let mut result = Routed::default();
        for (index, event) in events.iter().enumerate() {
            let Event::Key {
                physical_key,
                pressed,
                repeat,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            let Some(physical_key) = physical_key else {
                // An asymmetric release cannot safely identify the held key.
                // Never substitute a logical key for an authored physical key.
                result.missing_identity = enabled;
                result.release |= enabled;
                self.invalidate();
                continue;
            };
            let Some(code) = physical_code(*physical_key) else {
                continue;
            };
            let release = code == PhysicalKey::Escape
                || (code == PhysicalKey::Tab && !self.bound.contains(&code));
            if enabled && (release || self.bound.contains(&code)) {
                result.consumed.push(index);
            }
            if !pressed {
                self.down.remove(&code);
                self.blocked.remove(&code);
                self.held.remove(&code);
                continue;
            }
            let fresh = self.down.insert(code);
            if !enabled || *repeat || modifiers.ctrl || modifiers.command || modifiers.alt {
                if fresh || !enabled {
                    self.blocked.insert(code);
                }
                continue;
            }
            if !fresh || self.blocked.contains(&code) {
                continue;
            }
            if release {
                result.release = true;
            } else if self.bound.contains(&code) {
                self.held.insert(code);
            }
        }
        if result.release || !enabled {
            self.invalidate();
            return result;
        }
        let input = self.bindings.input_from_held(self.held.iter().copied());
        result.keys = Keys {
            x: input.move_x,
            y: -input.move_z,
            fire: input.buttons != 0,
        };
        result
    }
}

fn physical_code(key: Key) -> Option<PhysicalKey> {
    Some(match key {
        Key::A => PhysicalKey::A,
        Key::B => PhysicalKey::B,
        Key::C => PhysicalKey::C,
        Key::D => PhysicalKey::D,
        Key::E => PhysicalKey::E,
        Key::F => PhysicalKey::F,
        Key::G => PhysicalKey::G,
        Key::H => PhysicalKey::H,
        Key::I => PhysicalKey::I,
        Key::J => PhysicalKey::J,
        Key::K => PhysicalKey::K,
        Key::L => PhysicalKey::L,
        Key::M => PhysicalKey::M,
        Key::N => PhysicalKey::N,
        Key::O => PhysicalKey::O,
        Key::P => PhysicalKey::P,
        Key::Q => PhysicalKey::Q,
        Key::R => PhysicalKey::R,
        Key::S => PhysicalKey::S,
        Key::T => PhysicalKey::T,
        Key::U => PhysicalKey::U,
        Key::V => PhysicalKey::V,
        Key::W => PhysicalKey::W,
        Key::X => PhysicalKey::X,
        Key::Y => PhysicalKey::Y,
        Key::Z => PhysicalKey::Z,
        Key::ArrowLeft => PhysicalKey::ArrowLeft,
        Key::ArrowRight => PhysicalKey::ArrowRight,
        Key::ArrowUp => PhysicalKey::ArrowUp,
        Key::ArrowDown => PhysicalKey::ArrowDown,
        Key::Space => PhysicalKey::Space,
        Key::Enter => PhysicalKey::Enter,
        Key::Tab => PhysicalKey::Tab,
        Key::Escape => PhysicalKey::Escape,
        _ => return None,
    })
}

#[cfg(test)]
#[path = "room_controls_tests.rs"]
mod tests;
