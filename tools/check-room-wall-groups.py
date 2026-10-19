#!/usr/bin/env python3
"""Additive exact-count wall-group acceptance; no empty/ignored runs accepted."""
import argparse
import hashlib
import json
import re
import os
from pathlib import Path
import platform
import runpy


# The workflow sets this before interpreter startup; direct invocations must too.
if os.environ.get("PYTHONDONTWRITEBYTECODE") != "1":
    raise RuntimeError("Set PYTHONDONTWRITEBYTECODE=1 before running this gate")
helpers = runpy.run_path("tools/check-authoring-controls.py")


def run(evidence, name, args):
    return helpers["run_logged"](["cargo", args[0], "--locked", "-p", "orr_editor", *args[1:]], evidence / (name + ".log"))


def cpu(evidence):
    if platform.system() not in ("Linux", "Windows"):
        raise RuntimeError("This gate supports Linux and Windows only")
    linux = platform.system() == "Linux"
    features = "room-authoring" if linux else "windows-projects"
    suites = [
        ("room_wall_stamp::tests", 8),
        ("editor::room_authoring::room_wall_stamp_editor_tests", 3),
        ("room_wall_stamp_store::tests", 2 if linux else 3),
    ]
    if linux:
        if platform.machine() != "x86_64":
            raise RuntimeError("Linux wall groups acceptance requires x86_64")
        suites.append(("room_wall_stamp_store::linux_tests", 17))
    for suite, count in suites:
        output = run(evidence, suite.replace("::", "-"), ["test", "--features", features,
                     "--lib", suite + "::", "--", "--format=pretty", "--color=never", "--test-threads=1"])
        helpers["validate_tests"](output, suite, count)
    for name, args in (
        ("default-check", ["check", "--lib"]),
        ("default-lint", ["clippy", "--lib", "--", "-D", "warnings"]),
        ("minimal-check", ["check", "--lib", "--no-default-features", "--features", "room-authoring"]),
        ("authoring-lint", ["clippy", "--lib", "--features", features, "--", "-D", "warnings"]),
        ("deps", ["test", "--test", "deps", "--features", features]),
        ("ui-lint", ["clippy", "--test", "room_wall_stamp_linux_ui" if linux else "room_wall_stamp_ui",
                     "--features", features, "--", "-D", "warnings"]),
    ):
        run(evidence, name, args)


def gpu(evidence):
    evidence.mkdir(parents=True, exist_ok=True)
    capture = evidence / "linux-ui"
    if capture.exists():
        raise RuntimeError("GPU acceptance requires a fresh absent capture directory")
    os.environ["ORR_ROOM_WALL_STAMPS_LINUX_DIR"] = str(capture.resolve())
    name = "linux_room_wall_groups_ui_geometry_history_reopen_and_collision"
    output = run(evidence, "linux-ui", ["test", "--features", "room-authoring", "--test",
                 "room_wall_stamp_linux_ui", name, "--", "--exact", "--ignored",
                 "--format=pretty", "--color=never", "--test-threads=1"])
    summaries = helpers["SUMMARY"].findall(output)
    if (len(summaries) != 1 or tuple(map(int, summaries[0][:4])) != (1, 0, 0, 0)
            or re.findall(r"^running (\d+) tests?$", output, re.M) != ["1"]
            or helpers["CASE"].findall(output) != [(name, "ok")]):
        raise RuntimeError("Expected exactly one executed passing ignored Linux UI test")
    receipt = json.loads((capture / "acceptance.json").read_text())
    expected = {"schema": 1, "platform": "linux-x86_64", "physical_os_input": False}
    expected.update(dict.fromkeys((
        "production_editor_app", "file_menu_wall_groups", "software_gpu",
        "exact_collision_geometry_and_fresh_guids", "one_history_entry_per_instance",
        "checksum_stale_capture_rejected", "out_of_bounds_transaction_rolled_back",
        "invalid_load_clears_template", "removed_source_save_rejected",
        "loaded_snapshot_reusable_without_source", "repeated_instantiation",
        "undo_redo_save_fresh_reopen", "source_hidden_during_reopen",
        "path_epoch_stale_capture_rejected", "open_scene_cancel_preserves_selection_and_history"), True))
    if any(type(receipt.get(k)) is not type(v) or receipt.get(k) != v for k, v in expected.items()):
        raise RuntimeError("Incomplete Linux wall-group UI receipt")
    if "llvmpipe" not in str(receipt.get("adapter", "")).lower():
        raise RuntimeError("Expected actual software Vulkan llvmpipe adapter")
    if receipt.get("visual_proof") != {"viewport_changed": True, "reopen_pixels_equal": True}:
        raise RuntimeError("Missing visual proof")
    gameplay = receipt.get("gameplay_proof", {})
    control_end = gameplay.get("control_end", [])
    blocked_start = gameplay.get("blocked_start", [])
    blocked_end = gameplay.get("blocked_end", [])
    if (any(not isinstance(v, list) or len(v) != 3 for v in (control_end, blocked_start, blocked_end))
            or not control_end[0] > 2.5
            or not blocked_start[0] + 0.05 < blocked_end[0] < 1.7):
        raise RuntimeError("Missing or inconsistent control-versus-instanced-wall gameplay proof")
    expected_images = (
        "01-original-route-unblocked.png", "02-source-walls-viewport.png",
        "03-captured-saved-loaded.png", "04-rejected-out-of-bounds.png",
        "05-instantiated-undo-redo.png", "06-saved-expanded-viewport.png",
        "07-fresh-source-hidden-viewport.png", "08-instanced-wall-blocks-gameplay.png",
        "09-path-epoch-stale-selection.png")
    captures = receipt.get("captures", [])
    if [item.get("name") for item in captures] != list(expected_images):
        raise RuntimeError("Missing required capture receipt")
    for item in captures:
        data = (capture / item["name"]).read_bytes()
        if (len(data) < 33 or data[:8] != b"\x89PNG\r\n\x1a\n" or data[12:16] != b"IHDR"
                or item.get("bytes") != len(data) or item.get("sha256") != hashlib.sha256(data).hexdigest()):
            raise RuntimeError("Missing, malformed or mismatched PNG capture")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--gpu", action="store_true")
    args = parser.parse_args()
    if args.gpu:
        gpu(args.evidence)
    else:
        args.evidence.mkdir(parents=True, exist_ok=False)
        cpu(args.evidence)


if __name__ == "__main__":
    main()
