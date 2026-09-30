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
//! ```

use std::path::Path;

use orr_fp::{FPVec2, FPVec3, FP};
use orr_reflect::{decimal, TypeKind, Value};

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

fn last_error(ed: &Editor) -> String {
    ed.status().map(|m| m.text.clone()).unwrap_or_default()
}

/// Runs one command line.
pub fn run_command(ed: &mut Editor, line: &str) -> Result<(), String> {
    let line = line.split('#').next().unwrap_or("").trim();
    if line.is_empty() {
        return Ok(());
    }
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
            let singleton = ed.doc().types().get(comp).is_some_and(|t| t.kind() == TypeKind::Singleton);
            let (owner, current) = if singleton {
                (Owner::Singleton(comp.to_string()), ed.view().singleton(comp, path).map_err(|e| e.to_string())?)
            } else {
                let t = ed.selection().cloned().ok_or("nothing selected")?;
                (Owner::Component(comp.to_string()), ed.view().field(&t, comp, path).map_err(|e| e.to_string())?)
            };
            let value = parse_like(&current, &text)?;
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
