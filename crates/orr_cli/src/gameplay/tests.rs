use super::*;
use std::collections::VecDeque;
struct Mock {
    replies: VecDeque<Result<J, CliErr>>,
    calls: Vec<(String, J)>,
}
impl Session for Mock {
    fn call(&mut self, m: &str, p: J, left: Duration) -> Result<J, CliErr> {
        assert!(left > Duration::ZERO);
        self.calls.push((m.into(), p));
        self.replies.pop_front().expect("unexpected extra call")
    }
}
fn base(phase: &str, scope: &str) -> J {
    json!({"binding":"1","connection":"2","scope":scope,"game":"fixture","build_id":"0x0000000000000003","intent":"act","mode":"play","playing":true,"error":false,"cleanup_pending":false,"phase":phase,"can_claim":phase=="off","can_submit":phase=="active","max_players":2})
}
fn complete() -> J {
    let mut v = base("active", "new");
    v["status"] = json!("completed");
    v["command_id"] = json!("7");
    v["consumed"] = json!({"tick":"10","stream_generation":"1","play_epoch":"2","timeline_epoch":"3","checksum":"0x0000000000000001","private":"must-not-emit"});
    v["neutral"] = json!({"tick":"11","stream_generation":"1","play_epoch":"2","timeline_epoch":"3","checksum":"0x0000000000000002"});
    v
}
fn values() -> Vec<J> {
    let mut claim = base("claiming", "claim");
    claim["status"] = json!("queued");
    let mut pending = base("active", "new");
    pending["status"] = json!("pending");
    pending["command_id"] = json!("7");
    let mut release = base("releasing", "release");
    release["status"] = json!("queued");
    vec![
        base("off", "old"),
        claim,
        base("claiming", "claim"),
        base("active", "new"),
        pending,
        complete(),
        base("active", "new"),
        release,
        base("off", "released"),
    ]
}
fn mock() -> Mock {
    Mock {
        replies: values().into_iter().map(Ok).collect(),
        calls: vec![],
    }
}
fn opts() -> Options {
    Options {
        intent: "act".into(),
        payload: json!({"press":1}),
        player: 0,
        timeout: Duration::from_secs(2),
    }
}
fn run(m: &mut Mock) -> Result<J, CliErr> {
    act(m, &opts(), Instant::now() + Duration::from_secs(2))
}
#[test]
fn gameplay_single_session_exact_mutations_fresh_fences_and_private_output() {
    let mut m = mock();
    let result = run(&mut m).unwrap();
    assert!(m.replies.is_empty());
    let writes: Vec<_> = m
        .calls
        .iter()
        .filter(|(method, _)| method.ends_with("control") || method.ends_with("submit"))
        .collect();
    assert_eq!(writes.len(), 3);
    for (i, (_, p)) in writes.iter().enumerate() {
        assert_eq!(p["request_key"], (i + 1).to_string());
    }
    assert_eq!(writes[0].1["scope"], "old");
    assert_eq!(writes[1].1["scope"], "new");
    assert_eq!(writes[2].1["scope"], "new");
    assert_eq!(writes[0].1["action"], "claim");
    assert_eq!(writes[2].1["action"], "release");
    for (method, p) in &m.calls {
        if method.ends_with("describe") || method.ends_with("status") {
            assert!(p.get("request_key").is_none());
            assert!(p.get("scope").is_none());
        }
    }
    let text = result.to_string();
    for secret in [
        "scope",
        "binding",
        "connection",
        "payload",
        "private",
        "must-not-emit",
    ] {
        assert!(!text.contains(secret));
    }
}
#[test]
fn gameplay_read_only_or_disconnected_stops_without_retry() {
    for index in [0, 1, 4, 5, 7] {
        for connection in [false, true] {
            let mut m = mock();
            m.replies[index] = Err(if connection {
                CliErr::Conn("fixture disconnect".into())
            } else {
                failed("request rejected")
            });
            assert!(run(&mut m).is_err());
            assert_eq!(m.calls.len(), index + 1);
        }
    }
}
#[test]
fn gameplay_not_running_or_unavailable_never_claims() {
    for (key, value) in [
        ("playing", json!(false)),
        ("mode", json!("edit")),
        ("can_claim", json!(false)),
        ("cleanup_pending", json!(true)),
        ("max_players", json!(0)),
    ] {
        let mut m = mock();
        m.replies[0].as_mut().unwrap()[key] = value;
        assert!(run(&mut m).is_err());
        assert_eq!(m.calls.len(), 1);
    }
}
#[test]
fn gameplay_stale_connection_or_scope_never_continues() {
    for (index, key, value) in [
        (3, "connection", "99"),
        (3, "binding", "99"),
        (3, "game", "other"),
        (5, "scope", "stale"),
        (5, "command_id", "8"),
        (6, "scope", "stale"),
    ] {
        let mut m = mock();
        m.replies[index].as_mut().unwrap()[key] = json!(value);
        assert!(run(&mut m).is_err());
        assert_eq!(m.calls.len(), index + 1);
    }
}
#[test]
fn gameplay_rejected_uncertain_or_invalid_receipt_is_not_success() {
    for status in ["rejected", "cancelled", "uncertain", "unknown"] {
        let mut m = mock();
        m.replies[5].as_mut().unwrap()["status"] = json!(status);
        assert!(run(&mut m).is_err());
        assert_eq!(m.calls.len(), 6);
    }
    for (key, value) in [
        ("tick", "10"),
        ("play_epoch", "99"),
        ("timeline_epoch", "bad"),
    ] {
        let mut m = mock();
        m.replies[5].as_mut().unwrap()["neutral"][key] = json!(value);
        assert!(run(&mut m).is_err());
        assert_eq!(m.calls.len(), 6);
    }
}
#[test]
fn gameplay_deadline_before_call_and_after_slow_reply_fails() {
    let mut m = mock();
    assert!(act(&mut m, &opts(), Instant::now()).is_err());
    assert!(m.calls.is_empty());
    struct Slow;
    impl Session for Slow {
        fn call(&mut self, _: &str, _: J, _: Duration) -> Result<J, CliErr> {
            std::thread::sleep(Duration::from_millis(65));
            Ok(base("off", "old"))
        }
    }
    assert!(act(
        &mut Slow,
        &opts(),
        Instant::now() + Duration::from_millis(60)
    )
    .is_err());
}
#[test]
fn gameplay_usage_does_not_echo_private_data() {
    for args in [
        vec!["act", "act", "--payload", "SECRET"],
        vec!["act", "act", "--player", "SECRET"],
        vec!["act", "act", "--timeout-ms", "SECRET"],
        vec!["act", "SECRET!"],
        vec!["act", "act", "--SECRET"],
    ] {
        let args = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
        let err = options(&args).err().expect("usage rejected");
        assert!(!format!("{err:?}").contains("SECRET"));
    }
    assert!(options(&["act".into(), "act".into()]).is_ok());
}

#[test]
fn gameplay_transient_claim_and_release_cleanup_are_read_only() {
    let mut m = mock();
    let mut active = base("active", "new");
    active["can_submit"] = json!(false);
    m.replies.insert(3, Ok(active));
    let mut cleanup = base("off", "released");
    cleanup["cleanup_pending"] = json!(true);
    m.replies.insert(9, Ok(cleanup));
    assert!(run(&mut m).is_ok());
    assert_eq!(
        m.calls
            .iter()
            .filter(|(m, _)| m.ends_with("control"))
            .count(),
        2
    );
    assert_eq!(
        m.calls
            .iter()
            .filter(|(m, _)| m.ends_with("submit"))
            .count(),
        1
    );
}
#[test]
fn gameplay_malformed_identity_and_checksum_fail_closed() {
    for key in ["game", "build_id", "intent"] {
        let mut m = mock();
        m.replies[3]
            .as_mut()
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove(key);
        assert!(run(&mut m).is_err());
        assert_eq!(m.calls.len(), 4);
    }
    let mut m = mock();
    m.replies[5].as_mut().unwrap()["neutral"]["checksum"] = json!("private");
    assert!(run(&mut m).is_err());
    assert_eq!(m.calls.len(), 6);
}

#[test]
fn gameplay_pending_status_deadline_never_resubmits_or_releases() {
    struct Pending {
        calls: Vec<(String, J)>,
    }
    impl Session for Pending {
        fn call(&mut self, method: &str, params: J, _: Duration) -> Result<J, CliErr> {
            self.calls.push((method.into(), params));
            let mut v = match self.calls.len() {
                1 => base("off", "old"),
                2 => {
                    let mut v = base("claiming", "claim");
                    v["status"] = json!("queued");
                    v
                }
                3 => base("active", "new"),
                _ => {
                    let mut v = base("active", "new");
                    v["status"] = json!("pending");
                    v["command_id"] = json!("7");
                    v
                }
            };
            // Keep private fixture data out of the error too.
            v["private"] = json!("SECRET");
            Ok(v)
        }
    }
    let mut s = Pending { calls: vec![] };
    let error = act(&mut s, &opts(), Instant::now() + Duration::from_millis(100)).unwrap_err();
    assert!(!format!("{error:?}").contains("SECRET"));
    assert_eq!(
        s.calls
            .iter()
            .filter(|(m, _)| m.ends_with("submit"))
            .count(),
        1
    );
    assert_eq!(
        s.calls
            .iter()
            .filter(|(m, _)| m.ends_with("control"))
            .count(),
        1
    );
    assert!(s.calls.iter().skip(4).all(|(m, _)| m.ends_with("status")));
}
