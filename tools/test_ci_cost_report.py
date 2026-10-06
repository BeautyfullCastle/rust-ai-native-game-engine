#!/usr/bin/env python3
"""Regression tests for offline CI timing comparisons (no builds or benchmarks)."""

from __future__ import annotations

import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import os
import platform
import hashlib
import io
import threading

import ci_cost_report as report


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "tools" / "ci_cost_report.py"


def record(record_id: str, *, run_id: int, seconds=("20", "10", "30")):
    """A complete caller-declared capture whose source and checkout trees match."""
    return {
        "id": record_id,
        "provenance": {
            "source_sha": f"{run_id:040x}",
            "checkout_sha": f"{run_id + 100:040x}",
            "source_tree_sha": "a" * 40,
            "checkout_tree_sha": "a" * 40,
            "run_id": run_id,
            "run_attempt": 1,
            "job_id": run_id + 200,
            "artifact": {
                "name": f"timings-{record_id}",
                "locator": f"run/{run_id}/artifact/{record_id}",
                "sha256": f"{run_id:064x}",
            },
        },
        "conditions": {
            "platform": {"os": "ubuntu-24.04", "arch": "x86_64", "image": "image-123"},
            "runner": {"class": "hosted", "hardware": "x64", "isolation": "exclusive"},
            "toolchain": "rustc-1.99.0",
            "profile": "release",
            "target": "x86_64-unknown-linux-gnu",
            "features": ["default"],
            "command": ["cargo", "test", "--locked"],
            "cache": {"state": "cold", "key": "cache-key-v1"},
        },
        "status": "success",
        "exit_code": 0,
        "measurements": {
            "compile": {"seconds": seconds[0], "scope": "cargo-build-wall", "source": "cargo-timings"},
            "runtime": {"seconds": seconds[1], "scope": "test-execution-wall", "source": "test-log"},
            "total": {"seconds": seconds[2], "scope": "job-wall", "source": "workflow-clock"},
        },
    }


def document(base=None, current=None):
    base = base or record("before", run_id=1)
    current = current or record("after", run_id=2)
    return {
        "schema": report.INPUT_SCHEMA,
        "records": [base, current],
        "comparisons": [{"baseline": base["id"], "current": current["id"]}],
    }


def comparison(result):
    return result["comparisons"][0]


class CiCostReportTests(unittest.TestCase):
    def test_independent_measurements_report_faster_slower_and_unchanged(self):
        result = comparison(report.build_report(document(
            record("before", run_id=1, seconds=("20", "12", "7")),
            record("after", run_id=2, seconds=("10", "18", "7")),
        )))
        self.assertEqual(result["status"], "comparable")
        self.assertEqual(result["metrics"]["compile"]["direction"], "faster")
        self.assertEqual(result["metrics"]["compile"]["ratio_baseline_over_current"], "2")
        self.assertEqual(result["metrics"]["compile"]["change_seconds"], "-10")
        self.assertEqual(result["metrics"]["runtime"]["direction"], "slower")
        self.assertEqual(result["metrics"]["runtime"]["ratio_baseline_over_current"],
                         "0.6666666666666666666666666667")
        self.assertEqual(result["metrics"]["runtime"]["change_seconds"], "6")
        self.assertEqual(result["metrics"]["total"]["direction"], "unchanged")
        self.assertEqual(result["metrics"]["total"]["ratio_baseline_over_current"], "1")
        self.assertIn("no causal or statistical", result["interpretation"])

    def test_declared_artifact_ids_and_commit_shas_may_differ_but_trees_must_match(self):
        baseline = record("base", run_id=11)
        current = record("candidate", run_id=22)
        current["provenance"]["source_tree_sha"] = "b" * 40
        current["provenance"]["checkout_tree_sha"] = "b" * 40
        current["provenance"]["artifact"]["name"] = "different-upload"
        result = comparison(report.build_report(document(baseline, current)))
        self.assertTrue(result["metrics"]["compile"]["comparable"])
        self.assertNotIn("condition_mismatch", " ".join(result["common_reasons"]))
        self.assertEqual(report.build_report(document())["provenance_verification"],
                         "caller-declared; artifacts are not fetched or attested")

    def test_every_relevant_condition_must_match(self):
        mutations = (
            ("platform.os", lambda r: r["conditions"]["platform"].__setitem__("os", "windows")),
            ("platform.arch", lambda r: r["conditions"]["platform"].__setitem__("arch", "arm64")),
            ("platform.image", lambda r: r["conditions"]["platform"].__setitem__("image", "image-456")),
            ("runner.class", lambda r: r["conditions"]["runner"].__setitem__("class", "self-hosted")),
            ("runner.hardware", lambda r: r["conditions"]["runner"].__setitem__("hardware", "different")),
            ("runner.isolation", lambda r: r["conditions"]["runner"].__setitem__("isolation", "shared")),
            ("toolchain", lambda r: r["conditions"].__setitem__("toolchain", "rustc-other")),
            ("profile", lambda r: r["conditions"].__setitem__("profile", "dev")),
            ("target", lambda r: r["conditions"].__setitem__("target", "other-target")),
            ("features", lambda r: r["conditions"].__setitem__("features", ["default", "extra"])),
            ("command", lambda r: r["conditions"].__setitem__("command", ["cargo", "check"])),
            ("cache.state", lambda r: r["conditions"]["cache"].__setitem__("state", "warm")),
            ("cache.key", lambda r: r["conditions"]["cache"].__setitem__("key", "different-cache")),
        )
        for field, mutate in mutations:
            with self.subTest(field=field):
                baseline = record("base", run_id=31)
                current = record("current", run_id=32)
                mutate(current)
                result = comparison(report.build_report(document(baseline, current)))
                self.assertFalse(result["metrics"]["compile"]["comparable"])
                self.assertTrue(result["common_reasons"])
                self.assertIsNone(result["metrics"]["compile"].get("ratio_baseline_over_current"))

    def test_unknown_condition_keys_are_preserved_and_block_every_metric(self):
        cases = (
            ("root_env_differs", (True, True), lambda base, current: (
                base["conditions"].__setitem__("env", {"RUSTFLAGS": "-C panic=abort"}),
                current["conditions"].__setitem__("env", {"RUSTFLAGS": "-C panic=unwind"}))),
            ("platform_extra_equal", (True, True), lambda base, current: (
                base["conditions"]["platform"].__setitem__("provider", "linux"),
                current["conditions"]["platform"].__setitem__("provider", "linux"))),
            ("runner_extra_null_on_baseline", (True, False), lambda base, current:
             base["conditions"]["runner"].__setitem__("host_image_digest", None)),
            ("cache_extra_only_on_current", (False, True), lambda base, current:
             current["conditions"]["cache"].__setitem__("implementation", "local-fs")),
            ("root_extra_null_both", (True, True), lambda base, current: (
                base["conditions"].__setitem__("env", None),
                current["conditions"].__setitem__("env", None))),
        )
        for label, invalid_sides, mutate in cases:
            with self.subTest(case=label):
                baseline = record("base", run_id=181)
                current = record("current", run_id=182)
                mutate(baseline, current)
                result = report.build_report(document(baseline, current))
                self.assertEqual(result["records"][0]["input"], baseline)
                self.assertEqual(result["records"][1]["input"], current)
                for row, invalid in zip(result["records"], invalid_sides):
                    self.assertEqual(bool(row["validation_errors"]), invalid)
                pair = result["comparisons"][0]
                for metric in pair["metrics"].values():
                    self.assertFalse(metric["comparable"])
                    self.assertNotIn("ratio_baseline_over_current", metric)
                self.assertEqual(pair["status"], "incomparable")

    def test_unknown_condition_key_cli_returns_incomparable_json_not_global_error(self):
        baseline = record("base", run_id=191)
        current = record("current", run_id=192)
        baseline["conditions"]["env"] = {"RUSTFLAGS": "-C panic=abort"}
        current["conditions"]["env"] = {"RUSTFLAGS": "-C panic=unwind"}
        with tempfile.TemporaryDirectory(prefix="ci-cost-unknown-condition-") as temp:
            path = Path(temp) / "unknown-condition.json"
            path.write_text(json.dumps(document(baseline, current)), encoding="utf-8")
            result = subprocess.run([sys.executable, str(SCRIPT), str(path), "--format", "json"],
                                    cwd=ROOT, text=True, encoding="utf-8", capture_output=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")
        output = json.loads(result.stdout)
        self.assertEqual(output["comparisons"][0]["status"], "incomparable")
        self.assertEqual(output["records"][0]["input"]["conditions"]["env"],
                         {"RUSTFLAGS": "-C panic=abort"})
        self.assertEqual(output["records"][1]["input"]["conditions"]["env"],
                         {"RUSTFLAGS": "-C panic=unwind"})
        self.assertTrue(all(row["validation_errors"] for row in output["records"]))
        for metric in output["comparisons"][0]["metrics"].values():
            self.assertFalse(metric["comparable"])
            self.assertNotIn("ratio_baseline_over_current", metric)

    def test_measurement_sources_are_labels_but_scope_mismatch_blocks_ratio(self):
        baseline = record("base", run_id=201)
        current = record("current", run_id=202)
        for metric in report.METRICS:
            baseline["measurements"][metric]["source"] = f"artifacts/run-1/{metric}.log"
            current["measurements"][metric]["source"] = f"artifacts/run-2/{metric}.log"
        comparable = comparison(report.build_report(document(baseline, current)))
        self.assertEqual(comparable["status"], "comparable")
        self.assertTrue(all(item["comparable"] for item in comparable["metrics"].values()))

        baseline["measurements"]["total"]["scope"] = "job-wall"
        current["measurements"]["total"]["scope"] = "command-wall"
        current["measurements"]["total"]["source"] = "a-different-log-path.json"
        result = comparison(report.build_report(document(baseline, current)))
        self.assertTrue(result["metrics"]["compile"]["comparable"])
        self.assertTrue(result["metrics"]["runtime"]["comparable"])
        self.assertFalse(result["metrics"]["total"]["comparable"])
        self.assertIn("measurement_scope_mismatch", result["metrics"]["total"]["reasons"])
        self.assertNotIn("ratio_baseline_over_current", result["metrics"]["total"])

    def test_failed_cancelled_timeout_or_unknown_runs_never_make_a_speedup(self):
        for status, code in (("failure", 1), ("cancelled", None), ("timed_out", 124), ("unknown", 0)):
            with self.subTest(status=status):
                baseline = record("base", run_id=41)
                current = record("current", run_id=42, seconds=("1", "1", "1"))
                current["status"] = status
                current["exit_code"] = code
                result = comparison(report.build_report(document(baseline, current)))
                self.assertFalse(result["metrics"]["compile"]["comparable"])
                self.assertIn("current: not_successful_exit0", result["common_reasons"])
                self.assertNotIn("ratio_baseline_over_current", result["metrics"]["compile"])

    def test_source_checkout_mismatch_runner_sharing_and_same_capture_block_pairs(self):
        cases = (
            ("tree mismatch", lambda r: r["provenance"].__setitem__("checkout_tree_sha", "c" * 40),
             "current: source_checkout_tree_mismatch"),
            ("shared runner", lambda r: r["conditions"]["runner"].__setitem__("isolation", "shared"),
             "current: runner_not_exclusive"),
        )
        for label, mutate, reason in cases:
            with self.subTest(label=label):
                baseline = record("base", run_id=51)
                current = record("current", run_id=52)
                mutate(current)
                result = comparison(report.build_report(document(baseline, current)))
                self.assertIn(reason, result["common_reasons"])
                self.assertFalse(result["metrics"]["runtime"]["comparable"])

        same = record("same", run_id=60)
        same_other_id = copy.deepcopy(same)
        same_other_id["id"] = "same-copy"
        result = comparison(report.build_report(document(same, same_other_id)))
        self.assertIn("same_capture", result["common_reasons"])
        self.assertFalse(result["metrics"]["total"]["comparable"])

    def test_bad_or_missing_measurement_has_no_ratio_and_does_not_infer_sums(self):
        bad_values = (None, [], {}, "0", "-1", True, "NaN", "Infinity", "1e999999",
                      "0.0000000001", "x" * 41)
        for value in bad_values:
            with self.subTest(seconds=value):
                baseline = record("base", run_id=71)
                baseline["measurements"]["compile"]["seconds"] = value
                result = comparison(report.build_report(document(baseline, record("current", run_id=72))))
                self.assertFalse(result["metrics"]["compile"]["comparable"])
                self.assertNotIn("ratio_baseline_over_current", result["metrics"]["compile"])

        baseline = record("base", run_id=73, seconds=("4", "5", "100"))
        current = record("current", run_id=74, seconds=("3", "6", "8"))
        result = comparison(report.build_report(document(baseline, current)))
        self.assertEqual(result["metrics"]["total"]["baseline_seconds"], "100")
        self.assertEqual(result["metrics"]["total"]["current_seconds"], "8")
        self.assertEqual(result["metrics"]["total"]["change_seconds"], "-92")

    def test_incompatible_scopes_unknown_sources_and_partial_comparability(self):
        baseline = record("base", run_id=81)
        current = record("current", run_id=82)
        current["measurements"]["runtime"]["scope"] = "mixed-build-runtime"
        current["measurements"]["runtime"]["source"] = "unknown"
        current["measurements"]["total"]["seconds"] = None
        result = comparison(report.build_report(document(baseline, current)))
        self.assertEqual(result["status"], "partially_comparable")
        self.assertTrue(result["metrics"]["compile"]["comparable"])
        self.assertFalse(result["metrics"]["runtime"]["comparable"])
        self.assertFalse(result["metrics"]["total"]["comparable"])
        self.assertIn("current: unknown_or_mixed_scope", result["metrics"]["runtime"]["reasons"])
        self.assertIn("current: unknown_measurement_source", result["metrics"]["runtime"]["reasons"])

    def test_unhashable_cache_state_and_measurement_scope_are_record_errors_not_crashes(self):
        malformed_values = ([], {}, ["cold"], {"cold": True}, True)
        for field in ("cache.state", "measurements.compile.scope"):
            for value in malformed_values:
                with self.subTest(field=field, value=value):
                    baseline = record("base", run_id=91)
                    current = record("current", run_id=92)
                    if field == "cache.state":
                        current["conditions"]["cache"]["state"] = value
                    else:
                        current["measurements"]["compile"]["scope"] = value
                    result = report.build_report(document(baseline, current))
                    row = next(item for item in result["records"] if item["input"]["id"] == "current")
                    self.assertTrue(row["validation_errors"])
                    pair = comparison(result)
                    self.assertFalse(pair["metrics"]["compile"]["comparable"])
                    self.assertNotIn("ratio_baseline_over_current", pair["metrics"]["compile"])

    def test_unknown_sentinels_and_wrongly_typed_provenance_cannot_look_known(self):
        sentinels = ("unknown", " UNKNOWN ", "UnAvAiLaBlE", " n/A ")
        condition_mutations = (
            ("platform.os", lambda r, value: r["conditions"]["platform"].__setitem__("os", value)),
            ("platform.arch", lambda r, value: r["conditions"]["platform"].__setitem__("arch", value)),
            ("platform.image", lambda r, value: r["conditions"]["platform"].__setitem__("image", value)),
            ("runner.class", lambda r, value: r["conditions"]["runner"].__setitem__("class", value)),
            ("runner.hardware", lambda r, value: r["conditions"]["runner"].__setitem__("hardware", value)),
            ("runner.isolation", lambda r, value: r["conditions"]["runner"].__setitem__("isolation", value)),
            ("toolchain", lambda r, value: r["conditions"].__setitem__("toolchain", value)),
            ("profile", lambda r, value: r["conditions"].__setitem__("profile", value)),
            ("target", lambda r, value: r["conditions"].__setitem__("target", value)),
            ("cache.key", lambda r, value: r["conditions"]["cache"].__setitem__("key", value)),
            ("features", lambda r, value: r["conditions"].__setitem__("features", ["default", value])),
            ("command", lambda r, value: r["conditions"].__setitem__("command", ["cargo", value])),
            ("measurement.source", lambda r, value: r["measurements"]["compile"].__setitem__("source", value)),
        )
        for field, mutate in condition_mutations:
            for sentinel in sentinels:
                with self.subTest(field=field, sentinel=sentinel):
                    baseline = record("base", run_id=151)
                    current = record("current", run_id=152)
                    mutate(current, sentinel)
                    result = comparison(report.build_report(document(baseline, current)))
                    metric = result["metrics"]["compile"]
                    self.assertFalse(metric["comparable"])
                    self.assertNotIn("ratio_baseline_over_current", metric)

        for field in ("artifact.name", "artifact.locator"):
            for value in ("unknown", " UNKNOWN ", "Unavailable", "n/a", 123, True, [], {}):
                with self.subTest(field=field, value=value):
                    baseline = record("base", run_id=153)
                    current = record("current", run_id=154)
                    current["provenance"]["artifact"][field.split(".")[1]] = value
                    result = comparison(report.build_report(document(baseline, current)))
                    self.assertFalse(result["metrics"]["total"]["comparable"])
                    self.assertTrue(any("unknown_provenance." + field in reason
                                        for reason in result["common_reasons"]))

        for field in ("source_sha", "checkout_sha", "source_tree_sha", "checkout_tree_sha"):
            baseline = record("base", run_id=155)
            current = record("current", run_id=156)
            current["provenance"][field] = " unknown "
            result = report.build_report(document(baseline, current))
            invalid = next(row for row in result["records"] if row["input"]["id"] == "current")
            self.assertTrue(invalid["validation_errors"], field)
            self.assertFalse(result["comparisons"][0]["metrics"]["compile"]["comparable"])

        for field in ("artifact.name", "artifact.locator"):
            baseline = record("base", run_id=157)
            current = record("current", run_id=158)
            current["provenance"]["artifact"][field.split(".")[1]] = 123
            result = report.build_report(document(baseline, current))
            self.assertFalse(result["comparisons"][0]["metrics"]["runtime"]["comparable"])

    def test_known_metadata_containers_and_boolean_numbers_are_rejected_fail_closed(self):
        malformed_values = ([], {}, True, 0, 1, None)
        for path in ("platform.os", "runner.class", "profile", "toolchain", "target", "cache.key"):
            for value in malformed_values:
                with self.subTest(path=path, value=value):
                    baseline = record("base", run_id=161)
                    current = record("current", run_id=162)
                    if path.startswith("platform."):
                        current["conditions"]["platform"][path.split(".")[1]] = value
                    elif path.startswith("runner."):
                        current["conditions"]["runner"][path.split(".")[1]] = value
                    elif path == "cache.key":
                        current["conditions"]["cache"]["key"] = value
                    else:
                        current["conditions"][path] = value
                    result = report.build_report(document(baseline, current))
                    self.assertFalse(result["comparisons"][0]["metrics"]["compile"]["comparable"])

        for key in ("run_id", "run_attempt", "job_id"):
            for value in (True, False, 0, -1, [], {}, "1", "unknown"):
                with self.subTest(key=key, value=value):
                    baseline = record("base", run_id=171)
                    current = record("current", run_id=172)
                    current["provenance"][key] = value
                    result = comparison(report.build_report(document(baseline, current)))
                    self.assertFalse(result["metrics"]["compile"]["comparable"])
                    self.assertTrue(any("unknown_provenance." + key in reason
                                        for reason in result["common_reasons"]))

    def test_malformed_nested_record_shapes_are_retained_with_errors(self):
        mutations = (
            ("provenance", []),
            ("conditions", "shared"),
            ("measurements", False),
        )
        for field, value in mutations:
            with self.subTest(field=field, value=value):
                current = record("current", run_id=102)
                current[field] = value
                result = report.build_report(document(record("base", run_id=101), current))
                row = next(item for item in result["records"] if item["input"]["id"] == "current")
                self.assertTrue(row["validation_errors"])
                for metric in result["comparisons"][0]["metrics"].values():
                    self.assertFalse(metric["comparable"])

        current = record("current", run_id=104)
        current["conditions"]["features"] = ["default", True]
        current["conditions"]["command"] = "cargo test"
        current["conditions"]["platform"] = ["windows", "x64"]
        result = report.build_report(document(record("base", run_id=103), current))
        row = next(item for item in result["records"] if item["input"]["id"] == "current")
        self.assertGreaterEqual(len(row["validation_errors"]), 3)
        self.assertEqual(result["comparisons"][0]["status"], "incomparable")

    def test_invalid_provenance_exit_code_status_and_duplicate_ids_fail_closed(self):
        malformed = record("bad", run_id=111)
        malformed["provenance"]["source_sha"] = "A" * 40
        malformed["provenance"]["run_id"] = True
        malformed["provenance"]["artifact"]["sha256"] = "f" * 63
        malformed["exit_code"] = False
        malformed["status"] = []
        result = report.build_report({"schema": report.INPUT_SCHEMA, "records": [malformed], "comparisons": []})
        errors = result["records"][0]["validation_errors"]
        self.assertGreaterEqual(len(errors), 5)

        item = record("duplicate", run_id=112)
        with self.assertRaisesRegex(ValueError, "duplicate record id"):
            report.build_report({"schema": report.INPUT_SCHEMA, "records": [item, copy.deepcopy(item)]})
        with self.assertRaisesRegex(ValueError, "unknown record"):
            report.build_report({"schema": report.INPUT_SCHEMA, "records": [item],
                                 "comparisons": [{"baseline": "duplicate", "current": "missing"}]})

    def test_json_parser_rejects_duplicate_keys_nonfinite_numbers_and_bad_shapes(self):
        for text in (
            '{"schema":"orr.ci-cost-input/1","schema":"orr.ci-cost-input/1","records":[]}',
            '{"schema":"orr.ci-cost-input/1","records":[],"x":NaN}',
            '{"schema":"orr.ci-cost-input/1","records":[],"x":Infinity}',
        ):
            with self.subTest(text=text), self.assertRaises(ValueError):
                report.loads_document(text)
        for value in ([], None, 3, True):
            with self.subTest(value=value), self.assertRaises(ValueError):
                report.build_report(value)
        with self.assertRaisesRegex(ValueError, "unknown top-level"):
            report.build_report({"schema": report.INPUT_SCHEMA, "records": [record("one", run_id=121)],
                                 "unexpected": 1})

    def test_cli_json_markdown_and_bad_utf8_exit_contract(self):
        sample = document(record("base", run_id=131), record("current", run_id=132))
        sample["records"][0]["measurements"]["compile"]["seconds"] = 10.125
        sample["records"][1]["measurements"]["compile"]["seconds"] = 9.5
        with tempfile.TemporaryDirectory(prefix="ci-cost-report-") as temp:
            path = Path(temp) / "input.json"
            path.write_text(json.dumps(sample), encoding="utf-8")
            result = subprocess.run([sys.executable, str(SCRIPT), str(path), "--format", "json"],
                                    cwd=ROOT, text=True, encoding="utf-8", capture_output=True, check=False)
            self.assertEqual(result.returncode, 0, result.stderr)
            output = json.loads(result.stdout)
            self.assertEqual(output["schema"], report.REPORT_SCHEMA)
            self.assertEqual(output["comparisons"][0]["status"], "comparable")
            self.assertEqual(output["records"][0]["input"]["measurements"]["compile"]["seconds"], "10.125")
            self.assertEqual(output["comparisons"][0]["metrics"]["compile"]["baseline_seconds"], "10.125")
            self.assertEqual(output["comparisons"][0]["metrics"]["compile"]["current_seconds"], "9.5")

            markdown = subprocess.run([sys.executable, str(SCRIPT), "-", "--format", "markdown"],
                                      cwd=ROOT, input=json.dumps(sample), text=True, encoding="utf-8",
                                      capture_output=True, check=False)
            self.assertEqual(markdown.returncode, 0, markdown.stderr)
            self.assertIn("Provenance is caller-declared", markdown.stdout)
            self.assertIn("base → current: comparable", markdown.stdout)

            bad_path = Path(temp) / "invalid.json"
            bad_path.write_bytes(b"\xff")
            invalid = subprocess.run([sys.executable, str(SCRIPT), str(bad_path)], cwd=ROOT,
                                     text=True, encoding="utf-8", capture_output=True, check=False)
            self.assertEqual(invalid.returncode, 2)
            self.assertIn("ci-cost-report:", invalid.stderr)

    def test_markdown_escapes_multiline_html_and_table_syntax_but_json_keeps_original_ids(self):
        hostile_id = "base</td>\n<script>alert(1)</script>| `code` # [link](url)"
        data = document(record(hostile_id, run_id=141), record("current", run_id=142))
        result = report.build_report(data)
        rendered = report.render_markdown(result)
        safe = report.render_markdown(report.build_report(
            document(record("base", run_id=141), record("current", run_id=142))))

        self.assertEqual(result["comparisons"][0]["baseline"], hostile_id)
        self.assertEqual(result["records"][0]["input"]["id"], hostile_id)
        self.assertNotIn("<script>", rendered)
        self.assertNotIn("</td>", rendered)
        self.assertNotIn("\n<script>", rendered)
        self.assertIn("&lt;script&gt;", rendered)
        self.assertIn("\\|", rendered)
        self.assertIn("\\`", rendered)
        self.assertIn("\\#", rendered)
        self.assertEqual(len(rendered.splitlines()), len(safe.splitlines()))


def local_plan(root):
    """Synthetic plan metadata; no source checkout or capture is executed."""
    root = Path(root).resolve()
    bin_dir = root / "installed-bin"
    suffix = ".exe" if os.name == "nt" else ""
    toolchain = {
        "cargo_path": str(bin_dir / ("cargo" + suffix)),
        "rustc_path": str(bin_dir / ("rustc" + suffix)),
        "rustdoc_path": str(bin_dir / ("rustdoc" + suffix)),
        "cargo_version": "cargo 1.97.1 (synthetic)",
        "rustc_version": "rustc 1.97.1 (synthetic)",
        "rustdoc_version": "rustdoc 1.97.1 (synthetic)",
    }
    receipt = {"path": str(root / "receipt.json"), "sha256": "a" * 64}
    command = ["cargo", "test", "--workspace", "--release", "--timings",
               "--exclude", "orr_sample", "--exclude", "orr_editor", "--exclude", "orr_web_gpu"]
    captures = []
    for side in ("baseline", "current"):
        target = str(root / (side + "-target"))
        environment = {
            "CARGO_NET_OFFLINE": "true", "CARGO_TARGET_DIR": target,
            "RUSTC": toolchain["rustc_path"], "RUSTDOC": toolchain["rustdoc_path"],
            "RUSTFLAGS": None, "CARGO_ENCODED_RUSTFLAGS": None,
        }
        captures.append({"id": side + "-native", "side": side,
                         "lane": "test-native", "workload": "native",
                         "checkout": str(root / side), "command": command[:],
                         "environment": environment,
                         "cache": {"state": "unverified", "key": "same-preparation-v1",
                                   "target_dir": target, "prepared": True, "receipt": copy.deepcopy(receipt)}})
    return {"schema": "orr.ci-cost-local-plan/1", "plan_id": "synthetic-plan-v1",
            "once_dir": str(root / "once"),
            "baseline": {"sha": "1" * 40, "tree_sha": "3" * 40},
            "current": {"sha": "2" * 40, "tree_sha": "4" * 40},
            "runner": {"os": platform.system(), "arch": platform.machine(),
                       "class": "coordinated-local", "hardware": "synthetic-test-runner",
                       "isolation": "coordinated", "toolchain": toolchain},
            "coordination": {"lane_receipt": copy.deepcopy(receipt),
                             "n6_receipt": copy.deepcopy(receipt), "n6_completed": True},
            "watchdog": {"seconds": 10, "max_log_bytes": 1048576,
                         "cleanup": "windows-job" if os.name == "nt" else "posix-process-group"},
            "captures": captures}


class LocalCollectorPlanTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ci-local-plan-tests-")
        self.addCleanup(self.temp.cleanup)
        self.plan = local_plan(self.temp.name)

    def test_validation_is_offline_and_keeps_local_identity_separate(self):
        with mock.patch.object(report.subprocess, "Popen", side_effect=AssertionError("no process")), \
             mock.patch.object(report.subprocess, "run", side_effect=AssertionError("no process")):
            validated = report.validate_local_plan(self.plan)
        self.assertEqual(validated, self.plan)
        self.assertIsNot(validated, self.plan)
        self.assertNotIn("run_id", validated)
        validated["captures"][0]["command"].append("mutated")
        self.assertNotIn("mutated", self.plan["captures"][0]["command"])

    def test_n6_completion_and_actual_receipt_bindings_cannot_be_omitted(self):
        for change in (
            lambda p: p["coordination"].__setitem__("n6_completed", False),
            lambda p: p["coordination"].__setitem__("n6_completed", 1),
            lambda p: p["coordination"]["lane_receipt"].__setitem__("sha256", "not-a-digest"),
            lambda p: p["coordination"].__setitem__("n6_receipt", None),
        ):
            candidate = copy.deepcopy(self.plan); change(candidate)
            with self.subTest(candidate=candidate["coordination"]), self.assertRaises(ValueError):
                report.validate_local_plan(candidate)

    def test_seventh_capture_and_duplicate_identity_are_rejected_before_execution(self):
        too_many = copy.deepcopy(self.plan)
        too_many["captures"] = [dict(copy.deepcopy(self.plan["captures"][0]), id=f"capture-{n}") for n in range(7)]
        duplicate = copy.deepcopy(self.plan)
        duplicate["captures"][1]["id"] = duplicate["captures"][0]["id"]
        for candidate in (too_many, duplicate):
            with self.subTest(candidate=candidate), self.assertRaises(ValueError):
                report.validate_local_plan(candidate)

    def test_exact_full_workload_cannot_be_replaced_with_check_or_filtered_tests(self):
        for command in (["cargo", "check"], self.plan["captures"][0]["command"] + ["--", "only_one_test"],
                        self.plan["captures"][0]["command"] + ["--no-run"]):
            candidate = copy.deepcopy(self.plan)
            candidate["captures"][0]["command"] = command
            with self.subTest(command=command), self.assertRaises(ValueError):
                report.validate_local_plan(candidate)

    def test_environment_secrets_and_changed_pair_conditions_are_not_silently_accepted(self):
        mutations = (
            lambda p: p["captures"][0]["environment"].__setitem__("ORR_ERP_TOKEN", "secret"),
            lambda p: p["captures"][1]["environment"].__setitem__("RUSTFLAGS", "-C opt-level=1"),
            lambda p: p["captures"][1]["cache"].__setitem__("key", "different-preparation"),
            lambda p: p["captures"][0]["environment"].__setitem__("CARGO_NET_OFFLINE", "false"),
            lambda p: p["captures"][0]["environment"].__setitem__("CARGO_TARGET_DIR", str(Path(self.temp.name)/"other")),
        )
        for mutate in mutations:
            candidate = copy.deepcopy(self.plan); mutate(candidate)
            with self.subTest(candidate=candidate["captures"]), self.assertRaises(ValueError):
                report.validate_local_plan(candidate)

    def test_watchdog_and_log_bounds_require_finite_positive_limits(self):
        for field, value in (("seconds", 0), ("seconds", True), ("seconds", float("inf")),
                             ("max_log_bytes", 0), ("max_log_bytes", True),
                             ("cleanup", "kill-all-cargo")):
            candidate = copy.deepcopy(self.plan); candidate["watchdog"][field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                report.validate_local_plan(candidate)

    def test_revision_paths_and_plan_scope_must_be_explicit(self):
        mutations = (
            lambda p: p["baseline"].__setitem__("sha", "abc123"),
            lambda p: p["captures"][0].__setitem__("checkout", "relative-checkout"),
            lambda p: p.__setitem__("once_dir", "relative-markers"),
            lambda p: p.__setitem__("automatic_retries", 10),
        )
        for mutate in mutations:
            candidate = copy.deepcopy(self.plan); mutate(candidate)
            with self.subTest(candidate=candidate), self.assertRaises(ValueError):
                report.validate_local_plan(candidate)

    def test_malformed_enum_shapes_fail_validation_instead_of_crashing(self):
        for mutate in (
            lambda p: p["runner"].__setitem__("isolation", []),
            lambda p: p["watchdog"].__setitem__("cleanup", {}),
            lambda p: p["captures"][0].__setitem__("side", []),
            lambda p: p["captures"][0]["cache"].__setitem__("state", {}),
        ):
            candidate = copy.deepcopy(self.plan); mutate(candidate)
            with self.subTest(candidate=candidate), self.assertRaises(ValueError):
                report.validate_local_plan(candidate)

    def test_each_capture_requires_a_distinct_target_directory(self):
        self.plan["captures"][1]["cache"]["target_dir"] = self.plan["captures"][0]["cache"]["target_dir"]
        self.plan["captures"][1]["environment"]["CARGO_TARGET_DIR"] = self.plan["captures"][0]["cache"]["target_dir"]
        with self.assertRaises(ValueError):
            report.validate_local_plan(self.plan)


def independent_linux_plan(root):
    plan = local_plan(root)
    root = Path(root).resolve()
    plan["runner"].update(os="Linux", arch="x86_64")
    plan["watchdog"]["cleanup"] = "posix-process-group"
    plan["coordination"] = {
        "mode": "independent-linux", "n6_receipt": None, "n6_completed": False,
        "resource_receipt": {"path": str(root / "resource.json"), "sha256": "b" * 64},
        "lane_receipt": {"path": str(root / "lane.json"), "sha256": "c" * 64},
    }
    for original in plan["captures"][:]:
        capture = copy.deepcopy(original)
        capture.update(id=original["side"] + "-sample", lane="test-sample", workload="sample",
                       command=report.LOCAL_COMMANDS["sample"][:])
        target = str(root / (original["side"] + "-sample-target"))
        capture["cache"]["target_dir"] = target
        capture["environment"]["CARGO_TARGET_DIR"] = target
        plan["captures"].append(capture)
    return plan


class IndependentLinuxResourceGateTests(unittest.TestCase):
    """New gate only: synthetic receipts and metadata, never Cargo or Linux load."""
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ci-independent-linux-gate-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.plan = independent_linux_plan(self.root)
        self.resource = {
            "schema": report.LOCAL_RESOURCE_SCHEMA, "owner": "su",
            "runner_id": "linux-independent", "n6_runner_id": "windows-n6",
            "runner_machine_id_sha256": "d" * 64, "runner_os": "Linux", "runner_arch": "x86_64",
            "separate_resources": True,
        }
        self.lane = {"schema": report.LOCAL_ADMISSION_SCHEMA, "runner_id": "linux-independent",
                     "observed_at_utc": "2026-10-05T06:30:00Z", "quiet": True, "active_related_pids": []}
        self.save_receipts()
        for patch in (
            mock.patch.object(report.platform, "system", return_value="Linux"),
            mock.patch.object(report.platform, "machine", return_value="x86_64"),
            mock.patch.object(report, "_local_utc", return_value="2026-10-05T06:31:00+00:00"),
            mock.patch.object(report, "_local_linux_machine_identity", return_value={"sha256": "d" * 64}),
        ):
            patch.start(); self.addCleanup(patch.stop)

    def save_receipts(self):
        for key, document in (("resource_receipt", self.resource), ("lane_receipt", self.lane)):
            path = Path(self.plan["coordination"][key]["path"])
            raw = (json.dumps(document, sort_keys=True) + "\n").encode("utf-8")
            path.write_bytes(raw)
            self.plan["coordination"][key]["sha256"] = hashlib.sha256(raw).hexdigest()

    def admit(self):
        return report._local_independent_admission(self.plan["coordination"], self.plan["runner"])

    def prepare_mock_collection(self):
        tools = self.plan["runner"]["toolchain"]
        for name in ("cargo", "rustc", "rustdoc"):
            path = Path(tools[name + "_path"])
            path.parent.mkdir(exist_ok=True)
            path.write_bytes(("synthetic-" + name).encode("ascii"))
        cache_path = self.root / "receipt.json"
        cache_path.write_bytes(b'{"synthetic_cache":true}\n')
        for capture in self.plan["captures"]:
            capture["cache"]["receipt"]["sha256"] = hashlib.sha256(cache_path.read_bytes()).hexdigest()
        for patch in (
            mock.patch.object(report, "_local_cleanup_mode", return_value="posix-process-group"),
            mock.patch.object(report, "_local_probe", side_effect=lambda command, *args:
                              "" if command == ["ps", "-eo", "pid=,pgid="] else
                              tools[Path(command[0]).stem + "_version"]),
            mock.patch.object(report, "_local_source", side_effect=lambda checkout, expected: dict(expected, clean=True)),
            mock.patch.object(report, "_local_environment", return_value={}),
        ):
            patch.start(); self.addCleanup(patch.stop)

    @staticmethod
    def census(related=None):
        return {"observed_started_utc": report._local_utc(),
                "observed_completed_utc": report._local_utc(),
                "related_processes": related or [], "scope": "synthetic PID/comm only"}

    def execution(self, status="success"):
        def run(command, checkout, environment, directory, watchdog):
            (directory / "stdout.raw").write_bytes(b"synthetic\xff\x00")
            (directory / "stderr.raw").write_bytes(b"diagnostic\r\n")
            return {"status": status, "exit_code": 0 if status == "success" else 101,
                    "total_seconds": "1.25", "owned_process_reaped": True,
                    "streams": {key: report._local_sha(directory / (key + ".raw"))
                                for key in ("stdout", "stderr")}}
        return run

    def test_validate_only_accepts_four_explicit_pairs_without_n6_completion(self):
        with mock.patch.object(report.subprocess, "Popen", side_effect=AssertionError("no process")), \
             mock.patch.object(Path, "open", side_effect=AssertionError("no receipt read")):
            validated = report.validate_local_plan(self.plan)
        self.assertFalse(validated["coordination"]["n6_completed"])
        self.assertIsNone(validated["coordination"]["n6_receipt"])
        self.assertEqual(len(validated["captures"]), 4)

    def test_variant_rejects_windows_fake_completion_missing_pair_and_shared_lane(self):
        for mutate in (
            lambda p: p["runner"].__setitem__("os", "Windows"),
            lambda p: p["runner"].__setitem__("isolation", "shared"),
            lambda p: p["coordination"].__setitem__("mode", "bypass-n6"),
            lambda p: p["coordination"].__setitem__("n6_completed", 0),
            lambda p: p["coordination"].__setitem__("n6_completed", True),
            lambda p: p["coordination"].__setitem__("n6_receipt", {"path": str(self.root / "old"), "sha256": "a" * 64}),
            lambda p: p["coordination"].__setitem__("extra", True),
            lambda p: p["watchdog"].__setitem__("cleanup", "windows-job"),
            lambda p: p["captures"].pop(),
            lambda p: p["captures"][2].__setitem__("side", "current"),
            lambda p: p["captures"][1].__setitem__("lane", "other-native"),
            lambda p: p["captures"][2].__setitem__("lane", "test-native"),
        ):
            candidate = copy.deepcopy(self.plan); mutate(candidate)
            with self.subTest(candidate=candidate), self.assertRaises(ValueError):
                report.validate_local_plan(candidate)

    def test_admission_snapshots_hashes_identity_and_owner_attestation_boundary(self):
        admitted = self.admit()
        self.assertEqual(admitted["lane_age_seconds"], 60)
        self.assertEqual(admitted["actual_machine_identity"]["sha256"], "d" * 64)
        self.assertEqual(admitted["receipts"]["resource_receipt"]["document"], self.resource)
        self.assertIn("owner-attested", admitted["verification"])
        self.assertIn("not authenticated", admitted["verification"])

    def test_admission_rejects_stale_future_naive_or_non_utc_timestamp(self):
        for observed in ("2026-10-05T06:28:59Z", "2026-10-05T06:31:00.000001Z",
                         "2026-10-05T06:30:00", "2026-10-05T06:30:00+01:00", "not-time"):
            self.lane["observed_at_utc"] = observed; self.save_receipts()
            with self.subTest(observed=observed), self.assertRaises(ValueError):
                self.admit()
        for observed in ("2026-10-05T06:29:00Z", "2026-10-05T06:31:00Z"):
            self.lane["observed_at_utc"] = observed; self.save_receipts()
            self.assertIn(self.admit()["lane_age_seconds"], (0, 120))

    def test_admission_rejects_unknown_same_or_mismatched_resource_and_lane_identity(self):
        original_resource, original_lane = copy.deepcopy(self.resource), copy.deepcopy(self.lane)
        for mutate in (
            lambda: self.resource.__setitem__("runner_id", "unknown"),
            lambda: self.resource.__setitem__("runner_id", "linux-unknown"),
            lambda: self.resource.__setitem__("n6_runner_id", "linux-independent"),
            lambda: self.resource.__setitem__("n6_runner_id", "LINUX-INDEPENDENT"),
            lambda: self.resource.__setitem__("owner", "someone-else"),
            lambda: self.resource.__setitem__("separate_resources", 1),
            lambda: self.resource.__setitem__("runner_arch", "other-arch"),
            lambda: self.resource.__setitem__("runner_machine_id_sha256", "e" * 64),
            lambda: self.resource.__setitem__("extra", True),
            lambda: self.lane.__setitem__("runner_id", "different-runner"),
            lambda: self.lane.__setitem__("quiet", 1),
            lambda: self.lane.__setitem__("active_related_pids", [23]),
            lambda: self.lane.__setitem__("active_related_pids", {}),
        ):
            self.resource, self.lane = copy.deepcopy(original_resource), copy.deepcopy(original_lane)
            mutate(); self.save_receipts()
            with self.subTest(resource=self.resource, lane=self.lane), self.assertRaises(ValueError):
                self.admit()
        with mock.patch.object(report.platform, "system", return_value="Windows"), self.assertRaises(ValueError):
            self.admit()

    def test_bad_receipt_bytes_and_duplicate_json_keys_refuse_admission(self):
        path = Path(self.plan["coordination"]["lane_receipt"]["path"])
        path.write_bytes(b"tampered\n")
        with self.assertRaisesRegex(ValueError, "SHA256 mismatch"):
            self.admit()
        for raw in (b'{}' * 32769, b'\xff', b'{"schema":1,"schema":2}', b'{"field":NaN}'):
            path.write_bytes(raw)
            self.plan["coordination"]["lane_receipt"]["sha256"] = hashlib.sha256(raw).hexdigest()
            with self.subTest(raw=raw[:50]), self.assertRaises(ValueError):
                self.admit()

    def test_current_machine_id_requires_valid_non_placeholder_bytes(self):
        # Exercise the real identity helper underneath the setUp mock.
        helper = self.machine_identity_helper
        for raw in (b"", b"0" * 32 + b"\n", b"f" * 32, b"uninitialized\n", b"a" * 129):
            with self.subTest(raw=raw), mock.patch.object(Path, "open", return_value=io.BytesIO(raw)), \
                 self.assertRaises(ValueError):
                helper()
        raw = b"0123456789abcdef0123456789abcdef\n"
        with mock.patch.object(Path, "open", return_value=io.BytesIO(raw)):
            identity = helper()
        self.assertEqual(identity["sha256"], hashlib.sha256(raw).hexdigest())
        self.assertIn("not physical", identity["scope"])

    machine_identity_helper = staticmethod(report._local_linux_machine_identity)

    def test_census_uses_names_only_and_rejects_malformed_or_duplicate_pids(self):
        text = ("17 python\n29 cargo\n31 orr_remote_host\n41 rustc\n42 physics\n43 physics3d\n"
                "44 arena\n45 ccache\n46 sccache\n47 ninja\n48 make\n49 cmake\n")
        with mock.patch.object(report, "_local_probe", return_value=text) as probe:
            census = report._local_linux_census()
        probe.assert_called_once_with(["ps", "-eo", "pid=,comm="])
        self.assertEqual([entry["pid"] for entry in census["related_processes"]], [29, 31, 41, *range(42, 50)])
        self.assertEqual(census["observed_text_sha256"], hashlib.sha256(text.encode()).hexdigest())
        self.assertIn("mutable/truncated", census["scope"])
        for text in ("", "17 python\n17 cargo", "-3 cargo", "17", "x cargo", "17 bad\tname"):
            with self.subTest(text=text), mock.patch.object(report, "_local_probe", return_value=text), \
                 self.assertRaises(ValueError):
                report._local_linux_census()

    def test_failed_admission_refuses_workload_and_plan_marker(self):
        self.prepare_mock_collection()
        self.resource["runner_machine_id_sha256"] = "e" * 64; self.save_receipts()
        with mock.patch.object(report, "_local_run", side_effect=AssertionError("no workload")), \
             self.assertRaisesRegex(ValueError, "machine-id differs"):
            report.collect_local(self.plan, str(self.root / "output"))
        self.assertFalse((self.root / "output").exists())
        self.assertFalse(Path(self.plan["once_dir"]).exists())

    def test_long_capture_does_not_expire_snapshotted_receipt_and_resamples_each_launch(self):
        self.prepare_mock_collection()
        def run(*args):
            # Later launch has a new census, while initial receipt freshness is checked once.
            report._local_utc.return_value = "2026-10-05T12:00:00+00:00"
            return self.execution()(*args)
        with mock.patch.object(report, "_local_receipt_document", wraps=report._local_receipt_document) as snapshots, \
             mock.patch.object(report, "_local_linux_census", side_effect=lambda: self.census()) as census, \
             mock.patch.object(report, "_local_run", side_effect=run) as execution:
            result = report.collect_local(self.plan, str(self.root / "output"))
        self.assertEqual((snapshots.call_count, census.call_count, execution.call_count), (2, 4, 4))
        self.assertEqual(result["status"], "completed")
        self.assertEqual(result["comparisons"], [])
        self.assertEqual(result["resource_admission"]["lane_age_seconds"], 60)
        self.assertTrue(all("launch_census" in capture for capture in result["captures"]))
        self.assertEqual(result["captures"][1]["launch_census"]["observed_started_utc"], "2026-10-05T12:00:00+00:00")
        self.assertIsNone(result["captures"][0]["measurements"]["compile"]["seconds"])

    def test_new_related_pid_blocks_next_launch_and_preserves_census_receipt(self):
        self.prepare_mock_collection()
        occupied = self.census([{"pid": 77, "name": "cargo"}])
        with mock.patch.object(report, "_local_linux_census", side_effect=[self.census(), occupied]) as census, \
             mock.patch.object(report, "_local_run", side_effect=self.execution()) as execution:
            result = report.collect_local(self.plan, str(self.root / "output"))
        self.assertEqual((census.call_count, execution.call_count), (2, 1))
        self.assertEqual((result["status"], result["attempted_captures"]), ("failed_stop", 2))
        self.assertEqual(result["captures"][1]["launch_census"], occupied)
        self.assertNotIn("execution", result["captures"][1])
        self.assertFalse((self.root / "output" / self.plan["captures"][1]["id"] / "started.json").exists())

    def test_first_workload_failure_preserves_raw_and_never_launches_second_capture(self):
        self.prepare_mock_collection()
        with mock.patch.object(report, "_local_linux_census", return_value=self.census()) as census, \
             mock.patch.object(report, "_local_run", side_effect=self.execution("failure")) as execution:
            result = report.collect_local(self.plan, str(self.root / "output"))
        self.assertEqual((census.call_count, execution.call_count), (1, 1))
        self.assertEqual(result["status"], "failed_stop")
        self.assertEqual((self.root / "output" / "baseline-native" / "stdout.raw").read_bytes(), b"synthetic\xff\x00")
        with mock.patch.object(report, "_local_probe", side_effect=AssertionError("no metadata probe after once marker")), \
             self.assertRaisesRegex(ValueError, "rerun refused"):
            report.collect_local(self.plan, str(self.root / "second-output"))
        self.assertFalse((self.root / "second-output").exists())


class LocalCollectorExecutionTests(unittest.TestCase):
    """Exercise collection orchestration with synthetic metadata, never Cargo."""
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ci-local-execution-tests-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.plan = local_plan(self.root)
        tools = self.plan["runner"]["toolchain"]
        for name in ("cargo", "rustc", "rustdoc"):
            path = Path(tools[name + "_path"])
            path.parent.mkdir(exist_ok=True)
            path.write_bytes(("synthetic-" + name).encode("ascii"))
        receipt_path = self.root / "receipt.json"
        receipt_path.write_bytes(b'{"synthetic":true}\n')
        digest = hashlib.sha256(receipt_path.read_bytes()).hexdigest()
        for receipt in (self.plan["coordination"]["lane_receipt"], self.plan["coordination"]["n6_receipt"],
                        *(capture["cache"]["receipt"] for capture in self.plan["captures"])):
            receipt["sha256"] = digest
        self.output = self.root / "output"
        self.probe = mock.patch.object(report, "_local_probe", side_effect=lambda command, *args:
                                      "" if command == ["ps", "-eo", "pid=,pgid="] else
                                      tools[Path(command[0]).stem + "_version"])
        self.source = mock.patch.object(report, "_local_source", side_effect=lambda checkout, expected:
                                       dict(expected, clean=True))
        self.environment = mock.patch.object(report, "_local_environment", return_value={})
        for patch in (self.probe, self.source, self.environment):
            patch.start()
            self.addCleanup(patch.stop)

    def execution(self, status="success"):
        def run(command, checkout, environment, directory, watchdog):
            # Binary bytes deliberately include invalid UTF-8 and line endings.
            (directory / "stdout.raw").write_bytes(b"suite-output\r\n\xff\x00")
            (directory / "stderr.raw").write_bytes(b"diagnostic\r\n")
            return {"status": status, "exit_code": 0 if status == "success" else 101,
                    "total_seconds": "1.25", "owned_process_reaped": True,
                    "streams": {name: report._local_sha(directory / (name + ".raw"))
                                for name in ("stdout", "stderr")}}
        return run

    def test_first_failure_preserves_binary_raw_and_refuses_a_second_plan_attempt(self):
        with mock.patch.object(report, "_local_run", side_effect=self.execution("failure")) as execution:
            result = report.collect_local(self.plan, str(self.output))
        self.assertEqual(execution.call_count, 1)
        self.assertEqual(result["status"], "failed_stop")
        self.assertEqual((result["planned_captures"], result["attempted_captures"]), (2, 1))
        raw = self.output / "baseline-native" / "stdout.raw"
        self.assertEqual(raw.read_bytes(), b"suite-output\r\n\xff\x00")
        self.assertEqual(result["captures"][0]["execution"]["streams"]["stdout"]["sha256"],
                         hashlib.sha256(raw.read_bytes()).hexdigest())
        self.assertFalse((self.output / "current-native").exists())
        with mock.patch.object(report, "_local_probe", side_effect=AssertionError("no probe after marker")), \
             self.assertRaisesRegex(ValueError, "rerun refused"):
            report.collect_local(self.plan, str(self.root / "second-output"))
        self.assertFalse((self.root / "second-output").exists())

    def test_local_success_has_no_ci_identity_ratio_or_derived_compile_runtime(self):
        with mock.patch.object(report, "_local_run", side_effect=self.execution()) as execution:
            result = report.collect_local(self.plan, str(self.output))
        self.assertEqual(execution.call_count, 2)
        self.assertEqual(result["status"], "completed")
        self.assertEqual(result["comparisons"], [])
        self.assertEqual(result["schema"], report.LOCAL_REPORT_SCHEMA)
        for capture in result["captures"]:
            self.assertTrue(capture["local_capture_id"].startswith("local-"))
            self.assertNotIn("run_id", capture)
            self.assertNotIn("job_id", capture)
            self.assertEqual(capture["measurements"]["total"]["seconds"], "1.25")
            self.assertIsNone(capture["measurements"]["compile"]["seconds"])
            self.assertIsNone(capture["measurements"]["runtime"]["seconds"])
        saved = json.loads((self.output / "report.json").read_text(encoding="utf-8"))
        self.assertEqual(saved, result)

    def test_changed_source_after_execution_stops_future_captures(self):
        def source(checkout, expected):
            source.calls += 1
            if source.calls == 4:  # two preflights, first before, first after
                raise ValueError("source changed after execution")
            return dict(expected, clean=True)
        source.calls = 0
        with mock.patch.object(report, "_local_source", side_effect=source), \
             mock.patch.object(report, "_local_run", side_effect=self.execution()) as execution:
            result = report.collect_local(self.plan, str(self.output))
        self.assertEqual(execution.call_count, 1)
        self.assertEqual(result["status"], "failed_stop")
        capture = result["captures"][0]
        self.assertEqual(capture["status"], "failure")
        self.assertIn("source changed", capture["failure_reason"])
        self.assertEqual(capture["execution"]["status"], "success")
        self.assertIsNone(capture["source_after"])

    def test_tampered_receipt_fails_before_workload_or_capture_marker(self):
        (self.root / "receipt.json").write_bytes(b"changed\n")
        with mock.patch.object(report, "_local_run", side_effect=AssertionError("no workload")), \
             self.assertRaisesRegex(ValueError, "receipt SHA256 mismatch"):
            report.collect_local(self.plan, str(self.output))
        self.assertFalse(self.output.exists())
        self.assertFalse(Path(self.plan["once_dir"]).exists())

    def test_posix_census_fixture_does_not_lookup_a_toolchain_version(self):
        self.assertEqual(report._local_probe(["ps", "-eo", "pid=,pgid="]), "")


class LocalCollectorLaunchFailureTests(unittest.TestCase):
    def test_resume_failure_retains_owned_thread_and_other_handles(self):
        process = object.__new__(report._WindowsOwnedProcess)
        process.kernel = mock.Mock()
        process.kernel.ResumeThread.return_value = 0xffffffff
        process.handle, process.job = 11, 12
        process.thread = None
        process._resume_owned_child(13)
        self.assertTrue(process.resume_pending)
        self.assertEqual((process.handle, process.job, process.thread), (11, 12, 13))
        self.assertEqual(process.kernel.ResumeThread.call_count, 2)
        process.kernel.CloseHandle.assert_not_called()

    def test_reader_start_failure_launches_no_child_and_releases_waiting_reader(self):
        original = threading.Thread.start
        calls = []
        def start(thread):
            calls.append(thread)
            if len(calls) == 2:
                raise RuntimeError("injected reader startup failure")
            return original(thread)
        with tempfile.TemporaryDirectory() as temporary, \
             mock.patch.object(threading.Thread, "start", start), \
             mock.patch.object(report, "_WindowsOwnedProcess") as windows, \
             mock.patch.object(report.subprocess, "Popen") as posix:
            directory = Path(temporary)
            result = report._local_run([sys.executable, "-c", "pass"], temporary,
                dict(os.environ), directory, {"seconds": 1, "max_log_bytes": 1024,
                    "cleanup": "windows-job" if os.name == "nt" else "posix-process-group"})
        windows.assert_not_called()
        posix.assert_not_called()
        self.assertIsNone(result["pid"])
        self.assertIn("RuntimeError", result["failure_reason"])
        self.assertFalse(any(thread.is_alive() for thread in calls))


class LocalCollectorChildTests(unittest.TestCase):
    """Small Python children verify raw capture and natural completion, not CI cost."""
    def capture(self, code, *, seconds=5, max_log_bytes=1048576):
        self.temp = tempfile.TemporaryDirectory(prefix="ci-local-child-tests-")
        self.addCleanup(self.temp.cleanup)
        directory = Path(self.temp.name)
        result = report._local_run([sys.executable, "-c", code], str(directory), dict(os.environ), directory,
                                   {"seconds": seconds, "max_log_bytes": max_log_bytes,
                                    "cleanup": "windows-job" if os.name == "nt" else "posix-process-group"})
        return result, directory

    def test_raw_binary_streams_and_nonzero_exit_are_preserved(self):
        result, directory = self.capture(
            "import os,sys;os.write(1,b'raw\\x00\\xff\\r\\n');os.write(2,b'err\\xfe');sys.exit(7)")
        self.assertEqual(result["status"], "failure")
        self.assertEqual(result["exit_code"], 7)
        self.assertTrue(result["owned_process_reaped"])
        self.assertEqual((directory / "stdout.raw").read_bytes(), b"raw\x00\xff\r\n")
        self.assertEqual((directory / "stderr.raw").read_bytes(), b"err\xfe")
        self.assertEqual(result["discarded_bytes"], 0)
        for name in ("stdout", "stderr"):
            raw = (directory / (name + ".raw")).read_bytes()
            self.assertEqual(result["streams"][name]["sha256"], hashlib.sha256(raw).hexdigest())
            self.assertEqual(result["streams"][name]["observed_bytes"], len(raw))

    def test_output_limit_keeps_bounded_prefix_and_waits_for_natural_exit(self):
        result, directory = self.capture("import os,time;os.write(1,b'x'*40000);time.sleep(.08);os.write(2,b'natural-end')",
                                         max_log_bytes=1024)
        self.assertEqual(result["status"], "failure")
        self.assertEqual(result["exit_code"], 0)
        self.assertTrue(result["owned_process_reaped"])
        self.assertEqual(sum((directory / (name + ".raw")).stat().st_size for name in ("stdout", "stderr")), 1024)
        self.assertEqual(sum(result["streams"][name]["observed_bytes"] for name in ("stdout", "stderr")), 40011)
        self.assertEqual(result["discarded_bytes"], 38987)

    def test_watchdog_marks_failure_without_terminating_the_owned_child(self):
        result, directory = self.capture("import os,time;os.write(1,b'before');time.sleep(1.1);os.write(1,b'natural-after')",
                                         seconds=1)
        self.assertEqual(result["status"], "timed_out")
        self.assertEqual(result["exit_code"], 0)
        self.assertTrue(result["owned_process_reaped"])
        self.assertEqual((directory / "stdout.raw").read_bytes(), b"beforenatural-after")
        self.assertGreaterEqual(float(result["total_seconds"]), 1.1)

    def test_raw_storage_failure_keeps_draining_chatty_child_to_natural_eof(self):
        original = Path.open
        class BrokenWriter:
            def __init__(self, stream):
                self.stream = stream
            def write(self, value):
                self.stream.write(value[:7])
                raise OSError("injected raw storage failure")
            def flush(self):
                self.stream.flush()
            def close(self):
                self.stream.close()
        def open_file(path, *args, **kwargs):
            stream = original(path, *args, **kwargs)
            return BrokenWriter(stream) if path.name == "stdout.raw" and args and args[0] == "xb" else stream
        with mock.patch.object(Path, "open", open_file):
            result, directory = self.capture("import os;os.write(1,b'x'*400000);os.write(2,b'natural-end')")
        self.assertEqual(result["status"], "failure")
        self.assertEqual(result["exit_code"], 0)
        self.assertTrue(result["drain_complete"])
        self.assertFalse(result["lane_held"])
        self.assertEqual(result["streams"]["stdout"]["observed_bytes"], 400000)
        self.assertEqual((directory / "stdout.raw").read_bytes(), b'x'*7)
        self.assertEqual(result["streams"]["stdout"]["sha256"], hashlib.sha256(b'x'*7).hexdigest())
        self.assertIsNone(result["streams"]["stdout"]["original_full_sha256"])
        self.assertEqual((directory / "stderr.raw").read_bytes(), b'natural-end')
        self.assertTrue(result["failure_reason"].startswith("raw_storage_write_error"))


if __name__ == "__main__":
    unittest.main()
