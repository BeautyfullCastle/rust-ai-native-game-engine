use crate::{editor::Editor, model::Mode};
use egui::Context;
use orr_fp::FP;

pub struct Panel {
    pub open: bool,
    pub player: [String; 2],
    pub key: [String; 2],
    pub exit: [String; 2],
    pub wall_name: String,
    pub wall_center: [String; 2],
    pub wall_half_extents: [String; 2],
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
            status: String::new(),
        }
    }
}

impl Panel {
    pub fn show(&mut self, context: &Context, editor: &mut Editor) {
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
                        self.status = match (center, half_extents) {
                            (Ok(center), Ok(half_extents)) => editor
                                .spawn_room_wall(self.wall_name.trim(), center, half_extents)
                                .map(|name| format!("Created wall {name}."))
                                .unwrap_or_else(|error| error),
                            (Err(error), _) | (_, Err(error)) => error,
                        };
                    }
                });
                if !editable {
                    ui.label("Authoring requires a mutable local Room in Edit mode with no preview or active gesture.");
                }
                if !self.status.is_empty() {
                    ui.label(&self.status);
                }
            });
        self.open = open;
    }
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
