import os
from pathlib import Path
import tempfile
import signal
import subprocess
import sys
import time
import unittest
from unittest.mock import patch
from types import SimpleNamespace

import check_linux_compact as compact


class CompactTests(unittest.TestCase):
    def test_environment_is_scoped_and_preserves_security(self):
        original = {"HTTPS_PROXY": "proxy", "RUSTFLAGS": "--cfg example",
                    "CARGO_PROFILE_RELEASE_DEBUG": "2", "CARGO_INCREMENTAL": "1"}
        result = compact.compact_environment(original, Path("/tmp/target"))
        self.assertEqual(original["CARGO_INCREMENTAL"], "1")
        self.assertEqual(result["CARGO_INCREMENTAL"], "0")
        self.assertEqual(result["CARGO_PROFILE_DEV_DEBUG"], "1")
        self.assertEqual(result["CARGO_PROFILE_TEST_DEBUG"], "1")
        for key in ("HTTPS_PROXY", "RUSTFLAGS", "CARGO_PROFILE_RELEASE_DEBUG"):
            self.assertEqual(result[key], original[key])

    def test_reject_relative_and_low_reserve(self):
        for target, floor in (("relative", 8), ("/tmp/target", 0), ("/tmp/target", -1)):
            with self.assertRaises(ValueError):
                compact.preflight(target, floor)

    def test_low_space_does_not_create_target_or_start_build(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "tools").mkdir()
            (root / "tools/check-linux-local.sh").write_text("exit 0\n")
            target = root / "new/target"
            with patch.object(compact.shutil, "disk_usage", return_value=SimpleNamespace(free=1)), patch.object(compact.subprocess, "run") as child:
                with self.assertRaises(ValueError):
                    compact.run(target, 1, root)
                child.assert_not_called()
            self.assertFalse(target.exists())

    def test_missing_gate_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(ValueError):
                compact.run(Path(directory) / "target", 1, Path(directory))

    def test_non_linux_refused(self):
        with patch.object(compact.sys, "platform", "darwin"):
            with self.assertRaisesRegex(ValueError, "requires Linux"):
                compact.run(Path("/tmp/unused"), 8, Path("/tmp/unused"))

    def test_second_space_check_refuses_depleted_disk(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "tools").mkdir()
            (root / "tools/check-linux-local.sh").write_text("exit 0\n")
            with patch.object(compact.shutil, "disk_usage", side_effect=[SimpleNamespace(free=16 * compact.GIB), SimpleNamespace(free=1)]), patch.object(compact.subprocess, "run") as child:
                with self.assertRaisesRegex(ValueError, "GiB required"):
                    compact.run(root / "target", 8, root)
                child.assert_not_called()

    def test_lock_survives_terminated_launcher_while_gate_runs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "tools").mkdir()
            (root / "tools/check-linux-local.sh").write_text(
                'echo ready > ready\nsleep 30\n')
            code = ('import sys; from pathlib import Path; from types import SimpleNamespace; '
                    f'sys.path.insert(0, {str(Path(compact.__file__).parent)!r}); '
                    'import check_linux_compact as c; '
                    'c.shutil.disk_usage=lambda p: SimpleNamespace(free=16*c.GIB); '
                    f'root=Path({str(root)!r}); c.run(root/"target",8,root)')
            process = subprocess.Popen([sys.executable, "-B", "-c", code],
                                       start_new_session=True, stdout=subprocess.DEVNULL)
            try:
                deadline = time.monotonic() + 5
                while not (root / "ready").exists() and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertTrue((root / "ready").exists())
                process.terminate()
                process.wait(timeout=5)
                with (root / "target/.orr-compact.lock").open("a") as lock:
                    with self.assertRaises(BlockingIOError):
                        compact.fcntl.flock(lock, compact.fcntl.LOCK_EX | compact.fcntl.LOCK_NB)
            finally:
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                process.wait(timeout=5)

    def test_baseline_command_exit_and_target_reuse(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "tools").mkdir()
            gate = root / "tools/check-linux-local.sh"
            gate.write_text("exit 0\n")
            target = root / "target"
            with patch.object(compact.shutil, "disk_usage", return_value=SimpleNamespace(free=16 * compact.GIB)), patch.object(compact.subprocess, "run") as child:
                for code, expected in ((0, 0), (7, 7), (-15, 143)):
                    child.return_value = SimpleNamespace(returncode=code)
                    self.assertEqual(compact.run(target, 8, root), expected)
                    args, kwargs = child.call_args
                    self.assertEqual(args, (["bash", str(gate)],))
                    self.assertEqual(kwargs["cwd"], root)
                    self.assertEqual(kwargs["env"]["CARGO_TARGET_DIR"], str(target))
            self.assertEqual(sorted(p.name for p in target.iterdir()), [".orr-compact.lock"])

    def test_target_is_file(self):
        with tempfile.NamedTemporaryFile() as file:
            with self.assertRaises(ValueError):
                compact.preflight(file.name, 1)

    def test_concurrent_launcher_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "tools").mkdir()
            (root / "tools/check-linux-local.sh").write_text("exit 0\n")
            target = root / "target"
            target.mkdir()
            with (target / ".orr-compact.lock").open("a") as lock:
                compact.fcntl.flock(lock, compact.fcntl.LOCK_EX)
                with patch.object(compact.shutil, "disk_usage", return_value=SimpleNamespace(free=16 * compact.GIB)), patch.object(compact.subprocess, "run") as child:
                    with self.assertRaisesRegex(ValueError, "another compact"):
                        compact.run(target, 8, root)
                    child.assert_not_called()

    def test_actual_child_sees_compact_environment_and_failure_survives(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "tools").mkdir()
            (root / "tools/check-linux-local.sh").write_text(
                'test "$CARGO_INCREMENTAL" = 0 || exit 90\n'
                'test "$CARGO_PROFILE_DEV_DEBUG" = 1 || exit 91\n'
                'test "$CARGO_PROFILE_TEST_DEBUG" = 1 || exit 92\n'
                'test "$CARGO_TARGET_DIR" = "$PWD/target" || exit 93\n'
                'exit 23\n')
            with patch.object(compact.shutil, "disk_usage", return_value=SimpleNamespace(free=16 * compact.GIB)):
                self.assertEqual(compact.run(root / "target", 8, root), 23)


if __name__ == "__main__":
    unittest.main()
