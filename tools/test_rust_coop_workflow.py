"""Actual legacy/native web copy and portable-recipe workflow differential.

Requires ORR_PKG_BIN, ORR_COOP_WEB_BUILD_DIR and Node on PATH; Pillow is only
legacy test infrastructure. No Cargo is run by these tests. Script rejection
checks use a harmless fake cargo marker; the real official build is a separate gate.
Copy parity covers bytes and directory inventory, NOT chmod/mtime preservation
or atomic multi-file replacement. Source-hidden HTML proof is payload integrity,
not browser/runtime execution after relocation.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
import base64

import create_coop_defense_scene as scene_author
from test_rust_coop_offline import png, Scripts

TOOLS = Path(__file__).resolve().parent
REPO = TOOLS.parent


def tree(root):
    result = {}
    if not root.exists():
        return result
    for directory, dirs, names in os.walk(root, followlinks=False):
        for name in dirs + names:
            path = Path(directory) / name
            relative = path.relative_to(root).as_posix()
            if path.is_symlink(): result[relative] = ('link', os.readlink(path))
            elif path.is_dir(): result[relative] = ('directory',)
            elif path.is_file(): result[relative] = ('file', path.read_bytes())
            else: result[relative] = ('nonregular',)
    return result


class RustCoopWorkflow(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        binary, build = os.environ.get('ORR_PKG_BIN'), os.environ.get('ORR_COOP_WEB_BUILD_DIR')
        if not binary or not Path(binary).is_absolute() or not Path(binary).is_file() or not os.access(binary, os.X_OK):
            raise RuntimeError('ORR_PKG_BIN must identify an explicit trusted executable')
        if not build or not Path(build).is_absolute() or not Path(build).is_dir():
            raise RuntimeError('ORR_COOP_WEB_BUILD_DIR must identify the reviewed actual clean build')
        if not shutil.which('node'):
            raise RuntimeError('Node required for actual packer/WASM/sampler admission')
        cls.binary, cls.build = binary, Path(build)
        cls.wasm = (cls.build / 'game.wasm').read_bytes()
        if cls.wasm[:8] != b'\0asm\x01\0\0\0':
            raise RuntimeError('Actual built WASM required, not a placeholder')

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / 'source'
        shutil.copytree(REPO / 'crates/orr_coop_defense_web/web', self.source)
        (self.source / 'assets/optional-empty/nested').mkdir(parents=True)
        (self.source / 'assets/optional-extra.txt').write_bytes(b'user optional extra\r\n')
        self.sentinel = self.root / 'unrelated'
        self.sentinel.write_bytes(b'keep')

    def execute(self, args, env=None):
        return subprocess.run(list(map(str, args)), capture_output=True, text=True, timeout=60,
                              env={**os.environ, 'PYTHONDONTWRITEBYTECODE': '1', **(env or {})})

    def copy_command(self, native, output, source=None, wasm=None):
        source = self.source if source is None else source
        if native:
            args = [self.binary, 'copy-coop-web-assets', '--source-dir', source, '--output', output]
            if wasm is not None: args += ['--wasm', wasm]
            return args
        # Legacy public CLI has a fixed repo source, so exercise its unchanged
        # actual library API with explicit test-owned source/output arguments.
        assert wasm is None
        return [sys.executable, '-B', '-c',
                'import sys;sys.path.insert(0,sys.argv[1]);from copy_coop_defense_web_assets import copy_assets;copy_assets(sys.argv[2],sys.argv[3])',
                TOOLS, source, output]

    def success(self, args):
        result = self.execute(args)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result

    def test_copy_exact_bytes_empty_optional_dirs_rerun_and_preserved_unowned_files(self):
        before = tree(self.source)
        outputs = [self.root / f'copy-{native}' for native in (False, True)]
        for native, output in zip((False, True), outputs):
            output.mkdir()
            for name, data in [('game.wasm', b'owned-by-cargo'), ('stormglass-foundry-offline.html', b'prior delivery'), ('unrelated.bin', b'keep')]:
                (output / name).write_bytes(data)
            self.success(self.copy_command(native, output))
        self.assertEqual(tree(outputs[0]), tree(outputs[1]))
        self.assertTrue((outputs[1] / 'assets/optional-empty/nested').is_dir())
        self.assertEqual(tree(self.source), before)
        css = self.source / 'style.css'
        css.write_bytes(css.read_bytes() + b'\n/* changed user build source */\n')
        (self.source / 'assets/optional-extra.txt').write_bytes(b'changed optional bytes')
        for native, output in zip((False, True), outputs):
            self.success(self.copy_command(native, output))
            self.assertEqual((output / 'style.css').read_bytes(), css.read_bytes())
            self.assertEqual((output / 'assets/optional-extra.txt').read_bytes(), b'changed optional bytes')
            self.assertEqual((output / 'game.wasm').read_bytes(), b'owned-by-cargo')
            self.assertEqual((output / 'stormglass-foundry-offline.html').read_bytes(), b'prior delivery')
            self.assertEqual((output / 'unrelated.bin').read_bytes(), b'keep')
        self.assertEqual(tree(outputs[0]), tree(outputs[1]))

    def test_missing_late_required_copy_input_never_modifies_output(self):
        late = self.source / 'assets/enemies.png'
        late.unlink()
        for native in (False, True):
            for exists in (False, True):
                output = self.root / f'missing-{native}-{exists}'
                if exists:
                    output.mkdir()
                    (output / 'style.css').write_bytes(b'preserve old style')
                    (output / 'sentinel').write_bytes(b'preserve')
                before = tree(output)
                result = self.execute(self.copy_command(native, output))
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(output.exists(), exists)
                self.assertEqual(tree(output), before)

    def test_native_copy_nofollow_types_overlap_preflight(self):
        # Legacy shutil follows links; stronger native admission is an explicit delta.
        target = self.source / 'assets/optional-extra.txt'
        original = target.read_bytes()
        target.unlink()
        target.symlink_to(self.sentinel)
        out = self.root / 'copy-refused'
        result = self.execute(self.copy_command(True, out))
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(out.exists())
        target.unlink()
        target.write_bytes(original)
        for kind in ('link', 'directory', 'fifo'):
            output = self.root / f'bad-dest-{kind}'
            output.mkdir()
            path = output / 'style.css'
            if kind == 'link': path.symlink_to(self.sentinel)
            elif kind == 'directory': path.mkdir()
            else: os.mkfifo(path)
            before = tree(output)
            self.assertNotEqual(self.execute(self.copy_command(True, output)).returncode, 0)
            self.assertEqual(tree(output), before)
            self.assertEqual(self.sentinel.read_bytes(), b'keep')
        before = tree(self.source)
        for output in (self.source, self.source / 'nested-output'):
            self.assertNotEqual(self.execute(self.copy_command(True, output)).returncode, 0)
            self.assertEqual(tree(self.source), before)

    def checked(self, web, html):
        result = self.execute([sys.executable, '-B', TOOLS / 'check_coop_defense_offline.py', web, html])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(json.loads(result.stdout)['passed'])

    def test_explicit_actual_wasm_copy_then_native_pack_and_checker(self):
        output = self.root / 'actual-build'
        output.mkdir()
        (output / 'game.wasm').write_bytes(b'replaced only by explicit --wasm')
        self.success(self.copy_command(True, output, wasm=self.build / 'game.wasm'))
        self.assertEqual((output / 'game.wasm').read_bytes(), self.wasm)
        html = self.root / 'copied-game.html'
        self.success([self.binary, 'pack-coop-offline', '--build-dir', output, '--output', html])
        self.checked(output, html)
        self.assertEqual(base64.b64decode(Scripts(html.read_text()).values['wasm-core'], validate=True), self.wasm)

    def recipe_fixture(self):
        bundle = self.root / 'recipe-bundle'
        bundle.mkdir()
        build = bundle / 'build'
        shutil.copytree(self.build, build)
        (build / 'assets/workflow').mkdir()
        (build / 'assets/workflow/atlas.png').write_bytes(png())
        (bundle / 'scene.yaml').write_text(scene_author.scene_text(250, 75, 3, 70))
        (bundle / 'animation.json').write_text(json.dumps({'unit:0': {'image': 'workflow/atlas.png', 'clips': {
            'idle': {'frames': [[0, 0, 2, 2], [2, 0, 2, 2]], 'fps': 12, 'loop': True}}}}))
        shutil.copytree(REPO / 'crates/orr_coop_defense_web/web/assets/audio', bundle / 'audio')
        data = {'format': 'orr_coop_project', 'version': 1, 'build': 'build',
                'scene': 'scene.yaml', 'animations': 'animation.json', 'audio': 'audio/bank.json'}
        recipe = bundle / 'project.json'
        recipe.write_text(json.dumps(data))
        return bundle, recipe, data

    def recipe_command(self, native, recipe, output):
        return ([self.binary, 'build-coop-recipe', '--recipe', recipe, '--output', output] if native else
                [sys.executable, '-B', TOOLS / 'build_coop_defense_recipe.py', recipe, '--output', output])

    def test_all_authored_recipe_portable_bundle_actual_html_then_sources_hidden(self):
        bundle, recipe, _ = self.recipe_fixture()
        outputs = [self.root / f'recipe-{native}.html' for native in (False, True)]
        for native, output in zip((False, True), outputs):
            self.success(self.recipe_command(native, recipe, output))
        old, new = [Scripts(p.read_text()).values for p in outputs]
        self.assertEqual(set(old), set(new))
        for name in old:
            if name in ('character-atlas-meta', 'authored-scene-receipt', 'authored-animation-receipt', 'authored-audio-bank'):
                self.assertEqual(json.loads(old[name]), json.loads(new[name]))
            else: self.assertEqual(old[name], new[name])
        self.checked(bundle / 'build', outputs[1])
        scene = (bundle / 'scene.yaml').read_bytes()
        animation = (bundle / 'animation.json').read_bytes()
        receipt = json.loads(new['authored-scene-receipt'])
        self.assertEqual(receipt['sceneSha256'], hashlib.sha256(scene).hexdigest())
        self.assertEqual(receipt['wasmSha256'], hashlib.sha256(self.wasm).hexdigest())
        self.assertRegex(receipt['initialChecksum'], r'^[0-9a-f]{16}$')
        moved = self.root / 'relocated bundle'
        bundle.rename(moved)
        relocated = self.root / 'relocated.html'
        self.success(self.recipe_command(True, moved / 'project.json', relocated))
        self.assertEqual(relocated.read_bytes(), outputs[1].read_bytes())
        self.checked(moved / 'build', relocated)
        moved.rename(self.root / 'hidden-source-bundle')
        self.assertFalse(moved.exists())
        # Payloads remain self-contained; no browser/engine run after hiding claimed.
        embedded = Scripts(relocated.read_text()).values
        self.assertEqual(base64.b64decode(embedded['wasm-core'], validate=True), self.wasm)
        self.assertEqual(base64.b64decode(embedded['authored-scene'], validate=True), scene)
        self.assertEqual(base64.b64decode(embedded['authored-animation-manifest'], validate=True), animation)
        self.assertNotIn(str(moved), relocated.read_text())
        self.assertNotIn(str(bundle), relocated.read_text())

    def test_recipe_closed_schema_relative_paths_symlinks_and_output_preservation(self):
        bundle, recipe, valid = self.recipe_fixture()
        bad = [None, [], {}, {**valid, 'unknown': True}, {**valid, 'format': 'bad'}]
        bad += [{**valid, 'version': value} for value in (True, False, 1.0, 2, '1', None)]
        bad += [{**valid, key: None} for key in ('build', 'scene', 'animations', 'audio')]
        bad += [{**valid, 'scene': value} for value in ('../scene.yaml', '/scene.yaml', './scene.yaml', 'x//scene.yaml', 'x\\scene.yaml', 'https://x/y')]
        for value in bad:
            recipe.write_text(json.dumps(value))
            for native in (False, True):
                output = self.root / 'invalid-recipe.html'
                self.assertNotEqual(self.execute(self.recipe_command(native, recipe, output)).returncode, 0)
                self.assertFalse(output.exists())
        for raw in (b'{"version":1,"version":1}', b' ' * 8193):
            recipe.write_bytes(raw)
            for native in (False, True):
                output = self.root / 'invalid-recipe.html'
                self.assertNotEqual(self.execute(self.recipe_command(native, recipe, output)).returncode, 0)
                self.assertFalse(output.exists())
        recipe.write_text(json.dumps(valid))
        scene = bundle / 'scene.yaml'
        original = scene.read_bytes()
        scene.unlink()
        scene.symlink_to(self.sentinel)
        for native in (False, True):
            output = self.root / 'invalid-recipe.html'
            self.assertNotEqual(self.execute(self.recipe_command(native, recipe, output)).returncode, 0)
            self.assertFalse(output.exists())
        scene.unlink()
        scene.write_bytes(original)
        for native in (False, True):
            output = self.root / f'preserved-recipe-{native}'
            output.write_bytes(b'keep existing delivery')
            self.assertNotEqual(self.execute(self.recipe_command(native, recipe, output)).returncode, 0)
            self.assertEqual(output.read_bytes(), b'keep existing delivery')
        self.assertEqual(self.sentinel.read_bytes(), b'keep')

    def test_official_script_refuses_existing_default_or_wasm_link_before_cargo(self):
        fakebin = self.root / 'fake-bin'
        fakebin.mkdir()
        marker = self.root / 'cargo-was-invoked'
        fake = fakebin / 'cargo'
        fake.write_text('#!/bin/sh\nprintf ran >> "$ORR_TEST_CARGO_MARKER"\nexit 97\n')
        fake.chmod(0o755)
        env = {'PATH': str(fakebin) + os.pathsep + os.environ['PATH'],
               'ORR_TEST_CARGO_MARKER': str(marker), 'CARGO_TARGET_DIR': str(self.root / 'unused-target')}
        script = TOOLS / 'build_coop_defense_web.sh'
        for kind in ('existing-default', 'wasm-symlink'):
            output = self.root / kind
            output.mkdir()
            if kind == 'existing-default':
                (output / 'stormglass-foundry-offline.html').write_bytes(b'keep prior delivery')
            else:
                (output / 'game.wasm').symlink_to(self.sentinel)
            before = tree(output)
            result = self.execute([shutil.which('bash'), script, output], env=env)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(marker.exists(), 'preflight must precede any Cargo invocation')
            self.assertEqual(tree(output), before)
            self.assertEqual(self.sentinel.read_bytes(), b'keep')


if __name__ == '__main__':
    unittest.main()
