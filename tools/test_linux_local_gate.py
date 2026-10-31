#!/usr/bin/env python3
"""Policy-only fixtures. Never invoke Cargo, rustc, or the full gate."""
import copy
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
import linux_local_gate as gate


class ReceiptPolicy(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.log = self.root / 'fake.log'
        self.artifact = self.root / 'artifact'
        self.artifact.write_text('binary stand-in')
        self.phase = {'name': 'fake', 'command': 'fake only', 'advisory': False}
        self.inputs = dict(source={'tree': 'same', 'files': 'same'},
                           context={'cwd': '/fixed', 'toolchain': 'same', 'native': 'same'},
                           environment={'PATH': '/fixed', 'CARGO_NET_OFFLINE': 'true',
                                        'HTTP_PROXY': 'private-value'}, producers={})
        with self.log.open('wb') as out:
            result = subprocess.run([sys.executable, '-c', 'print("fake passed")'], stdout=out)
        self.key = gate.phase_key(self.phase, **self.inputs)
        self.receipt = gate.terminal_receipt(self.phase, self.key, result.returncode,
                                             self.log, [self.artifact])

    def test_completed_same_source_restart_reuses(self):
        self.assertTrue(gate.reusable(self.receipt, self.phase, self.key))

    def test_shell_bookkeeping_only(self):
        self.inputs['environment'].update(SHLVL='42', OLDPWD='/old', _='bookkeeping')
        self.assertEqual(self.key, gate.phase_key(self.phase, **self.inputs))

    def test_unknown_and_sensitive_environment_fail_closed(self):
        for name in ['UNKNOWN', 'CODEX_TOOL_CALL_ID', 'HTTP_PROXY', 'CARGO_HOME',
                     'RUSTFLAGS', 'ORR_ERP', 'PATH', 'HOME', 'TMPDIR', 'PYTHONPATH', 'CC']:
            with self.subTest(name=name):
                args = copy.deepcopy(self.inputs)
                args['environment'][name] = 'changed'
                self.assertNotEqual(self.key, gate.phase_key(self.phase, **args))

    def test_no_environment_values_or_per_key_hashes(self):
        identity = gate.environment_identity(self.inputs['environment'])
        self.assertEqual(set(identity), {'names', 'fingerprint'})
        self.assertNotIn('private-value', repr(identity))

    def test_source_context_and_producer_changes_invalidate(self):
        for field in ['source', 'context', 'producers']:
            args = copy.deepcopy(self.inputs)
            args[field]['changed'] = True
            self.assertNotEqual(self.key, gate.phase_key(self.phase, **args))

    def test_phase_features_change_invalidates(self):
        phase = dict(self.phase, command='fake --features changed')
        self.assertNotEqual(self.key, gate.phase_key(phase, **self.inputs))

    def test_unknown_failure_and_interruption_never_reuse(self):
        for code, interrupted in [(1, False), (0, True), (-15, False), (130, False)]:
            receipt = gate.terminal_receipt(self.phase, self.key, code, self.log,
                                            interrupted=interrupted)
            self.assertFalse(gate.reusable(receipt, self.phase, self.key))
        self.assertFalse(gate.reusable({'status': 'RUNNING'}, self.phase, self.key))

    def test_advisory_is_explicit_and_not_signal(self):
        phase = dict(self.phase, advisory=True)
        key = gate.phase_key(phase, **self.inputs)
        for code in [0, 1, 127]:
            self.assertTrue(gate.reusable(gate.terminal_receipt(phase, key, code, self.log), phase, key))
        self.assertFalse(gate.reusable(gate.terminal_receipt(phase, key, 130, self.log), phase, key))
        self.assertFalse(gate.reusable(self.receipt, phase, key))

    def test_log_artifact_tamper_and_deletion_reject(self):
        for path in [self.log, self.artifact]:
            before = path.read_bytes()
            path.write_text('tampered')
            self.assertFalse(gate.reusable(self.receipt, self.phase, self.key))
            path.write_bytes(before)
        self.artifact.unlink()
        self.assertFalse(gate.reusable(self.receipt, self.phase, self.key))

    def test_missing_file_inventory_rejects(self):
        receipt = copy.deepcopy(self.receipt)
        receipt['files'].pop(str(self.artifact))
        self.assertFalse(gate.reusable(receipt, self.phase, self.key))

    def test_aggregate_missing_and_duplicate_phase_rejects(self):
        self.assertIsNone(gate.composed_coverage([], []))
        self.assertIsNone(gate.composed_coverage([self.phase], []))
        other = dict(self.phase, name='other')
        evidence = [(self.receipt, self.key)] * 2
        self.assertIsNone(gate.composed_coverage([self.phase, other], evidence))
        result = gate.composed_coverage([self.phase], [(self.receipt, self.key)])
        self.assertFalse(result['original_uninterrupted_exit_zero'])

    def test_isolated_adapter_fake_commands_and_proxy_churn(self):
        # Separate isolated interpreter exercises the real adapter; fake script
        # and mocked snapshots avoid Cargo and expensive host toolchain scans.
        code = r"""
import importlib.util, os, pathlib, sys, tempfile
from unittest.mock import patch
spec = importlib.util.spec_from_file_location('policy', sys.argv[1])
gate = importlib.util.module_from_spec(spec); spec.loader.exec_module(gate)
with tempfile.TemporaryDirectory() as temp:
    base = pathlib.Path(temp); root = base / 'source'; (root / 'tools').mkdir(parents=True)
    receipts = base / 'receipts'; name = 'static_game_standalone_focus_sync'
    script = root / 'tools' / ('test_' + name + '.py')
    dependency = root / gate.LOCAL_SOURCE_DEPENDENCIES[name][0]
    dependency.parent.mkdir(parents=True); dependency.write_text('fixed source')
    counter = base / 'counter'
    script.write_text('from pathlib import Path\np=Path(' + repr(str(counter)) + ')\np.write_text(p.read_text()+"x" if p.exists() else "x")\n')
    with patch.object(gate, 'source_snapshot', return_value={'source': 'fixed'}), patch.object(gate, 'python_snapshot', return_value={'runtime': 'fixed'}), patch.dict(gate.AUDITED_SCRIPT_SHA256, {name: gate.file_stamp(script)['sha256']}):
        assert gate.execute_local(name, receipts, root) == 0
        os.environ['HTTP_PROXY'] = 'different-private-proxy'
        os.environ['CODEX_TOOL_CALL_ID'] = 'different-call'
        assert gate.execute_local(name, receipts, root) == 0
        assert counter.read_text() == 'x', 'audited metadata caused rerun'
        os.environ['UNKNOWN_INPUT'] = 'changed'
        assert gate.execute_local(name, receipts, root) == 0
        assert counter.read_text() == 'xx', 'unknown change was ignored'
        import json
        latest = receipts / (name + '.latest.json')
        selected = json.loads(latest.read_text())['start']
        (receipts / selected.replace('.start.json', '.log')).write_text('tampered')
        assert gate.execute_local(name, receipts, root) == 0
        assert counter.read_text() == 'xxx'
        import time
        unknown = name + '-000-old-clock.start.json'
        (receipts / unknown).write_text('{}')
        latest.write_text(json.dumps({'start': unknown}))
        assert gate.execute_local(name, receipts, root) == 0
        assert counter.read_text() == 'xxxx', 'interrupted newer attempt resurrected old PASS'
        dependency.write_text('changed without index change')
        assert gate.execute_local(name, receipts, root) == 0
        assert counter.read_text() == 'xxxxx', 'fixed source omitted from git index was ignored'
        assert os.environ['HTTP_PROXY'] == 'different-private-proxy'
        script.write_text('print(\"changed script\")')
        try: gate.execute_local(name, receipts, root)
        except ValueError: pass
        else: raise AssertionError('changed script received exemptions')
        script.unlink(); script.symlink_to(counter)
        try: gate.execute_local(name, receipts, root)
        except ValueError: pass
        else: raise AssertionError('external script symlink accepted')
        for phase in ['workspace-tests', 'workspace-build', 'format']:
            try: gate.execute_local(phase, receipts, root)
            except ValueError: pass
            else: raise AssertionError('unaudited phase admitted')
"""
        result = subprocess.run([sys.executable, '-I', '-S', '-B', '-c', code,
                                 str(Path(gate.__file__).resolve())], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_python_bytecode_zip_and_import_path_binding(self):
        import sysconfig
        import platform
        from unittest.mock import patch
        lib = self.root / 'stdlib'
        cache = lib / '__pycache__'
        cache.mkdir(parents=True)
        (lib / 'lib-dynload').mkdir()
        pyc = cache / 'unittest.pyc'
        pyc.write_bytes(b'old bytecode')
        archive = self.root / 'python.zip'
        archive.write_bytes(b'old archive')
        def stamp(path):
            path = Path(path)
            return {'payload': path.read_bytes().hex()} if self.root in path.parents else {'path': str(path)}
        with patch.object(sysconfig, 'get_path', return_value=str(lib)), \
             patch.object(sys, 'path', [str(archive), str(lib), str(lib / 'lib-dynload')]), \
             patch.object(gate, 'file_stamp', side_effect=stamp):
            before = gate.python_snapshot()
            pyc.write_bytes(b'new bytecode')
            after = gate.python_snapshot()
            self.assertNotEqual(before, after)
            archive.write_bytes(b'new archive')
            self.assertNotEqual(after, gate.python_snapshot())
            link = lib / 'linked-package'
            link.symlink_to(cache, target_is_directory=True)
            with self.assertRaises(RuntimeError):
                gate.python_snapshot()
            link.unlink()
            sys.path.append(str(self.root / 'unreviewed'))
            with self.assertRaises(RuntimeError):
                gate.python_snapshot()

    def test_explicit_no_bytecode_write_flag_required(self):
        from types import SimpleNamespace
        from unittest.mock import patch
        flags = SimpleNamespace(isolated=1, no_site=1, dont_write_bytecode=0, optimize=0)
        with patch.object(sys, 'flags', flags), self.assertRaises(RuntimeError):
            gate.execute_local('static_game_standalone_focus_sync', self.root, self.root)

    def test_shell_opt_in_dispatch_and_failure_without_real_commands(self):
        import os
        bin_dir = self.root / 'bin'
        bin_dir.mkdir()
        stub = r"""#!/bin/sh
printf '%s %s\n' "${0##*/}" "$*" >> "$ORR_FAKE_COMMAND_LOG"
case "$*" in
  *tools/linux_local_gate.py*) exit "${ORR_FAKE_FAIL:-0}" ;;
esac
if [ "${0##*/}" = python3 ] && [ "$1" = - ]; then printf '/fake/assembler\n'; fi
exit 0
"""
        for name in ['cargo', 'rustc', 'python3']:
            executable = bin_dir / name
            executable.write_text(stub)
            executable.chmod(0o755)
        shell = Path(gate.__file__).with_name('check-linux-local.sh')
        command_log = self.root / 'commands.log'
        env = dict(os.environ, PATH=str(bin_dir) + os.pathsep + os.environ['PATH'],
                   ORR_FAKE_COMMAND_LOG=str(command_log))
        env.pop('ORR_LOCAL_POLICY_RECEIPTS', None)
        result = subprocess.run(['bash', str(shell)], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        original = command_log.read_text().splitlines()
        self.assertFalse(any('linux_local_gate.py' in line for line in original))
        command_log.write_text('')
        env['ORR_LOCAL_POLICY_RECEIPTS'] = str(self.root / 'receipts')
        result = subprocess.run(['bash', str(shell)], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        selected = command_log.read_text().splitlines()
        adapter = [line for line in selected if 'linux_local_gate.py' in line]
        self.assertEqual(len(adapter), 3)
        for phase in gate.LOCAL_PHASES:
            self.assertTrue(any('python3 -I -S -B tools/linux_local_gate.py --phase ' + phase in line for line in adapter))
        # Other dispatches are identical, except generated temporary filenames.
        def unaffected(lines):
            return [line for line in lines if 'linux_local_gate.py' not in line
                    and not any('test_' + phase + '.py' in line for phase in gate.LOCAL_PHASES)
                    and not line.startswith('python3 - /tmp/')]
        self.assertEqual(unaffected(original), unaffected(selected))
        command_log.write_text(''); env['ORR_FAKE_FAIL'] = '7'
        result = subprocess.run(['bash', str(shell)], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 7)
        failed = command_log.read_text()
        self.assertEqual(failed.count('tools/linux_local_gate.py'), 1)
        self.assertNotIn('python3 -B tools/test_static_game_actions_observation_pacing.py', failed)

    def test_legacy_phase_coverage(self):
        self.assertEqual(len(gate.PHASES), 23)
        self.assertEqual(len({p['name'] for p in gate.PHASES}), 23)
        self.assertEqual([p['name'] for p in gate.PHASES if p['advisory']], ['format'])
        commands = '\n'.join(p['command'] for p in gate.PHASES)
        self.assertIn('cargo test --locked --workspace --release', commands)
        self.assertIn('--features standalone-native-acceptance --test standalone_x11 --no-run', commands)
        self.assertIn('int(counts[0]) <= 0', commands)


if __name__ == '__main__':
    unittest.main()
