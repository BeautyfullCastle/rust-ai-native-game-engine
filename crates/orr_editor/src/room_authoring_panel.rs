use crate::{
    editor::{Editor, RoomWall},
    model::Mode,
};
use egui::Context;
use orr_fp::FP;
use std::path::PathBuf;

pub struct Panel {
    pub open: bool,
    pub player: [String; 2],
    pub key: [String; 2],
    pub exit: [String; 2],
    pub wall_name: String,
    pub wall_center: [String; 2],
    pub wall_half_extents: [String; 2],
    walls: Vec<RoomWall>,
    walls_current: bool,
    wall_source_stamp: Option<(Option<PathBuf>, u64)>,
    selected_wall_guid: Option<String>,
    selected_wall_name: String,
    selected_wall_center: [String; 2],
    selected_wall_half_extents: [String; 2],
    duplicate_wall_name: String,
    duplicate_wall_center: [String; 2],
    confirm_remove_wall: bool,
    pub status: String,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            player: ["0".into(), "0".into()],
            key: ["0".into(), "0".into()],
            exit: ["0".into(), "0".into()],
            wall_name: "wall".into(),
            wall_center: ["0".into(), "3".into()],
            wall_half_extents: ["1".into(), "1".into()],
            walls: Vec::new(),
            walls_current: false,
            wall_source_stamp: None,
            selected_wall_guid: None,
            selected_wall_name: String::new(),
            selected_wall_center: ["0".into(), "0".into()],
            selected_wall_half_extents: ["1".into(), "1".into()],
            duplicate_wall_name: "wall copy".into(),
            duplicate_wall_center: ["0".into(), "0".into()],
            confirm_remove_wall: false,
            status: String::new(),
        }
    }
}

impl Panel {
    pub fn show(&mut self, context: &Context, editor: &mut Editor, removal_blocker: Option<&str>) {
        let current_stamp = (editor.path(), editor.sim().checksum);
        if self
            .wall_source_stamp
            .as_ref()
            .is_some_and(|stamp| stamp != &current_stamp)
        {
            self.walls_current = false;
            self.confirm_remove_wall = false;
        }
        if !self.open {
            return;
        }
        let editable = editor.game().is_room()
            && editor.spec().is_local()
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && editor.previewing().is_none()
            && !editor.in_gesture();
        let mut open = self.open;
        egui::Window::new("Room level authoring")
            .open(&mut open)
            .default_width(480.0)
            .default_pos(egui::pos2(800.0, 80.0))
            .vscroll(true)
            .show(context, |ui| {
                ui.label("Edit PLAYER, KEY and EXIT X/Z positions together, or add one wall.");
                ui.add_enabled_ui(editable, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Read Room layout").clicked() {
                            match editor.read_room_layout() {
                                Ok([[player_x, player_z], [key_x, key_z], [exit_x, exit_z]]) => {
                                    self.player = [format_fixed(player_x), format_fixed(player_z)];
                                    self.key = [format_fixed(key_x), format_fixed(key_z)];
                                    self.exit = [format_fixed(exit_x), format_fixed(exit_z)];
                                    self.status = "Read PLAYER, KEY and EXIT positions.".into();
                                }
                                Err(error) => self.status = error,
                            }
                        }
                        let positions = [
                            parse_position(&self.player),
                            parse_position(&self.key),
                            parse_position(&self.exit),
                        ];
                        let parsed = positions.into_iter().collect::<Result<Vec<_>, _>>();
                        if ui
                            .add_enabled(
                                parsed.is_ok(),
                                egui::Button::new("Apply Room layout"),
                            )
                            .clicked()
                        {
                            match &parsed {
                                Ok(positions) => {
                                    let positions = [positions[0], positions[1], positions[2]];
                                    self.status = match editor.apply_room_layout(positions) {
                                        Ok(()) => "Applied all three positions as one edit.".into(),
                                        Err(error) => error,
                                    };
                                }
                                Err(error) => self.status = error.clone(),
                            }
                        }
                        if let Err(error) = &parsed {
                            ui.label(error);
                        }
                    });
                    ui.small("Apply submits all three positions as one undo entry.");
                    ui.separator();
                    position_fields(ui, "Player X", "Player Z", &mut self.player);
                    position_fields(ui, "Key X", "Key Z", &mut self.key);
                    position_fields(ui, "Exit X", "Exit Z", &mut self.exit);
                    ui.separator();
                    ui.label("Add Room wall");
                    text_field(ui, "Room wall name", &mut self.wall_name, 64);
                    position_fields(ui, "Wall center X", "Wall center Z", &mut self.wall_center);
                    position_fields(
                        ui,
                        "Wall half-width X",
                        "Wall half-depth Z",
                        &mut self.wall_half_extents,
                    );
                    let center = parse_position(&self.wall_center);
                    let half_extents = parse_position(&self.wall_half_extents);
                    let can_add_wall = !self.wall_name.trim().is_empty()
                        && self.wall_name.trim().len() <= 64
                        && center.is_ok()
                        && half_extents.is_ok();
                    if let Err(error) = &center {
                        ui.label(error);
                    }
                    if let Err(error) = &half_extents {
                        ui.label(error);
                    }
                    if self.wall_name.trim().is_empty() || self.wall_name.trim().len() > 64 {
                        ui.label("Room wall name must contain 1..=64 bytes.");
                    }
                    if ui
                        .add_enabled(can_add_wall, egui::Button::new("Create Room wall"))
                        .clicked()
                    {
                        match (center, half_extents) {
                            (Ok(center), Ok(half_extents)) => match editor
                                .spawn_room_wall(self.wall_name.trim(), center, half_extents)
                            {
                                Ok(name) => {
                                    self.status = match self.refresh_walls(editor) {
                                        Ok(count) => format!("Created wall {name}; read {count} Room wall(s)."),
                                        Err(error) => format!("Created wall {name}, but could not refresh the wall list: {error}"),
                                    };
                                }
                                Err(error) => self.status = error,
                            },
                            (Err(error), _) | (_, Err(error)) => self.status = error,
                        }
                    }
                });
                if !editable {
                    ui.label("Authoring requires a mutable local Room in Edit mode with no preview or active gesture.");
                }
                ui.separator();
                egui::CollapsingHeader::new("Manage existing Room walls")
                    .default_open(false)
                    .show(ui, |ui| {
                        self.wall_list_and_editor(ui, editor, editable, removal_blocker);
                    });
                if !self.status.is_empty() {
                    ui.label(&self.status);
                }
            });
        self.open = open;
    }

    fn wall_list_and_editor(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        editable: bool,
        removal_blocker: Option<&str>,
    ) {
        ui.heading("Room walls");
        if ui.button("Read Room walls").clicked() {
            self.status = match self.refresh_walls(editor) {
                Ok(count) => format!("Read {count} Room wall(s)."),
                Err(error) => error,
            };
        }
        if !self.walls_current {
            ui.small("Read Room walls before editing, duplicating or removing a wall.");
        }
        let mut clicked_wall_guid = None;
        egui::ScrollArea::vertical()
            .max_height(140.0)
            .show(ui, |ui| {
                for wall in &self.walls {
                    let label = format!("{} · {}", wall.name, wall.guid);
                    if ui
                        .selectable_label(
                            self.selected_wall_guid.as_deref() == Some(wall.guid.as_str()),
                            label,
                        )
                        .clicked()
                    {
                        clicked_wall_guid = Some(wall.guid.clone());
                    }
                }
            });
        if let Some(guid) = clicked_wall_guid {
            if let Some(wall) = self.walls.iter().find(|wall| wall.guid == guid).cloned() {
                if let Some(target) = crate::Target::parse(&wall.guid) {
                    editor.select(Some(target));
                }
                self.select_wall(&wall);
            }
        }

        let selected_exists = self
            .selected_wall_guid
            .as_ref()
            .is_some_and(|guid| self.walls.iter().any(|wall| &wall.guid == guid));
        if selected_exists && ui.button("Read selected Room wall").clicked() {
            let guid = self.selected_wall_guid.clone().unwrap_or_default();
            match editor.read_room_walls() {
                Ok(walls) => {
                    self.cache_walls(walls, editor);
                    if let Some(wall) = self.walls.iter().find(|wall| wall.guid == guid).cloned() {
                        self.select_wall(&wall);
                        self.status = format!("Read Room wall {}.", wall.guid);
                    } else {
                        self.walls_current = false;
                        self.status =
                            "The selected Room wall no longer exists; read the wall list again."
                                .into();
                    }
                }
                Err(error) => {
                    self.walls_current = false;
                    self.status = error;
                }
            }
        }

        if let Some(guid) = self.selected_wall_guid.clone().filter(|_| selected_exists) {
            ui.separator();
            ui.label(format!("Selected Room wall: {guid}"));
            text_field(ui, "Selected wall name", &mut self.selected_wall_name, 64);
            position_fields(
                ui,
                "Selected wall center X",
                "Selected wall center Z",
                &mut self.selected_wall_center,
            );
            position_fields(
                ui,
                "Selected wall half-width X",
                "Selected wall half-depth Z",
                &mut self.selected_wall_half_extents,
            );
            let selected_center = parse_position(&self.selected_wall_center);
            let selected_half_extents = parse_position(&self.selected_wall_half_extents);
            let selected_name_valid = valid_wall_name(&self.selected_wall_name);
            let can_update = editable
                && self.walls_current
                && selected_name_valid
                && selected_center.is_ok()
                && selected_half_extents.is_ok();
            if let Err(error) = &selected_center {
                ui.label(error);
            }
            if let Err(error) = &selected_half_extents {
                ui.label(error);
            }
            if !selected_name_valid {
                ui.label("Wall name must contain 1..=64 bytes.");
            }
            if ui
                .add_enabled(can_update, egui::Button::new("Apply Room wall"))
                .clicked()
            {
                self.walls_current = false;
                match (selected_center, selected_half_extents) {
                    (Ok(center), Ok(half_extents)) => match editor.update_room_wall(
                        &guid,
                        self.selected_wall_name.trim(),
                        center,
                        half_extents,
                    ) {
                        Ok(()) => {
                            self.status = match self.refresh_walls(editor) {
                                Ok(_) => format!("Updated Room wall {guid}."),
                                Err(error) => format!("Updated Room wall {guid}, but could not refresh the wall list: {error}"),
                            };
                        }
                        Err(error) => self.status = error,
                    },
                    (Err(error), _) | (_, Err(error)) => self.status = error,
                }
            }

            text_field(ui, "Duplicate wall name", &mut self.duplicate_wall_name, 64);
            position_fields(
                ui,
                "Duplicate wall center X",
                "Duplicate wall center Z",
                &mut self.duplicate_wall_center,
            );
            let duplicate_center = parse_position(&self.duplicate_wall_center);
            let duplicate_name_valid = valid_wall_name(&self.duplicate_wall_name);
            if let Err(error) = &duplicate_center {
                ui.label(error);
            }
            if !duplicate_name_valid {
                ui.label("Duplicate wall name must contain 1..=64 bytes.");
            }
            if ui
                .add_enabled(
                    editable
                        && self.walls_current
                        && duplicate_name_valid
                        && duplicate_center.is_ok(),
                    egui::Button::new("Duplicate Room wall"),
                )
                .clicked()
            {
                self.walls_current = false;
                match duplicate_center {
                    Ok(center) => match editor.duplicate_room_wall(
                        &guid,
                        self.duplicate_wall_name.trim(),
                        center,
                    ) {
                        Ok(new_guid) => {
                            if let Some(target) = crate::Target::parse(&new_guid) {
                                editor.select(Some(target));
                            }
                            self.selected_wall_guid = Some(new_guid.clone());
                            self.status = match self.refresh_walls(editor) {
                                Ok(_) => format!("Duplicated Room wall as {new_guid}."),
                                Err(error) => format!("Duplicated Room wall as {new_guid}, but could not refresh the wall list: {error}"),
                            };
                        }
                        Err(error) => self.status = error,
                    },
                    Err(error) => self.status = error,
                }
            }

            ui.checkbox(
                &mut self.confirm_remove_wall,
                "I confirm removing this Room wall",
            );
            let can_remove = editable
                && self.walls_current
                && self.confirm_remove_wall
                && removal_blocker.is_none();
            if ui
                .add_enabled(can_remove, egui::Button::new("Remove selected Room wall"))
                .clicked()
            {
                self.walls_current = false;
                match editor.remove_room_wall(&guid) {
                    Ok(()) => {
                        self.selected_wall_guid = None;
                        self.confirm_remove_wall = false;
                        self.status = match self.refresh_walls(editor) {
                            Ok(_) => format!("Removed Room wall {guid}."),
                            Err(error) => format!("Removed Room wall {guid}, but could not refresh the wall list: {error}"),
                        };
                    }
                    Err(error) => self.status = error,
                }
            }
            if let Some(reason) = removal_blocker {
                ui.label(format!("Wall removal blocked: {reason}"));
            }
        }
    }

    fn refresh_walls(&mut self, editor: &mut Editor) -> Result<usize, String> {
        match editor.read_room_walls() {
            Ok(walls) => {
                self.cache_walls(walls, editor);
                Ok(self.walls.len())
            }
            Err(error) => {
                self.walls_current = false;
                Err(error)
            }
        }
    }

    fn cache_walls(&mut self, walls: Vec<RoomWall>, editor: &Editor) {
        self.walls = walls;
        self.walls_current = true;
        self.wall_source_stamp = Some((editor.path(), editor.sim().checksum));
        if let Some(guid) = self.selected_wall_guid.clone() {
            if let Some(wall) = self.walls.iter().find(|wall| wall.guid == guid).cloned() {
                self.select_wall(&wall);
            } else {
                self.selected_wall_guid = None;
                self.confirm_remove_wall = false;
            }
        }
    }

    fn select_wall(&mut self, wall: &RoomWall) {
        self.selected_wall_guid = Some(wall.guid.clone());
        self.selected_wall_name = wall.name.clone();
        self.selected_wall_center = [format_fixed(wall.center[0]), format_fixed(wall.center[1])];
        self.selected_wall_half_extents = [
            format_fixed(wall.half_extents[0]),
            format_fixed(wall.half_extents[1]),
        ];
        self.confirm_remove_wall = false;
    }
}

fn valid_wall_name(name: &str) -> bool {
    !name.trim().is_empty() && name.trim().len() <= 64 && !name.trim().chars().any(char::is_control)
}

fn position_fields(ui: &mut egui::Ui, x_label: &str, z_label: &str, position: &mut [String; 2]) {
    ui.horizontal(|ui| {
        let x_response = ui.label(x_label);
        ui.add(
            egui::TextEdit::singleline(&mut position[0])
                .desired_width(90.0)
                .char_limit(48),
        )
        .labelled_by(x_response.id);
        let z_response = ui.label(z_label);
        ui.add(
            egui::TextEdit::singleline(&mut position[1])
                .desired_width(90.0)
                .char_limit(48),
        )
        .labelled_by(z_response.id);
    });
}

fn text_field(ui: &mut egui::Ui, label: &str, value: &mut String, max_chars: usize) {
    ui.horizontal(|ui| {
        let response = ui.label(label);
        ui.add(
            egui::TextEdit::singleline(value)
                .desired_width(ui.available_width())
                .char_limit(max_chars),
        )
        .labelled_by(response.id);
    });
}

fn parse_position(position: &[String; 2]) -> Result<[FP; 2], String> {
    Ok([parse_fixed(&position[0])?, parse_fixed(&position[1])?])
}

fn parse_fixed(text: &str) -> Result<FP, String> {
    let text = text.trim();
    if text.is_empty() || text.len() > 48 {
        return Err("Fixed-point decimal must contain 1..=48 bytes".into());
    }
    orr_reflect::decimal::parse_fp(text)
        .map_err(|error| format!("Invalid fixed-point decimal: {error:?}"))
}

fn format_fixed(value: FP) -> String {
    orr_reflect::decimal::fp_to_decimal(value)
}
