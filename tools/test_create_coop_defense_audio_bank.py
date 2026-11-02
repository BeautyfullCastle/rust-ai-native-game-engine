import hashlib
import json
import os
import shutil
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch

from create_coop_defense_audio_bank import create_bank, MAX_PCM
from pack_coop_defense_audio import CUES, snapshot_audio
from pack_coop_defense_offline import snapshot_file, strict_json


class Authoring(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / 'source'
        self.source.mkdir()
        self.inputs = []
        for i, cue in enumerate(CUES):
            path = self.source / (cue + '.pcm')
            path.write_bytes(struct.pack('<IIhhh', 48000, 3, i+1, -i-1, 0))
            self.inputs.append(f'{cue}={path}')
        self.license = self.source / 'LICENSE.txt'
        self.license.write_text('Synthetic fixture license\n')
        self.output = self.root / 'created'

    def create(self, **kw):
        return create_bank(kw.get('inputs', self.inputs), kw.get('license', self.license),
                           kw.get('output', self.output))

    def test_deterministic_complete_bank_and_source_preservation(self):
        before = {p.name: p.read_bytes() for p in self.source.iterdir()}
        self.create()
        other = self.root / 'other'
        self.create(inputs=list(reversed(self.inputs)), output=other)
        files = {p.name: p.read_bytes() for p in self.output.iterdir()}
        self.assertEqual(files, {p.name: p.read_bytes() for p in other.iterdir()})
        self.assertEqual(set(files), {'bank.json', 'LICENSE.txt'} | {cue+'.pcm' for cue in CUES})
        self.assertEqual(before, {p.name: p.read_bytes() for p in self.source.iterdir()})
        self.assertEqual(files['LICENSE.txt'], before['LICENSE.txt'])
        manifest = json.loads(files['bank.json'])
        for i, r in enumerate(manifest['cues']):
            self.assertEqual(r['assetId'], f'a_000000005444{i+1:04x}')
            self.assertEqual(r['sha256'], hashlib.sha256(files[r['file']]).hexdigest())
        self.assertIn('authored-audio-bank', snapshot_audio(self.output/'bank.json', snapshot_file, strict_json))

    def test_generated_bank_is_exported_by_existing_full_offline_packer(self):
        import pack_coop_defense_offline as packer
        self.create()
        web = self.root / 'web'
        web.mkdir()
        source = Path(__file__).resolve().parents[1] / 'crates/orr_coop_defense_web/web'
        for name in (*packer.MODULES, *packer.EXTRA_MODULES, 'game.js', 'index.html', 'style.css'):
            target = web / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source / name, target)
        # Packaging fixture only; deliberately not executable WASM/art.
        (web/'game.wasm').write_bytes(b'not executable packaging fixture')
        (web/'assets').mkdir()
        for name, data in {'characters.json': b'{"heroes":[],"enemies":[]}',
                           'units.png': b'fixture heroes', 'enemies.png': b'fixture enemies'}.items():
            (web/'assets'/name).write_bytes(data)
        destination = self.root/'delivery.html'
        html = packer.package(web, output=destination, audio=self.output/'bank.json').read_text()
        self.assertIn(snapshot_audio(self.output/'bank.json', snapshot_file, strict_json), html)
        self.assertNotIn(str(self.source), html)
        self.assertNotIn(str(self.output), html)

    def test_missing_duplicate_unknown_and_empty_cues_publish_nothing(self):
        for inputs in [self.inputs[:-1], self.inputs+[self.inputs[0]],
                       ['unknown=x']+self.inputs[1:], ['attack-light=']+self.inputs[1:]]:
            with self.assertRaises(ValueError): self.create(inputs=inputs)
            self.assertFalse(self.output.exists())

    def test_pcm_contract_failures_publish_nothing(self):
        path = self.source/'attack-light.pcm'
        for data in [b'x', struct.pack('<IIh', 44100,1,1), struct.pack('<IIh',48000,2,1),
                     struct.pack('<IIh',48000,1,8193), struct.pack('<IIh',48000,1,-8193),
                     struct.pack('<II',48000,0), b'x'*(MAX_PCM+1)]:
            path.write_bytes(data)
            with self.assertRaises(ValueError): self.create()
            self.assertFalse(self.output.exists())

    def test_license_empty_nonutf8_and_oversize_fail_without_output(self):
        for data in [b' ', b'\xff', b'x'*(64*1024+1)]:
            self.license.write_bytes(data)
            with self.assertRaises(ValueError): self.create()
            self.assertFalse(self.output.exists())

    def test_existing_output_file_directory_and_symlink_are_never_overwritten(self):
        self.output.write_bytes(b'keep')
        with self.assertRaises(FileExistsError): self.create()
        self.assertEqual(self.output.read_bytes(), b'keep')
        self.output.unlink()
        self.output.mkdir()
        (self.output/'sentinel').write_bytes(b'keep')
        with self.assertRaises(FileExistsError): self.create()
        self.assertEqual((self.output/'sentinel').read_bytes(), b'keep')
        linked = self.root/'linked'
        linked.symlink_to(self.output, target_is_directory=True)
        with self.assertRaises(FileExistsError): self.create(output=linked)
        self.assertTrue(linked.is_symlink())

    def test_symlink_leaves_ancestors_and_output_parent_rejected(self):
        link = self.root/'source-link'
        link.symlink_to(self.source, target_is_directory=True)
        with self.assertRaises(OSError): self.create(inputs=[f'{CUES[0]}={link}/{CUES[0]}.pcm']+self.inputs[1:])
        with self.assertRaises(OSError): self.create(output=link/'created')
        leaf = self.source/'linked-license'
        leaf.symlink_to(self.license)
        with self.assertRaises(OSError): self.create(license=leaf)
        self.assertFalse(self.output.exists())
        self.assertFalse((self.source/'created').exists())

    def test_parent_traversal_rejected_without_output(self):
        with self.assertRaises(ValueError): self.create(output=self.source/'..'/'created')
        with self.assertRaises(ValueError): self.create(license=self.source/'..'/'source'/'LICENSE.txt')
        self.assertFalse(self.output.exists())

    def test_fifo_is_rejected_without_waiting_for_writer(self):
        fifo = self.source/'fifo'
        os.mkfifo(fifo)
        with self.assertRaises(ValueError): self.create(inputs=[f'{CUES[0]}={fifo}']+self.inputs[1:])
        self.assertFalse(self.output.exists())

    def test_write_failure_cleans_only_new_directory(self):
        def fail(_fd): raise OSError('injected owned-output failure')
        with patch('create_coop_defense_audio_bank.os.fsync', side_effect=fail):
            with self.assertRaisesRegex(OSError, 'injected'): self.create()
        self.assertFalse(self.output.exists())
        self.assertTrue(self.license.exists())
        self.create()
        self.assertTrue(self.output.exists())


if __name__ == '__main__':
    unittest.main()
