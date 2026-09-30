//! From the YAML tree to typed scene values, guided by the type registry.
//!
//! The schema decides every type. A scalar becomes a number only where the
//! field is a number, a bool only where the field is a bool. `no` is a
//! string unless the field is a bool, and then it is an error.

use std::collections::BTreeSet;

use orr_ecs::Entity;

use super::node::{Diag, Node, NodeKind, Pos};
use super::{Guid, Scene, SceneEntity, SCENE_SCHEMA};
use crate::decimal::{self, NumberError};
use crate::desc::{Kind, TypeDesc};
use crate::registry::{TypeKind, TypeRegistry};
use crate::value::{ReflectError, Value};

/// Most problems reported for one file.
const MAX_DIAGS: usize = 50;

struct Ctx<'a> {
    guids: BTreeSet<&'a str>,
}

pub(super) fn decode_scene(root: &Node, reg: &TypeRegistry) -> Result<Scene, Vec<Diag>> {
    let mut diags: Vec<Diag> = Vec::new();
    let push = |d: Diag, diags: &mut Vec<Diag>| {
        if diags.len() < MAX_DIAGS {
            diags.push(d);
        }
    };
    let NodeKind::Map(top) = &root.kind else {
        return Err(vec![Diag::new(root.pos, format!("a scene file is a map with 'schema' and 'entities', found {}", root.describe()))]);
    };

    let mut schema: Option<&Node> = None;
    let mut singletons: Option<&Node> = None;
    let mut entities: Option<&Node> = None;
    for (k, v) in top {
        match k.key_text().unwrap_or("") {
            "schema" => schema = Some(v),
            "singletons" => singletons = Some(v),
            "entities" => entities = Some(v),
            other => push(Diag::new(k.pos, format!("unknown top-level key '{other}' (expected schema, singletons, entities)")), &mut diags),
        }
    }

    match schema {
        None => push(Diag::new(root.pos, format!("missing 'schema: {SCENE_SCHEMA}'")), &mut diags),
        Some(n) => match &n.kind {
            NodeKind::Scalar { text, .. } if text == SCENE_SCHEMA => {}
            NodeKind::Scalar { text, .. } => push(
                Diag::new(n.pos, format!("unsupported schema '{text}' (this build reads {SCENE_SCHEMA})")),
                &mut diags,
            ),
            _ => push(Diag::new(n.pos, format!("'schema' must be the text {SCENE_SCHEMA}")), &mut diags),
        },
    }

    let mut scene = Scene::default();

    // Entity keys first, so references can be checked while decoding components.
    let mut guids: BTreeSet<&str> = BTreeSet::new();
    let mut entity_nodes: Vec<(Guid, &Node, &Node)> = Vec::new();
    match entities {
        None => push(Diag::new(root.pos, "missing 'entities' (write 'entities: {}' for an empty scene)"), &mut diags),
        Some(n) => match &n.kind {
            NodeKind::Map(entries) => {
                for (k, v) in entries {
                    let text = k.key_text().unwrap_or("");
                    match Guid::parse(text) {
                        Ok(g) => {
                            guids.insert(text);
                            entity_nodes.push((g, k, v));
                        }
                        Err(why) => push(Diag::new(k.pos, why), &mut diags),
                    }
                }
            }
            _ => push(Diag::new(n.pos, format!("'entities' must be a map from GUID to entity, found {}", n.describe())), &mut diags),
        },
    }
    let ctx = Ctx { guids };

    if let Some(n) = singletons {
        match &n.kind {
            NodeKind::Map(entries) => {
                for (k, v) in entries {
                    let name = k.key_text().unwrap_or("");
                    match reg.get(name) {
                        None => push(Diag::new(k.pos, unknown_type(reg, name, TypeKind::Singleton)), &mut diags),
                        Some(t) if t.kind() != TypeKind::Singleton => push(
                            Diag::new(k.pos, format!("'{name}' is a component, not a singleton; put it on an entity")),
                            &mut diags,
                        ),
                        Some(t) => match decode_typed(t.desc(), t.name(), v, &ctx) {
                            Ok(val) => scene.singletons.push((name.to_string(), val)),
                            Err(d) => push(d, &mut diags),
                        },
                    }
                }
            }
            _ => push(Diag::new(n.pos, format!("'singletons' must be a map from name to value, found {}", n.describe())), &mut diags),
        }
    }
    scene.singletons.sort_by(|a, b| a.0.cmp(&b.0));

    for (guid, key, node) in entity_nodes {
        match decode_entity(node, reg, &ctx, &mut diags) {
            Some(e) => {
                scene.entities.insert(guid, e);
            }
            None => {
                let _ = key;
            }
        }
    }

    if diags.is_empty() {
        Ok(scene)
    } else {
        Err(diags)
    }
}

fn unknown_type(reg: &TypeRegistry, name: &str, kind: TypeKind) -> String {
    let known: Vec<&str> = reg.types().filter(|t| t.kind() == kind).map(|t| t.name()).collect();
    let what = if kind == TypeKind::Component { "component" } else { "singleton" };
    if reg.get(name).is_some() {
        return format!("'{name}' is not a {what}");
    }
    format!("unknown {what} '{name}' (known: {})", if known.is_empty() { "none".to_string() } else { known.join(", ") })
}

fn decode_entity(node: &Node, reg: &TypeRegistry, ctx: &Ctx<'_>, diags: &mut Vec<Diag>) -> Option<SceneEntity> {
    let mut push = |d: Diag| {
        if diags.len() < MAX_DIAGS {
            diags.push(d);
        }
    };
    let NodeKind::Map(entries) = &node.kind else {
        push(Diag::new(node.pos, format!("an entity is a map of 'name' and components, found {}", node.describe())));
        return None;
    };
    let mut ent = SceneEntity::default();
    let mut ok = true;
    for (k, v) in entries {
        let name = k.key_text().unwrap_or("");
        if name == "name" {
            match &v.kind {
                NodeKind::Scalar { text, .. } if !text.is_empty() => ent.name = Some(text.clone()),
                _ => {
                    push(Diag::new(v.pos, format!("'name' must be text, found {}", v.describe())));
                    ok = false;
                }
            }
            continue;
        }
        match reg.get(name) {
            None => {
                push(Diag::new(k.pos, unknown_type(reg, name, TypeKind::Component)));
                ok = false;
            }
            Some(t) if t.kind() != TypeKind::Component => {
                push(Diag::new(k.pos, format!("'{name}' is a singleton; put it under 'singletons'")));
                ok = false;
            }
            Some(t) => match decode_typed(t.desc(), t.name(), v, ctx) {
                Ok(val) => ent.components.push((name.to_string(), val)),
                Err(d) => {
                    push(d);
                    ok = false;
                }
            },
        }
    }
    ent.components.sort_by(|a, b| a.0.cmp(&b.0));
    ok.then_some(ent)
}

/// Decodes a whole component or singleton and checks that it can be written.
fn decode_typed(desc: &TypeDesc, name: &str, node: &Node, ctx: &Ctx<'_>) -> Result<Value, Diag> {
    let value = decode_value(desc, node, ctx)?;
    // Trial write: catches what only the type itself knows (a polygon that is not convex).
    let mut scratch = vec![0u8; desc.size()];
    let probe = resolve_to_none(&value);
    if let Err(e) = desc.write(&mut scratch, &probe) {
        let msg = e.to_string();
        let pos = match &e {
            ReflectError::Invalid(m) | ReflectError::OutOfRange(m) => locate(node, m),
            _ => node.pos,
        };
        return Err(Diag::new(pos, format!("invalid {name}: {msg}")));
    }
    Ok(value)
}

/// The value with every GUID reference replaced by `Entity::NONE`.
fn resolve_to_none(v: &Value) -> Value {
    map_entities(v, &mut |_| Entity::NONE)
}

/// Maps every entity reference in `v` (GUID form) to an `Entity`.
pub(super) fn map_entities(v: &Value, f: &mut dyn FnMut(Option<&str>) -> Entity) -> Value {
    match v {
        Value::EntityGuid(g) => Value::Entity(f(g.as_deref())),
        Value::Array(items) => Value::Array(items.iter().map(|x| map_entities(x, f)).collect()),
        Value::Struct(fields) => Value::Struct(fields.iter().map(|(n, x)| (n.clone(), map_entities(x, f))).collect()),
        Value::Variant(name, fields) => Value::Variant(name.clone(), fields.iter().map(|(n, x)| (n.clone(), map_entities(x, f))).collect()),
        other => other.clone(),
    }
}

/// Best effort position of the part of `node` that an error path (`shape.verts[2]: ...`) names.
fn locate(node: &Node, message: &str) -> Pos {
    let Some((path, _)) = message.split_once(": ") else {
        return node.pos;
    };
    if path.is_empty() || !path.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'[' | b']')) {
        return node.pos;
    }
    let mut cur = node;
    let mut rest = path;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('.') {
            rest = r;
            continue;
        }
        if let Some(r) = rest.strip_prefix('[') {
            let Some(end) = r.find(']') else { return cur.pos };
            let Ok(i) = r[..end].parse::<usize>() else { return cur.pos };
            match &cur.kind {
                NodeKind::Seq(items) => match items.get(i) {
                    Some(n) => cur = n,
                    None => return cur.pos,
                },
                _ => return cur.pos,
            }
            rest = &r[end + 1..];
            continue;
        }
        let end = rest.find(['.', '[']).unwrap_or(rest.len());
        let key = &rest[..end];
        match &cur.kind {
            NodeKind::Map(entries) => match entries.iter().find(|(k, _)| k.key_text() == Some(key)) {
                Some((_, v)) => cur = v,
                None => return cur.pos,
            },
            _ => return cur.pos,
        }
        rest = &rest[end..];
    }
    cur.pos
}

fn expected(what: &str, node: &Node) -> Diag {
    Diag::new(node.pos, format!("expected {what}, found {}", node.describe()))
}

fn leaf_error(node: &Node, e: ReflectError) -> Diag {
    Diag::new(node.pos, e.to_string())
}

fn plain_scalar<'n>(node: &'n Node, what: &str) -> Result<&'n str, Diag> {
    match &node.kind {
        NodeKind::Scalar { text, quoted: false } if !text.is_empty() => Ok(text),
        NodeKind::Scalar { text, quoted: true } => Err(Diag::new(
            node.pos,
            format!("expected {what}, found the string \"{}\" (write it without quotes)", text.chars().take(40).collect::<String>()),
        )),
        _ => Err(expected(what, node)),
    }
}

fn decode_fixed(node: &Node) -> Result<orr_fp::FP, Diag> {
    let text = plain_scalar(node, "a number like 12 or 0.5")?;
    decimal::parse_fp(text).map_err(|e| match e {
        NumberError::NotDecimal => Diag::new(node.pos, format!("expected a plain decimal number (like -12 or 0.5), found '{text}'")),
        NumberError::Overflow => Diag::new(node.pos, format!("the number '{text}' is too large for fixed point (Q48.16)")),
    })
}

fn decode_value(desc: &TypeDesc, node: &Node, ctx: &Ctx<'_>) -> Result<Value, Diag> {
    match &desc.kind {
        Kind::Bool { .. } => {
            let text = plain_scalar(node, "true or false")?;
            match text {
                "true" => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                _ => Err(Diag::new(node.pos, format!("expected true or false, found '{text}' (yes, no, on, off are not booleans here)"))),
            }
        }
        Kind::Int { int, .. } => {
            let text = plain_scalar(node, "an integer")?;
            let n = decimal::parse_int(text).map_err(|e| match e {
                NumberError::NotDecimal => Diag::new(node.pos, format!("expected a plain integer (like 12 or -3), found '{text}'")),
                NumberError::Overflow => Diag::new(node.pos, format!("the integer '{text}' does not fit {}", int.name())),
            })?;
            let v = Value::Int(n);
            desc.check(&v, true).map_err(|e| leaf_error(node, e))?;
            Ok(v)
        }
        Kind::Fixed { .. } => {
            let v = Value::Fixed(decode_fixed(node)?);
            desc.check(&v, true).map_err(|e| leaf_error(node, e))?;
            Ok(v)
        }
        Kind::Fixed32 { .. } => {
            let text = plain_scalar(node, "a number like 12 or 0.5")?;
            let raw = decimal::parse_fp32_raw(text).map_err(|e| match e {
                NumberError::NotDecimal => Diag::new(node.pos, format!("expected a plain decimal number (like -12 or 0.5), found '{text}'")),
                NumberError::Overflow => Diag::new(node.pos, format!("the number '{text}' is too large for a 32-bit fixed point (Q16.16)")),
            })?;
            let v = Value::Fixed32(orr_fp::FP32::from_raw(raw));
            desc.check(&v, true).map_err(|e| leaf_error(node, e))?;
            Ok(v)
        }
        Kind::Vec2 { range } => {
            let items = seq_of(node, 2, "a list of 2 numbers, like [1.5, -2]")?;
            let x = decode_fixed(&items[0])?;
            let y = decode_fixed(&items[1])?;
            let one = TypeDesc::new(Kind::Fixed { range: *range });
            one.check(&Value::Fixed(x), true).map_err(|e| leaf_error(&items[0], e))?;
            one.check(&Value::Fixed(y), true).map_err(|e| leaf_error(&items[1], e))?;
            Ok(Value::Vec2(orr_fp::FPVec2::new(x, y)))
        }
        Kind::Vec3 { range } => {
            let items = seq_of(node, 3, "a list of 3 numbers, like [1.5, 0, -2]")?;
            let x = decode_fixed(&items[0])?;
            let y = decode_fixed(&items[1])?;
            let z = decode_fixed(&items[2])?;
            let one = TypeDesc::new(Kind::Fixed { range: *range });
            for (v, n) in [(x, &items[0]), (y, &items[1]), (z, &items[2])] {
                one.check(&Value::Fixed(v), true).map_err(|e| leaf_error(n, e))?;
            }
            Ok(Value::Vec3(orr_fp::FPVec3 { x, y, z }))
        }
        Kind::Entity => match &node.kind {
            NodeKind::Scalar { text, quoted: false } if text == "null" => Ok(Value::EntityGuid(None)),
            NodeKind::Scalar { text, .. } => {
                if Guid::parse(text).is_err() {
                    return Err(Diag::new(node.pos, format!("expected an entity GUID like e_7f3a91c2 or null, found '{text}'")));
                }
                if !ctx.guids.contains(text.as_str()) {
                    return Err(Diag::new(node.pos, format!("no entity with GUID '{text}' in this scene")));
                }
                Ok(Value::EntityGuid(Some(text.clone())))
            }
            _ => Err(expected("an entity GUID like e_7f3a91c2 or null", node)),
        },
        Kind::Enum { variants, .. } => match &node.kind {
            NodeKind::Scalar { text, .. } if variants.iter().any(|(n, _)| n == text) => Ok(Value::Enum(text.clone())),
            NodeKind::Scalar { text, .. } => Err(Diag::new(
                node.pos,
                format!("'{text}' is not one of {}", variants.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")),
            )),
            _ => Err(expected(
                &format!("one of {}", variants.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")),
                node,
            )),
        },
        Kind::Flags { bits, .. } => {
            let NodeKind::Seq(items) = &node.kind else {
                return Err(expected(
                    &format!("a list of names from {} (may be empty: [])", bits.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")),
                    node,
                ));
            };
            let mut names: Vec<String> = Vec::new();
            for it in items {
                let NodeKind::Scalar { text, .. } = &it.kind else {
                    return Err(expected("a flag name", it));
                };
                if !bits.iter().any(|(n, _)| n == text) {
                    return Err(Diag::new(
                        it.pos,
                        format!("'{text}' is not one of {}", bits.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")),
                    ));
                }
                if names.contains(text) {
                    return Err(Diag::new(it.pos, format!("'{text}' is listed twice")));
                }
                names.push(text.clone());
            }
            // Declaration order, whatever the file order was.
            names.sort_by_key(|n| bits.iter().position(|(b, _)| b == n));
            Ok(Value::Flags(names))
        }
        Kind::Array { elem, len } => {
            let items = seq_of(node, *len, &format!("a list of {len} values"))?;
            Ok(Value::Array(items.iter().map(|n| decode_value(elem, n, ctx)).collect::<Result<_, _>>()?))
        }
        Kind::List { elem, min, max } => {
            let NodeKind::Seq(items) = &node.kind else {
                return Err(expected(&format!("a list of {min} to {max} values"), node));
            };
            if items.len() < *min || items.len() > *max {
                return Err(Diag::new(node.pos, format!("expected {min} to {max} values, found {}", items.len())));
            }
            Ok(Value::Array(items.iter().map(|n| decode_value(elem, n, ctx)).collect::<Result<_, _>>()?))
        }
        Kind::Struct { fields, .. } => {
            let NodeKind::Map(entries) = &node.kind else {
                return Err(expected(
                    &format!("a map with the fields {}", fields.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", ")),
                    node,
                ));
            };
            let list: Vec<(&str, &TypeDesc)> = fields.iter().map(|f| (f.name.as_str(), &f.ty)).collect();
            Ok(Value::Struct(decode_fields(&list, entries, node, ctx, None)?))
        }
        Kind::Tagged(t) => {
            let NodeKind::Map(entries) = &node.kind else {
                return Err(expected("a map with a 'kind' key", node));
            };
            let variant_names = || t.variants.iter().map(|v| v.name.as_str()).collect::<Vec<_>>().join(", ");
            let Some((_, kind_node)) = entries.iter().find(|(k, _)| k.key_text() == Some("kind")) else {
                return Err(Diag::new(node.pos, format!("missing 'kind' (one of {})", variant_names())));
            };
            let kind_text = match &kind_node.kind {
                NodeKind::Scalar { text, .. } => text.as_str(),
                _ => return Err(expected(&format!("one of {}", variant_names()), kind_node)),
            };
            let Some(var) = t.variants.iter().find(|v| v.name == kind_text) else {
                return Err(Diag::new(kind_node.pos, format!("'{kind_text}' is not one of {}", variant_names())));
            };
            let list: Vec<(&str, &TypeDesc)> = var.fields.iter().map(|f| (f.name.as_str(), &f.ty)).collect();
            let fields = decode_fields(&list, entries, node, ctx, Some("kind"))?;
            Ok(Value::Variant(var.name.clone(), fields))
        }
    }
}

fn seq_of<'n>(node: &'n Node, len: usize, what: &str) -> Result<&'n [Node], Diag> {
    match &node.kind {
        NodeKind::Seq(items) if items.len() == len => Ok(items),
        NodeKind::Seq(items) => Err(Diag::new(node.pos, format!("expected {what}, found a list of {}", items.len()))),
        _ => Err(expected(what, node)),
    }
}

fn decode_fields(
    fields: &[(&str, &TypeDesc)],
    entries: &[(Node, Node)],
    node: &Node,
    ctx: &Ctx<'_>,
    skip_key: Option<&str>,
) -> Result<Vec<(String, Value)>, Diag> {
    let names = || fields.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ");
    for (k, _) in entries {
        let key = k.key_text().unwrap_or("");
        if Some(key) == skip_key {
            continue;
        }
        if !fields.iter().any(|(n, _)| *n == key) {
            return Err(Diag::new(k.pos, format!("unknown field '{key}' (fields: {})", names())));
        }
    }
    let mut out = Vec::with_capacity(fields.len());
    for (name, ty) in fields {
        let Some((_, v)) = entries.iter().find(|(k, _)| k.key_text() == Some(name)) else {
            return Err(Diag::new(node.pos, format!("missing field '{name}'")));
        };
        out.push(((*name).to_string(), decode_value(ty, v, ctx)?));
    }
    Ok(out)
}
