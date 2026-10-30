//! Names are display labels; the admitted clip index is always the identity.
fn label(index: u32, name: &str, duration: f32) -> String {
    let name = if name.trim().is_empty() {
        "(unnamed)"
    } else {
        name
    };
    format!("Clip {index}: {name} ({duration:.3}s)")
}

pub(super) fn show(ui: &mut egui::Ui, clips: &[(u32, String, f32)], selected: &mut String) {
    show_labeled(ui, "Animation preview clip choice", clips, selected);
}

/// Stage an admitted clip index; labels are display-only, never identities.
pub(super) fn show_labeled(
    ui: &mut egui::Ui,
    control_label: &str,
    clips: &[(u32, String, f32)],
    selected: &mut String,
) -> bool {
    let before = selected.clone();
    let selected_index = selected.trim().parse::<u32>().ok();
    let selected_label = clips
        .iter()
        .find(|(index, _, _)| Some(*index) == selected_index)
        .map(|(index, name, duration)| label(*index, name, *duration))
        .unwrap_or_else(|| "Choose an admitted clip".into());
    egui::ComboBox::from_label(control_label)
        .selected_text(selected_label)
        .show_ui(ui, |ui| {
            for (index, name, duration) in clips {
                ui.selectable_value(selected, index.to_string(), label(*index, name, *duration));
            }
        });
    *selected != before
}

#[cfg(test)]
#[path = "clip_choices/tests.rs"]
mod tests;
