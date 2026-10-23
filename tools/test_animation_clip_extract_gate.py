#!/usr/bin/env python3
"""Exercise the actual inline clip CPU prerequisite without running Cargo."""
import ast
import hashlib
import json
import os
import pathlib
import shutil
import sys
import tempfile
import textwrap
import unittest
import urllib.parse
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / '.github/workflows/determinism.yml'


def cpu_source():
    text = WORKFLOW.read_text(encoding='utf-8')
    return textwrap.dedent(text.split("python -B - <<'PYCLIPCPU'\n", 1)[1]
                          .split('\n          PYCLIPCPU', 1)[0])


class Prerequisites(unittest.TestCase):
    def setUp(self):
        self.tree = ast.parse(cpu_source())
        functions = [node for node in self.tree.body if isinstance(node, ast.FunctionDef)
                     and node.name in {'executable', 'build_regression_cli'}]
        self.ns = dict(pathlib=pathlib, json=json, urllib=urllib, os=os, shutil=shutil,
                       digest=lambda path: hashlib.sha256(path.read_bytes()).hexdigest(),
                       record=lambda: None, report={}, base=['--locked', '--release', '-p', 'orr_model'])
        exec(compile(ast.Module(body=functions, type_ignores=[]), str(WORKFLOW), 'exec'), self.ns)

    def test_exact_build_pin_and_environment(self):
        run = mock.Mock(return_value='producer-json')
        pin = mock.Mock(return_value='/retained/assemble')
        self.ns.update(run=run, executable=pin)
        with mock.patch.dict(os.environ, {'ORR_STATIC_MODEL_ASSEMBLE_BIN': '/stale'}):
            self.assertEqual(self.ns['build_regression_cli'](), '/retained/assemble')
            run.assert_called_once_with('assemble-regression-build', [
                'cargo', 'build', '--locked', '--release', '-p', 'orr_model',
                '--features', 'animation,import', '--example', 'static_model_assemble',
                '--message-format=json'])
            pin.assert_called_once_with('producer-json', 'assemble-regression-cli', 'orr_model',
                                        'example', ['animation', 'default', 'import'], 64 * 1024**2,
                                        target_name='static_model_assemble')
            self.assertEqual(os.environ['ORR_STATIC_MODEL_ASSEMBLE_BIN'], '/retained/assemble')

    def test_build_failure_does_not_pin_or_rebind(self):
        pin = mock.Mock()
        self.ns.update(run=mock.Mock(side_effect=RuntimeError('build failed')), executable=pin)
        with mock.patch.dict(os.environ, {'ORR_STATIC_MODEL_ASSEMBLE_BIN': '/stale'}):
            with self.assertRaises(RuntimeError):
                self.ns['build_regression_cli']()
            pin.assert_not_called()
            self.assertEqual(os.environ['ORR_STATIC_MODEL_ASSEMBLE_BIN'], '/stale')

    def test_pin_failure_does_not_rebind(self):
        self.ns.update(run=mock.Mock(return_value='bad-json'),
                       executable=mock.Mock(side_effect=AssertionError('wrong producer')))
        with mock.patch.dict(os.environ, {'ORR_STATIC_MODEL_ASSEMBLE_BIN': '/stale'}):
            with self.assertRaises(AssertionError):
                self.ns['build_regression_cli']()
            self.assertEqual(os.environ['ORR_STATIC_MODEL_ASSEMBLE_BIN'], '/stale')

    def test_full_regression_command_and_prerequisite_order_are_preserved(self):
        lane = next(node for node in self.tree.body if isinstance(node, ast.Try))
        calls = [node.value for node in lane.body if isinstance(node, ast.Expr)
                 and isinstance(node.value, ast.Call)]
        regression = next(call for call in calls if call.args and isinstance(call.args[0], ast.Constant)
                          and call.args[0].value == 'model-regressions')
        prerequisite = calls[calls.index(regression) - 1]
        self.assertEqual(ast.unparse(prerequisite), 'build_regression_cli()')
        self.assertEqual(ast.unparse(regression.args[1]),
                         "['cargo', 'test', *base, '--features', 'animation,import']")
        text = cpu_source()
        self.assertIn("'CLIP_EXTRACT_PREREQUISITES: 8 passed; 0 failed; 0 skipped'", text)
        self.assertIn("run('assemble-prerequisite-fixtures'", text)

    def producer(self, root, target):
        crate = root / 'crates/orr_model'
        crate.mkdir(parents=True, exist_ok=True)
        original = root / (target + '.bin')
        original.write_bytes(b'fresh producer bytes')
        return {'reason': 'compiler-artifact', 'manifest_path': str(crate / 'Cargo.toml'),
                'package_id': 'path+' + crate.as_uri() + '#orr_model@0.0.1',
                'target': {'name': target, 'kind': ['example'], 'crate_types': ['bin'],
                           'src_path': str(crate / 'examples' / (target + '.rs'))},
                'profile': {'test': False, 'opt_level': '3'},
                'features': ['animation', 'default', 'import'], 'executable': str(original)}

    def pin(self, root, producer, target):
        evidence = root / 'evidence'
        evidence.mkdir(exist_ok=True)
        self.ns.update(repo=root, evidence=evidence)
        return self.ns['executable'](json.dumps(producer), 'retained', 'orr_model', 'example',
                                      ['animation', 'default', 'import'], 64 * 1024**2,
                                      target_name=target)

    def test_both_exact_target_producers_are_retained_and_hashed(self):
        for target in ['animation_clip_extract', 'static_model_assemble']:
            with self.subTest(target=target), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory).resolve()
                producer = self.producer(root, target)
                retained = pathlib.Path(self.pin(root, producer, target))
                self.assertEqual(retained.read_bytes(), pathlib.Path(producer['executable']).read_bytes())
                self.assertEqual(self.ns['report']['retained']['producer'], producer)
                self.assertEqual(self.ns['report']['retained']['sha256'], self.ns['digest'](retained))

    def test_wrong_or_arbitrary_target_cannot_be_pinned(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory).resolve()
            producer = self.producer(root, 'animation_clip_extract')
            with self.assertRaises(AssertionError):
                self.pin(root, producer, 'static_model_assemble')
            with self.assertRaises(AssertionError):
                self.pin(root, producer, '../arbitrary')

    def test_wrong_source_features_package_profile_or_kind_rejected(self):
        for change in ['source', 'features', 'manifest', 'package', 'profile', 'kind']:
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory).resolve()
                producer = self.producer(root, 'static_model_assemble')
                if change == 'source': producer['target']['src_path'] = str(root / 'wrong.rs')
                if change == 'features': producer['features'] = ['import']
                if change == 'manifest': producer['manifest_path'] = str(root / 'Cargo.toml')
                if change == 'package': producer['package_id'] = 'registry+untrusted'
                if change == 'profile': producer['profile']['test'] = True
                if change == 'kind': producer['target']['kind'] = ['test']
                with self.assertRaises(AssertionError):
                    self.pin(root, producer, 'static_model_assemble')

    def test_source_fence_includes_prerequisite_and_original_inputs(self):
        node = next(node for node in self.tree.body if isinstance(node, ast.Assign)
                    and any(isinstance(target, ast.Name) and target.id == 'paths' for target in node.targets))
        paths = ast.literal_eval(node.value)
        for path in ['.github/workflows/determinism.yml', 'tools/test_animation_clip_extract_gate.py',
                     'crates/orr_model/examples/static_model_assemble.rs',
                     'crates/orr_model/examples/static_model_assemble/authoring.rs',
                     'crates/orr_model/tests/static_model_assemble.rs',
                     'crates/orr_model/examples/animation_clip_extract.rs',
                     'crates/orr_model/tests/animation_clip_extract.rs']:
            self.assertIn(path, paths)


if __name__ == '__main__':
    if not __debug__:
        raise SystemExit('nonoptimized Python required')
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Prerequisites))
    if not result.wasSuccessful() or result.skipped or result.testsRun != 8:
        raise SystemExit(1)
    print('CLIP_EXTRACT_PREREQUISITES: 8 passed; 0 failed; 0 skipped', flush=True)
