use crate::mesh_workspace::mission::Settings;
use std::path::PathBuf;

pub(crate) struct Panel {
    open: bool,
    scene_path: String,
    width: String,
    depth: String,
    collectible_count: String,
    distance_per_tick: String,
    error: Option<String>,
}

impl Default for Panel {
    fn default() -> Self {
        let settings = Settings::default();
        Self {
            open: false,
            scene_path: String::new(),
            width: settings.width.to_string(),
            depth: settings.depth.to_string(),
            collectible_count: settings.collectible_count.to_string(),
            distance_per_tick: settings.distance_per_tick,
            error: None,
        }
    }
}

impl Panel {
    pub(crate) fn open(&mut self) {
        self.open = true;
    }

    pub(crate) fn busy(&self) -> bool {
        self.open
    }

    pub(crate) fn show(
        &mut self,
        context: &egui::Context,
        blocked: Option<&str>,
    ) -> Option<(PathBuf, Settings)> {
        if !self.open {
            return None;
        }

        let mut request = None;
        let mut canceled = false;
        egui::Modal::new(egui::Id::new("new-mesh-game-modal")).show(context, |ui| {
                ui.set_min_width(500.0);
                ui.heading("New mesh game");
                ui.label("Create a new local mesh scene with a walkable floor and point-navigation collectibles.");
                ui.label("This starter uses deterministic point navigation; it does not add physics or collision.");
                if let Some(reason) = blocked {
                    ui.label(reason);
                }

                let mut changed = false;
                changed |= field(ui, "New mesh game scene path", &mut self.scene_path);
                changed |= field(ui, "Game floor width", &mut self.width);
                changed |= field(ui, "Game floor depth", &mut self.depth);
                changed |= field(ui, "Game collectible count", &mut self.collectible_count);
                changed |= field(ui, "Game distance per tick", &mut self.distance_per_tick);
                if changed {
                    self.error = None;
                }

                let parsed = parse_settings(
                    &self.width,
                    &self.depth,
                    &self.collectible_count,
                    &self.distance_per_tick,
                );
                let path = self.scene_path.trim();
                let path_valid = !path.is_empty()
                    && path.len() <= 4096
                    && PathBuf::from(path).is_absolute();
                if let Err(error) = &parsed {
                    ui.colored_label(egui::Color32::RED, error);
                } else if !path_valid {
                    ui.weak("Choose a bounded absolute scene path; an existing scene is never overwritten.");
                }

                let can_create = blocked.is_none() && path_valid && parsed.is_ok();
                if ui
                    .add_enabled(can_create, egui::Button::new("Create mesh game"))
                    .clicked()
                {
                    if let Ok(settings) = parsed {
                        self.error = None;
                        request = Some((PathBuf::from(path), settings));
                    }
                }
                canceled = ui.button("Cancel new mesh game").clicked();
                if let Some(error) = &self.error {
                    ui.colored_label(egui::Color32::RED, error);
                }
        });

        self.open = !canceled && !context.input(|input| input.key_pressed(egui::Key::Escape));
        if canceled || !self.open {
            self.error = None;
            request = None;
        }
        request
    }

    pub(crate) fn succeeded(&mut self) {
        self.open = false;
        self.error = None;
    }

    pub(crate) fn failed(&mut self, message: String) {
        self.open = true;
        self.error = Some(message);
    }
}

fn field(ui: &mut egui::Ui, label: &str, value: &mut String) -> bool {
    let label_response = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(f32::INFINITY)
            .char_limit(4096),
    )
    .labelled_by(label_response.id)
    .changed()
}

fn parse_settings(
    width: &str,
    depth: &str,
    collectible_count: &str,
    distance_per_tick: &str,
) -> Result<Settings, String> {
    let parse_integer = |value: &str, name: &str| {
        value
            .trim()
            .parse::<u32>()
            .map_err(|_| format!("{name} must be an unsigned whole number"))
    };
    if distance_per_tick.trim().is_empty() {
        return Err("Game distance per tick is required".into());
    }
    Ok(Settings {
        width: parse_integer(width, "Game floor width")?,
        depth: parse_integer(depth, "Game floor depth")?,
        collectible_count: parse_integer(collectible_count, "Game collectible count")?,
        distance_per_tick: distance_per_tick.to_owned(),
    })
}

#[cfg(test)]
#[path = "mesh_game_creator/tests.rs"]
mod tests;
