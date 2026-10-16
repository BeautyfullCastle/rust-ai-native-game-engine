//! Local ERP screenshot protocol and UI-pump tests with synthetic egui screenshot events.
//! This harness initializes no renderer and proves no readback; `arena_native_window_smoke`
//! exercises the real native framebuffer.
#![allow(clippy::disallowed_types)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Color32, ColorImage, Event, Pos2, Rect, ViewportId};
use egui_kittest::Harness;
use orr_editor::app::EditorApp;
use orr_editor::editor::{default_scene_path, Editor};
use orr_editor::HostSpec;
use orr_remote::{Auth, ErpClient, ServerConfig};
use serde_json::{json, Value as J};

fn harness() -> (Harness<'static, EditorApp>, ErpClient) {
    let spec = HostSpec::Local {
        scene: default_scene_path(),
        listen: Some(ServerConfig::new(Auth::DevNoAuth)),
        debug_hooks: false,
    };
    let mut editor = Editor::start(&spec).expect("start local editor host with ERP");
    editor.sync();
    let url = editor.erp_status().expect("the local editor host listens").0;
    let mut harness = Harness::builder()
        .with_size([960.0, 720.0])
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(move |_cc| EditorApp::new(editor, None));
    harness.input_mut().viewports.get_mut(&ViewportId::ROOT).unwrap().inner_rect =
        Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(960.0, 720.0)));
    harness.run_steps(3);
    let agent = ErpClient::connect(&url, None).expect("connect external ERP client");
    (harness, agent)
}

fn screenshot_command(harness: &Harness<'_, EditorApp>) -> Option<u64> {
    let root = harness.output().viewport_output.get(&ViewportId::ROOT)?;
    root.commands.iter().find_map(|command| {
        let egui::ViewportCommand::Screenshot(data) = command else { return None };
        data.data.as_ref()?.downcast_ref::<u64>().copied()
    })
}

fn wait_for_screenshot_command(
    harness: &mut Harness<'_, EditorApp>,
    agent: &mut ErpClient,
) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(ticket) = screenshot_command(harness) {
            return ticket;
        }
        harness.run_steps(1);
        agent.poll().expect("ERP stays connected while UI settles");
        assert!(Instant::now() < deadline, "editor did not emit a screenshot command");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn synthetic_screenshot(ticket: u64, viewport_id: ViewportId, size: [usize; 2]) -> Event {
    let pixels = (0..size[0] * size[1])
        .map(|index| {
            if (index / size[0] + index % size[0]).is_multiple_of(2) {
                Color32::from_rgb(24, 80, 144)
            } else {
                Color32::from_rgb(216, 176, 72)
            }
        })
        .collect();
    Event::Screenshot {
        viewport_id,
        user_data: egui::UserData::new(ticket),
        image: Arc::new(ColorImage::new(size, pixels)),
    }
}

fn wait_response(
    harness: &mut Harness<'_, EditorApp>,
    agent: &mut ErpClient,
    id: u64,
) -> Result<J, orr_remote::RpcError> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        agent.poll().expect("ERP stays connected while screenshot encodes");
        if let Some(response) = agent.take_response(id) {
            return response;
        }
        harness.run_steps(1);
        assert!(Instant::now() < deadline, "screenshot response did not arrive");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn wait_for_screenshot_command_before_timeout(
    harness: &mut Harness<'_, EditorApp>,
    agent: &mut ErpClient,
    request_id: u64,
) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(ticket) = screenshot_command(harness) {
            return ticket;
        }
        agent.poll().expect("ERP stays connected while capture is requested");
        if let Some(response) = agent.take_response(request_id) {
            panic!("short-deadline screenshot failed before it issued a GPU ticket: {response:?}");
        }
        harness.run_steps(1);
        assert!(Instant::now() < deadline, "editor did not issue the short-deadline screenshot command");
        std::thread::yield_now();
    }
}

fn replace_screenshot_event(harness: &mut Harness<'_, EditorApp>, event: Event) {
    harness
        .input_mut()
        .events
        .retain(|queued| !matches!(queued, Event::Screenshot { .. }));
    harness.input_mut().events.push(event);
}

#[test]
fn local_erp_screenshot_rejects_foreign_tickets_then_returns_correlated_metadata() {
    let (mut harness, mut agent) = harness();
    let discovery = agent.call("rpc.discover", J::Null).expect("discover local editor");
    let state = agent.call("sim.state", J::Null).expect("read baseline state");
    let id = agent
        .post("view.screenshot", json!({"target":"app_framebuffer", "timeout_ms":5000}))
        .expect("post screenshot request");

    // The editor must continue servicing ordinary ERP while its UI capture is pending.
    let concurrent = agent.call("sim.state", J::Null).expect("ordinary ERP read during capture");
    assert_eq!(concurrent["checksum"], state["checksum"]);

    let ticket = wait_for_screenshot_command(&mut harness, &mut agent);
    replace_screenshot_event(
        &mut harness,
        synthetic_screenshot(ticket.wrapping_add(1), ViewportId::ROOT, [960, 720]),
    );
    harness.run_steps(1);
    agent.poll().expect("poll after foreign screenshot event");
    assert!(agent.take_response(id).is_none(), "a foreign ticket must not complete the request");

    replace_screenshot_event(
        &mut harness,
        synthetic_screenshot(ticket, ViewportId::from_hash_of("foreign-viewport"), [960, 720]),
    );
    harness.run_steps(1);
    agent.poll().expect("poll after foreign viewport event");
    assert!(agent.take_response(id).is_none(), "a non-root viewport must not complete the request");

    replace_screenshot_event(
        &mut harness,
        synthetic_screenshot(ticket, ViewportId::ROOT, [960, 720]),
    );
    harness.run_steps(1);
    let image = wait_response(&mut harness, &mut agent, id).expect("matching synthetic screenshot succeeds");
    assert_eq!(image["status"], "captured");
    assert_eq!(image["source"], "editor.app_framebuffer");
    assert_eq!(image["game"], discovery["engine"]["game"]);
    assert_eq!(image["build_id"], discovery["engine"]["build_id"]);
    assert_eq!(image["mode"], state["mode"]);
    assert_eq!(image["paused"], true);
    assert_eq!(image["tick"], state["head_tick"].as_u64().unwrap().to_string());
    assert_eq!(image["epoch"], state["epoch"].as_u64().unwrap().to_string());
    assert_eq!(image["checksum"], state["checksum"]);
    assert_eq!(image["mime_type"], "image/png");
    assert_eq!(image["width"], 960);
    assert_eq!(image["height"], 720);
    for key in ["frame_seq", "ui_frame"] {
        assert!(image[key].as_str().unwrap().parse::<u64>().unwrap() > 0, "{key}: {image}");
    }
    assert!(image["png_base64"].as_str().unwrap().len() > 100);
    assert_eq!(agent.call("sim.state", J::Null).unwrap()["checksum"], state["checksum"]);
}

#[test]
fn local_erp_screenshot_is_stale_if_the_window_resizes_after_capture_request() {
    let (mut harness, mut agent) = harness();
    let id = agent
        .post("view.screenshot", json!({"target":"app_framebuffer", "timeout_ms":5000}))
        .expect("post screenshot request");
    let ticket = wait_for_screenshot_command(&mut harness, &mut agent);

    // Deliver a correctly ticketed image from the old root geometry only after
    // the next UI pass has observed a resize. It must not be restamped as fresh.
    harness.set_size(egui::vec2(1000.0, 720.0));
    harness.input_mut().viewports.get_mut(&ViewportId::ROOT).unwrap().inner_rect =
        Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 720.0)));
    replace_screenshot_event(
        &mut harness,
        synthetic_screenshot(ticket, ViewportId::ROOT, [960, 720]),
    );
    harness.run_steps(1);
    let error = wait_response(&mut harness, &mut agent, id).unwrap_err();
    assert_eq!(error.kind(), Some("view_stale"), "{error}");
}

#[test]
fn local_erp_screenshot_is_unavailable_during_proposal_preview() {
    let (mut harness, mut agent) = harness();
    let proposal = agent
        .call("proposal.begin", json!({"label":"screenshot preview boundary"}))
        .expect("begin proposal");
    let proposal_id = proposal["id"].as_str().expect("proposal id").to_string();
    harness.state_mut().editor.sync();
    assert!(harness.state_mut().editor.set_preview(Some(proposal_id.clone())));
    assert_eq!(harness.state().editor.previewing(), Some(proposal_id.as_str()));
    harness.run_steps(2);

    let id = agent
        .post("view.screenshot", json!({"target":"app_framebuffer", "timeout_ms":5000}))
        .expect("post screenshot request during preview");
    let error = wait_response(&mut harness, &mut agent, id).unwrap_err();
    assert_eq!(error.kind(), Some("view_unavailable"), "{error}");
}

#[test]
fn timed_out_gpu_ticket_keeps_admission_busy_until_its_late_event_is_discarded() {
    let (mut harness, mut agent) = harness();
    let expired_id = agent
        .post("view.screenshot", json!({"target":"app_framebuffer", "timeout_ms":1000}))
        .expect("post bounded-deadline screenshot request");
    let expired_ticket = wait_for_screenshot_command_before_timeout(&mut harness, &mut agent, expired_id);

    let error = wait_response(&mut harness, &mut agent, expired_id).unwrap_err();
    assert_eq!(error.kind(), Some("view_timeout"), "{error}");

    // A foreign late completion cannot release the encoder/GPU ticket permit.
    replace_screenshot_event(
        &mut harness,
        synthetic_screenshot(expired_ticket.wrapping_add(1), ViewportId::ROOT, [960, 720]),
    );
    harness.run_steps(1);
    let busy = agent.call_err("view.screenshot", json!({"target":"app_framebuffer"}));
    assert_eq!(busy.kind(), Some("view_busy"), "foreign ticket must leave the outstanding GPU request busy");

    replace_screenshot_event(
        &mut harness,
        synthetic_screenshot(expired_ticket, ViewportId::from_hash_of("foreign-viewport"), [960, 720]),
    );
    harness.run_steps(1);
    let busy = agent.call_err("view.screenshot", json!({"target":"app_framebuffer"}));
    assert_eq!(busy.kind(), Some("view_busy"), "foreign viewport must leave the outstanding GPU request busy");

    // Only the canceled request's exact late event may be discarded to retire
    // its outstanding viewport command and release that reserved permit.
    replace_screenshot_event(
        &mut harness,
        synthetic_screenshot(expired_ticket, ViewportId::ROOT, [960, 720]),
    );
    harness.run_steps(1);

    let next_id = agent
        .post("view.screenshot", json!({"target":"app_framebuffer", "timeout_ms":5000}))
        .expect("post next screenshot after late event was discarded");
    let next_ticket = wait_for_screenshot_command(&mut harness, &mut agent);
    assert_ne!(next_ticket, expired_ticket, "a new request must get a fresh GPU event ticket");
    replace_screenshot_event(
        &mut harness,
        synthetic_screenshot(next_ticket, ViewportId::ROOT, [960, 720]),
    );
    harness.run_steps(1);
    assert_eq!(
        wait_response(&mut harness, &mut agent, next_id).unwrap()["status"],
        "captured"
    );
}
