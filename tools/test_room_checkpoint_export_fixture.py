#!/usr/bin/env python3
"""Run the real fixture with mocked processes; no Cargo, bwrap or GPU runs.

The ancestry model represents an unmapped host ancestor, not an observation of
hosted namespace metadata. Production checkpoint rejection remains unchanged.
"""
import contextlib
import io
import json
from pathlib import Path
import runpy
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import room_checkpoint_syscall


class ExportFixture(unittest.TestCase):
    def drive(self, syscall=False, reject=False):
        source = Path(__file__).with_name('check-room-checkpoint-export.py')
        with tempfile.TemporaryDirectory(prefix='checkpoint-fixture-test-') as temporary:
            work = Path(temporary)
            binary = work/'tool'; binary.write_text('mock tool'); binary.chmod(0o700)
            evidence = work/'evidence'
            calls = []
            proof_calls = []
            host_data = None
            modes = []

            def fake_run(command, **kwargs):
                nonlocal host_data
                calls.append(command)
                env = kwargs['env']
                if command[0] != 'bwrap':
                    if '--template' in command:
                        Path(command[command.index('--output')+1]).mkdir()
                    else:
                        bundle = Path(command[command.index('--output')+1]); bundle.mkdir()
                        (bundle/'project').mkdir()
                        (bundle/'project/orr.project.json').write_text('{}')
                        (bundle/'orr.export.json').write_text('{}')
                        (bundle/'run-room-escape').write_text('mock runtime')
                    host_data = Path(env['XDG_DATA_HOME'])
                    return SimpleNamespace(returncode=0, stdout='mock exporter\n')
                self.assertEqual(command[:4], ['bwrap','--ro-bind','/','/'])
                settings = {command[i+1]:command[i+2] for i,v in enumerate(command) if v == '--setenv'}
                binds = {command[i+2]:command[i+1] for i,v in enumerate(command) if v == '--bind'}
                data = Path(settings['XDG_DATA_HOME']); home = Path(settings['HOME'])
                if syscall:
                    Path(command[command.index('-o')+1]).write_text('mock trace\n')
                if '--headless' in command:
                    Path(command[command.index('--capture')+1]).write_bytes(b'mock PNG')
                    return SimpleNamespace(returncode=0, stdout='room key: 0 won: 0\n')
                # Model the production rejection when traversal still crosses
                # an unmapped host ancestor. A direct, private bind removes it.
                if data.parent != Path('/opt') or reject:
                    return SimpleNamespace(returncode=101, stdout='Checkpoint not saved: checkpoint ancestor must be trusted and not writable by other users\n')
                self.assertEqual(home, Path('/opt/home'))
                readonly = {command[i+2]:command[i+1] for i,v in enumerate(command) if v == '--ro-bind'}
                self.assertEqual(Path(readonly['/opt/home']),host_data.parent/'home')
                self.assertEqual(set(binds),{'/opt',str(host_data.parent.parent/'captures'),str(host_data.parent.parent/'traces')})
                hidden = [command[i+1] for i,v in enumerate(command) if v == '--tmpfs']
                self.assertEqual(hidden,[str(work/'hidden'),str(host_data.parent.parent/'source')])
                self.assertEqual(Path(binds['/opt']), host_data.parent)
                self.assertEqual(host_data.parent.stat().st_mode & 0o777, 0o700)
                self.assertEqual(host_data.stat().st_mode & 0o777, 0o700)
                self.assertEqual((host_data.parent/'home').stat().st_mode & 0o777, 0o700)
                self.assertNotIn(str(host_data), binds)
                mode = next(x.split('=',1)[1] for x in command if x.startswith('ORR_ROOM_CHECKPOINT_MODE='))
                modes.append(mode)
                (host_data/'checkpoint.json').write_text(json.dumps({'key_collected': mode in ('acquire','resume')}))
                return SimpleNamespace(returncode=0, stdout=f'mode={mode} profile={data}/checkpoint.json status=fixture\ntest result: ok. 1 passed; 0 failed;\n')

            args = [str(source)]
            for name in ('creator','runtime','exporter','app-tests'):
                args += ['--'+name,str(binary)]
            args += ['--evidence',str(evidence),'--workspace',str(work/'hidden')]
            if syscall: args += ['--syscall']
            with patch.object(sys,'argv',args), patch('subprocess.run',side_effect=fake_run), \
                 patch.object(room_checkpoint_syscall,'check_syscall_proof',side_effect=lambda *x:proof_calls.append(x)), \
                 contextlib.redirect_stdout(io.StringIO()):
                runpy.run_path(str(source),run_name='__main__')
            self.assertEqual(modes,['acquire','resume','newgame','empty'])
            result = json.loads((evidence/'result.json').read_text())
            self.assertEqual(result['app_processes'],4)
            self.assertTrue(result['bundle_hashes_unchanged'])
            if syscall:
                self.assertEqual(len(proof_calls),1)
                self.assertEqual(proof_calls[0][2:],(Path('/opt/data'),Path('/opt/home'),Path('/opt/data/checkpoint.json')))
            else:
                self.assertFalse(proof_calls)
            return calls

    def test_unmapped_host_ancestor_is_not_in_profile_path(self):
        self.drive()

    def test_syscall_checker_uses_namespace_paths(self):
        self.drive(syscall=True)

    def test_failed_checkpoint_process_still_fails_gate(self):
        with self.assertRaisesRegex(AssertionError,'ancestor must be trusted'):
            self.drive(reject=True)


if __name__ == '__main__':
    unittest.main(verbosity=2)
