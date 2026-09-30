//! Values, names and the op mini-syntax.
//!
//! ```text
//! set <entity> <Component.path>=<value>...   patch fields
//! rename <entity> <name>                     set the display name
//! spawn [--name n] [Component=<json>...]     create an entity
//! despawn <entity>
//! add <entity> <Component> [json]            add a component
//! remove <entity> <Component>
//! sset <Singleton.path>=<value>...           patch singleton fields
//! ```
//!
//! An `<entity>` is a GUID (`e_0000000e`) or a display name; a component may
//! be written by its short name when that is unambiguous. Every op becomes
//! the JSON op object of ERP proposals.

use serde_json::{json, Value as J};

use crate::{Ctx, CliErr};

/// The op verbs of the mini-syntax.
pub const VERBS: &[&str] = &["set", "rename", "spawn", "despawn", "add", "remove", "sset"];

/// A value from the command line: JSON when it parses (exact decimals kept),
/// else a bare decimal (`.5`, `+3`, `7.`), else a string.
pub fn parse_value(text: &str) -> J {
    if let Ok(v) = serde_json::from_str::<J>(text) {
        return v;
    }
    if let Some(n) = bare_number(text) {
        if let Ok(v) = serde_json::from_str::<J>(&n) {
            return v;
        }
    }
    J::String(text.to_string())
}

/// `.5` -> `0.5`, `+3` -> `3`, `7.` -> `7`, `-.25` -> `-0.25`; `None` if it is not a plain decimal.
fn bare_number(t: &str) -> Option<String> {
    let (neg, body) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let (int, frac) = body.split_once('.').unwrap_or((body, ""));
    if (int.is_empty() && frac.is_empty()) || !int.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let int = if int.is_empty() { "0" } else { int };
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    out.push_str(int);
    if !frac.is_empty() {
        out.push('.');
        out.push_str(frac);
    }
    Some(out)
}

fn is_guid(t: &str) -> bool {
    t.strip_prefix("e_").is_some_and(|h| !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit()))
}

fn is_handle(t: &str) -> bool {
    t.split_once('v').is_some_and(|(a, b)| !a.is_empty() && !b.is_empty() && a.chars().all(|c| c.is_ascii_digit()) && b.chars().all(|c| c.is_ascii_digit()))
}

/// True if `t` is `name=...` with a plausible name (a component or field path), not a verb or a flag.
pub fn is_assignment(t: &str) -> bool {
    match t.split_once('=') {
        Some((lhs, _)) => !lhs.is_empty() && lhs.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':' || c == '.'),
        None => false,
    }
}

/// Splits `Component.path=value` into (component, path, value).
pub fn split_assignment(t: &str) -> Result<(&str, &str, J), CliErr> {
    let (lhs, value) = t.split_once('=').filter(|_| is_assignment(t)).ok_or_else(|| CliErr::Usage(format!("expected Component.path=value, got '{t}'")))?;
    let (comp, path) = lhs.split_once('.').unwrap_or((lhs, ""));
    Ok((comp, path, parse_value(value)))
}

/// Looks names up on the host: entities by display name, types by short name.
pub struct Resolver<'a> {
    ctx: &'a mut Ctx,
    types: Option<Vec<(String, String)>>,
    /// Names given by earlier `rename` ops of the same list: (new name, GUID).
    renamed: Vec<(String, String)>,
}

impl<'a> Resolver<'a> {
    pub fn new(ctx: &'a mut Ctx) -> Resolver<'a> {
        Resolver { ctx, types: None, renamed: Vec::new() }
    }

    /// The GUID (or play handle) of an entity given by GUID or display name.
    pub fn entity(&mut self, token: &str) -> Result<String, CliErr> {
        if is_guid(token) || is_handle(token) {
            return Ok(token.to_string());
        }
        if let Some((_, g)) = self.renamed.iter().rev().find(|(n, _)| n == token) {
            return Ok(g.clone());
        }
        let r = self.ctx.call("world.query", json!({"name": token, "limit": 5000}))?;
        let list = r["entities"].as_array().map(Vec::as_slice).unwrap_or_default();
        let line = |e: &J| {
            let comps: Vec<&str> = e["components"].as_array().map(|c| c.iter().filter_map(J::as_str).collect()).unwrap_or_default();
            format!("  {}  {}  [{}]", e["id"].as_str().unwrap_or("?"), e["name"].as_str().unwrap_or("(unnamed)"), comps.join(", "))
        };
        let exact: Vec<&J> = list.iter().filter(|e| e["name"] == token).collect();
        match exact.as_slice() {
            [one] => Ok(one["id"].as_str().unwrap_or_default().to_string()),
            [] => {
                let near: Vec<String> = list.iter().take(6).map(line).collect();
                let mut msg = format!("no entity named '{token}' (and it is not a GUID like e_0000000e).");
                if near.is_empty() {
                    msg.push_str(" See `orr scene`.");
                } else {
                    msg.push_str(&format!(" Names containing it:\n{}", near.join("\n")));
                }
                Err(CliErr::Erp(msg))
            }
            many => {
                let lines: Vec<String> = many.iter().map(|e| line(e)).collect();
                Err(CliErr::Erp(format!("'{token}' is ambiguous: {} entities have that name. Use a GUID:\n{}", many.len(), lines.join("\n"))))
            }
        }
    }

    fn load_types(&mut self) -> Result<&[(String, String)], CliErr> {
        if self.types.is_none() {
            let r = self.ctx.call("registry.types", J::Null)?;
            let list = r["types"]
                .as_array()
                .map(|l| l.iter().map(|t| (t["name"].as_str().unwrap_or("").to_string(), t["kind"].as_str().unwrap_or("").to_string())).collect())
                .unwrap_or_default();
            self.types = Some(list);
        }
        Ok(self.types.as_deref().unwrap_or_default())
    }

    /// A registered type name from a full or short name. `kind`: `component`, `singleton` or `any`.
    pub fn type_name(&mut self, token: &str, kind: &str) -> Result<String, CliErr> {
        let types: Vec<(String, String)> = self.load_types()?.iter().filter(|(_, k)| kind == "any" || k == kind).cloned().collect();
        if types.iter().any(|(n, _)| n == token) {
            return Ok(token.to_string());
        }
        let last = |n: &str| n.rsplit("::").next().unwrap_or(n).to_string();
        let mut hits: Vec<&str> = types.iter().filter(|(n, _)| last(n) == token).map(|(n, _)| n.as_str()).collect();
        if hits.is_empty() {
            let low = token.to_ascii_lowercase();
            hits = types.iter().filter(|(n, _)| last(n).to_ascii_lowercase() == low).map(|(n, _)| n.as_str()).collect();
        }
        match hits.as_slice() {
            [one] => Ok((*one).to_string()),
            [] => {
                let names: Vec<&str> = types.iter().map(|(n, _)| n.as_str()).collect();
                let what = if kind == "any" { "type".to_string() } else { format!("{kind} type") };
                Err(CliErr::Erp(format!("unknown {what} '{token}'. Known: {}", names.join(", "))))
            }
            many => Err(CliErr::Erp(format!("'{token}' is ambiguous: it matches {}. Use the full name.", many.join(", ")))),
        }
    }

    /// Turns one op of the mini-syntax, starting at `tokens[0]`, into ops; returns them and the number of tokens used.
    pub fn parse_op(&mut self, tokens: &[String]) -> Result<(Vec<J>, usize), CliErr> {
        let verb = tokens[0].as_str();
        let need = |n: usize, usage: &str| -> Result<(), CliErr> {
            if tokens.len() > n { Ok(()) } else { Err(CliErr::Usage(format!("'{verb}' needs: {usage}"))) }
        };
        match verb {
            "set" => {
                need(2, "set <entity> <Component.path>=<value>...")?;
                let entity = self.entity(&tokens[1])?;
                let mut ops = Vec::new();
                let mut used = 2;
                while let Some(t) = tokens.get(used).filter(|t| is_assignment(t)) {
                    let (comp, path, value) = split_assignment(t)?;
                    let component = self.type_name(comp, "component")?;
                    let mut op = json!({"op": "patch", "entity": entity, "component": component, "value": value});
                    if !path.is_empty() {
                        op["path"] = json!(path);
                    }
                    ops.push(op);
                    used += 1;
                }
                if ops.is_empty() {
                    return Err(CliErr::Usage("'set' needs at least one Component.path=value after the entity".into()));
                }
                Ok((ops, used))
            }
            "sset" => {
                need(1, "sset <Singleton.path>=<value>...")?;
                let mut ops = Vec::new();
                let mut used = 1;
                while let Some(t) = tokens.get(used).filter(|t| is_assignment(t)) {
                    let (name, path, value) = split_assignment(t)?;
                    let name = self.type_name(name, "singleton")?;
                    let mut op = json!({"op": "singleton.patch", "name": name, "value": value});
                    if !path.is_empty() {
                        op["path"] = json!(path);
                    }
                    ops.push(op);
                    used += 1;
                }
                if ops.is_empty() {
                    return Err(CliErr::Usage("'sset' needs at least one Singleton.path=value".into()));
                }
                Ok((ops, used))
            }
            "rename" => {
                need(2, "rename <entity> <new name>")?;
                let entity = self.entity(&tokens[1])?;
                let name = tokens[2].clone();
                self.renamed.retain(|(_, g)| *g != entity);
                self.renamed.push((name.clone(), entity.clone()));
                Ok((vec![json!({"op": "rename", "entity": entity, "name": name})], 3))
            }
            "despawn" => {
                need(1, "despawn <entity>")?;
                let entity = self.entity(&tokens[1])?;
                Ok((vec![json!({"op": "despawn", "entity": entity})], 2))
            }
            "remove" => {
                need(2, "remove <entity> <Component>")?;
                let entity = self.entity(&tokens[1])?;
                let component = self.type_name(&tokens[2], "component")?;
                Ok((vec![json!({"op": "remove", "entity": entity, "component": component})], 3))
            }
            "add" => {
                need(2, "add <entity> <Component> [json]")?;
                let entity = self.entity(&tokens[1])?;
                let component = self.type_name(&tokens[2], "component")?;
                let mut op = json!({"op": "insert", "entity": entity, "component": component});
                let mut used = 3;
                if let Some(v) = tokens.get(3).filter(|t| !VERBS.contains(&t.as_str())) {
                    op["value"] = parse_value(v);
                    used = 4;
                }
                Ok((vec![op], used))
            }
            "spawn" => {
                let mut op = json!({"op": "spawn"});
                let mut comps = serde_json::Map::new();
                let mut used = 1;
                while let Some(t) = tokens.get(used) {
                    if t == "--name" {
                        let n = tokens.get(used + 1).ok_or_else(|| CliErr::Usage("spawn --name needs a value".into()))?;
                        op["name"] = json!(n);
                        used += 2;
                    } else if is_assignment(t) {
                        let (comp, path, value) = split_assignment(t)?;
                        if !path.is_empty() {
                            return Err(CliErr::Usage(format!("spawn takes whole components (Component=<json>), not '{comp}.{path}'")));
                        }
                        comps.insert(self.type_name(comp, "component")?, value);
                        used += 1;
                    } else {
                        break;
                    }
                }
                if !comps.is_empty() {
                    op["components"] = J::Object(comps);
                }
                Ok((vec![op], used))
            }
            other => Err(CliErr::Usage(format!("unknown op '{other}' (ops: {})", VERBS.join(", ")))),
        }
    }

    /// Parses a whole token list of the mini-syntax.
    pub fn parse_ops(&mut self, tokens: &[String]) -> Result<Vec<J>, CliErr> {
        let mut ops = Vec::new();
        let mut i = 0;
        while i < tokens.len() {
            let (mut got, used) = self.parse_op(&tokens[i..])?;
            ops.append(&mut got);
            i += used;
        }
        Ok(ops)
    }

    /// Normalises ops written as JSON (`--ops-file`): names for entities and
    /// short type names are resolved like on the command line.
    pub fn normalize_json_ops(&mut self, ops: &J) -> Result<Vec<J>, CliErr> {
        let list = match ops {
            J::Array(l) => l.clone(),
            J::Object(o) if o.get("ops").is_some_and(J::is_array) => o["ops"].as_array().cloned().unwrap_or_default(),
            _ => return Err(CliErr::Usage("the ops must be a JSON list of op objects (or {\"ops\": [...]})".into())),
        };
        let mut out = Vec::new();
        for mut op in list {
            if !op.is_object() {
                return Err(CliErr::Usage("each op must be a JSON object like {\"op\":\"patch\",...}".into()));
            }
            if let Some(e) = op["entity"].as_str().map(str::to_string) {
                op["entity"] = json!(self.entity(&e)?);
            }
            if let Some(c) = op["component"].as_str().map(str::to_string) {
                op["component"] = json!(self.type_name(&c, "component")?);
            }
            if op["op"] == "singleton.patch" {
                if let Some(n) = op["name"].as_str().map(str::to_string) {
                    op["name"] = json!(self.type_name(&n, "singleton")?);
                }
            }
            if let Some(m) = op["components"].as_object().cloned() {
                let mut fixed = serde_json::Map::new();
                for (k, v) in m {
                    fixed.insert(self.type_name(&k, "component")?, v);
                }
                op["components"] = J::Object(fixed);
            }
            out.push(op);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_parse_as_json_then_decimal_then_string() {
        assert_eq!(parse_value("[6,18]").to_string(), "[6,18]");
        assert_eq!(parse_value("0.1").to_string(), "0.1", "exact decimal text is kept");
        assert_eq!(parse_value(".5").to_string(), "0.5");
        assert_eq!(parse_value("+3").to_string(), "3");
        assert_eq!(parse_value("-.25").to_string(), "-0.25");
        assert_eq!(parse_value("7.").to_string(), "7");
        assert_eq!(parse_value("dynamic"), json!("dynamic"));
        assert_eq!(parse_value("e_00000005"), json!("e_00000005"));
        assert_eq!(parse_value("null"), J::Null);
        assert_eq!(parse_value("\"7\""), json!("7"));
        assert_eq!(parse_value("true"), json!(true));
        assert_eq!(parse_value("1.2.3"), json!("1.2.3"));
    }

    #[test]
    fn assignments_split_at_the_first_dot_and_equals() {
        let (c, p, v) = split_assignment("orr_physics::Body.shape.radius=0.5").unwrap();
        assert_eq!((c, p, v.to_string().as_str()), ("orr_physics::Body", "shape.radius", "0.5"));
        let (c, p, v) = split_assignment("Body={\"a\":\"x=y\"}").unwrap();
        assert_eq!((c, p, v.to_string().as_str()), ("Body", "", "{\"a\":\"x=y\"}"));
        assert!(is_assignment("Body.pos=[1,2]") && !is_assignment("rename") && !is_assignment("=3") && !is_assignment("--name=x"));
    }

    #[test]
    fn guids_and_handles_are_not_names() {
        assert!(is_guid("e_0000000e") && !is_guid("e_") && !is_guid("body_05") && !is_guid("e_xyz"));
        assert!(is_handle("12v0") && !is_handle("v0") && !is_handle("hero"));
    }
}
