import contextlib
import hashlib
import io
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import create_coop_defense_scene as creator


class SceneCreation(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.wasm = self.root / "trusted.wasm"
        self.wasm.write_bytes(b"unit-test placeholder, not runtime evidence")
        self.output = self.root / "new.scene.yaml"

    def create(self, **kwargs):
        return creator.create_scene(self.wasm, self.output, **kwargs)

    def test_deterministic_defaults_and_boundaries(self):
        self.assertEqual(creator.scene_text(), creator.scene_text(150, 30, 10))
        for values in [(0, 1, 1), (10000, 1000, 10), (250, 75, 3)]:
            text = creator.scene_text(*values)
            self.assertIn(f"starting_gold: {values[0]}\n", text)
            self.assertIn(f"reactor_hp: {values[1]}\n", text)
            self.assertIn(f"last_wave: {values[2]}\n", text)
            self.assertNotIn("WavePacing", text)

    def test_invalid_values_before_engine_or_output(self):
        for name, values in [("starting_gold", [-1, 10001]),
                             ("reactor_hp", [0, 1001]), ("last_wave", [0, 11])]:
            for value in [*values, True, 1.5, "3", None]:
                with self.subTest(name=name, value=value), patch.object(creator, "validate_scene") as validate:
                    with self.assertRaises(ValueError):
                        self.create(**{name: value})
                    validate.assert_not_called()
                    self.assertFalse(self.output.exists())

    def test_validated_exact_bytes_and_receipt(self):
        receipt = {"test": "admission stub only"}
        with patch.object(creator, "validate_scene", return_value=receipt) as validate:
            self.assertEqual(self.create(starting_gold=250, reactor_hp=75, last_wave=3), receipt)
        validate.assert_called_once_with(self.output.read_bytes(), self.wasm.read_bytes())
        self.assertEqual(self.output.read_text(), creator.scene_text(250, 75, 3))

    def test_failed_admission_and_timeout_never_publish(self):
        for error in [ValueError("rejected"), subprocess.TimeoutExpired("node", 15),
                      subprocess.CalledProcessError(1, "node")]:
            with patch.object(creator, "validate_scene", side_effect=error):
                with self.assertRaises(type(error)):
                    self.create()
            self.assertFalse(self.output.exists())

    def test_existing_output_is_unchanged(self):
        self.output.write_bytes(b"keep")
        with patch.object(creator, "validate_scene", return_value={}):
            with self.assertRaises(FileExistsError):
                self.create()
        self.assertEqual(self.output.read_bytes(), b"keep")
        self.assertEqual(sorted(p.name for p in self.root.iterdir()), ["new.scene.yaml", "trusted.wasm"])

    def test_output_links_and_parent_links_rejected(self):
        self.output.symlink_to(self.root / "absent")
        with patch.object(creator, "validate_scene", return_value={}):
            with self.assertRaises(FileExistsError):
                self.create()
        self.assertTrue(self.output.is_symlink())
        self.assertFalse((self.root / "absent").exists())
        parent = self.root / "alias"
        parent.symlink_to(self.root, target_is_directory=True)
        with patch.object(creator, "validate_scene", return_value={}):
            with self.assertRaises(OSError):
                creator.create_scene(self.wasm, parent / "elsewhere.yaml")
        self.assertFalse((self.root / "elsewhere.yaml").exists())

    def test_wasm_link_directory_fifo_and_size_rejected(self):
        link = self.root / "link.wasm"
        link.symlink_to(self.wasm)
        fifo = self.root / "fifo.wasm"
        os.mkfifo(fifo)
        oversized = self.root / "large.wasm"
        with oversized.open("wb") as stream:
            stream.truncate(creator.MAX_ASSET + 1)
        for source in [link, fifo, self.root, oversized]:
            with self.subTest(source=source), patch.object(creator, "validate_scene") as validate:
                with self.assertRaises((OSError, ValueError)):
                    creator.create_scene(source, self.output)
                validate.assert_not_called()
                self.assertFalse(self.output.exists())

    def test_cli_argument_errors(self):
        for value in ["1.5", "no", "", "1\nlast_wave: 10"]:
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as caught:
                creator.main(["--wasm", str(self.wasm), "--output", str(self.output), "--last-wave", value])
            self.assertEqual(caught.exception.code, 2)
            self.assertFalse(self.output.exists())

    def test_cli_validation_error_exit(self):
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(creator.main(["--wasm", str(self.wasm), "--output", str(self.output), "--reactor-hp", "0"]), 1)
        self.assertFalse(self.output.exists())


@unittest.skipUnless(os.environ.get("ORR_DEFENSE_WASM"), "set trusted ORR_DEFENSE_WASM for actual engine tests")
class ActualEngine(unittest.TestCase):
    def test_saved_scene_consumption_and_restart(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for values in [(150, 30, 10), (0, 1, 1), (10000, 1000, 10), (250, 75, 3)]:
                output = root / ("-".join(map(str, values)) + ".yaml")
                receipt = creator.create_scene(os.environ["ORR_DEFENSE_WASM"], output, *values)
                self.assertEqual(receipt["sceneSha256"], hashlib.sha256(output.read_bytes()).hexdigest())
                self.assertEqual(receipt["sceneBytes"], output.stat().st_size)
                script = """
const fs=require('fs'),assert=require('assert/strict');
(async()=>{const w=(await WebAssembly.instantiate(fs.readFileSync(process.argv[1]),{})).instance.exports;
const scene=fs.readFileSync(process.argv[2]);
new Uint8Array(w.memory.buffer,w.defense_scene_buffer(),scene.length).set(scene);
assert.equal(w.defense_init_scene(scene.length),0);
const read=()=>{let p=w.defense_snapshot();return JSON.parse(new TextDecoder().decode(new Uint8Array(w.memory.buffer,p,w.defense_snapshot_len())))};
const [gold,hp,waves]=process.argv.slice(3).map(Number);
for(let i=0;i<2;i++){let s=read();assert.equal(s.tick,0);assert.deepEqual(s.gold,[gold,gold]);assert.equal(s.hp,hp);assert.equal(s.maxHp,hp);assert.equal(s.lastWave,waves);w.defense_step(1,0,0,1);w.defense_step(5,0,0,2);}
})().catch(e=>{console.error(e);process.exitCode=1});
"""
                subprocess.run(["node", "-e", script, os.environ["ORR_DEFENSE_WASM"], str(output), *map(str, values)], check=True, timeout=15)

    def test_malformed_wasm_no_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            wasm = root / "bad.wasm"
            wasm.write_bytes(b"not WASM")
            with self.assertRaises(subprocess.CalledProcessError):
                creator.create_scene(wasm, root / "scene.yaml")
            self.assertFalse((root / "scene.yaml").exists())


if __name__ == "__main__":
    unittest.main()
