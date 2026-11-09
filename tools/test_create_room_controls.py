"""Unit tests; explicit real-binary acceptance via run_integration(binary_dir)."""
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import create_room_controls as creator

ROOT = Path(__file__).resolve().parents[1]
BINDINGS = dict(zip(creator.ACTIONS, ("j,arrow_left", "l,arrow_right", "i,arrow_up", "k,arrow_down", "o")))


def command(output, bindings=None):
    args = ["--output", str(output)]
    for name, value in (bindings or BINDINGS).items():
        args += ["--" + name.replace("_", "-"), value]
    return args


class ControlsTests(unittest.TestCase):
    def test_rust_contract_inventory(self):
        source = (ROOT / "crates/orr_input/src/lib.rs").read_text()
        enum = re.search(r"pub enum Key \{(.*?)\n\}", source, re.S)[1]
        keys = {re.sub(r"(?<!^)(?=[A-Z])", "_", k).lower()
                for k in re.findall(r"^    (\w+),$", enum, re.M)}
        self.assertEqual(keys, creator.KEYS)
        self.assertIn("pub const MAX_BINDINGS: usize = 8;", source)
        self.assertIn("pub const MAX_FILE_BYTES: usize = 65536;", source)
        room = (ROOT / "crates/orr_sample/src/room_controls.rs").read_text()
        names = re.search(r"const REQUIRED:.*?= \[(.*?)\];", room, re.S)[1]
        self.assertEqual(tuple(re.findall(r'"(\w+)"', names)), creator.ACTIONS)
        self.assertIn("Key::R | Key::Escape", room)

    def test_deterministic_alias_order_and_exact_schema(self):
        original = creator.encode(BINDINGS)
        reversed_aliases = {k: ",".join(reversed(v.split(","))) for k, v in reversed(list(BINDINGS.items()))}
        self.assertEqual(original, creator.encode(reversed_aliases))
        data = json.loads(original)
        self.assertEqual(set(data), {"version", "actions"})
        self.assertEqual(data["version"], 1)
        self.assertEqual([a["name"] for a in data["actions"]], list(creator.ACTIONS))
        for action in data["actions"]:
            self.assertEqual(set(action), {"name", "bindings"})
            for binding in action["bindings"]:
                self.assertEqual(binding["device"], "Keyboard")
                self.assertEqual(set(binding), {"device", "key"})

    def test_invalid_bindings(self):
        for bad in ("", "J", "KeyJ", "j,", " j", "j,j", "o", "r", "escape", "mouse_left",
                    "digit1", "é", "j\n", "j\x00", "a,b,c,d,e,f,g,h,i", "a" * 129, None):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                creator.encode({**BINDINGS, "move_left": bad})
        for change in ({}, {**BINDINGS, "jump": "q"}, {k:v for k,v in BINDINGS.items() if k != "interact"}):
            with self.assertRaises(ValueError):
                creator.encode(change)

    def test_all_supported_nonreserved_keys(self):
        for key in creator.KEYS - creator.RESERVED:
            others = sorted(creator.KEYS - creator.RESERVED - {key})[:4]
            creator.encode(dict(zip(creator.ACTIONS, [key] + others)))

    def test_cli_and_no_extra_changes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            marker = root / "orr.project.json"
            marker.write_bytes(b"unchanged")
            output = root / "room.controls.json"
            result = subprocess.run(["python3", str(ROOT / "tools/create_room_controls.py"), *command(output)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn('entry.controls to "room.controls.json"', result.stdout)
            self.assertIn("room-input-actions", result.stdout)
            self.assertEqual(marker.read_bytes(), b"unchanged")
            self.assertEqual({p.name for p in root.iterdir()}, {marker.name, output.name})
            self.assertEqual(output.read_bytes(), creator.encode(BINDINGS))

    def test_cli_errors_write_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "controls.json"
            valid = command(output)
            for args in (valid[:-2], valid + ["--interact", "p"], valid + ["--wat"],
                         valid[:-1] + ["r"], valid[:-1] + ["a" * 4097]):
                result = subprocess.run(["python3", str(ROOT / "tools/create_room_controls.py"), *args], capture_output=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(list(Path(tmp).iterdir()), [])

    def test_existing_files_directories_and_symlinks_preserved(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            target = root / "target.json"
            target.write_bytes(b"original")
            dangling = root / "dangling.json"
            dangling.symlink_to(root / "missing")
            link = root / "link.json"
            link.symlink_to(target)
            directory = root / "directory.json"
            directory.mkdir()
            for path in (target, dangling, link, directory):
                with self.assertRaises(FileExistsError):
                    creator.publish_new(str(path), creator.encode(BINDINGS))
            self.assertEqual(target.read_bytes(), b"original")
            self.assertTrue(dangling.is_symlink())
            self.assertTrue(link.is_symlink())
            self.assertEqual(len(list(root.iterdir())), 4)

    def test_parent_symlink_and_invalid_destination(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "link").symlink_to(root, target_is_directory=True)
            for path in (root / "link/out.json", root / "missing/out.json", root / "bad\n.json", root / "out.txt",
                         root / "ORR.PROJECT.json", root / "orr.packages.lock.json"):
                with self.assertRaises((OSError, ValueError)):
                    creator.publish_new(str(path), creator.encode(BINDINGS))
            self.assertEqual([p.name for p in root.iterdir()], ["link"])

    def test_sync_and_link_failure_cleanup(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "controls.json"
            for operation in ("fsync", "link"):
                with patch.object(creator.os, operation, side_effect=OSError("injected failure")):
                    with self.assertRaises(OSError):
                        creator.publish_new(str(output), creator.encode(BINDINGS))
                self.assertEqual(list(Path(tmp).iterdir()), [])

    def test_write_failure_preserves_directory(self):
        real_fdopen = os.fdopen
        @contextmanager
        def broken_writer(fd, mode):
            with real_fdopen(fd, mode) as stream:
                with patch.object(stream, "write", side_effect=OSError("write fault")):
                    yield stream
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "controls.json"
            with patch.object(creator.os, "fdopen", side_effect=broken_writer):
                with self.assertRaisesRegex(OSError, "write fault"):
                    creator.publish_new(str(output), creator.encode(BINDINGS))
            self.assertEqual(list(Path(tmp).iterdir()), [])

    def test_bounds(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "controls.json"
            for data in (b"", b"x" * 65537, "not bytes"):
                with self.assertRaises(ValueError):
                    creator.publish_new(str(output), data)
            for path in ("", "a" * 4097, str(Path(tmp) / ("a" * 124 + ".json"))):
                with self.assertRaises(ValueError):
                    creator.publish_new(path, creator.encode(BINDINGS))
            self.assertEqual(list(Path(tmp).iterdir()), [])

    def test_cleanup_failure_reports_publication_state(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "controls.json"
            with patch.object(creator.os, "unlink", side_effect=OSError("cleanup fault")):
                with self.assertRaisesRegex(OSError, "output was published"):
                    creator.publish_new(str(output), creator.encode(BINDINGS))
            self.assertEqual(output.read_bytes(), creator.encode(BINDINGS))
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "controls.json"
            with patch.object(creator.os, "link", side_effect=OSError("link fault")), \
                 patch.object(creator.os, "unlink", side_effect=OSError("cleanup fault")):
                with self.assertRaisesRegex(OSError, "link fault; staging cleanup also failed: cleanup fault"):
                    creator.publish_new(str(output), creator.encode(BINDINGS))
            self.assertFalse(output.exists())

    def test_racing_destination_not_replaced(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "controls.json"
            real_link = os.link
            def race(source, destination):
                output.write_bytes(b"racing author")
                real_link(source, destination)
            with patch.object(creator.os, "link", side_effect=race):
                with self.assertRaises(FileExistsError):
                    creator.publish_new(str(output), creator.encode(BINDINGS))
            self.assertEqual(output.read_bytes(), b"racing author")
            self.assertEqual([p.name for p in Path(tmp).iterdir()], ["controls.json"])


def run_integration(binary_dir):
    """Explicit trusted binaries only. CPU admission/export/reopen, not physical input."""
    binaries = Path(binary_dir).resolve()
    def run(*args):
        result = subprocess.run(list(map(str, args)), capture_output=True, text=True, timeout=90)
        if result.returncode:
            raise AssertionError(f"{args}: {result.returncode}\n{result.stdout}\n{result.stderr}")
        print(result.stdout, end="")
        return result.stdout
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        project = root / "project"
        run(binaries / "orr_new_arena", "--output", project, "--template", "room-escape-3d-v1", "--seed", "room-controls-authoring")
        output = project / "room.controls.json"
        run("python3", ROOT / "tools/create_room_controls.py", *command(output))
        manifest_path = project / "orr.project.json"
        manifest = json.loads(manifest_path.read_bytes())
        manifest["entry"]["controls"] = output.name
        manifest_path.write_text(json.dumps(manifest) + "\n")
        runtime = binaries / "room_escape"
        before = run(runtime, "--project", project, "--headless", "--ticks", "42", "--hold", "left,right,up,down,interact", "--check-model-poses")
        exported = root / "exported"
        digest = hashlib.sha256(runtime.read_bytes()).hexdigest()
        run(binaries / "orr_export_room", "--project", project, "--runtime", runtime,
            "--runtime-sha256", digest, "--output", exported, "--trusted-runtime")
        exported_controls = exported / "project/room.controls.json"
        assert exported_controls.read_bytes() == output.read_bytes()
        assert json.loads((exported / "project/orr.project.json").read_bytes())["entry"]["controls"] == output.name
        after = run(exported / "run-room-escape", "--headless", "--ticks", "42", "--hold", "left,right,up,down,interact", "--check-model-poses")
        assert before == after, (before, after)
        print("PASS: authored sidecar admitted, exported byte-exactly, reopened with CPU model-pose checks.")
        print("Runtime SHA256:", digest)
        print("Controls SHA256:", hashlib.sha256(output.read_bytes()).hexdigest())


if __name__ == "__main__":
    unittest.main()
