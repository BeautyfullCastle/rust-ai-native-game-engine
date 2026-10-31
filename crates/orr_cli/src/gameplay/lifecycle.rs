//! Existing editor lifecycle: one mutation, then read-only state observation.
use super::*;

pub(super) struct Options {
    action: Option<String>,
    intent: Option<String>,
    pub(super) timeout: Duration,
}
pub(super) fn options(args: &[String]) -> Result<Options, CliErr> {
    let p = parse(args, &[("--timeout-ms", true)], false)
        .map_err(|_| CliErr::Usage("invalid gameplay options; see `orr help gameplay`".into()))?;
    if p.all("--timeout-ms").len() > 1 {
        return Err(CliErr::Usage("duplicate gameplay option".into()));
    }
    let (action, intent) = match p.pos.first().map(String::as_str) {
        Some("describe") if p.pos.len() <= 2 => (None, p.pos.get(1).cloned()),
        Some("control")
            if p.pos.len() == 2
                && matches!(p.pos[1].as_str(), "play" | "pause" | "step" | "stop") =>
        {
            (Some(p.pos[1].clone()), None)
        }
        _ => {
            return Err(CliErr::Usage(
                "expected gameplay describe [intent] or control play|pause|step|stop".into(),
            ))
        }
    };
    if intent.as_ref().is_some_and(|s| {
        s.is_empty()
            || s.len() > 96
            || !s
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-:".contains(&b))
    }) {
        return Err(CliErr::Usage("invalid gameplay intent ID".into()));
    }
    let ms = p
        .get("--timeout-ms")
        .unwrap_or("10000")
        .parse::<u64>()
        .ok()
        .filter(|ms| (100..=60000).contains(ms))
        .ok_or_else(|| CliErr::Usage("timeout-ms must be 100..60000".into()))?;
    Ok(Options {
        action,
        intent,
        timeout: Duration::from_millis(ms),
    })
}
fn description(s: &mut impl Session, d: Instant, intent: Option<&str>) -> Result<J, CliErr> {
    let params = intent.map_or_else(|| json!({}), |id| json!({"intent":id}));
    let v = request(s, d, "editor.gameplay.describe", params)?;
    fences(&v)?;
    if v["game"]
        .as_str()
        .is_none_or(|s| s.is_empty() || s.len() > 256)
        || !checksum(&v["build_id"])
        || decimal(&v["tick"]).is_none()
        || !matches!(v["mode"].as_str(), Some("edit" | "play"))
        || !matches!(
            v["phase"].as_str(),
            Some("off" | "claiming" | "active" | "releasing")
        )
        || [
            "playing",
            "error",
            "cleanup_pending",
            "can_claim",
            "can_submit",
        ]
        .iter()
        .any(|k| !v[k].is_boolean())
        || v["max_players"].as_u64().is_none_or(|n| n == 0 || n > 16)
        || v["intents"]
            .as_array()
            .is_none_or(|ids| ids.iter().any(|id| id.as_str().is_none()))
        || intent.is_some_and(|id| v["intent"] != id || !v["payload_schema"].is_object())
    {
        return Err(failed("invalid gameplay description"));
    }
    Ok(v)
}
fn public_description(v: &J) -> J {
    let mut out = json!({});
    for key in [
        "game",
        "build_id",
        "mode",
        "playing",
        "tick",
        "phase",
        "error",
        "cleanup_pending",
        "can_claim",
        "can_submit",
        "max_players",
        "max_sequence",
        "intents",
        "intent",
        "payload_schema",
    ] {
        if let Some(value) = v.get(key) {
            out[key] = value.clone();
        }
    }
    out
}
pub(super) fn execute(s: &mut impl Session, o: &Options, d: Instant) -> Result<J, CliErr> {
    let before = description(s, d, o.intent.as_deref())?;
    let Some(action) = o.action.as_deref() else {
        return Ok(public_description(&before));
    };
    if before["error"] != false || before["cleanup_pending"] != false {
        return Err(failed("lifecycle is not ready"));
    }
    let target_tick = if action == "step" {
        if before["mode"] != "play" || before["playing"] != false {
            return Err(failed("step requires paused Play"));
        }
        Some(
            decimal(&before["tick"])
                .unwrap()
                .checked_add(1)
                .ok_or_else(|| failed("step tick exhausted"))?,
        )
    } else {
        None
    };
    let ack = request(
        s,
        d,
        "editor.gameplay.control",
        mutation(&before, 1, json!({"action":action})),
    )?;
    same(&before, &ack)?;
    if ack["status"] != "queued" {
        return Err(failed("control was not acknowledged"));
    }
    loop {
        let now = description(s, d, None)?;
        same(&before, &now)?;
        if now["error"] != false {
            return Err(failed("lifecycle error"));
        }
        let reached = match action {
            "play" => now["mode"] == "play" && now["playing"] == true,
            "pause" => now["mode"] == "play" && now["playing"] == false,
            "stop" => now["mode"] == "edit" && now["playing"] == false,
            "step" => {
                now["mode"] == "play"
                    && now["playing"] == false
                    && decimal(&now["tick"]).unwrap() >= target_tick.unwrap()
            }
            _ => unreachable!(),
        };
        if reached && now["cleanup_pending"] == false {
            return Ok(
                json!({"status":"observed","action":action,"state":public_description(&now),"confirmation":"target_state_not_exclusive_causal_receipt"}),
            );
        }
        pause(d)?;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Mock {
        replies: std::collections::VecDeque<Result<J, CliErr>>,
        calls: Vec<(String, J)>,
    }
    impl Session for Mock {
        fn call(&mut self, method: &str, params: J, _: Duration) -> Result<J, CliErr> {
            self.calls.push((method.into(), params));
            self.replies.pop_front().expect("unexpected request")
        }
    }

    fn state(mode: &str, playing: bool, tick: u64) -> J {
        json!({"binding":"1","connection":"2","scope":"current","game":"fixture","build_id":"0x0000000000000003","mode":mode,"playing":playing,"tick":tick.to_string(),"phase":"off","error":false,"cleanup_pending":false,"can_claim":true,"can_submit":false,"max_players":1,"max_sequence":"60","intents":["act"],"private":"do-not-emit"})
    }
    fn queued() -> J {
        json!({"binding":"1","connection":"2","scope":"queued","status":"queued"})
    }
    fn mock(replies: Vec<J>) -> Mock {
        Mock {
            replies: replies.into_iter().map(Ok).collect(),
            calls: vec![],
        }
    }
    fn options_for(action: &str) -> Options {
        options(&["control".into(), action.into()]).unwrap()
    }
    fn execute_control(m: &mut Mock, action: &str) -> Result<J, CliErr> {
        execute(
            m,
            &options_for(action),
            Instant::now() + Duration::from_secs(2),
        )
    }
    #[test]
    fn lifecycle_controls_observe_after_queue_and_only_mutate_once() {
        for (action, before, after) in [
            ("play", state("edit", false, 0), state("play", true, 0)),
            ("pause", state("play", true, 4), state("play", false, 4)),
            ("step", state("play", false, 4), state("play", false, 5)),
            ("stop", state("play", true, 9), state("edit", false, 0)),
        ] {
            let mut pending = before.clone();
            pending["cleanup_pending"] = json!(true);
            let mut m = mock(vec![before, queued(), pending, after]);
            let result = execute_control(&mut m, action).unwrap();
            assert_eq!(result["status"], "observed");
            assert_eq!(
                result["confirmation"],
                "target_state_not_exclusive_causal_receipt"
            );
            assert_eq!(m.calls.len(), 4);
            assert_eq!(m.calls[1].0, "editor.gameplay.control");
            assert_eq!(m.calls[1].1["request_key"], "1");
            assert_eq!(m.calls[1].1["action"], action);
            for (i, (method, params)) in m.calls.iter().enumerate() {
                if i != 1 {
                    assert_eq!(method, "editor.gameplay.describe");
                    assert!(params.get("scope").is_none());
                }
            }
            for key in ["private", "scope", "connection", "binding"] {
                assert!(result["state"].get(key).is_none());
            }
        }
    }
    #[test]
    fn lifecycle_describe_is_read_only_and_includes_requested_contract() {
        let mut value = state("edit", false, 0);
        value["intent"] = json!("act");
        value["payload_schema"] = json!({"type":"object"});
        let mut m = mock(vec![value]);
        let o = options(&["describe".into(), "act".into()]).unwrap();
        let result = execute(&mut m, &o, Instant::now() + o.timeout).unwrap();
        assert_eq!(result["intent"], "act");
        assert!(result["payload_schema"].is_object());
        assert_eq!(m.calls.len(), 1);
        assert_eq!(m.calls[0].1["intent"], "act");
        assert!(result.get("private").is_none());
    }
    #[test]
    fn lifecycle_stale_error_and_malformed_observations_fail_without_retry() {
        for key in ["binding", "connection", "game", "build_id", "error", "tick"] {
            let mut bad = state("play", true, 1);
            bad[key] = match key {
                "error" => json!(true),
                "tick" => json!("01"),
                "build_id" => json!("0x0000000000000004"),
                _ => json!("different"),
            };
            let mut m = mock(vec![state("edit", false, 0), queued(), bad]);
            assert!(execute_control(&mut m, "play").is_err());
            assert_eq!(m.calls.len(), 3);
        }
    }
    #[test]
    fn lifecycle_rejection_and_disconnect_end_after_one_mutation() {
        for fail_at in [1, 2] {
            let mut m = mock(vec![
                state("edit", false, 0),
                queued(),
                state("play", true, 1),
            ]);
            m.replies[fail_at] = Err(failed("fixture disconnect or rejection"));
            assert!(execute_control(&mut m, "play").is_err());
            assert_eq!(m.calls.len(), fail_at + 1);
        }
    }
    #[test]
    fn lifecycle_step_preconditions_and_expired_deadline_never_mutate() {
        for before in [
            state("edit", false, 0),
            state("play", true, 0),
            state("play", false, u64::MAX),
        ] {
            let mut m = mock(vec![before]);
            assert!(execute_control(&mut m, "step").is_err());
            assert_eq!(m.calls.len(), 1);
        }
        let mut m = mock(vec![]);
        assert!(execute(&mut m, &options_for("play"), Instant::now()).is_err());
        assert!(m.calls.is_empty());
    }
    #[test]
    fn lifecycle_queued_but_never_reached_times_out_without_second_mutation() {
        struct Waiting {
            mutations: usize,
        }
        impl Session for Waiting {
            fn call(&mut self, method: &str, _: J, _: Duration) -> Result<J, CliErr> {
                if method.ends_with("control") {
                    self.mutations += 1;
                    Ok(queued())
                } else {
                    Ok(state("edit", false, 0))
                }
            }
        }
        let mut waiting = Waiting { mutations: 0 };
        assert!(execute(
            &mut waiting,
            &options_for("play"),
            Instant::now() + Duration::from_millis(100)
        )
        .is_err());
        assert_eq!(waiting.mutations, 1);
    }
    #[test]
    fn lifecycle_describe_without_intent_lists_contract_without_mutation() {
        let mut m = mock(vec![state("edit", false, 0)]);
        let o = options(&["describe".into()]).unwrap();
        let result = execute(&mut m, &o, Instant::now() + o.timeout).unwrap();
        assert_eq!(result["intents"], json!(["act"]));
        assert_eq!(m.calls.len(), 1);
        assert!(m.calls[0].1.get("intent").is_none());
        assert!(result.get("scope").is_none());
    }
    #[test]
    fn gameplay_dispatch_accepts_leading_flags_and_ack_must_be_queued() {
        assert!(is_act(&[
            "--timeout-ms".into(),
            "10000".into(),
            "act".into(),
            "act".into()
        ])
        .unwrap());
        assert!(!is_act(&["--timeout-ms".into(), "10000".into(), "describe".into()]).unwrap());
        for status in ["completed", "observed", "uncertain"] {
            let mut ack = queued();
            ack["status"] = json!(status);
            let mut m = mock(vec![state("edit", false, 0), ack]);
            assert!(execute_control(&mut m, "play").is_err());
            assert_eq!(m.calls.len(), 2);
        }
    }
    #[test]
    fn lifecycle_options_reject_wrong_actions_extras_and_secret_echo() {
        for args in [
            vec!["control", "claim"],
            vec!["control", "step", "2"],
            vec!["describe", "act", "extra"],
            vec!["describe", "--payload", "secret"],
            vec!["describe", "--timeout-ms", "1"],
        ] {
            assert!(options(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).is_err());
        }
    }
}
