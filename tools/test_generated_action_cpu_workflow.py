#!/usr/bin/env python3
"""Fail-closed wiring checks; these are not hosted CI or Rust acceptance evidence."""
import copy
import json
import os
import tempfile
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
JOB_SHA256 = 'acf67bafa6640bb40fbe1e7140b1efac04e10705b2bb8134a3028da673f505f4'


def job_text(text):
    return text.split('  '+JOB+':\n', 1)[1].split('  generated-user-game-native:\n', 1)[0]


def contract(text):
    assert text.count('  '+JOB+':\n') == 1
    assert hashlib.sha256(job_text(text).encode()).hexdigest() == JOB_SHA256
    jobs = yaml.safe_load(text)['jobs']
    job = jobs[JOB]
    assert job['runs-on'] == 'ubuntu-22.04'
    assert job['timeout-minutes'] == 97
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
            ('timeout-minutes: 5', 'timeout-minutes: 50'),
            ('pkg-config --exists', 'true # pkg-config --exists'),
            ('timeout-minutes: 80', 'timeout-minutes: 1'),
            ('if: always()', 'if: success()'),
            ('if-no-files-found: error', 'if-no-files-found: warn'),
            ('retention-days: 7', 'retention-days: 1'),
            ('    runs-on: ubuntu-22.04', '    if: false\n    runs-on: ubuntu-22.04'),
            ('    timeout-minutes: 97', '    continue-on-error: true\n    timeout-minutes: 97'),
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

    def test_setup_budget_preserves_gate_and_retention(self):
        job = contract(WORKFLOW.read_text())[JOB]
        steps = job['steps']
        self.assertEqual(sum(step['timeout-minutes'] for step in steps), job['timeout-minutes'])
        setup = next(step for step in steps if step.get('name') == 'Install required CPU compilation dependencies')
        self.assertEqual(setup['timeout-minutes'], 5)
        self.assertEqual(next(step for step in steps if step.get('name') == 'Required generated preview portable action contracts')['timeout-minutes'], 80)
        self.assertEqual(steps[-1]['timeout-minutes'], 5)

    def test_package_setup_exit_and_partial_evidence(self):
        job = contract(WORKFLOW.read_text())[JOB]
        setup = next(step for step in job['steps'] if step.get('name') == 'Install required CPU compilation dependencies')
        # Execute the actual shell with inert command doubles, never invoking sudo/apt.
        for fail_at, exit_code in (('', 0), ('update', 23), ('install', 24), ('--exists', 25), ('--modversion', 26)):
            with self.subTest(fail_at=fail_at), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                evidence = root / 'generated-action-preview-job'
                evidence.mkdir()
                commands = root / 'bin'
                commands.mkdir()
                for name in ('sudo', 'pkg-config'):
                    command = commands / name
                    command.write_text('#!/bin/bash\n'
                        'printf "%s\\n" "$*" >> "$CALLS"\n'
                        'for arg in "$@"; do\n'
                        '  if [[ -n "$FAIL_AT" && "$arg" == "$FAIL_AT" ]]; then exit "$FAIL_CODE"; fi\n'
                        'done\n'
                        'echo simulated-command-output\n')
                    command.chmod(0o755)
                env = dict(os.environ, PATH=str(commands) + os.pathsep + os.environ['PATH'],
                           RUNNER_TEMP=str(root), CALLS=str(root / 'calls'), FAIL_AT=fail_at, FAIL_CODE=str(exit_code))
                result = subprocess.run(['bash', '-e', '-o', 'pipefail', '-c', setup['run']], env=env, text=True, capture_output=True)
                self.assertEqual(result.returncode, exit_code, result.stderr)
                receipt = json.loads((evidence / 'setup-packages-status.json').read_text())
                self.assertEqual(receipt['exit_code'], exit_code)
                self.assertGreaterEqual(receipt['elapsed_seconds'], 0)
                self.assertIs(receipt['native_acceptance'], False)
                self.assertTrue((evidence / 'setup-packages.log').is_file())
                calls = (root / 'calls').read_text().splitlines()
                expected = ['apt-get update -q',
                    'apt-get install -y -q libasound2-dev libudev-dev libxkbcommon-dev libwayland-dev pkg-config',
                    '--exists alsa libudev xkbcommon wayland-client',
                    '--modversion alsa libudev xkbcommon wayland-client']
                count = {'': 4, 'update': 1, 'install': 2, '--exists': 3, '--modversion': 4}[fail_at]
                self.assertEqual(calls, expected[:count])
                # Neither success nor failure promotes the overall gate setup receipt.
                self.assertFalse((evidence / 'setup.json').exists())

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
