//! `orr_reflect::Value` <-> JSON, exactly.
//!
//! # The mapping
//!
//! It is the shape of the scene file (`orr.scene/1`) and of the JSON Schema
//! that `registry.schema` returns:
//!
//! | `Value` | JSON |
//! |---|---|
//! | `Bool` | `true` / `false` |
//! | `Int` | number (`-3`, `4294967295`); a value outside `i64`/`u64` is a decimal string |
//! | `Fixed`, `Fixed32` | **number with an exact decimal text** (`12`, `-0.5`, `0.00002` = raw 1) |
//! | `Vec2`, `Vec3` | `[x, y]`, `[x, y, z]` of fixed-point numbers |
//! | `Entity` | `null` for none, else the frame handle text `"12v0"` (an entity without a GUID) |
//! | `EntityGuid` | `null` or the GUID text `"e_7f3a91c2"` |
//! | `Enum` | the variant name |
//! | `Flags` | array of flag names |
//! | `Array` | array |
//! | `Struct` | object, fields in declaration order |
//! | `Variant(name, fields)` | object `{"kind": name, ...fields}` |
//!
//! # Fixed-point numbers never touch `f64`
//!
//! Output: [`orr_reflect::decimal::fp_to_decimal`] gives the shortest decimal
//! text that reads back to the same raw value (the same text a scene file
//! has: raw 1 = 1/65536 prints as `0.00002`), and it is written as a JSON
//! number with exactly that text (`serde_json`'s `arbitrary_precision`
//! keeps number text as is). Input: the number's text (or a JSON string
//! holding it) goes to [`orr_reflect::decimal::parse_fp`], integer
//! arithmetic only, rounded to the nearest 1/65536. Both spellings are
//! accepted: `1.5` and `"1.5"`. An exponent (`1e-7`, `2.5E3`) is expanded to
//! plain decimal digits by string manipulation first (exponent at most 60),
//! so a client that prints small numbers with an exponent still works. The
//! JSON Schema says `number`; a string with the same text is accepted as a
//! superset, and entity handles (`"12v0"`) are accepted where the schema
//! says GUID (they stand for entities created during play).
//!
//! Integers above 2^53 are exact in the text, but a JavaScript client that
//! parses them as doubles loses them. Such a client should send strings.

use orr_ecs::Entity;
use orr_reflect::{decimal, parse_path, Guid, Kind, PathSeg, TypeDesc, Value};
use serde_json::{Map, Number, Value as J};

use orr_fp::{FPVec2, FPVec3, FP32};

/// A JSON number with exactly this text (which must be valid JSON number text).
fn number_text(text: &str) -> J {
    match text.parse::<Number>() {
        Ok(n) => J::Number(n),
        Err(_) => J::String(text.to_string()),
    }
}

fn fixed_json(v: orr_fp::FP) -> J {
    number_text(&decimal::fp_to_decimal(v))
}

/// The text form of a frame entity handle: `12v0`.
pub fn handle_text(e: Entity) -> String {
    format!("{}v{}", e.index, e.version)
}

/// Parses `12v0`.
pub fn parse_handle(text: &str) -> Option<Entity> {
    let (i, v) = text.split_once('v')?;
    let digits = |s: &str| !s.is_empty() && s.len() <= 10 && s.bytes().all(|c| c.is_ascii_digit());
    if !digits(i) || !digits(v) {
        return None;
    }
    Some(Entity { index: i.parse().ok()?, version: v.parse().ok()? })
}

/// A value as JSON (see the module docs).
pub fn value_to_json(v: &Value) -> J {
    match v {
        Value::Bool(b) => J::Bool(*b),
        Value::Int(i) => {
            if let Ok(n) = i64::try_from(*i) {
                J::Number(n.into())
            } else if let Ok(n) = u64::try_from(*i) {
                J::Number(n.into())
            } else {
                J::String(i.to_string())
            }
        }
        Value::Fixed(f) => fixed_json(*f),
        Value::Fixed32(f) => number_text(&decimal::fp32_raw_to_decimal(f.raw())),
        Value::Vec2(p) => J::Array(vec![fixed_json(p.x), fixed_json(p.y)]),
        Value::Vec3(p) => J::Array(vec![fixed_json(p.x), fixed_json(p.y), fixed_json(p.z)]),
        Value::Entity(e) if *e == Entity::NONE => J::Null,
        Value::Entity(e) => J::String(handle_text(*e)),
        Value::EntityGuid(None) => J::Null,
        Value::EntityGuid(Some(g)) => J::String(g.clone()),
        Value::Enum(n) => J::String(n.clone()),
        Value::Flags(names) => J::Array(names.iter().map(|n| J::String(n.clone())).collect()),
        Value::Array(items) => J::Array(items.iter().map(value_to_json).collect()),
        Value::Struct(fields) => J::Object(fields.iter().map(|(n, x)| (n.clone(), value_to_json(x))).collect()),
        Value::Variant(name, fields) => {
            let mut o = Map::new();
            o.insert("kind".into(), J::String(name.clone()));
            for (n, x) in fields {
                o.insert(n.clone(), value_to_json(x));
            }
            J::Object(o)
        }
    }
}

/// Expands an exponent (`1e-7`) into plain decimal digits, exactly. Text
/// without an exponent is returned as is.
fn expand_exponent(s: &str) -> Result<String, String> {
    let Some(pos) = s.find(['e', 'E']) else { return Ok(s.to_string()) };
    let bad = || format!("'{s}' is not a number");
    let (mant, exp) = (&s[..pos], &s[pos + 1..]);
    let exp: i32 = exp.strip_prefix('+').unwrap_or(exp).parse().map_err(|_| bad())?;
    if exp.abs() > 60 {
        return Err(format!("the exponent of '{s}' is too large"));
    }
    let (neg, mant) = match mant.strip_prefix('-') {
        Some(m) => (true, m),
        None => (false, mant),
    };
    let (int, frac) = mant.split_once('.').unwrap_or((mant, ""));
    if int.is_empty() || !int.bytes().chain(frac.bytes()).all(|c| c.is_ascii_digit()) {
        return Err(bad());
    }
    let digits = format!("{int}{frac}");
    let point = int.len() as i32 + exp;
    let mut out = if point <= 0 {
        format!("0.{}{}", "0".repeat((-point) as usize), digits)
    } else if point as usize >= digits.len() {
        format!("{digits}{}", "0".repeat(point as usize - digits.len()))
    } else {
        format!("{}.{}", &digits[..point as usize], &digits[point as usize..])
    };
    // Strip leading zeros of the integer part (keep one).
    let int_end = out.find('.').unwrap_or(out.len());
    let trimmed = out[..int_end].trim_start_matches('0').to_string();
    let lead = if trimmed.is_empty() { "0".to_string() } else { trimmed };
    out = format!("{lead}{}", &out[int_end..]);
    Ok(if neg { format!("-{out}") } else { out })
}

fn text_of(j: &J, what: &str) -> Result<String, String> {
    match j {
        J::Number(n) => Ok(n.to_string()),
        J::String(s) => Ok(s.clone()),
        other => Err(format!("expected {what}, found {}", json_kind(other))),
    }
}

fn json_kind(j: &J) -> &'static str {
    match j {
        J::Null => "null",
        J::Bool(_) => "a boolean",
        J::Number(_) => "a number",
        J::String(_) => "a string",
        J::Array(_) => "an array",
        J::Object(_) => "an object",
    }
}

fn fixed_of(j: &J) -> Result<orr_fp::FP, String> {
    let text = expand_exponent(&text_of(j, "a number like 12 or -0.5")?)?;
    decimal::parse_fp(&text).map_err(|e| match e {
        decimal::NumberError::NotDecimal => format!("expected a plain decimal number (like -12 or 0.5), found '{text}'"),
        decimal::NumberError::Overflow => format!("the number '{text}' is too large for fixed point (Q48.16)"),
    })
}

fn prefix(seg: &str, e: String) -> String {
    if e.starts_with('[') || e.starts_with('.') {
        format!("{seg}{e}")
    } else {
        format!("{seg}: {e}")
    }
}

fn names_of<I: IntoIterator<Item = S>, S: AsRef<str>>(it: I) -> String {
    it.into_iter().map(|s| s.as_ref().to_string()).collect::<Vec<_>>().join(", ")
}

/// JSON to a value of type `desc`.
///
/// `partial`: a struct may leave fields out (the document fills them from the
/// type's default), at every struct level that is not inside a list or a
/// tagged value. Use it for whole-component values. Fields that are not part
/// of the type, and every type or range mismatch, are errors with the path.
pub fn json_to_value(desc: &TypeDesc, j: &J, partial: bool) -> Result<Value, String> {
    match &desc.kind {
        Kind::Bool { .. } => match j {
            J::Bool(b) => Ok(Value::Bool(*b)),
            other => Err(format!("expected true or false, found {}", json_kind(other))),
        },
        Kind::Int { int, .. } => {
            let mut text = text_of(j, "an integer")?;
            // A client may print 3 as 3.0.
            if let Some((i, f)) = text.split_once('.') {
                if !f.is_empty() && f.bytes().all(|c| c == b'0') {
                    text = i.to_string();
                }
            }
            let text = expand_exponent(&text)?;
            let n = decimal::parse_int(&text).map_err(|e| match e {
                decimal::NumberError::NotDecimal => format!("expected a plain integer (like 12 or -3), found '{text}'"),
                decimal::NumberError::Overflow => format!("the integer '{text}' does not fit {}", int.name()),
            })?;
            let v = Value::Int(n);
            desc.check(&v, true).map_err(|e| e.to_string())?;
            Ok(v)
        }
        Kind::Fixed { .. } => {
            let v = Value::Fixed(fixed_of(j)?);
            desc.check(&v, true).map_err(|e| e.to_string())?;
            Ok(v)
        }
        Kind::Fixed32 { .. } => {
            let text = expand_exponent(&text_of(j, "a number like 12 or -0.5")?)?;
            let raw = decimal::parse_fp32_raw(&text).map_err(|e| match e {
                decimal::NumberError::NotDecimal => format!("expected a plain decimal number (like -12 or 0.5), found '{text}'"),
                decimal::NumberError::Overflow => format!("the number '{text}' is too large for a 32-bit fixed point (Q16.16)"),
            })?;
            let v = Value::Fixed32(FP32::from_raw(raw));
            desc.check(&v, true).map_err(|e| e.to_string())?;
            Ok(v)
        }
        Kind::Vec2 { range } => {
            let items = list_of(j, 2, "a list of 2 numbers, like [1.5, -2]")?;
            let one = TypeDesc::new(Kind::Fixed { range: *range });
            let mut c = [orr_fp::FP::ZERO; 2];
            for (i, it) in items.iter().enumerate() {
                let f = fixed_of(it).map_err(|e| prefix(&format!("[{i}]"), e))?;
                one.check(&Value::Fixed(f), true).map_err(|e| prefix(&format!("[{i}]"), e.to_string()))?;
                c[i] = f;
            }
            Ok(Value::Vec2(FPVec2::new(c[0], c[1])))
        }
        Kind::Vec3 { range } => {
            let items = list_of(j, 3, "a list of 3 numbers, like [1.5, 0, -2]")?;
            let one = TypeDesc::new(Kind::Fixed { range: *range });
            let mut c = [orr_fp::FP::ZERO; 3];
            for (i, it) in items.iter().enumerate() {
                let f = fixed_of(it).map_err(|e| prefix(&format!("[{i}]"), e))?;
                one.check(&Value::Fixed(f), true).map_err(|e| prefix(&format!("[{i}]"), e.to_string()))?;
                c[i] = f;
            }
            Ok(Value::Vec3(FPVec3 { x: c[0], y: c[1], z: c[2] }))
        }
        Kind::Entity => match j {
            J::Null => Ok(Value::EntityGuid(None)),
            J::String(s) => {
                if Guid::parse(s).is_ok() {
                    Ok(Value::EntityGuid(Some(s.clone())))
                } else if let Some(e) = parse_handle(s) {
                    Ok(Value::Entity(e))
                } else {
                    Err(format!("expected an entity GUID like e_7f3a91c2 (or null), found '{s}'"))
                }
            }
            other => Err(format!("expected an entity GUID like e_7f3a91c2 or null, found {}", json_kind(other))),
        },
        Kind::Enum { variants, .. } => match j {
            J::String(s) if variants.iter().any(|(n, _)| n == s) => Ok(Value::Enum(s.clone())),
            J::String(s) => Err(format!("'{s}' is not one of {}", names_of(variants.iter().map(|(n, _)| n)))),
            other => Err(format!("expected one of {}, found {}", names_of(variants.iter().map(|(n, _)| n)), json_kind(other))),
        },
        Kind::Flags { bits, .. } => {
            let J::Array(items) = j else {
                return Err(format!(
                    "expected a list of names from {} (may be empty: [])",
                    names_of(bits.iter().map(|(n, _)| n))
                ));
            };
            let mut names: Vec<String> = Vec::new();
            for it in items {
                let J::String(s) = it else { return Err("expected a flag name".to_string()) };
                if !bits.iter().any(|(n, _)| n == s) {
                    return Err(format!("'{s}' is not one of {}", names_of(bits.iter().map(|(n, _)| n))));
                }
                if names.contains(s) {
                    return Err(format!("'{s}' is listed twice"));
                }
                names.push(s.clone());
            }
            names.sort_by_key(|n| bits.iter().position(|(b, _)| b == n));
            Ok(Value::Flags(names))
        }
        Kind::Array { elem, len } => {
            let items = list_of(j, *len, &format!("a list of {len} values"))?;
            items
                .iter()
                .enumerate()
                .map(|(i, x)| json_to_value(elem, x, false).map_err(|e| prefix(&format!("[{i}]"), e)))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array)
        }
        Kind::List { elem, min, max } => {
            let J::Array(items) = j else { return Err(format!("expected a list of {min} to {max} values")) };
            if items.len() < *min || items.len() > *max {
                return Err(format!("expected {min} to {max} values, found {}", items.len()));
            }
            items
                .iter()
                .enumerate()
                .map(|(i, x)| json_to_value(elem, x, false).map_err(|e| prefix(&format!("[{i}]"), e)))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array)
        }
        Kind::Struct { fields, .. } => {
            let J::Object(obj) = j else {
                return Err(format!(
                    "expected an object with the fields {}, found {}",
                    names_of(fields.iter().map(|f| &f.name)),
                    json_kind(j)
                ));
            };
            let list: Vec<(&str, &TypeDesc)> = fields.iter().map(|f| (f.name.as_str(), &f.ty)).collect();
            Ok(Value::Struct(fields_of(&list, obj, partial, None)?))
        }
        Kind::Tagged(t) => {
            let J::Object(obj) = j else { return Err(format!("expected an object with a 'kind' key, found {}", json_kind(j))) };
            let names = || names_of(t.variants.iter().map(|v| &v.name));
            let Some(kind) = obj.get("kind") else { return Err(format!("missing 'kind' (one of {})", names())) };
            let J::String(kind) = kind else { return Err(format!("'kind' must be one of {}", names())) };
            let Some(var) = t.variants.iter().find(|v| &v.name == kind) else {
                return Err(format!("'{kind}' is not one of {}", names()));
            };
            let list: Vec<(&str, &TypeDesc)> = var.fields.iter().map(|f| (f.name.as_str(), &f.ty)).collect();
            Ok(Value::Variant(var.name.clone(), fields_of(&list, obj, false, Some("kind"))?))
        }
    }
}

fn list_of<'a>(j: &'a J, len: usize, what: &str) -> Result<&'a [J], String> {
    match j {
        J::Array(items) if items.len() == len => Ok(items),
        J::Array(items) => Err(format!("expected {what}, found a list of {}", items.len())),
        other => Err(format!("expected {what}, found {}", json_kind(other))),
    }
}

fn fields_of(
    fields: &[(&str, &TypeDesc)],
    obj: &Map<String, J>,
    partial: bool,
    skip_key: Option<&str>,
) -> Result<Vec<(String, Value)>, String> {
    for k in obj.keys() {
        if Some(k.as_str()) != skip_key && !fields.iter().any(|(n, _)| n == k) {
            return Err(format!("unknown field '{k}' (fields: {})", names_of(fields.iter().map(|(n, _)| n))));
        }
    }
    let mut out = Vec::with_capacity(fields.len());
    for (name, ty) in fields {
        match obj.get(*name) {
            Some(x) => out.push(((*name).to_string(), json_to_value(ty, x, partial).map_err(|e| prefix(name, e))?)),
            None if partial => {}
            None => return Err(format!("missing field '{name}'")),
        }
    }
    Ok(out)
}

/// The type at `path` inside `desc` (`""` = `desc` itself). Vector
/// components (`pos.x`) are fixed-point numbers. In a tagged value a field is
/// looked up in the first variant that has it.
pub fn desc_at_path(desc: &TypeDesc, path: &str) -> Result<TypeDesc, String> {
    let segs = parse_path(path).map_err(|e| e.to_string())?;
    let mut cur = desc.clone();
    for seg in &segs {
        cur = match (&cur.kind, seg) {
            (Kind::Struct { fields, .. }, PathSeg::Field(n)) => fields
                .iter()
                .find(|f| &f.name == n)
                .map(|f| f.ty.clone())
                .ok_or_else(|| format!("no field '{n}' (fields: {})", names_of(fields.iter().map(|f| &f.name))))?,
            (Kind::Array { elem, len }, PathSeg::Index(i)) => {
                if i >= len {
                    return Err(format!("index {i} is out of range for {len} elements"));
                }
                (**elem).clone()
            }
            (Kind::List { elem, .. }, PathSeg::Index(_)) => (**elem).clone(),
            (Kind::Vec2 { range }, PathSeg::Field(n)) if n == "x" || n == "y" => TypeDesc::new(Kind::Fixed { range: *range }),
            (Kind::Vec3 { range }, PathSeg::Field(n)) if n == "x" || n == "y" || n == "z" => {
                TypeDesc::new(Kind::Fixed { range: *range })
            }
            (Kind::Tagged(t), PathSeg::Field(n)) => t
                .variants
                .iter()
                .flat_map(|v| v.fields.iter())
                .find(|f| &f.name == n)
                .map(|f| f.ty.clone())
                .ok_or_else(|| format!("no field '{n}' in any variant"))?,
            _ => return Err(format!("path '{path}' does not fit the type")),
        };
    }
    Ok(cur)
}

/// Rebuilds `v`, replacing every entity reference with what `f` returns.
pub fn map_entity_refs(v: &Value, f: &mut dyn FnMut(&Value) -> Result<Value, String>) -> Result<Value, String> {
    Ok(match v {
        Value::EntityGuid(_) | Value::Entity(_) => f(v)?,
        Value::Array(items) => Value::Array(items.iter().map(|x| map_entity_refs(x, f)).collect::<Result<_, _>>()?),
        Value::Struct(fields) => {
            Value::Struct(fields.iter().map(|(n, x)| Ok((n.clone(), map_entity_refs(x, f)?))).collect::<Result<_, String>>()?)
        }
        Value::Variant(name, fields) => Value::Variant(
            name.clone(),
            fields.iter().map(|(n, x)| Ok((n.clone(), map_entity_refs(x, f)?))).collect::<Result<_, String>>()?,
        ),
        other => other.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_fp::FP;

    fn fixed_desc() -> TypeDesc {
        TypeDesc::fixed()
    }

    #[test]
    fn fixed_is_exact_decimal_text() {
        let one = FP::from_raw(1);
        let j = value_to_json(&Value::Fixed(one));
        assert_eq!(j.to_string(), "0.00002", "shortest text that reads back to raw 1");
        let back = json_to_value(&fixed_desc(), &j, false).unwrap();
        assert_eq!(back, Value::Fixed(one));
        // Any spelling of 1/65536 reads as raw 1, as a number or a string.
        let back = json_to_value(&fixed_desc(), &J::String("0.00001525878".into()), false).unwrap();
        assert_eq!(back, Value::Fixed(one));
    }

    #[test]
    fn exponents_expand_exactly() {
        assert_eq!(expand_exponent("1e-7").unwrap(), "0.0000001");
        assert_eq!(expand_exponent("2.5E3").unwrap(), "2500");
        assert_eq!(expand_exponent("-1.25e1").unwrap(), "-12.5");
        assert_eq!(expand_exponent("0.5e0").unwrap(), "0.5");
        assert_eq!(expand_exponent("1e+2").unwrap(), "100");
        assert!(expand_exponent("1e999").is_err());
        assert!(expand_exponent("e5").is_err());
    }

    #[test]
    fn handles() {
        assert_eq!(parse_handle("12v3"), Some(Entity { index: 12, version: 3 }));
        assert_eq!(parse_handle("12"), None);
        assert_eq!(parse_handle("v3"), None);
        assert_eq!(handle_text(Entity { index: 1, version: 0 }), "1v0");
    }
}
