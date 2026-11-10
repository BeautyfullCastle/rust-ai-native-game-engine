//! Two existing Arena components, installed by the executable, not editor core.
use egui::{Id, Ui};
use orr_editor::custom_inspector::{Extension, FieldEdit, Registry, SchemaKey, Snapshot};
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
    ui.button("Set lifetime to 180 ticks")
        .clicked()
        .then(|| FieldEdit {
            field: "ttl".into(),
            value: Value::Int(180),
        })
}
