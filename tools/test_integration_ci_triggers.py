"""Fast trigger-contract checks. No compiler or hosted CI is invoked."""
from pathlib import Path
import ast
import textwrap
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


def check_room_usage_selectors(text):
    # Keep the new checks in the existing batch, with no separate job or workflow.
    required = (
        'for room_features in ("room-project", "room-animated-models"):',
        '"--features", room_features, "--example", "room_package_usage", "--",',
        'helpers["validate_tests"](output, "tests", 12 if os.name == "nt" else 13)',
        '"--test", "room_package_usage_consumer", "--",',
        'helpers["CASE"].findall(output) != [(consumer, "ok")]',
        '"room-usage-static-lint"', '"room-usage-animated-lint"',
        '"room-usage-default-off-lint"', '"room-usage-consumer-lint"',
        '"--test", "checked_remove"', '"--test", "restore_removed"',
        '"package_manager::removal::tests"', '"package_manager_panel::tests"',
        '"--test", "package_removal_ui"',
    )
    for selector in required:
        assert selector in text, selector
    assert text.count('  package-removal:\n') == 1
    assert 'os: [ubuntu-22.04, windows-latest]' in text
    # Parse each inline Python block without executing Rust or hosted actions.
    for block in re.findall(r'        shell: python \{0\}\n        run: \|\n((?:          .*\n|\n)+)', text):
        ast.parse(textwrap.dedent(block))


class IntegrationTriggers(unittest.TestCase):
    def test_all_build_test_workflows(self):
        paths = list((ROOT / '.github/workflows').glob('*.yml'))
        self.assertEqual(len(paths), 47)
        for path in paths:
            with self.subTest(workflow=path.name):
                check(path.read_text())

    def test_package_removal_keeps_saved_usage_in_existing_batch(self):
        text = (ROOT / ".github/workflows/package-removal.yml").read_text()
        check_room_usage_selectors(text)

    def test_reject_missing_room_usage_route_or_wrong_platform_count(self):
        text = (ROOT / ".github/workflows/package-removal.yml").read_text()
        for before, after in (
            ('for room_features in ("room-project", "room-animated-models"):',
             'for room_features in ("room-project",):'),
            ('"--test", "room_package_usage_consumer", "--",',
             '"--test", "unrelated_consumer", "--",'),
            ('12 if os.name == "nt" else 13', '13'),
            ('"--test", "restore_removed"', '"--test", "unrelated"'),
        ):
            with self.subTest(selector=before), self.assertRaises(AssertionError):
                check_room_usage_selectors(text.replace(before, after))

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
