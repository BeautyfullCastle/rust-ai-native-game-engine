#!/usr/bin/env python3
"""Additive prerequisites only; the existing shared 29-case gate stays intact."""
import ast
import json
import sys
import os
import pathlib
import tempfile
import unittest
from unittest import mock

import static_model_extract_gate as gate
import native_assemble_cli_prerequisite as native


class Prerequisites(unittest.TestCase):
    def test_exact_build_pin_and_environment(self):
        with mock.patch.object(gate, 'run', return_value='json') as run, \
                mock.patch.object(gate, 'pin', return_value='/retained/assemble') as pin, \
                mock.patch.dict(os.environ, {'ORR_STATIC_MODEL_ASSEMBLE_BIN': '/stale'}):
            self.assertEqual(gate.build_assemble_regression_cli(), '/retained/assemble')
            run.assert_called_once_with('build-assemble-regression-cli', [
                'cargo', 'build', '--locked', '--release', '-p', 'orr_model',
                '--no-default-features', '--features', 'import', '--example',
                'static_model_assemble', '--message-format=json'])
            pin.assert_called_once_with('json', 'assemble-regression-cli', 'orr_model',
                                        'static_model_assemble', 'example', ['import'])
            self.assertEqual(os.environ['ORR_STATIC_MODEL_ASSEMBLE_BIN'], '/retained/assemble')

    def test_build_failure_does_not_pin_or_rebind(self):
        with mock.patch.object(gate, 'run', side_effect=RuntimeError('build failed')), \
                mock.patch.object(gate, 'pin') as pin, \
                mock.patch.dict(os.environ, {'ORR_STATIC_MODEL_ASSEMBLE_BIN': '/stale'}):
            with self.assertRaises(RuntimeError): gate.build_assemble_regression_cli()
            pin.assert_not_called()
            self.assertEqual(os.environ['ORR_STATIC_MODEL_ASSEMBLE_BIN'], '/stale')

    def test_pin_failure_does_not_rebind(self):
        with mock.patch.object(gate, 'run', return_value='json'), \
                mock.patch.object(gate, 'pin', side_effect=AssertionError('not fresh')), \
                mock.patch.dict(os.environ, {'ORR_STATIC_MODEL_ASSEMBLE_BIN': '/stale'}):
            with self.assertRaises(AssertionError): gate.build_assemble_regression_cli()
            self.assertEqual(os.environ['ORR_STATIC_MODEL_ASSEMBLE_BIN'], '/stale')

    def test_extract_retains_full_command_after_prerequisite(self):
        for filename, function, label, command in (
            ('static_model_extract_gate.py','cpu','model-import-regressions',
             "['cargo', 'test', *model, '--features', 'import']"),
            ('static_model_recolor_gate.py','model','model-full-regressions',
             "['cargo', 'test', *flags, '--features', 'import', '--tests', '--', '--test-threads=1']")):
            node = ast.parse(pathlib.Path(gate.__file__).with_name(filename).read_text())
            body = next(n for n in node.body if isinstance(n, ast.FunctionDef) and n.name == function)
            calls = sorted((n for n in ast.walk(body) if isinstance(n, ast.Call)), key=lambda n:n.lineno)
            prereq = next(n for n in calls if isinstance(n.func, ast.Name) and n.func.id == 'build_assemble_regression_cli')
            regression = next(n for n in calls if n.args and isinstance(n.args[0], ast.Constant) and n.args[0].value == label)
            self.assertLess(prereq.lineno, regression.lineno)
            self.assertEqual(ast.unparse(regression.args[1]), command)
        self.assertIn('test_assemble_cli_prerequisites.py', pathlib.Path(gate.__file__).read_text())

    def test_environment_path_spaces_and_unicode_are_one_value(self):
        path = pathlib.Path(tempfile.gettempdir()) / 'space 東京' / 'assemble.exe'
        self.assertEqual(native.environment_lines(path),
            'ORR_STATIC_MODEL_ASSEMBLE_BIN=' + str(path.resolve()) + '\nPYTHONDONTWRITEBYTECODE=1\n')

    def test_environment_rejects_newline_carriage_return_and_nul(self):
        for bad in ['evil\nNAME=value', 'evil\rNAME=value', 'evil\0name']:
            with self.assertRaises((AssertionError, ValueError)): native.environment_lines(bad)

    def test_native_prepare_then_verify_unique_labels_and_jobs(self):
        identity = ('a'*40, 'b'*40, {'source': {'sha256': 'c'*64}})
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            evidence = root / 'evidence'
            env = root / 'env'
            events = []
            saved = {key:getattr(gate,key) for key in ('repo','evidence','report','frozen','target_dir')}
            try:
                with mock.patch.object(native, 'source_identity', side_effect=lambda label: events.append(label) or identity), \
                        mock.patch.object(native, 'run', return_value='json') as run, \
                        mock.patch.object(gate, 'pin', return_value=str(root/'assemble')) as pin, \
                        mock.patch.dict(os.environ):
                    os.environ.pop('GITHUB_SHA', None)
                    native.prepare(root, evidence, env)
                    self.assertEqual(os.environ['CARGO_BUILD_JOBS'], '2')
                    self.assertTrue(env.exists())
                    native.verify(root, evidence)
                self.assertEqual(events, ['prepare-before','prepare-after','verify-after-workspace'])
                self.assertEqual(len(events),len(set(events)))
                self.assertTrue(json.loads((evidence/'gate-receipt.json').read_text())['post_workspace_identity_verified'])
                self.assertIn('--example',run.call_args.args[1])
                pin.assert_called_once_with('json','native-assemble','orr_model','static_model_assemble','example',['import'])
                with self.assertRaises(FileExistsError): gate.initialize(root,evidence,'cpu')
            finally:
                for key,value in saved.items(): setattr(gate,key,value)

    def test_native_failure_never_exposes_binary(self):
        with tempfile.TemporaryDirectory() as directory:
            env = pathlib.Path(directory) / 'env'
            with mock.patch.object(gate, 'initialize'), \
                    mock.patch.object(native, 'source_identity', side_effect=RuntimeError('source fence')), \
                    mock.patch.object(gate, 'record'), mock.patch.object(gate, 'report', {}), \
                    mock.patch.dict(os.environ):
                with self.assertRaises(RuntimeError): native.prepare(directory, directory, env)
            self.assertFalse(env.exists())

    def test_verify_rejects_changed_source_and_retained_binary(self):
        for change in ('source','binary'):
            with tempfile.TemporaryDirectory() as directory:
                root=pathlib.Path(directory)
                receipt={'status':'passed','checkout':str(root.resolve()),'head':'a'*40,
                         'tree':'b'*40,'source_sha256':{},'commands':[],'producers':{}}
                (root/'gate-receipt.json').write_text(json.dumps(receipt))
                saved={key:getattr(gate,key) for key in ('repo','evidence','report')}
                try:
                    with mock.patch.object(native,'source_identity',return_value=('changed','b'*40,{})), \
                            mock.patch.object(gate,'fence',side_effect=AssertionError('binary changed') if change=='binary' else None):
                        with self.assertRaises(AssertionError): native.verify(root,root)
                    self.assertEqual(json.loads((root/'gate-receipt.json').read_text())['status'],'failed')
                finally:
                    for key,value in saved.items():setattr(gate,key,value)

    def test_native_linux_arm64_x64_windows_dispatch_preserves_shared_runner(self):
        for platform in ('linux','linux','win32'):
            with mock.patch.object(native.sys,'platform',platform), \
                    mock.patch.object(gate,'run',return_value='ok') as run:
                self.assertEqual(native.run('label',['command'],seconds=5),'ok')
                run.assert_called_once_with('label',['command'],seconds=5)

    def test_macos_supervisor_bounded_positive_and_failure(self):
        # Cross-platform protocol fixture: macOS runtime remains hosted coverage.
        for code in (0, 3, "timeout", "output-cap"):
            with tempfile.TemporaryDirectory() as directory:
                saved={key:getattr(gate,key) for key in ('repo','evidence','report')}
                gate.repo=gate.evidence=pathlib.Path(directory)
                gate.report={'commands':[],'producers':{}}
                process=mock.Mock(pid=123,returncode=None)
                def launch(argv, **kwargs):
                    self.assertTrue(kwargs['start_new_session'])
                    self.assertEqual(kwargs['stdin'],native.subprocess.PIPE)
                    if isinstance(code, int): pathlib.Path(argv[-1]).write_text(json.dumps(code))
                    kwargs['stdout'].write(b'ready\n');kwargs['stdout'].flush()
                    return process
                try:
                    with mock.patch.object(native.sys,'platform','darwin'), \
                            mock.patch.object(native.subprocess,'Popen',side_effect=launch), \
                            mock.patch.object(native.os,'killpg',create=True) as kill, \
                            mock.patch.object(native.signal,'SIGKILL',9,create=True), \
                            mock.patch.object(gate,'LOG_CAP',1 if code=='output-cap' else gate.LOG_CAP):
                        if code==0:self.assertEqual(native.run('mac',['command']).strip(),'ready')
                        else:
                            with self.assertRaises(AssertionError):native.run('mac',['command'],seconds=0 if code=='timeout' else 5)
                    kill.assert_called_once_with(123,9)
                    process.poll.assert_not_called()
                    process.wait.assert_called_once_with(timeout=10)
                    process.stdin.close.assert_called_once()
                finally:
                    for key,value in saved.items():setattr(gate,key,value)


if __name__ == '__main__':
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(Prerequisites)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    if not result.wasSuccessful() or result.skipped or result.testsRun != 11:
        raise SystemExit(1)
    print('ASSEMBLE_CLI_PREREQUISITES: 11 passed; 0 failed; 0 skipped')
