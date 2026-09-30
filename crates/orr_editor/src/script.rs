//! `--script`: editor commands from a text file, one per line. For headless
//! checks and screenshots; every command goes through the same [`Editor`]
//! methods as the UI.
//!
//! ```text
//! # comment
//! select body_05            # or a GUID, or `select none`
//! set orr_physics::Body pos 3 4.5      # component, path, value
//! set Scene max_entities 5000          # a singleton
//! spawn | delete | undo | redo | save [path]
//! play | pause | step [n] | seek <tick> | speed <x> | branch | stop
//!
//! # agent proposals (stage what an ERP agent would, to look at it; there is no approval step)
//! propose [--as <agent>] <label>          # new proposal, origin agent:<agent> (default "script")
//! propose.set <id> <entity> <component> <path> <value>   # stage a field edit
//! propose.rename <id> <entity> <name>     # stage a rename
//! preview <id> | preview off              # viewport preview (edit mode)
//! agent_tab                               # open the Agent tab at start
//! agent_expand <method>...                # feed rows of these methods start expanded
//! ```
//! `<id>` is `p3`, `3` or `last`.

use std::path::Path;

use orr_fp::{FPVec2, FPVec3, FP};
use orr_reflect::{decimal, Guid, TypeKind, Value};
use orr_remote::json::value_to_json;
use serde_json::{json, Value as J};

use crate::editor::{Editor, Owner};

/// Parses `text` into a value of the same kind as `current`.
pub fn parse_like(current: &Value, text: &str) -> Result<Value, String> {
    let nums = |n: usize| -> Result<Vec<FP>, String> {
        let parts: Vec<&str> = text.split([' ', ',']).filter(|s| !s.is_empty()).collect();
        if parts.len() != n {
            return Err(format!("expected {n} numbers, got {}", parts.len()));
        }
        parts.iter().map(|p| decimal::parse_fp(p).map_err(|_| format!("'{p}' is not a plain decimal number"))).collect()
    };
    match current {
        Value::Fixed(_) => decimal::parse_fp(text).map(Value::Fixed).map_err(|_| format!("'{text}' is not a plain decimal number")),
        Value::Fixed32(_) => decimal::parse_fp32_raw(text)
            .map(|r| Value::Fixed32(orr_fp::FP32::from_raw(r)))
            .map_err(|_| format!("'{text}' is not a plain decimal number")),
        Value::Int(_) => decimal::parse_int(text).map(Value::Int).map_err(|_| format!("'{text}' is not a plain integer")),
        Value::Bool(_) => match text {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => Err("expected true or false".to_string()),
        },
        Value::Enum(_) => Ok(Value::Enum(text.to_string())),
        Value::Vec2(_) => nums(2).map(|v| Value::Vec2(FPVec2::new(v[0], v[1]))),
        Value::Vec3(_) => nums(3).map(|v| Value::Vec3(FPVec3::new(v[0], v[1], v[2]))),
        other => Err(format!("cannot set a {} from text", other.kind_name())),
    }
}

/// The proposal a script command names: `p3`, `3` or `last`.
fn proposal_id(ed: &Editor, text: &str) -> Result<String, String> {
    if text == "last" {
        return ed.proposals().last().map(|i| i.id.clone()).ok_or_else(|| "there are no proposals".to_string());
    }
    let n = text.strip_prefix('p').unwrap_or(text);
    let id = format!("p{}", n.parse::<u64>().map_err(|_| format!("'{text}' is not a proposal id (p3, 3 or last)"))?);
    ed.proposals().iter().find(|i| i.id == id).map(|i| i.id.clone()).ok_or_else(|| format!("unknown proposal {id}"))
}

fn entity_guid(ed: &Editor, name: &str) -> Result<Guid, String> {
    ed.rows()
        .iter()
        .find(|e| e.name.as_deref() == Some(name) || e.guid.as_ref().is_some_and(|g| g.to_string() == name))
        .and_then(|e| e.guid.clone())
        .ok_or_else(|| format!("no entity named '{name}'"))
}

fn last_error(ed: &Editor) -> String {
    ed.status().map(|m| m.text.clone()).unwrap_or_default()
}

/// Stages ops on a proposal as the agent that made it.
fn stage(ed: &mut Editor, id: &str, ops: J) -> Result<(), String> {
    let agent = ed.script_proposal_owner(id).unwrap_or("script").to_string();
    let c = ed.agent_client(&agent)?;
    c.call("proposal.apply", json!({"id": id, "ops": ops})).map(|_| ()).map_err(|e| e.to_string())?;
    Ok(())
}

/// Runs one command line. The editor is brought up to date first and after,
/// so a command sees what the one before did.
pub fn run_command(ed: &mut Editor, line: &str) -> Result<(), String> {
    let line = line.split('#').next().unwrap_or("").trim();
    if line.is_empty() {
        return Ok(());
    }
    ed.sync();
    let r = run_one(ed, line);
    ed.sync();
    r
}

fn run_one(ed: &mut Editor, line: &str) -> Result<(), String> {
    let mut parts = line.split_whitespace();
    let cmd = parts.next().unwrap_or_default();
    let rest: Vec<&str> = parts.collect();
    let need = |n: usize| if rest.len() < n { Err(format!("'{cmd}' needs {n} argument(s)")) } else { Ok(()) };
    match cmd {
        "select" => {
            need(1)?;
            if rest[0] == "none" {
                ed.select(None);
            } else if !ed.select_named(rest[0]) {
                return Err(format!("no entity named '{}'", rest[0]));
            }
        }
        "set" => {
            need(3)?;
            let (comp, path) = (rest[0], rest[1]);
            let text = rest[2..].join(" ");
            let singleton = ed.types().get(comp).is_some_and(|t| t.kind() == TypeKind::Singleton);
            let owner = if singleton { Owner::Singleton(comp.to_string()) } else { Owner::Component(comp.to_string()) };
            if !singleton && ed.selection().is_none() {
                return Err("nothing selected".to_string());
            }
            let value = ed.parse_field_text(comp, path, &text)?;
            let errors_before = ed.log().iter().filter(|m| m.error).count();
            ed.set_field(&owner, path, value);
            if ed.log().iter().filter(|m| m.error).count() > errors_before {
                return Err(last_error(ed));
            }
        }
        "spawn" => {
            let c = ed.camera.center;
            if !ed.spawn_body(c) {
                return Err(last_error(ed));
            }
        }
        "delete" => {
            if !ed.delete_selected() {
                return Err(last_error(ed));
            }
        }
        "undo" => {
            ed.undo();
        }
        "redo" => {
            ed.redo();
        }
        "save" => {
            let ok = match rest.first() {
                Some(p) => ed.save_as(Path::new(p)),
                None => ed.save(),
            };
            if !ok {
                return Err(last_error(ed));
            }
        }
        "play" => ed.play(),
        "pause" => ed.pause(),
        "step" => ed.step(rest.first().map_or(Ok(1), |n| n.parse::<u32>()).map_err(|_| "step needs a whole number")?),
        "seek" => {
            need(1)?;
            ed.seek(rest[0].parse().map_err(|_| "seek needs a tick number")?);
        }
        "speed" => {
            need(1)?;
            ed.set_speed(rest[0].parse().map_err(|_| "speed needs a number like 0.5")?);
        }
        "branch" => ed.branch(),
        "stop" => {
            ed.stop();
        }
        "propose" => {
            let (agent, label) = match rest.as_slice() {
                ["--as", name, label @ ..] => (*name, label.join(" ")),
                label => ("script", label.join(" ")),
            };
            if label.is_empty() {
                return Err("'propose' needs a label".into());
            }
            let c = ed.agent_client(agent)?;
            let r = c.call("proposal.begin", json!({"label": label})).map_err(|e| e.to_string())?;
            let id = r["id"].as_str().unwrap_or_default().to_string();
            ed.note_script_proposal(&id, agent);
            ed.info(format!("proposal {id} \"{label}\" by agent:{agent}"));
        }
        "propose.set" => {
            need(5)?;
            let id = proposal_id(ed, rest[0])?;
            let (guid, comp, path) = (entity_guid(ed, rest[1])?, rest[2], rest[3]);
            let value = ed.parse_field_text(comp, path, &rest[4..].join(" "))?;
            let op = json!({"op": "patch", "entity": guid.to_string(), "component": comp, "path": path, "value": value_to_json(&value)});
            stage(ed, &id, json!([op]))?;
        }
        "propose.rename" => {
            need(3)?;
            let id = proposal_id(ed, rest[0])?;
            let guid = entity_guid(ed, rest[1])?;
            let op = json!({"op": "rename", "entity": guid.to_string(), "name": rest[2..].join(" ")});
            stage(ed, &id, json!([op]))?;
        }
        "preview" => {
            need(1)?;
            let id = if rest[0] == "off" { None } else { Some(proposal_id(ed, rest[0])?) };
            if !ed.set_preview(id) {
                return Err(last_error(ed));
            }
        }
        "agent_tab" => ed.agent_mut().request_tab(),
        "agent_expand" => {
            need(1)?;
            ed.agent_mut().expand_methods.extend(rest.iter().map(|m| m.to_string()));
        }
        other => return Err(format!("unknown command '{other}'")),
    }
    Ok(())
}

/// Runs a whole script. `Err` names the line.
pub fn run_script(ed: &mut Editor, text: &str) -> Result<(), String> {
    for (i, line) in text.lines().enumerate() {
        run_command(ed, line).map_err(|e| format!("script line {}: {e}", i + 1))?;
    }
    Ok(())
}
