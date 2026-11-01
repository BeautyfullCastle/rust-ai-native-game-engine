"""Bounded multi-role authoring tests using the real existing PNG/FPS validator."""
import contextlib
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from PIL import Image

import compose_coop_defense_animations as composer


class AnimationComposition(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'art').mkdir()
        Image.new('RGBA', (4, 2), (255, 0, 0, 255)).save(self.root / 'art/shared.png')
        self.definition = {'image': 'art/shared.png', 'pivot': [1, 2], 'clips': {
            'idle': {'frames': [[0, 0, 2, 2], [2, 0, 2, 2]], 'fps': 10, 'loop': True}}}
        self.paths = [self.write('hero.json', {'unit:0': self.definition}),
                      self.write('enemy.json', {'enemy:0': self.definition})]
        self.output = self.root / 'composed.json'

    def write(self, name, value):
        path = self.root / name
        path.write_text(json.dumps(value), encoding='utf-8')
        return path

    def compose(self, paths=None, output=None):
        return composer.compose_animations(self.root, output or self.output,
                                           self.paths if paths is None else paths)

    def test_real_png_sampler_composition_deterministic_exact_source(self):
        before = {p: p.read_bytes() for p in self.paths + [self.root / 'art/shared.png']}
        receipt = self.compose()
        reverse = self.root / 'reverse.json'
        self.compose(list(reversed(self.paths)), reverse)
        self.assertEqual(reverse.read_bytes(), self.output.read_bytes())
        self.assertEqual(json.loads(self.output.read_bytes()),
                         {'unit:0': self.definition, 'enemy:0': self.definition})
        self.assertEqual(receipt['roles'], ['enemy:0', 'unit:0'])
        self.assertEqual(len(receipt['images']), 1)
        self.assertEqual(receipt['manifestSha256'], hashlib.sha256(self.output.read_bytes()).hexdigest())
        self.assertEqual(receipt['sources'], [{'bytes': len(before[p]), 'sha256': hashlib.sha256(before[p]).hexdigest()} for p in self.paths])
        self.assertTrue(all(p.read_bytes() == value for p, value in before.items()))

    def test_duplicate_roles_and_keys_fail_closed(self):
        for paths in [[self.paths[0], self.paths[0]],
                      [self.paths[0], self.write('same.json', {'unit:0': self.definition})]]:
            with self.assertRaisesRegex(ValueError, 'Duplicate animation role'):
                self.compose(paths)
        duplicate = self.root / 'duplicate.json'
        duplicate.write_text('{"unit:0":{},"unit:0":{}}')
        with self.assertRaisesRegex(ValueError, 'Duplicate JSON key'):
            self.compose([duplicate])
        self.assertFalse(self.output.exists())

    def test_count_limits_before_reads_and_all_roles(self):
        for paths in [[], [self.paths[0]] * 12]:
            with patch.object(composer, 'snapshot_file') as read:
                with self.assertRaisesRegex(ValueError, '1..11'):
                    self.compose(paths)
                read.assert_not_called()
        roles = [*(f'unit:{i}' for i in range(6)), *(f'enemy:{i}' for i in range(5))]
        paths = [self.write(f'role-{i}.json', {role: self.definition}) for i, role in enumerate(roles)]
        self.assertEqual(self.compose(paths)['roles'], sorted(roles))

    def test_schema_and_paths_rejected_before_image_reads(self):
        for image in ['../hero.png', '/hero.png', 'art//hero.png', 'art\\hero.png', 'https://x/a.png']:
            bad = self.write('bad.json', {'unit:0': {**self.definition, 'image': image}})
            with patch.object(composer, 'snapshot_file', wraps=composer.snapshot_file) as read:
                with self.assertRaises(ValueError): self.compose([bad])
                self.assertEqual(read.call_count, 1)
        bad = self.write('bad.json', {'unit:6': self.definition})
        with self.assertRaises(ValueError): self.compose([bad])
        self.assertFalse(self.output.exists())

    def test_input_and_composed_size_limits(self):
        large = self.root / 'large.json'
        large.write_bytes(b' ' * (composer.MAX_ANIMATION + 1))
        with self.assertRaisesRegex(ValueError, 'byte limit'): self.compose([large])
        definition = {**self.definition, 'clips': {name: {'frames': [[0, 0, 1, 1]] * 256}
            for name in ['idle', 'move', 'attack', 'hit', 'death']}}
        paths = [self.write(f'large-{i}.json', {f'unit:{i}': definition}) for i in range(6)]
        self.assertTrue(all(p.stat().st_size < composer.MAX_ANIMATION for p in paths))
        with self.assertRaisesRegex(ValueError, 'Composed.*64 KiB'): self.compose(paths)
        self.assertFalse(self.output.exists())

    def test_invalid_png_bounds_and_missing_images_do_not_publish(self):
        bad = self.write('bad.json', {'unit:0': {**self.definition, 'clips': {
            'idle': {'frames': [[3, 0, 2, 2]]}}}})
        with self.assertRaisesRegex(ValueError, 'outside PNG'): self.compose([bad])
        (self.root / 'art/shared.png').write_bytes(b'not PNG')
        with self.assertRaises(ValueError): self.compose()
        (self.root / 'art/shared.png').unlink()
        with self.assertRaises(FileNotFoundError): self.compose()
        self.assertFalse(self.output.exists())

    def test_total_image_limit_is_enforced_before_validator(self):
        second = self.write('second.json', {'enemy:0': {**self.definition, 'image': 'art/other.png'}})
        (self.root / 'art/other.png').write_bytes((self.root / 'art/shared.png').read_bytes())
        size = (self.root / 'art/shared.png').stat().st_size
        with patch.object(composer, 'MAX_IMAGES_TOTAL', size), patch.object(composer, 'validate_animations') as validate:
            with self.assertRaisesRegex(ValueError, 'total byte limit'):
                self.compose([self.paths[0], second])
            validate.assert_not_called()
        self.assertFalse(self.output.exists())

    def test_symlink_inputs_assets_and_output_rejected(self):
        linked = self.root / 'linked.json'; linked.symlink_to(self.paths[0])
        with self.assertRaises(OSError): self.compose([linked])
        (self.root / 'linked').symlink_to(self.root / 'art', target_is_directory=True)
        bad = self.write('bad.json', {'unit:0': {**self.definition, 'image': 'linked/shared.png'}})
        with self.assertRaises(OSError): self.compose([bad])
        self.output.symlink_to(self.root / 'absent')
        with self.assertRaises(FileExistsError): self.compose()
        self.assertTrue(self.output.is_symlink())
        self.assertFalse((self.root / 'absent').exists())

    def test_no_clobber_and_validation_failure_preserve_destination(self):
        self.output.write_bytes(b'keep')
        with self.assertRaises(FileExistsError): self.compose()
        with patch.object(composer, 'validate_animations', side_effect=ValueError('sampler failure')):
            with self.assertRaisesRegex(ValueError, 'sampler failure'): self.compose()
        self.assertEqual(self.output.read_bytes(), b'keep')
        self.assertEqual(list(self.root.glob('.orr-offline-*')), [])

    def test_cli_success_and_duplicate_failure(self):
        args = ['--assets-dir', str(self.root), '--output', str(self.output)]
        for path in self.paths: args += ['--manifest', str(path)]
        with contextlib.redirect_stdout(io.StringIO()) as out:
            self.assertEqual(composer.main(args), 0)
        self.assertEqual(json.loads(out.getvalue())['roles'], ['enemy:0', 'unit:0'])
        args[3] = str(self.root / 'failed.json')
        with contextlib.redirect_stderr(io.StringIO()) as err:
            self.assertEqual(composer.main(args + ['--manifest', str(self.paths[0])]), 1)
        self.assertIn('Duplicate animation role', err.getvalue())
        self.assertFalse((self.root / 'failed.json').exists())


if __name__ == '__main__':
    unittest.main()
