"""Unit tests and separately selected real Rust-packer integration tests."""
import contextlib
import io
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import zlib

import create_sprite_package as tool


def png_bytes():
    def chunk(name, data):
        return struct.pack(">I", len(data)) + name + data + struct.pack(">I", zlib.crc32(name + data))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 8, 4, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress((b"\0" + b"\xff\x20\x40\xff" * 8) * 4)) + chunk(b"IEND", b""))


class Fixture(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="sprite rectangle tests ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.png = self.root / "input image.png"
        self.png.write_bytes(png_bytes())
        self.license = self.root / "license.txt"
        self.license.write_bytes(b"Test-generated pixels; CC0.\n")
        self.output = self.root / "new package"

    def args(self, packer):
        return ["--packer", str(packer), "--png", str(self.png), "--license", str(self.license),
                "--name", "hero", "--version", "1.0.0", "--out", str(self.output),
                "--atlas-size", "8", "4", "--region", "10", "1", "0", "2", "3",
                "--region", "20", "5", "1", "3", "2", "--clip", "idle", "loop", "10:200",
                "--clip", "attack", "once", "10:80,20:120,10:80"]


class UnitTests(Fixture):
    def setUp(self):
        super().setUp()
        self.packer = self.root / "trusted packer"
        self.packer.write_text("not executed by unit tests")
        self.packer.chmod(0o700)

    def test_exact_document_and_fixed_argv_cleanup(self):
        captured = {}
        def run(argv, **kwargs):
            captured["argv"] = argv
            captured["path"] = Path(argv[argv.index("--json") + 1])
            captured["doc"] = json.loads(captured["path"].read_bytes())
            self.assertEqual(kwargs, {"check": False})
            return subprocess.CompletedProcess(argv, 0)
        with patch.object(tool.subprocess, "run", side_effect=run):
            self.assertEqual(tool.main(self.args(self.packer)), 0)
        doc = captured["doc"]
        self.assertEqual(doc, {"format": "orr_sprite", "version": 1,
            "atlas": {"image": "atlas.png", "width": 8, "height": 4},
            "regions": [{"id": 10, "x": 1, "y": 0, "width": 2, "height": 3},
                        {"id": 20, "x": 5, "y": 1, "width": 3, "height": 2}],
            "clips": [{"id": "idle", "mode": "loop", "frames": [{"region": 10, "duration_ms": 200}]},
                      {"id": "attack", "mode": "once", "frames": [{"region": 10, "duration_ms": 80},
                       {"region": 20, "duration_ms": 120}, {"region": 10, "duration_ms": 80}]}]})
        self.assertEqual(captured["argv"], [str(self.packer), "--png", str(self.png), "--json",
            str(captured["path"]), "--license", str(self.license), "--name", "hero", "--version", "1.0.0", "--out", str(self.output)])
        self.assertFalse(captured["path"].parent.exists())
        self.assertFalse(self.output.exists())

    def test_failure_and_interruption_never_delete_destination(self):
        for result in (7, -9):
            with self.subTest(result=result):
                self.output.mkdir(exist_ok=True)
                marker = self.output / "partial"
                marker.write_text("preserve")
                with patch.object(tool.subprocess, "run", return_value=subprocess.CompletedProcess([], result)):
                    self.assertEqual(tool.main(self.args(self.packer)), result if result >= 0 else 1)
                self.assertEqual(marker.read_text(), "preserve")

    def test_launch_error_cleans_only_temporary_json(self):
        paths = []
        def fail(argv, **kwargs):
            paths.append(Path(argv[argv.index("--json") + 1]))
            raise OSError("cannot execute")
        with patch.object(tool.subprocess, "run", side_effect=fail), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(tool.main(self.args(self.packer)), 1)
        self.assertFalse(paths[0].parent.exists())
        self.assertFalse(self.output.exists())

    def test_syntax_and_budget_fail_before_invocation(self):
        variants = [self.args(self.packer)[:-1] + ["10:2:3"], self.args(self.packer)[:-1] + ["x:2"],
                    self.args(self.packer)[:-1] + ["1:" + "1" * 22],
                    self.args(self.packer) + ["--clip", "big", "loop", "1:1," * tool.MAX_FRAMES + "1:1"],
                    self.args(self.packer) + ["x" * tool.MAX_JSON_BYTES]]
        with patch.object(tool.subprocess, "run") as run, contextlib.redirect_stderr(io.StringIO()):
            for args in variants:
                self.assertEqual(tool.main(args), 1)
            run.assert_not_called()

    def test_collection_and_encoded_document_guards(self):
        for regions, clips in [([["1"] * 5] * (tool.MAX_REGIONS + 1), []),
                               ([], [["a", "loop", "1:1"]] * (tool.MAX_CLIPS + 1)),
                               ([], [["a", "loop", ",".join(["1:1"] * tool.MAX_FRAMES)]] * 5),
                               ([], [["x" * tool.MAX_JSON_BYTES, "loop", "1:1"]])]:
            with self.assertRaises(ValueError):
                tool.document_bytes(["8", "4"], regions, clips)

    def test_semantics_are_delegated_not_reimplemented(self):
        doc = json.loads(tool.document_bytes(["0", "4"], [["1", "0", "0", "0", "1"]],
                                             [["", "unknown", "999:0"]]))
        self.assertEqual(doc["clips"][0]["frames"][0]["duration_ms"], 0)

    def test_help_and_missing_binary_do_not_execute(self):
        with patch.object(tool.subprocess, "run") as run, contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as result:
                tool.main(["--help"])
            self.assertEqual(result.exception.code, 0)
            self.assertEqual(tool.main(self.args(self.root / "missing")), 1)
            run.assert_not_called()


@unittest.skipUnless(os.environ.get("ORR_RECTANGLE_PACKER"), "explicit trusted real packer not selected")
class RealPackerTests(Fixture):
    def run_cli(self, args):
        return subprocess.run([os.sys.executable, str(Path(tool.__file__).resolve()), *args],
                              capture_output=True, text=True, check=False)

    def args(self, packer=None):
        return super().args(os.environ["ORR_RECTANGLE_PACKER"])

    def test_package_preserves_sources_and_contract(self):
        result = self.run_cli(self.args())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual({p.name for p in self.output.iterdir()}, {"atlas.png", "sprites.json", "LICENSE.txt", "orr.package.json"})
        self.assertEqual((self.output / "atlas.png").read_bytes(), self.png.read_bytes())
        self.assertEqual((self.output / "LICENSE.txt").read_bytes(), self.license.read_bytes())
        self.assertEqual(json.loads((self.output / "sprites.json").read_bytes())["clips"][1]["frames"],
                         [{"region": 10, "duration_ms": 80}, {"region": 20, "duration_ms": 120}, {"region": 10, "duration_ms": 80}])
        self.assertEqual(json.loads((self.output / "orr.package.json").read_bytes())["capabilities"], ["sprite"])

    def test_rust_rejects_invalid_documents_without_output(self):
        base = self.args()
        cases = [base + ["--region", "10", "0", "0", "1", "1"],
                 base + ["--region", "30", "8", "0", "1", "1"],
                 base + ["--clip", "bad", "loop", "999:1"],
                 base + ["--clip", "bad", "loop", "10:0"],
                 base + ["--clip", "bad", "loop", "10:18446744073709551615,20:1"],
                 base + ["--clip", "idle", "loop", "10:1"],
                 base + ["--clip", "bad", "unknown", "10:1"]]
        mismatch = base.copy()
        mismatch[mismatch.index("--atlas-size") + 1] = "9"
        cases.append(mismatch)
        for args in cases:
            with self.subTest(args=args[-4:]):
                result = self.run_cli(args)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(self.output.exists(), result.stderr)

    def test_existing_directory_and_symlink_unchanged(self):
        existing = self.root / "existing"
        existing.mkdir()
        marker = existing / "sentinel"
        marker.write_bytes(b"keep")
        self.output.symlink_to(existing, target_is_directory=True)
        self.assertNotEqual(self.run_cli(self.args()).returncode, 0)
        self.assertTrue(self.output.is_symlink())
        self.output.unlink()
        self.output.mkdir()
        (self.output / "sentinel").write_bytes(b"keep")
        self.assertNotEqual(self.run_cli(self.args()).returncode, 0)
        self.assertEqual((self.output / "sentinel").read_bytes(), b"keep")
        self.assertEqual(marker.read_bytes(), b"keep")


if __name__ == "__main__":
    unittest.main()
