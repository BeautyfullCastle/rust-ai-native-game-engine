#!/usr/bin/env python3
"""Independent gate contract and negative fixtures; standard library only."""
import ast
import pathlib
import re
import tempfile
import unittest
from unittest import mock
import static_model_assemble_gate as gate

ROOT = pathlib.Path(__file__).resolve().parents[1]

def workflow_contract(text):
    assert text.startswith("name: Static model assemble\non:\n  pull_request:\n    branches: [main, su/engine-correctness-hardening]\n    types: [ready_for_review]\n  workflow_dispatch:\npermissions:\n")
    assert 'permissions:\n  contents: read\n' in text
    assert text.count('      matrix:\n') == 1
    rows = re.findall(r'          - \{mode: (\w+), os: ([\w.-]+)\}', text)
    assert rows == list(gate.WORKFLOW_ROWS)
    assert 'continue-on-error' not in text and 'paths-ignore' not in text
    assert '    timeout-minutes: 90\n' in text
    assert '      PYTHONDONTWRITEBYTECODE: "1"\n' in text
    assert "        run: python -B tools/static_model_assemble_gate.py '${{ matrix.mode }}'" in text
    assert '          ORR_STATIC_ASSEMBLE_EVIDENCE: '+gate.WORKFLOW_EVIDENCE in text
    assert '        if: always()\n        uses: actions/upload-artifact@v4' in text
    assert '          if-no-files-found: error' in text
    assert '          name: '+gate.WORKFLOW_ARTIFACT in text
    assert '          path: '+gate.WORKFLOW_EVIDENCE in text
    assert '    if: always()\n    needs: acceptance' in text
    assert '        run: test "$ACCEPTANCE" = success' in text
    assert 'mesa-vulkan-drivers' in text
    # Contexts: runner.temp is forbidden in job-level env and allowed only in
    # the reviewed individual step env/artifact with blocks.
    for line in text.splitlines():
        if 'runner.temp' in line:
            assert line.startswith('          ORR_') or line.startswith('          path:')

class Fixtures(unittest.TestCase):
    def test_workflow_exact_contract(self):
        workflow_contract((ROOT/'.github/workflows/static-model-assemble.yml').read_text())

    def test_workflow_negative_mutations(self):
        original=(ROOT/'.github/workflows/static-model-assemble.yml').read_text()
        # Only explicit integration readiness or dispatch may start expensive CI.
        trigger_filters = "    branches: [main, su/engine-correctness-hardening]\n    types: [ready_for_review]\n"
        for old, new in [
            (trigger_filters, ''),
            ('branches: [main, su/engine-correctness-hardening]', "branches: ['**']"),
            ('branches: [main, su/engine-correctness-hardening]', 'branches: [main]'),
            ('branches: [main, su/engine-correctness-hardening]', 'branches: [su/engine-correctness-hardening]'),
            ("    types: [ready_for_review]\n", ''),
            ('types: [ready_for_review]', 'types: [opened, synchronize]'),
            ('  pull_request:\n', ''),
            ('  pull_request:\n', '  pull_request:\n    branches: [main]\n'),
            ('  pull_request:\n', '  pull_request:\n    paths: [src/**]\n'),
        ]:
            with self.subTest(trigger=old), self.assertRaises(AssertionError):
                workflow_contract(original.replace(old, new, 1))
        for old,new in [('mode: gpu','mode: cpu'),('if-no-files-found: error','if-no-files-found: warn'),('if: always()','if: success()'),('test "$ACCEPTANCE" = success','true'),('      PYTHONDONTWRITEBYTECODE: "1"','      PYTHONDONTWRITEBYTECODE: "0"')]:
            with self.assertRaises(AssertionError): workflow_contract(original.replace(old,new))
        with self.assertRaises(AssertionError): workflow_contract(original.replace('    env:\n','    env:\n      BAD: ${{ runner.temp }}\n',1))

    def test_exact_model_census_and_socket_required(self):
        source=(ROOT/'crates/orr_model/tests/static_model_assemble.rs').read_text()
        names=re.findall(r'#\[test\]\nfn (\w+)',source)
        self.assertEqual(set(names),set(gate.model_tests('linux'))|{'cli_windows_nonlocal_prefixes_reject'})
        self.assertIn('cli_unix_socket_inputs_reject_without_opening_or_publishing',gate.model_tests('linux'))
        self.assertNotIn('cli_unix_socket_inputs_reject_without_opening_or_publishing',gate.model_tests('win32'))

    def test_source_snapshot_bytes_modes_and_inventory(self):
        with tempfile.TemporaryDirectory() as root:
            root=pathlib.Path(root); p=root/'a';p.write_bytes(b'a')
            with mock.patch.object(gate,'source_root',root):
                first=gate.snapshot(); p.write_bytes(b'b'); self.assertNotEqual(first,gate.snapshot())
                p.write_bytes(b'a'); p.chmod(0o700); self.assertNotEqual(first,gate.snapshot())
                (root/'extra').write_bytes(b'x'); self.assertNotEqual(set(first),set(gate.snapshot()))

    def test_source_fence_detects_mutation(self):
        with tempfile.TemporaryDirectory() as root:
            root=pathlib.Path(root); p=root/'a';p.write_bytes(b'a')
            with mock.patch.object(gate,'source_root',root):
                frozen=gate.snapshot()
                with mock.patch.object(gate,'frozen',frozen):
                    gate.source_fence(); p.write_bytes(b'b')
                    with self.assertRaises(AssertionError): gate.source_fence()

    def test_exact_test_results_reject_skip_failure_or_missing(self):
        names=['one','two']
        good='running 2 tests\ntest one ... ok\ntest two ... ok\n\ntest result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n'
        gate.runner.result(good,names)
        for bad in [good.replace('two ... ok','two ... ignored'),good.replace('2 passed','1 passed'),good.replace('0 filtered','1 filtered'),good.replace('two ... ok','wrong ... ok')]:
            with self.assertRaises(AssertionError): gate.runner.result(bad,names)

    def test_required_sources_include_runner_and_gpu_checker(self):
        names=set(gate.FEATURE_PATHS+gate.INFRA_PATHS)
        for package in ('orr_model','orr_render'):
            names.update([f'crates/{package}/Cargo.toml',f'crates/{package}/src/lib.rs'])
        gate.require_sources(dict.fromkeys(names))
        for name in ('tools/static_model_extract_gate.py','tools/static_model_assemble_gpu_check.py',*gate.FEATURE_PATHS):
            with self.assertRaises(AssertionError): gate.require_sources(dict.fromkeys(names-{name}))

    def test_producer_and_final_fence_are_mandatory(self):
        source=(ROOT/'tools/static_model_assemble_gate.py').read_text(); ast.parse(source)
        self.assertIn("runner.fence()",source)
        self.assertIn("source_fence()",source)
        self.assertIn("--message-format=json",source)
        self.assertIn("expected=2",source)
        self.assertIn("'--', '-D', 'warnings'",source)
        self.assertIn("check_coverage()",source)
        self.assertNotIn("'--skip'",source)

if __name__=='__main__':
    suite=unittest.defaultTestLoader.loadTestsFromTestCase(Fixtures)
    result=unittest.TextTestRunner(verbosity=2).run(suite)
    if not result.wasSuccessful() or result.testsRun != 8 or result.skipped:
        raise SystemExit(1)
    print('STATIC_ASSEMBLE_GATE_FIXTURES: 8 passed; 0 failed; 0 skipped')
