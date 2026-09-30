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
//! # agent proposals (what an ERP agent does, for tests and screenshots)
//! propose [--as <agent>] <label>          # new proposal, origin agent:<agent> (default "script"); selects it
//! propose.set <id> <entity> <component> <path> <value>   # stage a field edit
//! propose.rename <id> <entity> <name>     # stage a rename
//! checks <check> [; <check>]...           # the checks box; `checks default` restores it
//! verify <id> [bot <n>]                   # run and wait; default inputs: the last play
//! preview <id> | preview off              # viewport preview (edit mode)
//! accept <id> | reject <id>
//! agent_tab                               # open the Agent tab at start
//! ```
//! `<id>` is `p3`, `3` or `last`.

use std::path::Path;

use orr_fp::{FPVec2, FPVec3, FP};
use orr_edit::{Op, Origin, ProposalId, Target};
use orr_reflect::{decimal, Guid, TypeKind, Value};

use crate::agent::{VerifySource, DEFAULT_CHECKS};
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

fn proposal_id(ed: &Editor, text: &str) -> Result<ProposalId, String> {
    if text == "last" {
        return ed.doc().list_proposals().last().map(|i| i.id).ok_or_else(|| "there are no proposals".to_string());
    }
    let n = text.strip_prefix('p').unwrap_or(text);
    let id = ProposalId(n.parse().map_err(|_| format!("'{text}' is not a proposal id (p3, 3 or last)"))?);
    ed.doc().proposal_info(id).map(|i| i.id).map_err(|e| e.to_string())
}

fn entity_guid(ed: &Editor, name: &str) -> Result<Guid, String> {
    ed.doc()
        .view()
        .entities()
        .into_iter()
        .find(|e| e.name.as_deref() == Some(name) || e.guid.as_ref().is_some_and(|g| g.to_string() == name))
        .and_then(|e| e.guid)
        .ok_or_else(|| format!("no entity named '{name}'"))
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
        "propose" => {
            let (agent, label) = match rest.as_slice() {
                ["--as", name, label @ ..] => (*name, label.join(" ")),
                label => ("script", label.join(" ")),
            };
            if label.is_empty() {
                return Err("'propose' needs a label".into());
            }
            let id = ed.doc_mut().propose(&label, Origin::Agent(agent.to_string())).map_err(|e| e.to_string())?;
            ed.select_proposal(Some(id));
            ed.info(format!("proposal {id} \"{label}\" by agent:{agent}"));
        }
        "propose.set" => {
            need(5)?;
            let id = proposal_id(ed, rest[0])?;
            let (guid, comp, path) = (entity_guid(ed, rest[1])?, rest[2], rest[3]);
            let current = ed.doc().proposal_preview(id).and_then(|v| v.field(&Target::Guid(guid.clone()), comp, path)).map_err(|e| e.to_string())?;
            let value = parse_like(&current, &rest[4..].join(" "))?;
            let op = Op::SetField { guid, component: comp.to_string(), path: path.to_string(), value };
            ed.doc_mut().proposal_apply(id, op).map_err(|e| e.to_string())?;
        }
        "propose.rename" => {
            need(3)?;
            let id = proposal_id(ed, rest[0])?;
            let guid = entity_guid(ed, rest[1])?;
            let op = Op::Rename { guid, name: Some(rest[2..].join(" ")) };
            ed.doc_mut().proposal_apply(id, op).map_err(|e| e.to_string())?;
        }
        "checks" => {
            need(1)?;
            ed.agent_mut().checks = if rest[0] == "default" { DEFAULT_CHECKS.to_string() } else { rest.join(" ").split(';').map(str::trim).collect::<Vec<_>>().join("\n") };
        }
        "verify" => {
            need(1)?;
            let id = proposal_id(ed, rest[0])?;
            let source = match rest.get(1..) {
                Some(["bot", n]) => VerifySource::Bot(n.parse().map_err(|_| "bot needs a tick count")?),
                Some(["bot"]) => VerifySource::Bot(ed.agent().bot_ticks),
                Some([]) | None => VerifySource::LastPlay,
                Some(_) => return Err("verify <id> [bot <ticks>]".into()),
            };
            ed.agent_mut().source = source;
            ed.select_proposal(Some(id));
            ed.start_verify(id, source)?;
            ed.wait_verify();
            let text = match ed.verify_result() {
                Some(run) => match (&run.report, &run.outcome) {
                    (Err(e), _) => format!("verify {id}: {e}"),
                    (Ok(_), Some(o)) if o.passed => format!("verify {id}: all checks passed ({} ms)", run.millis),
                    (Ok(_), Some(o)) => format!("verify {id}: {} check(s) FAILED", o.results.iter().filter(|r| !r.passed).count()),
                    (Ok(_), None) => format!("verify {id}: done"),
                },
                None => format!("verify {id}: no result"),
            };
            ed.info(text);
        }
        "preview" => {
            need(1)?;
            let id = if rest[0] == "off" { None } else { Some(proposal_id(ed, rest[0])?) };
            if !ed.set_preview(id) {
                return Err(last_error(ed));
            }
        }
        "accept" => {
            need(1)?;
            let id = proposal_id(ed, rest[0])?;
            ed.accept_proposal(id).map_err(|e| e.to_string())?;
        }
        "reject" => {
            need(1)?;
            let id = proposal_id(ed, rest[0])?;
            ed.reject_proposal(id).map_err(|e| e.to_string())?;
        }
        "agent_tab" => ed.agent_mut().request_tab(),
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
