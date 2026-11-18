//! Project-owned opening-boss toggles for the ten authored waves.
use orr_editor::custom_inspector::{
    Extension, FieldEdit, FieldTarget, Registry, SchemaKey, Snapshot,
};
use orr_games::coop_defense_game::{scenario_types, WAVE_COMPOSITION_NAME};
use orr_reflect::Value;

pub(super) fn register(registry: &mut Registry) {
    registry
        .register(Extension {
            id: "stormglass-opening-boss",
            schema: SchemaKey::for_type(&scenario_types(), WAVE_COMPOSITION_NAME)
                .expect("registered Stormglass composition schema"),
            priority: 0,
            api_version: 1,
            draw,
        })
        .expect("unique Stormglass opening boss inspector");
}

fn draw(ui: &mut egui::Ui, id: egui::Id, snapshot: &Snapshot) -> Option<FieldEdit> {
    let Value::Array(flags) = snapshot.value().field("opening_boss")? else {
        return None;
    };
    ui.small("A boss pair replaces the first pair of its wave. Use generic fields for enemy kind patterns.");
    let mut edit = None;
    ui.push_id(id, |ui| {
        for (index, flag) in flags.iter().enumerate() {
            let Value::Int(flag) = flag else { continue };
            let wave = index + 1;
            let (state, label, next) = if *flag == 0 {
                ("regular", format!("Wave {wave}: add opening boss"), 1)
            } else {
                ("boss", format!("Wave {wave}: remove opening boss"), 0)
            };
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("Wave {wave} opens with {state} enemies"));
                if ui.button(label).clicked() && edit.is_none() {
                    edit = Some(FieldEdit {
                        field: FieldTarget::ArrayElement {
                            field: "opening_boss".into(),
                            index,
                        },
                        value: Value::Int(next),
                    });
                }
            });
        }
    });
    edit
}

#[cfg(all(test, feature = "api-acceptance"))]
mod tests;
