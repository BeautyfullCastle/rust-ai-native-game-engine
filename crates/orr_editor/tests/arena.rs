//! Arena visual authoring over real WebSockets, sharing the host's document and input.
#![allow(clippy::disallowed_types)]
mod common;
use common::{arena::*, *};
use orr_editor::{Editor, Mode, Owner};
use orr_editor::game::EditorGame;
use serde_json::{json, Value as J};

#[test]
fn arena_attach_inspect_drag_undo_and_proposal_sources() {
    let host = ArenaHost::start(false);
    let mut agent = host.client();
    let mut ed = Editor::attach(&host.url, None).unwrap();
    ed.sync();
    assert_eq!(ed.game(), EditorGame::Arena);
    assert_eq!(ed.bodies().len(), 2);
    assert!(ed.types().get("Position").is_some());
    assert!(ed.types().get("PlayerTag").is_some());
    assert!(ed.types().get(BODY).is_none());
    assert!(ed.singletons().iter().any(|(name, _)| name == "Score"));
    assert_eq!(ed.pick([-300.0, 0.0]), Some(target_named(&ed, "hero")));
    assert!(ed.pick([0.0, 0.0]).is_none());
    let hero_screen = ed.camera.world_to_screen([-300.0, 0.0], (800, 600));
    assert!(hero_screen[0] > 0.0 && hero_screen[0] < 800.0);
    let original = ed.checksum();
    assert!(ed.select_named("hero"));
    ed.sync();
    assert_eq!(ed.movement_owner(), Some(Owner::Component("Position".into())));
    assert!(ed.inspect().unwrap().components.iter().any(|(n, _)| n == "PlayerTag"));
    ed.camera = orr_render::Camera::new([25.0, 50.0], 500.0);
    let camera = ed.camera;
    ed.begin_edit("move Arena player");
    for x in -299..=-250 { assert!(ed.set_field(&Owner::Component("Position".into()), "pos", vec2(x, 40))); }
    ed.end_edit();
    ed.sync();
    assert_eq!(ed.camera, camera, "presentation resync must not refit during a drag");
    assert_eq!(position(&mut agent), json!([-250, 40]));
    assert_eq!(ed.history().entries.len(), 1);
    assert_eq!(ed.history().entries[0].origin, "user");
    assert!(!ed.spawn_body([0.0, 0.0]));
    assert!(ed.undo()); ed.sync();
    assert_eq!(ed.checksum(), original);
    assert_eq!(position(&mut agent), json!([-300, 0]));

    let first = proposal(&mut agent, -100, 100);
    let second = proposal(&mut agent, 100, -100);
    ed.sync();
    assert!(ed.set_preview(Some(first)));
    assert!(ed.set_preview(Some(second.clone())));
    ed.sync();
    assert_eq!(ed.previewing(), Some(second.as_str()));
    assert!(ed.preview_bodies().unwrap().iter().any(|b| b.pos == [100.0, -100.0]));
    assert!(ed.bodies().iter().any(|b| b.pos == [-300.0, 0.0]));
    let list = ed.viewport_list((800,600), &[]);
    assert!(list.lines.iter().any(|l| l.color == orr_editor::viewport::PREVIEW_GHOST));
    assert!(ed.set_preview(None));
    assert_eq!(position(&mut agent), json!([-300, 0]));
}

#[test]
fn arena_gui_preserves_agent_input_and_replays_normal_fire() {
    let host = ArenaHost::start(false);
    let mut agent = host.client();
    let mut ed = Editor::attach(&host.url, None).unwrap(); ed.sync();
    let document = ed.checksum();
    ed.start_play();
    input(&mut agent, 1, true);
    // Neither GUI pumping nor stepping sends a neutral/raw input over the agent's held input.
    for _ in 0..10 { ed.pump(); }
    ed.step(1); ed.sync();
    assert_eq!(position(&mut agent), json!([-294, 0]));
    assert_eq!(ed.bodies().len(), 3, "normal fire generated its command");
    ed.step(1); ed.sync();
    assert_eq!(position(&mut agent), json!([-288, 0]));
    assert_eq!(ed.bodies().len(), 4, "agent held fire survived another GUI step");
    input(&mut agent, 0, false);
    ed.step(25); ed.sync();
    let end = ed.checksum();
    assert!(agent.call("world.singleton.get", json!({"name":"Score","path":"kills[0]"})).unwrap()["value"].as_u64().unwrap() >= 1);
    ed.seek(1); ed.sync();
    assert_eq!(position(&mut agent), json!([-294, 0]));
    ed.seek(27); ed.sync(); assert_eq!(ed.checksum(), end);
    let stopped = ed.stop().unwrap().clone(); ed.sync();
    assert_eq!(ed.mode(), Mode::Edit);
    assert_eq!(ed.checksum(), document);
    let reader = orr_session::ReplayReader::<orr_sample::arena_game::Arena>::parse(&stopped.replay).unwrap();
    assert_eq!(reader.tick(1).unwrap().1.len(), 1);
    assert_eq!(reader.tick(2).unwrap().1.len(), 1);
    assert!(reader.tick(3).unwrap().1.is_empty());
    assert_eq!(agent.call("verify.self", json!({"inputs":{"kind":"last_play"},"checks":["recording_matches"]})).unwrap()["passed"], true);
}

#[test]
fn replay_viewer_inspects_and_seeks_without_any_mutation_or_auto_branch() {
    let host = ArenaHost::start(true);
    let mut agent = host.client();
    let mut ed = Editor::attach(&host.url, None).unwrap(); ed.sync();
    assert!(ed.sim().viewer && ed.is_viewer());
    assert!(!ed.can_mutate());
    ed.select_named("hero"); ed.sync();
    assert!(ed.inspect().is_some());
    let before = agent.call("sim.state", J::Null).unwrap();
    ed.begin_edit("refused Viewer drag");
    assert!(!ed.set_field(&Owner::Component("Position".into()), "pos", vec2(0,0)));
    ed.end_edit();
    assert!(!ed.add_component("Bullet"));
    assert!(!ed.remove_component("PlayerTag"));
    assert!(!ed.delete_selected());
    assert!(!ed.save_as(&temp_path("viewer-must-not-save.yaml")));
    assert!(ed.host_call("sim.input_value", json!({"player":0,"value":{"axis_x":1,"axis_y":0,"buttons":[]}})).is_err());
    assert!(ed.host_call("world.patch", json!({"entity":"e_00000001","component":"Position","path":"pos","value":[0,0]})).is_err());
    ed.branch();
    assert!(!ed.in_gesture());
    let after = agent.call("sim.state", J::Null).unwrap();
    assert_eq!(after["session_mode"], "viewer");
    assert_eq!(before["branches"], after["branches"]);
    assert_eq!(before["checksum"], after["checksum"]);
    ed.seek(10); ed.sync(); assert_eq!(ed.sim().head_tick, 10);
    ed.step(1); ed.sync(); assert_eq!(ed.sim().head_tick, 11);
    assert!(ed.is_viewer());
    ed.stop(); ed.sync(); assert!(!ed.is_viewer());
    assert!(ed.can_mutate());
}

#[test]
fn unsupported_and_supported_but_incompatible_hosts_fail_before_frames() {
    for (game, error) in [("Yard3D", "unsupported editor game"), ("", "unsupported editor game"), ("PhysGame", "reflected schema mismatch")] {
        let host = ArenaHost::named(false, game);
        let message = Editor::attach(&host.url, None).err().expect("no decoder fallback");
        assert!(message.contains(error), "{message}");
    }
}

#[test]
fn arena_disconnect_clears_gesture_and_disables_editing() {
    let mut host = ArenaHost::start(false);
    let mut ed = Editor::attach(&host.url, None).unwrap(); ed.sync(); ed.select_named("hero");
    ed.begin_edit("disconnect drag");
    ed.set_field(&Owner::Component("Position".into()), "pos", vec2(-200, 20));
    host.stop();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while ed.down().is_none() && std::time::Instant::now() < until { ed.pump(); std::thread::sleep(std::time::Duration::from_millis(2)); }
    assert!(ed.down().is_some());
    assert!(!ed.in_gesture());
    assert!(!ed.can_mutate());
    assert!(!ed.set_field(&Owner::Component("Position".into()), "pos", vec2(1,2)));
}

#[test]
fn loading_arena_scene_fits_the_confirmed_new_frame() {
    let host = ArenaHost::start(false);
    let mut ed = Editor::attach(&host.url,None).unwrap(); ed.sync();
    let path = temp_path("arena-new-layout.scene.yaml");
    std::fs::write(&path, SCENE.replace("[-300,0]", "[1100,0]").replace("[300,0]", "[1700,0]")).unwrap();
    assert!(ed.open_path(&path)); ed.sync();
    assert_eq!(ed.camera.center, [1400.0,0.0]);
    assert!(ed.bodies().iter().all(|b| b.pos[0] >= 1100.0));
}

/// Opt-in real-window proof. Ordinary test runs (including Windows) skip it;
/// Linux CI requires it under Xvfb with the installed Mesa Vulkan driver.
#[test]
fn arena_native_window_smoke() {
    if std::env::var("ORR_REQUIRE_NATIVE_EDITOR").as_deref() != Ok("1") {
        eprintln!("SKIP: native Arena window smoke; set ORR_REQUIRE_NATIVE_EDITOR=1 under a working X11 display");
        return;
    }
    assert!(std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty()), "required native editor smoke needs DISPLAY (run under xvfb-run)");
    let out = std::env::var_os("ORR_NATIVE_EDITOR_ARTIFACT_DIR").map(std::path::PathBuf::from)
        .unwrap_or_else(|| temp_path("arena-native-window"));
    std::fs::create_dir_all(&out).unwrap();
    let host = ArenaHost::start(false);
    let mut agent = host.client();
    // Author through normal ERP, rather than passing a specially rendered mock frame.
    agent.call("scene.load", json!({"text":include_str!("../../../scenes/arena_blank.scene.yaml")})).unwrap();
    for (name, x, slot) in [("hero", -300, 0), ("target", 300, 1)] {
        agent.call("world.spawn", json!({"name":name,"components":{"Position":{"pos":[x,0]},"PlayerTag":{"slot":slot}}})).unwrap();
    }
    let edit = agent.call("sim.state", J::Null).unwrap();
    capture_native_arena(&host.url, &out, "edit");
    assert_eq!(agent.call("sim.state", J::Null).unwrap()["checksum"], edit["checksum"], "native GUI changed the edit frame");
    std::fs::write(out.join("edit-state.json"), serde_json::to_vec_pretty(&edit).unwrap()).unwrap();

    agent.call("sim.start", json!({})).unwrap();
    input(&mut agent, 0, true);
    agent.call("sim.step", json!({"n":1})).unwrap();
    input(&mut agent, 0, false);
    agent.call("sim.step", json!({"n":19})).unwrap();
    let score = agent.call("world.singleton.get", json!({"name":"Score"})).unwrap();
    assert_eq!(score["value"]["kills"][0], 1, "ordinary Arena fire must produce the displayed score");
    let play = agent.call("sim.state", J::Null).unwrap();
    assert_eq!(play["head_tick"], 20);
    assert_eq!(play["playing"], false);
    capture_native_arena(&host.url, &out, "score");
    assert_eq!(agent.call("sim.state", J::Null).unwrap()["checksum"], play["checksum"], "native GUI changed the paused play frame");
    std::fs::write(out.join("play-state.json"), serde_json::to_vec_pretty(&play).unwrap()).unwrap();
    std::fs::write(out.join("score.json"), serde_json::to_vec_pretty(&score).unwrap()).unwrap();
    agent.call("sim.stop", J::Null).unwrap();
    assert_eq!(agent.call("sim.state", J::Null).unwrap()["checksum"], edit["checksum"], "Stop changed the document");
    let verify = agent.call("verify.self", json!({"inputs":{"kind":"last_play"},"checks":["recording_matches","score_0.final == 1"]})).unwrap();
    assert_eq!(verify["passed"], true, "{verify}");
    std::fs::write(out.join("verified.json"), serde_json::to_vec_pretty(&verify).unwrap()).unwrap();
    eprintln!("PASS: real native Arena edit/score windows, pixels, host checksums and replay verified in {}", out.display());
}

fn capture_native_arena(url: &str, out: &std::path::Path, name: &str) {
    use std::{process::{Command, Stdio}, time::{Duration, Instant}};
    let path = out.join(format!("{name}.png"));
    assert!(!path.exists(), "refusing stale native screenshot {}", path.display());
    let log_path = out.join(format!("{name}.log"));
    let log = std::fs::File::create(&log_path).unwrap();
    // RAII cleanup also covers a panic before the normal bounded wait finishes.
    struct Window(std::process::Child);
    impl Drop for Window {
        fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
    }
    let mut child = Window(Command::new(env!("CARGO_BIN_EXE_orr_editor"))
        .args(["--connect",url,"--select","hero","--frames","30","--screenshot-settle","--size","1200x850","--screenshot"])
        .arg(&path).stdin(Stdio::null()).stdout(Stdio::from(log.try_clone().unwrap())).stderr(Stdio::from(log))
        .spawn().expect("start the normal orr_editor binary"));
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(status) = child.0.try_wait().expect("poll native editor") {
            assert!(status.success(), "native editor failed: {status}\n{}", std::fs::read_to_string(&log_path).unwrap_or_default());
            break;
        }
        assert!(Instant::now() < deadline, "native editor timed out\n{}", std::fs::read_to_string(&log_path).unwrap_or_default());
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut reader = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).expect("native window PNG exists"))).read_info().unwrap();
    let mut bytes = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut bytes).expect("decode native PNG");
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    assert!(info.width >= 1000 && info.height >= 700, "native window dimensions {}x{}",info.width,info.height);
    // sRGB encoding of the existing player-0/player-1/selection shader colors.
    let colors = [[137u8,203,255], [255,196,124], [255,237,124]];
    let mut counts = [0usize;3];
    // Exclude menus, hierarchy and inspector: only pixels from the viewport count.
    for y in 24..info.height-140 {
        for x in 240..info.width-345 {
            let pixel = &bytes[((y*info.width+x)*4) as usize..][..3];
            for (i,color) in colors.iter().enumerate() {
                if pixel.iter().zip(color).all(|(actual,want)| actual.abs_diff(*want) <= 8) { counts[i]+=1; }
            }
        }
    }
    assert!(counts[0] > 100 && counts[1] > 100 && counts[2] > 25,
        "native {name} viewport must show both Arena players and hero selection, got pixel counts {counts:?}");
}
