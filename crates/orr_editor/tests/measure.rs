//! Numbers for the decision-12 change (run with `--nocapture`):
//!
//! - **drag to viewport**: from a pointer move of a body drag to the first
//!   frame whose viewport shows the body at the new place. Before the change
//!   the edit was applied to the editor's own document inside the same UI
//!   frame (0 frames of delay); now it goes to the host thread and comes back
//!   as a snapshot.
//! - **play-mode frame time with 1000 bodies**: the cost of one UI frame
//!   (layout, panels, render list; no GPU) while the host plays.
//!
//! The assertions are loose on purpose (machines differ); the printed
//! numbers are what is compared.
#![allow(clippy::disallowed_types)]
#![allow(clippy::float_arithmetic)]

mod common;

use std::time::{Duration, Instant};

use common::*;
use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use orr_editor::editor::Editor;
use orr_editor::EditorApp;
use serde_json::json;

fn thousand_bodies() -> Editor {
    let mut ed = demo_editor();
    for i in 0..1000 {
        let x = -14.0 + (i % 40) as f32 * 0.7;
        let y = 2.0 + (i / 40) as f32 * 0.7;
        assert!(ed.spawn_body([x, y]));
    }
    ed.sync();
    ed
}

fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..2 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

#[test]
fn drag_to_viewport_latency() {
    let mut h = Harness::builder().with_size([1500.0, 900.0]).with_step_dt(1.0 / 60.0).build_eframe(|_cc| EditorApp::new(demo_editor(), None));
    settle(&mut h);
    let rect = h.state().ui.viewport_rect.expect("viewport");
    let vp = h.state().ui.viewport_px;
    let t = target_named(&h.state().editor, "body_05");
    let p = xy(&field(&mut h.state_mut().editor, &t, BODY, "pos"));
    let s = h.state().editor.camera.world_to_screen(p, vp);
    let at = Pos2::new(rect.min.x + s[0], rect.min.y + s[1]);
    h.event(Event::PointerMoved(at));
    h.step();
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    let start_world = h.state().editor.camera.screen_to_world([at.x - rect.min.x, at.y - rect.min.y], vp);
    let (mut frames, mut millis) = (Vec::new(), Vec::new());
    for i in 1..=40 {
        let pointer = at + egui::vec2(i as f32 * 3.0, 0.0);
        let world = h.state().editor.camera.screen_to_world([pointer.x - rect.min.x, pointer.y - rect.min.y], vp);
        let want_x = p[0] + (world[0] - start_world[0]);
        h.event(Event::PointerMoved(pointer));
        let t0 = Instant::now();
        let mut n = 0;
        loop {
            h.step();
            n += 1;
            // Within a pixel or two of where the pointer put it (the pointer steps are whole pixels).
            let px_world = (world[0] - start_world[0]).abs() / (i as f32 * 3.0);
            let shown = h.state().editor.bodies().iter().any(|b| (b.pos[1] - p[1]).abs() < 0.5 && (b.pos[0] - want_x).abs() < 2.5 * px_world);
            if shown || n >= 200 {
                break;
            }
        }
        if n >= 200 {
            let near: Vec<_> = h.state().editor.bodies().iter().filter(|b| (b.pos[1] - p[1]).abs() < 1.0).map(|b| b.pos).collect();
            panic!("the moved body never showed (move {i}): want x {want_x}, p {p:?}, bodies near: {near:?}, history {:?}, status {:?}", h.state().editor.history(), h.state().editor.status());
        }
        frames.push(f64::from(n));
        millis.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    settle(&mut h);
    let worst = frames.iter().copied().fold(0.0, f64::max);
    let ms_worst = millis.iter().copied().fold(0.0, f64::max);
    eprintln!("per move (frames, ms): {:?}", frames.iter().zip(&millis).map(|(f, m)| format!("{f:.0}/{m:.1}")).collect::<Vec<_>>());
    eprintln!(
        "MEASURE drag->viewport: median {:.0} frame(s) / {:.2} ms, worst {:.0} frame(s) / {:.2} ms over 40 moves",
        median(&mut frames.clone()),
        median(&mut millis.clone()),
        worst,
        ms_worst
    );
    assert!(ms_worst < 40.0, "a dragged body shows within a couple of display frames");
}

#[test]
fn play_frame_time_with_a_thousand_bodies() {
    let ed = thousand_bodies();
    let mut h = Harness::builder().with_size([1500.0, 900.0]).with_step_dt(1.0 / 60.0).build_eframe(move |_cc| EditorApp::new(ed, None));
    settle(&mut h);
    let time_steps = |h: &mut Harness<'_, EditorApp>, n: u32| -> (f64, f64) {
        let (mut total, mut worst) = (0.0f64, 0.0f64);
        for _ in 0..n {
            let t = Instant::now();
            h.step();
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            total += ms;
            worst = worst.max(ms);
        }
        (total / f64::from(n), worst)
    };
    let (edit_mean, _) = time_steps(&mut h, 60);
    eprintln!("MEASURE edit-mode UI frame (1000+ bodies): {edit_mean:.3} ms");

    h.state_mut().editor.play();
    h.run_steps(5);
    let t0 = h.state().editor.sim().head_tick;
    let wall = Instant::now();
    let (mean, worst) = time_steps(&mut h, 240);
    let secs = wall.elapsed().as_secs_f64();
    let t1 = h.state_mut().editor.host_call("sim.state", json!({})).unwrap()["head_tick"].as_u64().unwrap();
    eprintln!("MEASURE play-mode UI frame (1000+ bodies, local host thread, 1x): mean {mean:.3} ms, worst {worst:.3} ms, host ran {} ticks in {secs:.2} s", t1 - t0);

    // The host at 4x: the UI frame does not get slower, the host thread does the simulating.
    h.state_mut().editor.set_speed(4.0);
    h.state_mut().editor.control(orr_bridge::ControlOp::Play);
    let (mean4, worst4) = time_steps(&mut h, 240);
    eprintln!("MEASURE play-mode UI frame (4x): mean {mean4:.3} ms, worst {worst4:.3} ms");

    // How fast the host simulates (a blocking step of 600 ticks, which also publishes one frame).
    h.state_mut().editor.pause();
    let t = Instant::now();
    h.state_mut().editor.step(600);
    let per_tick = t.elapsed().as_secs_f64() * 1000.0 / 600.0;
    eprintln!("MEASURE host simulation: {per_tick:.3} ms per tick (600-tick step, 1000+ bodies)");
    assert!(mean < 50.0 && mean4 < 50.0, "the UI stays responsive");
    let _ = Duration::ZERO;
}
