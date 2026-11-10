//! Project-owned cadence shortcuts; edits use the normal Inspector transaction path.
use orr_editor::custom_inspector::{Extension, FieldEdit, Registry, SchemaKey, Snapshot};
use orr_games::coop_defense_game::{scenario_types, WAVE_PACING_NAME};
use orr_reflect::Value;

/// Explicitly install only the exact Stormglass pacing schema.
pub fn registry() -> Registry {
    let mut registry = Registry::default();
    registry
        .register(Extension {
            id: "stormglass-wave-cadence",
            schema: SchemaKey::for_type(&scenario_types(), WAVE_PACING_NAME)
                .expect("registered Stormglass pacing schema"),
            priority: 0,
            api_version: 1,
            draw,
        })
        .expect("unique Stormglass cadence inspector");
    registry
}

fn draw(ui: &mut egui::Ui, id: egui::Id, snapshot: &Snapshot) -> Option<FieldEdit> {
    let Value::Struct(fields) = snapshot.value() else {
        return None;
    };
    let Some((_, Value::Int(current))) = fields
        .iter()
        .find(|(name, _)| name == "spawn_interval_ticks")
    else {
        return None;
    };
    ui.label(format!("Current pair interval: {current} ticks"));
    ui.small("60 ticks/second. Choose a cadence, or Use generic fields for any exact value from 35 to 350.");
    let mut edit = None;
    ui.push_id(id, |ui| {
        ui.horizontal_wrapped(|ui| {
            for ticks in [35, 70, 140, 350] {
                if ui
                    .selectable_label(*current == ticks, format!("Every {ticks} ticks"))
                    .clicked()
                    && *current != ticks
                {
                    edit = Some(FieldEdit {
                        field: "spawn_interval_ticks".into(),
                        value: Value::Int(ticks),
                    });
                }
            }
        });
    });
    edit
}

#[cfg(all(test, feature = "api-acceptance"))]
mod tests;
