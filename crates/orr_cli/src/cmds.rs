//! The commands.

use serde_json::{json, Value as J};

use orr_mcp::report;
use orr_mcp::tools::{self, Staged};

use crate::args::{parse, Parsed};
use crate::ops::Resolver;
use crate::{out, out_raw, CliErr, Ctx};

/// Runs one command.
pub fn dispatch(ctx: &mut Ctx, cmd: &str, args: &[String]) -> Result<(), CliErr> {
    match cmd {
        "status" => status(ctx, args),
        "scene" => scene(ctx, args),
        "get" => get(ctx, args),
        "schema" => schema(ctx, args),
        "set" => edit(ctx, "set", args),
        "spawn" => spawn(ctx, args),
        "despawn" | "rename" | "add" | "remove" => edit(ctx, cmd, args),
        "propose" => propose(ctx, args),
        "verify" => verify(ctx, args),
        "accept" => accept_reject(ctx, "accept_proposal", args),
        "reject" => accept_reject(ctx, "reject_proposal", args),
        "proposals" => proposals(ctx, args),
        "diff" => diff(ctx, args),
        "apply" => apply(ctx, args),
        "history" => history(ctx, args),
        "undo" => undo(ctx, args),
        "redo" => redo(ctx, args),
        "sim" => sim(ctx, args),
        "activity" => activity(ctx, args),
        "save" => save(ctx, args),
        "agents-md" => agents_md(ctx, args),
        other => Err(CliErr::Usage(format!("unknown command '{other}'. Run `orr help`."))),
    }
}

fn no_operands(p: &Parsed) -> Result<(), CliErr> {
    match p.pos.first() {
        Some(x) => Err(CliErr::Usage(format!("unexpected argument '{x}'"))),
        None => Ok(()),
    }
}

fn s(v: &J) -> &str {
    v.as_str().unwrap_or("")
}

fn pretty(v: &J) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

// ---- reading ----

fn status(ctx: &mut Ctx, args: &[String]) -> Result<(), CliErr> {
    no_operands(&parse(args, &[], false)?)?;
    let d = ctx.call("rpc.discover", J::Null)?;
    let st = ctx.call("sim.state", J::Null)?;
    let h = ctx.call("history.list", J::Null)?;
    let p = ctx.call("proposal.list", J::Null)?;
    let a = ctx.call("activity.list", json!({"limit": 1}))?;
    let e = &d["engine"];
    let caps: Vec<&str> = d["you"]["capabilities"].as_array().map(|c| c.iter().filter_map(J::as_str).collect()).unwrap_or_default();
    let clients: Vec<&str> = a["clients"].as_array().map(|c| c.iter().filter_map(|x| x["client"].as_str()).collect()).unwrap_or_default();
    let props = p["proposals"].as_array().map(Vec::as_slice).unwrap_or_default();
    let hist = h["entries"].as_array().map_or(0, Vec::len);
    let mut text = format!(
        "{} {} (ERP {}), game {}, build id {}\nhost {}   you: {} [{}]\n",
        s(&e["name"]),
        s(&e["version"]),
        e["erp_version"],
        s(&e["game"]),
        s(&e["build_id"]),
        crate::args::redact_url(&ctx.url),
        s(&d["you"]["client"]),
        caps.join(", ")
    );
    let tick = if st["mode"] == "play" { format!("tick {} ({})", st["head_tick"], if st["playing"] == true { "running" } else { "paused" }) } else { "no play session".to_string() };
    text.push_str(&format!(
        "mode: {}   {}   checksum {}   entities {}   unsaved changes: {}\n",
        s(&st["mode"]),
        tick,
        s(&st["checksum"]),
        st["entities"],
        if h["dirty"] == true { "yes" } else { "no" }
    ));
    text.push_str(&format!(
        "history: {hist} entries (undo: {}, redo: {})   open proposals: {}\n",
        yes(&h["can_undo"]),
        yes(&h["can_redo"]),
        props.len()
    ));
    for q in props {
        text.push_str(&format!("  {}  \"{}\"  by {}  {} op(s)\n", s(&q["id"]), s(&q["label"]), s(&q["origin"]), q["op_count"]));
    }
    text.push_str(&format!("clients connected: {}\n", if clients.is_empty() { "none".to_string() } else { clients.join(", ") }));
    let json = json!({
        "host": crate::args::redact_url(&ctx.url),
        "engine": e,
        "you": d["you"],
        "state": st,
        "history": {"entries": hist, "can_undo": h["can_undo"], "can_redo": h["can_redo"], "dirty": h["dirty"]},
        "proposals": p["proposals"],
        "clients": a["clients"],
    });
    ctx.emit(&text, &json);
    Ok(())
}

fn yes(v: &J) -> &'static str {
    if *v == true { "yes" } else { "no" }
}

fn scene(ctx: &mut Ctx, args: &[String]) -> Result<(), CliErr> {
    let p = parse(args, &[("--components", false), ("--filter", true), ("--has", true), ("--limit", true), ("--offset", true)], false)?;
    no_operands(&p)?;
    let mut has = Vec::new();
    {
        let mut r = Resolver::new(ctx);
        for h in p.all("--has").iter().flat_map(|t| t.split(',')) {
            has.push(r.type_name(h.trim(), "component")?);
        }
    }
    if !p.has("--components") {
        let mut a = json!({});
        if let Some(f) = p.get("--filter") {
            a["name"] = json!(f);
        }
        if !has.is_empty() {
            a["components"] = json!(has);
        }
        for (flag, key) in [("--limit", "limit"), ("--offset", "offset")] {
            if let Some(n) = p.num(flag)? {
                a[key] = json!(n);
            }
        }
        let o = ctx.tool("scene_overview", a)?;
        ctx.emit(&o.text, &o.structured);
        return Ok(());
    }
    let mut q = json!({"values": true, "limit": p.num("--limit")?.unwrap_or(200)});
    if let Some(f) = p.get("--filter") {
        q["name"] = json!(f);
    }
    if !has.is_empty() {
        q["components"] = json!(has);
    }
    if let Some(n) = p.num("--offset")? {
        q["offset"] = json!(n);
    }
    let r = ctx.call("world.query", q)?;
    let singles = ctx.call("world.singleton.get", json!({}))?;
    let mut text = format!("{} entities match, checksum {}.\n", r["total"], s(&r["checksum"]));
    if let Some(m) = singles["singletons"].as_object() {
        for (k, v) in m {
            text.push_str(&format!("singleton {k} = {v}\n"));
        }
    }
    for e in r["entities"].as_array().map(Vec::as_slice).unwrap_or_default() {
        text.push_str(&format!("{}  {}\n", s(&e["id"]), e["name"].as_str().unwrap_or("(unnamed)")));
        match e["values"].as_object() {
            Some(m) => {
                for (k, v) in m {
                    text.push_str(&format!("    {k} = {v}\n"));
                }
            }
            None => text.push_str(&format!("    [{}]\n", e["components"].as_array().map(|c| c.iter().filter_map(J::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default())),
        }
    }
    if r["truncated"] == true {
        text.push_str("(more entities: page with --offset/--limit, or narrow with --filter/--has)\n");
    }
    ctx.emit(&text, &json!({"entities": r["entities"], "total": r["total"], "truncated": r["truncated"], "checksum": r["checksum"], "singletons": singles["singletons"]}));
    Ok(())
}

fn get(ctx: &mut Ctx, args: &[String]) -> Result<(), CliErr> {
    let p = parse(args, &[("--proposal", true)], false)?;
    let Some(target) = p.pos.first() else { return Err(CliErr::Usage("get needs an entity (GUID or name)".into())) };
    if p.pos.len() > 2 {
        return Err(CliErr::Usage(format!("unexpected argument '{}'", p.pos[2])));
    }
    if let Some(single) = target.strip_prefix('@') {
        let (name, path) = single.split_once('.').unwrap_or((single, ""));
        let name = Resolver::new(ctx).type_name(name, "singleton")?;
        let mut a = json!({"name": name});
        if !path.is_empty() {
            a["path"] = json!(path);
        }
        let r = ctx.call("world.singleton.get", a)?;
        let shown = if r["value"].is_null() { r["singletons"].clone() } else { r["value"].clone() };
        ctx.emit(&pretty(&shown), &r);
        return Ok(());
    }
    let mut a = json!({});
    {
        let mut r = Resolver::new(ctx);
        a["entity"] = json!(r.entity(target)?);
        if let Some(spec) = p.pos.get(1) {
            let (c, path) = spec.split_once('.').unwrap_or((spec, ""));
            a["component"] = json!(r.type_name(c, "component")?);
            if !path.is_empty() {
                a["path"] = json!(path);
            }
        }
    }
    if let Some(id) = p.get("--proposal") {
        a["proposal_id"] = json!(id);
    }
    let o = ctx.tool("get_entity", a)?;
    let shown = if o.structured["value"].is_null() { o.structured.clone() } else { o.structured["value"].clone() };
    ctx.emit(&pretty(&shown), &o.structured);
    Ok(())
}

fn schema(ctx: &mut Ctx, args: &[String]) -> Result<(), CliErr> {
    let p = parse(args, &[("--types", false)], false)?;
    let o = if p.has("--types") {
        ctx.tool("get_schema", json!({"list_types": true}))?
    } else {
        let mut a = json!({});
        if let Some(t) = p.pos.first() {
            a["type"] = json!(Resolver::new(ctx).type_name(t, "any")?);
        }
        ctx.tool("get_schema", a)?
    };
    ctx.emit(&o.text, &o.structured);
    Ok(())
}

// ---- direct edits ----

/// The result of a direct edit.
struct Applied {
    lines: Vec<String>,
    spawned: Vec<String>,
    history: J,
}

fn compact(v: &J) -> String {
    if v.is_null() { "?".to_string() } else { v.to_string() }
}

/// Runs `ops` as ONE undo step (a transaction). Rolled back entirely on any failure.
fn direct_apply(ctx: &mut Ctx, label: &str, ops: &[J]) -> Result<Applied, CliErr> {
    let before = ctx.call("history.list", J::Null)?;
    let newest_before = before["entries"].as_array().and_then(|l| l.iter().filter_map(|e| e["id"].as_u64()).max()).unwrap_or(0);
    ctx.call("tx.begin", json!({"label": label}))?;
    let mut lines = Vec::new();
    let mut spawned = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        match run_op(ctx, op) {
            Ok((line, guid)) => {
                lines.push(line);
                spawned.extend(guid);
            }
            Err(e) => {
                let _ = ctx.call("tx.rollback", J::Null);
                return Err(match e {
                    CliErr::Erp(m) => CliErr::Erp(format!("op {} ({}) failed; nothing was changed: {m}", i + 1, s(&op["op"]))),
                    other => other,
                });
            }
        }
    }
    let c = ctx.call("tx.commit", J::Null)?;
    // The entry is ours if it is newer than the newest one from before.
    let history = c["history"]["entries"].as_array().and_then(|l| l.last()).filter(|e| e["id"].as_u64().unwrap_or(0) > newest_before).cloned().unwrap_or(J::Null);
    Ok(Applied { lines, spawned, history })
}

fn get_value(ctx: &mut Ctx, op: &J) -> J {
    let mut a = json!({"entity": op["entity"], "component": op["component"]});
    if let Some(p) = op["path"].as_str() {
        a["path"] = json!(p);
    }
    ctx.call("world.get", a).map(|r| r["value"].clone()).unwrap_or(J::Null)
}

fn run_op(ctx: &mut Ctx, op: &J) -> Result<(String, Option<String>), CliErr> {
    let kind = s(&op["op"]);
    let pass = |keys: &[&str]| {
        let mut m = json!({});
        for k in keys {
            if !op[*k].is_null() {
                m[*k] = op[*k].clone();
            }
        }
        m
    };
    let ent = s(&op["entity"]).to_string();
    let comp = s(&op["component"]).to_string();
    match kind {
        "patch" => {
            let before = get_value(ctx, op);
            ctx.call("world.patch", pass(&["entity", "component", "path", "value"]))?;
            let after = get_value(ctx, op);
            let path = op["path"].as_str().map(|p| format!(".{p}")).unwrap_or_default();
            Ok((format!("set {ent} {comp}{path}: {} -> {}", compact(&before), compact(&after)), None))
        }
        "insert" => {
            ctx.call("world.insert", pass(&["entity", "component", "value"]))?;
            Ok((format!("added {comp} to {ent}"), None))
        }
        "remove" => {
            ctx.call("world.remove", pass(&["entity", "component"]))?;
            Ok((format!("removed {comp} from {ent}"), None))
        }
        "spawn" => {
            let r = ctx.call("world.spawn", pass(&["name", "guid", "components"]))?;
            let guid = r["guid"].as_str().or(r["handle"].as_str()).unwrap_or("?").to_string();
            Ok((format!("spawned {guid}{}", op["name"].as_str().map(|n| format!(" \"{n}\"")).unwrap_or_default()), Some(guid)))
        }
        "despawn" => {
            ctx.call("world.despawn", pass(&["entity"]))?;
            Ok((format!("despawned {ent}"), None))
        }
        "rename" => {
            ctx.call("world.rename", pass(&["entity", "name"]))?;
            Ok((format!("renamed {ent} to {}", compact(&op["name"])), None))
        }
        "singleton.patch" => {
            ctx.call("world.singleton.patch", pass(&["name", "path", "value"]))?;
            let path = op["path"].as_str().map(|p| format!(".{p}")).unwrap_or_default();
            Ok((format!("set singleton {}{path} = {}", s(&op["name"]), compact(&op["value"])), None))
        }
        other => Err(CliErr::Erp(format!("unknown op '{other}'"))),
    }
}

fn label_of(prefix: &str, tokens: &[String]) -> String {
    let mut l = format!("orr {prefix} {}", tokens.join(" "));
    if l.len() > 80 {
        l.truncate(77);
        l.push_str("...");
    }
    l
}

fn print_applied(ctx: &mut Ctx, a: &Applied) {
    let mut text = String::new();
    for l in &a.lines {
        text.push_str(l);
        text.push('\n');
    }
    match a.history.get("id") {
        Some(id) => text.push_str(&format!(
            "Recorded as history entry #{id} \"{}\" by {} (one undo step). `orr undo` takes it back.\n",
            s(&a.history["label"]),
            s(&a.history["origin"])
        )),
        None => text.push_str("No effective change: the scene is as it was.\n"),
    }
    let json = json!({"applied": a.lines, "spawned": a.spawned, "history": a.history});
    ctx.emit(&text, &json);
}

/// `set`, `despawn`, `rename`, `add`, `remove`: one verb of the op syntax as a direct edit.
fn edit(ctx: &mut Ctx, verb: &str, args: &[String]) -> Result<(), CliErr> {
    let mut tokens = vec![verb.to_string()];
    tokens.extend_from_slice(&parse(args, &[], true)?.pos);
    let (ops, used) = Resolver::new(ctx).parse_op(&tokens)?;
    if used != tokens.len() {
        return Err(CliErr::Usage(format!("unexpected argument '{}'", tokens[used])));
    }
    let applied = direct_apply(ctx, &label_of(verb, &tokens[1..]), &ops)?;
    print_applied(ctx, &applied);
    Ok(())
}

fn spawn(ctx: &mut Ctx, args: &[String]) -> Result<(), CliErr> {
    let p = parse(args, &[("--name", true)], false)?;
    let mut tokens = vec!["spawn".to_string()];
    if let Some(n) = p.get("--name") {
        tokens.push("--name".into());
        tokens.push(n.to_string());
    }
    tokens.extend(p.pos.iter().cloned());
    let (ops, used) = Resolver::new(ctx).parse_op(&tokens)?;
    if used != tokens.len() {
        return Err(CliErr::Usage(format!("unexpected argument '{}'", tokens[used])));
    }
    let applied = direct_apply(ctx, &label_of("spawn", &tokens[1..]), &ops)?;
    print_applied(ctx, &applied);
    Ok(())
}

// ---- proposals ----

/// The label and the ops of `propose` / `apply`: `<label> <ops...>`, or the label and a JSON list.
fn label_and_ops(ctx: &mut Ctx, p: &Parsed) -> Result<(String, Vec<J>), CliErr> {
    let Some(label) = p.pos.first() else { return Err(CliErr::Usage("a label is needed first, then the ops".into())) };
    let rest = &p.pos[1..];
    let json_ops: Option<J> = if let Some(f) = p.get("--ops-file") {
        let text = std::fs::read_to_string(f).map_err(|e| CliErr::Usage(format!("--ops-file {f}: {e}")))?;
        Some(serde_json::from_str(&text).map_err(|e| CliErr::Usage(format!("--ops-file {f}: not JSON: {e}")))?)
    } else if rest.len() == 1 && rest[0] == "-" {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut text).map_err(|e| CliErr::Usage(format!("cannot read stdin: {e}")))?;
        Some(serde_json::from_str(&text).map_err(|e| CliErr::Usage(format!("stdin is not a JSON op list: {e}")))?)
    } else {
        None
    };
    let mut r = Resolver::new(ctx);
    let ops = match json_ops {
        Some(j) => {
            if !rest.is_empty() && p.get("--ops-file").is_some() {
                return Err(CliErr::Usage("give either op arguments or --ops-file, not both".into()));
            }
            r.normalize_json_ops(&j)?
        }
        None => {
            if rest.is_empty() {
                return Err(CliErr::Usage("no ops given (e.g. `set hero Body.pos=[6,18]`, or --ops-file ops.json, or `-` for stdin)".into()));
            }
            r.parse_ops(rest)?
        }
    };
    if ops.is_empty() {
        return Err(CliErr::Usage("the op list is empty".into()));
    }
    Ok((label.clone(), ops))
}

const OPS_FLAGS: &[(&str, bool)] = &[
    ("--ops-file", true),
    ("--check", true),
    ("--bot", true),
    ("--idle", true),
    ("--seed", true),
    ("--players", true),
    ("--replay", true),
    ("--ticks", true),
    ("--last-play", false),
    ("--keep", false),
];

fn propose(ctx: &mut Ctx, args: &[String]) -> Result<(), CliErr> {
    let p = parse(args, &[("--ops-file", true)], true)?;
    let (label, ops) = label_and_ops(ctx, &p)?;
    let Staged { id, got, spawned } = tools::stage_proposal(&mut ctx.bridge, &label, J::Array(ops), None).map_err(|f| ctx.fail(f.0))?;
    let mut text = report::describe_proposal(&id, &got);
    text.push_str(&report::spawned_text(&spawned));
    text.push_str(&format!(
        "\nThe scene is unchanged. Next: `orr verify {id} --check \"<rule>\"`, then `orr accept {id}` (or `orr reject {id}`). `orr apply` does all three in one step.\n"
    ));
    let mut j = got;
    j["spawned"] = spawned;
    ctx.emit(&text, &j);
    Ok(())
}

fn is_proposal_id(t: &str) -> bool {
    t.strip_prefix('p').is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

fn proposal_arg(p: &Parsed, what: &str) -> Result<String, CliErr> {
    match p.pos.as_slice() {
        [id] if is_proposal_id(id) => Ok(id.clone()),
        [other] => Err(CliErr::Usage(format!("'{other}' is not a proposal id (like p1); see `orr proposals`"))),
        [] => Err(CliErr::Usage(format!("{what} needs a proposal id (like p1); see `orr proposals`"))),
        _ => Err(CliErr::Usage("only one proposal id is allowed".into())),
    }
}

fn accept_reject(ctx: &mut Ctx, tool: &str, args: &[String]) -> Result<(), CliErr> {
    let p = parse(args, &[], false)?;
    let id = proposal_arg(&p, if tool == "accept_proposal" { "accept" } else { "reject" })?;
    let o = ctx.tool(tool, json!({"proposal_id": id}))?;
    ctx.emit(&o.text, &o.structured);
    Ok(())
}

fn proposals(ctx: &mut Ctx, args: &[String]) -> Result<(), CliErr> {
    let p = parse(args, &[], false)?;
    let a = if p.pos.is_empty() { json!({}) } else { json!({"proposal_id": proposal_arg(&p, "proposals")?}) };
    let o = ctx.tool("list_proposals", a)?;
    ctx.emit(&o.text, &o.structured);
    Ok(())
}

fn diff(ctx: &mut Ctx, args: &[String]) -> Result<(), CliErr> {
    let p = parse(args, &[], false)?;
    let id = proposal_arg(&p, "diff")?;
    let got = ctx.call("proposal.get", json!({"id": id}))?;
    let mut text = report::describe_proposal(&id, &got);
    if got["stale"] == true {
        text.push_str("The scene changed since this proposal was made.\n");
    }
    ctx.emit(&text, &got);
    Ok(())
}

// ---- verify ----

/// `--bot N` and the like as ERP `inputs`.
fn inputs_of(ctx: &mut Ctx, p: &Parsed) -> Result<J, CliErr> {
    let given = ["--bot", "--idle", "--replay"].iter().filter(|f| p.get(f).is_some()).count() + usize::from(p.has("--last-play"));
    if given > 1 {
        return Err(CliErr::Usage("choose one of --bot N, --idle N, --last-play, --replay file".into()));
    }
    if let Some(n) = p.num("--bot")? {
        let mut i = json!({"kind": "bot", "ticks": n});
        if let Some(seed) = p.num("--seed")? {
            i["seed"] = json!(seed);
        }
        if let Some(pl) = p.num("--players")? {
            i["players"] = json!(pl);
        }
        return Ok(i);
    }
    if let Some(n) = p.num("--idle")? {
        return Ok(json!({"kind": "idle", "ticks": n}));
    }
    if p.has("--last-play") {
        return Ok(json!({"kind": "last_play"}));
    }
    if let Some(f) = p.get("--replay") {
        let bytes = std::fs::read(f).map_err(|e| CliErr::Usage(format!("--replay {f}: {e}")))?;
        return Ok(json!({"kind": "replay", "base64": base64(&bytes)}));
    }
    // The default: scripted players if the host has them, else no input.
    let d = ctx.call("rpc.discover", J::Null)?;
    if d["engine"]["verify"]["bot_available"] == true {
        let mut i = json!({"kind": "bot", "ticks": 300});
        if let Some(seed) = p.num("--seed")? {
            i["seed"] = json!(seed);
        }
        if let Some(pl) = p.num("--players")? {
            i["players"] = json!(pl);
        }
        Ok(i)
    } else {
        Ok(json!({"kind": "idle", "ticks": 300}))
    }
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

const VERIFY_FLAGS: &[(&str, bool)] =
    &[("--check", true), ("--bot", true), ("--idle", true), ("--seed", true), ("--players", true), ("--replay", true), ("--ticks", true), ("--last-play", false)];

fn verify_args(ctx: &mut Ctx, p: &Parsed, proposal: Option<&str>, checks: &[String]) -> Result<J, CliErr> {
    let mut a = json!({"inputs": inputs_of(ctx, p)?});
    if let Some(id) = proposal {
        a["proposal_id"] = json!(id);
    }
    if !checks.is_empty() {
        a["checks"] = json!(checks);
    }
    if let Some(t) = p.num("--ticks")? {
        a["ticks"] = json!(t);
    }
    Ok(a)
}

fn checks_failed(report: &J) -> bool {
    report["checks"]["passed"] == false
}

fn verify(ctx: &mut Ctx, args: &[String]) -> Result<(), CliErr> {
    let p = parse(args, VERIFY_FLAGS, false)?;
    let proposal = match p.pos.as_slice() {
        [] => None,
        _ => Some(proposal_arg(&p, "verify")?),
    };
    let checks: Vec<String> = p.all("--check").iter().map(ToString::to_string).collect();
    let a = verify_args(ctx, &p, proposal.as_deref(), &checks)?;
    let o = ctx.tool("verify_proposal", a)?;
    ctx.emit(&o.text, &o.structured);
    if checks_failed(&o.structured) { Err(CliErr::Checks) } else { Ok(()) }
}

// ---- apply ----

fn apply(ctx: &mut Ctx, args: &[String]) -> Result<(), CliErr> {
    let p = parse(args, OPS_FLAGS, true)?;
    let (label, ops) = label_and_ops(ctx, &p)?;
    let mut checks: Vec<String> = p.all("--check").iter().map(ToString::to_string).collect();
    let keep = p.has("--keep");
    // Everything that can be wrong with the command line is checked before anything is staged.
    let mut checks_default = false;
    if checks.is_empty() {
        let d = ctx.call("rpc.discover", J::Null)?;
        if d["engine"]["metrics"].as_array().is_some_and(|m| m.iter().any(|x| x["name"] == "lost_bodies")) {
            checks.push("lost_bodies.max == 0".to_string());
            checks_default = true;
        }
    }
    let inputs_probe = verify_args(ctx, &p, Some("p0"), &checks)?;

    let Staged { id, got, spawned } = tools::stage_proposal(&mut ctx.bridge, &label, J::Array(ops), None).map_err(|f| ctx.fail(f.0))?;
    let mut text = format!("Proposal {id} \"{}\": {} op(s) staged.\n", s(&got["label"]), got["op_count"]);
    for l in got["summary"]["lines"].as_array().map(Vec::as_slice).unwrap_or_default() {
        text.push_str(&format!("  {}\n", s(l)));
    }
    text.push_str(&report::spawned_text(&spawned));
    if checks_default {
        text.push_str("(no --check given: using the default `lost_bodies.max == 0`)\n");
    } else if checks.is_empty() {
        text.push_str("(no --check given and the game reports no lost_bodies metric: nothing is judged; consider --check)\n");
    }
    let mut va = inputs_probe;
    va["proposal_id"] = json!(id);
    let verified = ctx.tool("verify_proposal", va);
    let o = match verified {
        Ok(o) => o,
        Err(e) => {
            if !keep {
                let _ = ctx.call("proposal.reject", json!({"id": id}));
            }
            out(text.trim_end());
            return Err(match e {
                CliErr::Erp(m) => CliErr::Erp(format!("{m}\n{}", if keep { format!("Proposal {id} is left open (--keep).") } else { format!("Proposal {id} was rejected; the scene is unchanged.") })),
                other => other,
            });
        }
    };
    text.push_str(&o.text);
    let failed = checks_failed(&o.structured);
    let mut result = json!({"proposal": id, "label": label, "verify": o.structured, "spawned": spawned});
    if failed {
        if keep {
            text.push_str(&format!("NOT APPLIED: checks failed. Proposal {id} is left open (--keep): change it, then `orr verify {id}` and `orr accept {id}`, or `orr reject {id}`. The scene is unchanged.\n"));
            result["outcome"] = json!("kept");
        } else {
            let _ = ctx.call("proposal.reject", json!({"id": id}));
            text.push_str(&format!("NOT APPLIED: checks failed; proposal {id} rejected. The scene is unchanged.\n"));
            result["outcome"] = json!("rejected");
        }
        result["accepted"] = json!(false);
        ctx.emit(&text, &result);
        return Err(CliErr::Checks);
    }
    match ctx.call("proposal.accept", json!({"id": id})) {
        Ok(r) => {
            result["accepted"] = json!(true);
            result["outcome"] = json!("accepted");
            result["history_id"] = r["history_id"].clone();
            match r["history_id"].as_u64() {
                Some(h) => text.push_str(&format!(
                    "APPLIED: {id} accepted as history entry #{h} ({} op(s)); scene checksum {}. `orr undo` takes it back.\n",
                    r["applied"],
                    s(&r["checksum"])
                )),
                None => text.push_str(&format!("APPLIED: {id} accepted, but its ops changed nothing (no history entry). Scene checksum {}.\n", s(&r["checksum"]))),
            }
            ctx.emit(&text, &result);
            Ok(())
        }
        Err(e) => {
            // The proposal stays open: a conflict needs a new proposal, a missing capability a person.
            out(text.trim_end());
            Err(match e {
                CliErr::Erp(m) => CliErr::Erp(format!("{m}\nChecks passed but the proposal was not accepted; {id} is still open (see `orr proposals`)")),
                other => other,
            })
        }
    }
}

// ---- history ----

fn history(ctx: &mut Ctx, args: &[String]) -> Result<(), CliErr> {
    let p = parse(args, &[("-n", true)], false)?;
    no_operands(&p)?;
    let mut r = ctx.call("history.list", J::Null)?;
    if let Some(n) = p.num("-n")? {
        if let Some(list) = r["entries"].as_array_mut() {
            let skip = list.len().saturating_sub(usize::try_from(n).unwrap_or(usize::MAX));
            list.drain(..skip);
        }
    }
    ctx.emit(&report::history_text(&r), &r);
    Ok(())
}

fn undo(ctx: &mut Ctx, args: &[String]) -> Result<(), CliErr> {
    no_operands(&parse(args, &[], false)?)?;
    let o = ctx.tool("undo", json!({}))?;
    ctx.emit(&o.text, &o.structured);
    Ok(())
}

fn redo(ctx: &mut Ctx, args: &[String]) -> Result<(), CliErr> {
    no_operands(&parse(args, &[], false)?)?;
    let before = ctx.call("history.list", J::Null)?;
    let next = before["entries"].as_array().and_then(|l| l.iter().find(|e| e["undone"] == true)).cloned();
    let r = ctx.call("history.redo", J::Null)?;
    let text = match next {
        Some(e) => format!("Redid entry #{} \"{}\" by {}. Scene checksum {}.\n", e["id"], s(&e["label"]), s(&e["origin"]), s(&r["checksum"])),
        None => format!("Redid the last undone entry. Scene checksum {}.\n", s(&r["checksum"])),
    };
    ctx.emit(&text, &r);
    Ok(())
}

// ---- sim ----

/// `1.5` -> 1500 (thousandths), without floats.
fn permille(t: &str) -> Result<u64, CliErr> {
    let bad = || CliErr::Usage(format!("speed must be a positive number like 0.5, 1 or 2, got '{t}'"));
    let (int, frac) = t.split_once('.').unwrap_or((t, ""));
    if int.is_empty() && frac.is_empty() || !int.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) {
        return Err(bad());
    }
    let whole: u64 = if int.is_empty() { 0 } else { int.parse().map_err(|_| bad())? };
    let mut f = frac.chars().take(3).collect::<String>();
    while f.len() < 3 {
        f.push('0');
    }
    let v = whole.saturating_mul(1000) + f.parse::<u64>().map_err(|_| bad())?;
    if v == 0 { Err(bad()) } else { Ok(v) }
}

fn sim(ctx: &mut Ctx, args: &[String]) -> Result<(), CliErr> {
    let p = parse(args, &[("--players", true)], false)?;
    let Some(action) = p.pos.first().map(String::as_str) else {
        return Err(CliErr::Usage("sim needs an action: start, stop, play, pause, step, seek, speed, state".into()));
    };
    let arg = p.pos.get(1).map(String::as_str);
    if p.pos.len() > 2 {
        return Err(CliErr::Usage(format!("unexpected argument '{}'", p.pos[2])));
    }
    let num = |what: &str| -> Result<u64, CliErr> {
        arg.ok_or_else(|| CliErr::Usage(format!("sim {action} needs {what}")))?.parse().map_err(|_| CliErr::Usage(format!("sim {action}: {what} must be a non-negative integer")))
    };
    let o = match action {
        "state" | "play" | "pause" => ctx.tool("sim_run", json!({"action": action}))?,
        "start" => {
            let mut a = json!({"action": "start"});
            if let Some(n) = p.num("--players")? {
                a["player_count"] = json!(n);
            }
            ctx.tool("sim_run", a)?
        }
        "step" => {
            let n = if arg.is_some() { num("a tick count")? } else { 1 };
            ctx.tool("sim_run", json!({"action": "step", "n": n}))?
        }
        "seek" => ctx.tool("sim_run", json!({"action": "seek", "tick": num("a tick")?}))?,
        "speed" => {
            let x = arg.ok_or_else(|| CliErr::Usage("sim speed needs a factor like 0.5 or 2".into()))?;
            let pm = permille(x)?;
            let r = ctx.call("sim.speed", json!({"permille": pm}))?;
            let shown = r["speed_permille"].as_u64().unwrap_or(pm);
            tools::Out { text: format!("{}Speed {}.{:03}x.\n", report::state_text(&r), shown / 1000, shown % 1000), structured: r }
        }
        "stop" => {
            let r = ctx.call("sim.stop", json!({}))?;
            let text = format!(
                "Play stopped at tick {}, checksum {}. The recording ({} bytes) is kept: `orr verify --last-play` replays it.\n",
                r["tick"],
                s(&r["checksum"]),
                r["replay_bytes"]
            );
            tools::Out { text, structured: r }
        }
        other => return Err(CliErr::Usage(format!("unknown sim action '{other}' (start, stop, play, pause, step, seek, speed, state)"))),
    };
    ctx.emit(&o.text, &o.structured);
    Ok(())
}

// ---- activity ----

/// One activity entry as a line.
fn activity_line(e: &J) -> String {
    let ms = e["at_ms"].as_u64().unwrap_or(0);
    let mut l = format!("#{:<4} +{:>4}.{}s  {:<10} {:<8} {}", e["seq"], ms / 1000, (ms % 1000) / 100, s(&e["client"]), s(&e["kind"]), s(&e["summary"]));
    if e["change"]["old"].is_null() {
        // no previous value to show
    } else {
        l.push_str(&format!("  (was {})", e["change"]["old"]));
    }
    if e["ok"] == false {
        l.push_str(&format!("  ERROR: {}", s(&e["error"])));
    }
    if let Some(v) = e["verify"]["checks"]["passed"].as_bool() {
        l.push_str(if v { "  [checks passed]" } else { "  [checks FAILED]" });
    }
    l
}

fn activity(ctx: &mut Ctx, args: &[String]) -> Result<(), CliErr> {
    let p = parse(args, &[("--since", true), ("--reads", false), ("-n", true), ("-f", false), ("--follow", false)], false)?;
    no_operands(&p)?;
    let follow = p.has("-f") || p.has("--follow");
    let reads = p.has("--reads");
    let since = p.num("--since")?;
    let n = usize::try_from(p.num("-n")?.unwrap_or(if follow && since.is_none() { 10 } else { 50 })).unwrap_or(usize::MAX);
    // Every `orr` call connects and disconnects: those session lines are noise unless asked for (--reads).
    let wanted = |e: &J| reads || e["kind"] != "session";
    let fetch = n.saturating_mul(10).clamp(50, 2000);
    let mut r = ctx.call("activity.list", json!({"since": since.unwrap_or(0), "limit": fetch, "include_reads": reads}))?;
    let mut last = r["last_seq"].as_u64().unwrap_or(0);
    let mut list: Vec<J> = r["entries"].as_array().map(|l| l.iter().filter(|e| wanted(e)).cloned().collect()).unwrap_or_default();
    list.drain(..list.len().saturating_sub(n));
    r["entries"] = J::Array(list.clone());
    if !follow {
        let text = if list.is_empty() { "No activity.\n".to_string() } else { list.iter().map(activity_line).collect::<Vec<_>>().join("\n") + "\n" };
        ctx.emit(&text, &r);
        return Ok(());
    }
    let mut client = orr_remote::ErpClient::connect(&ctx.url, ctx.token.as_deref()).map_err(|e| ctx.conn_err(&e.to_string()))?;
    client.call("watch.subscribe", json!({"topics": ["activity"], "include_reads": reads})).map_err(|e| CliErr::Erp(e.to_string()))?;
    let show = |ctx: &Ctx, e: &J| {
        if ctx.json {
            out(&e.to_string());
        } else {
            out(&activity_line(e));
        }
    };
    for e in &list {
        show(ctx, e);
    }
    // Entries recorded between the listing and the subscription.
    let gap = client.call("activity.list", json!({"since": last, "limit": 2000, "include_reads": reads})).map_err(|e| CliErr::Erp(e.to_string()))?;
    for e in gap["entries"].as_array().map(Vec::as_slice).unwrap_or_default() {
        last = last.max(e["seq"].as_u64().unwrap_or(0));
        if wanted(e) {
            show(ctx, e);
        }
    }
    loop {
        match client.wait_notification("watch.activity", std::time::Duration::from_secs(1)) {
            Ok(Some(n)) => {
                for e in n["params"]["entries"].as_array().map(Vec::as_slice).unwrap_or_default() {
                    let seq = e["seq"].as_u64().unwrap_or(0);
                    if seq > last {
                        last = seq;
                        if wanted(e) {
                            show(ctx, e);
                        }
                    }
                }
            }
            Ok(None) => {}
            Err(e) => return Err(ctx.conn_err(&format!("connection lost: {e}"))),
        }
    }
}

// ---- save, agents-md ----

fn save(ctx: &mut Ctx, args: &[String]) -> Result<(), CliErr> {
    let p = parse(args, &[("--write", false)], false)?;
    no_operands(&p)?;
    if p.has("--write") {
        let r = ctx.call("scene.save", json!({"write": true}))?;
        let text = match r["written"].as_str() {
            Some(path) => format!("Wrote {path}. Scene checksum {}.\n", s(&r["checksum"])),
            None if r["written"] == true => format!("Wrote the host's scene file. Scene checksum {}.\n", s(&r["checksum"])),
            None => return Err(CliErr::Erp("the host has no scene file to write (it was started without one); use `orr save > file.scene.yaml`".into())),
        };
        let mut j = r;
        if let Some(o) = j.as_object_mut() {
            o.remove("text");
        }
        ctx.emit(&text, &j);
        return Ok(());
    }
    let r = ctx.call("scene.save", json!({}))?;
    if ctx.json {
        ctx.emit("", &r);
    } else {
        out_raw(s(&r["text"]));
    }
    Ok(())
}

fn agents_md(ctx: &mut Ctx, args: &[String]) -> Result<(), CliErr> {
    no_operands(&parse(args, &[], false)?)?;
    let text = orr_mcp::generate_agents_md(&mut ctx.bridge, orr_mcp::ToolGroups::ALL).map_err(|f| ctx.fail(f.0))?;
    if ctx.json {
        ctx.emit("", &json!({"text": text}));
    } else {
        out_raw(&text);
    }
    Ok(())
}
