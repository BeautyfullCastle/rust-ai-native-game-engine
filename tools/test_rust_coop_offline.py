"""Actual Rust/legacy offline packer differential over the reviewed clean WASM build.

Requires ORR_PKG_BIN, ORR_COOP_WEB_BUILD_DIR, Node on PATH and Pillow for the
legacy oracle. No mock engine/sampler. Native output is always explicit/new.
Raw metadata JSON formatting may differ; decoded values/payload bytes must not.
This is package integrity and Node admission/syntax, never browser/GPU acceptance.
"""
import base64
import hashlib
from html.parser import HTMLParser
import itertools
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest
import zlib

import create_coop_defense_scene as scene_author
import pack_coop_defense_offline as legacy
from pack_coop_defense_audio import snapshot_audio

TOOLS = Path(__file__).resolve().parent
REPO = TOOLS.parent


def png():
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    pixels = b''.join(b'\0' + bytes([255, 0, 0, 255]) * 4 for _ in range(2))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 4, 2, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(pixels)) + chunk(b'IEND', b''))


class Scripts(HTMLParser):
    def __init__(self, text):
        super().__init__()
        self.current = None
        self.values = {}
        self.external = []
        self.feed(text)
        self.close()

    def handle_starttag(self, tag, attrs):
        if tag == 'script':
            values = dict(attrs)
            if 'src' in values:
                self.external.append(values['src'])
            self.current = values.get('id')
            if self.current:
                if self.current in self.values:
                    raise AssertionError('duplicate script ID')
                self.values[self.current] = ''
        if tag == 'link' and dict(attrs).get('rel') == 'stylesheet':
            self.external.append(dict(attrs).get('href'))

    def handle_endtag(self, tag):
        if tag == 'script':
            self.current = None

    def handle_data(self, text):
        if self.current:
            self.values[self.current] += text


def digest(data):
    return hashlib.sha256(data).hexdigest()


class RustCoopOffline(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        binary, build = os.environ.get('ORR_PKG_BIN'), os.environ.get('ORR_COOP_WEB_BUILD_DIR')
        if not binary or not Path(binary).is_absolute() or not Path(binary).is_file() or not os.access(binary, os.X_OK):
            raise RuntimeError('ORR_PKG_BIN must identify an explicit trusted native executable')
        if not build or not Path(build).is_absolute() or not Path(build).is_dir():
            raise RuntimeError('ORR_COOP_WEB_BUILD_DIR must identify the actual reviewed clean build')
        if not shutil.which('node'):
            raise RuntimeError('Node is required for the actual verifier/sampler/checker')
        cls.binary, cls.build = binary, Path(build)
        if (cls.build / 'game.wasm').read_bytes()[:8] != b'\0asm\x01\0\0\0':
            raise RuntimeError('Actual WASM build required, not placeholder bytes')

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.web = self.root / 'web'
        shutil.copytree(self.build, self.web)
        self.scene = self.root / 'scene.yaml'
        self.scene.write_text(scene_author.scene_text(250, 75, 3, 70))
        (self.web / 'assets/authored-test').mkdir()
        self.atlas = self.web / 'assets/authored-test/atlas.png'
        self.atlas.write_bytes(png())
        self.animation = self.root / 'animation.json'
        self.animation.write_text(json.dumps({'unit:0': {'image': 'authored-test/atlas.png', 'clips': {
            'idle': {'frames': [[0, 0, 2, 2], [2, 0, 2, 2]], 'fps': 12, 'loop': True}}}}))
        self.bank = self.root / 'bank'
        shutil.copytree(REPO / 'crates/orr_coop_defense_web/web/assets/audio', self.bank)
        self.audio = self.bank / 'bank.json'
        self.sentinel = self.root / 'unrelated-user-file'
        self.sentinel.write_bytes(b'keep')
        self.serial = 0

    def command(self, native, output, scene=False, animations=False, audio=False, build=None):
        build = self.web if build is None else build
        args = ([self.binary, 'pack-coop-offline', '--build-dir', str(build)] if native else
                [sys.executable, '-B', str(TOOLS / 'pack_coop_defense_offline.py'), str(build)])
        args += ['--output', str(output)]
        for flag, enabled, path in [('--scene', scene, self.scene), ('--animations', animations, self.animation), ('--audio', audio, self.audio)]:
            if enabled:
                args += [flag, str(path)]
        return args

    def execute(self, args):
        return subprocess.run(args, capture_output=True, text=True, timeout=60,
                              env={**os.environ, 'PYTHONDONTWRITEBYTECODE': '1'})

    def pack(self, native, **options):
        self.serial += 1
        out = self.root / f'packed-{self.serial}-{native}.html'
        result = self.execute(self.command(native, out, **options))
        self.assertEqual(result.returncode, 0, result.stderr)
        return out

    def rejected(self, both=True, **options):
        for native in (False, True) if both else (True,):
            self.serial += 1
            out = self.root / f'refused-{self.serial}.html'
            result = self.execute(self.command(native, out, **options))
            self.assertNotEqual(result.returncode, 0, result.stdout)
            self.assertFalse(os.path.lexists(out), 'failed admission published output')
            self.assertEqual(self.sentinel.read_bytes(), b'keep')

    def checked(self, path):
        result = self.execute([sys.executable, '-B', str(TOOLS / 'check_coop_defense_offline.py'), str(self.web), str(path)])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(json.loads(result.stdout)['passed'])

    def assert_semantics(self, old, new, scene=False, animations=False, audio=False):
        a, b = Scripts(old.read_text()), Scripts(new.read_text())
        self.assertFalse(a.external)
        self.assertFalse(b.external)
        self.assertEqual(set(a.values), set(b.values))
        for identity in a.values:
            if identity in ('character-atlas-meta', 'authored-scene-receipt', 'authored-animation-receipt', 'authored-audio-bank'):
                self.assertEqual(json.loads(a.values[identity]), json.loads(b.values[identity]), identity)
            else:
                self.assertEqual(a.values[identity], b.values[identity], identity)
        self.assertEqual(b.values['game-module'], legacy.bundle_game(self.web))
        for identity, name in [('wasm-core', 'game.wasm'), ('tower-atlas', 'assets/units.png'), ('enemy-atlas', 'assets/enemies.png')]:
            self.assertEqual(base64.b64decode(b.values[identity], validate=True), (self.web / name).read_bytes())
        self.assertEqual(json.loads(b.values['character-atlas-meta']), json.loads((self.web / 'assets/characters.json').read_bytes()))
        self.assertEqual('authored-scene' in b.values, scene)
        self.assertEqual('authored-animation-manifest' in b.values, animations)
        self.assertEqual('authored-audio-bank' in b.values, audio)
        if scene:
            self.assertEqual(base64.b64decode(b.values['authored-scene'], validate=True), self.scene.read_bytes())
            receipt = json.loads(b.values['authored-scene-receipt'])
            self.assertEqual(receipt['sceneSha256'], digest(self.scene.read_bytes()))
            self.assertEqual(receipt['wasmSha256'], digest((self.web / 'game.wasm').read_bytes()))
            self.assertRegex(receipt['initialChecksum'], r'^[0-9a-f]{16}$')
        if animations:
            self.assertEqual(base64.b64decode(b.values['authored-animation-manifest'], validate=True), self.animation.read_bytes())
            receipt = json.loads(b.values['authored-animation-receipt'])
            self.assertEqual(receipt['manifestSha256'], digest(self.animation.read_bytes()))
            self.assertEqual(receipt['manifestBytes'], self.animation.stat().st_size)
            for image in receipt['images']:
                payload = base64.b64decode(b.values[image['id']], validate=True)
                self.assertEqual(payload, (self.web / 'assets' / image['name']).read_bytes())
                self.assertEqual(digest(payload), image['sha256'])
        if audio:
            payload = json.loads(b.values['authored-audio-bank'])
            expected = Scripts(snapshot_audio(self.audio, legacy.snapshot_file, legacy.strict_json)).values
            self.assertEqual(payload, json.loads(expected['authored-audio-bank']))
            for name, encoded in payload['objects'].items():
                self.assertEqual(base64.b64decode(encoded, validate=True), (self.bank / name).read_bytes())
        return b

    def test_all_eight_optional_combinations_actual_payloads_and_unchanged_checker(self):
        before = {name: digest((self.web / name).read_bytes()) for name in legacy.ASSETS}
        for scene, animations, audio in itertools.product((False, True), repeat=3):
            with self.subTest(scene=scene, animations=animations, audio=audio):
                options = dict(scene=scene, animations=animations, audio=audio)
                old, new = self.pack(False, **options), self.pack(True, **options)
                self.assert_semantics(old, new, **options)
                self.checked(new)  # actual supplied WASM, build sampler and Node --check
        self.assertEqual(before, {name: digest((self.web / name).read_bytes()) for name in legacy.ASSETS})

    def test_uses_actual_build_sampler_not_repository_authoring_sampler(self):
        sampler = self.web / 'combat-presentation.mjs'
        original = sampler.read_text()
        self.assertEqual(original.count('const raw = Math.floor'), 1)
        sampler.write_text(original.replace('const raw = Math.floor', 'const raw = 1 + Math.floor'))
        self.rejected(animations=True)
        self.assertEqual(sampler.read_text(), original.replace('const raw = Math.floor', 'const raw = 1 + Math.floor'))

    def test_html_id_tokenizer_entities_case_comments_and_raw_text(self):
        index = self.web / 'index.html'
        original = index.read_text()
        for suffix in ['<div id="wasm-core"></div>', '<div ID="wasm-core"></div>',
                       '<div id="&#x77;asm-core"></div>', '<div id="authored-animation-image-0"></div>',
                       '<script id="combat-animation-manifest">{}</script>',
                       '<div id="x" id="y"></div>', '<div id="same"></div><p id="same"></p>',
                       '<div id></div>']:
            index.write_text(original + suffix)
            self.rejected()
        for suffix in ['<!-- <div id="wasm-core"></div> -->',
                       '<script>const text = \'<div id="wasm-core"></div>\';</script>',
                       '<style>/* <div id="wasm-core"></div> */</style>',
                       '<div id="WASM-CORE"></div>']:
            index.write_text(original + suffix)
            old, new = self.pack(False), self.pack(True)
            self.assert_semantics(old, new)
        for anchor in ['<link rel="stylesheet" href="style.css">', '<script type="module" src="game.js"></script>']:
            self.assertEqual(original.count(anchor), 1)
            for changed in (original.replace(anchor, ''), original + anchor):
                index.write_text(changed)
                self.rejected()
        index.write_text(original)

    def test_closed_javascript_graph_and_mixed_case_html_terminators(self):
        source = self.web / 'combat-presentation.mjs'
        original = source.read_text()
        for extra in ["import { x } from './unknown.mjs';", "await import('./unknown.mjs');",
                      "export * from './unknown.mjs';", "import/**/ './unknown.mjs';",
                      "export/**/ {x} from './unknown.mjs';", "// unsupported import keyword",
                      "// </ScRiPt>"]:
            source.write_text(original + '\n' + extra)
            self.rejected()
        source.write_text(original)
        entry = self.web / 'game.js'
        original_entry = entry.read_text()
        entry.write_text("import './unknown.mjs';\n" + original_entry)
        self.rejected()
        entry.write_text(original_entry)
        css = self.web / 'style.css'
        original_css = css.read_text()
        css.write_text(original_css + '/* </StYlE> */')
        self.rejected()
        css.write_text(original_css)

    def test_audio_unknown_fields_order_and_independent_pcm_guards(self):
        manifest = json.loads(self.audio.read_bytes())
        manifest['description'] = '</script><img src=x> 사용자'
        manifest['future'] = {'value': 9007199254740993, 'nested': [True, None]}
        manifest['cues'][0]['comment'] = 'preserve record extension'
        manifest['cues'].reverse()
        self.audio.write_text(json.dumps(manifest, ensure_ascii=False, indent=2))
        old, new = self.pack(False, audio=True), self.pack(True, audio=True)
        embedded = self.assert_semantics(old, new, audio=True)
        self.assertEqual(json.loads(embedded.values['authored-audio-bank'])['manifest'], manifest)
        saved_manifest = self.audio.read_bytes()
        record = manifest['cues'][0]
        pcm = self.bank / record['file']
        original = pcm.read_bytes()
        for mode in ('digest', 'header', 'peak', 'short'):
            changed = bytearray(original)
            if mode == 'short':
                changed = changed[:-1]
            elif mode == 'peak':
                struct.pack_into('<h', changed, 8, 32767)
            else:
                changed[0] ^= 1
            value = json.loads(saved_manifest)
            if mode != 'digest':
                value['cues'][0]['sha256'] = digest(changed)
            pcm.write_bytes(changed)
            self.audio.write_text(json.dumps(value))
            self.rejected(audio=True)
        pcm.write_bytes(original)
        for raw in [b'{"version":1,"version":1}', b'{"version":NaN}', b' ' * (128 * 1024 + 1)]:
            self.audio.write_bytes(raw)
            self.rejected(audio=True)
        self.audio.write_bytes(saved_manifest)

    def test_character_metadata_last_wins_large_raw_numbers_and_strict_delta(self):
        metadata = self.web / 'assets/characters.json'
        original = metadata.read_text()
        value = json.loads(original)
        # Preserve genuine hero/enemy arrays for the unchanged offline checker.
        suffix = ',"probe":0,"probe":9007199254740993,"exponent":1e2,"negativeZero":-0.0,"label":"사용자😀</script>"}'
        metadata.write_text(original.rstrip()[:-1] + suffix)
        old, new = self.pack(False), self.pack(True)
        embedded = self.assert_semantics(old, new)
        actual = json.loads(embedded.values['character-atlas-meta'])
        self.assertEqual(actual['probe'], 9007199254740993)
        self.assertEqual(actual['exponent'], 100.0)
        self.assertEqual(actual['heroes'], value['heroes'])
        self.checked(new)
        for bad in ('NaN', 'Infinity', '-Infinity'):
            metadata.write_text(original.rstrip()[:-1] + ',"probe":' + bad + '}')
            # Deliberate safety improvement: legacy emits invalid-for-browser JSON.
            legacy_output = self.pack(False)
            self.assertIn(bad, Scripts(legacy_output.read_text()).values['character-atlas-meta'])
            self.rejected(both=False)
        metadata.write_text('{"nested":' + '[' * 129 + '0' + ']' * 129 + '}')
        self.rejected(both=False)  # documented new maximum JSON nesting
        metadata.write_text(original)

    def test_nofollow_fifo_missing_late_input_and_no_overwrite(self):
        image_bytes = self.atlas.read_bytes()
        self.atlas.unlink()
        self.atlas.symlink_to(self.web / 'assets/units.png')
        self.rejected(animations=True)
        self.atlas.unlink()
        self.atlas.write_bytes(image_bytes)
        pcm = self.bank / json.loads(self.audio.read_bytes())['cues'][-1]['file']
        pcm_bytes = pcm.read_bytes()
        pcm.unlink()
        os.mkfifo(pcm)
        self.rejected(audio=True)
        pcm.unlink()
        pcm.write_bytes(pcm_bytes)
        original_scene = self.scene.read_bytes()
        self.scene.unlink()
        self.scene.symlink_to(self.sentinel)
        self.rejected(scene=True)
        self.scene.unlink()
        self.scene.write_bytes(original_scene)
        alias = self.root / 'build-alias'
        alias.symlink_to(self.web, target_is_directory=True)
        self.rejected(build=alias)
        nested = self.web / 'audio'
        nested.rename(self.web / 'saved-audio')
        nested.symlink_to(self.web / 'saved-audio', target_is_directory=True)
        self.rejected()
        nested.unlink()
        (self.web / 'saved-audio').rename(nested)
        late = self.web / 'assets/enemies.png'
        data = late.read_bytes()
        late.unlink()
        self.rejected()
        late.write_bytes(data)
        for native in (False, True):
            for kind in ('file', 'directory', 'link'):
                out = self.root / f'existing-{native}-{kind}'
                if kind == 'file': out.write_bytes(b'keep')
                elif kind == 'directory':
                    out.mkdir()
                    (out / 'sentinel').write_bytes(b'keep')
                else: out.symlink_to('missing')
                result = self.execute(self.command(native, out))
                self.assertNotEqual(result.returncode, 0)
                if kind == 'file': self.assertEqual(out.read_bytes(), b'keep')
                elif kind == 'directory': self.assertEqual((out / 'sentinel').read_bytes(), b'keep')
                else: self.assertEqual(os.readlink(out), 'missing')
            link = self.root / f'parent-{native}'
            link.symlink_to(self.root, target_is_directory=True)
            result = self.execute(self.command(native, link / 'not-created'))
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((self.root / 'not-created').exists())

    def test_fixed_assets_opaque_without_animation_and_native_requires_new_output(self):
        atlas = self.web / 'assets/units.png'
        atlas.write_bytes(b'opaque fixed atlas, deliberately not PNG')
        old, new = self.pack(False), self.pack(True)
        self.assert_semantics(old, new)
        self.animation.write_text(json.dumps({'unit:0': {'image': 'units.png', 'clips': {'idle': {'frames': [[0, 0, 1, 1]]}}}}))
        self.rejected(animations=True)
        # The reviewed clean build may already contain its historical default HTML.
        # Missing --output must neither create nor overwrite that unrelated artifact.
        default_output = self.web / 'stormglass-foundry-offline.html'
        default_existed = os.path.lexists(default_output)
        default_bytes = default_output.read_bytes() if default_existed else None
        result = self.execute([self.binary, 'pack-coop-offline', '--build-dir', str(self.web)])
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertEqual(os.path.lexists(default_output), default_existed)
        if default_existed:
            self.assertEqual(default_output.read_bytes(), default_bytes)
        for flag in ('--node', '--script', '--command'):
            output = self.root / 'no-output'
            result = self.execute(self.command(True, output) + [flag, str(self.sentinel)])
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertFalse(output.exists())


if __name__ == '__main__':
    unittest.main()
