//! Two existing Arena components, installed by the executable, not editor core.
use egui::{Id, Ui};
use orr_editor::custom_inspector::{Extension, FieldEdit, Registry, SchemaKey, Snapshot};
use orr_fp::{FPVec2, FP};
use orr_reflect::{TypeRegistry, Value};

pub fn panels() -> Result<Registry, String> {
    let mut types = TypeRegistry::new();
    orr_sample::arena_game::register_reflect(&mut types);
    let mut registry = Registry::default();
    registry.register(Extension {
        id: "arena-player-slot-v1",
        schema: SchemaKey::for_type(&types, "PlayerTag")?,
        priority: 0,
        api_version: 1,
        draw: player,
    })?;
    registry.register(Extension {
        id: "arena-bullet-lifetime-v1",
        schema: SchemaKey::for_type(&types, "Bullet")?,
        priority: 0,
        api_version: 1,
        draw: bullet,
    })?;
    Ok(registry)
}

fn player(ui: &mut Ui, _: Id, snapshot: &Snapshot) -> Option<FieldEdit> {
    ui.label(format!(
        "Arena player slot: {:?}",
        snapshot.value().field("slot")
    ));
    ui.label("Assign the authored player to a managed input slot.");
    for slot in 0..=7 {
        if ui.button(format!("Assign slot {slot}")).clicked() {
            return Some(FieldEdit {
                field: "slot".into(),
                value: Value::Int(slot),
            });
        }
    }
    None
}

fn bullet(ui: &mut Ui, _: Id, snapshot: &Snapshot) -> Option<FieldEdit> {
    ui.label(format!(
        "Bullet remaining ticks: {:?}",
        snapshot.value().field("ttl")
    ));
    ui.label(format!(
        "Velocity per tick: {:?}",
        snapshot.value().field("velocity")
    ));
    ui.label(
        "Direction presets set speed to 2 world units per tick. Generic fields allow exact values.",
    );
    for (label, x, y) in [
        ("Velocity left", -2, 0),
        ("Velocity right", 2, 0),
        ("Velocity up", 0, 2),
        ("Velocity down", 0, -2),
        ("Velocity stopped", 0, 0),
    ] {
        let value = Value::Vec2(FPVec2::new(FP::from_int(x), FP::from_int(y)));
        // Do not create an Undo step for a repeated already-applied preset.
        if ui
            .add_enabled(
                snapshot.value().field("velocity") != Some(&value),
                egui::Button::new(label),
            )
            .clicked()
        {
            return Some(FieldEdit {
                field: "velocity".into(),
                value,
            });
        }
    }
    ui.button("Set lifetime to 180 ticks")
        .clicked()
        .then(|| FieldEdit {
            field: "ttl".into(),
            value: Value::Int(180),
        })
}
