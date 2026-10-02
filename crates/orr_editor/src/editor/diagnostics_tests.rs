use super::*;

#[test]
fn timings_follow_calls_replies_and_changed_snapshots_without_polling_on_read() {
    let mut ed = Editor::open(&default_scene_path()).unwrap();
    ed.sync();
    let baseline = ed.diagnostics();
    assert!(baseline.sync_erp_wait.total_samples >= 7, "initial editor queries are timed");
    assert!(baseline.snapshot_extract.total_samples > 0);
    assert_eq!(baseline.ui_frame.samples, 0, "headless editor does not invent UI frames");
    assert_eq!(baseline.pending_requests, 0);

    let snapshot = ed.snapshot().unwrap().clone();
    ed.apply_snapshot(snapshot);
    assert_eq!(ed.diagnostics().snapshot_extract, baseline.snapshot_extract, "unchanged snapshots do no extraction");
    assert_eq!(ed.diagnostics(), baseline, "reading diagnostics is side-effect free");

    assert!(ed.call("sim.state", J::Null).is_ok());
    assert!(ed.call("unknown.telemetry.test", J::Null).is_err());
    assert_eq!(ed.diagnostics().sync_erp_wait.total_samples, baseline.sync_erp_wait.total_samples + 2);
    assert!(ed.down().is_none(), "RPC errors do not change connection behavior");

    ed.post("sim.state", J::Null, Pend::State);
    ed.post("unknown.telemetry.test", J::Null, Pend::State);
    let posted = ed.diagnostics();
    assert_eq!(posted.pending_requests, 2);
    assert!(posted.pending_high_water >= 2);
    assert_eq!(posted.async_request.total_samples, baseline.async_request.total_samples);
    // The query must not ingest a reply even if the host has already sent it.
    assert_eq!(ed.diagnostics(), posted);
    assert!(ed.call("sim.state", J::Null).is_ok(), "ordered host barrier ingests both replies");
    assert_eq!(ed.diagnostics().pending_requests, 0);
    assert_eq!(ed.diagnostics().async_request.total_samples, baseline.async_request.total_samples + 2);
    ed.pump();
    assert!(ed.diagnostics().pump.total_samples > baseline.pump.total_samples);
}

#[test]
fn disconnected_requests_do_not_create_latency_samples() {
    let mut ed = Editor::open(&default_scene_path()).unwrap();
    ed.sync();
    // Test the terminal-state fast path without crashing a host or waiting on
    // wall-clock thresholds. Existing crash tests exercise real disconnection.
    ed.down = Some(Down {
        reason: "test disconnected".into(),
        local: true,
    });
    let before = ed.diagnostics();
    assert!(ed.call("sim.state", J::Null).is_err());
    ed.post("sim.state", J::Null, Pend::State);
    ed.pump();
    let after = ed.diagnostics();
    assert_eq!(after.sync_erp_wait, before.sync_erp_wait);
    assert_eq!(after.async_request, before.async_request);
    assert_eq!(after.pending_requests, 0);
    assert_eq!(after.pump.total_samples, before.pump.total_samples + 1);
}
