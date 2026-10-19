//! Explicit second half of authoring-App -> saved bytes -> production Keys proof.
//! This consumer never generates a substitute fixture and never opens a window.
use super::*;

#[test]
#[ignore = "requires the exact file produced by editor authoring_app_exports_standalone_acceptance_map"]
fn authoring_app_saved_bytes_drive_production_keys() {
    let path = std::path::PathBuf::from(
        std::env::var_os("ORR_COLLECT_ACCEPTANCE_MAP")
            .expect("run the actual authoring App producer and set ORR_COLLECT_ACCEPTANCE_MAP"),
    );
    let bytes = std::fs::read(&path).expect("authoring App saved map must exist");
    let loaded = ActionMap::load_file(&path).unwrap();
    assert_eq!(
        loaded,
        remap(),
        "producer must save the accepted JLIK/Enter/Q map"
    );
    let trace = play_trace(
        Keys::with_bindings(CollectBindings::load_file(&path).unwrap()),
        KeyCode::KeyL,
        KeyCode::Enter,
        KeyCode::KeyQ,
    );
    let default_trace = play_trace(
        Keys::default(),
        KeyCode::KeyD,
        KeyCode::Space,
        KeyCode::Escape,
    );
    assert_eq!(
        trace, default_trace,
        "production Keys and Bridge trajectories must match"
    );
    let mut keys = Keys::with_bindings(CollectBindings::load_file(&path).unwrap());
    let mut bridge = test_bridge();
    keys.focus(true);
    for old in [KeyCode::KeyD, KeyCode::Space] {
        assert!(!keys.keyboard_event(old, true, false, false, false));
        let sample = deliver(&mut keys, &mut bridge, Duration::from_millis(17));
        assert_eq!(sample.input, CollectInput::default());
        assert_eq!(sample.pending_restarts, 0);
        assert!(!keys.keyboard_event(old, false, false, false, false));
    }
    for _ in 0..12 {
        tap(&mut keys, KeyCode::Enter);
    }
    for _ in 0..3 {
        let sample = deliver(&mut keys, &mut bridge, Duration::ZERO);
        assert_eq!(
            sample.pending_restarts, 8,
            "zero ticks cannot acknowledge queued taps"
        );
    }
    for index in 0..15 {
        let sample = deliver(&mut keys, &mut bridge, Duration::from_millis(17));
        assert_eq!(sample.run.restart_held, u32::from(index % 2 == 0));
    }
    assert_eq!(keys.pending_restarts, 0);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let observed_hash = std::env::var("ORR_COLLECT_ACCEPTANCE_SHA256")
        .expect("gate must pass the producer SHA256 observation");
    assert!(observed_hash.len() == 64 && observed_hash.bytes().all(|b| b.is_ascii_hexdigit()));
    let receipt_path = path.with_extension("receipt.json");
    assert!(!receipt_path.exists(), "preserve prior acceptance receipts");
    let receipt = serde_json::json!({
        "input_map_path": path.canonicalize().unwrap().display().to_string(),
        "input_map_sha256": observed_hash,
        "sha256_source": "gate producer observation; gate independently verifies after consumer",
        "production_keys": true,
        "real_bridge": true,
        "map_bytes_unchanged": true,
    });
    std::fs::write(&receipt_path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
    eprintln!(
        "production Keys consumed unchanged {} ({} bytes)",
        path.display(),
        bytes.len()
    );
}
