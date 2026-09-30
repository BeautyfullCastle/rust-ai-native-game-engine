//! Entity references inside [`Value`]s: GUID form (scene) and `Entity` form (frame).

use orr_reflect::Value;

use crate::error::EditError;

/// Rebuilds `v`, replacing every entity reference (`Value::EntityGuid` and
/// `Value::Entity`) with what `f` returns for it.
pub(crate) fn map_refs(v: &Value, f: &mut dyn FnMut(&Value) -> Result<Value, EditError>) -> Result<Value, EditError> {
    Ok(match v {
        Value::EntityGuid(_) | Value::Entity(_) => f(v)?,
        Value::Array(items) => Value::Array(items.iter().map(|x| map_refs(x, f)).collect::<Result<_, _>>()?),
        Value::Struct(fields) => {
            Value::Struct(fields.iter().map(|(n, x)| Ok((n.clone(), map_refs(x, f)?))).collect::<Result<_, EditError>>()?)
        }
        Value::Variant(name, fields) => {
            Value::Variant(name.clone(), fields.iter().map(|(n, x)| Ok((n.clone(), map_refs(x, f)?))).collect::<Result<_, EditError>>()?)
        }
        other => other.clone(),
    })
}

/// True if `v` holds a reference to the entity with this GUID text.
pub(crate) fn refers_to(v: &Value, guid: &str) -> bool {
    match v {
        Value::EntityGuid(Some(g)) => g == guid,
        Value::Array(items) => items.iter().any(|x| refers_to(x, guid)),
        Value::Struct(f) | Value::Variant(_, f) => f.iter().any(|(_, x)| refers_to(x, guid)),
        _ => false,
    }
}
