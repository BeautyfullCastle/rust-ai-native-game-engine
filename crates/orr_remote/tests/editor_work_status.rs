#![allow(clippy::disallowed_types)]
//! Real-host contract checks. Synthetic owner publications deliberately isolate
//! the ERP boundary; these are not GUI/frame-publication acceptance tests.

mod common;

use common::demo_doc;
use orr_remote::{Auth, Cap, Caps, ErpClient, ErpServer, LocalHost, ServerConfig};
use orr_sample::physics_game::PhysGame;
use serde_json::{json, Value as J};

const METHOD: &str = "editor.work_status";

fn local_client(host: &LocalHost, name: &str, caps: Caps) -> ErpClient {
    let mut client =
        ErpClient::with_transport(Box::new(host.connector().connect(name, caps).unwrap()));
    client.call_timeout = std::time::Duration::from_secs(5);
    client
}

fn legacy_host() -> LocalHost {
    LocalHost::spawn::<PhysGame>(|| {
        let mut cfg = ServerConfig::new(Auth::DevNoAuth);
        cfg.listen = false;
        Ok((demo_doc(), cfg))
    })
    .unwrap()
}

#[test]
fn editor_work_status_preserves_exhaustive_config_and_old_entrypoints() {
    // No `..default`: adding any public field breaks this downstream literal.
    let defaults = ServerConfig::new(Auth::DevNoAuth);
    let cfg = ServerConfig {
        bind: defaults.bind,
        auth: defaults.auth,
        max_message_bytes: defaults.max_message_bytes,
        allowed_origins: defaults.allowed_origins,
        max_connections: defaults.max_connections,
        max_queued_requests: defaults.max_queued_requests,
        max_queued_request_bytes: defaults.max_queued_request_bytes,
        max_requests_per_poll: defaults.max_requests_per_poll,
        tx_timeout: defaults.tx_timeout,
        limits: defaults.limits,
        activity_capacity: defaults.activity_capacity,
        listen: false,
        screenshot: defaults.screenshot,
    };
    let _: fn(ServerConfig) -> Result<ErpServer, orr_remote::ServerError> = ErpServer::start;
    let server = ErpServer::start(cfg.clone()).unwrap();
    assert!(!server.is_listening());
    drop(server);

    let host = LocalHost::spawn::<PhysGame>(move || Ok((demo_doc(), cfg))).unwrap();
    let mut reader = local_client(&host, "reader", Caps::of(&[Cap::Read]));
    let result = reader.call(METHOD, json!({})).unwrap();
    assert_eq!(result["availability"], "not_paired");
    assert_eq!(result["assessment"]["state"], "indeterminate");
    assert_eq!(result["host_scene"]["owner"], "host");
    assert!(result["observation"]["snapshot_seq"].is_null());
    let discovery = reader.call("rpc.discover", json!({})).unwrap();
    assert!(discovery["engine"].get("editor_work_status").is_none());

    #[cfg(feature = "sample-host")]
    {
        type Factory =
            fn(String, Option<std::path::PathBuf>, ServerConfig) -> Result<LocalHost, String>;
        let _: Factory = orr_remote::sample::spawn_phys_host;
        let _: Factory = orr_remote::sample::spawn_arena_host;
        #[cfg(feature = "collect-dodge")]
        let _: Factory = orr_remote::collect_dodge::spawn_host;
    }
}

#[test]
fn editor_work_status_denies_nonread_caps_before_params_or_unpaired_state() {
    let host = legacy_host();
    for caps in [
        Caps::NONE,
        Caps::of(&[Cap::SceneEdit]),
        Caps::of(&[Cap::Approve]),
        Caps::of(&[Cap::SimControl]),
        Caps::of(&[Cap::SceneEdit, Cap::Approve, Cap::SimControl]),
    ] {
        let mut client = local_client(&host, "not-a-reader", caps);
        let valid = client.call_err(METHOD, json!({}));
        let invalid = client.call_err(METHOD, json!({"path":"NO_READ_SECRET_PATH","force":true}));
        assert_eq!(valid.code, orr_remote::PERMISSION_DENIED);
        assert_eq!(valid.kind(), Some("permission_denied"));
        assert_eq!(
            valid, invalid,
            "authorization must precede argument inspection"
        );
        assert_eq!(valid, client.call_err(METHOD, json!([])));
        let serialized = valid.to_json().to_string();
        for hidden in [
            "NO_READ_SECRET_PATH",
            "not_paired",
            "owner_binding_id",
            "domains",
        ] {
            assert!(!serialized.contains(hidden), "{serialized}");
        }
        let discovery = client.call("rpc.discover", json!({})).unwrap();
        let method = discovery["methods"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == METHOD)
            .unwrap();
        assert_eq!(method["capability"], "read");
        assert_eq!(method["allowed"], false);
        assert!(discovery["engine"].get("editor_work_status").is_none());
    }
}

#[cfg(feature = "sample-host")]
mod paired {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use common::{demo_text, guid_of, token};
    use orr_remote::sample::{
        spawn_arena_host, spawn_arena_host_with_editor_work_status, spawn_phys_host,
        spawn_phys_host_with_editor_work_status,
    };
    use orr_remote::work_status::{
        DomainId, DomainRecord, EditorSnapshot, Operations, WorkStatusOwner, WorkStatusService,
    };

    const SECRET: &str = "WORK_STATUS_SECRET_SENTINEL";
    const BODY: &str = "orr_physics::Body";
    const ARENA: &str = "schema: orr.scene/1\nsingletons:\n  Score: { kills: [0,0,0,0,0,0,0,0] }\nentities:\n  e_00000001:\n    name: hero\n    Position: { pos: [-300,0] }\n    PlayerTag: { slot: 0 }\n  e_00000002:\n    name: target\n    Position: { pos: [300,0] }\n    PlayerTag: { slot: 1 }\n";

    fn config(listen: bool) -> ServerConfig {
        let mut cfg = ServerConfig::new(Auth::Tokens(vec![
            token("reader", SECRET, "read"),
            token("writer", "work-status-writer", "all"),
            token(
                "nonreader",
                "work-status-no-read",
                "scene_edit,approve,sim_control",
            ),
        ]));
        cfg.listen = listen;
        cfg
    }

    fn host(listen: bool) -> (LocalHost, WorkStatusOwner) {
        let (service, owner) = WorkStatusService::pair();
        let host =
            spawn_phys_host_with_editor_work_status(demo_text(), None, config(listen), service)
                .unwrap();
        (host, owner)
    }

    fn snapshot(operations: Operations) -> EditorSnapshot {
        EditorSnapshot {
            domains: DomainId::ALL
                .iter()
                .copied()
                .map(|id| DomainRecord::observed(id, false))
                .collect(),
            operations,
        }
    }

    fn publish(owner: &mut WorkStatusOwner, value: EditorSnapshot) {
        assert!(
            owner.publish(value),
            "uncontended owner publication must succeed"
        );
    }

    fn status(client: &mut ErpClient) -> J {
        client.call(METHOD, json!({"max_age_ms":5000})).unwrap()
    }

    fn decimal(value: &J) -> u64 {
        let text = value
            .as_str()
            .expect("identity/counter is a decimal string");
        assert!(!text.is_empty() && text.bytes().all(|c| c.is_ascii_digit()));
        text.parse().unwrap()
    }

    fn normalized(mut value: J) -> J {
        value["observation"]["age_ms"] = J::Null;
        value
    }

    fn no_secret(value: &J) {
        let text = value.to_string();
        assert!(!text.contains(SECRET), "private text escaped: {text}");
        for forbidden in [
            "scene_path",
            "tx_owner",
            "safe_to_close",
            "last_close_attempt",
        ] {
            assert!(
                !text.contains(forbidden),
                "{forbidden} is outside the v1 response"
            );
        }
        assert!(text.len() <= 16 * 1024, "status must remain bounded");
    }

    #[test]
    fn editor_work_status_owner_lifecycle_and_discovery_are_not_permission() {
        let legacy = spawn_phys_host(demo_text(), None, config(false)).unwrap();
        let mut legacy_reader = local_client(&legacy, "reader", Caps::of(&[Cap::Read]));
        assert_eq!(status(&mut legacy_reader)["availability"], "not_paired");

        let (host, mut owner) = host(false);
        let mut reader = local_client(&host, "reader", Caps::of(&[Cap::Read]));
        let before = status(&mut reader);
        assert_eq!(before["availability"], "not_ready");
        assert_eq!(before["assessment"]["state"], "indeterminate");
        assert!(before["observation"]["snapshot_seq"].is_null());
        let discovery = reader.call("rpc.discover", json!({})).unwrap();
        assert_eq!(
            discovery["engine"]["editor_work_status"],
            json!({
                "supported":true,"availability":"not_ready",
                "default_max_age_ms":1000,"max_age_ms_limit":5000,
            })
        );

        publish(&mut owner, snapshot(Operations::Idle));
        let ready = status(&mut reader);
        assert_eq!(ready["availability"], "ready");
        assert_eq!(ready["assessment"]["state"], "none_observed");
        let binding = decimal(&ready["observation"]["owner_binding_id"]);
        assert!(binding > 0);
        assert!(decimal(&ready["observation"]["snapshot_seq"]) > 0);
        assert!(decimal(&ready["observation"]["ui_frame"]) > 0);
        assert_eq!(ready["host_scene"]["owner"], "paired_host");
        assert_eq!(
            reader.call("rpc.discover", json!({})).unwrap()["engine"]["editor_work_status"]
                ["availability"],
            "ready"
        );

        let mut denied = local_client(&host, "nonreader", Caps::of(&[Cap::SceneEdit]));
        assert_eq!(
            denied.call_err(METHOD, json!({})).kind(),
            Some("permission_denied")
        );
        assert!(denied.call("rpc.discover", json!({})).unwrap()["engine"]
            .get("editor_work_status")
            .is_none());
        drop(owner);
        let absent = status(&mut reader);
        assert_eq!(absent["availability"], "owner_absent");
        assert_eq!(absent["assessment"]["state"], "indeterminate");
        assert_eq!(
            reader.call("rpc.discover", json!({})).unwrap()["engine"]["editor_work_status"]
                ["availability"],
            "owner_absent"
        );
        assert!(absent["observation"]["snapshot_seq"].is_null());
        assert_eq!(absent["editor_local"]["domains"], json!([]));
        assert!(host.is_running());

        let (replacement, mut replacement_owner) = self::host(false);
        publish(&mut replacement_owner, snapshot(Operations::Idle));
        let mut next_reader = local_client(&replacement, "reader", Caps::of(&[Cap::Read]));
        let next = status(&mut next_reader);
        assert_ne!(decimal(&next["observation"]["owner_binding_id"]), binding);
        assert_eq!(status(&mut reader)["availability"], "owner_absent");
    }

    #[test]
    fn editor_work_status_finite_inventory_incomplete_coverage_and_operations() {
        let (host, mut owner) = host(false);
        let mut reader = local_client(&host, "reader", Caps::of(&[Cap::Read]));
        let mut input = snapshot(Operations::Idle);
        input.domains[0] = DomainRecord::disabled(DomainId::ALL[0]);
        input.domains[1] = DomainRecord::not_loaded(DomainId::ALL[1]);
        publish(&mut owner, input);
        let clean = status(&mut reader);
        assert_eq!(clean["schema_version"], 1);
        assert_eq!(clean["scope"], "known_main_editor_authoring_domains_v1");
        assert_eq!(clean["target"], "host_paired_local_editor");
        assert_eq!(
            clean["consistency"],
            "host_at_request_local_at_published_frame"
        );
        assert_eq!(clean["assessment"]["state"], "none_observed");
        let rows = clean["editor_local"]["domains"].as_array().unwrap();
        let actual: Vec<_> = rows
            .iter()
            .map(|row| row["domain_id"].as_str().unwrap())
            .collect();
        assert_eq!(
            actual,
            [
                "sprite_bindings",
                "tilemap",
                "collect_ui_document",
                "collect_ui_preview",
                "model_bindings",
                "animation_bindings",
                "terrain",
                "irradiance_bindings",
                "room_camera",
                "mesh_geometry_art",
                "mesh_camera",
                "mesh_lighting",
                "mesh_hud",
                "mesh_mission",
                "mesh_hazards",
                "mesh_animation",
            ]
        );
        assert_eq!(rows[0]["presence"], "feature_disabled");
        assert_eq!(rows[1]["presence"], "not_loaded");
        assert_eq!(rows[0]["work_state"], "not_applicable");
        assert_eq!(rows[1]["work_state"], "not_applicable");
        assert_eq!(rows[2]["presence"], "loaded");
        assert_eq!(rows[2]["work_state"], "clean");
        for row in rows {
            assert_eq!(row["owner"], "editor_local");
            assert!(row["domain_revision"].is_null());
            assert!(row["persistence_scope"].is_string());
        }
        no_secret(&clean);

        let mut incomplete = snapshot(Operations::Idle);
        incomplete.domains[1] = DomainRecord::unknown(DomainId::ALL[1]);
        publish(&mut owner, incomplete);
        assert_eq!(status(&mut reader)["assessment"]["state"], "indeterminate");
        let mut blocked = snapshot(Operations::Idle);
        blocked.domains[0] = DomainRecord::observed(DomainId::ALL[0], true);
        blocked.domains[1] = DomainRecord::unknown(DomainId::ALL[1]);
        publish(&mut owner, blocked);
        let result = status(&mut reader);
        assert_eq!(result["editor_local"]["coverage"], "incomplete");
        assert_eq!(result["assessment"]["state"], "known_blockers");
        assert_eq!(result["assessment"]["blockers"], json!(["sprite_bindings"]));
        assert!(!result["assessment"]["reasons"]
            .as_array()
            .unwrap()
            .is_empty());

        for (operations, expected) in [
            (Operations::PendingMutation, "pending_mutation"),
            (Operations::OutcomeUncertain, "outcome_uncertain"),
            (Operations::Unknown, "unknown"),
        ] {
            publish(&mut owner, snapshot(operations));
            let result = status(&mut reader);
            assert_eq!(result["editor_operations"]["state"], expected);
            assert!(result["editor_operations"]["reason"].is_string());
            assert_eq!(result["assessment"]["state"], "indeterminate");
        }
        let mut missing = snapshot(Operations::Idle);
        missing.domains.pop();
        publish(&mut owner, missing);
        assert_eq!(status(&mut reader)["assessment"]["state"], "indeterminate");
        let mut overflowing = snapshot(Operations::Idle);
        overflowing.domains = (0..33)
            .map(|_| DomainRecord::disabled(DomainId::ALL[0]))
            .collect();
        publish(&mut owner, overflowing);
        let result = status(&mut reader);
        assert_eq!(result["availability"], "incomplete");
        assert_eq!(result["assessment"]["state"], "indeterminate");
        no_secret(&result);
    }

    #[test]
    fn editor_work_status_reads_never_refresh_clean_or_dirty_snapshot_age() {
        let (host, mut owner) = host(false);
        let mut reader = local_client(&host, "reader", Caps::of(&[Cap::Read]));
        let mut previous_sequence = 0;
        for dirty in [false, true] {
            let mut input = snapshot(Operations::Idle);
            input.domains[0] = DomainRecord::observed(DomainId::ALL[0], dirty);
            publish(&mut owner, input);
            let fresh = status(&mut reader);
            assert_eq!(fresh["availability"], "ready");
            let sequence = decimal(&fresh["observation"]["snapshot_seq"]);
            assert!(sequence > previous_sequence);
            previous_sequence = sequence;
            std::thread::sleep(Duration::from_millis(10));
            let mut age = 0;
            for _ in 0..8 {
                let stale = reader.call(METHOD, json!({"max_age_ms":1})).unwrap();
                assert_eq!(stale["availability"], "stale");
                assert_eq!(stale["assessment"]["state"], "indeterminate");
                assert_eq!(
                    stale["observation"]["snapshot_seq"],
                    fresh["observation"]["snapshot_seq"]
                );
                assert_eq!(
                    stale["observation"]["ui_frame"],
                    fresh["observation"]["ui_frame"]
                );
                let next_age = stale["observation"]["age_ms"].as_u64().unwrap();
                assert!(next_age >= age);
                age = next_age;
            }
            assert!(age >= 1);
        }
    }

    struct SceneFile(PathBuf);

    impl SceneFile {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "{SECRET}-{}-{}.scene.yaml",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::write(&path, demo_text()).unwrap();
            Self(path)
        }
    }

    impl Drop for SceneFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn editor_work_status_host_is_authoritative_across_edit_play_save_and_rollback() {
        let file = SceneFile::new();
        let (service, mut owner) = WorkStatusService::pair();
        let host = spawn_phys_host_with_editor_work_status(
            demo_text(),
            Some(file.0.clone()),
            config(false),
            service,
        )
        .unwrap();
        let mut writer = local_client(&host, "writer", Caps::ALL);
        let mut reader = local_client(&host, "reader", Caps::of(&[Cap::Read]));
        publish(&mut owner, snapshot(Operations::Idle));
        let initial = status(&mut reader);
        let guid = guid_of(&mut writer, "body_05");
        writer
            .call(
                "world.patch",
                json!({"entity":guid,"component":BODY,"path":"pos.x","value":"1.25"}),
            )
            .unwrap();
        let edited = status(&mut reader);
        assert_eq!(edited["host_scene"]["authority"], "erp_host");
        assert_eq!(edited["host_scene"]["dirty"], true);
        assert_eq!(edited["assessment"]["state"], "known_blockers");
        assert_ne!(
            edited["host_scene"]["doc_checksum"],
            initial["host_scene"]["doc_checksum"]
        );
        assert_eq!(
            edited["observation"]["snapshot_seq"],
            initial["observation"]["snapshot_seq"]
        );
        assert_eq!(
            edited["host_scene"]["doc_checksum"],
            writer.call("sim.state", json!({})).unwrap()["doc_checksum"]
        );
        writer.call("scene.save", json!({"write":true})).unwrap();
        let saved = status(&mut reader);
        assert_eq!(saved["host_scene"]["dirty"], false);
        assert_eq!(
            saved["host_scene"]["preview_revision"], edited["host_scene"]["preview_revision"],
            "a save is not a new preview revision"
        );
        decimal(&saved["host_scene"]["preview_revision"]);
        assert_eq!(saved["assessment"]["state"], "none_observed");

        writer.call("tx.begin", json!({"label":SECRET})).unwrap();
        writer
            .call(
                "world.patch",
                json!({"entity":guid,"component":BODY,"path":"pos.x","value":"2.5"}),
            )
            .unwrap();
        let staged = status(&mut reader);
        assert_eq!(staged["host_scene"]["in_tx"], true);
        assert_eq!(staged["assessment"]["state"], "known_blockers");
        no_secret(&staged);
        writer.call("tx.rollback", json!({})).unwrap();
        let rolled_back = status(&mut reader);
        assert_eq!(rolled_back["host_scene"]["dirty"], false);
        assert_eq!(rolled_back["host_scene"]["in_tx"], false);
        assert_eq!(
            rolled_back["host_scene"]["doc_checksum"],
            saved["host_scene"]["doc_checksum"]
        );
        writer.call("sim.start", json!({})).unwrap();
        writer.call("sim.step", json!({"n":10})).unwrap();
        let play = writer.call("sim.state", json!({})).unwrap();
        assert_eq!(play["mode"], "play");
        assert_ne!(play["checksum"], play["doc_checksum"]);
        let during_play = status(&mut reader);
        assert_eq!(
            during_play["host_scene"]["doc_checksum"],
            play["doc_checksum"]
        );
        assert_eq!(during_play["host_scene"], rolled_back["host_scene"]);
        assert_eq!(during_play["assessment"]["state"], "none_observed");
        writer.call("sim.stop", json!({})).unwrap();
    }

    #[test]
    fn editor_work_status_duplicate_reads_preserve_drafts_history_disk_and_tx_owner() {
        let file = SceneFile::new();
        let (service, mut owner) = WorkStatusService::pair();
        let host = spawn_phys_host_with_editor_work_status(
            demo_text(),
            Some(file.0.clone()),
            config(false),
            service,
        )
        .unwrap();
        let mut writer = local_client(&host, "writer", Caps::ALL);
        let mut other = local_client(&host, "other", Caps::ALL);
        let guid = guid_of(&mut writer, "body_05");
        writer
            .call("world.rename", json!({"entity":guid,"name":SECRET}))
            .unwrap();
        writer
            .call("proposal.begin", json!({"label":SECRET}))
            .unwrap();
        writer.call("tx.begin", json!({"label":SECRET})).unwrap();
        writer
            .call(
                "world.patch",
                json!({"entity":guid,"component":BODY,"path":"pos.x","value":"9.5"}),
            )
            .unwrap();
        let mut input = snapshot(Operations::Idle);
        input.domains[0] = DomainRecord::observed(DomainId::ALL[0], true);
        publish(&mut owner, input);
        let before_history = writer.call("history.list", json!({})).unwrap();
        let before_scene = writer.call("scene.save", json!({})).unwrap();
        let before_proposals = writer.call("proposal.list", json!({})).unwrap();
        let disk = std::fs::read(&file.0).unwrap();
        let before_status = status(&mut writer);
        for _ in 0..16 {
            let result = status(&mut writer);
            no_secret(&result);
            assert_eq!(normalized(result), normalized(before_status.clone()));
        }
        assert_eq!(
            writer.call("history.list", json!({})).unwrap(),
            before_history
        );
        assert_eq!(writer.call("scene.save", json!({})).unwrap(), before_scene);
        assert_eq!(
            writer.call("proposal.list", json!({})).unwrap(),
            before_proposals
        );
        assert_eq!(std::fs::read(&file.0).unwrap(), disk);
        assert_eq!(
            other.call_err("tx.commit", json!({})).kind(),
            Some("tx_busy")
        );
        writer.call("tx.rollback", json!({})).unwrap();
    }

    fn tcp_call(writer: &mut TcpStream, reader: &mut BufReader<TcpStream>, value: J) -> J {
        writeln!(writer, "{value}").unwrap();
        let mut line = String::new();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        serde_json::from_str(&line).unwrap()
    }

    #[test]
    fn editor_work_status_authenticated_ws_tcp_and_local_have_the_same_contract() {
        let (host, mut owner) = host(true);
        publish(&mut owner, snapshot(Operations::Idle));
        let mut local = local_client(&host, "reader", Caps::of(&[Cap::Read]));
        let mut ws = ErpClient::connect(host.url().unwrap(), Some(SECRET)).unwrap();
        ws.call_timeout = Duration::from_secs(5);
        let expected = normalized(status(&mut local));
        assert_eq!(normalized(status(&mut ws)), expected);
        let stream =
            TcpStream::connect(host.url().unwrap().strip_prefix("ws://").unwrap()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        let before_auth = tcp_call(
            &mut writer,
            &mut reader,
            json!({"jsonrpc":"2.0","id":"before-auth","method":METHOD,"params":{}}),
        );
        assert_eq!(before_auth["error"]["code"], orr_remote::UNAUTHENTICATED);
        let authenticated = tcp_call(
            &mut writer,
            &mut reader,
            json!({"jsonrpc":"2.0","id":"authenticate","method":"auth","params":{"token":SECRET}}),
        );
        assert_eq!(authenticated["result"]["client"], "reader");
        let response = tcp_call(
            &mut writer,
            &mut reader,
            json!({"jsonrpc":"2.0","id":"status-string-id","method":METHOD,"params":{"max_age_ms":5000}}),
        );
        assert_eq!(response["id"], "status-string-id");
        assert!(response.get("error").is_none(), "{response}");
        assert_eq!(normalized(response["result"].clone()), expected);
        no_secret(&response);

        let mut denied =
            ErpClient::connect(host.url().unwrap(), Some("work-status-no-read")).unwrap();
        denied.call_timeout = Duration::from_secs(5);
        assert_eq!(
            denied.call_err(METHOD, json!({"max_age_ms":0})).kind(),
            Some("permission_denied")
        );
        assert!(denied.call("rpc.discover", json!({})).unwrap()["engine"]
            .get("editor_work_status")
            .is_none());
    }

    #[test]
    fn editor_work_status_rejects_nonobject_extra_or_invalid_age_without_echoing_secrets() {
        let (host, mut owner) = host(false);
        let mut reader = local_client(&host, "reader", Caps::of(&[Cap::Read]));
        publish(&mut owner, snapshot(Operations::Idle));
        for invalid in [
            J::Null,
            json!([]),
            json!(true),
            json!(17),
            json!(SECRET),
            json!({"max_age_ms":null}),
            json!({"max_age_ms":false}),
            json!({"max_age_ms":"1000"}),
            json!({"max_age_ms":1.5}),
            json!({"max_age_ms":0}),
            json!({"max_age_ms":5001}),
            json!({"max_age_ms":-1}),
            json!({"max_age_ms":u64::MAX}),
            json!({"path":SECRET}),
            json!({"url":format!("https://{SECRET}@example.invalid/?key={SECRET}#{SECRET}")}),
            json!({"target":SECRET}),
            json!({"force":true}),
            json!({"save":true}),
            json!({"refresh":true}),
            json!({"wait":true}),
            json!({"close":true}),
            json!({SECRET:{"nested":SECRET}}),
        ] {
            let error = reader.call_err(METHOD, invalid);
            assert_eq!(error.code, orr_remote::INVALID_PARAMS);
            assert_eq!(error.kind(), Some("invalid_params"));
            assert_eq!(
                error.message,
                "editor.work_status expects only optional max_age_ms, an integer from 1 to 5000"
            );
            assert_eq!(error.data, Some(json!({"kind":"invalid_params"})));
            no_secret(&error.to_json());
        }
        let default_age = reader.call(METHOD, json!({})).unwrap();
        assert_eq!(default_age["observation"]["max_age_ms"], 1000);
        for age in [1, 5000] {
            assert_eq!(
                reader.call(METHOD, json!({"max_age_ms":age})).unwrap()["observation"]
                    ["max_age_ms"],
                age
            );
        }
        let activity = reader
            .call("activity.list", json!({"include_reads":true}))
            .unwrap();
        assert!(
            !activity.to_string().contains(SECRET),
            "status input must not enter activity"
        );

        let (service, _owner) = WorkStatusService::pair();
        let mut cfg = config(true);
        cfg.max_queued_requests = 0;
        let saturated =
            spawn_phys_host_with_editor_work_status(demo_text(), None, cfg, service).unwrap();
        let mut client = ErpClient::connect(saturated.url().unwrap(), Some(SECRET)).unwrap();
        client.call_timeout = Duration::from_secs(5);
        let error = client.call_err(METHOD, json!({}));
        assert_eq!(error.code, orr_remote::LIMIT_EXCEEDED);
        assert_eq!(error.kind(), Some("busy"));
        assert!(saturated.is_running());
    }

    #[test]
    fn editor_work_status_does_not_acquire_or_wait_for_screenshot_capture() {
        let (service, mut owner) = WorkStatusService::pair();
        let (screenshot, screenshot_owner) = orr_remote::ScreenshotService::pair();
        let mut cfg = config(false);
        cfg.screenshot = Some(screenshot);
        let host =
            spawn_phys_host_with_editor_work_status(demo_text(), None, cfg, service).unwrap();
        let mut capture = local_client(&host, "capture", Caps::of(&[Cap::Read]));
        let mut reader = local_client(&host, "reader", Caps::of(&[Cap::Read]));
        publish(&mut owner, snapshot(Operations::Idle));
        assert_eq!(status(&mut reader)["availability"], "ready");
        assert_eq!(screenshot_owner.take_request(), None);
        let _capture_id = capture
            .post("view.screenshot", json!({"timeout_ms":5000}))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let request = loop {
            if let Some(request) = screenshot_owner.take_request() {
                break request;
            }
            assert!(
                Instant::now() < deadline,
                "capture request was not admitted"
            );
            std::thread::sleep(Duration::from_millis(1));
        };
        screenshot_owner.begin_capture(request.serial).unwrap();
        for _ in 0..8 {
            assert_eq!(status(&mut reader)["availability"], "ready");
            assert!(screenshot_owner.is_active(request.serial));
        }
        assert_eq!(screenshot_owner.take_request(), None);
        assert_eq!(reader.call("sim.state", json!({})).unwrap()["mode"], "edit");
    }

    fn input_startup(client: &mut ErpClient, title: &str, input: J) -> J {
        let schema = client.call("registry.input", json!({})).unwrap();
        assert_eq!(schema["schema"]["title"], title);
        assert_eq!(schema["managed_held"]["version"], 1);
        let discover = client.call("rpc.discover", json!({})).unwrap();
        assert_eq!(discover["engine"]["input"], schema);
        client.call("sim.start", json!({})).unwrap();
        let grant = client
            .call("sim.input_claim", json!({"player":0,"replace_held":true}))
            .unwrap();
        client
            .call(
                "sim.input_value",
                json!({
                    "player":0,"grant":grant["grant"],"generation":grant["generation"],
                    "sequence":"1","value":input
                }),
            )
            .unwrap();
        client.call("sim.step", json!({"n":1})).unwrap();
        let held = client.call("sim.state", json!({})).unwrap();
        assert_eq!(held["managed_held"]["slots"][0]["owned_by_you"], true);
        client.call("sim.input_release", json!({
            "player":0,"grant":grant["grant"],"generation":grant["generation"],"sequence":"2"
        })).unwrap();
        client.call("sim.stop", json!({})).unwrap();
        schema
    }

    #[test]
    fn editor_work_status_arena_sibling_preserves_first_ready_input_setup() {
        let old = spawn_arena_host(ARENA.into(), None, config(false)).unwrap();
        let (service, mut owner) = WorkStatusService::pair();
        let new =
            spawn_arena_host_with_editor_work_status(ARENA.into(), None, config(false), service)
                .unwrap();
        let mut old_client = local_client(&old, "writer", Caps::ALL);
        let mut new_client = local_client(&new, "writer", Caps::ALL);
        assert_eq!(status(&mut old_client)["availability"], "not_paired");
        assert_eq!(status(&mut new_client)["availability"], "not_ready");
        assert_eq!(
            old_client.call("sim.checksum", json!({})).unwrap(),
            new_client.call("sim.checksum", json!({})).unwrap()
        );
        let input = json!({"axis_x":1,"axis_y":0,"buttons":["fire"]});
        let old_schema = input_startup(&mut old_client, "ArenaInput", input.clone());
        let new_schema = input_startup(&mut new_client, "ArenaInput", input);
        assert_eq!(old_schema, new_schema);
        publish(&mut owner, snapshot(Operations::Idle));
        assert_eq!(
            status(&mut new_client)["assessment"]["state"],
            "none_observed"
        );
        let (service, _owner) = WorkStatusService::pair();
        assert!(
            spawn_arena_host_with_editor_work_status(
                "invalid scene".into(),
                None,
                config(false),
                service,
            )
            .is_err(),
            "failed scene setup must never expose a ready host"
        );
    }

    #[cfg(feature = "collect-dodge")]
    #[test]
    fn editor_work_status_collect_sibling_preserves_first_ready_input_setup_and_admission() {
        use orr_remote::collect_dodge::{spawn_host, spawn_host_with_editor_work_status};
        const SOURCE: &str = include_str!("../../../scenes/collect_dodge_v1.scene.yaml");
        let old = spawn_host(SOURCE.into(), None, config(false)).unwrap();
        let (service, mut owner) = WorkStatusService::pair();
        let new = spawn_host_with_editor_work_status(SOURCE.into(), None, config(false), service)
            .unwrap();
        let mut old_client = local_client(&old, "writer", Caps::ALL);
        let mut new_client = local_client(&new, "writer", Caps::ALL);
        assert_eq!(status(&mut old_client)["availability"], "not_paired");
        assert_eq!(status(&mut new_client)["availability"], "not_ready");
        assert_eq!(
            old_client.call("sim.checksum", json!({})).unwrap(),
            new_client.call("sim.checksum", json!({})).unwrap()
        );
        let input = json!({"x":1,"y":0,"buttons":[]});
        assert_eq!(
            input_startup(&mut old_client, "CollectInput", input.clone()),
            input_startup(&mut new_client, "CollectInput", input)
        );
        publish(&mut owner, snapshot(Operations::Idle));
        assert_eq!(
            status(&mut new_client)["assessment"]["state"],
            "none_observed"
        );
        let before_tx = status(&mut new_client);
        new_client
            .call("tx.begin", json!({"label":SECRET}))
            .unwrap();
        new_client
            .call(
                "world.patch",
                json!({
                    "entity":"e_00000002","component":"CollectDodgeV1::Actor",
                    "path":"position.x","value":12,
                }),
            )
            .unwrap();
        let staged = status(&mut new_client);
        assert_eq!(staged["host_scene"]["in_tx"], true);
        assert_ne!(
            staged["host_scene"]["preview_revision"],
            before_tx["host_scene"]["preview_revision"]
        );
        new_client.call("tx.rollback", json!({})).unwrap();
        assert_eq!(
            status(&mut new_client)["host_scene"],
            before_tx["host_scene"],
            "admission-backed rollback restores an earlier preview revision"
        );
        let invalid = SOURCE.replace("time_limit_ticks: 600", "time_limit_ticks: 0");
        assert_ne!(invalid, SOURCE);
        assert!(spawn_host(invalid.clone(), None, config(false)).is_err());
        let (service, _owner) = WorkStatusService::pair();
        assert!(spawn_host_with_editor_work_status(invalid, None, config(false), service).is_err());
    }
}
