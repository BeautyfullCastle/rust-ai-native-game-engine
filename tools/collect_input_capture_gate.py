#!/usr/bin/env python3
"""Fail-closed physical capture acceptance; retain the earlier integration gates.

Run from the repository root. CPU and GPU stages require a fresh evidence tree;
ignored producers/consumers run explicitly, never as inferred ordinary coverage.
"""
import argparse
import json
import math
import struct
import zlib
import os
from pathlib import Path
import re

import editor_collect_controls_gate as previous

HELPERS = previous.HELPERS
BASE = ["cargo", "test", "--locked", "-p", "orr_editor", "--no-default-features",
        "--features", "collect-input-map-editor"]
CPU = BASE + ["--test", "collect_input_capture"]
GPU_BASE = BASE + ["--test", "collect_input_capture_ui"]
GPU_TEST = "collect_capture_widgets_have_real_gpu_pixels"
DOCUMENT_NAMES = {
    "initial_defaults_are_clean_exact_collect_contract",
    "each_action_applies_one_complete_undoable_map",
    "draft_conflicts_preserve_document_history_selection_and_draft",
    "complete_map_admission_rejects_wrong_schema_devices_and_conflicts_atomically",
    "drafts_enforce_eight_keyboard_choices_without_early_conflict_validation",
    "all_six_actions_may_be_unbound_including_restart_and_quit",
    "undo_redo_history_is_exact_and_bounded_to_32_edits",
    "no_op_apply_and_defaults_preserve_history_and_redo",
    "pending_draft_blocks_navigation_replacement_and_open_until_explicit_resolution",
    "invalid_selection_is_atomic_and_import_order_preserves_semantic_selection",
    "save_reopen_roundtrip_uses_existing_format_and_real_collect_loader",
    "save_accepted_state_preserves_even_invalid_unapplied_draft_and_redo",
    "failed_open_preserves_all_state_for_malformed_oversized_missing_and_directory_paths",
    "open_rejects_shared_format_mouse_map_and_semantically_wrong_actions",
    "shared_loader_accepts_exact_64_kib_boundary_and_refuses_one_more_byte",
    "save_as_never_overwrites_existing_destinations_and_failed_save_preserves_state",
}
UNIX_DOCUMENT_NAMES = {
    "direct_and_dangling_symlinks_cannot_be_opened_or_overwritten",
    "preexisting_fifo_and_device_are_refused_without_opening_or_changing_state",
}
# Explicit names are updated only after reviewing the compiled target inventory.
CAPTURE_NAMES = {
    'all_supported_physical_keys_capture_and_escape_cancels',
    'physical_identity_wins_and_missing_physical_is_refused',
    'unsupported_and_modifier_chords_never_bind',
    'held_activation_release_requires_later_frame_and_fresh_press',
    'repeat_and_duplicate_down_need_release_before_capture',
    'focus_loss_even_regained_in_same_frame_cancels_before_candidate',
    'explicit_cancel_and_context_changes_preserve_no_capture',
    'text_ime_paste_and_mouse_never_bind_and_consumption_is_sorted_unique',
    'live_validation_matches_apply_for_same_and_cross_action_conflicts',
    'invalid_draft_save_and_guards_preserve_accepted_map_and_redo',
    'live_validation_accepts_exact_eight_and_rejects_ninth_without_mutation',
    'app_capture_changes_only_draft_and_apply_is_one_undoable_edit',
    'app_logical_escape_and_tab_with_physical_j_capture_without_widget_side_effects',
    'app_same_frame_focus_loss_and_regain_cancel_without_draft_change',
    'app_modal_text_paste_ime_and_pointer_cannot_mutate_underlying_widgets',
    'app_captured_conflict_is_live_and_discard_restores_accepted_row',
    'app_missing_physical_escape_refuses_without_closing_modal',
}
OLD_WIDGET_NAMES = {
    "six_action_widgets_save_reopen_and_exact_undo_redo",
    "conflicting_draft_blocks_navigation_and_save_preserves_it",
    "empty_system_actions_warn_and_eight_keyboard_bindings_are_editable",
    "saved_widget_map_drives_collect_loader_and_real_bridge_trace",
}


def document_inventory(windows):
    names = DOCUMENT_NAMES | (set() if windows else UNIX_DOCUMENT_NAMES)
    return {"collect_input_map_authoring::tests::" + name for name in names}


def validate_inventory(output, expected):
    """Check compiled libtest names, duplicates and final test/benchmark count."""
    expected = set(expected)
    output = output.replace("\r\n", "\n")
    names = re.findall(r"^(\S+): test$", output, re.M)
    counts = re.findall(r"^(\d+) tests?, (\d+) benchmarks?$", output, re.M)
    if (not expected or len(names) != len(expected) or set(names) != expected
            or counts != [(str(len(expected)), "0")]):
        raise ValueError("Compiled test inventory changed: " + repr(sorted(expected)))


def inventory(base, expected, evidence, label):
    output = HELPERS["run_logged"](base + ["--", "--list", "--format=pretty", "--color=never"],
                                   evidence / (label + "-inventory.log"))
    validate_inventory(output, expected)


def contracts(evidence):
    if not CAPTURE_NAMES:
        raise ValueError("Capture inventory has not been reviewed")
    expected = document_inventory(os.name == "nt") | CAPTURE_NAMES
    inventory(CPU, expected, evidence, "capture-cpu")
    inventory(GPU_BASE, document_inventory(os.name == "nt") | {GPU_TEST}, evidence, "capture-gpu")
    output = HELPERS["run_logged"](CPU + ["--", "--format=pretty", "--color=never", "--test-threads=1"],
                                   evidence / "capture-cpu.log")
    previous.validate_exact(output, expected)
    # The saved-map producer now uses capture, but the exact file/hash contract,
    # private Keys consumer and main-editor regression requirements stay intact.
    previous.contracts(evidence)
    previous.boundaries(evidence)
    old_base = BASE + ["--test", "collect_input_map_authoring_ui"]
    for name in sorted(OLD_WIDGET_NAMES):
        previous.run_case(old_base, name, evidence, "old-authoring-" + name)
    HELPERS["run_logged"]([
        "cargo", "clippy", "--locked", "-p", "orr_editor", "--no-default-features",
        "--features", "collect-input-map-editor", "--lib", "--bin", "orr_collect_input_map",
        "--test", "collect_input_capture", "--test", "collect_input_capture_ui",
        "--test", "collect_input_map_authoring_ui", "--", "-D", "warnings"], evidence / "capture-strict.log")
    # Preserve an explicit default-feature compilation in addition to previous
    # gate's no-default/UI-only/map-only/combined boundary matrix.
    HELPERS["run_logged"](["cargo", "check", "--locked", "-p", "orr_editor", "--lib"],
                           evidence / "default-features-check.log")


CAPTURE_FRAMES = {
    "capture-initial.png": {"Capture binding 1", "Binding 1"},
    "capture-listening.png": {"Capture physical key", "Cancel capture"},
    "capture-physical-unavailable.png": {"Capture physical key", "Cancel capture"},
    "capture-live-conflict.png": {"Draft error: duplicate physical binding for right", "Apply action"},
    "capture-eight-rows.png": {"Binding 8", "Capture binding 8", "Remove binding 8"},
    "capture-escape-policy.png": {"Compatibility warning: Escape on movement or restart is valid for standalone Collect, but cannot be loaded by the main editor, where Escape releases controls.", "Apply action"},
}
CAPTURE_FLAGS = (
    "production_eframe_app", "physical_egui_events", "six_physical_actions",
    "logical_keys_differ", "capture_draft_only", "live_conflict_before_apply",
    "cancel_escape_held_repeat_focus_loss", "modal_text_paste_tab_enter_isolation",
    "physical_none_refused_dropdown_fallback", "eight_rows_scrolled",
    "accepted_only_save", "exact_undo_redo", "standalone_escape_dropdown",
)


def validate_png(path):
    previous.sha(path)  # Reject symlinks and missing/nonregular artifacts.
    previous.png_size(path, [720, 650])
    raw = path.read_bytes()
    offset, chunks, compressed = 8, [], bytearray()
    header = None
    while offset < len(raw):
        if len(raw) - offset < 12:
            raise ValueError("Truncated PNG chunk")
        size = struct.unpack(">I", raw[offset:offset + 4])[0]
        end = offset + size + 12
        if end > len(raw):
            raise ValueError("Truncated PNG data")
        kind, data = raw[offset + 4:offset + 8], raw[offset + 8:end - 4]
        crc = struct.unpack(">I", raw[end - 4:end])[0]
        if zlib.crc32(kind + data) != crc:
            raise ValueError("Invalid PNG checksum")
        chunks.append(kind)
        if kind == b"IHDR":
            header = data
        if kind == b"IEND" and data:
            raise ValueError("Invalid PNG end chunk")
        if kind == b"IDAT":
            compressed.extend(data)
        offset = end
    if (not chunks or chunks[0] != b"IHDR" or chunks[-1] != b"IEND"
            or chunks.count(b"IHDR") != 1 or chunks.count(b"IEND") != 1 or not compressed):
        raise ValueError("Incomplete PNG artifact")
    if header != struct.pack(">IIBBBBB", 720, 650, 8, 6, 0, 0, 0):
        raise ValueError("Expected non-interlaced RGBA8 capture")
    decoder = zlib.decompressobj()
    pixels = decoder.decompress(compressed, (720 * 4 + 1) * 650 + 1)
    if (len(pixels) != (720 * 4 + 1) * 650 or not decoder.eof
            or decoder.unused_data or decoder.unconsumed_tail
            or any(pixels[row * (720 * 4 + 1)] > 4 for row in range(650))):
        raise ValueError("Invalid PNG pixel data")


def validate_capture_gpu(captures):
    receipt_path = captures / "collect-input-capture-acceptance.json"
    previous.sha(receipt_path)
    receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
    if type(receipt.get("schema")) is not int or receipt["schema"] != 1:
        raise ValueError("Unknown capture receipt schema")
    if receipt.get("window_pixels") != [720, 650] or receipt.get("native_os_keyboard") is not False:
        raise ValueError("Incorrect capture dimensions or native-input scope")
    for key in CAPTURE_FLAGS:
        if receipt.get(key) is not True:
            raise ValueError("Missing capture proof: " + key)
    adapter = receipt.get("adapter_details")
    if (not isinstance(adapter, dict) or adapter.get("backend") != "Vulkan"
            or adapter.get("device_type") != "Cpu"
            or "llvmpipe" not in str(adapter.get("name", "")).lower()
            or any(not isinstance(adapter.get(k), str) for k in ("driver", "driver_info"))):
        raise ValueError("Explicit software Vulkan capture adapter required")
    frames = receipt.get("frames")
    if not isinstance(frames, list) or len(frames) != len(CAPTURE_FRAMES):
        raise ValueError("Missing capture frames")
    seen = set()
    for frame in frames:
        if not isinstance(frame, dict):
            raise ValueError("Malformed capture frame")
        filename = frame.get("filename")
        if not isinstance(filename, str) or filename not in CAPTURE_FRAMES or filename in seen:
            raise ValueError("Wrong or duplicate capture artifact")
        seen.add(filename)
        if not isinstance(frame.get("proof"), str) or not frame["proof"].strip():
            raise ValueError("Missing capture frame assertion")
        widgets = frame.get("widgets")
        if not isinstance(widgets, list) or len(widgets) != len(CAPTURE_FRAMES[filename]):
            raise ValueError("Missing visible widget bounds")
        labels = set()
        for widget in widgets:
            if not isinstance(widget, dict) or widget.get("label") not in CAPTURE_FRAMES[filename]:
                raise ValueError("Unexpected visible widget")
            label, rect = widget["label"], widget.get("rect")
            if label in labels:
                raise ValueError("Duplicate visible widget")
            labels.add(label)
            if (not isinstance(rect, list) or len(rect) != 4
                    or any(type(x) not in (int, float) or not math.isfinite(x) for x in rect)
                    or not (0 <= rect[0] < rect[2] <= 720 and 0 <= rect[1] < rect[3] <= 650)
                    or rect[2] - rect[0] < 5 or rect[3] - rect[1] < 8):
                raise ValueError("Clipped or unreadable widget bounds")
        validate_png(captures / filename)
    saved_path = captures / "capture-accepted-only.json"
    previous.sha(saved_path)
    saved = json.loads(saved_path.read_text(encoding="utf-8"))
    expected = {"version": 1, "actions": [
        {"name": action, "bindings": [{"device": "Keyboard", "key": key}]}
        for action, key in (("left", "j"), ("right", "l"), ("up", "i"),
                            ("down", "k"), ("restart", "enter"), ("quit", "q"))]}
    if saved != expected:
        raise ValueError("Capture saved bytes differ from accepted six-action map")


def gpu(evidence):
    captures = evidence / "capture-software-gpu"
    captures.mkdir(exist_ok=False)
    os.environ["ORR_COLLECT_INPUT_CAPTURE_EVIDENCE"] = str(captures.resolve())
    previous.run_case(GPU_BASE, GPU_TEST, evidence, "capture-software-gpu", ignored=True)
    validate_capture_gpu(captures)
    previous.gpu(evidence)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("contracts", "gpu"))
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    evidence = args.evidence.resolve()
    marker = evidence / "capture-contracts-complete.json"
    if args.mode == "contracts":
        evidence.mkdir(parents=True, exist_ok=False)
        contracts(evidence)
        marker.write_text(json.dumps({"schema": 1, "capture_contracts_complete": True}), encoding="utf-8")
    else:
        if json.loads(marker.read_text(encoding="utf-8")) != {"schema": 1, "capture_contracts_complete": True}:
            raise ValueError("Capture CPU contracts and boundaries must complete first")
        path = evidence / "editor-authored-controls.json"
        before = (evidence / "producer-sha256.txt").read_text().strip()
        previous.validate_handoff(json.loads(path.with_suffix(".receipt.json").read_text()), path, before)
        gpu(evidence)


if __name__ == "__main__":
    main()
