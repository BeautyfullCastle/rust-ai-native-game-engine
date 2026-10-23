#!/usr/bin/env python3
"""Fail-closed acceptance for main EditorApp session-only Collect controls.

CPU authoring is a producer, private production Keys is a separate consumer.
The explicitly ignored GPU test is never counted as covered by ordinary tests.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import runpy
import struct

HELPERS = runpy.run_path(str(Path(__file__).with_name("check-authoring-controls.py")))
FEATURES = "collect-ui,collect-input-map-editor"
EDITOR = ["cargo", "test", "--locked", "-p", "orr_editor", "--no-default-features",
          "--features", FEATURES, "--lib"]
SAMPLE = ["cargo", "test", "--locked", "-p", "orr_sample", "--no-default-features",
          "--features", "collect-ui,input-actions", "--lib"]
RUNTIME = EDITOR[:-1] + ["--test", "collect_controls_runtime"]
UI = EDITOR[:-1] + ["--test", "collect_controls_ui"]
PRODUCER = "authoring_app_exports_standalone_acceptance_map"
GPU = "authored_map_drives_actual_editor_gpu_victory_restart_reopen"
CONSUMER = "collect_app::input_runtime_tests::controls_acceptance_tests::authoring_app_saved_bytes_drive_production_keys"

# Ordinary production-input coverage; the saved-map consumer runs explicitly in contracts.
INPUT_RUNTIME_CASES = {
    'collect_app::input_runtime_tests::invalid_map_is_rejected_before_project_open_and_direct_headless_options_are_guarded',
    'collect_app::input_runtime_tests::map_reload_reproduces_production_bridge_trace_without_writing_map',
    'collect_app::input_runtime_tests::remapped_aliases_are_independent_and_opposing_axes_cancel',
    'collect_app::input_runtime_tests::remapped_focus_synthetic_capture_and_quit_keep_existing_policy',
    'collect_app::input_runtime_tests::remapped_keys_match_default_collect_win_restart_and_quit_bridge_trace',
    'collect_app::input_runtime_tests::remapped_old_keys_are_inert_in_production_policy_and_bridge',
    'collect_app::input_runtime_tests::remapped_queue_cap_repeat_subtick_and_neutral_ack_use_real_bridge',
    'collect_app::input_runtime_tests::remapped_restart_aliases_retain_queued_edges_and_held_state_after_ack',
}


def validate_exact(output, expected):
    """Reject zero, duplicate, ignored, failed, extra or missing test cases."""
    expected = set(expected)
    if not expected:
        raise ValueError("Empty expected inventory is not coverage")
    output = output.replace("\r\n", "\n")
    cases = HELPERS["CASE"].findall(output)
    summaries = HELPERS["SUMMARY"].findall(output)
    if (len(summaries) != 1 or tuple(map(int, summaries[0][:4])) != (len(expected), 0, 0, 0)
            or re.findall(r"^running (\d+) tests?$", output, re.M) != [str(len(expected))]
            or len(cases) != len(expected) or {n for n, _ in cases} != expected
            or any(status != "ok" for _, status in cases)):
        raise ValueError("Expected exactly these executed passing cases: " + repr(sorted(expected)))


def validate_input_runtime_regression(output):
    """Pin eight passing regressions and the separately executed saved-map hook."""
    output = output.replace("\r\n", "\n")
    expected = {name: "ok" for name in INPUT_RUNTIME_CASES}
    expected[CONSUMER] = "ignored"
    cases = re.findall(r"^test (\S+) \.\.\. (ok|FAILED|ignored)(?:, .*)?$", output, re.M)
    summaries = HELPERS["SUMMARY"].findall(output)
    if (output.count("test result:") != 1 or len(summaries) != 1
            or tuple(map(int, summaries[0][:4])) != (8, 0, 1, 0)
            or re.findall(r"^running (\d+) tests?$", output, re.M) != ["9"]
            or len(re.findall(r"^test (?!result:)", output, re.M)) != len(expected)
            or len(cases) != len(expected) or dict(cases) != expected):
        raise ValueError("Expected eight exact passing input regressions and only the saved-map hook ignored")


def run_case(base, name, evidence, label, ignored=False):
    args = [name, "--", "--exact", "--format=pretty", "--color=never", "--test-threads=1"]
    if ignored:
        args += ["--ignored"]
    output = HELPERS["run_logged"](base + args, evidence / (label + ".log"))
    validate_exact(output, {name})


def sha(path):
    if not path.is_file() or path.is_symlink():
        raise ValueError("Required producer file must be an ordinary file")
    return hashlib.sha256(path.read_bytes()).hexdigest()


def validate_handoff(receipt, path, before):
    if sha(path) != before or receipt.get("input_map_sha256") != before:
        raise ValueError("Consumer must prove unchanged producer bytes with SHA-256")
    received = receipt.get("input_map_path")
    # Rust canonicalize uses extended-length paths on Windows; compare actual
    # file identity instead of rejecting the equivalent \\?\ spelling.
    if not isinstance(received, str) or not Path(received).is_absolute():
        raise ValueError("Consumer read a different producer path")
    try:
        same = os.path.samefile(received, path)
    except OSError:
        same = False
    if not same:
        raise ValueError("Consumer read a different producer file")
    for key in ("production_keys", "real_bridge", "map_bytes_unchanged"):
        if receipt.get(key) is not True:
            raise ValueError("Missing runtime consumer proof: " + key)


def contracts(evidence):
    path = evidence / "editor-authored-controls.json"
    if path.exists() or path.is_symlink():
        raise ValueError("Producer destination must be fresh; stale evidence is forbidden")
    os.environ["ORR_COLLECT_ACCEPTANCE_MAP"] = str(path.resolve())
    run_case(RUNTIME, PRODUCER, evidence, "cpu-producer", ignored=True)
    before = sha(path)
    (evidence / "producer-sha256.txt").write_text(before + "\n", encoding="utf-8")
    os.environ["ORR_COLLECT_ACCEPTANCE_SHA256"] = before
    run_case(SAMPLE, CONSUMER, evidence, "production-keys-consumer", ignored=True)
    receipt_path = path.with_suffix(".receipt.json")
    validate_handoff(json.loads(receipt_path.read_text(encoding="utf-8")), path, before)


def boundaries(evidence):
    state_names = {
        "all_supported_keys_have_distinct_physical_mapping",
        "aliases_are_ored_and_opposing_directions_cancel",
        "fresh_restart_aliases_are_ordered_without_repeat_or_hold_retrigger",
        "physical_identity_does_not_leak_logical_defaults",
        "quit_and_escape_release_and_cancel_frame_restarts",
        "disabled_frames_and_reset_quarantine_held_keys_until_release",
        "epoch_replacement_resets_map_but_same_epoch_authoring_preserves_it",
        "invalid_missing_and_escape_configs_retain_accepted_map",
        "pre_pump_release_peek_is_non_consuming_and_obeys_quarantine",
        "physical_tab_is_action_when_bound_and_release_when_unbound",
        "logical_escape_tab_do_not_override_physical_action_and_missing_physical_falls_back",
    }
    suite = "app::collect_controls::tests"
    output = HELPERS["run_logged"](EDITOR + [suite + "::", "--", "--format=pretty", "--color=never", "--test-threads=1"], evidence / "state-cpu.log")
    validate_exact(output, {suite + "::" + name for name in state_names})
    for name in (
        "authored_map_main_app_uses_physical_keys_and_preserves_project_bytes",
        "aliases_opposition_and_focus_dialog_stop_reclaim_require_fresh_press",
        "same_path_scene_replacement_discards_session_map",
        "mapped_tab_retains_viewport_control_and_physical_escape_always_releases",
        "ordered_restart_aliases_cross_authoritative_ticks",
        "same_frame_restart_then_quit_discards_undelivered_intent",
    ):
        run_case(RUNTIME, name, evidence, name)
    # Canonical preexisting document suite includes two Unix-only filesystem cases.
    base = EDITOR[:-1] + ["--test", "collect_input_map_authoring_ui"]
    suite = "collect_input_map_authoring::tests"
    output = HELPERS["run_logged"](base + [suite + "::", "--", "--format=pretty", "--color=never", "--test-threads=1"], evidence / "document-regression.log")
    HELPERS["validate_tests"](output, suite, 16 if os.name == "nt" else 18)

    for label, features in (("default", None), ("ui-only", "collect-ui"),
                            ("map-only", "collect-input-map-editor"), ("both", FEATURES)):
        args = ["cargo", "check", "--locked", "-p", "orr_editor", "--no-default-features", "--lib"]
        if features:
            args += ["--features", features]
        HELPERS["run_logged"](args, evidence / (label + "-check.log"))
    HELPERS["run_logged"](["cargo", "clippy", "--locked", "-p", "orr_editor", "--no-default-features",
                           "--features", FEATURES, "--lib", "--test", "collect_controls_runtime", "--test", "collect_controls_ui",
                           "--", "-D", "warnings"], evidence / "both-strict.log")
    # Existing unrelated control contracts remain mandatory, not inferred from compilation.
    for name in (
        "editor::input::collect_tests::collect_queued_edges_require_observed_ticks_and_neutral_rearm",
        "editor::input::collect_tests::collect_stop_error_and_reclaim_cancel_edges_and_prime_new_epoch",
        "editor::input::collect_tests::collect_reclaim_does_not_turn_old_held_space_into_a_new_press",
    ):
        run_case(EDITOR, name, evidence, name.rsplit("::", 1)[-1])
    room = ["cargo", "test", "--locked", "-p", "orr_editor", "--no-default-features", "--features", "room-project", "--lib"]
    run_case(room, "editor::input::room_input_tests::room_claim_and_reclaim_require_neutral_interact_before_fresh_press",
             evidence, "room-lease-regression")
    arena = ["cargo", "test", "--locked", "-p", "orr_editor", "--no-default-features",
             "--features", "input-map-editor", "--test", "input_map_authoring"]
    arena_names = ["seven_action_widgets_save_reopen_and_exact_undo_redo",
                   "widget_conflicts_pending_drafts_and_empty_actions_preserve_contract",
                   "malformed_limits_and_failed_open_leave_document_and_redo_unchanged",
                   "authored_saved_actions_drive_production_controls_and_real_frame_trace"]
    if os.name != "nt":
        arena_names.append("symlinks_and_existing_destinations_are_rejected_without_replacement")
    for name in arena_names:
        run_case(arena, name, evidence, "arena-" + name)



def gpu(evidence):
    captures = evidence / "software-gpu"
    captures.mkdir(exist_ok=False)
    os.environ["ORR_COLLECT_CONTROLS_EVIDENCE"] = str(captures.resolve())
    run_case(UI, GPU, evidence, "main-editor-software-gpu", ignored=True)
    validate_gpu(captures)


def png_size(path, expected):
    raw = path.read_bytes()
    if (len(raw) < 24 or raw[:8] != b"\x89PNG\r\n\x1a\n" or raw[12:16] != b"IHDR"
            or struct.unpack(">II", raw[16:24]) != tuple(expected)):
        raise ValueError("Missing or wrong-sized capture: " + path.name)


def validate_gpu(captures):
    receipt = json.loads((captures / "collect-controls-acceptance.json").read_text(encoding="utf-8"))
    if type(receipt.get("schema")) is not int or receipt["schema"] != 1:
        raise ValueError("Unknown main EditorApp receipt schema")
    if ("llvmpipe" not in str(receipt.get("adapter", "")).lower()
            or receipt.get("backend") != "Vulkan" or receipt.get("device_type") != "Cpu"):
        raise ValueError("Explicit software Vulkan adapter required")
    for key in ("main_editor_app", "standalone_authoring_app", "physical_egui_events",
                "mapped_victory_restart", "normalized_restart_viewport_equal",
                "fresh_app_explicit_reload", "configuration_bytes_unchanged",
                "authoritative_victory", "restart_acknowledged"):
        if receipt.get(key) is not True:
            raise ValueError("Missing main EditorApp proof: " + key)
    if receipt.get("native_os_keyboard") is not False:
        raise ValueError("Software GPU scope cannot claim native input")
    if receipt.get("window_pixels") != [1600, 1000] or receipt.get("authoring_pixels") != [720, 650]:
        raise ValueError("Unexpected editor or authoring dimensions")
    png_size(captures / "collect-controls-authoring.png", [720, 650])
    for name, phase, score in (("mapped", 0, 0), ("victory", 1, 2),
                               ("restart", 0, 0), ("reopened-victory", 1, 2)):
        stem = "collect-controls-" + name
        png_size(captures / (stem + ".png"), [1600, 1000])
        frame = json.loads((captures / (stem + ".json")).read_text())
        rect = frame.get("viewport")
        if (not isinstance(rect, list) or len(rect) != 4
                or any(type(x) is not int for x in rect) or min(rect) < 0
                or min(rect[2:]) <= 0 or rect[0] + rect[2] > 1600 or rect[1] + rect[3] > 1000):
            raise ValueError("Invalid bounded viewport capture")
        # Fixed 1600x1000 harness: omit only the top 24px PLAY/global-tick
        # overlay from the viewport. The complete window remains mandatory.
        if rect != [230, 90, 1030, 792]:
            raise ValueError("Unexpected normalized scene rectangle")
        png_size(captures / (stem + "-viewport.png"), rect[2:])
        if (frame.get("verified_tick") != frame.get("tick")
                or frame.get("verified_checksum") != frame.get("checksum")
                or frame.get("phase") != phase or frame.get("score") != score
                or frame.get("active_actors") != 4 - score
                or type(frame.get("tick")) is not int or frame["tick"] < 0
                or re.fullmatch(r"[0-9a-f]{16}", str(frame.get("checksum", ""))) is None):
            raise ValueError("Incorrect game phase/score/checksum capture: " + name)
    if ((captures / "collect-controls-mapped-viewport.png").read_bytes()
            != (captures / "collect-controls-restart-viewport.png").read_bytes()):
        raise ValueError("Restart must restore exactly the retained normalized scene pixels")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("contracts", "gpu"))
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    evidence = args.evidence.resolve()
    if args.mode == "contracts":
        evidence.mkdir(parents=True, exist_ok=False)
        contracts(evidence)
        boundaries(evidence)
        (evidence / "contracts-complete.json").write_text(json.dumps({"schema": 1, "contracts_complete": True}), encoding="utf-8")
    else:
        if not evidence.is_dir():
            raise ValueError("Run contracts first and preserve their evidence")
        if json.loads((evidence / "contracts-complete.json").read_text()) != {"schema": 1, "contracts_complete": True}:
            raise ValueError("All CPU contracts and boundaries must complete before GPU")
        # GPU cannot be passed independently of the actual CPU producer/consumer proof.
        path = evidence / "editor-authored-controls.json"
        before = (evidence / "producer-sha256.txt").read_text().strip()
        validate_handoff(json.loads(path.with_suffix(".receipt.json").read_text()), path, before)
        gpu(evidence)


if __name__ == "__main__":
    main()
