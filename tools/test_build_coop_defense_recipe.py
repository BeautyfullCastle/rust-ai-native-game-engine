"""Recipe admission and real CLI/packer checks; fake WASM is packaging-only."""
import base64
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

from build_coop_defense_recipe import build_recipe
from pack_coop_defense_audio import snapshot_audio
from pack_coop_defense_offline import snapshot_file, strict_json

TOOLS = Path(__file__).resolve().parent
WEB = TOOLS.parent / 'crates/orr_coop_defense_web/web'
_UNSET = object()


def embedded(html, name):
    match = re.search(r'<script[^>]*id="' + re.escape(name) + r'"[^>]*>(.*?)</script>', html, re.S)
    if not match:
        raise AssertionError(f'Missing embedded script {name}')
    return match.group(1).strip()


class Recipe(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.project = self.root / 'project'
        self.project.mkdir()
        self.build = self.project / 'build'
        shutil.copytree(WEB, self.build)
        self.wasm = b'non-executable packaging-only fixture'
        (self.build / 'game.wasm').write_bytes(self.wasm)
        self.recipe = self.project / 'project.json'
        self.output = self.root / 'delivery.html'
        self.data = {'format': 'orr_coop_project', 'version': 1, 'build': 'build'}
        self.write()

    def write(self, data=_UNSET):
        self.recipe.write_text(json.dumps(self.data if data is _UNSET else data))

    def rejected(self):
        before = sorted(str(p.relative_to(self.root)) for p in self.root.rglob('*'))
        with self.assertRaises((ValueError, OSError, RuntimeError, subprocess.SubprocessError)):
            build_recipe(self.recipe, self.output)
        self.assertFalse(os.path.lexists(self.output))
        self.assertEqual(sorted(str(p.relative_to(self.root)) for p in self.root.rglob('*')), before,
                         'failure must not leave new output or partial publication files')

    def test_strict_schema_missing_unknown_null_and_noninteger_versions(self):
        invalid = [[], None, {}, {**self.data, 'format': 'other'}, {**self.data, 'extra': 1},
                   {k: v for k, v in self.data.items() if k != 'build'}]
        invalid += [{**self.data, 'version': value} for value in [True, False, None, 1.0, 2, '1']]
        invalid += [{**self.data, key: None} for key in ['build', 'scene', 'animations', 'audio']]
        for data in invalid:
            with self.subTest(data=data):
                self.write(data)
                self.rejected()

    def test_malformed_duplicate_oversize_and_nonutf8_recipe_reject(self):
        for raw in [b'{', b'\xff', b' '*8193,
                    b'{"format":"orr_coop_project","version":1,"build":"build","build":"build"}',
                    b'{"format":"orr_coop_project","version":NaN,"build":"build"}']:
            with self.subTest(raw=raw[:100]):
                self.recipe.write_bytes(raw)
                self.rejected()

    def test_every_path_field_rejects_unsafe_syntax(self):
        for key in ['build', 'scene', 'animations', 'audio']:
            for value in ['', '.', '..', '/tmp/x', 'a/../b', './build', 'a//b', 'build/',
                          'a\\b', 'C:build', 'https:x', 1, True, []]:
                with self.subTest(key=key, value=value):
                    self.write({**self.data, key: value})
                    self.rejected()

    def test_symlink_components_and_recipe_symlink_reject(self):
        alias = self.project / 'alias'
        alias.symlink_to(self.build, target_is_directory=True)
        for data in [{**self.data, 'build': 'alias'},
                     {**self.data, 'audio': 'alias/assets/audio/bank.json'}]:
            self.write(data)
            self.rejected()
        final_link = self.project / 'scene-link.yaml'
        real_scene = self.project / 'real-scene.yaml'
        real_scene.write_text('schema: orr.scene/1')
        final_link.symlink_to(real_scene)
        self.write({**self.data, 'scene': final_link.name})
        self.rejected()
        self.write()
        real_parent = self.root / 'real-output'
        real_parent.mkdir()
        linked_parent = self.root / 'linked-output'
        linked_parent.symlink_to(real_parent, target_is_directory=True)
        with self.assertRaises((ValueError, OSError)):
            build_recipe(self.recipe, linked_parent / 'delivery.html')
        self.assertEqual(list(real_parent.iterdir()), [])
        original = self.project / 'original.json'
        self.recipe.rename(original)
        self.recipe.symlink_to(original)
        self.rejected()

    def test_output_collision_and_nonexistent_parent_preserve_state(self):
        self.output.write_bytes(b'keep existing delivery')
        with self.assertRaises((ValueError, OSError)):
            build_recipe(self.recipe, self.output)
        self.assertEqual(self.output.read_bytes(), b'keep existing delivery')
        self.output.unlink()
        target = self.root / 'target'
        target.write_bytes(b'keep target')
        self.output.symlink_to(target)
        with self.assertRaises((ValueError, OSError)):
            build_recipe(self.recipe, self.output)
        self.assertTrue(self.output.is_symlink())
        self.assertEqual(target.read_bytes(), b'keep target')
        self.output.unlink()
        with self.assertRaises((ValueError, OSError)):
            build_recipe(self.recipe, self.root / 'missing' / 'delivery.html')
        self.assertFalse((self.root / 'missing').exists())

    def test_actual_cli_resolves_relocated_recipe_not_current_directory(self):
        moved = self.root / 'relocated'
        self.project.rename(moved)
        result = subprocess.run([sys.executable, str(TOOLS / 'build_coop_defense_recipe.py'),
                                 str(moved / 'project.json'), '--output', str(self.output)],
                                cwd=self.root, capture_output=True, text=True, timeout=30,
                                env={**os.environ, 'PYTHONDONTWRITEBYTECODE': '1'})
        self.assertEqual(result.returncode, 0, result.stderr)
        html = self.output.read_text()
        self.assertEqual(base64.b64decode(embedded(html, 'wasm-core')), self.wasm)
        self.assertNotIn(str(moved), html)
        self.assertNotIn("from './run-save.mjs'", html)

    def test_optional_audio_animation_combination_exact_embedding_and_fail_closed(self):
        manifest = {'unit:0': {'image': 'units.png', 'clips': {'idle': {'frames': [[0, 0, 1, 1]]}}}}
        animation = self.project / 'animation.json'
        animation.write_text(json.dumps(manifest))
        self.data.update(animations='animation.json', audio='build/assets/audio/bank.json')
        self.write()
        result = build_recipe(self.recipe, self.output)
        self.assertEqual(Path(result), self.output)
        html = self.output.read_text()
        self.assertEqual(base64.b64decode(embedded(html, 'authored-animation-manifest')), animation.read_bytes())
        self.assertIn(snapshot_audio(self.build / 'assets/audio/bank.json', snapshot_file, strict_json), html)
        self.output.unlink()
        animation.write_text('{"unit:0":{"image":"missing.png","clips":{"idle":{"frames":[[0,0,1,1]]}}}}')
        self.rejected()
        animation.write_text(json.dumps(manifest))
        (self.build / 'assets/audio/summon.pcm').write_bytes(b'tampered')
        self.rejected()

    @unittest.skipUnless(os.environ.get('ORR_RECIPE_TEST_BUILD') and os.environ.get('ORR_RECIPE_TEST_SCENE'),
                         'Actual WASM/scene admission requires ORR_RECIPE_TEST_BUILD and ORR_RECIPE_TEST_SCENE')
    def test_explicit_verified_build_and_scene_use_actual_wasm_admission(self):
        shutil.rmtree(self.build)
        shutil.copytree(Path(os.environ['ORR_RECIPE_TEST_BUILD']), self.build)
        source = Path(os.environ['ORR_RECIPE_TEST_SCENE'])
        scene = self.project / 'authored.scene.yaml'
        shutil.copyfile(source, scene)
        animation = self.project / 'actual-animation.json'
        animation.write_text(json.dumps({'unit:0': {'image': 'units.png',
            'clips': {'idle': {'frames': [[0, 0, 1, 1]]}}}}))
        self.data.update(scene=scene.name, animations=animation.name,
                         audio='build/assets/audio/bank.json')
        self.write()
        build_recipe(self.recipe, self.output)
        html = self.output.read_text()
        self.assertEqual(base64.b64decode(embedded(html, 'authored-scene')), source.read_bytes())
        self.assertEqual(base64.b64decode(embedded(html, 'wasm-core')), (self.build / 'game.wasm').read_bytes())
        self.assertIsInstance(json.loads(embedded(html, 'authored-scene-receipt')), dict)
        self.assertEqual(base64.b64decode(embedded(html, 'authored-animation-manifest')), animation.read_bytes())
        self.assertIn(snapshot_audio(self.build / 'assets/audio/bank.json', snapshot_file, strict_json), html)
        self.output.unlink()
        scene.write_bytes(b'schema: invalid.scene/1\nentities: {}\n')
        self.rejected()


if __name__ == '__main__':
    unittest.main()
