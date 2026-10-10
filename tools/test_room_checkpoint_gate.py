#!/usr/bin/env python3
"""Static/synthetic gate checks only. These do not execute GPU or strace."""
import json
from pathlib import Path
import re
import textwrap
import unittest

import room_checkpoint_gate as gate


ROOT = Path(__file__).resolve().parents[1]
INVENTORIES = json.loads(gate.INVENTORY.read_text())


def check_inherited_package_gates(workflow):
    expected = INVENTORIES['package-closure']['names']
    lines = re.findall(r'^\s*(check_count|check_library|check_result) package(?: (\d+)(?: (\d+))?)? cargo test --locked --release -p orr_package --lib$', workflow, re.M)
    assert len(lines) == 4 and sum(bool(count) for _, count, _ in lines) == 3
    for _, count, ignored in lines:
        if count: assert int(count) == len(expected), ('stale package count', count)
        if ignored: assert int(ignored) == 0
    inventories = []
    for body in re.findall(r"cat >[^\n]*<<'JSONINVENTORY'\n(.*?)^\s*JSONINVENTORY$", workflow, re.M | re.S):
        value = json.loads(textwrap.dedent(body))
        if 'package' not in value: continue
        spec = value['package']
        names = spec if isinstance(spec, list) else spec['names']
        assert sorted(names) == expected
        if isinstance(spec, dict):
            assert spec['passed'] == len(expected) and spec['ignored'] == 0
            assert spec['expected_summary_rows'] == [[1, 0, 0], [len(expected), 0, 0]]
        inventories.append(spec)
    assert len(inventories) == 2


def summaries(spec):
    return ''.join(f'test result: ok. {passed} passed; {failed} failed; {ignored} ignored; 0 measured; 0 filtered out; finished in 0.01s\n'
                   for passed, failed, ignored in spec['expected_summary_rows'])


class InventoryChecks(unittest.TestCase):
    def test_all_four_inherited_package_gates_match_source_inventory(self):
        workflow = (ROOT / '.github/workflows/determinism.yml').read_text()
        check_inherited_package_gates(workflow)
        for command in ('check_count package 36', 'check_library package 36 0', 'check_result package 36 0'):
            with self.assertRaises(AssertionError):
                check_inherited_package_gates(workflow.replace(command, command.replace('36', '28')))

    def test_all_exact_positive_inventories(self):
        for lane, spec in INVENTORIES.items():
            with self.subTest(lane=lane):
                for mode in ('list', 'ignored'):
                    wanted = spec['names'] if mode == 'list' else spec['ignored_names']
                    text = ''.join(f'{name}: test\n' for name in wanted)
                    gate.check_log(spec, mode, text, spec['passed'], spec['ignored'])
                gate.check_log(spec, 'result', summaries(spec), spec['passed'], spec['ignored'])

    def test_zero_missing_extra_failed_and_skip_rejected(self):
        for lane, spec in INVENTORIES.items():
            good = summaries(spec)
            bad = ['', good + good, good + 'test result: malformed\n',
                   good.replace('passed; 0 failed;', 'passed; 1 failed;'),
                   re.sub(r'\d+ passed;', '0 passed;', good), good + 'SKIP: unavailable\n']
            for text in bad:
                with self.subTest(lane=lane, text=text):
                    with self.assertRaises(AssertionError):
                        gate.check_log(spec, 'result', text, spec['passed'], spec['ignored'])
            with self.assertRaises(AssertionError):
                gate.check_log(spec, 'list', '', spec['passed'], spec['ignored'])

    def test_inventory_name_substitution_and_duplicates_rejected(self):
        for spec in INVENTORIES.values():
            good = ''.join(f'{name}: test\n' for name in spec['names'])
            for text in (good + good, good.replace(spec['names'][0], 'wrong_test', 1)):
                with self.assertRaises(AssertionError):
                    gate.check_log(spec, 'list', text, spec['passed'], spec['ignored'])

    def test_source_test_functions_exist(self):
        source = '\n'.join(path.read_text() for folder in ('orr_games', 'orr_package', 'orr_sample', 'orr_editor')
                           for path in (ROOT / 'crates' / folder / 'src').rglob('*.rs'))
        for spec in INVENTORIES.values():
            for name in spec['names']:
                self.assertRegex(source, r'\bfn\s+' + re.escape(name.rsplit('::', 1)[1]) + r'\s*\(')

    def test_package_requires_fifo_child_and_full_parent(self):
        spec = INVENTORIES['package-closure']
        self.assertEqual(spec['expected_summary_rows'], [[1, 0, 0], [36, 0, 0]])
        for rows in ([[36, 0, 0]], [[1, 0, 0]], [[0, 0, 0], [36, 0, 0]]):
            with self.assertRaises(AssertionError):
                gate.check_log(spec, 'result', summaries({'expected_summary_rows': rows}), 36, 0)


if __name__ == '__main__':
    unittest.main()
