//! Project-owned controls for ten authored wave counts, including inactive waves.
use orr_editor::custom_inspector::{
    Extension, FieldEdit, FieldTarget, Registry, SchemaKey, Snapshot,
};
use orr_games::coop_defense_game::{scenario_types, WAVE_COUNTS_NAME};
use orr_reflect::Value;

pub(super) fn register(registry: &mut Registry) {
    registry
        .register(Extension {
            id: "stormglass-wave-counts",
            schema: SchemaKey::for_type(&scenario_types(), WAVE_COUNTS_NAME)
                .expect("registered Stormglass count schema"),
            priority: 0,
            api_version: 1,
            draw,
        })
        .expect("unique Stormglass counts inspector");
}

fn draw(ui: &mut egui::Ui, id: egui::Id, snapshot: &Snapshot) -> Option<FieldEdit> {
    let Value::Array(counts) = snapshot.value().field("pair_counts")? else {
        return None;
    };
    ui.small("1–28 pairs per wave, one enemy per lane per pair. Later waves stay authored even when the campaign ends early.");
    ui.small("Use generic fields to type an exact count.");
    let mut edit = None;
    ui.push_id(id, |ui| {
        for (index, value) in counts.iter().enumerate() {
            let Value::Int(count) = value else { continue };
            let wave = index + 1;
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("Wave {wave}: {count} pairs"));
                for (delta, enabled, label) in [
                    (-1, *count > 1, format!("Wave {wave}: fewer pairs")),
                    (1, *count < 28, format!("Wave {wave}: more pairs")),
                ] {
                    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() && edit.is_none()
                    {
                        edit = Some(FieldEdit {
                            field: FieldTarget::ArrayElement {
                                field: "pair_counts".into(),
                                index,
                            },
                            value: Value::Int(count + delta),
                        });
                    }
                }
            });
        }
    });
    edit
}

#[cfg(all(test, feature = "api-acceptance"))]
mod tests;
