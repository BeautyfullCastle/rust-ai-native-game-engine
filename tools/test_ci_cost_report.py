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


if __name__ == "__main__":
    unittest.main()
