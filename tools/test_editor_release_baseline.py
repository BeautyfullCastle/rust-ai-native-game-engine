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

    def test_collect_persists_malformed_key_outcomes_and_marks_manifest_incomplete(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            output = root / "capture"
            counts = {"editor_measure": 0, "gpu_2d_default": 0,
                      "gpu_3d_low": 0, "gpu_3d_default": 0}
            identity = {"head": "commit", "tree": "tree", "dirty_status": "",
                        "source_sha256": "a" * 64, "source_file_count": 1}

            def fake_launch(argv, env, cwd, run_dir):
                result = {"exit_code": 0, "timed_out": False, "interrupted": False,
                          "launch_error": None, "child_cleanup_verified": True,
                          "elapsed_seconds": 0.01, "started_at": "start", "finished_at": "finish",
                          "cleanup": {"action": "normal_exit"}}
                if "--no-run" in argv:
                    (run_dir / "stdout.log").write_text("build ok\n", encoding="utf-8")
                    (run_dir / "stderr.log").write_text("", encoding="utf-8")
                    return result
                if "orr_editor" in argv:
                    suite = "editor_measure"
                    records = editor_records()
                    if counts[suite] == 0:
                        next(row for row in records if row.get("case") == "drag_move")["move_index"] = None
                else:
                    target = argv[argv.index("--test") + 1]
                    suite = {"gpu": "gpu_2d_default", "gpu3d":
                             ("gpu_3d_low" if "release_baseline_3d_low" in argv else "gpu_3d_default")}[target]
                    records = gpu_records(suite)
                    if counts[suite] == 0:
                        frame = next(row for row in records if row.get("case") == "frame")
                        frame["frame_class"] = None
                        frame["frame_index"] = []
                counts[suite] += 1
                (run_dir / "stdout.log").write_text("", encoding="utf-8")
                with (run_dir / "stderr.log").open("w", encoding="utf-8") as stream:
                    for record in records:
                        stream.write("ORR_BASELINE " + json.dumps(record) + "\n")
                return result

            args = type("Args", (), {"output": "capture", "repetitions": 3, "gpu_mode": "default"})()
            with patch.object(baseline, "REPO", root), \
                 patch.object(baseline, "_source_identity", return_value=identity), \
                 patch.object(baseline, "_toolchain", return_value={"rustc_verbose": "test"}), \
                 patch.object(baseline, "_power_metadata", return_value={"active_scheme": "test"}), \
                 patch.object(baseline, "_output_path", return_value=output), \
                 patch.object(baseline, "_compiled_binary", return_value={"target": "test", "sha256": "b" * 64, "bytes": 1}), \
                 patch.object(baseline, "_launch", side_effect=fake_launch):
                result = baseline.collect(args)
            self.assertEqual(result, 1)
            manifest = json.loads((output / "manifest.json").read_text(encoding="utf-8"))
            self.assertEqual(manifest["status"], "incomplete")
            self.assertEqual(len(manifest["build_runs"]), 3)
            self.assertEqual(len(manifest["runs"]), 12)
            self.assertEqual({name: sum(row["suite_name"] == name for row in manifest["runs"])
                              for name in counts}, {name: 3 for name in counts})
            editor = next(row for row in manifest["runs"] if row["suite_name"] == "editor_measure" and row["repetition"] == 1)
            gpu = next(row for row in manifest["runs"] if row["suite_name"] == "gpu_2d_default" and row["repetition"] == 1)
            self.assertEqual(editor["exit_code"], 0)
            self.assertEqual(gpu["exit_code"], 0)
            self.assertEqual(editor["record_count"], len(editor_records()))
            self.assertEqual(gpu["record_count"], 43)
            self.assertFalse(editor["validation"]["valid"])
            self.assertFalse(gpu["validation"]["valid"])
            self.assertIn("records.jsonl", editor["record_file"])
            self.assertIn("records.jsonl", gpu["record_file"])
            for name in counts:
                for repetition in (2, 3):
                    row = next(row for row in manifest["runs"] if row["suite_name"] == name and
                               row["repetition"] == repetition)
                    self.assertTrue(row["validation"]["valid"], (name, repetition, row["validation"]["errors"]))
            persisted_editor = [json.loads(line) for line in
                                (output / "rep-01-editor_measure" / "records.jsonl").read_text(encoding="utf-8").splitlines()]
            self.assertEqual(len(persisted_editor), 585)
            self.assertIsNone(next(row for row in persisted_editor if row.get("case") == "drag_move")["move_index"])
            persisted_gpu = [json.loads(line) for line in
                             (output / "rep-01-gpu_2d_default" / "records.jsonl").read_text(encoding="utf-8").splitlines()]
            self.assertEqual(len(persisted_gpu), 43)
            malformed_frame = next(row for row in persisted_gpu if row.get("frame_index") == [])
            self.assertIsNone(malformed_frame["frame_class"])
            self.assertEqual(malformed_frame["frame_index"], [])

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

    def test_editor_malformed_sort_and_phase_keys_are_rejected_without_raising(self):
        for key_value in (None, "2", [], True):
            rows = editor_records()
            next(row for row in rows if row.get("case") == "drag_move")["move_index"] = key_value
            errors: list[str] = []
            with self.subTest(move_index=key_value):
                summary = baseline._validate_editor(rows, errors)
                self.assertTrue(any("indices must be integers" in error for error in errors))
                self.assertTrue(any("drag_move records" in error for error in errors))
                self.assertIsInstance(summary, dict)
        for malformed_phase in ([], {}):
            rows = editor_records()
            next(row for row in rows if row.get("case") == "diagnostics")["phase"] = malformed_phase
            errors = []
            with self.subTest(diagnostic_phase=malformed_phase):
                baseline._validate_editor(rows, errors)
                self.assertTrue(any("phase keys must be strings" in error for error in errors))
        rows = editor_records()
        next(row for row in rows if row.get("case") == "drag_move")["body_count"] = []
        errors = []
        baseline._validate_editor(rows, errors)
        self.assertTrue(any("body_count must be a positive integer" in error for error in errors))

    def test_editor_zero_sample_timing_cannot_be_reported_as_a_measurement(self):
        rows = editor_records()
        diagnostic = next(row for row in rows if row.get("case") == "diagnostics")
        diagnostic["metrics"]["ui_frame"]["last_ms"] = 0.25
        errors: list[str] = []
        baseline._validate_editor(rows, errors)
        self.assertTrue(any("timings without samples" in error for error in errors))


class RendererContractTests(unittest.TestCase):
    def test_dx12_empty_driver_info_is_unavailable_but_missing_info_fails(self):
        records = gpu_records("gpu_2d_default", software=True)
        for record in records:
            if record["case"] in ("metadata", "frame"):
                record["adapter_backend"] = "Dx12"
                record["driver_info"] = ""
        errors = []
        summary = baseline._validate_gpu("gpu_2d_default", records, errors, "software")
        self.assertEqual(errors, [])
        self.assertFalse(summary["driver_metadata_availability"]["driver_info"])
        del records[0]["driver_info"]
        errors = []
        baseline._validate_gpu("gpu_2d_default", records, errors, "software")
        self.assertTrue(any("present string" in error for error in errors))

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

    def test_gpu_malformed_frame_pair_keys_are_rejected_without_raising(self):
        for field, value in (("frame_class", None), ("frame_class", []),
                             ("frame_index", None), ("frame_index", []), ("frame_index", True)):
            rows = gpu_records("gpu_2d_default")
            frame = next(row for row in rows if row.get("case") == "frame")
            frame[field] = value
            errors: list[str] = []
            with self.subTest(field=field, value=value):
                summary = baseline._validate_gpu("gpu_2d_default", rows, errors, "default")
                self.assertIsNotNone(summary)
                self.assertTrue(any("frame class/index keys" in error for error in errors))
                self.assertTrue(any("exact indices" in error for error in errors))

    def test_gpu_unhashable_case_key_is_rejected_without_raising(self):
        rows = gpu_records("gpu_2d_default")
        rows[1]["case"] = []
        errors: list[str] = []
        summary = baseline._validate_gpu("gpu_2d_default", rows, errors, "default")
        self.assertIsNotNone(summary)
        self.assertTrue(any("expected exactly 1 metadata" in error for error in errors))

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
