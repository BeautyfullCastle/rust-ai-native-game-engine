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
    def test_unsupported_inherited_build_override_is_rejected(self):
        with mock.patch.dict(probe.os.environ, {"CARGO_TARGET_X86_64_CUSTOM_LINKER": "synthetic"}, clear=True):
            with self.assertRaisesRegex(ValueError, "unsupported inherited build override"):
                probe.environment_identity()

    def validate(self, plan, *, current_source=None, current_toolchain=None, binary_digest="binary"):
        current_source = current_source if current_source is not None else plan["source"]
        with mock.patch.object(probe, "source_identity", return_value=current_source), mock.patch.object(
            probe, "digest", return_value=binary_digest
        ), mock.patch.object(probe, "environment_identity", return_value=plan.get("environment", {})), mock.patch.object(
            probe, "toolchain_identity", return_value=current_toolchain if current_toolchain is not None else plan.get("toolchain", {})
        ):
            probe.validate_plan(plan, Path("unused-root"))

    def test_valid_prebuilt_plan_is_accepted(self):
        self.validate(valid_plan())

    def test_repeat_count_zero_and_four_are_rejected(self):
        for repeats in (0, 4):
            with self.subTest(repeats=repeats):
                plan = valid_plan()
                plan["repeats"] = repeats
                with self.assertRaises(ValueError):
                    self.validate(plan)

    def test_boolean_repeat_count_is_rejected_even_though_bool_is_an_int_subclass(self):
        plan = valid_plan()
        plan["repeats"] = True
        with self.assertRaises(ValueError):
            self.validate(plan)

    def test_boolean_case_build_and_campaign_watchdogs_are_rejected(self):
        mutations = (
            lambda p: p["cases"][0].__setitem__("watchdog_seconds", True),
            lambda p: p.__setitem__("build_watchdog_seconds", True),
            lambda p: p.__setitem__("campaign_watchdog_seconds", True),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                plan = valid_plan()
                mutate(plan)
                with self.assertRaises(ValueError):
                    self.validate(plan)

    def test_duplicate_case_is_rejected(self):
        plan = valid_plan()
        plan["cases"].append(copy.deepcopy(plan["cases"][0]))
        with self.assertRaises(ValueError):
            self.validate(plan)

    def test_unapproved_c_client_case_is_rejected(self):
        plan = valid_plan()
        plan["cases"] = [{"binary": "c_client", "name": "c_program_sees_what_the_rust_bridge_sees", "watchdog_seconds": 90}]
        with self.assertRaises(ValueError):
            self.validate(plan)

    def test_source_revision_or_manifest_drift_is_rejected(self):
        plan = valid_plan()
        changed = dict(plan["source"], head="d" * 40)
        with self.assertRaises(ValueError):
            self.validate(plan, current_source=changed)

    def test_toolchain_drift_is_rejected(self):
        plan = valid_plan()
        plan["toolchain"] = {"rustc": "rustc A", "cargo": "cargo A"}
        changed = {"rustc": "rustc B", "cargo": "cargo A"}
        with self.assertRaises(ValueError):
            self.validate(plan, current_toolchain=changed)

    def test_prebuilt_binary_hash_drift_is_rejected(self):
        with self.assertRaises(ValueError):
            self.validate(valid_plan(), binary_digest="changed-binary")

    def test_selector_not_listed_by_prebuilt_binary_is_rejected(self):
        plan = valid_plan()
        plan["binaries"]["loopback"]["listed_tests"] = ["idle_timeout_detected::quic"]
        with self.assertRaises(ValueError):
            self.validate(plan)


class CaptureLifecycleTests(unittest.TestCase):
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
        plan = valid_plan()
        second_name = "two_clients_over_websocket"
        plan["binaries"]["net_e2e"] = {
            "path": "prebuilt-net-e2e.exe", "sha256": "binary", "listed_tests": [second_name]
        }
        plan["cases"].append({"binary": "net_e2e", "name": second_name, "watchdog_seconds": 90})

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
                probe, "validate_plan", return_value=None
            ), mock.patch.object(
                probe, "launch", side_effect=fake_launch
            ):
                result = probe.campaign(plan, Path("fake-root"), output, mode="idle-only")

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


if __name__ == "__main__":
    unittest.main()
