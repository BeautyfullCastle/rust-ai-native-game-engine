//! A single-connection semantic action, never a simulation lifecycle command.
use crate::{args::parse, CliErr, Ctx};
use orr_remote::{ClientError, ErpClient, PumpedWs};
use serde_json::{json, Value as J};
use std::time::{Duration, Instant};

struct Options {
    intent: String,
    payload: J,
    player: u8,
    timeout: Duration,
}
fn options(args: &[String]) -> Result<Options, CliErr> {
    // Do not echo arguments: payloads may contain private game data.
    let p = parse(
        args,
        &[
            ("--payload", true),
            ("--player", true),
            ("--timeout-ms", true),
        ],
        false,
    )
    .map_err(|_| CliErr::Usage("invalid gameplay options; see `orr help gameplay`".into()))?;
    if p.pos.len() != 2 || p.pos[0] != "act" {
        return Err(CliErr::Usage("expected gameplay act <intent>".into()));
    }
    let intent = &p.pos[1];
    if intent.is_empty()
        || intent.len() > 96
        || !intent
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-:".contains(&b))
    {
        return Err(CliErr::Usage("invalid gameplay intent ID".into()));
    }
    for flag in ["--payload", "--player", "--timeout-ms"] {
        if p.all(flag).len() > 1 {
            return Err(CliErr::Usage("duplicate gameplay option".into()));
        }
    }
    let raw = p.get("--payload").unwrap_or("{}");
    if raw.len() > 64 * 1024 {
        return Err(CliErr::Usage("gameplay payload exceeds 64 KiB".into()));
    }
    let payload = serde_json::from_str(raw)
        .map_err(|_| CliErr::Usage("gameplay payload must be JSON".into()))?;
    let player = p
        .get("--player")
        .unwrap_or("0")
        .parse::<u8>()
        .ok()
        .filter(|p| *p < 16)
        .ok_or_else(|| CliErr::Usage("player must be 0..15".into()))?;
    let ms = p
        .get("--timeout-ms")
        .unwrap_or("10000")
        .parse::<u64>()
        .ok()
        .filter(|ms| (100..=60000).contains(ms))
        .ok_or_else(|| CliErr::Usage("timeout-ms must be 100..60000".into()))?;
    Ok(Options {
        intent: intent.clone(),
        payload,
        player,
        timeout: Duration::from_millis(ms),
    })
}
fn failed(reason: &str) -> CliErr {
    CliErr::Erp(format!(
        "gameplay {reason}; no retry was attempted; completion is not confirmed"
    ))
}
fn remaining(deadline: Instant) -> Result<Duration, CliErr> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left < Duration::from_millis(50) {
        Err(failed("deadline elapsed"))
    } else {
        Ok(left)
    }
}
trait Session {
    fn call(&mut self, method: &str, params: J, left: Duration) -> Result<J, CliErr>;
}
impl Session for ErpClient {
    fn call(&mut self, method: &str, params: J, left: Duration) -> Result<J, CliErr> {
        self.call_timeout = left;
        ErpClient::call(self,method,params).map_err(|e|match e {
            ClientError::Rpc(_)=>failed("request rejected"),
            _=>CliErr::Conn("gameplay connection failed; outcome may be uncertain; no retry or reconnect was attempted".into()),
        })
    }
}
fn request(
    s: &mut impl Session,
    deadline: Instant,
    method: &str,
    mut params: J,
) -> Result<J, CliErr> {
    let left = remaining(deadline)?;
    params["timeout_ms"] = json!(left.as_millis().min(5000) as u64);
    let value = s.call(method, params, left)?;
    remaining(deadline)?;
    Ok(value)
}
fn checksum(v: &J) -> bool {
    v.as_str().is_some_and(|s| {
        s.len() == 18 && s.starts_with("0x") && s[2..].bytes().all(|b| b.is_ascii_hexdigit())
    })
}
fn decimal(v: &J) -> Option<u64> {
    let s = v.as_str()?;
    let n = s.parse::<u64>().ok()?;
    (n.to_string() == s).then_some(n)
}
fn fences(v: &J) -> Result<(), CliErr> {
    if ["binding", "connection"]
        .iter()
        .any(|k| decimal(&v[k]).is_none_or(|n| n == 0))
        || v["scope"].as_str().is_none_or(|s| {
            s.is_empty() || s.len() > 256 || !s.bytes().all(|b| b.is_ascii_graphic())
        })
    {
        return Err(failed("invalid fence"));
    }
    Ok(())
}
fn same(a: &J, b: &J) -> Result<(), CliErr> {
    fences(b)?;
    if ["binding", "connection", "game", "build_id"]
        .iter()
        .any(|k| a.get(k).is_some() && b.get(k).is_some() && a[k] != b[k])
    {
        return Err(failed("stale connection or game"));
    }
    Ok(())
}
fn running(v: &J) -> Result<(), CliErr> {
    if v["mode"] != "play"
        || v["playing"] != true
        || v["error"] != false
        || v["cleanup_pending"] != false
    {
        return Err(failed("requires an already running, ready game"));
    }
    Ok(())
}
fn describe(s: &mut impl Session, d: Instant, intent: &str) -> Result<J, CliErr> {
    let v = request(s, d, "editor.gameplay.describe", json!({"intent":intent}))?;
    fences(&v)?;
    if v["game"]
        .as_str()
        .is_none_or(|s| s.is_empty() || s.len() > 256)
        || !checksum(&v["build_id"])
        || v["intent"] != intent
        || v["max_players"].as_u64().is_none_or(|n| n == 0 || n > 16)
        || [
            "can_claim",
            "can_submit",
            "playing",
            "cleanup_pending",
            "error",
        ]
        .iter()
        .any(|k| !v[k].is_boolean())
        || !matches!(
            v["phase"].as_str(),
            Some("off" | "claiming" | "active" | "releasing")
        )
    {
        return Err(failed("invalid gameplay description"));
    }
    Ok(v)
}
fn mutation(v: &J, key: u64, fields: J) -> J {
    let mut p = json!({"binding":v["binding"],"connection":v["connection"],"scope":v["scope"],"request_key":key.to_string()});
    p.as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    p
}
fn pause(d: Instant) -> Result<(), CliErr> {
    let left = remaining(d)?;
    std::thread::sleep(Duration::from_millis(5).min(left));
    Ok(())
}
fn receipt(v: &J) -> Result<J, CliErr> {
    let c = &v["consumed"];
    let n = &v["neutral"];
    if decimal(&c["tick"])
        .zip(decimal(&n["tick"]))
        .is_none_or(|(c, n)| n <= c)
    {
        return Err(failed("invalid consumption receipts"));
    }
    for k in ["stream_generation", "play_epoch", "timeline_epoch"] {
        if decimal(&c[k]).is_none() || c[k] != n[k] {
            return Err(failed("receipt scope mismatch"));
        }
    }
    for stamp in [c, n] {
        if !checksum(&stamp["checksum"]) {
            return Err(failed("invalid receipt checksum"));
        }
    }
    // Copy only documented scalar fields; never reflect host extras or lease data.
    let stamp = |v: &J| json!({"tick":v["tick"],"stream_generation":v["stream_generation"],"play_epoch":v["play_epoch"],"timeline_epoch":v["timeline_epoch"],"checksum":v["checksum"]});
    Ok(
        json!({"status":"completed","command_id":v["command_id"],"consumed":stamp(c),"neutral":stamp(n)}),
    )
}
fn act(s: &mut impl Session, o: &Options, d: Instant) -> Result<J, CliErr> {
    let first = describe(s, d, &o.intent)?;
    fences(&first)?;
    running(&first)?;
    if first["can_claim"] != true
        || first["max_players"]
            .as_u64()
            .is_none_or(|n| u64::from(o.player) >= n)
    {
        return Err(failed("control is unavailable for this player"));
    }
    let claimed = request(
        s,
        d,
        "editor.gameplay.control",
        mutation(&first, 1, json!({"action":"claim","player":o.player})),
    )?;
    same(&first, &claimed)?;
    if claimed["status"] != "queued" {
        return Err(failed("claim was not acknowledged"));
    }
    let ready = loop {
        let v = describe(s, d, &o.intent)?;
        same(&first, &v)?;
        running(&v)?;
        if v["can_submit"] == true && v["phase"] == "active" {
            break v;
        }
        if !matches!(v["phase"].as_str(), Some("claiming" | "active")) {
            return Err(failed("claim did not become active"));
        }
        pause(d)?;
    };
    let accepted = request(
        s,
        d,
        "editor.gameplay.submit",
        mutation(&ready, 2, json!({"intent":o.intent,"payload":o.payload})),
    )?;
    same(&ready, &accepted)?;
    let id = decimal(&accepted["command_id"])
        .filter(|n| *n > 0)
        .ok_or_else(|| failed("invalid command receipt"))?;
    let mut status = accepted;
    let result = loop {
        same(&ready, &status)?;
        if status["scope"] != ready["scope"] {
            return Err(failed("stale gameplay scope"));
        }
        if decimal(&status["command_id"]) != Some(id) {
            return Err(failed("command receipt mismatch"));
        }
        match status["status"].as_str() {
            Some("completed") => break receipt(&status)?,
            Some("pending" | "consumed") => {}
            _ => return Err(failed("command did not complete")),
        }
        pause(d)?;
        status = request(
            s,
            d,
            "editor.gameplay.status",
            json!({"binding":ready["binding"],"connection":ready["connection"],"command_id":id.to_string()}),
        )?;
    };
    let latest = describe(s, d, &o.intent)?;
    same(&ready, &latest)?;
    running(&latest)?;
    if latest["scope"] != ready["scope"] || latest["phase"] != "active" {
        return Err(failed("stale release scope"));
    }
    let released = request(
        s,
        d,
        "editor.gameplay.control",
        mutation(&latest, 3, json!({"action":"release"})),
    )?;
    same(&ready, &released)?;
    if released["status"] != "queued" {
        return Err(failed("release was not acknowledged"));
    }
    loop {
        let v = describe(s, d, &o.intent)?;
        same(&ready, &v)?;
        if v["phase"] == "off" && v["cleanup_pending"] == false && v["error"] == false {
            break;
        }
        if v["error"] != false || !matches!(v["phase"].as_str(), Some("releasing" | "off")) {
            return Err(failed("release is unconfirmed"));
        }
        pause(d)?;
    }
    Ok(result)
}
pub fn run(ctx: &mut Ctx, args: &[String]) -> Result<(), CliErr> {
    let o = options(args)?;
    let d = Instant::now() + o.timeout;
    let transport = PumpedWs::connect(&ctx.url, remaining(d)?)
        .map_err(|_| CliErr::Conn("gameplay connection failed; no reconnect attempted".into()))?;
    let mut s = ErpClient::with_transport(Box::new(transport));
    if let Some(token) = ctx.token.as_deref() {
        let left = remaining(d)?;
        Session::call(&mut s, "auth", json!({"token":token}), left)?;
    }
    let result = act(&mut s, &o, d)?;
    ctx.emit("Gameplay action consumed, neutral input confirmed, control released. This is not a game-victory assertion.",&result);
    Ok(())
}
#[cfg(test)]
mod tests;
