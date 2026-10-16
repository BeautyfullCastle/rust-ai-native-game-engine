//! Writing a scene as Strict YAML with clean diffs.
//!
//! Rules, all deterministic:
//! - Top-level keys in fixed order: `schema`, `singletons`, `entities`.
//! - Entities sorted by GUID, components and singletons sorted by name,
//!   struct fields in declaration order.
//! - A value that fits in 100 columns is written on one line in flow style
//!   (`Transform: { pos: [12.5, 0, -4], rot: 90 }`). Longer values use block
//!   style, one field or list item per line.
//! - Numbers are the shortest decimal that reads back to the same bits.
//! - Two spaces per level, `\n` line ends, one final newline.

use super::{Scene, SCENE_SCHEMA};
use crate::decimal;
use crate::value::Value;

/// Widest line the writer keeps in flow style.
const WIDTH: usize = 100;

pub(super) fn write_scene(scene: &Scene) -> String {
    let mut out = String::new();
    for line in &scene.header_comments {
        push_comment(&mut out, 0, line);
    }
    out.push_str("schema: ");
    #[cfg(not(feature = "linked-prefabs"))]
    out.push_str(SCENE_SCHEMA);
    #[cfg(feature = "linked-prefabs")]
    out.push_str(if scene.prefab_links.is_empty() { SCENE_SCHEMA } else { "orr.scene/2" });
    out.push('\n');

    if !scene.singletons.is_empty() {
        out.push_str("singletons:\n");
        for (name, v) in &scene.singletons {
            for c in scene.comments.get(&format!("singleton:{name}")).into_iter().flatten() {
                push_comment(&mut out, 2, c);
            }
            emit_entry(&mut out, 2, &fmt_key(name), v, false);
        }
    }

    if scene.entities.is_empty() {
        out.push_str("entities: {}\n");
    } else {
        out.push_str("entities:\n");
        for (guid, ent) in &scene.entities {
            for c in scene.comments.get(&format!("entity:{guid}")).into_iter().flatten() {
                push_comment(&mut out, 2, c);
            }
            if ent.name.is_none() && ent.components.is_empty() {
                out.push_str(&format!("  {guid}: {{}}\n"));
                continue;
            }
            out.push_str(&format!("  {guid}:\n"));
            if let Some(n) = &ent.name {
                out.push_str(&format!("    name: {}\n", fmt_string(n)));
            }
            for (cname, v) in &ent.components {
                for c in scene.comments.get(&format!("component:{guid}:{cname}")).into_iter().flatten() {
                    push_comment(&mut out, 4, c);
                }
                emit_entry(&mut out, 4, &fmt_key(cname), v, false);
            }
        }
    }
    #[cfg(feature = "linked-prefabs")]
    super::linked::write_links(scene, &mut out);
    out
}

fn push_comment(out: &mut String, indent: usize, text: &str) {
    for _ in 0..indent {
        out.push(' ');
    }
    if text.is_empty() {
        out.push('#');
    } else {
        out.push_str("# ");
        out.push_str(text);
    }
    out.push('\n');
}

fn spaces(n: usize) -> String {
    " ".repeat(n)
}

/// One `key: value` entry at `indent`. `dash` puts `- ` in front of the first line (list item).
fn emit_entry(out: &mut String, indent: usize, key: &str, v: &Value, dash: bool) {
    let lead = if dash { format!("{}- ", spaces(indent - 2)) } else { spaces(indent) };
    let compound = matches!(v, Value::Struct(_) | Value::Variant(..) | Value::Array(_));
    let flow = flow(v);
    let line = format!("{lead}{key}: {flow}");
    if !compound || line.chars().count() <= WIDTH {
        out.push_str(&line);
        out.push('\n');
        return;
    }
    out.push_str(&format!("{lead}{key}:\n"));
    emit_block(out, indent + 2, v);
}

/// The children of a value in block style, at `indent`.
fn emit_block(out: &mut String, indent: usize, v: &Value) {
    match v {
        Value::Struct(fields) => emit_fields(out, indent, fields, None),
        Value::Variant(name, fields) => emit_fields(out, indent, fields, Some(name)),
        Value::Array(items) => {
            for it in items {
                let f = flow(it);
                let line = format!("{}- {f}", spaces(indent));
                let compound = matches!(it, Value::Struct(_) | Value::Variant(..) | Value::Array(_));
                if !compound || line.chars().count() <= WIDTH {
                    out.push_str(&line);
                    out.push('\n');
                } else {
                    match it {
                        Value::Struct(fields) => emit_fields_dash(out, indent + 2, fields, None),
                        Value::Variant(name, fields) => emit_fields_dash(out, indent + 2, fields, Some(name)),
                        _ => {
                            out.push_str(&format!("{}-\n", spaces(indent)));
                            emit_block(out, indent + 2, it);
                        }
                    }
                }
            }
        }
        other => {
            out.push_str(&format!("{}{}\n", spaces(indent), flow(other)));
        }
    }
}

fn kind_value(name: &str) -> Value {
    Value::Enum(name.to_string())
}

fn emit_fields(out: &mut String, indent: usize, fields: &[(String, Value)], tag: Option<&str>) {
    if let Some(t) = tag {
        emit_entry(out, indent, "kind", &kind_value(t), false);
    }
    for (k, v) in fields {
        emit_entry(out, indent, &fmt_key(k), v, false);
    }
}

fn emit_fields_dash(out: &mut String, indent: usize, fields: &[(String, Value)], tag: Option<&str>) {
    let mut first = true;
    if let Some(t) = tag {
        emit_entry(out, indent, "kind", &kind_value(t), true);
        first = false;
    }
    for (k, v) in fields {
        emit_entry(out, indent, &fmt_key(k), v, first);
        first = false;
    }
    if first {
        out.push_str(&format!("{}- {{}}\n", spaces(indent - 2)));
    }
}

/// The whole value in flow style, on one line.
fn flow(v: &Value) -> String {
    match v {
        Value::Bool(b) => b.to_string(),
        Value::Int(n) => decimal::int_to_decimal(*n),
        Value::Fixed(x) => decimal::fp_to_decimal(*x),
        Value::Fixed32(x) => decimal::fp32_raw_to_decimal(x.raw()),
        Value::Vec2(x) => format!("[{}, {}]", decimal::fp_to_decimal(x.x), decimal::fp_to_decimal(x.y)),
        Value::Vec3(x) => format!(
            "[{}, {}, {}]",
            decimal::fp_to_decimal(x.x),
            decimal::fp_to_decimal(x.y),
            decimal::fp_to_decimal(x.z)
        ),
        Value::Entity(_) | Value::EntityGuid(None) => "null".to_string(),
        Value::EntityGuid(Some(g)) => g.clone(),
        Value::Enum(n) => fmt_string(n),
        Value::Flags(names) => format!("[{}]", names.iter().map(|n| fmt_string(n)).collect::<Vec<_>>().join(", ")),
        Value::Array(items) => format!("[{}]", items.iter().map(flow).collect::<Vec<_>>().join(", ")),
        Value::Struct(fields) => flow_map(None, fields),
        Value::Variant(name, fields) => flow_map(Some(name), fields),
    }
}

fn flow_map(tag: Option<&str>, fields: &[(String, Value)]) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(t) = tag {
        parts.push(format!("kind: {}", fmt_string(t)));
    }
    for (k, v) in fields {
        parts.push(format!("{}: {}", fmt_key(k), flow(v)));
    }
    if parts.is_empty() {
        "{}".to_string()
    } else {
        format!("{{ {} }}", parts.join(", "))
    }
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b':') && !s.ends_with(':')
}

fn fmt_key(k: &str) -> String {
    if is_identifier(k) {
        k.to_string()
    } else {
        quote(k)
    }
}

/// Plain when it is safe and cannot look like another type, double quoted otherwise.
pub(super) fn fmt_string(s: &str) -> String {
    let safe_chars = s.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'/' | b' ' | b':'));
    let edge_ok = !s.starts_with(' ') && !s.ends_with(' ') && !s.starts_with('-') && !s.ends_with(':') && !s.contains("  ");
    let first_ok = s.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    let lower = s.to_ascii_lowercase();
    let reserved = matches!(lower.as_str(), "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "y" | "n" | "~");
    if safe_chars && edge_ok && first_ok && !reserved && !s.contains(": ") {
        s.to_string()
    } else {
        quote(s)
    }
}

pub(super) fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{7f}' || c == '\u{2028}' || c == '\u{2029}' || c == '\u{85}' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
