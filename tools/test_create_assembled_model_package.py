"""Boundary/transaction tests. Mock stages are not model admission evidence."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import create_assembled_model_package as author


class Boundaries(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.recipe = self.root / 'recipe.json'
        self.recipe.write_bytes(b'{}')
        self.license = self.root / 'LICENSE.txt'
        self.license.write_bytes(b'User supplied fixture license; no rights inference.\n')
        # Never executed: placeholders only exercise explicit executable-path checks.
        self.tool = self.root / 'native-placeholder'
        self.tool.write_bytes(b'not an executable implementation')
        self.tool.chmod(0o700)
        self.output = self.root / 'package'
        self.sentinel = self.root / 'existing.keep'
        self.sentinel.write_bytes(b'preserve')
        self.stages = []

    def create(self, **changes):
        args = dict(recipe=self.recipe, asset_id='assembled/model', license_path=self.license,
                    name='assembled-fixture', version='1.0.0', engine='0.0.1', output=self.output,
                    assembler=self.tool, package_source=self.tool)
        args.update(changes)
        return author.create_package(**args)

    def assert_clean(self):
        self.assertFalse(os.path.lexists(self.output))
        self.assertEqual(self.sentinel.read_bytes(), b'preserve')
        for stage in self.stages:
            self.assertFalse(stage.exists(), 'private native staging directory must be cleaned')

    def reject_before_native(self, **changes):
        with patch.object(author, 'run_tool') as run:
            with self.assertRaises((OSError, ValueError, RuntimeError)):
                self.create(**changes)
            run.assert_not_called()
        self.assert_clean()

    def fixture_stage(self, arguments):
        # Deliberately shape-only mock output, never claimed to be a valid model
        # or a verified package. Native success/install has separate Rust tests.
        if '--output' in arguments:
            model = Path(arguments[arguments.index('--output') + 1])
            self.stages.append(model.parent)
            model.write_bytes(b'fixture model bytes')
        else:
            stage = Path(arguments[arguments.index('--source') + 1])
            (stage / 'orr.package.json').write_bytes(b'fixture manifest bytes')

    def test_missing_empty_nonutf8_oversize_license_and_recipe_bounds(self):
        self.license.unlink()
        self.reject_before_native()
        for data in [b' ', b'\xff', b'x' * (author.MAX_LICENSE + 1)]:
            self.license.write_bytes(data)
            self.reject_before_native()
        self.license.write_bytes(b'license')
        self.recipe.write_bytes(b'x' * (author.MAX_RECIPE + 1))
        self.reject_before_native()
        self.recipe.unlink()
        self.reject_before_native()

    def test_existing_output_file_directory_and_dangling_link_preserved(self):
        self.output.write_bytes(b'existing file')
        with self.assertRaises(FileExistsError): self.create()
        self.assertEqual(self.output.read_bytes(), b'existing file')
        self.output.unlink()
        self.output.mkdir()
        (self.output / 'keep').write_bytes(b'existing directory')
        with self.assertRaises(FileExistsError): self.create()
        self.assertEqual((self.output / 'keep').read_bytes(), b'existing directory')
        (self.output / 'keep').unlink()
        self.output.rmdir()
        self.output.symlink_to(self.root / 'absent')
        with self.assertRaises(FileExistsError): self.create()
        self.assertTrue(self.output.is_symlink())

    def test_symlink_recipe_license_parent_and_native_paths_rejected(self):
        for argument, target in [('recipe', self.recipe), ('license_path', self.license),
                                 ('assembler', self.tool), ('package_source', self.tool)]:
            link = self.root / ('link-' + argument)
            link.symlink_to(target)
            self.reject_before_native(**{argument: link})
        directory = self.root / 'real'
        directory.mkdir()
        alias = self.root / 'alias'
        alias.symlink_to(directory, target_is_directory=True)
        self.reject_before_native(output=alias / 'output')
        self.assertEqual(list(directory.iterdir()), [])
        self.tool.chmod(0o600)
        self.reject_before_native()

    def test_internal_store_and_parent_traversal_are_rejected_for_all_paths(self):
        for argument in ['recipe', 'license_path', 'output', 'assembler', 'package_source']:
            for path in [self.root / '.orr' / 'candidate', self.root / 'x' / '..' / 'candidate']:
                self.reject_before_native(**{argument: path})

    def test_timeout_and_native_rejection_never_publish(self):
        for failure in [subprocess.TimeoutExpired(['native'], 60), ValueError('native rejected input')]:
            with patch.object(author, 'run_tool', side_effect=failure):
                with self.assertRaises(type(failure)): self.create()
            self.assert_clean()
        with patch.object(author.subprocess, 'run', return_value=subprocess.CompletedProcess(['native'], 7)) as run:
            with self.assertRaisesRegex(ValueError, 'exit 7'): author.run_tool([str(self.tool)])
            self.assertFalse(run.call_args.kwargs['shell'])
            self.assertEqual(run.call_args.kwargs['timeout'], author.TIMEOUT)

    def test_publication_write_failure_cleans_only_new_objects(self):
        with patch.object(author, 'run_tool', side_effect=self.fixture_stage), \
             patch.object(author.os, 'fsync', side_effect=OSError('injected write durability failure')):
            with self.assertRaisesRegex(OSError, 'injected'): self.create()
        self.assert_clean()

    def test_native_output_extra_file_or_license_mutation_rejected_and_cleaned(self):
        for mode in ['extra', 'license']:
            def native(arguments):
                self.fixture_stage(arguments)
                if '--source' in arguments:
                    stage = Path(arguments[arguments.index('--source') + 1])
                    (stage / ('extra' if mode == 'extra' else 'LICENSE.txt')).write_bytes(b'changed')
            with patch.object(author, 'run_tool', side_effect=native):
                with self.assertRaises(ValueError): self.create()
            self.assert_clean()

    def test_recipe_mutation_between_native_steps_is_rejected(self):
        def native(arguments):
            self.fixture_stage(arguments)
            self.recipe.write_bytes(b'{"changed":true}')
        with patch.object(author, 'run_tool', side_effect=native) as run:
            with self.assertRaisesRegex(ValueError, 'Recipe changed'): self.create()
            self.assertEqual(run.call_count, 1)
        self.assert_clean()

    @unittest.skipUnless(os.environ.get('ORR_ASSEMBLE_BIN'), 'actual assembler negative requires ORR_ASSEMBLE_BIN')
    def test_actual_native_assembler_rejects_malformed_bounded_recipe(self):
        native = Path(os.environ['ORR_ASSEMBLE_BIN'])
        self.recipe.write_bytes(b'{invalid recipe')
        # Package-source path is admitted but never executed after assembler fails.
        with self.assertRaises(ValueError): self.create(assembler=native)
        self.assert_clean()


if __name__ == '__main__':
    unittest.main()
