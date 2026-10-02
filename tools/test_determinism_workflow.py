#!/usr/bin/env python3
"""Regression checks for the required cross-platform checksum gate.

Run from any directory with Python 3, Bash, and PyYAML installed:
    python3 tools/test_determinism_workflow.py

These checks execute the actual gate script with synthetic GitHub job results.
They do not run the Rust targets or emulate the GitHub Actions scheduler.
"""

import itertools
import os
from pathlib import Path
import subprocess
import unittest

import yaml


WORKFLOW = Path(__file__).resolve().parents[1] / ".github/workflows/determinism.yml"
RESULTS = {
    "NATIVE_RESULT": "${{ needs.native.result }}",
    "WASM32_RESULT": "${{ needs.wasm32.result }}",
    "ANDROID_ARM64_RESULT": "${{ needs['android-arm64'].result }}",
}
REQUIRED_JOBS = {"native", "wasm32", "android-arm64"}


class DeterminismWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.jobs = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))["jobs"]
        cls.gate = cls.jobs["cross_platform_checksum_compare"]
        cls.step = cls.gate["steps"][0]

    def assert_required(self, entry):
        self.assertFalse(entry.get("continue-on-error", False))
        self.assertNotIn("if", entry)

    def assert_required_command(self, job, command, expected_env=None):
        steps = [step for step in job["steps"] if command in step.get("run", "").splitlines()]
        self.assertEqual(len(steps), 1, command)
        self.assert_required(steps[0])
        for name, value in (expected_env or {}).items():
            self.assertEqual(steps[0].get("env", {}).get(name), value)

    def test_gate_wiring(self):
        self.assertEqual(set(self.gate["needs"]), REQUIRED_JOBS)
        self.assertEqual(self.gate["if"], "${{ always() }}")
        self.assertEqual(self.gate["name"], "cross-platform checksum comparison")
        self.assertEqual(self.gate["runs-on"], "ubuntu-latest")
        self.assertFalse(self.gate.get("continue-on-error", False))
        self.assertEqual(len(self.gate["steps"]), 1)
        self.assert_required(self.step)
        self.assertEqual(self.step["shell"], "bash")
        self.assertEqual(self.step["env"], RESULTS)

    def test_actual_golden_runners_are_required(self):
        # There are no checksum artifacts. A green target proves determinism
        # only while its mandatory commands execute the shared pinned tests.
        for name in REQUIRED_JOBS:
            with self.subTest(job=name):
                self.assert_required(self.jobs[name])
        native = self.jobs["native"]
        self.assertFalse(native["strategy"]["fail-fast"])
        self.assert_required_command(
            native,
            "cargo test --workspace --release --exclude orr_sample --exclude orr_editor --exclude orr_web_gpu",
        )
        suites = (
            ("orr_session", "arena", "golden"),
            ("orr_physics", "physics", "golden"),
            ("orr_physics3d", "golden", "golden"),
            ("orr_games", "golden", ""),
            ("orr_wasm_bench", "checksums", ""),
        )
        for name, target, runner in (
            ("wasm32", "wasm32-wasip1", "wasmtime"),
            ("android-arm64", "aarch64-linux-android", "qemu-aarch64"),
        ):
            env = {f"CARGO_TARGET_{target.upper().replace('-', '_')}_RUNNER": runner}
            prefix = f"cargo test --release --target {target}"
            android = name == "android-arm64"
            for package in ("orr_fp", "orr_ecs"):
                suffix = " --tests -- --test-threads=1" if android else ""
                self.assert_required_command(self.jobs[name], f"{prefix} -p {package}{suffix}", env)
            for package, suite, test_filter in suites:
                args = " ".join(arg for arg in (test_filter, "--test-threads=1" if android else "") if arg)
                suffix = f" -- {args}" if args else ""
                self.assert_required_command(self.jobs[name], f"{prefix} -p {package} --test {suite}{suffix}", env)

        simd_env = {"CARGO_TARGET_WASM32_WASIP1_RUNNER": "wasmtime", "RUSTFLAGS": "-C target-feature=+simd128"}
        for package in ("orr_fp", "orr_ecs", "orr_games --test golden", "orr_wasm_bench --test checksums"):
            self.assert_required_command(
                self.jobs["wasm32"],
                f"cargo test --release --target wasm32-wasip1 --target-dir target/simd -p {package}",
                simd_env,
            )

    def run_gate(self, results):
        env = {key: value for key, value in os.environ.items() if key not in RESULTS}
        env.update(results)
        return subprocess.run(
            ["bash", "--noprofile", "--norc", "-e", "-o", "pipefail", "-c", self.step["run"]],
            env=env, capture_output=True, text=True, check=False,
        )

    def test_gate_bash_syntax(self):
        result = subprocess.run(["bash", "-n"], input=self.step["run"], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_only_all_success_passes(self):
        # 216 combinations include all four GitHub result states, empty
        # results, and an unknown state: allow success, rather than denylisting.
        states = ("success", "failure", "cancelled", "skipped", "", "unknown")
        for values in itertools.product(states, repeat=len(RESULTS)):
            results = dict(zip(RESULTS, values))
            with self.subTest(results=results):
                result = self.run_gate(results)
                self.assertEqual(result.returncode == 0, all(value == "success" for value in values), result.stdout + result.stderr)
                for name, value in results.items():
                    self.assertIn(f"{name}={value}", result.stdout)

    def test_unset_results_fail_closed(self):
        for missing in RESULTS:
            results = {name: "success" for name in RESULTS if name != missing}
            with self.subTest(missing=missing):
                result = self.run_gate(results)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
