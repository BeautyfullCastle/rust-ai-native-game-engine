"""Official web asset staging contract; fake WASM checks packaging only."""
import base64
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

from copy_coop_defense_web_assets import copy_assets
from pack_coop_defense_offline import ASSETS

TOOLS = Path(__file__).resolve().parent
WEB = TOOLS.parent / 'crates/orr_coop_defense_web/web'
REQUIRED = tuple(name for name in ASSETS if name != 'game.wasm')


def tree(root):
    return {str(path.relative_to(root)): path.read_bytes()
            for path in root.rglob('*') if path.is_file()}


class CopyWebAssets(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.output = self.root / 'output'

    def test_real_web_required_bytes_nested_audio_and_complete_optional_art(self):
        before = tree(WEB)
        copy_assets(WEB, self.output)
        for name in REQUIRED:
            self.assertEqual((self.output / name).read_bytes(), before[name], name)
        self.assertEqual(tree(self.output / 'assets'), tree(WEB / 'assets'))
        for name in ['run-save.mjs', 'audio/authored-asset-audio.mjs', 'audio/coop-defense-audio.mjs']:
            self.assertIn(name, REQUIRED)
            self.assertTrue((self.output / name).is_file(), name)
        self.assertFalse((self.output / 'game.wasm').exists(), 'helper must not fabricate a native build')
        self.assertEqual(tree(WEB), before)

    def test_each_missing_required_file_preflights_before_output_creation(self):
        source = self.root / 'source'
        shutil.copytree(WEB, source)
        for name in REQUIRED:
            with self.subTest(name=name):
                path = source / name
                original = path.read_bytes()
                path.unlink()
                try:
                    with self.assertRaises(FileNotFoundError):
                        copy_assets(source, self.output)
                    self.assertFalse(self.output.exists())
                finally:
                    path.write_bytes(original)

    def test_missing_required_leaves_existing_output_completely_unchanged(self):
        source = self.root / 'source'
        shutil.copytree(WEB, source)
        (source / 'run-save.mjs').unlink()
        self.output.mkdir()
        (self.output / 'game.js').write_bytes(b'old retained game')
        (self.output / 'extra.keep').write_bytes(b'owner file')
        before = tree(self.output)
        with self.assertRaises(FileNotFoundError):
            copy_assets(source, self.output)
        self.assertEqual(tree(self.output), before)

    def test_rerun_updates_required_and_optional_assets_without_deleting_extras(self):
        source = self.root / 'source'
        shutil.copytree(WEB, source)
        optional = source / 'assets/optional/nested/user-art.bin'
        optional.parent.mkdir(parents=True)
        optional.write_bytes(b'first custom art')
        copy_assets(source, self.output)
        (self.output / 'extra.keep').write_bytes(b'keep')
        (self.output / 'assets/owner-extra.bin').write_bytes(b'keep extra asset')
        (self.output / 'game.wasm').write_bytes(b'previous build remains until build replaces it')
        (source / 'style.css').write_bytes(b'/* updated source */')
        optional.write_bytes(b'updated custom art')
        copy_assets(source, self.output)
        self.assertEqual((self.output / 'style.css').read_bytes(), b'/* updated source */')
        self.assertEqual((self.output / 'assets/optional/nested/user-art.bin').read_bytes(), b'updated custom art')
        self.assertEqual((self.output / 'extra.keep').read_bytes(), b'keep')
        self.assertEqual((self.output / 'assets/owner-extra.bin').read_bytes(), b'keep extra asset')
        self.assertEqual((self.output / 'game.wasm').read_bytes(), b'previous build remains until build replaces it')

    def test_actual_helper_cli_then_actual_offline_packer_packaging_only(self):
        environment = {**os.environ, 'PYTHONDONTWRITEBYTECODE': '1'}
        copied = subprocess.run([sys.executable, str(TOOLS / 'copy_coop_defense_web_assets.py'), str(self.output)],
                                capture_output=True, text=True, timeout=30, env=environment)
        self.assertEqual(copied.returncode, 0, copied.stderr)
        # Deliberately not executable: this establishes closure/embedding, not
        # actual Rust build correctness or browser execution.
        fake_wasm = b'non-executable packaging-only WASM fixture'
        (self.output / 'game.wasm').write_bytes(fake_wasm)
        destination = self.root / 'offline.html'
        packed = subprocess.run([sys.executable, str(TOOLS / 'pack_coop_defense_offline.py'),
                                 str(self.output), '--output', str(destination)],
                                capture_output=True, text=True, timeout=30, env=environment)
        self.assertEqual(packed.returncode, 0, packed.stderr)
        html = destination.read_text()
        embedded = re.search(r'<script[^>]*id="wasm-core"[^>]*>(.*?)</script>', html, re.S)
        self.assertIsNotNone(embedded)
        self.assertEqual(base64.b64decode(embedded.group(1).strip()), fake_wasm)
        self.assertIn('class CoopDefenseAudio', html)
        self.assertIn('class AuthoredAssetAudio', html)
        self.assertIn('function parseRunSave', html)
        self.assertNotIn("from './run-save.mjs'", html)
        self.assertNotIn("from './audio/coop-defense-audio.mjs'", html)
        for name in REQUIRED:
            self.assertEqual((self.output / name).read_bytes(), (WEB / name).read_bytes())


if __name__ == '__main__':
    unittest.main()
