//! Project-owned enemy health scale panel. Edits use the normal Inspector transaction path.
use orr_editor::custom_inspector::{Extension, FieldEdit, Registry, SchemaKey, Snapshot};
use orr_games::coop_defense_game::{scenario_types, ENEMY_HEALTH_NAME};
use orr_reflect::Value;

const MIN_SCALE: i128 = 1;
// Mirrors the reflected `1..=4` range; the host validates every edit regardless.
const MAX_SCALE: i128 = 4;

pub(super) fn register(registry: &mut Registry) {
    registry
        .register(Extension {
            id: "stormglass-enemy-health",
            schema: SchemaKey::for_type(&scenario_types(), ENEMY_HEALTH_NAME)
                .expect("registered Stormglass enemy health schema"),
            priority: 0,
            api_version: 1,
            draw,
        })
        .expect("unique Stormglass enemy health inspector");
}

fn draw(ui: &mut egui::Ui, id: egui::Id, snapshot: &Snapshot) -> Option<FieldEdit> {
    let Value::Int(current) = snapshot.value().field("multiplier")? else {
        return None;
    };
    let current = *current;
    ui.small("Every enemy's base HP is multiplied by this scale. Use generic fields to type an exact value.");
    let mut edit = None;
    ui.push_id(id, |ui| {
        ui.add(
            egui::ProgressBar::new(current as f32 / MAX_SCALE as f32)
                .text(format!("Enemy health scale x{current} of x{MAX_SCALE}")),
        );
        ui.horizontal_wrapped(|ui| {
            for (enabled, label, target) in [
                (
                    current < MAX_SCALE,
                    "Set enemy health to maximum (x4)",
                    MAX_SCALE,
                ),
                (
                    current > MIN_SCALE,
                    "Reset enemy health to default (x1)",
                    MIN_SCALE,
                ),
            ] {
                if ui.add_enabled(enabled, egui::Button::new(label)).clicked() && edit.is_none() {
                    edit = Some(FieldEdit {
                        field: "multiplier".into(),
                        value: Value::Int(target),
                    });
                }
            }
        });
    });
    edit
}

#[cfg(all(test, feature = "api-acceptance"))]
mod tests;
