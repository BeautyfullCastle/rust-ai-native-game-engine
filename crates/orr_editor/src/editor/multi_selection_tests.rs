//! Controller-level multi-selection regressions. The public websocket tests
//! live in `tests/multi_select.rs`; this module additionally exercises cache
//! fencing and a response lost after the host has already applied a batch.
use super::*;
use std::sync::{Arc, Mutex};

use orr_remote::{Auth, Incoming, Request, Transport, WsTransport};
use serde_json::{json, Value as J};

fn editor() -> Editor {
    let mut editor = Editor::open(&default_scene_path()).expect("open editor fixture");
    editor.sync();
    editor
}

fn guid(text: &str) -> Guid {
    Guid::parse(text).expect("fixture GUID")
}

#[test]
fn stale_guid_selection_does_not_follow_a_reused_entity_handle() {
    let mut editor = editor();
    let old = guid("e_00000001");
    let replacement = guid("e_0000f001");
    let handle = editor
        .rows
        .iter()
        .find(|row| row.guid.as_ref() == Some(&old))
        .unwrap()
        .entity;
    editor.select(Some(Target::Guid(old.clone())));

    // Simulate a fresh authoritative rows response after despawn/reuse: the
    // ECS handle is the same, but the document identity is different.
    editor.rows = vec![EntityRow {
        entity: handle,
        guid: Some(replacement.clone()),
        name: Some("replacement".into()),
        components: vec!["orr_physics::Body".into()],
    }];
    editor.dirty.rows = false;
    editor.inflight.rows = false;
    editor.sanitize_selection();

    assert!(editor.selected_guids().is_empty());
    assert!(editor.selection().is_none());
    assert!(!editor.is_selected(&Target::Guid(replacement)));
}

struct DropAcceptedBatchAck {
    inner: Option<Box<dyn Transport>>,
    batch_ids: Vec<u64>,
    accepted: Arc<Mutex<Vec<J>>>,
}

impl Transport for DropAcceptedBatchAck {
    fn send(&mut self, request: Request) -> Result<(), ClientError> {
        if request.method == "world.patch_batch" {
            if let Some(id) = request.id {
                self.batch_ids.push(id);
            }
        }
        self.inner
            .as_mut()
            .ok_or_else(|| ClientError::Transport("injected connection close".into()))?
            .send(request)
    }

    fn recv(&mut self, timeout: Duration) -> Result<Option<Incoming>, ClientError> {
        let Some(message) = self
            .inner
            .as_mut()
            .ok_or_else(|| ClientError::Transport("injected connection close".into()))?
            .recv(timeout)?
        else {
            return Ok(None);
        };
        if let Incoming::Text(text) = &message {
            let value: J = serde_json::from_str(text).expect("ERP response JSON");
            if value
                .get("id")
                .and_then(J::as_u64)
                .is_some_and(|id| self.batch_ids.contains(&id))
            {
                let result = value
                    .get("result")
                    .cloned()
                    .expect("successful host response");
                assert_eq!(
                    result["changed"], true,
                    "host applied the batch before its acknowledgement was dropped"
                );
                self.accepted.lock().unwrap().push(result);
                self.inner.take(); // close the underlying WebSocket after acceptance
                return Err(ClientError::Transport(
                    "injected loss after host accepted batch".into(),
                ));
            }
        }
        Ok(Some(message))
    }
}

#[test]
fn accepted_batch_with_lost_ack_is_uncertain_reconnected_without_retry() {
    const SCENE: &str = "schema: orr.scene/1\nsingletons:\n  Score: { kills: [0,0,0,0,0,0,0,0] }\nentities:\n  e_00000001:\n    name: hero\n    Position: { pos: [-300,0] }\n    PlayerTag: { slot: 0 }\n  e_00000002:\n    name: target\n    Position: { pos: [300,0] }\n    PlayerTag: { slot: 1 }\n";
    let mut config = orr_remote::ServerConfig::new(Auth::DevNoAuth);
    config.listen = true;
    config.limits.allow_scene_paths = true;
    let host = orr_remote::sample::spawn_arena_host(SCENE.to_string(), None, config)
        .expect("start Arena fixture");
    let url = host.url().expect("listening host URL").to_string();
    let spec = HostSpec::remote(&url, None);
    let mut backend = Backend::connect(&spec).expect("connect Arena view/editor channels");
    let accepted = Arc::new(Mutex::new(Vec::new()));
    let client_url = if url.contains('?') {
        format!("{url}&client=user")
    } else {
        format!("{url}/?client=user")
    };
    backend.erp = ErpClient::with_transport(Box::new(DropAcceptedBatchAck {
        inner: Some(Box::new(
            WsTransport::connect(&client_url).expect("ERP websocket"),
        )),
        batch_ids: Vec::new(),
        accepted: accepted.clone(),
    }));
    let mut editor = Editor::on_backend(backend).expect("initialize remote editor");
    editor.sync();
    let hero = guid("e_00000001");
    let target = guid("e_00000002");
    editor.select(Some(Target::Guid(hero.clone())));
    assert!(editor.toggle_selection(Target::Guid(target)));
    assert!(editor.can_nudge_selection());

    assert!(!editor.nudge_selected(FPVec2::new(FP::from_int(1), FP::ZERO)));
    assert_eq!(
        accepted.lock().unwrap().len(),
        1,
        "the wrapper observed a successful host result"
    );
    assert!(editor.batch_outcome_uncertain());
    assert!(
        !editor.can_nudge_selection(),
        "uncertain outcome fences another mutation"
    );
    assert!(
        !editor.nudge_selected(FPVec2::new(FP::from_int(1), FP::ZERO)),
        "the editor refuses an explicit retry while the outcome is uncertain"
    );
    assert_eq!(
        accepted.lock().unwrap().len(),
        1,
        "the uncertain-state refusal sends no second batch"
    );
    assert_eq!(
        editor.selected_guids(),
        &[hero, guid("e_00000002")],
        "the unsynced UI selection remains visible until reconnection"
    );

    assert!(
        editor.restart(),
        "remote reconnect performs a fresh fenced read"
    );
    assert!(!editor.batch_outcome_uncertain());
    assert!(editor
        .history()
        .entries
        .iter()
        .any(|entry| entry.label == "move selected entities"));
    assert_eq!(
        accepted.lock().unwrap().len(),
        1,
        "reconnect never retries the accepted request"
    );
    assert!(editor.log().iter().any(|entry| entry
        .text
        .contains("authoritative document and history read")
        && entry.text.contains("no batch was retried")));
    let mut observer = ErpClient::connect(&client_url, None).expect("fresh independent ERP read");
    assert_eq!(
        observer
            .call(
                "world.get",
                json!({"entity":"e_00000001","component":"Position","path":"pos"})
            )
            .unwrap()["value"],
        json!([-299, 0])
    );
    assert_eq!(
        observer
            .call(
                "world.get",
                json!({"entity":"e_00000002","component":"Position","path":"pos"})
            )
            .unwrap()["value"],
        json!([301, 0])
    );
    drop(editor);
    drop(host);
}
