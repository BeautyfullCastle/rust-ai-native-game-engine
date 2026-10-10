#![allow(clippy::disallowed_types)]

mod common;

use std::time::{Duration, Instant};

use common::{guid_of, token, TestHost};
use orr_remote::screenshot::{CapturedImage, ScreenshotOwner, ScreenshotService};
use orr_remote::{Auth, ErpClient, ServerConfig};
use serde_json::{json, Value as J};

fn hosted(service: Option<ScreenshotService>, auth: Auth) -> TestHost {
    let mut cfg = ServerConfig::new(auth);
    cfg.screenshot = service;
    TestHost::start(cfg)
}

fn wait_request(owner: &ScreenshotOwner) -> orr_remote::CaptureRequest {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(request) = owner.take_request() { return request; }
        assert!(Instant::now() < end, "screenshot request was not delivered");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn wait_response(client: &mut ErpClient, id: u64) -> Result<J, orr_remote::RpcError> {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        client.poll().expect("client remains connected");
        if let Some(response) = client.take_response(id) { return response; }
        assert!(Instant::now() < end, "screenshot response did not arrive");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn captured(request: &orr_remote::CaptureRequest, png: &[u8]) -> CapturedImage {
    CapturedImage {
        png: png.to_vec(),
        width: request.options.max_width.min(640),
        height: request.options.max_height.min(480),
        captured: request.requested,
        frame_seq: 17,
        ui_frame: 29,
    }
}

#[test]
fn permission_and_params_precede_owner_availability() {
    let denied = hosted(None, Auth::Tokens(vec![token("scene", "scene-token", "scene_edit")]));
    let mut no_read = ErpClient::connect(&denied.url, Some("scene-token")).unwrap();
    let error = no_read.call_err("view.screenshot", json!({"not_a_param": true}));
    assert_eq!(error.kind(), Some("permission_denied"));

    let headless = hosted(None, Auth::DevNoAuth);
    let mut reader = ErpClient::connect(&headless.url, None).unwrap();
    let error = reader.call_err("view.screenshot", json!({"not_a_param": true}));
    assert_eq!(error.kind(), Some("invalid_params"));
    let error = reader.call_err("view.screenshot", json!({"max_width": 2049}));
    assert_eq!(error.kind(), Some("invalid_params"));
    let error = reader.call_err("view.screenshot", json!({}));
    assert_eq!(error.kind(), Some("view_unavailable"));
}

#[test]
fn pending_capture_is_nonblocking_correlated_and_busy_for_other_callers() {
    let (service, owner) = ScreenshotService::pair();
    let host = hosted(Some(service), Auth::DevNoAuth);
    let mut caller = ErpClient::connect(&host.url, None).unwrap();
    let mut other = ErpClient::connect(&host.url, None).unwrap();

    let before = caller.call("sim.state", json!({})).unwrap();
    let id = caller.post("view.screenshot", json!({"target": "app_framebuffer", "max_width": 640, "max_height": 480})).unwrap();
    let request = wait_request(&owner);
    assert_eq!(owner.take_request(), None, "one ticket is delivered once");
    assert_eq!(other.call_err("view.screenshot", json!({})).kind(), Some("view_busy"));
    assert_eq!(caller.call("sim.state", json!({})).unwrap()["mode"], "edit", "ordinary ERP reads continue while capture is pending");

    owner.begin_capture(request.serial).unwrap();
    owner.complete(request.serial, Ok(captured(&request, b"PNG")));
    let result = wait_response(&mut caller, id).unwrap();
    assert_eq!(result["status"], "captured");
    assert_eq!(result["source"], "editor.app_framebuffer");
    assert_eq!(result["mode"], "edit");
    assert_eq!(result["paused"], true);
    assert_eq!(result["tick"], "0");
    assert_eq!(result["epoch"], before["epoch"].as_u64().unwrap().to_string());
    assert_eq!(result["checksum"], before["checksum"], "checksum is the exact state admitted for capture");
    assert_eq!(result["frame_seq"], "17");
    assert_eq!(result["ui_frame"], "29");
    assert_eq!(result["mime_type"], "image/png");
    assert_eq!(result["png_base64"], "UE5H");
}

#[test]
fn timeout_cancels_ticket_and_stale_host_view_discards_late_image() {
    let (service, owner) = ScreenshotService::pair();
    let host = hosted(Some(service), Auth::DevNoAuth);
    let mut caller = ErpClient::connect(&host.url, None).unwrap();
    let timeout_id = caller.post("view.screenshot", json!({"timeout_ms": 50})).unwrap();
    let expired = wait_request(&owner);
    owner.begin_capture(expired.serial).unwrap();
    assert_eq!(wait_response(&mut caller, timeout_id).unwrap_err().kind(), Some("view_timeout"));
    assert!(!owner.is_active(expired.serial));
    owner.complete(expired.serial, Ok(captured(&expired, b"late")));
    owner.end_capture(expired.serial);

    let stale_id = caller.post("view.screenshot", json!({})).unwrap();
    let stale = wait_request(&owner);
    owner.begin_capture(stale.serial).unwrap();
    let guid = guid_of(&mut caller, "body_05");
    caller.call("world.patch", json!({"entity": guid, "component": "orr_physics::Body", "path": "pos.x", "value": "1.25"})).unwrap();
    owner.complete(stale.serial, Ok(captured(&stale, b"stale")));
    assert_eq!(wait_response(&mut caller, stale_id).unwrap_err().kind(), Some("view_stale"));
}

#[test]
fn dropped_owner_and_disconnect_do_not_retarget_late_completion() {
    let (service, owner) = ScreenshotService::pair();
    let host = hosted(Some(service), Auth::DevNoAuth);
    let mut caller = ErpClient::connect(&host.url, None).unwrap();
    let id = caller.post("view.screenshot", json!({})).unwrap();
    let request = wait_request(&owner);
    drop(owner);
    assert_eq!(wait_response(&mut caller, id).unwrap_err().kind(), Some("view_unavailable"));
    assert!(host.is_running(), "host should remain alive after owner drop");
    let error = caller.call_err("view.screenshot", json!({}));
    assert_eq!(error.kind(), Some("view_unavailable"));
    let _ = request;
}

#[test]
fn string_json_rpc_id_is_preserved_on_the_real_websocket_route() {
    use tungstenite::{connect, Message};

    let (service, owner) = ScreenshotService::pair();
    let host = hosted(Some(service), Auth::DevNoAuth);
    let (mut socket, _) = connect(host.url.as_str()).unwrap();
    if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_mut() {
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    }
    socket.send(Message::Text(json!({"jsonrpc":"2.0", "id":"capture-client-7", "method":"view.screenshot", "params":{}}).to_string().into())).unwrap();
    let request = wait_request(&owner);
    socket.send(Message::Text(json!({"jsonrpc":"2.0", "id":"capture-client-7", "method":"view.screenshot", "params":{"unsupported":true}}).to_string().into())).unwrap();
    socket.send(Message::Text(json!({"jsonrpc":"2.0", "id":"barrier", "method":"sim.state", "params":{}}).to_string().into())).unwrap();
    let barrier: J = serde_json::from_str(socket.read().unwrap().into_text().unwrap().as_str()).unwrap();
    assert_eq!(barrier["id"], "barrier");
    assert_eq!(barrier["result"]["mode"], "edit");
    owner.begin_capture(request.serial).unwrap();
    owner.complete(request.serial, Ok(captured(&request, b"PNG")));
    let message = socket.read().unwrap();
    let Message::Text(text) = message else { panic!("expected a JSON-RPC text response") };
    let response: J = serde_json::from_str(text.as_str()).unwrap();
    assert_eq!(response["id"], "capture-client-7");
    assert_eq!(response["result"]["status"], "captured", "same-ID malformed retry coalesces without an early second response");
    assert_eq!(response["result"]["frame_seq"], "17");
    if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_mut() {
        stream.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
    }
    assert!(matches!(socket.read(), Err(tungstenite::Error::Io(error)) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)), "one JSON-RPC ID gets at most one response");
}

#[test]
fn notifications_do_not_allocate_capture_and_disconnect_cancels_its_ticket() {
    use tungstenite::{connect, Message};

    let (service, owner) = ScreenshotService::pair();
    let host = hosted(Some(service), Auth::DevNoAuth);
    let (mut socket, _) = connect(host.url.as_str()).unwrap();
    if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_mut() {
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    }
    socket.send(Message::Text(json!({"jsonrpc":"2.0", "method":"view.screenshot", "params":{}}).to_string().into())).unwrap();
    socket.send(Message::Text(json!({"jsonrpc":"2.0", "id":"barrier", "method":"sim.state", "params":{}}).to_string().into())).unwrap();
    let barrier: J = serde_json::from_str(socket.read().unwrap().into_text().unwrap().as_str()).unwrap();
    assert_eq!(barrier["id"], "barrier");
    assert_eq!(owner.take_request(), None, "a notification must not allocate a ticket");
    drop(socket);

    let mut caller = ErpClient::connect(&host.url, None).unwrap();
    let _id = caller.post("view.screenshot", json!({})).unwrap();
    let request = wait_request(&owner);
    owner.begin_capture(request.serial).unwrap();
    drop(caller);
    let end = Instant::now() + Duration::from_secs(5);
    while owner.is_active(request.serial) {
        assert!(Instant::now() < end, "disconnected caller retained its request slot");
        std::thread::sleep(Duration::from_millis(1));
    }
    owner.complete(request.serial, Ok(captured(&request, b"late")));
    owner.end_capture(request.serial);
    let mut replacement = ErpClient::connect(&host.url, None).unwrap();
    let next_id = replacement.post("view.screenshot", json!({})).unwrap();
    let next = wait_request(&owner);
    assert_ne!(next.serial, request.serial);
    owner.begin_capture(next.serial).unwrap();
    owner.complete(next.serial, Ok(captured(&next, b"PNG")));
    assert_eq!(wait_response(&mut replacement, next_id).unwrap()["status"], "captured");
}

#[test]
fn host_view_incarnation_change_invalidates_a_late_capture() {
    let (service, owner) = ScreenshotService::pair();
    let host = hosted(Some(service), Auth::DevNoAuth);
    let mut caller = ErpClient::connect(&host.url, None).unwrap();
    let id = caller.post("view.screenshot", json!({})).unwrap();
    let request = wait_request(&owner);
    owner.begin_capture(request.serial).unwrap();
    caller.call("sim.start", json!({})).unwrap();
    caller.call("sim.stop", json!({})).unwrap();
    owner.complete(request.serial, Ok(captured(&request, b"old incarnation")));
    assert_eq!(wait_response(&mut caller, id).unwrap_err().kind(), Some("view_stale"));
}
