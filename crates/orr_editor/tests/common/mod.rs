#![allow(dead_code)]
#![allow(clippy::disallowed_types)]

use std::sync::{Mutex, MutexGuard};

use orr_editor::editor::{default_scene_path, Editor};
use orr_editor::{HostSpec, Target};
use orr_fp::{FPVec2, FP};
use orr_reflect::{Guid, Value};
use orr_remote::json::value_to_json;
use orr_sample::physics_game::{PhysConfig, PhysGame, SceneMode};
use orr_session::{ControlOp, PlaySession};
use serde_json::json;

pub const BODY: &str = "orr_physics::Body";
pub const COLLIDER: &str = "orr_physics::Collider";

/// An editor on its own host thread, on the demo scene, brought up to date.
pub fn demo_editor() -> Editor {
    let mut ed = Editor::open(&default_scene_path()).expect("open the demo scene");
    ed.sync();
    ed
}

/// An editor whose host thread has the `debug.panic` hook.
pub fn crashable_editor() -> Editor {
    let spec = HostSpec::Local { scene: default_scene_path(), listen: None, debug_hooks: true };
    let mut ed = Editor::start(&spec).expect("start");
    ed.sync();
    ed
}

pub fn guid_named(ed: &Editor, name: &str) -> Guid {
    ed.rows()
        .iter()
        .find(|e| e.name.as_deref() == Some(name))
        .and_then(|e| e.guid.clone())
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

/// `[x, y]` of a `Vec2` value, in world units (view layer).
pub fn xy(v: &Value) -> [f32; 2] {
    let Value::Vec2(p) = v else { panic!("a vec2, got {v:?}") };
    [orr_view::fp_to_f32(p.x), orr_view::fp_to_f32(p.y)]
}

/// One field of an entity, read from the host.
pub fn field(ed: &mut Editor, t: &Target, component: &str, path: &str) -> Value {
    ed.field_of(t, component, path).unwrap_or_else(|e| panic!("{component}.{path}: {e}"))
}

/// `fixed` as the JSON ERP takes.
pub fn fixed_json(n: i32) -> serde_json::Value {
    value_to_json(&fixed(n))
}

/// Checksums of ticks `from..=to` recorded by the play session (asked of the host).
pub fn checksums(ed: &mut Editor, from: u64, to: u64) -> Vec<u64> {
    (from..=to)
        .map(|t| {
            let r = ed.host_call("sim.checksum", json!({"tick": t})).unwrap_or_else(|e| panic!("checksum of tick {t}: {e}"));
            orr_remote::wire::parse_checksum(&r["checksum"]).expect("checksum")
        })
        .collect()
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

/// The yaml text of the host's scene (what `scene.save` without `write` returns).
pub fn scene_text(ed: &mut Editor) -> String {
    ed.host_call("scene.save", json!({})).expect("scene.save")["text"].as_str().expect("text").to_string()
}

/// The checksum of the host's scene document.
pub fn doc_checksum(ed: &mut Editor) -> u64 {
    let r = ed.host_call("sim.state", json!({})).expect("sim.state");
    orr_remote::wire::parse_checksum(&r["doc_checksum"]).expect("doc_checksum")
}

pub mod arena;
