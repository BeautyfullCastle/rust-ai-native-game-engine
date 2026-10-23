#!/usr/bin/env python3
"""Fail-closed wiring checks; these are not hosted CI or Rust acceptance evidence."""
import copy
import hashlib
from pathlib import Path
import subprocess
import unittest
import yaml

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / '.github/workflows/determinism.yml'
JOB = 'generated-user-game-actions-cpu'
RESULT = 'GENERATED_ACTION_CPU_RESULT'
# Pin the reviewed complete job, including bounded setup and failure retention.
JOB_SHA256 = 'a8259c7c97f07516219ab62638d5f6eb7cca00981b3de01d50caa4c75a31b781'


def job_text(text):
    return text.split('  '+JOB+':\n', 1)[1].split('  generated-user-game-native:\n', 1)[0]


def contract(text):
    assert text.count('  '+JOB+':\n') == 1
    assert hashlib.sha256(job_text(text).encode()).hexdigest() == JOB_SHA256
    jobs = yaml.safe_load(text)['jobs']
    job = jobs[JOB]
    assert job['runs-on'] == 'ubuntu-22.04'
    assert job['timeout-minutes'] == 95
    assert not any(key in job for key in ('if', 'needs', 'strategy', 'continue-on-error'))
    aggregate = jobs['cross_platform_checksum_compare']
    assert aggregate['needs'].count(JOB) == 1
    assert aggregate['if'] == '${{ always() }}'
    first = aggregate['steps'][0]
    assert first['env'][RESULT] == "${{ needs['generated-user-game-actions-cpu'].result }}"
    assert ' '+RESULT+'; do' in first['run']
    assert not any(key in first for key in ('if', 'continue-on-error'))
    for step in job['steps']:
        assert isinstance(step['timeout-minutes'], int)
        assert 'continue-on-error' not in step
        if 'run' in step:
            checked = subprocess.run(['bash', '-n'], input=step['run'], text=True, capture_output=True)
            assert checked.returncode == 0, checked.stderr
    return jobs


class GeneratedActionCpuWorkflowTests(unittest.TestCase):
    def test_reviewed_job_and_shell_syntax(self):
        contract(WORKFLOW.read_text())

    def test_pin_paths_python_and_retention_mutations_rejected(self):
        text = WORKFLOW.read_text()
        original = job_text(text)
        changes = [
            ('6f97ac85a80aabda26679b482d182c9365bff92e', 'HEAD'),
            ('cd7cc2a03990a8778a99e60a82c3cfbb425c4d87', '0'*40),
            ('git archive "$PIN"', 'git archive HEAD'),
            ('cargo fetch --locked', 'cargo fetch'),
            ('update-environment: false', 'update-environment: true'),
            ('${{ steps.action_preview_python.outputs.python-path }}', 'python'),
            ('generated action preview', '../generated action preview'),
            ('timeout-minutes: 80', 'timeout-minutes: 1'),
            ('if: always()', 'if: success()'),
            ('if-no-files-found: error', 'if-no-files-found: warn'),
            ('retention-days: 7', 'retention-days: 1'),
            ('    runs-on: ubuntu-22.04', '    if: false\n    runs-on: ubuntu-22.04'),
            ('    timeout-minutes: 95', '    continue-on-error: true\n    timeout-minutes: 95'),
            ('"native_acceptance":false', '"native_acceptance":true'),
        ]
        for old, new in changes:
            with self.subTest(old=old):
                self.assertIn(old, original)
                changed = text.replace(original, original.replace(old, new, 1), 1)
                with self.assertRaises(AssertionError):
                    contract(changed)

    def test_removed_aggregate_wiring_rejected(self):
        text = WORKFLOW.read_text()
        for old in (', '+JOB, "          "+RESULT+": ${{ needs['"+JOB+"'].result }}\n", ' '+RESULT+'; do'):
            with self.subTest(old=old):
                self.assertIn(old, text)
                with self.assertRaises((AssertionError, KeyError)):
                    contract(text.replace(old, '; do' if old.endswith('; do') else '', 1))

    def test_each_preview_terminal_result_fails_closed(self):
        jobs = contract(WORKFLOW.read_text())
        step = jobs['cross_platform_checksum_compare']['steps'][0]
        for state in ('success', 'failure', 'cancelled', 'skipped', '', 'unknown'):
            env = {key: 'success' for key in step['env']}
            env[RESULT] = state
            result = subprocess.run(['bash', '-c', step['run']], env=env, capture_output=True, text=True)
            with self.subTest(state=state):
                self.assertEqual(result.returncode == 0, state == 'success')


if __name__ == '__main__':
    unittest.main()
