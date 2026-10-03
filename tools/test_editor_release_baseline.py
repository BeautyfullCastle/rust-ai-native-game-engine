"""Contract tests for the bounded release-baseline parser and validator."""

from __future__ import annotations

import json
import os
from pathlib import Path
import sys
import tempfile
from unittest.mock import patch
import unittest

import editor_release_baseline as baseline


def gpu_records(run_name: str, *, software: bool = False) -> list[dict]:
    expected = baseline.GPU_EXPECTED[run_name]
    metadata = {"schema_version": 1, "suite": "renderer", "case": "metadata",
                **baseline.GPU_COMMON, **expected,
                "renderer_initialization_included": False, "target_creation_included": False,
                "cold_scope": "first_frame_after_renderer_construction",
                "adapter_name": "Test Adapter", "adapter_backend": "Vulkan",
                "adapter_device_type": "DiscreteGpu", "adapter_vendor": 123,
                "adapter_device": 456, "driver": "driver", "driver_info": "info", "software": software}
    common = {key: metadata.get(key) for key in baseline.GPU_FRAME_IDENTITY_KEYS}
    records = [metadata]
    for frame_class, count in (("cold", 1), ("warmup", 10), ("steady", 30)):
        for index in range(count):
            frame = {"schema_version": 1, "suite": "renderer", "case": "frame", **common,
                     "frame_class": frame_class, "frame_index": index, "wall_ms": 0.5,
                     "cpu_prepare_ms": None, "cpu_encode_ms": 0.1, "cpu_submit_ms": 0.2,
                     "shape_instances": 1000 if expected["renderer"] == "2d" else 0,
                     "mesh_instances": 0 if expected["renderer"] == "2d" else 1000,
                     "line_instances": 0,
                     "main": {"passes": 1, "draw_calls": 1, "instances": 1000},
                     "shadow": {"passes": 1 if expected["renderer"] == "3d" else 0,
                                "draw_calls": 1 if expected["renderer"] == "3d" else 0,
                                "instances": 1000 if expected["renderer"] == "3d" else 0},
                     "upload_calls": 0, "upload_bytes": 0, "buffer_reallocations": 0,
                     "attachment_allocations": 0, "attachment_reallocations": 0, "msaa_samples": 1}
            frame["wall_scope"] = "render_call_return_not_gpu_completion"
            records.append(frame)
    records.append({"schema_version": 1, "suite": "renderer", "case": "readback",
                    "renderer": expected["renderer"], "preset": expected["preset"],
                    "scene_id": expected["scene_id"], "resolution_px": [640, 360],
                    "readback_ms": 2.0, "pixel_bytes": 640 * 360 * 4,
                    "fnv1a64": "0123456789abcdef", "timing_included_in_frame_samples": False})
    return records


def editor_records() -> list[dict]:
    common = {"schema_version": 1, "suite": "editor", "window_px": [1500, 900],
              "backend": "egui_kittest", "erp_connected": False, "gpu": False}
    records = []
    for index in range(1, 41):
        records.append({**common, "case": "drag_move", "scene_id": "demo_editor", "body_count": 10,
                        "move_index": index, "group": "first" if index == 1 else "steady",
                        "frames": 1, "wall_ms": 4.0})
    for phase, count in (("edit", 60), ("play_1x", 240), ("play_4x", 240)):
        records.extend({**common, "case": "ui_frame", "scene_id": "demo_editor_1000_bodies",
                        "body_count": 1010, "phase": phase, "frame_index": index, "wall_ms": 1.0}
                       for index in range(count))
    for phase in ("drag", "edit", "play 1x", "play 4x", "600-tick step"):
        records.append({**common, "case": "diagnostics", "phase": phase,
                        "scene_id": "demo_editor" if phase == "drag" else "demo_editor_1000_bodies",
                        "body_count": 10 if phase == "drag" else 1010,
                        "metrics": {name: {"samples": 0 if name == "ui_frame" else 2,
                                            "total_samples": 0 if name == "ui_frame" else 2,
                                            "last_ms": 0.0, "max_ms": 0.0, "p95_ms": 0.0}
                                    for name in ("ui_frame", "pump", "sync_erp_wait", "async_request", "snapshot_extract")}})
    return records


class ParserTests(unittest.TestCase):
    def test_only_well_formed_object_records_are_accepted(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "stdout.log").write_text("ordinary output\n", encoding="utf-8")
            (root / "stderr.log").write_text("ORR_BASELINE {bad json}\nORR_BASELINE []\n", encoding="utf-8")
            records, errors = baseline._records(root)
            self.assertEqual(records, [])
            self.assertEqual(len(errors), 2)

    def test_parser_rejects_nonfinite_json_constants(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "stdout.log").write_text("ORR_BASELINE {\"v\":NaN}\n", encoding="utf-8")
            (root / "stderr.log").write_text("", encoding="utf-8")
            records, errors = baseline._records(root)
            self.assertFalse(records)
            self.assertTrue(errors)

    def test_normal_process_exit_is_reaped_and_does_not_abort_collection(self):
        for code in (0, 1):
            with self.subTest(exit_code=code), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                result = baseline._launch(
                    [sys.executable, "-c", f"print('finished'); raise SystemExit({code})"],
                    os.environ.copy(), Path.cwd(), root)
                self.assertEqual(result["exit_code"], code)
                self.assertTrue(result["child_cleanup_verified"])
                self.assertEqual(result["cleanup"]["action"], "normal_exit")
                self.assertTrue(result["cleanup"]["stdio_closed"])
                self.assertIsNone(result["launch_error"])
                self.assertFalse(result["timed_out"])
                self.assertIn(b"finished", (root / "stdout.log").read_bytes())

    def test_large_integer_timing_is_invalid_instead_of_crashing(self):
        records = gpu_records("gpu_2d_default")
        records[1]["wall_ms"] = 10 ** 400
        errors = []
        summary = baseline._validate_gpu("gpu_2d_default", records, errors, "default")
        self.assertTrue(any("finite" in error for error in errors))
        self.assertFalse(summary["frame_wall_ms_summary"]["cold"]["available"])

    def test_cargo_provenance_requires_the_actual_test_executable(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            executable = root / "fixture.exe"
            executable.write_bytes(b"test executable")
            artifact = {"reason": "compiler-artifact", "target": {"name": "gpu"},
                        "profile": {"test": True}, "executable": str(executable)}
            (root / "stdout.log").write_text(json.dumps(artifact) + "\n", encoding="utf-8")
            identity = baseline._compiled_binary(root, ["cargo", "--test", "gpu"])
            self.assertEqual(identity["bytes"], 15)
            executable.unlink()
            with self.assertRaises(baseline.BaselineError):
                baseline._compiled_binary(root, ["cargo", "--test", "gpu"])

    def test_process_timeout_preserves_bounded_logs_and_owns_cleanup(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            with patch.object(baseline, "MAX_RUN_SECONDS", 0.1):
                result = baseline._launch([sys.executable, "-c", "import time; print('started', flush=True); time.sleep(10)"],
                                         os.environ.copy(), Path.cwd(), root)
            self.assertTrue(result["timed_out"])
            self.assertTrue(result["child_cleanup_verified"])
            self.assertIsNotNone(result["exit_code"])
            self.assertIn(b"started", (root / "stdout.log").read_bytes())

    def test_fixed_plan_has_all_builds_and_all_independent_repetitions(self):
        plan = baseline._planned_commands(3, "software", Path("repo"))
        self.assertEqual(len(plan), len(baseline.BUILD_RUNS) + 12)
        measurements = [item for item in plan if item["phase"] == "measurement"]
        self.assertEqual(len(measurements), 12)
        self.assertTrue(all("--locked" in item["argv"] for item in plan))
        gpu = next(item for item in measurements if item["suite_name"] == "gpu_2d_default")
        self.assertEqual(gpu["env_allowlist"]["ORR_BASELINE_GPU_MODE"], "software")
        self.assertEqual(gpu["env_allowlist"]["ORR_REQUIRE_GPU"], "1")


class EditorContractTests(unittest.TestCase):
    def test_editor_counts_scene_identity_and_zero_sample_availability(self):
        errors: list[str] = []
        summary = baseline._validate_editor(editor_records(), errors)
        self.assertEqual(errors, [])
        self.assertEqual(len(summary["drag_steady_39_wall_ms"]), 39)
        stat = summary["diagnostic_availability"]["drag"]["ui_frame"]
        self.assertFalse(stat["available"])
        self.assertEqual(stat["samples"], 0)
        self.assertEqual(stat["raw_ms"]["last_ms"], 0.0)

    def test_editor_missing_frame_is_incomplete(self):
        rows = editor_records()
        rows.pop(next(i for i, row in enumerate(rows) if row.get("case") == "ui_frame" and row["phase"] == "play_4x"))
        errors: list[str] = []
        baseline._validate_editor(rows, errors)
        self.assertTrue(any("play_4x frame records" in error for error in errors))

    def test_editor_zero_sample_timing_cannot_be_reported_as_a_measurement(self):
        rows = editor_records()
        diagnostic = next(row for row in rows if row.get("case") == "diagnostics")
        diagnostic["metrics"]["ui_frame"]["last_ms"] = 0.25
        errors: list[str] = []
        baseline._validate_editor(rows, errors)
        self.assertTrue(any("timings without samples" in error for error in errors))


class RendererContractTests(unittest.TestCase):
    def test_valid_gpu_records_accept_nullable_cpu_timing_without_claiming_gpu_time(self):
        for name in baseline.GPU_EXPECTED:
            with self.subTest(name=name):
                errors: list[str] = []
                summary = baseline._validate_gpu(name, gpu_records(name), errors)
                self.assertEqual(errors, [])
                self.assertTrue(summary["cpu_frame_fields_are_not_gpu_completion"])
                self.assertTrue(summary["readback_ms_excluded"])

    def test_gpu_missing_counts_nonfinite_wall_and_scope_claim_are_rejected(self):
        name = "gpu_3d_low"
        rows = gpu_records(name)
        rows = [r for r in rows if not (r.get("case") == "frame" and r.get("frame_class") == "steady" and r.get("frame_index") == 29)]
        first_frame = next(r for r in rows if r.get("case") == "frame")
        first_frame["wall_ms"] = float("nan")
        first_frame["wall_scope"] = "gpu_completed"
        errors: list[str] = []
        baseline._validate_gpu(name, rows, errors)
        self.assertTrue(any("exact indices" in error for error in errors))
        self.assertTrue(any("finite nonnegative" in error for error in errors))
        self.assertTrue(any("must be labelled render-call return" in error for error in errors))

    def test_gpu_mode_must_match_resolved_adapter_kind(self):
        errors: list[str] = []
        baseline._validate_gpu("gpu_2d_default", gpu_records("gpu_2d_default", software=True),
                               errors, gpu_mode="default")
        self.assertTrue(any("hardware baseline is unverified" in error for error in errors))
        errors = []
        baseline._validate_gpu("gpu_2d_default", gpu_records("gpu_2d_default"), errors,
                               gpu_mode="software")
        self.assertTrue(any("did not resolve to a software adapter" in error for error in errors))

    def test_failed_scheduled_repetition_never_summarizes_as_complete(self):
        rows = [{"suite_name": "gpu_2d_default", "exit_code": 101, "timed_out": False,
                 "launch_error": None, "validation": {"valid": False, "errors": ["assertion failed"],
                                                       "summary": {"signature": {"scene_id": "grid_1000_v1"}}}},
                {"suite_name": "gpu_2d_default", "exit_code": 0, "timed_out": False,
                 "launch_error": None, "validation": {"valid": True, "errors": [],
                                                       "summary": {"signature": {"scene_id": "grid_1000_v1"}}}}]
        errors: list[str] = []
        baseline._validate_repetitions(rows, errors)
        self.assertTrue(any("failed or timed out" in error for error in errors))
        self.assertTrue(any("assertion failed" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
