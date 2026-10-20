#!/usr/bin/env python3
"""Pure-Python fail-closed fixtures; no Cargo, processes, or GPU."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import animation_clip_sequence_gate as gate


class GateTests(unittest.TestCase):
    def test_exact_cpu_batch(self):
        names = gate.CPU
        output = 'running 5 tests\n' + ''.join(f'test {n} ... ok\n' for n in names)
        output += 'test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1s\n'
        gate.validate_cases(output, names)
        for bad in [output.replace('5 passed', '0 passed'), output.replace('0 ignored', '1 ignored'),
                    output.replace(' ... ok', ' ... ignored', 1), output + 'SKIP: GPU\n',
                    output.replace(names[0], 'wrong_name'), output + output,
                    output.replace('running 5', 'running 0')]:
            with self.assertRaises(RuntimeError):
                gate.validate_cases(bad, names)

    def test_cpu_inventory_and_explicit_cli_selection(self):
        calls = []
        def run(label, command):
            calls.append(command)
            if '--list' in command:
                names = [gate.CLI] if '--ignored' in command else gate.CPU
                return ''.join(n + ': test\n' for n in names) + f'{len(names)} tests, 0 benchmarks\n'
            names = [gate.CLI] if '--ignored' in command else gate.PURE_CPU
            return f'running {len(names)} tests\n' + ''.join(f'test {n} ... ok\n' for n in names) + f'test result: ok. {len(names)} passed; 0 failed; 0 ignored; 0 measured; {5-len(names)} filtered out; finished in 1s\n'
        with patch.object(gate, 'run', side_effect=run), patch.object(gate.runner, 'report', {'coverage': {}}), patch.object(gate.runner, 'record'):
            gate.cases('pure-cpu', '/absolute/model', gate.PURE_CPU, all_names=gate.CPU, ignored_names=[gate.CLI], skip=gate.CLI)
            gate.cases('actual-cli', '/absolute/model', [gate.CLI], ignored=True, all_names=gate.CPU, ignored_names=[gate.CLI])
            self.assertEqual(gate.runner.report['coverage'], {'pure-cpu': 4, 'actual-cli': 1})
        self.assertIn('--skip', calls[2])
        self.assertIn('--ignored', calls[5])
        self.assertEqual(calls[5][1:4], ['--exact', gate.CLI, '--ignored'])

    def test_interleaved_stderr(self):
        output = ('running 1 test\ntest exact ... native warning\nmore diagnostic\nok\n\n'
                  'successes:\n    exact\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1s\n')
        gate.validate_cases(output, ['exact'])
        for bad in [output.replace('    exact', '    other'), output.replace('\nok\n', '\nFAILED\n'),
                    output.replace('\nok\n', '\n'), output.replace('\nok\n', '\nok\nok\n'),
                    output.replace('successes:\n    exact\n', ''), output.replace('native warning', 'skipping GPU')]:
            with self.assertRaises(RuntimeError):
                gate.validate_cases(bad, ['exact'])

    def test_empty_inventory(self):
        with self.assertRaises(RuntimeError):
            gate.validate_cases('running 0 tests\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0s\n', [])

    def test_adapters(self):
        gate.adapter({'name': 'llvmpipe (LLVM)', 'backend': 'Vulkan', 'device_type': 'Cpu'})
        for k, v in [('name', 'SwiftShader'), ('backend', 'Gl'), ('device_type', 'DiscreteGpu')]:
            a = {'name': 'llvmpipe', 'backend': 'Vulkan', 'device_type': 'Cpu'}
            a[k] = v
            with self.assertRaises(RuntimeError): gate.adapter(a)

    def test_receipt_types(self):
        for value in [1, 'true', False, None]:
            with self.assertRaises(RuntimeError): gate.fields({'passed': value}, {'passed': True})

    def test_duplicate_json(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / 'r.json'; p.write_text('{"passed":false,"passed":true}')
            with self.assertRaises(RuntimeError): gate.document(p)

    def test_capture_integrity(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / 'a.rgba').write_bytes(bytes([1, 2, 3, 255]))
            (root / 'a.ppm').write_bytes(b'P6\n1 1\n255\n\1\2\3')
            gate.rgba(root, 'a', (1, 1))
            with self.assertRaises(RuntimeError): gate.rgba(root, 'a', (192, 192))
            (root / 'a.ppm').write_bytes(b'P6\n1 1\n255\n\1\2\4')
            with self.assertRaises(RuntimeError): gate.rgba(root, 'a')

    def test_missing_symlink_evidence(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d); (root / 'a').write_text('x'); (root / 'b').symlink_to(root / 'a')
            for path in [root / 'missing', root / 'b']:
                with self.assertRaises(RuntimeError): gate.read(path)

    def fixture(self):
        adapter = {'name': 'llvmpipe', 'backend': 'Vulkan', 'device_type': 'Cpu'}
        controls = {'wrong_order_confident_texels': 51, 'wrong_transition_confident_texels': 51}
        docs = {'render/acceptance.json': {'status': 'passed', 'actual_cli': True, 'inputs_hidden': True,
                    'adapter': adapter, 'samples': [{'time': t, 'coverage': [101, 20001]} for t in [1., 2.25, 2.5, 2.75, 4.]], 'controls': controls},
                'render/adapter.json': adapter, 'render/control-plan.json': controls}
        for name in ['room', 'yard']:
            docs[f'editor/{name}/adapter.json'] = adapter
            docs[f'editor/{name}/acceptance.json'] = {'status': 'passed', 'actual_EditorApp_widgets_and_GPU': True,
                'catalog': True, 'save_reopen': True, 'instances': 2, 'authority_byte_parity': True,
                'inputs_hidden': True, 'visible_changed_pixels': 100}
            docs[f'editor/{name}/partial.json'] = {'status': 'partial', 'tick': 135, 'bindings': 2,
                'cooked_provenance_entries': 2, 'checksum': 7, 'presentation_off_checksum': 7,
                'binding_source_hash': gate.hashlib.sha256(b'output').hexdigest()}
        return docs

    def check_fixture(self, docs):
        def pixels(root, name, size=None):
            if name == 'empty': return bytes([0, 0, 0, 255]) * 192 * 192
            return bytes([255 if name in ['wrong-order', 'wrong-transition-duration', 'transition-three-quarter'] else 0, 0, 0, 255]) * 100
        def read(path):
            if path.name.endswith('stdout'): return b'published output'
            if path.name.endswith('stderr'): return b''
            if path.name == 'action.animated.json': return b'output'
            return b'1234'
        with patch.object(gate, 'cli_receipt'), patch.object(gate, 'document', side_effect=lambda p: docs[str(p)]), patch.object(gate, 'rgba', side_effect=pixels), patch.object(gate, 'read', side_effect=read):
            gate.receipts(Path('render'), Path('editor'), '/absolute/cli')

    def test_complete_receipts(self):
        self.check_fixture(self.fixture())

    def test_partial_not_success(self):
        docs = self.fixture(); docs['render/acceptance.json']['status'] = 'partial'
        with self.assertRaises(RuntimeError): self.check_fixture(docs)

    def test_missing_sample_or_editor_phase(self):
        docs = self.fixture(); docs['render/acceptance.json']['samples'].pop()
        with self.assertRaises(RuntimeError): self.check_fixture(docs)
        docs = self.fixture(); docs['editor/room/acceptance.json']['save_reopen'] = False
        with self.assertRaises(RuntimeError): self.check_fixture(docs)

    def test_tampered_parity_pixels_and_hash(self):
        for path, key, value in [('editor/yard/partial.json', 'checksum', 8),
                                 ('editor/room/partial.json', 'binding_source_hash', '0'*64),
                                 ('editor/room/acceptance.json', 'visible_changed_pixels', 101)]:
            docs = copy.deepcopy(self.fixture()); docs[path][key] = value
            with self.assertRaises(RuntimeError): self.check_fixture(docs)


if __name__ == '__main__':
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(GateTests)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    if result.testsRun != 13 or result.skipped or not result.wasSuccessful():
        raise SystemExit(1)
    print('ANIMATION_SEQUENCE_GATE_FIXTURES: 13 passed; 0 failed; 0 skipped')
