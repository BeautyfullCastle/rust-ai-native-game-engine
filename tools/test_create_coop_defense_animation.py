"""Focused admission/publication tests; no Rust/WASM or full test campaign."""
import contextlib
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from PIL import Image

import create_coop_defense_animation as creator


class AnimationCreation(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'art').mkdir()
        self.image = self.root / 'art/hero.png'
        Image.new('RGBA', (4, 2), (255, 0, 0, 255)).save(self.image)
        self.original = self.image.read_bytes()
        self.output = self.root / 'animations.json'
        self.frames = [('idle', 0, 0, 2, 2), ('idle', 2, 0, 2, 2)]

    def create(self, **kwargs):
        args = dict(assets_dir=self.root, output=self.output, role='unit:0',
                    image='art/hero.png', frames=self.frames)
        args.update(kwargs)
        return creator.create_animation(**args)

    def test_real_png_and_sampler_admission_exact_bytes(self):
        receipt = self.create(pivot=[1, 2], clips=[('idle', 12, 'loop')])
        self.assertEqual(receipt['manifestSha256'], hashlib.sha256(self.output.read_bytes()).hexdigest())
        self.assertEqual(receipt['images'][0]['sha256'], hashlib.sha256(self.original).hexdigest())
        self.assertEqual(self.image.read_bytes(), self.original)
        spec = json.loads(self.output.read_bytes())['unit:0']
        self.assertEqual(spec['pivot'], [1, 2])
        self.assertEqual(spec['clips']['idle']['frames'], [[0, 0, 2, 2], [2, 0, 2, 2]])

    def test_defaults_and_all_roles_clips_boundaries(self):
        for role in [*(f'unit:{i}' for i in range(6)), *(f'enemy:{i}' for i in range(5))]:
            frames = [(clip, 0, 0, 2, 2) for clip in ['idle', 'move', 'attack', 'hit', 'death']]
            spec = json.loads(creator.animation_text(role, 'art/hero.png', frames))[role]
            self.assertNotIn('pivot', spec)
            for clip in spec['clips'].values():
                self.assertEqual(set(clip), {'frames'})
        for fps in [1, 120, 12.5]:
            for mode in ['loop', 'once']:
                text = creator.animation_text('enemy:4', 'art/hero.png', self.frames,
                                              [('idle', fps, mode)], [-4096, 4096])
                self.assertEqual(json.loads(text)['enemy:4']['clips']['idle']['loop'], mode == 'loop')

    def test_invalid_schema_precedes_image_lookup(self):
        cases = [dict(role='unit:6'), dict(role='enemy:5'), dict(role='unit:00'),
                 dict(frames=[]), dict(frames=[('jump', 0, 0, 1, 1)]),
                 dict(frames=self.frames * 129), dict(pivot=[float('nan'), 1]),
                 dict(pivot=[4097, 1]), dict(frames=[('idle', -1, 0, 1, 1)]),
                 dict(frames=[('idle', 0, 0, 0, 1)])]
        for fps in [0, 121, float('nan'), float('inf'), True]:
            cases.append(dict(clips=[('idle', fps, 'loop')]))
        cases += [dict(clips=[('move', 10, 'loop')]), dict(clips=[('idle', 10, 'bad')]),
                  dict(clips=[('idle', 10, 'loop'), ('idle', 12, 'once')])]
        for case in cases:
            with self.subTest(case=case), patch.object(creator, 'snapshot_file') as read:
                with self.assertRaises(ValueError):
                    self.create(**case)
                read.assert_not_called()
                self.assertFalse(self.output.exists())

    def test_bad_paths_rejected_before_lookup(self):
        for image in ['../hero.png', '/hero.png', 'art/../hero.png', 'art//hero.png',
                      'art\\hero.png', './hero.png', 'https://host/a.png', 'hero.jpg']:
            with self.subTest(image=image), patch.object(creator, 'snapshot_file') as read:
                with self.assertRaises(ValueError):
                    self.create(image=image)
                read.assert_not_called()

    def test_bounds_bad_png_and_missing_image_no_output(self):
        for frames in [[('idle', 3, 0, 2, 2)], [('idle', 0, 1, 2, 2)]]:
            with self.assertRaisesRegex(ValueError, 'outside PNG'):
                self.create(frames=frames)
        with self.assertRaises(FileNotFoundError):
            self.create(image='missing.png')
        self.image.write_bytes(b'not a png')
        with self.assertRaises(ValueError):
            self.create()
        self.assertFalse(self.output.exists())

    def test_symlink_file_directory_and_output_rejected(self):
        (self.root / 'linked.png').symlink_to(self.image)
        (self.root / 'linked').symlink_to(self.root / 'art', target_is_directory=True)
        for image in ['linked.png', 'linked/hero.png']:
            with self.assertRaises(OSError):
                self.create(image=image)
        self.output.symlink_to(self.root / 'absent')
        with self.assertRaises(FileExistsError):
            self.create()
        self.assertTrue(self.output.is_symlink())
        self.assertFalse((self.root / 'absent').exists())

    def test_no_clobber_and_failed_admission(self):
        with patch.object(creator, 'validate_animations', side_effect=ValueError('reject')):
            with self.assertRaises(ValueError):
                self.create()
        self.assertFalse(self.output.exists())
        self.output.write_bytes(b'keep existing')
        with self.assertRaises(FileExistsError):
            self.create()
        self.assertEqual(self.output.read_bytes(), b'keep existing')
        self.assertEqual(self.image.read_bytes(), self.original)
        self.assertEqual(list(self.root.glob('.orr-offline-*')), [])

    def test_snapshot_and_publication_use_exact_admitted_bytes(self):
        def admission(data, images):
            self.assertEqual(images, {'art/hero.png': self.original})
            self.image.write_bytes(b'changed after snapshot')
            self.admitted = data
            return {'snapshot': True}
        with patch.object(creator, 'validate_animations', side_effect=admission):
            self.assertEqual(self.create(), {'snapshot': True})
        self.assertEqual(self.output.read_bytes(), self.admitted)

    def test_cli_success_and_invalid_number(self):
        argv = ['--assets-dir', str(self.root), '--output', str(self.output),
                '--role', 'unit:0', '--image', 'art/hero.png',
                '--frame', 'idle', '0', '0', '2', '2',
                '--frame', 'idle', '2', '0', '2', '2',
                '--clip', 'idle', '10', 'once', '--pivot', '1', '2']
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(creator.main(argv), 0)
        self.assertFalse(json.loads(self.output.read_bytes())['unit:0']['clips']['idle']['loop'])
        before = self.output.read_bytes()
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(creator.main(argv + ['--frame', 'idle', 'no', '0', '1', '1']), 1)
        self.assertEqual(self.output.read_bytes(), before)


if __name__ == '__main__':
    unittest.main()
