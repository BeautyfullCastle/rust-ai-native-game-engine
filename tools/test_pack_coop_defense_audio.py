import base64
import hashlib
from html.parser import HTMLParser
import json
import os
from pathlib import Path
import shutil
import struct
import tempfile
import unittest

from pack_coop_defense_audio import snapshot_audio
import pack_coop_defense_offline as packer
from pack_coop_defense_offline import snapshot_file, strict_json

SOURCE = Path(__file__).resolve().parents[1] / 'crates/orr_coop_defense_web/web/assets/audio'


def script_ids(html):
    # Script bodies contain selector strings too; inspect actual HTML tags.
    class Inventory(HTMLParser):
        def __init__(self):
            super().__init__()
            self.ids = []

        def handle_starttag(self, tag, attrs):
            if tag == 'script':
                self.ids.extend(value for name, value in attrs if name == 'id')

    parser = Inventory()
    parser.feed(html)
    parser.close()
    return parser.ids


class AudioAdmission(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / 'bank'
        shutil.copytree(SOURCE, self.root)
        self.path = self.root / 'bank.json'

    def admit(self):
        return snapshot_audio(self.path, snapshot_file, strict_json)

    def edit(self, fn):
        data = json.loads(self.path.read_text())
        fn(data)
        self.path.write_text(json.dumps(data))

    def test_real_bank_roundtrip(self):
        html = self.admit()
        data = json.loads(html.split('>', 1)[1].rsplit('<', 1)[0])
        self.assertEqual(len(data['objects']), 10)
        for name, encoded in data['objects'].items():
            self.assertEqual(base64.b64decode(encoded), (self.root / name).read_bytes())

    def test_script_escape(self):
        self.edit(lambda d: d.update(description='</script><img src=x>'))
        self.assertEqual(self.admit().count('</script>'), 1)

    def test_metadata_rejections(self):
        original = self.path.read_bytes()
        mutations = [lambda d: d.update(version=True), lambda d: d.update(sampleRate=True),
                     lambda d: d['cues'][0].update(frames=True),
                     lambda d: d['cues'][0].update(file='../attack-light.pcm'),
                     lambda d: d['cues'][0].update(assetId=d['cues'][1]['assetId']),
                     lambda d: d['cues'].__setitem__(1, d['cues'][0]),
                     lambda d: d['cues'][0].update(frames=48001)]
        for fn in mutations:
            self.path.write_bytes(original)
            self.edit(fn)
            with self.assertRaises(ValueError):
                self.admit()

    def test_duplicate_json_key(self):
        self.path.write_text('{"version":1,"version":1}')
        with self.assertRaises(ValueError):
            self.admit()

    def test_corruption_header_peak_and_short_data(self):
        original = self.path.read_bytes()
        pcm = self.root / 'attack-light.pcm'
        original_pcm = pcm.read_bytes()
        for mode in ('hash', 'header', 'peak', 'short'):
            self.path.write_bytes(original)
            data = bytearray(original_pcm)
            if mode == 'short':
                data = data[:-1]
            elif mode == 'peak':
                struct.pack_into('<h', data, 8, 32767)
            else:
                data[0] ^= 1
            pcm.write_bytes(data)
            if mode != 'hash':
                self.edit(lambda d: d['cues'][0].update(sha256=hashlib.sha256(data).hexdigest()))
            with self.assertRaises(ValueError):
                self.admit()

    def test_symlink_and_fifo_do_not_follow_or_block(self):
        pcm = self.root / 'attack-light.pcm'
        pcm.unlink()
        pcm.symlink_to(SOURCE / pcm.name)
        with self.assertRaises(OSError):
            self.admit()
        pcm.unlink()
        os.mkfifo(pcm)
        with self.assertRaises(ValueError):
            self.admit()

    def test_manifest_symlink_and_size_bound(self):
        self.path.unlink()
        self.path.symlink_to(SOURCE / 'bank.json')
        with self.assertRaises(OSError):
            self.admit()
        self.path.unlink()
        self.path.write_bytes(b' ' * (128 * 1024 + 1))
        with self.assertRaises(ValueError):
            self.admit()


class AudioPackage(unittest.TestCase):
    """Packaging contracts; fixture WASM is deliberately not executable."""
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / 'web'
        self.root.mkdir()
        web = SOURCE.parent.parent
        for name in (*packer.MODULES, *packer.EXTRA_MODULES, 'game.js', 'index.html', 'style.css'):
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(web / name, target)
        (self.root / 'game.wasm').write_bytes(b'fixture, not executable WASM')
        (self.root / 'assets').mkdir()
        for name, data in {'characters.json': b'{"heroes":[],"enemies":[]}',
                           'units.png': b'fixture heroes', 'enemies.png': b'fixture enemies'}.items():
            (self.root / 'assets' / name).write_bytes(data)
        self.bank = Path(self.temp.name) / 'bank'
        shutil.copytree(SOURCE, self.bank)
        self.output = Path(self.temp.name) / 'delivery.html'

    def test_absent_bank_retains_runsave_graph_without_pcm(self):
        html = packer.package(self.root, output=self.output).read_text()
        self.assertNotIn('authored-audio-bank', script_ids(html))
        self.assertIn((self.root / 'run-save.mjs').read_text(), html)
        self.assertIn('loadAuthoredAudio', html)
        self.assertFalse((self.root / 'assets/audio').exists())

    def test_present_bank_embeds_exact_pcm_and_runsave(self):
        html = packer.package(self.root, output=self.output, audio=self.bank / 'bank.json').read_text()
        self.assertEqual(script_ids(html).count('authored-audio-bank'), 1)
        self.assertIn((self.root / 'run-save.mjs').read_text(), html)
        self.assertIn(base64.b64encode((self.bank / 'ui-confirm.pcm').read_bytes()).decode(), html)
        self.assertNotIn(str(self.bank), html)
        with self.assertRaises(FileExistsError):
            packer.package(self.root, output=self.output, audio=self.bank / 'bank.json')

    def test_audio_requires_explicit_new_destination(self):
        with self.assertRaises(ValueError):
            packer.package(self.root, audio=self.bank / 'bank.json')
        self.assertFalse((self.root / 'stormglass-foundry-offline.html').exists())

    def test_malformed_explicit_bank_publishes_nothing(self):
        (self.bank / 'bank.json').write_text('{"version":false}')
        with self.assertRaisesRegex(ValueError, '^Unsupported audio bank$'):
            packer.package(self.root, output=self.output, audio=self.bank / 'bank.json')
        self.assertFalse(self.output.exists())
        self.assertFalse((self.root / 'stormglass-foundry-offline.html').exists())

    def test_reserved_audio_id_and_unknown_dependency_fail_closed(self):
        index = self.root / 'index.html'
        original = index.read_text()
        index.write_text(original + '<script id="authored-audio-bank"></script>')
        with self.assertRaisesRegex(ValueError, 'Reserved'):
            packer.package(self.root, output=self.output)
        index.write_text(original)
        module = self.root / 'audio/coop-defense-audio.mjs'
        module.write_text(module.read_text() + "\nawait import('https://example.invalid/audio.mjs');")
        with self.assertRaisesRegex(ValueError, 'dependency'):
            packer.package(self.root, output=self.output)
        self.assertFalse(self.output.exists())


if __name__ == '__main__':
    unittest.main()
