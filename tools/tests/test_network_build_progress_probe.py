"""Pure contract tests for the bounded network/build-progress probe.

These tests use synthetic libtest output, compiler samples, plans, and fake
children. They never launch Cargo, a test executable, a compiler, or a network
host.
"""
from __future__ import annotations

import copy
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import time
import unittest
from unittest import mock


PROBE_PATH = Path(__file__).resolve().parents[1] / "network_build_progress_probe.py"
SPEC = importlib.util.spec_from_file_location("network_build_progress_probe", PROBE_PATH)
assert SPEC is not None and SPEC.loader is not None
probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(probe)


def valid_plan() -> dict:
    name = "idle_timeout_detected::ws"
    return {
        "version": 1,
        "source": {"head": "a" * 40, "tree": "b" * 40, "files": {}, "manifest_sha256": "manifest"},
        "profile": "release-default-features",
        "repeats": 1,
        "campaign_watchdog_seconds": 1800,
        "build_watchdog_seconds": 900,
        "environment": {},
        "toolchain": {},
        "binaries": {"loopback": {"path": "prebuilt-loopback.exe", "sha256": "binary", "listed_tests": [name]}},
        "cases": [{"binary": "loopback", "name": name, "watchdog_seconds": 90}],
    }


class FakeChild:
    """In-memory child substitute; no OS process is spawned."""

    def __init__(self, argv, cwd, stdout, stderr, env, *, exit_code=0, polls=None, out=b"ok\n", err=b""):
        self.pid = 4242
        self.exit_code = exit_code
        self.polls = list(polls or [exit_code])
        self.out = out
        self.err = err
        self.closed = False
        stdout.write(out)
        stderr.write(err)
        stdout.flush()
        stderr.flush()

    def poll(self):
        if self.polls:
            return self.polls.pop(0)
        return self.exit_code

    def sample(self):
        return {"active": 0, "cpu_100ns": None, "compilers": [], "containment": "fake"}

    def wait(self):
        return self.exit_code

    def close(self):
        self.closed = True


class FakeJobChild(FakeChild):
    """Windows-like cleanup contract driven by finite in-memory samples."""

    requires_job_cleanup = True

    def __init__(self, argv, cwd, stdout, stderr, env, *, observations):
        super().__init__(argv, cwd, stdout, stderr, env, polls=[0], out=b"", err=b"")
        self.observations = list(observations)
        self.current_observation = {"active": None, "cpu_100ns": None, "compilers": []}
        self.closed_at_active = None

    def sample(self):
        if self.observations:
            self.current_observation = self.observations.pop(0)
        return copy.deepcopy(self.current_observation)

    def job_accounting(self):
        return {"active": self.current_observation.get("active"),
                "cpu_100ns": self.current_observation.get("cpu_100ns"), "compilers": []}

    def close(self):
        self.closed_at_active = self.current_observation.get("active")
        if self.closed_at_active != 0:
            raise AssertionError("job-backed child closed before accounting reached zero")
        self.closed = True


class ResultGateTests(unittest.TestCase):
    def test_exact_result_accepts_one_exact_pass_and_observes_filtered_count(self):
        text = (
            "running 4 tests\n"
            "test idle_timeout_detected::ws ... ok\n"
            "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out;\n"
        )
        self.assertEqual(
            probe.exact_result(text, "idle_timeout_detected::ws"),
            {"passed": 1, "failed": 0, "ignored": 0, "measured": 0, "filtered": 3},
        )
        self.assertEqual(probe.exact_result(text, "idle_timeout_detected::ws", expected_filtered=3)["filtered"], 3)
        with self.assertRaises(ValueError):
            probe.exact_result(text, "idle_timeout_detected::ws", expected_filtered=4)

    def test_exact_result_rejects_inventory_filtered_count_mismatch(self):
        text = (
            "test idle_timeout_detected::ws ... ok\n"
            "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out;\n"
        )
        with self.assertRaisesRegex(ValueError, "filtered"):
            probe.exact_result(text, "idle_timeout_detected::ws", expected_filtered=2)

    def test_exact_result_accepts_nocapture_diagnostics_between_prefix_and_summary(self):
        text = (
            "test two_c_abi_clients_and_a_rust_client_play_with_prediction_and_rollback ...\n"
            "client joined: state=1\n"
            "confirmed checksums agree\n"
            "ok\n"
            "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out;\n"
        )
        self.assertEqual(
            probe.exact_result(text, "two_c_abi_clients_and_a_rust_client_play_with_prediction_and_rollback")["passed"],
            1,
        )

    def test_exact_result_rejects_zero_passed(self):
        with self.assertRaises(ValueError):
            probe.exact_result("test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out;\n", "idle_timeout_detected::ws")

    def test_exact_result_rejects_ignored_target(self):
        text = "test idle_timeout_detected::ws ... ignored\ntest result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 3 filtered out;\n"
        with self.assertRaises(ValueError):
            probe.exact_result(text, "idle_timeout_detected::ws")

    def test_exact_result_rejects_wrong_selector(self):
        text = "test idle_timeout_detected::quic ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out;\n"
        with self.assertRaises(ValueError):
            probe.exact_result(text, "idle_timeout_detected::ws")

    def test_exact_result_rejects_failed_summary(self):
        text = "test idle_timeout_detected::ws ... FAILED\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 3 filtered out;\n"
        with self.assertRaises(ValueError):
            probe.exact_result(text, "idle_timeout_detected::ws")

    def test_exact_result_rejects_duplicate_summary_or_target_line(self):
        line = "test idle_timeout_detected::ws ... ok\n"
        summary = "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out;\n"
        with self.subTest("duplicate summary"):
            with self.assertRaises(ValueError):
                probe.exact_result(line + summary + summary, "idle_timeout_detected::ws")
        with self.subTest("duplicate target"):
            with self.assertRaises(ValueError):
                probe.exact_result(line + line + summary, "idle_timeout_detected::ws")


class CompilerOverlapTests(unittest.TestCase):
    @staticmethod
    def sample(t, compilers):
        return {"monotonic": t, "compilers": compilers}

    @staticmethod
    def compiler(pid=10, creation=100, cpu=10):
        return {"pid": pid, "creation_100ns": creation, "cpu_100ns": cpu}

    def test_requires_cpu_progress_covering_full_interval(self):
        samples = [
            self.sample(0.0, [self.compiler(cpu=10)]),
            self.sample(0.5, [self.compiler(cpu=20)]),
            self.sample(1.0, [self.compiler(cpu=30)]),
            self.sample(1.5, [self.compiler(cpu=40)]),
        ]
        self.assertTrue(probe.compiler_overlap(samples, 0.0, 1.5))

    def test_cargo_without_rustc_is_not_compiler_overlap(self):
        samples = [self.sample(0.0, []), self.sample(0.5, [])]
        self.assertFalse(probe.compiler_overlap(samples, 0.0, 0.5))

    def test_unchanged_rustc_cpu_is_cache_or_idle_not_progress(self):
        samples = [self.sample(0.0, [self.compiler(cpu=20)]), self.sample(0.5, [self.compiler(cpu=20)])]
        self.assertFalse(probe.compiler_overlap(samples, 0.0, 0.5))

    def test_discontinuous_samples_do_not_cover_gap(self):
        samples = [
            self.sample(0.0, [self.compiler(cpu=10)]),
            self.sample(0.5, [self.compiler(cpu=20)]),
            self.sample(2.0, [self.compiler(cpu=30)]),
            self.sample(2.5, [self.compiler(cpu=40)]),
        ]
        self.assertFalse(probe.compiler_overlap(samples, 0.0, 2.5))

    def test_partial_progress_does_not_cover_requested_interval(self):
        samples = [
            self.sample(0.5, [self.compiler(cpu=10)]),
            self.sample(1.0, [self.compiler(cpu=20)]),
            self.sample(1.5, [self.compiler(cpu=30)]),
        ]
        self.assertFalse(probe.compiler_overlap(samples, 0.0, 1.5))

    def test_reused_pid_with_new_creation_time_is_not_continuous_progress(self):
        samples = [
            self.sample(0.0, [self.compiler(pid=7, creation=100, cpu=100)]),
            self.sample(0.5, [self.compiler(pid=7, creation=200, cpu=200)]),
        ]
        self.assertFalse(probe.compiler_overlap(samples, 0.0, 0.5))


class PlanValidationTests(unittest.TestCase):
    def prepared(self):
        helper = PreparationAndReservationTests("test_prepare_uses_fresh_isolated_target_and_explicit_target_dir_argv")
        fixture = helper.prepare_fixture()
        self.addCleanup(helper.doCleanups)
        return helper, fixture

    def test_unsupported_inherited_build_override_is_rejected(self):
        with mock.patch.dict(probe.os.environ, {"CARGO_TARGET_X86_64_CUSTOM_LINKER": "synthetic"}, clear=True):
            with self.assertRaisesRegex(ValueError, "unsupported inherited build override"):
                probe.environment_identity()

    def validate(self, plan, root, *, current_source=None, current_toolchain=None, binary_digest=None):
        current_source = current_source if current_source is not None else plan["source"]
        with mock.patch.object(probe, "source_identity", return_value=current_source), mock.patch.object(
            probe, "digest", wraps=probe.digest if binary_digest is None else None
        ) as digest_mock, mock.patch.object(probe, "environment_identity", return_value=plan.get("parent_environment", {})), mock.patch.object(
            probe, "toolchain_identity", return_value=current_toolchain if current_toolchain is not None else plan.get("toolchain", {})
        ):
            if binary_digest is not None:
                digest_mock.return_value = binary_digest
            probe.validate_plan(plan, root)

    def resigned(self, helper, plan):
        helper.resign_plan_marker(plan)
        return plan

    def test_valid_prebuilt_plan_is_accepted(self):
        _, (root, _, _, plan, _) = self.prepared()
        self.validate(plan, root)

    def test_repeat_count_zero_and_four_are_rejected(self):
        helper, (root, _, _, original, _) = self.prepared()
        for repeats in (0, 4):
            with self.subTest(repeats=repeats):
                plan = copy.deepcopy(original)
                plan["repeats"] = repeats
                self.resigned(helper, plan)
                with self.assertRaises(ValueError):
                    self.validate(plan, root)

    def test_boolean_repeat_count_is_rejected_even_though_bool_is_an_int_subclass(self):
        helper, (root, _, _, original, _) = self.prepared()
        plan = copy.deepcopy(original)
        plan["repeats"] = True
        self.resigned(helper, plan)
        with self.assertRaises(ValueError):
            self.validate(plan, root)

    def test_boolean_case_build_and_campaign_watchdogs_are_rejected(self):
        helper, (root, _, _, original, _) = self.prepared()
        mutations = (
            lambda p: p["cases"][0].__setitem__("watchdog_seconds", True),
            lambda p: p.__setitem__("build_watchdog_seconds", True),
            lambda p: p.__setitem__("campaign_watchdog_seconds", True),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                plan = copy.deepcopy(original)
                mutate(plan)
                self.resigned(helper, plan)
                with self.assertRaises(ValueError):
                    self.validate(plan, root)

    def test_duplicate_case_is_rejected(self):
        helper, (root, _, _, original, _) = self.prepared()
        plan = copy.deepcopy(original)
        plan["cases"].append(copy.deepcopy(plan["cases"][0]))
        self.resigned(helper, plan)
        with self.assertRaises(ValueError):
            self.validate(plan, root)

    def test_unapproved_c_client_case_is_rejected(self):
        helper, (root, _, _, original, _) = self.prepared()
        plan = copy.deepcopy(original)
        plan["cases"] = [{"binary": "c_client", "name": "c_program_sees_what_the_rust_bridge_sees", "watchdog_seconds": 90}]
        self.resigned(helper, plan)
        with self.assertRaises(ValueError):
            self.validate(plan, root)

    def test_source_revision_or_manifest_drift_is_rejected(self):
        _, (root, _, _, plan, _) = self.prepared()
        changed = dict(plan["source"], head="d" * 40)
        with self.assertRaises(ValueError):
            self.validate(plan, root, current_source=changed)

    def test_toolchain_drift_is_rejected(self):
        helper, (root, _, _, plan, _) = self.prepared()
        changed = {"rustc": "rustc B", "cargo": "cargo A"}
        with self.assertRaises(ValueError):
            self.validate(plan, root, current_toolchain=changed)

    def test_prebuilt_binary_hash_drift_is_rejected(self):
        _, (root, _, _, plan, _) = self.prepared()
        with self.assertRaises(ValueError):
            self.validate(plan, root, binary_digest="changed-binary")

    def test_selector_not_listed_by_prebuilt_binary_is_rejected(self):
        helper, (root, _, _, original, _) = self.prepared()
        plan = copy.deepcopy(original)
        plan["binaries"]["loopback"]["listed_tests"] = ["idle_timeout_detected::quic"]
        self.resigned(helper, plan)
        with self.assertRaises(ValueError):
            self.validate(plan, root)


class PreparationAndReservationTests(unittest.TestCase):
    HEAD = "a" * 40
    SOURCE = {"head": HEAD, "tree": "b" * 40, "files": {}, "manifest_sha256": "manifest"}
    PARENT_ENV = {key: None for key in probe.ENV_KEYS}
    TOOLCHAIN = {"cargo": {"path": "cargo.exe", "sha256": "c" * 64, "version": "cargo 1"},
                 "rustc": {"path": "rustc.exe", "sha256": "d" * 64, "version": "rustc 1"}}

    class FinishedCapture:
        def __init__(self, folder, stdout=""):
            self.folder = Path(folder)
            self.folder.mkdir(parents=True, exist_ok=False)
            (self.folder / "stdout.txt").write_text(stdout, encoding="utf-8")

        def finish(self):
            return {"actual_exit": 0, "cleanup_complete": True, "handles_closed": True,
                    "failure": None, "start": 1.0, "end": 2.0}

    def prepare_fixture(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name) / "repo"
        target_root = root / "target"
        target_root.mkdir(parents=True)
        output = target_root / "prepare-once"
        target_dir = target_root / "isolated-build"
        build_argv = []

        def fake_launch(argv, cwd, folder, watchdog, env):
            argv = list(argv)
            if argv[0] == "cargo":
                target = argv[argv.index("--test") + 1]
                build_argv.append((argv, dict(env)))
                exe = target_dir / (target + ".exe")
                exe.write_bytes((target + " prebuilt").encode())
                artifact = {"reason": "compiler-artifact", "target": {"name": target},
                            "executable": str(exe)}
                return self.FinishedCapture(folder, probe.json.dumps(artifact) + "\n")
            target = Path(argv[0]).stem
            listed = list(probe.SAFE_CASES[target][1]) + ["unrelated_prepared_test"]
            return self.FinishedCapture(folder, "".join(name + ": test\n" for name in listed))

        with mock.patch.object(probe, "require_runtime_platform"), mock.patch.object(
            probe, "git", return_value=self.HEAD
        ), mock.patch.object(probe, "source_identity", return_value=copy.deepcopy(self.SOURCE)), mock.patch.object(
            probe, "environment_identity", return_value=copy.deepcopy(self.PARENT_ENV)
        ), mock.patch.object(probe, "toolchain_identity", return_value=copy.deepcopy(self.TOOLCHAIN)), mock.patch.object(
            probe, "launch", side_effect=fake_launch
        ):
            plan = probe.prepare(root, output, self.HEAD, target_dir)
        return root, output, target_dir, plan, build_argv

    def validate(self, root, plan):
        with mock.patch.object(probe, "source_identity", return_value=copy.deepcopy(self.SOURCE)), mock.patch.object(
            probe, "environment_identity", return_value=copy.deepcopy(self.PARENT_ENV)
        ), mock.patch.object(probe, "toolchain_identity", return_value=copy.deepcopy(self.TOOLCHAIN)):
            probe.validate_plan(plan, root)

    def resign_plan_marker(self, plan):
        marker_path = Path(plan["preparation_marker"])
        marker = probe.json.loads(marker_path.read_text(encoding="utf-8"))
        marker["plan_sha256"] = probe.canonical_sha256(plan)
        probe.save(marker_path, marker)

    def test_prepare_uses_fresh_isolated_target_and_explicit_target_dir_argv(self):
        root, output, target, plan, build_argv = self.prepare_fixture()
        self.assertTrue(target.is_dir())
        self.assertEqual(plan["status"], "prepared")
        self.assertEqual(plan["target_dir"], str(target.resolve()))
        self.assertEqual(plan["child_environment"]["CARGO_TARGET_DIR"], str(target.resolve()))
        self.assertEqual(plan["child_environment_overrides"], {
            "CARGO_TARGET_DIR": str(target.resolve()), "CARGO_TERM_COLOR": "never"})
        self.assertEqual(len(build_argv), len(probe.SAFE_CASES))
        for argv, env in build_argv:
            target_name = argv[argv.index("--test") + 1]
            crate = probe.SAFE_CASES[target_name][0]
            self.assertEqual(argv, ["cargo", "test", "--release", "--locked", "--offline", "-p", crate,
                                    "--test", target_name, "--no-run", "--message-format=json",
                                    "--target-dir", str(target.resolve())])
            self.assertEqual(env["CARGO_TARGET_DIR"], str(target.resolve()))
        self.validate(root, plan)

    def test_preparation_once_marker_blocks_same_identity_in_another_output(self):
        root, _, _, _, first_argv = self.prepare_fixture()
        other_output = root / "target" / "prepare-second"
        other_target = root / "target" / "isolated-build-second"
        launches = mock.Mock()
        with mock.patch.object(probe, "require_runtime_platform"), mock.patch.object(
            probe, "git", return_value=self.HEAD
        ), mock.patch.object(probe, "source_identity", return_value=copy.deepcopy(self.SOURCE)), mock.patch.object(
            probe, "environment_identity", return_value=copy.deepcopy(self.PARENT_ENV)
        ), mock.patch.object(probe, "toolchain_identity", return_value=copy.deepcopy(self.TOOLCHAIN)), mock.patch.object(
            probe, "launch", launches
        ):
            with self.assertRaises((FileExistsError, OSError)):
                probe.prepare(root, other_output, self.HEAD, other_target)
        launches.assert_not_called()
        self.assertFalse(other_output.exists())
        self.assertFalse(other_target.exists())
        self.assertEqual(len(first_argv), len(probe.SAFE_CASES))

    def test_preparation_rejects_existing_target_before_any_launch(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "repo"
            target_root = root / "target"
            target_root.mkdir(parents=True)
            output, target = target_root / "prepare", target_root / "occupied"
            target.mkdir()
            (target / "old-artifact").write_text("keep", encoding="utf-8")
            launch = mock.Mock()
            with mock.patch.object(probe, "require_runtime_platform"), mock.patch.object(
                probe, "git", return_value=self.HEAD
            ), mock.patch.object(probe, "source_identity", return_value=copy.deepcopy(self.SOURCE)), mock.patch.object(
                probe, "environment_identity", return_value=copy.deepcopy(self.PARENT_ENV)
            ), mock.patch.object(probe, "toolchain_identity", return_value=copy.deepcopy(self.TOOLCHAIN)), mock.patch.object(
                probe, "launch", launch
            ):
                with self.assertRaisesRegex(ValueError, "target must not preexist"):
                    probe.prepare(root, output, self.HEAD, target)
            launch.assert_not_called()
            self.assertEqual((target / "old-artifact").read_text(encoding="utf-8"), "keep")

    def test_tracked_and_untracked_source_changes_are_rejected(self):
        for status in (" M crates/orr_server/src/lib.rs", "?? scratch-created-by-user"):
            with self.subTest(status=status), mock.patch.object(
                probe, "git", return_value=status
            ) as git:
                with self.assertRaisesRegex(ValueError, "tracked or untracked changes"):
                    probe.source_identity(Path("synthetic-repo"))
                git.assert_called_once_with(Path("synthetic-repo"), "status", "--porcelain", "--untracked-files=all")

    def test_prepared_status_failure_and_partial_receipts_are_rejected(self):
        root, _, _, original, _ = self.prepare_fixture()
        mutations = (
            ("incomplete status", lambda plan: plan.__setitem__("status", "preparing")),
            ("recorded failure", lambda plan: plan.__setitem__("failure", "synthetic prior failure")),
            ("partial build receipt", lambda plan: plan["preparation"][0].__setitem__("cleanup_complete", False)),
            ("partial listing receipt", lambda plan: plan["binaries"]["loopback"]["listing"].__setitem__("handles_closed", False)),
        )
        for label, mutate in mutations:
            with self.subTest(label=label):
                plan = copy.deepcopy(original)
                mutate(plan)
                self.resign_plan_marker(plan)
                with self.assertRaises(ValueError):
                    self.validate(root, plan)

    def test_watchdog_caps_are_enforced_for_cases_build_and_campaign(self):
        root, _, _, original, _ = self.prepare_fixture()
        cases = (
            ("native case", lambda p: p["cases"][0].__setitem__("watchdog_seconds", 91)),
            ("FFI case", lambda p: next(c for c in p["cases"] if c["binary"] == "client_session").__setitem__("watchdog_seconds", 301)),
            ("build", lambda p: p.__setitem__("build_watchdog_seconds", 901)),
            ("campaign", lambda p: p.__setitem__("campaign_watchdog_seconds", 1801)),
        )
        for label, mutate in cases:
            with self.subTest(label=label):
                plan = copy.deepcopy(original)
                mutate(plan)
                self.resign_plan_marker(plan)
                with self.assertRaises(ValueError):
                    self.validate(root, plan)

    def test_campaign_once_marker_blocks_new_output_and_modified_plan_before_launch(self):
        root, _, _, plan, _ = self.prepare_fixture()
        launch_count = []

        class CompletedProbe:
            def __init__(self, folder, argv):
                self.folder = Path(folder)
                self.folder.mkdir(parents=True, exist_ok=False)
                name = argv[1]
                binary_name = Path(argv[0]).stem
                listed = plan["binaries"][binary_name]["listed_tests"]
                filtered = len(listed) - 1
                output = (f"test {name} ... ok\n"
                          f"test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; {filtered} filtered out;\n")
                (self.folder / "stdout.txt").write_text(output, encoding="utf-8")
                self.record = {"failure": None, "start": 1.0, "end": 2.0}
                self.child = type("Child", (), {"poll": lambda self: 0})()

            def finish(self):
                return {"failure": None, "start": 1.0, "end": 2.0, "actual_exit": 0,
                        "cleanup_complete": True, "handles_closed": True}

        def fake_launch(argv, cwd, folder, watchdog, env):
            launch_count.append(list(argv))
            return CompletedProbe(folder, argv)

        with mock.patch.object(probe, "require_runtime_platform"), mock.patch.object(
            probe, "source_identity", return_value=copy.deepcopy(self.SOURCE)
        ), mock.patch.object(probe, "environment_identity", return_value=copy.deepcopy(self.PARENT_ENV)), mock.patch.object(
            probe, "toolchain_identity", return_value=copy.deepcopy(self.TOOLCHAIN)
        ), mock.patch.object(probe, "launch", side_effect=fake_launch), tempfile.TemporaryDirectory() as temp:
            first = probe.campaign(plan, root, Path(temp) / "campaign-one", mode="idle-only")
            self.assertEqual(first["status"], "idle_passed")
            launched_after_first = len(launch_count)
            second = probe.campaign(plan, root, Path(temp) / "campaign-two", mode="idle-only")
            self.assertEqual(second["status"], "failed_stop")
            self.assertEqual(len(launch_count), launched_after_first)

        root2, _, _, modified, _ = self.prepare_fixture()
        modified["os"] = str(modified.get("os", "")) + " changed"
        with mock.patch.object(probe, "require_runtime_platform"), mock.patch.object(
            probe, "source_identity", return_value=copy.deepcopy(self.SOURCE)
        ), mock.patch.object(probe, "environment_identity", return_value=copy.deepcopy(self.PARENT_ENV)), mock.patch.object(
            probe, "toolchain_identity", return_value=copy.deepcopy(self.TOOLCHAIN)
        ), mock.patch.object(probe, "launch") as launch, tempfile.TemporaryDirectory() as temp:
            report = probe.campaign(modified, root2, Path(temp) / "modified-plan", mode="idle-only")
            self.assertEqual(report["status"], "failed_stop")
            launch.assert_not_called()

    @unittest.skipUnless(probe.os.name == "nt", "paired compiler sampling is Windows-only")
    def test_builder_sample_failure_stops_before_probe_launch(self):
        root, _, _, plan, _ = self.prepare_fixture()
        launches = []

        class Builder:
            def __init__(self, folder):
                self.folder = Path(folder)
                self.record = {"failure": None, "samples": [], "start": 1.0}
                self.child = type("Child", (), {"poll": lambda self: None})()
                self.calls = 0
                self.finished = False

            def sample(self):
                self.calls += 1
                if self.calls == 1:
                    self.record["samples"].append({"monotonic": 0.0, "compilers": [
                        {"pid": 11, "creation_100ns": 22, "cpu_100ns": 100}]})
                    return
                if self.calls == 2:
                    self.record["samples"].append({"monotonic": 1.0, "compilers": [
                        {"pid": 11, "creation_100ns": 22, "cpu_100ns": 200}]})
                    return
                self.record["failure"] = "process_observation_failed: synthetic builder sample failure"
                return

            def finish(self):
                self.finished = True
                return {"failure": "synthetic builder sample failure", "cleanup_complete": True,
                        "handles_closed": True, "actual_exit": 0}

        builder = None

        def fake_launch(argv, cwd, folder, watchdog, env):
            nonlocal builder
            launches.append(list(argv))
            if argv[0] == "cargo":
                builder = Builder(folder)
                return builder
            # Idle cases precede the paired build condition by contract.
            self.assertIsNone(builder)
            case_name = argv[1]
            binary_name = Path(argv[0]).stem
            filtered = len(plan["binaries"][binary_name]["listed_tests"]) - 1
            folder = Path(folder)
            folder.mkdir(parents=True, exist_ok=False)
            (folder / "stdout.txt").write_text(
                f"test {case_name} ... ok\n"
                f"test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; {filtered} filtered out;\n",
                encoding="utf-8",
            )

            class CompletedIdleProbe:
                record = {"failure": None, "start": 1.0, "end": 2.0, "actual_exit": 0,
                          "cleanup_complete": True, "handles_closed": True}
                child = type("Child", (), {"poll": lambda self: 0})()

                def __init__(self, output_folder):
                    self.folder = output_folder

                def finish(self):
                    return dict(self.record)

            return CompletedIdleProbe(folder)

        with tempfile.TemporaryDirectory() as temp, mock.patch.object(
            probe, "require_runtime_platform"
        ), mock.patch.object(probe, "validate_plan"), mock.patch.object(probe, "launch", side_effect=fake_launch), mock.patch.object(
            probe.time, "sleep", return_value=None
        ):
            result = probe.campaign(plan, root, Path(temp) / "campaign", mode="paired")

        build_indices = [i for i, argv in enumerate(launches) if argv[:2] == ["cargo", "build"]]
        self.assertEqual(len(build_indices), 1)
        self.assertEqual(build_indices[0], len(plan["cases"]))
        self.assertEqual(len(launches), len(plan["cases"]) + 1)
        self.assertTrue(all(attempt["condition"] == "idle" and attempt["status"] == "passed"
                            for attempt in result["attempts"]))
        self.assertEqual(len(result["attempts"]), len(plan["cases"]))
        self.assertIn("observation failed before probe launch", result["failure"])
        self.assertIn("synthetic builder sample failure", result["build"]["failure"])
        self.assertEqual(result["status"], "failed_stop")
        self.assertTrue(builder.finished)


class CaptureLifecycleTests(unittest.TestCase):
    def capture_job_child(self, folder, observations):
        child = None

        def factory(argv, cwd, stdout, stderr, env):
            nonlocal child
            child = FakeJobChild(argv, cwd, stdout, stderr, env, observations=observations)
            return child

        capture = probe.Capture(
            ["fake-test-executable", "case", "--exact"], Path("fake-root"), folder, 90, {}, factory=factory
        )
        return capture, lambda: child

    @staticmethod
    def observation(active, *, errors=(), identities=(), compilers=()):
        return {"active": active, "cpu_100ns": 100, "compilers": list(compilers),
                "process_identities": list(identities), "observation_errors": list(errors)}

    def test_accounting_survives_image_error_and_waits_for_zero_before_close(self):
        rows = [
            self.observation(2, errors=("rustc image query failed for PID 51",)),
            self.observation(0),
        ]
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(probe.time, "sleep", return_value=None):
            capture, get_child = self.capture_job_child(Path(temp) / "capture", rows)
            record = capture.finish()

        child = get_child()
        self.assertEqual(record["actual_exit"], 0)
        self.assertIn("rustc image query failed for PID 51", record["failure"])
        self.assertEqual([row["active"] for row in record["samples"]], [2, 0])
        self.assertTrue(record["cleanup_complete"])
        self.assertTrue(record["handles_closed"])
        self.assertTrue(child.closed)
        self.assertEqual(child.closed_at_active, 0)

    def test_unknown_accounting_waits_through_one_and_zero_without_hanging(self):
        rows = [
            self.observation(None, errors=("job accounting temporarily unavailable",)),
            self.observation(None, errors=("job accounting still unavailable",)),
            self.observation(1),
            self.observation(0),
        ]
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(probe.time, "sleep", return_value=None):
            capture, get_child = self.capture_job_child(Path(temp) / "capture", rows)
            record = capture.finish()

        child = get_child()
        self.assertEqual([row["active"] for row in record["samples"]], [None, None, 1, 0])
        self.assertIn("job accounting temporarily unavailable", record["failure"])
        self.assertIn("job accounting still unavailable", record["secondary_failures"][0])
        self.assertTrue(record["cleanup_complete"])
        self.assertTrue(record["handles_closed"])
        self.assertEqual(child.closed_at_active, 0)
        self.assertTrue(capture._cleanup_pending({"active": None}))
        self.assertTrue(capture._cleanup_pending({"active": 1}))
        self.assertFalse(capture._cleanup_pending({"active": 0}))

    def test_zero_accounting_allows_close_despite_independent_image_error(self):
        rows = [self.observation(0, errors=("rustc image lookup failed after job reached zero",))]
        with tempfile.TemporaryDirectory() as temp:
            capture, get_child = self.capture_job_child(Path(temp) / "capture", rows)
            record = capture.finish()

        child = get_child()
        self.assertIn("rustc image lookup failed after job reached zero", record["failure"])
        self.assertEqual(record["samples"][0]["active"], 0)
        self.assertTrue(record["cleanup_complete"])
        self.assertTrue(record["handles_closed"])
        self.assertEqual(child.closed_at_active, 0)

    def test_membership_race_keeps_active_count_without_compiler_attribution(self):
        # PID 51 disappeared/recycled before its image could be identified.
        # The authoritative job accounting still says one process remains.
        rows = [self.observation(1, identities=(), compilers=()), self.observation(0)]
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(probe.time, "sleep", return_value=None):
            capture, _ = self.capture_job_child(Path(temp) / "capture", rows)
            record = capture.finish()

        self.assertEqual(record["samples"][0]["active"], 1)
        self.assertEqual(record["samples"][0]["compilers"], [])
        self.assertEqual(record["samples"][0]["process_identities"], [])
        self.assertEqual(record["samples"][-1]["active"], 0)
        self.assertTrue(record["cleanup_complete"])

    def test_reopened_nonmember_pid_is_not_image_queried_or_attributed(self):
        class FakeKernel:
            def __init__(self):
                self.calls = []

            def OpenProcess(self, access, inherit, pid):
                self.calls.append(("open", pid))
                return 501

            def IsProcessInJob(self, handle, job, member_ptr):
                self.calls.append(("membership", handle, job))
                probe.ctypes.cast(member_ptr, probe.ctypes.POINTER(probe.wintypes.BOOL))[0] = 0
                return 1

            def CloseHandle(self, handle):
                self.calls.append(("close", handle))
                return 1

            def QueryFullProcessImageNameW(self, *args):
                raise AssertionError("a PID reused outside the job must not be image queried")

            def GetProcessTimes(self, *args):
                raise AssertionError("a non-member PID must not receive compiler identity attribution")

            def QueryInformationJobObject(self, job, info_class, buffer, size, returned):
                if info_class == probe._JOB_BASIC_ACCOUNTING:
                    acct = probe.ctypes.cast(buffer, probe.ctypes.POINTER(probe._JOB_ACCOUNTING)).contents
                    acct.ActiveProcesses = 1
                    return 1
                raise AssertionError("unexpected job query")

        child = object.__new__(probe.WindowsChild)
        child._assigned, child._closed, child._job = True, False, 77
        kernel = FakeKernel()
        with mock.patch.object(probe, "_api", return_value=kernel), mock.patch.object(
            probe, "_job_pids", return_value=[51]
        ):
            sample = child.sample()

        self.assertEqual(sample["active"], 1)
        self.assertEqual(sample["compilers"], [])
        self.assertEqual(sample["observation_errors"], [])
        self.assertEqual(sample["process_identities"], [
            {"pid": 51, "member": False, "creation_100ns": None}
        ])
        self.assertEqual(kernel.calls, [("open", 51), ("membership", 501, 77), ("close", 501)])

    def test_authoritative_active_survives_member_image_error_after_creation_identity(self):
        class FakeKernel:
            def __init__(self):
                self.calls = []

            def QueryInformationJobObject(self, job, info_class, buffer, size, returned):
                self.calls.append("job_accounting")
                acct = probe.ctypes.cast(buffer, probe.ctypes.POINTER(probe._JOB_ACCOUNTING)).contents
                acct.ActiveProcesses = 2
                acct.TotalUserTime = 30
                acct.TotalKernelTime = 40
                return 1

            def OpenProcess(self, access, inherit, pid):
                self.calls.append("open_process")
                return 502

            def IsProcessInJob(self, handle, job, member_ptr):
                self.calls.append("membership")
                probe.ctypes.cast(member_ptr, probe.ctypes.POINTER(probe.wintypes.BOOL))[0] = 1
                return 1

            def GetProcessTimes(self, handle, created_ptr, exited_ptr, kernel_ptr, user_ptr):
                self.calls.append("process_times")
                creation = probe.ctypes.cast(created_ptr, probe.ctypes.POINTER(probe._FILETIME)).contents
                creation.dwLowDateTime = 123
                creation.dwHighDateTime = 0
                return 1

            def QueryFullProcessImageNameW(self, *args):
                self.calls.append("image_query")
                raise OSError("synthetic image access denied")

            def CloseHandle(self, handle):
                self.calls.append("close_process")
                return 1

        child = object.__new__(probe.WindowsChild)
        child._assigned, child._closed, child._job = True, False, 88
        kernel = FakeKernel()
        with mock.patch.object(probe, "_api", return_value=kernel), mock.patch.object(
            probe, "_job_pids", return_value=[52, 53]
        ):
            sample = child.sample()

        self.assertEqual(sample["active"], 2)
        self.assertEqual(sample["cpu_100ns"], 70)
        self.assertEqual(sample["compilers"], [])
        self.assertEqual(len(sample["process_identities"]), 2)
        identity = sample["process_identities"][0]
        self.assertEqual(identity["pid"], 52)
        self.assertTrue(identity["member"])
        self.assertEqual(identity["creation_100ns"], 123)
        self.assertNotIn("image", identity)
        self.assertEqual(sample["observation_errors"], ["OSError('synthetic image access denied')"] * 2)
        self.assertEqual(kernel.calls.count("job_accounting"), 1)
        self.assertLess(kernel.calls.index("job_accounting"), kernel.calls.index("open_process"))
        self.assertLess(kernel.calls.index("process_times"), kernel.calls.index("image_query"))
        self.assertEqual(kernel.calls.count("close_process"), 2)

    def test_windows_child_close_uses_accounting_without_image_enumeration(self):
        child = object.__new__(probe.WindowsChild)
        child._closed, child._assigned = False, True
        child._parent_natural_exit, child._cleanup_zero_verified = True, False
        child._process, child._thread, child._job, child._stdin = 101, None, 202, None
        child._attr_list = child._attr_storage = None
        kernel = type("Kernel", (), {"CloseHandle": lambda self, handle: True})()
        with mock.patch.object(child, "poll", return_value=0), mock.patch.object(
            child, "job_accounting", return_value={"active": 0, "cpu_100ns": 9, "compilers": []}
        ) as accounting, mock.patch.object(child, "sample", side_effect=AssertionError("close must not enumerate images")), mock.patch.object(
            probe, "_api", return_value=kernel
        ):
            child.close()

        accounting.assert_called_once_with()
        self.assertTrue(child._closed)

    def test_close_accounting_retry_keeps_failure_and_blocks_next_attempt(self):
        helper = PreparationAndReservationTests("test_prepare_uses_fresh_isolated_target_and_explicit_target_dir_argv")
        root, _, _, plan, _ = helper.prepare_fixture()
        self.addCleanup(helper.doCleanups)
        launches = []
        children = []
        process_snapshots = []

        class IntermittentAccountingChild(FakeJobChild):
            def __init__(self, *args, **kwargs):
                super().__init__(*args, **kwargs)
                self.accounting_calls = 0

            def job_accounting(self):
                self.accounting_calls += 1
                if self.accounting_calls == 1:
                    raise OSError("synthetic close-time job accounting failure")
                return super().job_accounting()

            def close(self):
                accounting = self.job_accounting()
                if accounting["active"] != 0:
                    raise RuntimeError("fake job still active")
                self.closed_at_active = accounting["active"]
                self.closed = True

        def fake_launch(argv, cwd, folder, watchdog, env):
            launches.append(list(argv))

            def factory(child_argv, child_cwd, stdout, stderr, child_env):
                child = IntermittentAccountingChild(
                    child_argv, child_cwd, stdout, stderr, child_env, observations=[self.observation(0)]
                )
                children.append(child)
                return child

            return probe.Capture(argv, cwd, folder, watchdog, env, factory=factory)

        real_save = probe.save

        def saving_with_snapshots(path, value):
            real_save(path, value)
            if Path(path).name == "process.json":
                process_snapshots.append(copy.deepcopy(value))

        with tempfile.TemporaryDirectory() as temp, mock.patch.object(
            probe, "require_runtime_platform"
        ), mock.patch.object(probe, "source_identity", return_value=copy.deepcopy(helper.SOURCE)), mock.patch.object(
            probe, "environment_identity", return_value=copy.deepcopy(helper.PARENT_ENV)
        ), mock.patch.object(probe, "toolchain_identity", return_value=copy.deepcopy(helper.TOOLCHAIN)), mock.patch.object(
            probe, "launch", side_effect=fake_launch
        ), mock.patch.object(probe, "save", side_effect=saving_with_snapshots), mock.patch.object(
            probe.time, "sleep", return_value=None
        ):
            result = probe.campaign(plan, root, Path(temp) / "campaign", mode="idle-only")

        self.assertEqual(result["status"], "failed_stop")
        self.assertEqual(len(result["attempts"]), 1)
        self.assertEqual(len(launches), 1)
        self.assertIn("handle_closure_unverified", result["failure"])
        self.assertIn("synthetic close-time job accounting failure", result["failure"])
        self.assertEqual(len(children), 1)
        self.assertTrue(children[0].closed)
        self.assertEqual(children[0].closed_at_active, 0)
        failed_close = next(snapshot for snapshot in process_snapshots
                            if snapshot.get("handles_closed") is False and snapshot.get("cleanup_complete") is False)
        self.assertIn("synthetic close-time job accounting failure", failed_close["failure"])
        self.assertTrue(process_snapshots[-1]["handles_closed"])
        self.assertTrue(process_snapshots[-1]["cleanup_complete"])

    def test_reader_start_failure_before_child_preserves_setup_receipt(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp) / "capture"
            factory = mock.Mock()
            with mock.patch.object(probe.threading.Thread, "start", side_effect=RuntimeError("synthetic thread setup")):
                with self.assertRaisesRegex(RuntimeError, "synthetic thread setup"):
                    probe.Capture(["fake"], Path("fake-root"), folder, 90, {}, factory=factory)
            factory.assert_not_called()
            saved = probe.json.loads((folder / "process.json").read_text(encoding="utf-8"))
            self.assertIn("synthetic thread setup", saved["failure"])
            self.assertFalse(saved["cleanup_complete"])
            self.assertIsNone(saved["actual_exit"])

    def capture(self, folder, *, child_options=None, watchdog=90):
        options = child_options or {}
        return probe.Capture(
            ["fake-test-executable", "case", "--exact"],
            Path("fake-root"),
            folder,
            watchdog,
            {},
            factory=lambda argv, cwd, stdout, stderr, env: FakeChild(
                argv, cwd, stdout, stderr, env, **options
            ),
        )

    def test_capture_drains_streams_and_records_natural_cleanup(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp) / "capture"
            capture = self.capture(folder, child_options={"out": b"test output\n", "err": b"warning\n"})
            record = capture.finish()
            self.assertEqual(record["actual_exit"], 0)
            self.assertTrue(record["cleanup_complete"])
            self.assertTrue(record["handles_closed"])
            self.assertFalse(hasattr(capture.child, "terminate"))
            self.assertTrue(all(stream["eof"] for stream in record["streams"].values()))
            self.assertEqual((folder / "stdout.txt").read_bytes(), b"test output\n")
            self.assertEqual((folder / "stderr.txt").read_bytes(), b"warning\n")
            self.assertTrue(capture.child.closed)

    def test_writer_close_error_releases_gate_and_preserves_first_error(self):
        class CloseRaisesAfterClosing:
            def __init__(self, wrapped):
                self.wrapped = wrapped
                self.raised = False

            def write(self, data):
                return self.wrapped.write(data)

            def flush(self):
                return self.wrapped.flush()

            def __enter__(self):
                return self

            def __exit__(self, exc_type, exc_value, traceback):
                self.close()
                return False

            def close(self):
                self.wrapped.close()
                if not self.raised:
                    self.raised = True
                    raise OSError("synthetic writer close failure")

        original_fdopen = probe.os.fdopen
        writers = []

        def fdopen_with_close_error(fd, mode="r", *args, **kwargs):
            stream = original_fdopen(fd, mode, *args, **kwargs)
            if mode == "wb" and not writers:
                stream = CloseRaisesAfterClosing(stream)
                writers.append(stream)
            return stream

        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp) / "capture"
            with mock.patch.object(probe.os, "fdopen", side_effect=fdopen_with_close_error):
                capture = self.capture(folder, child_options={"out": b"drained stdout\n", "err": b"drained stderr\n"})
                record = capture.finish()

            self.assertIn("pipe_writer_close_error:", record["failure"])
            self.assertIn("synthetic writer close failure", record["failure"])
            self.assertEqual(record["actual_exit"], 0)
            self.assertTrue(record["cleanup_complete"])
            self.assertTrue(all(stream["eof"] for stream in record["streams"].values()))
            self.assertEqual((folder / "stdout.txt").read_bytes(), b"drained stdout\n")
            self.assertEqual((folder / "stderr.txt").read_bytes(), b"drained stderr\n")
            self.assertEqual(record["streams"]["stdout"]["sha256"], hashlib.sha256(b"drained stdout\n").hexdigest())
            self.assertEqual(record["streams"]["stderr"]["sha256"], hashlib.sha256(b"drained stderr\n").hexdigest())

    def test_raw_close_error_fails_capture_and_hashes_saved_file(self):
        class CloseRaisesAfterClosing:
            def __init__(self, wrapped):
                self.wrapped = wrapped
                self.raised = False

            def write(self, data):
                return self.wrapped.write(data)

            def __enter__(self):
                return self

            def __exit__(self, exc_type, exc_value, traceback):
                self.close()
                return False

            def close(self):
                self.wrapped.close()
                if not self.raised:
                    self.raised = True
                    raise OSError("synthetic raw close failure")

        original_open = Path.open

        def open_with_raw_close_error(path, mode="r", *args, **kwargs):
            stream = original_open(path, mode, *args, **kwargs)
            if path.name == "stdout.txt" and mode == "wb":
                return CloseRaisesAfterClosing(stream)
            return stream

        data = b"saved before close failure\n"
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp) / "capture"
            with mock.patch.object(Path, "open", new=open_with_raw_close_error):
                capture = self.capture(folder, child_options={"out": data})
                record = capture.finish()

            saved = (folder / "stdout.txt").read_bytes()
            self.assertEqual(saved, data)
            self.assertIn("raw_close_error:", record["failure"])
            self.assertIn("synthetic raw close failure", record["failure"])
            self.assertEqual(record["actual_exit"], 0)
            self.assertTrue(record["cleanup_complete"])
            self.assertEqual(record["streams"]["stdout"]["sha256"], hashlib.sha256(saved).hexdigest())
            self.assertEqual(record["streams"]["stdout"]["saved_bytes"], len(saved))

    def test_partial_raw_write_fails_but_drains_and_hashes_persisted_prefix(self):
        class ShortWrite:
            def __init__(self, wrapped):
                self.wrapped = wrapped
                self.first = True

            def write(self, data):
                if self.first:
                    self.first = False
                    return self.wrapped.write(data[:4])
                return 0

            def __enter__(self):
                return self

            def __exit__(self, exc_type, exc_value, traceback):
                self.wrapped.close()
                return False

            def close(self):
                self.wrapped.close()

        original_open = Path.open

        def open_with_short_raw_write(path, mode="r", *args, **kwargs):
            stream = original_open(path, mode, *args, **kwargs)
            if path.name == "stdout.txt" and mode == "wb":
                return ShortWrite(stream)
            return stream

        payload = b"persist-only-the-first-four-bytes"
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp) / "capture"
            with mock.patch.object(Path, "open", new=open_with_short_raw_write):
                capture = self.capture(folder, child_options={"out": payload})
                record = capture.finish()

            saved = (folder / "stdout.txt").read_bytes()
            stream = record["streams"]["stdout"]
            self.assertEqual(saved, payload[:4])
            self.assertIn("partial raw write", record["failure"])
            self.assertEqual(stream["observed_bytes"], len(payload))
            self.assertEqual(stream["saved_bytes"], len(saved))
            self.assertEqual(stream["sha256"], hashlib.sha256(saved).hexdigest())
            self.assertTrue(stream["eof"])
            self.assertTrue(record["cleanup_complete"])

    def test_watchdog_marks_timeout_but_waits_for_natural_exit(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(probe.time, "sleep", return_value=None):
            folder = Path(temp) / "capture"
            capture = self.capture(folder, watchdog=0, child_options={"polls": [None, 0]})
            record = capture.finish()
            self.assertTrue(record["watchdog_triggered"])
            self.assertEqual(record["failure"], "watchdog_expired_natural_exit_pending")
            self.assertEqual(record["actual_exit"], 0)
            self.assertTrue(record["cleanup_complete"])
            self.assertTrue(record["handles_closed"])

    def test_campaign_stops_after_first_failed_case_and_preserves_report(self):
        helper = PreparationAndReservationTests("test_prepare_uses_fresh_isolated_target_and_explicit_target_dir_argv")
        root, _, _, plan, _ = helper.prepare_fixture()
        self.addCleanup(helper.doCleanups)

        class FailedCapture:
            def __init__(self, folder):
                self.folder = folder
                self.child = mock.Mock()
                self.child.poll.return_value = 9
                self.record = {"start": 1.0, "end": 2.0, "failure": None}

            def finish(self):
                self.record["actual_exit"] = 9
                self.record["cleanup_complete"] = True
                self.record["failure"] = "child_exit_nonzero"
                return self.record

        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp) / "campaign"
            launches = []

            def fake_launch(argv, root, folder, watchdog, env):
                launches.append((list(argv), Path(folder)))
                return FailedCapture(Path(folder))

            with mock.patch.object(probe, "require_runtime_platform", return_value=None), mock.patch.object(
                probe, "source_identity", return_value=copy.deepcopy(helper.SOURCE)
            ), mock.patch.object(probe, "environment_identity", return_value=copy.deepcopy(helper.PARENT_ENV)), mock.patch.object(
                probe, "toolchain_identity", return_value=copy.deepcopy(helper.TOOLCHAIN)
            ), mock.patch.object(probe, "launch", side_effect=fake_launch):
                result = probe.campaign(plan, root, output, mode="idle-only")

            self.assertEqual(result["status"], "failed_stop")
            self.assertIn("probe failed", result["failure"])
            self.assertEqual(len(result["attempts"]), 1)
            self.assertEqual(len(launches), 1)
            self.assertEqual(result["attempts"][0]["process"]["actual_exit"], 9)
            self.assertEqual(result["attempts"][0]["process"]["failure"], "child_exit_nonzero")
            persisted = probe.json.loads((output / "report.json").read_text(encoding="utf-8"))
            self.assertEqual(persisted["status"], "failed_stop")
            self.assertEqual(len(persisted["attempts"]), 1)

    def test_paired_mode_rejects_unsupported_platform_before_launching_children(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path("fake-root")
            output = Path(temp) / "campaign"
            launch = mock.Mock()
            with mock.patch.object(probe, "validate_plan", return_value=None), mock.patch.object(
                probe, "_kernel32", None
            ), mock.patch.object(probe, "launch", launch):
                result = probe.campaign(valid_plan(), root, output, mode="paired")
            self.assertEqual(launch.call_count, 0)
            self.assertEqual(result["status"], "failed_stop")
            self.assertIn("Windows", result["failure"])

    def test_nonzero_exit_is_preserved_as_failure_after_cleanup(self):
        with tempfile.TemporaryDirectory() as temp:
            capture = self.capture(Path(temp) / "capture", child_options={"exit_code": 7})
            record = capture.finish()
            self.assertEqual(record["actual_exit"], 7)
            self.assertEqual(record["failure"], "child_exit_nonzero")
            self.assertTrue(record["cleanup_complete"])
            self.assertTrue(record["handles_closed"])

    def test_nonzero_exit_remains_first_failure_when_final_sample_errors(self):
        class LateSampleErrorChild(FakeChild):
            def sample(self):
                raise OSError("synthetic final observation failure")

        with tempfile.TemporaryDirectory() as temp:
            capture = probe.Capture(
                ["fake-test-executable", "case", "--exact"],
                Path("fake-root"),
                Path(temp) / "capture",
                90,
                {},
                factory=lambda argv, cwd, stdout, stderr, env: LateSampleErrorChild(
                    argv, cwd, stdout, stderr, env, exit_code=7, polls=[7]
                ),
            )
            record = capture.finish()

            self.assertEqual(record["actual_exit"], 7)
            self.assertEqual(record["failure"], "child_exit_nonzero")
            self.assertFalse(record["cleanup_complete"])
            self.assertTrue(record["handles_closed"])
            self.assertTrue(any("synthetic final observation failure" in reason
                                for reason in record["secondary_failures"]))

    def test_raw_output_cap_truncates_and_first_failure_is_preserved(self):
        with tempfile.TemporaryDirectory() as temp:
            with mock.patch.object(probe, "MAX_STREAM", 4):
                folder = Path(temp) / "capture"
                capture = self.capture(folder, child_options={"exit_code": 7, "out": b"abcdefgh"})
                deadline = time.monotonic() + 3
                while capture.record["failure"] is None and time.monotonic() < deadline:
                    time.sleep(0.001)
                self.assertEqual(capture.record["failure"], "raw_output_cap_exceeded")
                record = capture.finish()
            stream = record["streams"]["stdout"]
            self.assertEqual((folder / "stdout.txt").read_bytes(), b"abcd")
            self.assertEqual(stream["observed_bytes"], 8)
            self.assertEqual(stream["saved_bytes"], 4)
            self.assertEqual(stream["sha256"], hashlib.sha256(b"abcd").hexdigest())
            self.assertEqual(record["failure"], "raw_output_cap_exceeded")
            self.assertEqual(record["actual_exit"], 7)
            self.assertTrue(record["cleanup_complete"])
            self.assertTrue(record["handles_closed"])


@unittest.skipUnless(probe.os.name == "nt", "Windows query-handle ownership is Windows-only")
class WindowsQueryHandleRetentionTests(unittest.TestCase):
    @unittest.skipUnless(probe.os.name == "nt", "Windows query-handle ownership is Windows-only")
    def make_windows_child(self, job=707):
        child = object.__new__(probe.WindowsChild)
        child._assigned, child._closed, child._job = True, False, job
        child._process, child._thread, child._stdin = 101, None, None
        child._parent_natural_exit, child._cleanup_zero_verified = True, False
        child._attr_list = child._attr_storage = None
        child._query_handles = []
        return child

    @staticmethod
    def observation(active, *, errors=(), identities=(), compilers=()):
        return {"active": active, "cpu_100ns": 100, "compilers": list(compilers),
                "process_identities": list(identities), "observation_errors": list(errors)}

    class FakeKernel:
        def __init__(self, *, active=1, allow_query_close=False):
            self.active = active
            self.allow_query_close = allow_query_close
            self.open_process_calls = []
            self.close_handle_calls = []
            self.api_order = []

        def QueryInformationJobObject(self, job, info_class, buffer, size, returned):
            self.api_order.append("job_accounting")
            if info_class != probe._JOB_BASIC_ACCOUNTING:
                raise AssertionError("unexpected job information class")
            acct = probe.ctypes.cast(buffer, probe.ctypes.POINTER(probe._JOB_ACCOUNTING)).contents
            acct.ActiveProcesses = self.active
            acct.TotalUserTime = 30
            acct.TotalKernelTime = 40
            return 1

        def OpenProcess(self, access, inherit, pid):
            self.api_order.append("open_process")
            self.open_process_calls.append(pid)
            return 501

        def IsProcessInJob(self, handle, job, member_ptr):
            self.api_order.append("membership")
            probe.ctypes.cast(member_ptr, probe.ctypes.POINTER(probe.wintypes.BOOL))[0] = 1
            return 1

        def GetProcessTimes(self, handle, created_ptr, exited_ptr, kernel_ptr, user_ptr):
            self.api_order.append("process_times")
            created = probe.ctypes.cast(created_ptr, probe.ctypes.POINTER(probe._FILETIME)).contents
            created.dwLowDateTime, created.dwHighDateTime = 123, 0
            return 1

        def QueryFullProcessImageNameW(self, handle, flags, image, needed_ptr):
            self.api_order.append("image_query")
            image.value = "C:\\Rust\\rustc.exe"
            return 1

        def CloseHandle(self, handle):
            self.api_order.append("close_handle")
            self.close_handle_calls.append(handle)
            if handle == 501 and not self.allow_query_close:
                probe.ctypes.set_last_error(5)
                return 0
            return 1

    def test_sample_retains_failed_query_handle_and_blocks_pid_reopen_until_retry(self):
        child = self.make_windows_child()
        kernel = self.FakeKernel()
        with mock.patch.object(probe, "_api", return_value=kernel), mock.patch.object(
            probe, "_job_pids", return_value=[51]
        ):
            first = child.sample()
            self.assertEqual(first["active"], 1)
            self.assertEqual(first["query_handles_pending"], 1)
            self.assertEqual(first["query_handle_identities"][0]["pid"], 51)
            self.assertEqual(first["query_handle_identities"][0]["creation_100ns"], 123)
            self.assertIn("close_error", first["query_handle_identities"][0])
            self.assertNotIn("handle", first["query_handle_identities"][0])
            self.assertEqual(len(child._query_handles), 1)
            retained = child._query_handles[0]
            self.assertEqual(retained["handle"], 501)
            self.assertEqual(retained["identity"]["pid"], 51)

            second = child.sample()
            self.assertEqual(second["query_handles_pending"], 1)
            self.assertEqual(second["query_handle_identities"][0]["pid"], 51)
            self.assertEqual(kernel.open_process_calls, [51])
            self.assertEqual(len(child._query_handles), 1)
            self.assertIs(child._query_handles[0], retained)
            self.assertTrue(child._query_handles[0]["identity"]["member"])

            kernel.allow_query_close = True
            retry_errors = child._retry_query_handle_closes(kernel)

        self.assertEqual(retry_errors, [])
        self.assertEqual(child._query_handles, [])
        self.assertEqual(kernel.open_process_calls, [51])
        self.assertEqual(kernel.close_handle_calls, [501, 501, 501])

    def test_close_keeps_parent_and_job_owned_until_retained_query_handle_closes(self):
        child = self.make_windows_child()
        child._query_handles = [{"handle": 501,
                                 "identity": {"pid": 52, "member": True, "creation_100ns": 456},
                                 "close_error": "previous synthetic close failure"}]
        kernel = self.FakeKernel(active=0)
        with mock.patch.object(child, "poll", return_value=0), mock.patch.object(
            child, "job_accounting", return_value={"active": 0, "cpu_100ns": 70, "compilers": []}
        ), mock.patch.object(probe, "_api", return_value=kernel):
            with self.assertRaisesRegex(RuntimeError, "query handle closure pending"):
                child.close()
            self.assertEqual(child._process, 101)
            self.assertEqual(child._job, 707)
            self.assertEqual(kernel.close_handle_calls, [501])
            self.assertEqual(len(child._query_handles), 1)

            kernel.allow_query_close = True
            child.close()

        self.assertTrue(child._closed)
        self.assertEqual(child._query_handles, [])
        self.assertEqual(kernel.close_handle_calls, [501, 501, 101, 707])

    @unittest.skipUnless(probe.os.name == "nt", "Windows process cleanup is Windows-only")
    def test_capture_saves_pending_query_identity_and_campaign_stops_after_cleanup(self):
        helper = PreparationAndReservationTests("test_prepare_uses_fresh_isolated_target_and_explicit_target_dir_argv")
        root, _, _, plan, _ = helper.prepare_fixture()
        self.addCleanup(helper.doCleanups)
        kernel = self.FakeKernel(active=1)
        launches, children, process_snapshots, pending_ownership = [], [], [], []

        def fake_launch(argv, cwd, folder, watchdog, env):
            launches.append(list(argv))

            def factory(_child_argv, _child_cwd, _stdout, _stderr, _child_env):
                child = self.make_windows_child()
                child.pid = 88
                child._process = 101
                child._parent_natural_exit = False

                def natural_parent_exit():
                    child._parent_natural_exit = True
                    return 0

                child.poll = natural_parent_exit
                child.wait = lambda: 0
                children.append(child)
                return child

            return probe.Capture(argv, cwd, folder, watchdog, env, factory=factory)

        real_save = probe.save

        def save_with_snapshots(path, value):
            real_save(path, value)
            if Path(path).name == "process.json":
                process_snapshots.append(copy.deepcopy(value))
                if value.get("samples") and value["samples"][-1].get("query_handles_pending") == 1:
                    child = children[0]
                    pending_ownership.append({"process": child._process, "job": child._job,
                                              "query_handles": copy.deepcopy(child._query_handles)})
                    # Let the next bounded sample prove zero, retry the same
                    # query handle, and finish naturally without more opens.
                    kernel.active = 0
                    kernel.allow_query_close = True

        with tempfile.TemporaryDirectory() as temp, mock.patch.object(
            probe, "require_runtime_platform"
        ), mock.patch.object(probe, "source_identity", return_value=copy.deepcopy(helper.SOURCE)), mock.patch.object(
            probe, "environment_identity", return_value=copy.deepcopy(helper.PARENT_ENV)
        ), mock.patch.object(probe, "toolchain_identity", return_value=copy.deepcopy(helper.TOOLCHAIN)), mock.patch.object(
            probe, "launch", side_effect=fake_launch
        ), mock.patch.object(probe, "_api", return_value=kernel), mock.patch.object(
            probe, "_job_pids", return_value=[88]
        ), mock.patch.object(probe, "save", side_effect=save_with_snapshots), mock.patch.object(
            probe.time, "sleep", return_value=None
        ):
            result = probe.campaign(plan, root, Path(temp) / "campaign", mode="idle-only")

        self.assertEqual(result["status"], "failed_stop")
        self.assertEqual(len(result["attempts"]), 1)
        self.assertEqual(len(launches), 1)
        self.assertIn("process_observation_error", result["failure"])
        self.assertTrue(children[0]._closed)
        pending_snapshots = [snapshot for snapshot in process_snapshots
                             if snapshot.get("handles_closed") is False and
                             snapshot.get("cleanup_complete") is False and snapshot["samples"] and
                             snapshot["samples"][-1].get("query_handles_pending") == 1]
        self.assertTrue(pending_snapshots)
        saved = pending_snapshots[0]
        self.assertEqual(saved["actual_exit"], 0)
        self.assertIn("CloseHandle", saved["failure"])
        self.assertEqual(saved["samples"][-1]["query_handle_identities"][0]["pid"], 88)
        self.assertIn("CloseHandle", saved["samples"][-1]["query_handle_identities"][0]["close_error"])
        final_process = result["attempts"][0]["process"]
        self.assertEqual(final_process["failure"], saved["failure"])
        self.assertTrue(final_process["handles_closed"])
        self.assertTrue(final_process["cleanup_complete"])
        self.assertEqual(len(pending_ownership), 1)
        self.assertEqual(pending_ownership[0]["process"], 101)
        self.assertEqual(pending_ownership[0]["job"], 707)
        self.assertEqual(len(pending_ownership[0]["query_handles"]), 1)
        self.assertEqual(pending_ownership[0]["query_handles"][0]["handle"], 501)
        self.assertEqual(kernel.open_process_calls, [88])
        self.assertEqual(kernel.close_handle_calls, [501, 501, 101, 707])


if __name__ == "__main__":
    unittest.main()
