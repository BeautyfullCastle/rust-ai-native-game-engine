#!/usr/bin/env python3
"""Regression checks for the checksum/sample gate and native CI artifact checks.

Run from any directory with Python 3, Bash, and PyYAML installed:
    python3 tools/test_determinism_workflow.py

These checks execute the actual gate and FFI artifact scripts with synthetic
job results and library files. They do not run the Rust targets or emulate the
GitHub Actions scheduler or native Windows/macOS shells and toolchains.
"""

import itertools
import os
import re
from pathlib import Path
import subprocess
import tempfile
import unittest

import yaml


WORKFLOW = Path(__file__).resolve().parents[1] / ".github/workflows/determinism.yml"
RESULTS = {
    "NATIVE_RESULT": "${{ needs.native.result }}",
    "WASM32_RESULT": "${{ needs.wasm32.result }}",
    "ANDROID_ARM64_RESULT": "${{ needs['android-arm64'].result }}",
    "BROWSER_RESULT": "${{ needs['wasm32-browser'].result }}",
    "SAMPLE_BUILD_RESULT": "${{ needs['sample-build'].result }}",
}
REQUIRED_JOBS = {"native", "wasm32", "android-arm64", "wasm32-browser", "sample-build"}
NATIVE_TEST = "cargo test --workspace --release --timings --exclude orr_sample --exclude orr_editor --exclude orr_web_gpu"
SAMPLE_TEST = "cargo test -p orr_sample -p orr_view -p orr_bridge -p orr_rhi -p orr_render -p orr_editor -p orr_web_gpu --release --timings"
FFI_ARTIFACTS = {
    "Linux": ("liborr_ffi.so", "liborr_ffi.a"),
    "macOS": ("liborr_ffi.dylib", "liborr_ffi.a"),
    "Windows": ("orr_ffi.dll", "orr_ffi.dll.lib", "orr_ffi.lib"),
}


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

    def test_actual_checksum_and_sample_runners_are_required(self):
        # There are no checksum artifacts. A green target proves determinism
        # only while its mandatory commands execute the shared pinned tests.
        # The separate graphics/audio/editor matrix is mandatory as well.
        for name in REQUIRED_JOBS:
            with self.subTest(job=name):
                self.assert_required(self.jobs[name])
        native = self.jobs["native"]
        self.assertFalse(native["strategy"]["fail-fast"])
        self.assert_required_command(
            native,
            NATIVE_TEST,
            {"ORR_REQUIRE_C_COMPILER": "${{ runner.os == 'Linux' && '1' || '0' }}"},
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

    def test_native_and_sample_matrices_are_preserved(self):
        self.assertEqual(self.jobs["native"]["strategy"]["matrix"]["include"], [
            {"os": "ubuntu-latest", "name": "ubuntu-latest-x86_64"},
            {"os": "windows-latest", "name": "windows-latest-x86_64"},
            {"os": "macos-latest", "name": "macos-latest-arm64"},
            {"os": "ubuntu-24.04-arm", "name": "ubuntu-24.04-arm64"},
        ])
        self.assertEqual(self.jobs["sample-build"]["strategy"]["matrix"]["os"], ["ubuntu-latest", "windows-latest"])
        self.assertFalse(self.jobs["sample-build"]["strategy"]["fail-fast"])
        self.assert_required_command(self.jobs["sample-build"], SAMPLE_TEST)

    def test_release_timing_uploads_are_html_only_and_unique(self):
        for job, command, name in (
            ("native", NATIVE_TEST, "cargo-timings-native-${{ matrix.name }}"),
            ("sample-build", SAMPLE_TEST, "cargo-timings-sample-${{ matrix.os }}"),
        ):
            with self.subTest(job=job):
                steps = self.jobs[job]["steps"]
                uploads = [step for step in steps if step.get("uses", "").startswith("actions/upload-artifact@")
                           and step.get("with", {}).get("name", "").startswith("cargo-timings-")]
                self.assertEqual(len(uploads), 1)
                upload = uploads[0]
                self.assertEqual(upload["uses"], "actions/upload-artifact@v4")
                self.assertEqual(upload["if"], "${{ always() }}")
                self.assertEqual(upload["with"], {
                    "name": name,
                    "path": "target/cargo-timings/*.html",
                    "if-no-files-found": "ignore",
                    "retention-days": 7,
                })
                build = next(step for step in steps if step.get("run") == command)
                self.assertEqual(steps.index(upload), steps.index(build) + 1)

    def test_arena_native_window_smoke_requires_real_windows_and_uploads_evidence(self):
        steps = self.jobs["sample-build"]["steps"]
        smoke = next(step for step in steps if step.get("name") == "Arena editor native-window smoke")
        self.assertEqual(smoke["if"], "runner.os == 'Linux'")
        self.assertFalse(smoke.get("continue-on-error", False))
        self.assertEqual(smoke["timeout-minutes"], 3)
        self.assertIn("xvfb-run", smoke["run"])
        self.assertIn("--test arena arena_native_window_smoke -- --exact --nocapture", smoke["run"])
        self.assertEqual(smoke["env"]["ORR_REQUIRE_NATIVE_EDITOR"], "1")
        self.assertEqual(smoke["env"]["WGPU_BACKEND"], "vulkan")
        for package in ("mesa-vulkan-drivers", "xvfb"):
            install = next(step for step in steps if package in step.get("run", ""))
            self.assertLess(steps.index(install), steps.index(smoke))
        upload = next(step for step in steps if step.get("name") == "Upload Arena editor native-window evidence")
        self.assertEqual(upload["if"], "${{ always() && runner.os == 'Linux' }}")
        self.assertEqual(upload["with"]["path"], smoke["env"]["ORR_NATIVE_EDITOR_ARTIFACT_DIR"])
        self.assertEqual(upload["with"]["if-no-files-found"], "error")
        self.assertEqual(steps.index(upload), steps.index(smoke) + 1)

    def ffi_artifact_step(self):
        steps = self.jobs["native"]["steps"]
        matches = [step for step in steps if step.get("name", "").startswith("Verify orr_ffi artifacts")]
        self.assertEqual(len(matches), 1)
        return matches[0]

    def test_ffi_artifacts_are_required_without_a_post_test_rebuild(self):
        step = self.ffi_artifact_step()
        self.assert_required(step)
        self.assertEqual(step["shell"], "bash")
        steps = self.jobs["native"]["steps"]
        build = next(entry for entry in steps if entry.get("run") == NATIVE_TEST)
        self.assertLess(steps.index(build), steps.index(step))
        self.assertNotIn("cargo ", step["run"])
        self.assertFalse(any("cargo build -p orr_ffi" in entry.get("run", "") for entry in steps))
        result = subprocess.run(["bash", "-n"], input=step["run"], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)

    def run_ffi_artifact_check(self, root, runner_os):
        return subprocess.run(
            ["bash", "--noprofile", "--norc", "-e", "-o", "pipefail", "-c", self.ffi_artifact_step()["run"]],
            cwd=root, env={**os.environ, "RUNNER_OS": runner_os}, capture_output=True, text=True, check=False,
        )

    def test_every_ffi_artifact_must_be_a_nonempty_file(self):
        for runner_os, artifacts in FFI_ARTIFACTS.items():
            with self.subTest(runner_os=runner_os), tempfile.TemporaryDirectory() as root:
                lib_dir = Path(root) / "target/release"
                lib_dir.mkdir(parents=True)
                for name in artifacts:
                    (lib_dir / name).write_bytes(b"artifact")
                result = self.run_ffi_artifact_check(root, runner_os)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                for name in artifacts:
                    path = lib_dir / name
                    for state in ("missing", "empty", "directory"):
                        with self.subTest(artifact=name, state=state):
                            path.unlink()
                            if state == "empty":
                                path.touch()
                            elif state == "directory":
                                path.mkdir()
                            result = self.run_ffi_artifact_check(root, runner_os)
                            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                            self.assertIn(f"target/release/{name}", result.stdout)
                            if path.is_dir():
                                path.rmdir()
                            path.write_bytes(b"artifact")

    def test_unknown_ffi_platform_fails_closed(self):
        with tempfile.TemporaryDirectory() as root:
            result = self.run_ffi_artifact_check(root, "unexpected")
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("Unsupported runner OS", result.stdout)

    def test_browser_runtime_is_mandatory(self):
        job = self.jobs["wasm32-browser"]
        self.assert_required(job)
        commands = (
            "npm ci --prefix tools/webtransport",
            "tools/webtransport/node_modules/.bin/playwright install --with-deps chromium",
            'cargo install wasm-bindgen-cli --version "$version" --locked',
            "npm test --prefix tools/webtransport",
        )
        for command in commands:
            self.assert_required_command(job, command)
        self.assert_required_command(job, "tools/build_web.sh", {"WEB_PROFILE": "release", "WEB_GPU": "1"})
        self.assert_required_command(
            job,
            "cargo test -p orr_server --release --test browser_e2e -- --nocapture --test-threads=1",
            {"ORR_REQUIRE_BROWSER": "1", "ORR_REQUIRE_WEBGPU": "1"},
        )
        runs = [step.get("run", "") for step in job["steps"]]
        install = next(command for command in runs if "cargo install wasm-bindgen-cli" in command)
        self.assertIn('tomllib.load(open("Cargo.lock", "rb"))', install)
        self.assertIn('assert len(v) == 1', install)
        build_index = runs.index("tools/build_web.sh")
        boundary_index = runs.index("npm test --prefix tools/webtransport")
        browser_index = next(i for i, command in enumerate(runs) if "--test browser_e2e" in command)
        self.assertLess(build_index, boundary_index)
        self.assertLess(boundary_index, browser_index)

    def test_browser_software_gpu_paths_are_explicit(self):
        source = (WORKFLOW.parents[2] / "crates/orr_server/tests/browser_e2e.rs").read_text(encoding="utf-8")
        required_flags = {
            "WEBGL2": {"--use-gl=angle", "--use-angle=swiftshader", "--enable-unsafe-swiftshader"},
            "WEBGPU": {
                "--enable-unsafe-webgpu", "--enable-unsafe-swiftshader", "--use-webgpu-adapter=swiftshader",
                "--enable-features=Vulkan", "--use-vulkan=swiftshader", "--use-angle=swiftshader",
            },
        }
        for view, flags in required_flags.items():
            with self.subTest(view=view):
                declaration = re.search(rf'const {view}: View = View \{{(.*?)\}};', source, re.S)
                self.assertIsNotNone(declaration)
                configured = re.search(r'flags: "([^"]*)"', declaration.group(1))
                self.assertIsNotNone(configured)
                self.assertTrue(flags.issubset(set(configured.group(1).split())))
        self.assertIn('let flags = format!("{wss_flag} {} {extra_flags}", view.flags);', source)

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
        # 7776 combinations across all five jobs include all four GitHub result
        # states, empty results, and an unknown state: allow success, rather
        # than denylisting.
        states = ("success", "failure", "cancelled", "skipped", "", "unknown")
        for values in itertools.product(states, repeat=len(RESULTS)):
            results = dict(zip(RESULTS, values))
            with self.subTest(results=results):
                result = self.run_gate(results)
                self.assertEqual(result.returncode == 0, all(value == "success" for value in values), result.stdout + result.stderr)
                for name, value in results.items():
                    self.assertIn(f"{name}={value}", result.stdout)

    def test_sample_failure_skip_cancel_or_missing_cannot_pass(self):
        # Even when every checksum target passes, a non-success/missing sample
        # matrix result must fail the aggregate and never print success claims.
        for sample_result in ("failure", "skipped", "cancelled", "", None):
            results = {name: "success" for name in RESULTS if name != "SAMPLE_BUILD_RESULT"}
            if sample_result is not None:
                results["SAMPLE_BUILD_RESULT"] = sample_result
            with self.subTest(sample_result=sample_result):
                result = self.run_gate(results)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn("::error::SAMPLE_BUILD_RESULT must be success", result.stdout)
                self.assertNotIn("passed", result.stdout)

    def test_unset_results_fail_closed(self):
        for missing in RESULTS:
            results = {name: "success" for name in RESULTS if name != missing}
            with self.subTest(missing=missing):
                result = self.run_gate(results)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
