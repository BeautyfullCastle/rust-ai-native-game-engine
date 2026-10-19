use crate::{
    editor::{Editor, Mode, RoomWall},
    room_wall_stamp::Template,
};
use egui::Context;
use orr_fp::FP;
use std::{collections::BTreeSet, path::PathBuf};

type SourceStamp = (Option<PathBuf>, u64, u64);

pub(crate) struct Panel {
    pub open: bool,
    pub stamp_file: String,
    pub offset: [String; 2],
    pub pivot: [String; 2],
    pub quarter_turns: u8,
    pub status: String,
    walls: Vec<RoomWall>,
    selected: BTreeSet<String>,
    walls_current: bool,
    wall_stamp: Option<SourceStamp>,
    template: Option<Template>,
    template_file: Option<PathBuf>,
    expected_digest: Option<String>,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            stamp_file: String::new(),
            offset: ["0".into(), "0".into()],
            pivot: ["0".into(), "0".into()],
            quarter_turns: 0,
            status: String::new(),
            walls: Vec::new(),
            selected: BTreeSet::new(),
            walls_current: false,
            wall_stamp: None,
            template: None,
            template_file: None,
            expected_digest: None,
        }
    }
}

impl Panel {
    pub(crate) fn show(&mut self, context: &Context, editor: &mut Editor) {
        let current_stamp = scene_stamp(editor);
        if self
            .wall_stamp
            .as_ref()
            .is_some_and(|stamp| stamp != &current_stamp)
        {
            self.walls.clear();
            self.selected.clear();
            self.walls_current = false;
            self.wall_stamp = None;
            self.status = "Room changed; read the wall list again before capturing.".into();
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
        egui::Window::new("Room wall groups")
            .open(&mut open)
            .default_width(520.0)
            .default_pos(egui::pos2(680.0, 110.0))
            .vscroll(true)
            .show(context, |ui| {
                ui.label("Save reusable groups of ordinary Room walls; instances receive new GUIDs.");
                ui.add_enabled_ui(editable, |ui| {
                    if ui.button("Read Room walls").clicked() {
                        self.status = match self.refresh_walls(editor) {
                            Ok(count) => format!("Read {count} Room wall(s)."),
                            Err(error) => error,
                        };
                    }
                    if !self.walls_current {
                        ui.small("Read the current wall list before selecting or capturing walls.");
                    }
                    egui::ScrollArea::vertical()
                        .max_height(220.0)
                        .show(ui, |ui| {
                            for wall in &self.walls {
                                let mut checked = self.selected.contains(&wall.guid);
                                let label = format!("{} ({})", wall.name, wall.guid);
                                if ui.checkbox(&mut checked, label).changed() {
                                    if checked {
                                        self.selected.insert(wall.guid.clone());
                                    } else {
                                        self.selected.remove(&wall.guid);
                                    }
                                }
                            }
                        });
                    let can_capture = self.walls_current
                        && self.wall_stamp.as_ref() == Some(&current_stamp)
                        && !self.selected.is_empty()
                        && self.selected.len() <= 64;
                    if ui
                        .add_enabled(can_capture, egui::Button::new("Capture selected walls"))
                        .clicked()
                    {
                        let guids = self.selected.iter().cloned().collect::<Vec<_>>();
                        match editor.capture_room_wall_stamp(&guids) {
                            Ok(template) => {
                                self.template = Some(template);
                                self.status = format!(
                                    "Captured Room wall stamp: {} wall(s); save explicitly.",
                                    guids.len()
                                );
                            }
                            Err(error) => self.status = error,
                        }
                    }
                    ui.separator();
                    text_field(ui, "Room wall stamp file", &mut self.stamp_file, 4096);
                    let stamp_path = PathBuf::from(self.stamp_file.trim());
                    if ui
                        .add_enabled(
                            !self.stamp_file.trim().is_empty(),
                            egui::Button::new("Load Room wall stamp"),
                        )
                        .clicked()
                    {
                        self.template = None;
                        self.template_file = None;
                        self.expected_digest = None;
                        match crate::room_wall_stamp_store::load(&stamp_path) {
                            Ok(Some((template, digest))) => {
                                self.template = Some(template);
                                self.template_file = Some(stamp_path.clone());
                                self.expected_digest = Some(digest);
                                self.status = format!(
                                    "Loaded Room wall stamp: {} wall(s).",
                                    self.template.as_ref().map_or(0, |value| value.walls().len())
                                );
                            }
                            Ok(None) => self.status = "Room wall stamp does not exist.".into(),
                            Err(error) => self.status = error,
                        }
                    }
                    let can_save = self.template.is_some() && !self.stamp_file.trim().is_empty();
                    if ui
                        .add_enabled(can_save, egui::Button::new("Save Room wall stamp"))
                        .clicked()
                    {
                        let template = self.template.as_ref().expect("enabled with template");
                        match crate::room_wall_stamp_store::save(
                            &stamp_path,
                            template,
                            self.template_file
                                .as_ref()
                                .filter(|observed_path| *observed_path == &stamp_path)
                                .and(self.expected_digest.as_deref()),
                        ) {
                            Ok(digest) => {
                                self.template_file = Some(stamp_path.clone());
                                self.expected_digest = Some(digest);
                                self.status = format!("Saved Room wall stamp: {}", stamp_path.display());
                            }
                            Err(error) => self.status = error,
                        }
                    }
                    ui.separator();
                    ui.label("Stamp rotation (X/Z plane)");
                    ui.horizontal_wrapped(|ui| {
                        for (turns, label) in [(0, "0°"), (1, "90°"), (2, "180°"), (3, "270°")] {
                            ui.selectable_value(&mut self.quarter_turns, turns, label);
                        }
                    });
                    ui.small("+90° maps +X to +Z. Rotate around the pivot, then add the offset.");
                    position_fields(ui, "Stamp pivot", &mut self.pivot);
                    position_fields(ui, "Stamp offset", &mut self.offset);
                    let offset = parse_position(&self.offset);
                    if let Err(error) = &offset {
                        ui.label(error);
                    }
                    let pivot = parse_position(&self.pivot);
                    if let Err(error) = &pivot {
                        ui.label(format!("Stamp pivot: {error}"));
                    }
                    let can_instantiate =
                        editable && self.template.is_some() && offset.is_ok() && pivot.is_ok();
                    if ui
                        .add_enabled(
                            can_instantiate,
                            egui::Button::new("Instantiate Room wall stamp"),
                        )
                        .clicked()
                    {
                        match (self.template.as_ref(), offset, pivot) {
                            (Some(template), Ok(offset), Ok(pivot)) => {
                                // Placement operates on a copy so saving still writes the source stamp.
                                let result = template
                                    .placed(self.quarter_turns, pivot, offset)
                                    .and_then(|placed| {
                                        editor.instantiate_room_wall_stamp(&placed, [FP::ZERO; 2])
                                    });
                                match result {
                                    Ok(guids) => {
                                        self.selected = guids.iter().cloned().collect();
                                        self.walls_current = false;
                                        self.wall_stamp = None;
                                        self.status = match self.refresh_walls(editor) {
                                            Ok(_) => format!(
                                                "Instantiated Room wall stamp: {} ordinary actors in one edit.",
                                                guids.len()
                                            ),
                                            Err(error) => format!(
                                                "Instantiated Room wall stamp, but could not refresh the list: {error}"
                                            ),
                                        };
                                    }
                                    Err(error) => self.status = error,
                                }
                            }
                            (_, Err(error), _) | (_, _, Err(error)) => self.status = error,
                            (None, _, _) => self.status = "Load or capture a Room wall stamp first.".into(),
                        }
                    }
                });
                if !editable {
                    ui.label("Wall groups require an idle local Room in Edit mode.");
                }
                if !self.status.is_empty() {
                    ui.separator();
                    ui.label(&self.status);
                }
            });
        self.open = open;
    }

    fn refresh_walls(&mut self, editor: &mut Editor) -> Result<usize, String> {
        let walls = editor.read_room_walls()?;
        let valid = walls
            .iter()
            .map(|wall| wall.guid.as_str())
            .collect::<BTreeSet<_>>();
        self.selected.retain(|guid| valid.contains(guid.as_str()));
        self.walls = walls;
        self.wall_stamp = Some(scene_stamp(editor));
        self.walls_current = true;
        Ok(self.walls.len())
    }
}

fn scene_stamp(editor: &Editor) -> SourceStamp {
    (
        editor.path(),
        editor.scene_load_epoch(),
        editor.sim().checksum,
    )
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

fn position_fields(ui: &mut egui::Ui, prefix: &str, position: &mut [String; 2]) {
    for (axis, value) in ["X", "Z"].into_iter().zip(position) {
        // Separate rows keep the fields accessible in a narrow window.
        ui.horizontal(|ui| {
            let label = ui.label(format!("{prefix} {axis}"));
            ui.add(egui::TextEdit::singleline(value).desired_width(ui.available_width()))
                .labelled_by(label.id);
        });
    }
}

fn parse_position(values: &[String; 2]) -> Result<[FP; 2], String> {
    Ok([parse_fixed(&values[0])?, parse_fixed(&values[1])?])
}

fn parse_fixed(text: &str) -> Result<FP, String> {
    let text = text.trim();
    if text.is_empty() || text.len() > 48 {
        return Err("Fixed-point decimal must contain 1..=48 bytes".into());
    }
    orr_reflect::decimal::parse_fp(text)
        .map_err(|error| format!("Invalid fixed-point decimal: {error:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_defaults_preserve_unrotated_origin_and_offset() {
        let panel = Panel::default();
        assert_eq!(panel.quarter_turns, 0);
        assert_eq!(parse_position(&panel.pivot).unwrap(), [FP::ZERO; 2]);
        assert_eq!(parse_position(&panel.offset).unwrap(), [FP::ZERO; 2]);
    }

    #[test]
    fn placement_fields_reject_invalid_fixed_point_input() {
        for invalid in ["", "not-a-number", "NaN", "1e999"] {
            assert!(parse_position(&[invalid.into(), "0".into()]).is_err());
            assert!(parse_position(&["0".into(), invalid.into()]).is_err());
        }
        assert!(parse_fixed(&"0".repeat(49)).is_err());
    }
}
