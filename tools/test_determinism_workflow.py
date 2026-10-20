#!/usr/bin/env python3
"""Regression checks for the checksum/sample gate and native CI artifact checks.

Run from any directory with Python 3, Bash, and PyYAML installed:
    python3 tools/test_determinism_workflow.py

These checks execute the actual gate and FFI artifact scripts with synthetic
job results and library files. They do not run the Rust targets or emulate the
GitHub Actions scheduler or native Windows/macOS shells and toolchains.
"""

import copy
import hashlib
import json
import os
import re
from pathlib import Path
import subprocess
import sys
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
    "LINKED_PREFABS_RESULT": "${{ needs['sample-linked-prefabs'].result }}",
    "ROOM_RESULT": "${{ needs['sample-room'].result }}",
}
REQUIRED_JOBS = {"native", "wasm32", "android-arm64", "wasm32-browser", "sample-build", "sample-linked-prefabs", "sample-room", "static-model-authoring", "room-wall-authoring"}
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
        cls.authoring_step = cls.gate["steps"][1]
        cls.wall_step = cls.gate["steps"][2]

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
        self.assertEqual(len(self.gate["steps"]), 3)
        self.assert_required(self.step)
        self.assertEqual(self.step["shell"], "bash")
        self.assertEqual(self.step["env"], RESULTS)

    def test_static_authoring_matrix_routes_are_required(self):
        job = self.jobs["static-model-authoring"]
        self.assert_required(job)
        self.assertNotIn("needs", job)
        self.assertEqual(job["name"], "static model authoring (${{ matrix.feature }}, ${{ matrix.mode }}, ${{ matrix.os }})")
        self.assertEqual(job["runs-on"], "${{ matrix.os }}")
        self.assertEqual(job["timeout-minutes"], 90)
        self.assertEqual(job["env"], {"PYTHONDONTWRITEBYTECODE": "1"})
        self.assertEqual(job["strategy"], {"fail-fast": False, "matrix": {"include": [
            {"feature": feature, "mode": mode, "os": os_name, "timeout": 60 if mode == "export" else 45}
            for feature in ("uv", "pivot")
            for mode, os_name in (("cpu", "ubuntu-22.04"), ("cpu", "windows-latest"),
                                  ("gpu", "ubuntu-22.04"), ("export", "ubuntu-22.04"))
        ]}})
        steps = job["steps"]
        self.assertEqual(len(steps), 7)
        for step, action in zip(steps[:3], ("actions/checkout@v4", "dtolnay/rust-toolchain@stable", "Swatinem/rust-cache@v2")):
            self.assertEqual(step, {"uses": action})
        self.assertEqual(steps[3], {
            "name": "Install static authoring software Vulkan dependency",
            "if": "matrix.mode != 'cpu'",
            "run": "sudo apt-get update -q && sudo apt-get install -y -q mesa-vulkan-drivers",
        })
        self.assertEqual(steps[4], {
            "name": "Install static authoring namespace acceptance dependency",
            "if": "matrix.mode == 'export'",
            "run": "sudo apt-get update -q && sudo apt-get install -y -q bubblewrap",
        })
        evidence = "${{ matrix.mode == 'export' && '/tmp' || runner.temp }}/orr-static-${{ matrix.feature }}-${{ matrix.mode }}-${{ github.run_id }}-${{ github.run_attempt }}"
        self.assertEqual(steps[5], {
            "name": "Required independent static model authoring acceptance", "shell": "bash",
            "timeout-minutes": "${{ matrix.timeout }}",
            "env": {"ORR_STATIC_UV_EVIDENCE": evidence, "ORR_STATIC_PIVOT_EVIDENCE": evidence},
            "run": "python -B tools/static_model_${{ matrix.feature }}_gate.py ${{ matrix.mode }}",
        })
        self.assertEqual(steps[6], {
            "name": "Retain static authoring and partial failure evidence", "if": "always()",
            "uses": "actions/upload-artifact@v4", "with": {
                "name": "static-${{ matrix.feature }}-${{ matrix.mode }}-${{ matrix.os }}", "path": evidence,
                "include-hidden-files": True, "if-no-files-found": "error", "retention-days": 7,
            },
        })
        for name in ("sample-build", "sample-room"):
            for step in self.jobs[name]["steps"]:
                self.assertNotIn("static_model_uv_gate.py", step.get("run", ""))
                self.assertNotIn("static_model_pivot_gate.py", step.get("run", ""))

    def test_authoring_aggregate_is_independently_fail_closed(self):
        step = self.authoring_step
        self.assertEqual(set(step), {"name", "shell", "env", "run"})
        self.assert_required(step)
        self.assertEqual(step["name"], "Require every independent static authoring matrix row to pass")
        self.assertEqual(step["shell"], "bash")
        self.assertEqual(step["env"], {"AUTHORING_RESULT": "${{ needs['static-model-authoring'].result }}"})
        syntax = subprocess.run(["bash", "-n"], input=step["run"], capture_output=True, text=True, check=False)
        self.assertEqual(syntax.returncode, 0, syntax.stderr)
        for state in ("success", "failure", "cancelled", "skipped", "", "unknown", None):
            env = {key: value for key, value in os.environ.items() if key not in (*RESULTS, "AUTHORING_RESULT")}
            env.update({name: "success" for name in RESULTS})
            if state is not None:
                env["AUTHORING_RESULT"] = state
            for script in (step["run"], self.step["run"] + "\n" + step["run"]):
                with self.subTest(state=state, combined=script != step["run"]):
                    result = subprocess.run(["bash", "--noprofile", "--norc", "-e", "-o", "pipefail", "-c", script],
                                            env=env, capture_output=True, text=True, check=False)
                    self.assertEqual(result.returncode == 0, state == "success", result.stdout + result.stderr)
                    if state != "success":
                        self.assertIn("::error::AUTHORING_RESULT must be success", result.stdout)
                        self.assertNotIn("All eight independent static authoring routes passed", result.stdout)
        # A successful authoring matrix must never rescue any failed inherited lane.
        for name in RESULTS:
            for state in ("failure", "cancelled", "skipped", "", "unknown", None):
                env = {key: value for key, value in os.environ.items() if key not in (*RESULTS, "AUTHORING_RESULT")}
                env.update({key: "success" for key in RESULTS if key != name})
                env["AUTHORING_RESULT"] = "success"
                if state is not None:
                    env[name] = state
                result = subprocess.run(["bash", "--noprofile", "--norc", "-e", "-o", "pipefail", "-c", self.step["run"] + "\n" + step["run"]],
                                        env=env, capture_output=True, text=True, check=False)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertNotIn("All eight independent static authoring routes passed", result.stdout)

    def test_room_wall_matrix_routes_are_required(self):
        job = self.jobs["room-wall-authoring"]
        self.assert_required(job)
        self.assertNotIn("needs", job)
        self.assertEqual(job["name"], "room wall authoring (${{ matrix.mode }}, ${{ matrix.os }})")
        self.assertEqual(job["runs-on"], "${{ matrix.os }}")
        self.assertEqual(job["timeout-minutes"], 90)
        self.assertEqual(job["env"], {"PYTHONDONTWRITEBYTECODE": "1"})
        self.assertEqual(job["strategy"], {"fail-fast": False, "matrix": {"include": [
            {"mode": mode, "os": os_name, "timeout": 60 if mode == "export" else 45}
            for mode, os_name in (("cpu", "ubuntu-22.04"), ("cpu", "windows-latest"),
                                  ("gpu", "ubuntu-22.04"), ("export", "ubuntu-22.04"))
        ]}})
        steps = job["steps"]
        self.assertEqual(len(steps), 7)
        for step, action in zip(steps[:3], ("actions/checkout@v4", "dtolnay/rust-toolchain@stable", "Swatinem/rust-cache@v2")):
            self.assertEqual(step, {"uses": action})
        self.assertEqual(steps[3], {
            "name": "Install Room wall software Vulkan dependency", "if": "matrix.mode != 'cpu'",
            "run": "sudo apt-get update -q && sudo apt-get install -y -q mesa-vulkan-drivers",
        })
        self.assertEqual(steps[4], {
            "name": "Install Room wall namespace acceptance dependency", "if": "matrix.mode == 'export'",
            "run": "sudo apt-get update -q && sudo apt-get install -y -q bubblewrap",
        })
        evidence = "${{ matrix.mode == 'export' && '/tmp' || runner.temp }}/orr-room-wall-${{ matrix.mode }}-${{ github.run_id }}-${{ github.run_attempt }}"
        self.assertEqual(steps[5], {
            "name": "Required independent Room wall authoring acceptance", "shell": "bash",
            "timeout-minutes": "${{ matrix.timeout }}", "env": {"ORR_ROOM_WALL_GATE_EVIDENCE": evidence},
            "run": "unset PYTHONOPTIMIZE\npython -B tools/room_wall_gate.py ${{ matrix.mode }}\n",
        })
        self.assertEqual(steps[6], {
            "name": "Retain Room wall and partial failure evidence", "if": "always()",
            "uses": "actions/upload-artifact@v4", "with": {
                "name": "room-wall-${{ matrix.mode }}-${{ matrix.os }}", "path": evidence,
                "include-hidden-files": True, "if-no-files-found": "error", "retention-days": 7,
            },
        })
        for name, other in self.jobs.items():
            if name != "room-wall-authoring":
                for step in other["steps"]:
                    self.assertNotIn("tools/room_wall_gate.py", step.get("run", ""))

    def test_room_wall_aggregate_is_independently_fail_closed(self):
        step = self.wall_step
        self.assertEqual(set(step), {"name", "if", "shell", "env", "run"})
        self.assertEqual(step["if"], "always()")
        self.assertEqual(step["name"], "Require every independent Room wall authoring matrix row to pass")
        self.assertEqual(step["shell"], "bash")
        self.assertEqual(step["env"], {"ROOM_WALL_RESULT": "${{ needs['room-wall-authoring'].result }}"})
        syntax = subprocess.run(["bash", "-n"], input=step["run"], capture_output=True, text=True, check=False)
        self.assertEqual(syntax.returncode, 0, syntax.stderr)
        names = (*RESULTS, "AUTHORING_RESULT", "ROOM_WALL_RESULT")
        combined = self.step["run"] + "\n" + self.authoring_step["run"] + "\n" + step["run"]
        for state in ("success", "failure", "cancelled", "skipped", "", "unknown", None):
            env = {key: value for key, value in os.environ.items() if key not in names}
            env.update({name: "success" for name in names if name != "ROOM_WALL_RESULT"})
            if state is not None:
                env["ROOM_WALL_RESULT"] = state
            for script in (step["run"], combined):
                with self.subTest(state=state, combined=script == combined):
                    result = subprocess.run(["bash", "--noprofile", "--norc", "-e", "-o", "pipefail", "-c", script],
                                            env=env, capture_output=True, text=True, check=False)
                    self.assertEqual(result.returncode == 0, state == "success", result.stdout + result.stderr)
                    if state != "success":
                        self.assertIn("::error::ROOM_WALL_RESULT must be success", result.stdout)
                        self.assertNotIn("All four independent Room wall authoring routes passed", result.stdout)
        # A successful Room wall result cannot rescue any failed inherited result.
        # GitHub retains failure from earlier steps even though this step always runs.
        for name in names[:-1]:
            for state in ("failure", "cancelled", "skipped", "", "unknown", None):
                env = {key: value for key, value in os.environ.items() if key not in names}
                env.update({key: "success" for key in names if key != name})
                if state is not None:
                    env[name] = state
                with self.subTest(failed_name=name, state=state):
                    result = subprocess.run(["bash", "--noprofile", "--norc", "-e", "-o", "pipefail", "-c", combined],
                                            env=env, capture_output=True, text=True, check=False)
                    self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                    self.assertNotIn("All four independent Room wall authoring routes passed", result.stdout)

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
        self.assertEqual(self.jobs["sample-build"]["strategy"]["matrix"]["os"], ["ubuntu-22.04", "windows-latest"])
        self.assertFalse(self.jobs["sample-build"]["strategy"]["fail-fast"])
        self.assert_required_command(self.jobs["sample-build"], SAMPLE_TEST)

    def test_linked_prefab_lane_is_independent_and_mandatory(self):
        job = self.jobs["sample-linked-prefabs"]
        self.assert_required(job)
        self.assertEqual(job["runs-on"], "ubuntu-22.04")
        self.assertEqual(job["name"], "sample linked prefabs (ubuntu-22.04)")
        for key in ("needs", "strategy", "env", "defaults", "permissions"):
            self.assertNotIn(key, job)
        steps = job["steps"]
        self.assertEqual(steps[:3], [
            {"uses": "actions/checkout@v4"},
            {"uses": "dtolnay/rust-toolchain@stable"},
            {"uses": "Swatinem/rust-cache@v2"},
        ])
        self.assertEqual(len(steps), 9)
        self.assertEqual(steps[3], {
            "name": "Install linked-prefab software Vulkan dependency",
            "run": "sudo apt-get update -q && sudo apt-get install -y -q mesa-vulkan-drivers",
        })
        # Pin the byte-identical migrated command bodies, including the full
        # seven-crate feature union and pipefail/tee evidence behavior.
        expected_hashes = {
            'Linked Collect prefab contracts and feature boundaries':
                'fa8bd6e5eb9379924354cba2ed6e6eb94713429b4bfb37cfbf183a7f5711db84',
            'Install linked-prefab namespace acceptance dependency':
                'e6b3c77e96769b4ff63aee80df79359f9426eb90ab9da2b66598dfba2b9ad51b',
            'Linked Collect actual editor GPU and source-hidden export':
                '647f4d2de1252c38c05d3c1ac91e4cc06eb40d899b1526aa5094bc190c745afa',
            'Strict seven-crate linked prefab and inherited feature Clippy':
                '25b126a1f4925c1cf966d655a6118d5b7fea91fa92397fd2e8f415164b09f7d3',
        }
        self.assertEqual([step["name"] for step in steps[4:8]], list(expected_hashes))
        for step in steps[4:8]:
            self.assert_required(step)
            self.assertEqual(hashlib.sha256(step["run"].encode()).hexdigest(), expected_hashes[step["name"]])
            allowed = {"name", "run"}
            if step is not steps[5]:
                allowed.add("shell")
                self.assertEqual(step["shell"], "bash")
            if step is steps[6]:
                allowed.add("env")
                self.assertEqual(step["env"], {"WGPU_BACKEND": "vulkan", "ORR_REQUIRE_GPU": "1"})
            self.assertEqual(set(step), allowed)
            parsed = subprocess.run(["bash", "-n"], input=step["run"], capture_output=True, text=True, check=False)
            self.assertEqual(parsed.returncode, 0, parsed.stderr)
        self.assertEqual(steps[8], {
            "name": "Upload linked prefab acceptance evidence",
            "if": "${{ always() }}",
            "uses": "actions/upload-artifact@v4",
            "with": {
                "name": "linked-prefab-evidence-ubuntu-22.04",
                "path": "target/linked-prefab-evidence/*.log\ntarget/linked-prefab-evidence/*.sha256\n"
                        "target/linked-prefab-evidence/captures/*.png\ntarget/linked-prefab-evidence/captures/*.txt\n",
                "if-no-files-found": "error",
                "retention-days": 7,
            },
        })
        sample = self.jobs["sample-build"]
        self.assertEqual(sample["name"], "sample build (${{ matrix.os }})")
        self.assert_no_migrated_linked_prefab_steps(sample)

    def assert_no_migrated_linked_prefab_steps(self, sample):
        # Reimport acceptance retains its linked-prefabs Cargo feature dependency.
        migrated_names = {step["name"] for step in self.jobs["sample-linked-prefabs"]["steps"][4:]}
        self.assertFalse(any(step.get("name") in migrated_names
                             or "tools/check-linked-prefabs.sh" in step.get("run", "")
                             for step in sample["steps"]))

    def test_room_lane_is_independent_mandatory_and_preserves_migrated_commands(self):
        from test_room_project_gate import check_room_workflow, ROOM_MIGRATED_NAMES
        check_room_workflow(WORKFLOW.read_text(encoding="utf-8"))
        job = self.jobs["sample-room"]
        self.assert_required(job)
        self.assertEqual(set(job), {"name", "runs-on", "steps"})
        self.assertEqual(job["name"], "sample room acceptance (ubuntu-22.04)")
        self.assertEqual(job["runs-on"], "ubuntu-22.04")
        steps = job["steps"]
        self.assertEqual(len(steps), 12)
        self.assertEqual(steps[:3], [
            {"uses": "actions/checkout@v4"},
            {"uses": "dtolnay/rust-toolchain@stable"},
            {"uses": "Swatinem/rust-cache@v2"},
        ])
        self.assertEqual(steps[3:5], [
            {"name": "Install Room software Vulkan dependency",
             "run": "sudo apt-get update -q && sudo apt-get install -y -q mesa-vulkan-drivers"},
            {"name": "Install Room namespace acceptance dependency",
             "run": "sudo apt-get install -y -q bubblewrap"},
        ])
        self.assertEqual([step["name"] for step in steps[5:]], list(ROOM_MIGRATED_NAMES))
        # Exact pre-split command bodies from development bea5becf. Moving jobs
        # must not merge feature combinations, change producers, or relax gates.
        expected_hashes = {
            "RoomEscapeV1 contracts and optional feature combinations":
                "88ed25068f32ac4f3e12783c0a4695fae8b2d2ddf5d5bb945edb17d2acfbcf9d",
            "RoomEscapeV1 actual GPU and source-hidden production export":
                "c00d2ed483107154ca108b5e17fba019a41728d0474833031f02bd0451e8d6c4",
            "Room creator checker semantic fixtures":
                "302f80d14ad9f5bf84e93235e9c59896ddd6bd25c320fe5e488167d8caf94917",
            "Required Room creator API CLI editor and joint inventories":
                "fb8acab6c4e2da8141df5334435425f0421da75c4c50c29ff4d5d96ba1219989",
            "Required generated Room GPU and source-hidden production export":
                "edf70cd3eac722bdf3c5fc6971de8adde7c0a2f2cb4b81a50a7775ac154e810a",
        }
        for step in steps[5:]:
            self.assertFalse(step.get("continue-on-error", False))
            if "run" in step:
                self.assertEqual(hashlib.sha256(step["run"].encode()).hexdigest(), expected_hashes[step["name"]])
                result = subprocess.run(["bash", "-n"], input=step["run"], capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 0, result.stderr)
        sample_steps = self.jobs["sample-build"]["steps"]
        self.assertFalse(any(step.get("name") in ROOM_MIGRATED_NAMES
                             or any(helper in step.get("run", "") for helper in
                                    ("tools/check-room-project.sh", "tools/check-room-template.sh"))
                             for step in sample_steps))
        self.assertEqual(sum(step.get("name") == "Upload generated Arena acceptance evidence"
                             for step in sample_steps), 1)
        room_artifacts = [step["with"]["name"] for step in steps if "with" in step]
        self.assertEqual(room_artifacts, ["room-project-evidence-ubuntu-22.04", "room-template-evidence-ubuntu-22.04"])
        for name in room_artifacts:
            owners = [job_id for job_id, entry in self.jobs.items() for step in entry["steps"]
                      if step.get("with", {}).get("name") == name]
            self.assertEqual(owners, ["sample-room"])

    def test_linked_prefab_migration_allows_required_downstream_feature(self):
        step = {
            "name": "Required downstream image reimport acceptance",
            "run": "cargo test --locked -p orr_sample --features image-reimport,linked-prefabs --lib",
        }
        self.assert_required(step)
        self.assert_no_migrated_linked_prefab_steps({"steps": [step]})

    def test_linked_prefab_migration_rejects_renamed_helper_invocation(self):
        for lane in ("contracts", "gpu"):
            with self.subTest(lane=lane), self.assertRaises(AssertionError):
                self.assert_no_migrated_linked_prefab_steps({"steps": [{
                    "name": "Renamed downstream acceptance",
                    "run": f"tools/check-linked-prefabs.sh {lane}",
                }]})

    def test_linked_prefab_inventory_audit_rejects_false_success(self):
        # Execute only the unchanged script's embedded Python auditor with
        # synthetic evidence. No Cargo, GPU, binaries, or namespaces are run.
        script = (WORKFLOW.parents[2] / "tools/check-linked-prefabs.sh").read_text(encoding="utf-8")
        inventory = json.loads(script.split("<<'JSONINVENTORY'\n", 1)[1].split("\nJSONINVENTORY", 1)[0])
        auditor = script.split("<<'PYAUDIT'\n", 1)[1].split("\nPYAUDIT", 1)[0]
        self.assertIn("bwrap --ro-bind / / -- /bin/true", script)
        self.assertIn("run_test editor-gpu-source-hidden-export 1 0", script)
        with tempfile.TemporaryDirectory() as directory:
            data = Path(directory) / "inventory.json"
            log = Path(directory) / "test.log"
            data.write_text(json.dumps(inventory), encoding="utf-8")
            for lane, expected in inventory.items():
                listing = "".join(f"{name}: test\n" for name in expected["names"])
                results = "".join(f"test result: ok. {passed} passed; {failed} failed; {ignored} ignored;\n"
                                  for passed, failed, ignored in expected["summaries"])
                for mode, contents, passes in (
                    ("list", listing, True), ("list", listing + "unexpected_test: test\n", False),
                    ("list", "", False), ("results", results, True), ("results", "", False),
                    ("results", results + "skipping unavailable GPU\n", False),
                    ("results", results.replace(" passed;", "0 passed;", 1), False),
                    ("results", results.replace("test result: ok.", "test result: FAILED.", 1), False),
                ):
                    with self.subTest(lane=lane, mode=mode, passes=passes, contents=contents):
                        log.write_text(contents, encoding="utf-8")
                        result = subprocess.run([sys.executable, "-c", auditor, str(data), lane, mode, str(log)],
                                                capture_output=True, text=True, check=False)
                        self.assertEqual(result.returncode == 0, passes, result.stdout + result.stderr)

    def test_simulation_type_guard_and_fixtures_are_required(self):
        native = self.jobs["native"]
        self.assert_required_command(native, "python tools/check_sim_float_types.py")
        self.assert_required_command(native, "python tools/test_sim_float_guard.py")
        steps = native["steps"]
        tests = next(step for step in steps if step.get("run") == NATIVE_TEST)
        guards = []
        for command in ("python tools/check_sim_float_types.py", "python tools/test_sim_float_guard.py"):
            guard = next(step for step in steps if command in step.get("run", "").splitlines())
            # Each mandatory native command must own a step: the default
            # Windows pwsh wrapper returns only the final LASTEXITCODE.
            self.assertEqual(guard["run"].strip(), command)
            self.assertLess(steps.index(guard), steps.index(tests))
            guards.append(guard)
        self.assertIsNot(guards[0], guards[1])
        self.assertLess(steps.index(guards[0]), steps.index(guards[1]))

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
                self.assert_release_timing_upload_position(job, steps, build, upload)

    def assert_release_timing_upload_position(self, job, steps, build, upload):
        build_index = steps.index(build)
        if job == "native":
            # Native CLI identity verification and its retained evidence must run
            # after the workspace tests, even on failure, before timing upload.
            evidence = "${{ runner.temp }}/native-assemble-${{ github.run_id }}-${{ github.run_attempt }}"
            self.assertEqual(steps[build_index + 1:build_index + 3], [
                {
                    "name": "Verify native assemble CLI and source identity after workspace tests",
                    "if": "always()",
                    "run": f'python -B tools/native_assemble_cli_prerequisite.py verify --evidence "{evidence}"',
                },
                {
                    "name": "Retain native assemble CLI prerequisite and failure evidence",
                    "if": "always()",
                    "uses": "actions/upload-artifact@v4",
                    "with": {
                        "name": "native-assemble-${{ matrix.name }}",
                        "path": evidence,
                        "if-no-files-found": "error",
                        "retention-days": 7,
                    },
                },
            ])
            self.assertEqual(steps.index(upload), build_index + 3)
        else:
            self.assertEqual(steps.index(upload), build_index + 1)

    def test_native_timing_rejects_missing_reordered_or_altered_prerequisites(self):
        original = self.jobs["native"]["steps"]
        build_index = next(i for i, step in enumerate(original) if step.get("run") == NATIVE_TEST)
        mutations = []
        for offset in (1, 2):
            missing = copy.deepcopy(original)
            del missing[build_index + offset]
            mutations.append((f"missing prerequisite {offset}", missing))
            prerequisite = original[build_index + offset]
            # Cover every field, including nested artifact policy, and reject
            # added soft-failure behavior as well as omitted/changed fields.
            paths = [(key,) for key in prerequisite]
            paths += [("with", key) for key in prerequisite.get("with", {})]
            for path in paths:
                for operation in ("remove", "alter"):
                    changed = copy.deepcopy(original)
                    target = changed[build_index + offset]
                    for key in path[:-1]:
                        target = target[key]
                    if operation == "remove":
                        del target[path[-1]]
                    else:
                        target[path[-1]] = "altered"
                    mutations.append((f"{operation} prerequisite {offset} {path}", changed))
            softened = copy.deepcopy(original)
            softened[build_index + offset]["continue-on-error"] = True
            mutations.append((f"soft-fail prerequisite {offset}", softened))
        reordered = copy.deepcopy(original)
        reordered[build_index + 1:build_index + 3] = reversed(reordered[build_index + 1:build_index + 3])
        mutations.append(("reordered prerequisites", reordered))
        for label, steps in mutations:
            with self.subTest(mutation=label):
                upload = next(step for step in steps if step == original[build_index + 3])
                with self.assertRaises(AssertionError):
                    self.assert_release_timing_upload_position("native", steps, steps[build_index], upload)

    def test_release_timing_rejects_inserted_steps_and_detached_uploads(self):
        for job, command, offset in (("native", NATIVE_TEST, 3), ("sample-build", SAMPLE_TEST, 1)):
            original = self.jobs[job]["steps"]
            build_index = next(i for i, step in enumerate(original) if step.get("run") == command)
            # No arbitrary step is permitted anywhere between build and upload.
            for position in range(1, offset + 1):
                with self.subTest(job=job, inserted_after=position):
                    steps = copy.deepcopy(original)
                    upload = steps[build_index + offset]
                    steps.insert(build_index + position, {"run": "echo unrelated"})
                    with self.assertRaises(AssertionError):
                        self.assert_release_timing_upload_position(job, steps, steps[build_index], upload)
            for destination in (build_index, len(original) - 1):
                with self.subTest(job=job, moved_to=destination):
                    steps = copy.deepcopy(original)
                    build = steps[build_index]
                    upload = steps.pop(build_index + offset)
                    steps.insert(destination, upload)
                    with self.assertRaises(AssertionError):
                        self.assert_release_timing_upload_position(job, steps, build, upload)

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

    def test_asset_core_wasm_check_is_required_and_isolated(self):
        job = self.jobs["wasm32-browser"]
        command = "cargo check --target wasm32-unknown-unknown -p orr_asset --no-default-features"
        self.assert_required(job)
        self.assert_required_command(job, command)
        step = next(step for step in job["steps"] if step.get("run") == command)
        # Keep default-feature isolation in its own invocation rather than
        # unifying features with the existing view/browser package build.
        self.assertEqual(step["run"], command)
        install = next(step for step in job["steps"]
                       if step.get("with", {}).get("targets") == "wasm32-unknown-unknown")
        self.assertLess(job["steps"].index(install), job["steps"].index(step))

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
            env=env, capture_output=True, text=True, check=False, timeout=10,
        )

    def test_gate_bash_syntax(self):
        result = subprocess.run(["bash", "-n"], input=self.step["run"], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)

    def assert_independent_gate_contract(self, script):
        # This exact prefix is the proof obligation for reducing the Cartesian
        # test domain. Each iteration reads only its own input, compares only
        # with success, and monotonically ORs failure into a fresh accumulator.
        # No cross-input dependency, early exit, or later reset is permitted.
        # Thus all 6**7 inputs are covered by seven independent six-state
        # equivalence classes, not by assuming sampled combinations generalize.
        prefix = """failed=0
for result_var in NATIVE_RESULT WASM32_RESULT ANDROID_ARM64_RESULT BROWSER_RESULT SAMPLE_BUILD_RESULT LINKED_PREFABS_RESULT ROOM_RESULT; do
  result="${!result_var}"
  echo "$result_var=$result"
  if [[ "$result" != "success" ]]; then
    echo "::error::$result_var must be success, got '$result'"
    failed=1
  fi
done
if [[ "$failed" != "0" ]]; then
  exit 1
fi
"""
        self.assertTrue(script.startswith(prefix), "gate independence contract changed; review equivalence proof")
        suffix = script[len(prefix):].strip().splitlines()
        self.assertTrue(suffix, "gate success output is missing")
        # Echo-only literal success text cannot alter status or depend on an
        # input. Disallow substitutions/escapes, shell operators and new code.
        for line in suffix:
            self.assertRegex(line, r'^echo "[^"\\$`]*"$', "nonliteral gate success suffix")

    def test_only_all_success_passes(self):
        self.assert_independent_gate_contract(self.step["run"])
        states = ("failure", "cancelled", "skipped", "", "unknown")
        names = tuple(RESULTS)
        success = {name: "success" for name in names}
        cases = [success]
        # Execute every member of every independent input equivalence class.
        cases.extend({**success, name: state} for name in names for state in states)
        # Supplement the proof with both orderings of mixed failures and all-
        # failure vectors, checking that one bad lane cannot rescue another.
        for index, first in enumerate(names):
            for second in names[index + 1:]:
                cases.extend(({**success, first: "failure", second: "unknown"},
                              {**success, first: "unknown", second: "failure"}))
        cases.extend({name: state for name in names} for state in states)
        self.assertEqual(len(cases), 83)
        for results in cases:
            with self.subTest(results=results):
                result = self.run_gate(results)
                self.assertEqual(result.returncode == 0, all(value == "success" for value in results.values()),
                                 result.stdout + result.stderr)
                for name, value in results.items():
                    self.assertIn(f"{name}={value}", result.stdout)

    def test_gate_equivalence_contract_rejects_mutations(self):
        script = self.step["run"]
        for label, mutated in (
            ("false success", script.replace("exit 1", "exit 0")),
            ("inverted predicate", script.replace('!= "success"', '== "success"')),
            ("cross-input dependency", script.replace('result="${!result_var}"', 'result="$NATIVE_RESULT"')),
            ("failure reset", script.replace("failed=1", "failed=0")),
            ("early exit", script.replace("failed=1", "exit 1")),
            ("missing output", script.replace('echo "$result_var=$result"', ":")),
            ("extra control flow", script + "\nexit 1\n"),
            ("suffix expansion", script + '\necho "$(exit 1)"\n'),
            ("suffix backticks", script + '\necho "`exit 1`"\n'),
            ("suffix escape", script + '\necho "\\c"\n'),
            ("suffix operator", script + '\necho "ok"; false\n'),
        ):
            with self.subTest(mutation=label):
                self.assertNotEqual(mutated, script)
                with self.assertRaises(AssertionError):
                    self.assert_independent_gate_contract(mutated)

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

    def test_each_required_job_non_success_fails_closed(self):
        for name in RESULTS:
            for status in ("failure", "skipped", "cancelled", "", "unknown", None):
                results = {key: "success" for key in RESULTS if key != name}
                if status is not None:
                    results[name] = status
                with self.subTest(job=name, status=status):
                    result = self.run_gate(results)
                    self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                    self.assertIn(f"::error::{name} must be success", result.stdout)
                    self.assertNotIn("passed", result.stdout)

    def test_unset_results_fail_closed(self):
        for missing in RESULTS:
            results = {name: "success" for name in RESULTS if name != missing}
            with self.subTest(missing=missing):
                result = self.run_gate(results)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
