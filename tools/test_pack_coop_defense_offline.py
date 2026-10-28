#!/usr/bin/env python3
"""Portable regressions for the optional web package's exact module closure."""
import base64
import json
import os
import subprocess
from unittest import mock
import importlib.util
from pathlib import Path
import shutil
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("packer", Path(__file__).with_name("pack_coop_defense_offline.py"))
packer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(packer)
WEB = Path(__file__).resolve().parents[1] / "crates/orr_coop_defense_web/web"


class OfflineModules(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        for name in (*packer.MODULES, "game.js", "index.html", "style.css"):
            shutil.copyfile(WEB / name, self.root / name)
        (self.root / "game.wasm").write_bytes(b"test fixture, not executable WASM")
        (self.root / "assets").mkdir()
        for name, data in {"characters.json": b'{"heroes": [], "enemies": [], "label": "</script>"}', "units.png": b"heroes", "enemies.png": b"enemies"}.items():
            (self.root / "assets" / name).write_bytes(data)

    def test_exact_reviewed_modules_are_embedded_once(self):
        module = packer.bundle_game(self.root)
        for name in packer.MODULES:
            self.assertEqual(module.count((self.root / name).read_text()), 1)
        self.assertNotIn(packer.GAME_IMPORTS, module)
        html = packer.package(self.root).read_text()
        self.assertIn(module, html)
        self.assertNotIn('<script type="module" src=', html)
        self.assertIn('\\u003c/script>', html)

    def test_missing_dependency_fails(self):
        (self.root / packer.MODULES[0]).unlink()
        with self.assertRaises(FileNotFoundError): packer.package(self.root)
        self.assertFalse((self.root / "stormglass-foundry-offline.html").exists())

    def test_unknown_static_dynamic_and_reexport_dependencies_fail(self):
        original = (self.root / packer.MODULES[0]).read_text()
        for extra in ["import { x } from './missing.mjs';", "await import('./missing.mjs');", "export * from './missing.mjs';",
                      "import/* comment */ './missing.mjs';",
                      "export/* comment */ { x } from './missing.mjs';",
                      "await import/* comment */('./missing.mjs');",
                      "export/**/ * /**/ from './missing.mjs';",
                      "import\n// comment\n'./missing.mjs';"]:
            with self.subTest(extra=extra):
                (self.root / packer.MODULES[0]).write_text(original + '\n' + extra)
                with self.assertRaisesRegex(ValueError, "dependency"): packer.package(self.root)

    def test_changed_entry_graph_fails(self):
        (self.root / "game.js").write_text("import './extra.mjs';\n" + (self.root / "game.js").read_text())
        with self.assertRaisesRegex(ValueError, "import graph"): packer.package(self.root)

    def test_duplicate_anchor_fails(self):
        p = self.root / "index.html"; p.write_text(p.read_text() + '<script type="module" src="game.js"></script>')
        with self.assertRaisesRegex(ValueError, "one stylesheet"): packer.package(self.root)

    def test_script_and_style_breakouts_fail(self):
        p = self.root / "game.js"; original = p.read_text(); p.write_text(original + "\n// </script>")
        with self.assertRaisesRegex(ValueError, "terminate"): packer.package(self.root)
        p.write_text(original)
        p = self.root / "style.css"; p.write_text(p.read_text() + "/* </style> */")
        with self.assertRaisesRegex(ValueError, "terminate"): packer.package(self.root)

    def authored(self):
        scene = self.root / 'saved.scene.yaml'
        scene.write_text('schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::Scenario: {starting_gold: 250, reactor_hp: 75}}\n', encoding='utf-8')
        return scene, self.root / 'delivered.html'

    def test_authored_requires_new_explicit_destination(self):
        scene, out = self.authored()
        with self.assertRaises(ValueError): packer.package(self.root, scene)
        for destination in [scene, self.root / 'game.wasm', self.root / 'assets']:
            with self.assertRaises(FileExistsError): packer.package(self.root, scene, destination)
        out.symlink_to(scene)
        with self.assertRaises(FileExistsError): packer.package(self.root, scene, out)

    def test_nonregular_symlink_and_bounded_inputs(self):
        scene, out = self.authored()
        for data in [b'x' * 65537, b'\xff']:
            scene.write_bytes(data)
            with self.assertRaises((ValueError, UnicodeDecodeError)): packer.package(self.root, scene, out)
            self.assertFalse(out.exists())
        scene.unlink(); scene.symlink_to(self.root / 'index.html')
        with self.assertRaises(OSError): packer.package(self.root, scene, out)
        scene.unlink(); os.mkfifo(scene)
        with self.assertRaises(ValueError): packer.package(self.root, scene, out)
        scene.unlink(); scene.write_bytes(b'valid')
        asset = self.root / 'assets/units.png'; asset.unlink(); asset.symlink_to(self.root / 'game.wasm')
        with self.assertRaises(OSError): packer.package(self.root, scene, out)

    def test_directory_symlink_and_midread_mutation_rejected(self):
        scene, out = self.authored()
        linked = self.root / 'linked'; linked.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(OSError): packer.package(linked, scene, out)
        with self.assertRaises(OSError): packer.publish_new(linked / 'new.html', 'content')
        before = scene.stat()
        changed = mock.Mock(st_size=before.st_size, st_mtime_ns=before.st_mtime_ns + 1, st_ctime_ns=before.st_ctime_ns)
        with mock.patch.object(packer.os, 'fstat', side_effect=[before, changed]):
            with self.assertRaisesRegex(ValueError, 'changed'): packer.snapshot_file(scene, 65536)

    def test_snapshotted_bytes_not_reread_after_validation(self):
        scene, out = self.authored(); original = scene.read_bytes()
        def validate(data, wasm):
            self.assertEqual(data, original)
            scene.write_bytes(b'changed after validation')
            return {'version': 1, 'initialChecksum': '0'*16}
        with mock.patch.object(packer, 'validate_scene', side_effect=validate): packer.package(self.root, scene, out)
        self.assertIn(base64.b64encode(original).decode(), out.read_text())
        self.assertNotIn(base64.b64encode(scene.read_bytes()).decode(), out.read_text())

    def test_validator_failures_leave_no_output(self):
        scene, out = self.authored()
        for failure in [FileNotFoundError('node missing'), subprocess.TimeoutExpired('node',15), subprocess.CalledProcessError(1,'node')]:
            with mock.patch.object(packer.subprocess, 'run', side_effect=failure):
                with self.assertRaises(type(failure)): packer.package(self.root, scene, out)
            self.assertFalse(out.exists())

    def test_publication_race_and_failure_preserve_destination(self):
        scene, out = self.authored()
        def validate(*_):
            out.write_bytes(b'other writer')
            return {'version': 1}
        with mock.patch.object(packer, 'validate_scene', side_effect=validate):
            with self.assertRaises(FileExistsError): packer.package(self.root, scene, out)
        self.assertEqual(out.read_bytes(), b'other writer')
        self.assertEqual(list(self.root.glob('.orr-offline-*')), [])
        out.unlink()
        with mock.patch.object(packer, 'validate_scene', return_value={'version':1}), mock.patch.object(packer.os, 'link', side_effect=OSError('publication failed')):
            with self.assertRaises(OSError): packer.package(self.root, scene, out)
        self.assertFalse(out.exists()); self.assertEqual(list(self.root.glob('.orr-offline-*')), [])

    @unittest.skipUnless(os.environ.get('ORR_DEFENSE_WASM'), 'actual WASM not provided')
    def test_actual_wasm_scene_admission_and_receipt(self):
        scene, out = self.authored()
        shutil.copyfile(os.environ['ORR_DEFENSE_WASM'], self.root / 'game.wasm')
        scene.write_text(scene.read_text() + '# Unicode 한글 </script>\n', encoding='utf-8')
        exact = scene.read_bytes() + b'#' + b'x' * (65536 - len(scene.read_bytes()) - 1)
        self.assertEqual(packer.validate_scene(exact, (self.root/'game.wasm').read_bytes())['sceneBytes'], 65536)
        receipt = packer.validate_scene(scene.read_bytes(), (self.root / 'game.wasm').read_bytes())
        self.assertEqual(receipt['sceneBytes'], len(scene.read_bytes()))
        packer.package(self.root, scene, out)
        self.assertIn(receipt['initialChecksum'], out.read_text())
        self.assertNotIn('Unicode 한글', out.read_text())
        # Python accepts nonzero base64 pad bits; runtime intentionally rejects them.
        payload = scene.read_bytes()
        while len(payload) % 3 == 0:
            payload += b' '
        scene.write_bytes(payload)
        canonical = base64.b64encode(payload).decode('ascii')
        alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'
        last = len(canonical.rstrip('=')) - 1
        corrupted = canonical[:last] + alphabet[alphabet.index(canonical[last]) | 1] + canonical[last+1:]
        self.assertEqual(base64.b64decode(corrupted, validate=True), payload)
        out.unlink(); packer.package(self.root, scene, out)
        out.write_text(out.read_text().replace(canonical, corrupted))
        checked = subprocess.run(['python3', str(Path(__file__).with_name('check_coop_defense_offline.py')), str(self.root), str(out)], capture_output=True, text=True)
        self.assertNotEqual(checked.returncode, 0)
        self.assertIn('noncanonical authored scene base64', checked.stderr)

        for bad in [b'', b'schema: wrong', scene.read_bytes().replace(b'reactor_hp: 75', b'reactor_hp: 0')]:
            with self.assertRaises(subprocess.CalledProcessError): packer.validate_scene(bad, (self.root/'game.wasm').read_bytes())
        with self.assertRaises(subprocess.CalledProcessError): packer.validate_scene(scene.read_bytes(), b'bad wasm')


if __name__ == "__main__": unittest.main()
