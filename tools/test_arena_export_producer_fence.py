#!/usr/bin/env python3
"""Real runner boundary regressions; synthetic producers, never acceptance."""
from collections import Counter
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

import arena_editor_export_gate as gate


class Tests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.saved = gate.runner.repo, gate.runner.evidence, gate.runner.report
        self.addCleanup(self.restore)
        gate.runner.repo = self.root
        gate.runner.evidence = self.root / 'evidence'
        gate.runner.evidence.mkdir()
        gate.runner.report = {'commands': [], 'producers': {}, 'ui_profile': True}
        self.paths = []
        for name in ('selected', 'not-executed'):
            original = self.root / name
            copied = gate.runner.evidence / name
            for path in (original, copied):
                path.write_bytes(b'original fixture bytes')
                path.chmod(0o755)
            gate.runner.report['producers'][name] = {
                'producer': {'executable': str(original)}, 'file': copied.name,
                'bytes': original.stat().st_size, 'sha256': gate.runner.digest(original)}
            self.paths.extend((original, copied))
        self.source = mock.patch.object(gate.fence, 'source_fence')
        self.source_check = self.source.start()
        self.addCleanup(self.source.stop)

    def restore(self):
        gate.runner.repo, gate.runner.evidence, gate.runner.report = self.saved

    def run_command(self, code='print("fixture")'):
        return gate.run('command', [sys.executable, '-B', '-c', code], seconds=30)

    def mutation(self, path, exit_code=0):
        # Same length and restored mtime must still be rejected by byte hashing.
        return ('import os,pathlib,sys; p=pathlib.Path(' + repr(str(path)) + '); '
                's=p.stat(); p.write_bytes(b"X"*s.st_size); '
                'os.utime(p,ns=(s.st_atime_ns,s.st_mtime_ns)); sys.exit(' + str(exit_code) + ')')

    def test_all_originals_and_copies_hashed_twice_on_success(self):
        with mock.patch.object(gate.runner, 'digest', wraps=gate.runner.digest) as digest:
            self.assertEqual(self.run_command(), 'fixture\n')
        counts = Counter(Path(call.args[0]) for call in digest.call_args_list)
        self.assertEqual([counts[p] for p in self.paths], [2, 2, 2, 2])
        self.assertEqual(self.source_check.call_count, 2)
        self.assertEqual(gate.runner.report['commands'][0]['state'], 'passed')

    def test_standalone_fence_still_hashes_every_copy(self):
        with mock.patch.object(gate.runner, 'digest', wraps=gate.runner.digest) as digest:
            gate.producer_source_fence()
        counts = Counter(Path(call.args[0]) for call in digest.call_args_list)
        self.assertEqual([counts[p] for p in self.paths], [1, 1, 1, 1])
        self.paths[-1].write_bytes(b'X' * self.paths[-1].stat().st_size)
        with self.assertRaises(AssertionError):
            gate.producer_source_fence()

    def test_prelaunch_tampering_of_each_producer_is_rejected(self):
        for path in self.paths:
            with self.subTest(path=path.name, directory=path.parent.name):
                data = path.read_bytes(); state = path.stat()
                path.write_bytes(b'X' * len(data))
                os.utime(path, ns=(state.st_atime_ns, state.st_mtime_ns))
                with mock.patch.object(gate.runner.ProcessTree, 'launch') as launch:
                    with self.assertRaises(AssertionError):
                        self.run_command()
                    launch.assert_not_called()
                path.write_bytes(data)

    def test_postcommand_tampering_of_each_producer_is_rejected(self):
        for index, path in enumerate(self.paths):
            with self.subTest(path=path.name, directory=path.parent.name):
                data = path.read_bytes()
                with self.assertRaises(AssertionError):
                    gate.run('mutation-' + str(index),
                             [sys.executable, '-B', '-c', self.mutation(path)], seconds=30)
                path.write_bytes(data)

    def test_execute_permission_checked_for_all_paths_before_launch(self):
        real_access = gate.os.access
        for path in self.paths:
            with self.subTest(path=str(path)):
                with mock.patch.object(gate.os, 'access', side_effect=lambda p, mode: False if Path(p) == path else real_access(p, mode)):
                    with mock.patch.object(gate.runner.ProcessTree, 'launch') as launch:
                        with self.assertRaises(AssertionError):
                            self.run_command()
                        launch.assert_not_called()

    def test_execute_permission_checked_after_command(self):
        real_access = gate.os.access
        for index, path in enumerate(self.paths):
            marker = self.root / ('mode-marker-' + str(index))
            def access(p, mode):
                return False if Path(p) == path and marker.exists() else real_access(p, mode)
            with self.subTest(path=str(path)), mock.patch.object(gate.os, 'access', side_effect=access):
                with self.assertRaises(AssertionError):
                    gate.run('mode-' + str(index), [sys.executable, '-B', '-c',
                             'from pathlib import Path; Path(' + repr(str(marker)) + ').touch()'], seconds=30)

    def test_nonzero_exit_retains_hash_and_source_post_fences(self):
        with mock.patch.object(gate.runner, 'digest', wraps=gate.runner.digest) as digest:
            with self.assertRaises(gate.runner.GateFailure) as error:
                self.run_command('import sys; sys.exit(7)')
        self.assertEqual(error.exception.code, 7)
        counts = Counter(Path(call.args[0]) for call in digest.call_args_list)
        self.assertEqual([counts[p] for p in self.paths], [2, 2, 2, 2])
        self.assertEqual(self.source_check.call_count, 2)
        self.assertEqual(gate.runner.report['commands'][0]['state'], 'failed')
        self.assertEqual(gate.runner.report['commands'][0]['exit'], 7)

    def test_nonzero_exit_does_not_hide_original_tampering(self):
        with self.assertRaises(AssertionError):
            self.run_command(self.mutation(self.paths[2], exit_code=7))
        self.assertEqual(gate.runner.report['commands'][0]['state'], 'failed')
        self.assertEqual(gate.runner.report['commands'][0]['exit'], 7)
        self.assertEqual(self.source_check.call_count, 2)


if __name__ == '__main__':
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Tests))
    assert result.wasSuccessful() and result.testsRun == gate.FENCE_FIXTURE_COUNT and not result.skipped
    print(f'ARENA_EXPORT_FENCE_FIXTURES: {gate.FENCE_FIXTURE_COUNT} passed; 0 failed; 0 skipped', flush=True)
