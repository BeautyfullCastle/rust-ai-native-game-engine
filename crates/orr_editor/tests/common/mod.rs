#![allow(dead_code)]

use std::sync::{Mutex, MutexGuard};

use orr_edit::Target;
use orr_editor::editor::{default_scene_path, Editor};
use orr_fp::{FPVec2, FP};
use orr_reflect::{Guid, Value};
use orr_sample::physics_game::{PhysConfig, PhysGame, SceneMode};
use orr_session::{ControlOp, PlaySession};

pub const BODY: &str = "orr_physics::Body";

pub fn demo_editor() -> Editor {
    Editor::open(&default_scene_path()).expect("open the demo scene")
}

pub fn guid_named(ed: &Editor, name: &str) -> Guid {
    ed.view()
        .entities()
        .into_iter()
        .find(|e| e.name.as_deref() == Some(name))
        .and_then(|e| e.guid)
        .unwrap_or_else(|| panic!("no entity named {name}"))
}

pub fn target_named(ed: &Editor, name: &str) -> Target {
    Target::Guid(guid_named(ed, name))
}

pub fn fixed(n: i32) -> Value {
    Value::Fixed(FP::from_int(n))
}

pub fn vec2(x: i32, y: i32) -> Value {
    Value::Vec2(FPVec2::new(FP::from_int(x), FP::from_int(y)))
}

/// Checksums of ticks `from..=to` recorded by the play session.
pub fn checksums(ed: &Editor, from: u64, to: u64) -> Vec<u64> {
    let s = ed.play_controller().expect("play mode").session();
    (from..=to).map(|t| s.checksum_at(t).unwrap_or_else(|| panic!("no checksum for tick {t}"))).collect()
}

/// Opens a recording as a viewer, seeks to its end and returns `(last tick, checksum there)`.
pub fn replay_end(bytes: &[u8]) -> (u64, u64) {
    let cfg = PhysConfig::new(40, SceneMode::Rain);
    let mut viewer = PlaySession::<PhysGame>::open_replay(bytes, cfg, 0).expect("open replay");
    let last = viewer.last_tick();
    viewer.control(ControlOp::Seek(last));
    assert_eq!(viewer.head_tick(), last);
    (last, viewer.frame().checksum())
}

/// GPU tests run one at a time (several devices on one software adapter hung on some machines).
static SERIAL: Mutex<()> = Mutex::new(());

/// A headless GPU for the test, or `None` (after printing SKIP) when there is no adapter.
/// With `ORR_REQUIRE_GPU=1` a missing adapter fails the test instead.
pub fn gpu() -> Option<(MutexGuard<'static, ()>, orr_rhi::Wgpu)> {
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    match orr_rhi::Wgpu::headless(orr_rhi::WgpuOptions::default()) {
        Ok(g) => {
            eprintln!("gpu test adapter: {} (software: {})", orr_rhi::Rhi::adapter_name(&g), g.is_software());
            Some((guard, g))
        }
        Err(e) => {
            assert!(std::env::var_os("ORR_REQUIRE_GPU").is_none(), "ORR_REQUIRE_GPU is set but no adapter: {e}");
            eprintln!("SKIP: no GPU adapter ({e})");
            None
        }
    }
}

pub fn temp_path(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("orr_editor_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}
