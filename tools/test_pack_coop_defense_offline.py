#!/usr/bin/env python3
"""Portable regressions for the optional web package's exact module closure."""
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


if __name__ == "__main__": unittest.main()
