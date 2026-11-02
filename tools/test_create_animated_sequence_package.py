"""Changed animation frontend contracts; mocked shape is not animation admission."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import create_animated_sequence_package as author


class SequenceFrontend(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.recipe = self.root / 'recipe.json'
        self.recipe.write_bytes(b'{}')
        self.license = self.root / 'LICENSE.txt'
        self.license.write_bytes(b'User supplied fixture license\n')
        self.sequencer = self.root / 'sequencer'
        self.package_source = self.root / 'package-source'
        for tool in [self.sequencer, self.package_source]:
            tool.write_bytes(b'placeholder: never executed by mock tests')
            tool.chmod(0o700)
        self.output = self.root / 'package'
        self.stages = []

    def create(self, **changes):
        args = dict(recipe=self.recipe, asset_id='animated/sequence', license_path=self.license,
                    name='animation-fixture', version='1.0.0', engine='0.0.1', output=self.output,
                    sequencer=self.sequencer, package_source=self.package_source)
        args.update(changes)
        return author.create_package(**args)

    def shape(self, arguments):
        # Shape-only output to inspect orchestration, NOT a decoded animation.
        if arguments[0] == str(self.sequencer):
            target = Path(arguments[arguments.index('--output') + 1])
            self.stages.append(target.parent)
            target.write_bytes(b'not a valid animation; mock fixture')
        else:
            target = Path(arguments[arguments.index('--source') + 1])
            (target / 'orr.package.json').write_bytes(b'mock manifest')

    def clean(self):
        self.assertFalse(os.path.lexists(self.output))
        for stage in self.stages:
            self.assertFalse(stage.exists())

    def test_metadata_recipe_and_license_refuse_before_native(self):
        for key in ['asset_id', 'name', 'version', 'engine']:
            for value in ['', '\x00bad', 'x' * 4097, None]:
                with self.subTest(key=key, value=str(value)[:20]), patch.object(author, 'run_tool') as run:
                    with self.assertRaises(ValueError): self.create(**{key: value})
                    run.assert_not_called()
                    self.clean()
        for path, limit in [(self.recipe, author.MAX_RECIPE), (self.license, author.MAX_LICENSE)]:
            original = path.read_bytes()
            path.write_bytes(b'x' * (limit + 1))
            with patch.object(author, 'run_tool') as run:
                with self.assertRaises((ValueError, OSError)): self.create()
                run.assert_not_called()
            path.write_bytes(original)
        for value in [b'', b' ', b'\xff']:
            self.license.write_bytes(value)
            with patch.object(author, 'run_tool') as run:
                with self.assertRaises(ValueError): self.create()
                run.assert_not_called()
            self.clean()

    def test_exact_native_arguments_capabilities_files_and_no_auto_install_mock_shape(self):
        with patch.object(author, 'run_tool', side_effect=self.shape) as run:
            self.assertEqual(self.create(), self.output)
        self.assertEqual(run.call_count, 2, 'no implicit install or extra native operation')
        stage = self.stages[0]
        self.assertEqual(run.call_args_list[0].args[0], [str(self.sequencer), '--recipe', str(self.recipe),
            '--asset-id', 'animated/sequence', '--output', str(stage / 'model.animated.json')])
        self.assertEqual(run.call_args_list[1].args[0], [str(self.package_source), '--source', str(stage),
            '--name', 'animation-fixture', '--version', '1.0.0', '--engine', '0.0.1',
            '--capability', 'models', '--capability', 'animation',
            '--file', 'model.animated.json', '--file', 'LICENSE.txt'])
        self.assertEqual({p.name for p in self.output.iterdir()}, {'model.animated.json', 'orr.package.json', 'LICENSE.txt'})
        self.assertEqual((self.output / 'LICENSE.txt').read_bytes(), self.license.read_bytes())
        self.assertEqual((self.output / 'model.animated.json').read_bytes(), b'not a valid animation; mock fixture')
        self.assertFalse(stage.exists())
        self.assertFalse((self.root / '.orr').exists())

    def test_both_native_failure_stages_and_timeout_leave_no_publication(self):
        for failed_step in [0, 1]:
            for failure in [ValueError('native rejected'), subprocess.TimeoutExpired(['native'], 60)]:
                count = 0
                def native(arguments):
                    nonlocal count
                    step = count
                    count += 1
                    # Record private stage even if the sequencer itself fails.
                    if step == 0:
                        self.stages.append(Path(arguments[arguments.index('--output') + 1]).parent)
                    if step == failed_step:
                        raise failure
                    self.shape(arguments)
                with patch.object(author, 'run_tool', side_effect=native):
                    with self.assertRaises(type(failure)): self.create()
                self.clean()

    def test_changed_recipe_license_or_unexpected_stage_file_reject(self):
        for mode in ['recipe', 'license', 'extra']:
            self.recipe.write_bytes(b'{}')
            def native(arguments):
                self.shape(arguments)
                if '--output' in arguments and mode == 'recipe':
                    self.recipe.write_bytes(b'{"changed":true}')
                if '--source' in arguments:
                    stage = Path(arguments[arguments.index('--source') + 1])
                    if mode == 'license': (stage / 'LICENSE.txt').write_bytes(b'changed')
                    if mode == 'extra': (stage / 'unrequested.bin').write_bytes(b'extra')
            with patch.object(author, 'run_tool', side_effect=native):
                with self.assertRaises(ValueError): self.create()
            self.clean()

    def test_existing_destination_refuses_before_native_and_preserves_bytes(self):
        self.output.mkdir()
        (self.output / 'keep').write_bytes(b'prior user package')
        with patch.object(author, 'run_tool') as run:
            with self.assertRaises(FileExistsError): self.create()
            run.assert_not_called()
        self.assertEqual({p.name: p.read_bytes() for p in self.output.iterdir()}, {'keep': b'prior user package'})

    @unittest.skipUnless(os.environ.get('ORR_SEQUENCE_BIN'), 'actual sequencer negative requires ORR_SEQUENCE_BIN')
    def test_actual_sequencer_rejects_malformed_bounded_recipe(self):
        self.recipe.write_bytes(b'{malformed recipe')
        with self.assertRaises(ValueError):
            self.create(sequencer=Path(os.environ['ORR_SEQUENCE_BIN']))
        self.clean()


if __name__ == '__main__':
    unittest.main()
