//! Real egui frames populate telemetry without changing the UI or contacting
//! the host when diagnostics are queried. No machine-dependent time limits.
mod common;

use egui_kittest::Harness;
use orr_editor::diagnostics::LATENCY_WINDOW;
use orr_editor::EditorApp;

#[test]
fn ui_frames_record_bounded_wall_time_samples() {
    let mut h = Harness::builder().with_size([1500.0, 900.0]).build_eframe(|_| EditorApp::new(common::demo_editor(), None));
    let before_frames = h.state().frame_count();
    let before = h.state().editor.diagnostics();
    h.run_steps(LATENCY_WINDOW + 2);
    let app = h.state();
    let stats = app.editor.diagnostics();
    assert_eq!(stats.ui_frame.total_samples - before.ui_frame.total_samples, app.frame_count() - before_frames);
    assert_eq!(stats.ui_frame.samples, LATENCY_WINDOW);
    assert!(stats.ui_frame.max >= stats.ui_frame.last);
    assert!(stats.ui_frame.max >= stats.ui_frame.p95);
    assert_eq!(app.editor.diagnostics(), stats);
    assert!(app.editor.down().is_none());
}
