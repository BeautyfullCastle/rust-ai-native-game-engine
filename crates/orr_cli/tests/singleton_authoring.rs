//! Real CLI singleton authoring against an authenticated loopback host.
mod common;
use common::*;
use serde_json::Value as J;

#[test]
fn sset_multiple_fields_share_history_and_save_undo_redo() {
    let host = TestHost::start();
    let before = host.yaml();
    let r = host.orr(&["sset", "Scene.spawn_batch=3", "Scene.height=42.5", "--json"]);
    assert_eq!(r.code, 0, "{}", r.err);
    let result = r.json();
    assert_eq!(result["applied"].as_array().unwrap().len(), 2);
    assert_eq!(result["history"]["origin"], "agent:claude");
    assert_eq!(host.history().len(), 1);
    assert_eq!(host.orr(&["get", "@Scene.spawn_batch"]).out.trim(), "3");
    assert_eq!(host.orr(&["get", "@Scene.height"]).out.trim(), "42.5");
    let after = host.yaml();
    assert_ne!(after, before);
    let saved = host.orr(&["save"]);
    assert_eq!(saved.code, 0);
    assert_eq!(saved.out, after);
    assert_eq!(host.orr(&["undo"]).code, 0);
    assert_eq!(host.yaml(), before);
    assert_eq!(host.orr(&["redo"]).code, 0);
    assert_eq!(host.yaml(), after);
}

#[test]
fn sset_rejects_invalid_batches_atomically() {
    let host = TestHost::start();
    let before = host.yaml();
    for args in [
        vec!["sset", "Scene.spawn_batch=3", "Scene.no_such_field=1", "--json"],
        vec!["sset", "Scene.spawn_batch=3", "Scene.height=[]", "--json"],
        vec!["sset", "Body.angle=1", "--json"],
        vec!["sset", "Unknown.value=1", "--json"],
    ] {
        let r = host.orr(&args);
        assert_eq!(r.code, 1, "{args:?}: {}", r.err);
        assert_eq!(serde_json::from_str::<J>(&r.err).unwrap()["exit_code"], 1);
        assert_eq!(host.yaml(), before);
        assert!(host.history().is_empty());
    }
    // A failed transaction must not prevent the next permitted edit.
    assert_eq!(host.orr(&["sset", "Scene.spawn_batch=4"]).code, 0);
    assert_eq!(host.history().len(), 1);
}

#[test]
fn sset_preserves_permissions_and_play_lock() {
    let host = TestHost::start();
    let before = host.yaml();
    let denied = host.orr_as(Some("reader-tok"), &["sset", "Scene.spawn_batch=3", "--json"]);
    assert_eq!(denied.code, 1, "{}", denied.err);
    assert_eq!(serde_json::from_str::<J>(&denied.err).unwrap()["exit_code"], 1);
    assert_eq!(host.yaml(), before);
    assert!(host.history().is_empty());
    // Direct edits require scene_edit, not proposal approval.
    let allowed = host.orr_as(Some("limited-tok"), &["sset", "Scene.spawn_batch=3"]);
    assert_eq!(allowed.code, 0, "{}", allowed.err);
    let edited = host.yaml();
    assert_eq!(host.orr(&["sim", "start"]).code, 0);
    let playing = host.orr(&["sset", "Scene.spawn_batch=4", "--json"]);
    assert_eq!(playing.code, 1, "{}", playing.err);
    assert_eq!(host.history().len(), 1);
    assert_eq!(host.orr(&["sim", "stop"]).code, 0);
    assert_eq!(host.yaml(), edited);
}

#[test]
fn sset_help_and_usage_are_available_without_a_host() {
    for args in [vec!["help", "sset"], vec!["sset", "--help"]] {
        let r = finish(command("ws://127.0.0.1:1", None, &args).output().unwrap());
        assert_eq!(r.code, 0, "{}", r.err);
        assert!(r.out.contains("Singleton.path") && r.out.contains("one undo"));
    }
    for args in [vec!["sset", "--json"], vec!["sset", "not-an-assignment", "--json"]] {
        let r = finish(command("ws://127.0.0.1:1", None, &args).output().unwrap());
        assert_eq!(r.code, 2, "{}", r.err);
        assert_eq!(serde_json::from_str::<J>(&r.err).unwrap()["exit_code"], 2);
    }
}

#[test]
fn sset_nested_index_and_no_op_use_existing_reflection_contract() {
    let host = TestHost::start_stormglass();
    let before = host.yaml();
    let r = host.orr(&["sset", "CoopDefense::WaveCounts.pair_counts[0]=5", "CoopDefense::WaveComposition.regular_kinds[0][1]=3", "--json"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(host.orr(&["get", "@WaveCounts.pair_counts[0]"]).out.trim(), "5");
    assert_eq!(host.orr(&["get", "@WaveComposition.regular_kinds[0][1]"]).out.trim(), "3");
    let after = host.yaml();
    assert_ne!(after, before);
    assert_eq!(host.history().len(), 1);
    let again = host.orr(&["sset", "WaveCounts.pair_counts[0]=5", "--json"]);
    assert_eq!(again.code, 0, "{}", again.err);
    assert!(again.json()["history"].is_null());
    assert_eq!(host.history().len(), 1);
    for path in ["WaveCounts.pair_counts[10]=1", "WaveCounts.pair_counts[=1", "WaveCounts.pair_counts[abc]=1", "WaveCounts.pair_counts[0]=29"] {
        let invalid = host.orr(&["sset", "WaveCounts.pair_counts[1]=2", path]);
        assert_eq!(invalid.code, 1, "{}", invalid.err);
        assert_eq!(host.yaml(), after);
        assert_eq!(host.history().len(), 1);
    }
    assert_eq!(host.orr(&["undo"]).code, 0);
    assert_eq!(host.yaml(), before);
    assert_eq!(host.orr(&["redo"]).code, 0);
    assert_eq!(host.yaml(), after);
    assert_eq!(host.orr(&["save"]).out, after);
    let staged = host.orr(&["propose", "indexed wave edit", "sset", "WaveCounts.pair_counts[0]=6", "--json"]);
    assert_eq!(staged.code, 0, "{}", staged.err);
    let id = staged.json()["id"].as_str().unwrap().to_owned();
    assert_eq!(host.yaml(), after, "proposal must remain private");
    assert_eq!(host.orr(&["reject", &id]).code, 0);
    assert_eq!(host.yaml(), after);
}
