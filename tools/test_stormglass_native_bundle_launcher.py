#!/usr/bin/env python3
"""Launcher contract tests using a trusted shell fixture, not ELF/game acceptance."""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
TEMPLATE = ROOT / "crates/orr_package/src/coop_web_authoring/native_bundle/launch.sh.in"


class LauncherTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="orr-native-launcher-")
        self.addCleanup(self.tmp.cleanup)
        self.home = Path(self.tmp.name)
        self.bundle = self.home / "bundle space 한글 ; & $(false)"
        self.bundle.mkdir(mode=0o700)
        self.cwd = self.home / "unrelated working directory"
        self.cwd.mkdir()
        self.runtime = self.bundle / "coop_defense"
        self.scene = self.bundle / "game.scene.yaml"
        self.launcher = self.bundle / "launch.sh"
        self.marker = self.bundle / "fixture-invoked"
        self.runtime.write_text(
            '#!/bin/sh\nset -eu\nprintf "%s\\0" "$@" > "${0%/*}/fixture-invoked"\n',
            encoding="utf-8",
        )
        self.runtime.chmod(0o755)
        self.scene.write_text("trusted fixture scene\n", encoding="utf-8")
        text = TEMPLATE.read_text(encoding="utf-8")
        for token, path in (("@RUNTIME_SHA256@", self.runtime),
                            ("@SCENE_SHA256@", self.scene)):
            self.assertIn(token, text)
            text = text.replace(token, hashlib.sha256(path.read_bytes()).hexdigest())
        self.assertNotIn("@RUNTIME_SHA256@", text)
        self.assertNotIn("@SCENE_SHA256@", text)
        self.launcher.write_text(text, encoding="utf-8")
        self.launcher.chmod(0o755)

    def invoke(self, *args):
        return subprocess.run(
            [str(self.launcher), *args], cwd=self.cwd,
            env={"PATH": "", "LANG": "C"},
            capture_output=True, text=True, timeout=5, check=False,
        )

    def admitted(self, *args):
        result = self.invoke(*args)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.marker.exists())
        forwarded = self.marker.read_bytes().decode("utf-8").split("\0")
        self.assertEqual(forwarded.pop(), "")
        self.assertEqual(forwarded[:2], ["--scene", str(self.scene)])
        return forwarded[2:]

    def rejected(self, *args):
        self.marker.unlink(missing_ok=True)
        result = self.invoke(*args)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(self.marker.exists(), "fixture ran despite refused invocation")

    def test_relocated_interactive_launch_with_empty_path(self):
        self.assertEqual(self.admitted(), [])

    def test_headless_forwarding(self):
        self.assertIn("--headless", self.admitted("--headless"))
        for ticks in ("0", "1", "36000"):
            with self.subTest(ticks=ticks):
                self.assertEqual(self.admitted("--headless", "--ticks", ticks),
                                 ["--headless", "--ticks", ticks])

    def test_explicit_external_run_directory_forwarding(self):
        directory = self.home / "private saves 한글 ; &"
        directory.mkdir(mode=0o700)
        self.assertEqual(self.admitted("--run-directory", str(directory)),
                         ["--run-directory", str(directory)])

    def test_bundle_local_save_directories_never_execute(self):
        nested = self.bundle / "private saves"
        nested.mkdir(mode=0o700)
        for directory in (self.bundle, nested):
            with self.subTest(directory=directory):
                self.rejected("--run-directory", str(directory))

    def test_save_symlink_into_bundle_never_executes(self):
        nested = self.bundle / "private saves"
        nested.mkdir(mode=0o700)
        for target in (self.bundle, nested):
            with self.subTest(target=target):
                alias = self.home / "external-looking save alias"
                alias.symlink_to(target, target_is_directory=True)
                self.rejected("--run-directory", str(alias))
                alias.unlink()

    def test_newline_ending_bundle_path_is_preserved(self):
        moved = self.home / "bundle with trailing newline\n\n"
        self.bundle.rename(moved)
        self.bundle = moved
        self.runtime = moved / "coop_defense"
        self.scene = moved / "game.scene.yaml"
        self.launcher = moved / "launch.sh"
        self.marker = moved / "fixture-invoked"
        self.assertEqual(self.admitted("--headless", "--ticks", "1"),
                         ["--headless", "--ticks", "1"])
        nested = moved / "private saves\n"
        nested.mkdir(mode=0o700)
        self.rejected("--run-directory", str(nested))
        alias = self.home / "outside newline-bundle alias"
        alias.symlink_to(nested, target_is_directory=True)
        self.rejected("--run-directory", str(alias))

    def test_newline_save_path_is_preserved_and_bundle_alias_refused(self):
        external = self.home / "external saves\n\n"
        external.mkdir(mode=0o700)
        self.assertEqual(self.admitted("--run-directory", str(external)),
                         ["--run-directory", str(external)])
        nested = self.bundle / "private saves\n\n"
        nested.mkdir(mode=0o700)
        alias = self.home / "outside alias\n"
        alias.symlink_to(nested, target_is_directory=True)
        self.rejected("--run-directory", str(alias))

    def test_bad_flags_never_execute(self):
        cases = [
            ("--unknown",), ("--",), ("--scene", str(self.scene)),
            ("--capture", "/tmp/unrequested.png"), ("--audio", "off"),
            ("--audio-bank", "/tmp/bank"), ("--demo",),
            ("--headless", "--headless"), ("--ticks",),
            ("--ticks", "0"), ("--run-directory",),
            ("--run-directory", "relative"), ("--run-directory", ""),
            ("--run-directory", str(self.home), "--run-directory", str(self.home)),
            ("--headless", "--run-directory", str(self.home)),
            ("--run-directory", str(self.home), "--headless"),
            ("--headless", "--ticks", "1", "--ticks", "2"),
        ]
        cases.extend(("--headless", "--ticks", value)
                     for value in ("", "-1", "+1", "36001", "1.5", "1e2", "1 2", "abc",
                                   "999999999999999999999999999999999999999"))
        for args in cases:
            with self.subTest(args=args):
                self.rejected(*args)

    def test_tampered_files_never_execute(self):
        for path in (self.runtime, self.scene):
            with self.subTest(path=path.name):
                original = path.read_bytes()
                path.write_bytes(original + b"# changed\n")
                self.rejected()
                path.write_bytes(original)

    def test_symlink_files_never_execute_even_with_matching_hash(self):
        for path in (self.runtime, self.scene):
            with self.subTest(path=path.name):
                target = self.home / (path.name + ".original")
                path.rename(target)
                path.symlink_to(target)
                self.rejected()
                path.unlink()
                target.rename(path)

    def test_missing_files_never_execute(self):
        for path in (self.runtime, self.scene):
            with self.subTest(path=path.name):
                target = self.home / (path.name + ".held")
                path.rename(target)
                self.rejected()
                target.rename(path)

    def test_nonexecutable_runtime_never_executes(self):
        self.runtime.chmod(0o644)
        self.rejected()


if __name__ == "__main__":
    unittest.main()
