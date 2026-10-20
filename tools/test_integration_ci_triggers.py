"""Fast trigger-contract checks. No compiler or hosted CI is invoked."""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
TRIGGER = ('on:\n  pull_request:\n'
           '    branches: [main, su/engine-correctness-hardening]\n'
           '    types: [ready_for_review]\n'
           '  workflow_dispatch:\n')


def check(text):
    match = re.search(r'^on:\n(?:[ \t].*\n|\n)*', text, re.M)
    assert match and match.group() == TRIGGER
    assert 'pull_request_target:' not in text


class IntegrationTriggers(unittest.TestCase):
    def test_all_build_test_workflows(self):
        paths = list((ROOT / '.github/workflows').glob('*.yml'))
        self.assertEqual({path.name for path in paths}, {
            'arena-agent-trials.yml', 'blockout-authoring.yml', 'determinism.yml', 'input-map-authoring.yml', 'ui-subtree-duplicate.yml'
        })
        for path in paths:
            with self.subTest(workflow=path.name):
                check(path.read_text())

    def test_reject_automatic_or_incomplete_triggers(self):
        for bad in [
            TRIGGER.replace('ready_for_review', 'opened, synchronize'),
            TRIGGER.replace('    types: [ready_for_review]\n', ''),
            TRIGGER.replace('branches: [main, su/engine-correctness-hardening]', "branches: ['**']"),
            TRIGGER.replace('  workflow_dispatch:\n', ''),
            TRIGGER + '  push:\n',
            TRIGGER + '  schedule:\n    - cron: "0 * * * *"\n',
        ]:
            with self.subTest(trigger=bad), self.assertRaises(AssertionError):
                check(bad)


if __name__ == '__main__':
    unittest.main()
