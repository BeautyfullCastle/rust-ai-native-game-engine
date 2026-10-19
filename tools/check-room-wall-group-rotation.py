#!/usr/bin/env python3
"""Additive quarter-turn acceptance: actual CPU tests and Linux Vulkan UI proof."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import runpy
import sys
import unittest
import zlib

if os.environ.get("PYTHONDONTWRITEBYTECODE") != "1":
    raise RuntimeError("Set PYTHONDONTWRITEBYTECODE=1 before running this gate")
helpers = runpy.run_path("tools/check-authoring-controls.py")


def run(evidence, name, args):
    return helpers["run_logged"](
        ["cargo", args[0], "--locked", "-p", "orr_editor", *args[1:]],
        evidence / (name + ".log"))


def validate_executed(output, count, exact_names=None):
    """Require distinct, executed passing cases, not merely a successful process."""
    output = output.replace("\r\n", "\n")
    summaries = helpers["SUMMARY"].findall(output)
    cases = helpers["CASE"].findall(output)
    names = [name for name, _ in cases]
    if (len(summaries) != 1
            or tuple(map(int, summaries[0][:4])) != (count, 0, 0, 0)
            or re.findall(r"^running (\d+) tests?$", output, re.M) != [str(count)]
            or len(names) != count or len(set(names)) != count
            or any(status != "ok" for _, status in cases)
            or (exact_names is not None and names != exact_names)):
        raise RuntimeError("Expected exactly the required executed passing test cases")


def cpu(evidence):
    system = platform.system()
    if system not in ("Linux", "Windows"):
        raise RuntimeError("This acceptance gate supports Linux and Windows only")
    linux = system == "Linux"
    if linux and platform.machine() != "x86_64":
        raise RuntimeError("Linux rotation acceptance requires x86_64")
    features = "room-authoring" if linux else "windows-projects"
    suites = [
        ("room_wall_stamp::tests", 8),
        ("room_wall_stamp::placement::tests", 7),
        ("room_wall_stamp_panel::tests", 2),
        ("editor::room_authoring::room_wall_stamp_editor_tests", 3),
        ("room_wall_stamp_store::tests", 2 if linux else 3),
    ]
    if linux:
        suites.append(("room_wall_stamp_store::linux_tests", 17))
    for suite, count in suites:
        output = run(evidence, suite.replace("::", "-"), [
            "test", "--features", features, "--lib", suite + "::", "--",
            "--format=pretty", "--color=never", "--test-threads=1"])
        helpers["validate_tests"](output, suite, count)
    deps = run(evidence, "deps", ["test", "--test", "deps", "--features", features,
                                "--", "--format=pretty", "--color=never", "--test-threads=1"])
    validate_executed(deps, 6)
    for name, args in (
        ("default-check", ["check", "--lib"]),
        ("default-lint", ["clippy", "--lib", "--", "-D", "warnings"]),
        ("minimal-check", ["check", "--lib", "--no-default-features", "--features", "room-authoring"]),
        ("minimal-lint", ["clippy", "--lib", "--no-default-features", "--features", "room-authoring", "--", "-D", "warnings"]),
        ("authoring-lint", ["clippy", "--lib", "--features", features, "--", "-D", "warnings"]),
    ):
        run(evidence, name, args)


UI_TEST = "linux_room_wall_quarterturns_exact_geometry_history_reopen_and_collision"
CAPTURES = sorted([
    "01-source-viewport.png",
    *[f"02-controls-{degrees}-degrees.png" for degrees in (0, 90, 180, 270)],
    *[f"03-gameplay-{degrees}-degrees.png" for degrees in (0, 90, 180, 270)],
    "04-rotated-saved-viewport.png", "05-fresh-source-hidden-viewport.png",
    "06-fresh-source-hidden-gameplay.png",
])


def validate_receipt(receipt):
    expected = {"schema": 1, "platform": "linux-x86_64", "physical_os_input": False}
    expected.update(dict.fromkeys((
        "production_editor_app", "exact_geometry_all_four_rotations", "actual_rotation_controls",
        "fresh_guids_and_redo_identity", "one_atomic_history_entry",
        "invalid_pivot_and_bounds_failure_preserve_redo", "loaded_template_bytes_unchanged",
        "source_hidden_during_fresh_reopen", "reopen_pixels_equal"), True))
    if any(type(receipt.get(k)) is not type(v) or receipt.get(k) != v for k, v in expected.items()):
        raise RuntimeError("Incomplete quarter-turn UI acceptance receipt")
    if "llvmpipe" not in str(receipt.get("adapter", "")).lower():
        raise RuntimeError("Expected actual software Vulkan llvmpipe adapter")
    if not re.fullmatch(r"0x[0-9a-f]{16}", str(receipt.get("scene_checksum", ""))):
        raise RuntimeError("Missing final scene checksum")
    routes = receipt.get("gameplay_controls", [])
    if not isinstance(routes, list) or len(routes) != 4:
        raise RuntimeError("Missing wrong-rotation gameplay negative controls")
    if [item.get("degrees") for item in routes] != [0, 180, 270, 90]:
        raise RuntimeError("Unexpected rotation gameplay control order")
    for route in routes:
        degrees = route.get("degrees")
        if type(degrees) is not int or type(route.get("blocked")) is not bool:
            raise RuntimeError("Invalid rotation gameplay control types")
        if route["blocked"] != (degrees == 90):
            raise RuntimeError("Wrong-rotation negative control must remain unblocked")
        validate_route(route, degrees == 90)
    validate_route(receipt.get("reopened_gameplay", {}), True)


def validate_route(route, blocked):
    start, end = route.get("start"), route.get("end")
    for vector in (start, end):
        if (not isinstance(vector, list) or len(vector) != 3
                or any(type(value) not in (int, float) or not math.isfinite(value) for value in vector)):
            raise RuntimeError("Missing finite gameplay coordinates")
    if blocked:
        if not start[0] + 0.05 < end[0] < 1.7:
            raise RuntimeError("Rotated wall must block the gameplay route")
    elif not end[0] > 2.5:
        raise RuntimeError("Wrong-rotation control must pass the barrier position")


def gpu(evidence):
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        raise RuntimeError("Software Vulkan rotation acceptance requires Linux x86_64")
    evidence.mkdir(parents=True, exist_ok=True)
    capture = evidence / "linux-ui"
    if capture.exists():
        raise RuntimeError("GPU acceptance requires a fresh absent capture directory")
    os.environ["ORR_ROOM_WALL_ROTATION_LINUX_DIR"] = str(capture.resolve())
    run(evidence, "rotation-ui-lint", ["clippy", "--test", "room_wall_stamp_rotation_ui",
        "--features", "room-authoring", "--", "-D", "warnings"])
    output = run(evidence, "linux-ui", ["test", "--features", "room-authoring", "--test",
        "room_wall_stamp_rotation_ui", UI_TEST, "--", "--exact", "--ignored",
        "--format=pretty", "--color=never", "--test-threads=1"])
    validate_executed(output, 1, [UI_TEST])
    receipt = json.loads((capture / "acceptance.json").read_text())
    validate_receipt(receipt)
    captures = receipt.get("captures", [])
    if [item.get("name") for item in captures] != CAPTURES:
        raise RuntimeError("Missing required quarter-turn screenshot receipt")
    for item in captures:
        data = (capture / item["name"]).read_bytes()
        validate_capture(data, item)


def validate_capture(data, item):
    if (len(data) < 33 or data[:8] != b"\x89PNG\r\n\x1a\n"
            or data[8:12] != (13).to_bytes(4, "big") or data[12:16] != b"IHDR"
            or int.from_bytes(data[29:33], "big") != zlib.crc32(data[12:29])
            or type(item.get("bytes")) is not int or item["bytes"] != len(data)
            or item.get("sha256") != hashlib.sha256(data).hexdigest()):
        raise RuntimeError("Missing, malformed or mismatched PNG capture")
    width = int.from_bytes(data[16:20], "big")
    height = int.from_bytes(data[20:24], "big")
    if not 0 < width <= 0x7fffffff or not 0 < height <= 0x7fffffff:
        raise RuntimeError("PNG capture dimensions must be positive")
    if item.get("name") in (
            "01-source-viewport.png", "04-rotated-saved-viewport.png",
            "05-fresh-source-hidden-viewport.png") and (width, height) != (900, 850):
        raise RuntimeError("Viewport captures must have the asserted 900x850 dimensions")



class ValidatorTests(unittest.TestCase):
    def output(self):
        return ("running 1 test\ntest rotation_ui ... ok\n"
                "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; "
                "0 filtered out; finished in 0.01s\n")

    def test_executed_and_windows_newlines(self):
        for output in (self.output(), self.output().replace("\n", "\r\n")):
            validate_executed(output, 1, ["rotation_ui"])

    def test_rejects_empty_ignored_duplicate_missing_and_wrong_case(self):
        for output in ("", self.output() * 2,
                       self.output().replace("1 passed", "0 passed"),
                       self.output().replace("0 ignored", "1 ignored"),
                       self.output().replace("... ok", "... ignored"),
                       self.output().replace("rotation_ui", "unrelated"),
                       self.output().replace("running 1", "running 0"),
                       self.output().split("test result:")[0]):
            with self.assertRaises(RuntimeError):
                validate_executed(output, 1, ["rotation_ui"])


    def test_gameplay_negative_controls_are_required(self):
        validate_route({"start": [0, 0, 0], "end": [3, 0, 0]}, False)
        validate_route({"start": [0, 0, 0], "end": [1, 0, 0]}, True)
        for route, blocked in (({"start": [0, 0, 0], "end": [1, 0, 0]}, False),
                               ({"start": [0, 0, 0], "end": [3, 0, 0]}, True),
                               ({"start": [False, 0, 0], "end": [1, 0, 0]}, True),
                               ({"start": [0, 0, 0], "end": [float("nan"), 0, 0]}, False),
                               ({}, True)):
            with self.assertRaises(RuntimeError):
                validate_route(route, blocked)

    def test_capture_dimensions_integrity_and_viewport_size(self):
        def fixture(width, height):
            header = b"IHDR" + width.to_bytes(4, "big") + height.to_bytes(4, "big") + bytes([8, 6, 0, 0, 0])
            return b"\x89PNG\r\n\x1a\n" + (13).to_bytes(4, "big") + header + zlib.crc32(header).to_bytes(4, "big")

        def metadata(data, name="01-source-viewport.png"):
            return {"name": name, "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}

        valid = fixture(900, 850)
        validate_capture(valid, metadata(valid))
        other = fixture(1200, 900)
        validate_capture(other, metadata(other, "02-controls-0-degrees.png"))
        for invalid in (fixture(0, 850), fixture(900, 0), other,
                        b"bad PNG", b"broken!!" + valid[8:], valid[:-1] + b"x"):
            with self.assertRaises(RuntimeError):
                validate_capture(invalid, metadata(invalid))
        with self.assertRaises(RuntimeError):
            validate_capture(valid, dict(metadata(valid), sha256="bad digest"))

    def test_receipt_requires_every_rotation_and_reopen_proof(self):
        # Synthetic metadata checks the validator only; never written as acceptance evidence.
        receipt = {"schema": 1, "platform": "linux-x86_64", "physical_os_input": False,
                   "adapter": "llvmpipe test fixture", "scene_checksum": "0x0000000000000001"}
        receipt.update(dict.fromkeys((
            "production_editor_app", "exact_geometry_all_four_rotations", "actual_rotation_controls",
            "fresh_guids_and_redo_identity", "one_atomic_history_entry",
            "invalid_pivot_and_bounds_failure_preserve_redo", "loaded_template_bytes_unchanged",
            "source_hidden_during_fresh_reopen", "reopen_pixels_equal"), True))
        receipt["gameplay_controls"] = [
            {"degrees": degrees, "start": [0, 0, 0],
             "end": [1 if degrees == 90 else 3, 0, 0], "blocked": degrees == 90}
            for degrees in (0, 180, 270, 90)]
        receipt["reopened_gameplay"] = {"start": [0, 0, 0], "end": [1, 0, 0]}
        validate_receipt(receipt)
        for key in receipt:
            invalid = dict(receipt)
            del invalid[key]
            with self.assertRaises(RuntimeError, msg=key):
                validate_receipt(invalid)
        for key, value in (("schema", True), ("production_editor_app", 1),
                           ("adapter", "hardware Vulkan"), ("gameplay_controls", [])):
            invalid = dict(receipt, **{key: value})
            with self.assertRaises(RuntimeError, msg=key):
                validate_receipt(invalid)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--gpu", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        unittest.main(argv=[sys.argv[0]])
        return
    if args.evidence is None:
        parser.error("--evidence is required unless --self-test is selected")
    if args.gpu:
        gpu(args.evidence)
    else:
        args.evidence.mkdir(parents=True, exist_ok=False)
        cpu(args.evidence)


if __name__ == "__main__":
    main()
