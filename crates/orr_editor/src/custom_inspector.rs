//! Opt-in static custom-inspector contract. No EditorApp wiring is installed.
//!
//! Callbacks are trusted native code, not a sandbox. They receive no host or
//! document handle. Returned intents still require normal host authorization,
//! freshness, validation and Undo handling when eventually applied.
use egui::{Id, Ui};
use orr_reflect::{Kind, TypeDesc, Value};

use crate::inspector::{self, InspEvent};

/// Explicit compatibility key. Names are reflection names, never Rust TypeId.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaKey {
    /// Stable reflected component name, qualified by the game when necessary.
    pub type_name: String,
    /// Schema fingerprint supplied by the host's reflection registry.
    pub fingerprint: u64,
}

/// Owned, read-only input. UI state must be kept separately from this value.
pub struct Snapshot {
    key: SchemaKey,
    desc: TypeDesc,
    value: Value,
}

impl Snapshot {
    /// Reject malformed snapshots before invoking any extension.
    pub fn new(key: SchemaKey, desc: TypeDesc, value: Value) -> Result<Self, String> {
        if key.type_name.is_empty() {
            return Err("empty reflected type name".into());
        }
        check_unique_fields(&value)?;
        desc.check(&value, true).map_err(|e| e.to_string())?;
        Ok(Self { key, desc, value })
    }

    /// Compatibility identity; the caller must obtain it from the current host.
    pub fn key(&self) -> &SchemaKey {
        &self.key
    }
    /// Current reflected value, read-only.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Validate one top-level scalar field and produce an ordinary inspector event.
    /// Nested/structural edits are deliberately left to the generic inspector.
    /// This is not authority to apply an event to a newer or different document.
    pub fn checked_edit(&self, edit: FieldEdit) -> Result<InspEvent, String> {
        let Kind::Struct { fields, .. } = &self.desc.kind else {
            return Err("custom edits require a struct component".into());
        };
        let field = fields
            .iter()
            .find(|f| f.name == edit.field)
            .ok_or_else(|| "unknown top-level field".to_string())?;
        if edit.field.is_empty() || edit.field.contains(['.', '[', ']']) {
            return Err("nested field paths are not supported".into());
        }
        if !matches!(
            field.ty.kind,
            Kind::Bool { .. }
                | Kind::Int { .. }
                | Kind::Fixed { .. }
                | Kind::Fixed32 { .. }
                | Kind::Enum { .. }
                | Kind::Flags { .. }
        ) {
            return Err("custom edits require a scalar field".into());
        }
        field
            .ty
            .check(&edit.value, true)
            .map_err(|e| e.to_string())?;
        Ok(InspEvent::Set {
            path: edit.field,
            value: edit.value,
        })
    }
}

// Reflection checks known/missing fields but accepts repeated field names.
// Reject those before exposing any snapshot, including inside lists/variants.
fn check_unique_fields(value: &Value) -> Result<(), String> {
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            Value::Struct(fields) | Value::Variant(_, fields) => {
                let mut names = std::collections::BTreeSet::new();
                for (name, child) in fields {
                    if !names.insert(name) {
                        return Err(format!("duplicate snapshot field '{name}'"));
                    }
                    pending.push(child);
                }
            }
            Value::Array(items) => pending.extend(items),
            _ => {}
        }
    }
    Ok(())
}
/// One discrete edit, not an arbitrary command or callback chain.
pub struct FieldEdit {
    /// Exact top-level reflected field name.
    pub field: String,
    /// Proposed value; the descriptor checks kind and range.
    pub value: Value,
}

/// Statically linked draw function. Return at most one discrete edit per frame.
pub type Draw = fn(&mut Ui, Id, &Snapshot) -> Option<FieldEdit>;

/// One explicitly installed implementation of API version 1.
pub struct Extension {
    /// Stable extension identifier, distinct from its display label.
    pub id: &'static str,
    /// Exact supported type and schema; no wildcard matching.
    pub schema: SchemaKey,
    /// Higher priority wins. Equal priorities for a schema are rejected.
    pub priority: i32,
    /// Contract version. Currently only 1 is supported.
    pub api_version: u32,
    /// Trusted native presentation code.
    pub draw: Draw,
}

/// Small explicit registry; no filesystem/network discovery or dynamic loading.
#[derive(Default)]
pub struct Registry {
    entries: Vec<Extension>,
}

impl Registry {
    /// Install an extension atomically. Failed registration preserves the registry.
    pub fn register(&mut self, extension: Extension) -> Result<(), String> {
        if extension.api_version != 1
            || extension.id.is_empty()
            || extension.schema.type_name.is_empty()
        {
            return Err("unsupported API version or empty identity".into());
        }
        if self.entries.iter().any(|e| {
            e.id == extension.id
                || (e.schema == extension.schema && e.priority == extension.priority)
        }) {
            return Err("duplicate extension ID or ambiguous priority".into());
        }
        self.entries.push(extension);
        Ok(())
    }

    /// Resolve an exact compatible extension, or request generic fallback.
    pub fn resolve(&self, key: &SchemaKey) -> Option<&Extension> {
        self.entries
            .iter()
            .filter(|e| &e.schema == key)
            .max_by_key(|e| e.priority)
    }

    /// Draw custom UI or generic fallback. Disabled editing invokes no callback
    /// and emits no events, including from the generic fallback.
    /// The eventual host must independently enforce its current play-mode policy.
    pub fn show(
        &self,
        ui: &mut Ui,
        base: Id,
        snapshot: &Snapshot,
        editable: bool,
        out: &mut Vec<InspEvent>,
    ) {
        if !editable {
            ui.add_enabled_ui(false, |ui| {
                inspector::show_component(
                    ui,
                    base,
                    &snapshot.desc,
                    &snapshot.value,
                    &mut Vec::new(),
                );
            });
            return;
        }
        if let Some(extension) = self.resolve(snapshot.key()) {
            if let Some(edit) = (extension.draw)(ui, base.with(extension.id), snapshot) {
                out.push(snapshot.checked_edit(edit).unwrap_or_else(InspEvent::Error));
            }
        } else {
            inspector::show_component(ui, base, &snapshot.desc, &snapshot.value, out);
        }
    }
}
