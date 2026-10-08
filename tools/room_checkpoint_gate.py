#!/usr/bin/env python3
"""Exact checkpoint inventories and evidence; fixtures never certify execution."""
import json
from pathlib import Path
import re
import sys

import room_template_gate as shared

INVENTORY = Path(__file__).with_name('room-checkpoint-inventories.json')


def check_log(expected, mode, text, passed, ignored):
    assert passed > 0 and [passed, ignored] == [expected['passed'], expected['ignored']]
    assert len(expected['names']) > 0 and len(set(expected['names'])) == len(expected['names'])
    assert not shared.FORBIDDEN.search(text), 'skip/fallback diagnostic'
    if mode in ('list', 'ignored'):
        names = re.findall(r'^([A-Za-z0-9_:]+): test$', text, re.M)
        wanted = expected['names'] if mode == 'list' else expected['ignored_names']
        assert text.count(': test') == len(names), 'unparsed inventory'
        assert sorted(names) == sorted(wanted), (mode, names, wanted)
        return
    assert mode == 'result', mode
    # The package FIFO test deliberately launches one visible 1-test subprocess.
    # Require that row as well as the full positive parent; never accept any zero.
    rows = re.findall(r'^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; \d+ measured; \d+ filtered out; finished in [0-9.]+s$', text, re.M)
    assert text.count('test result:') == len(expected['expected_summary_rows']), 'extra/malformed/missing summary'
    assert [list(map(int, row)) for row in rows] == expected['expected_summary_rows'], rows
    assert all(int(row[0]) > 0 and int(row[1]) == 0 for row in rows)


def check_captures(directory):
    phases = ('chooser', 'resumed-unlocked', 'won')
    sizes = ((1024, 768), (480, 800))
    wanted = sorted(f'checkpoint-{phase}-{w}x{h}.png' for phase in phases for w, h in sizes)
    assert sorted(p.name for p in directory.glob('*.png')) == wanted, 'checkpoint capture inventory'
    for width, height in sizes:
        frames = [shared.read_png(directory / f'checkpoint-{phase}-{width}x{height}.png') for phase in phases]
        assert all(frame[:2] == (width, height) for frame in frames), 'checkpoint capture dimensions'
        for first, second in zip(frames, frames[1:]):
            assert shared.different_pixels(first, second) > 0, 'checkpoint phase did not change pixels'


def check_export(directory, mode):
    assert mode in ('gpu-export', 'syscall')
    result = shared.load_json((directory / 'result.json').read_text())
    assert result == {'source_hidden': True, 'bundle_readonly': True,
                      'bundle_hashes_unchanged': True, 'app_processes': 4,
                      'native_window': False, 'physical_gpu': False,
                      'syscall_proof': mode == 'syscall'}, result
    before = shared.load_json((directory / 'bundle-before.json').read_text())
    after = shared.load_json((directory / 'bundle-after.json').read_text())
    assert before and before == after and 'project/orr.project.json' in before
    assert (directory / 'orr.export.json').is_file()
    shared.read_png(directory / 'smoke.png')
    child = 'room_app::room_checkpoint_acceptance_tests::checkpoint_exported_app_process'
    expected = {'passed': 1, 'ignored': 0, 'names': [child], 'ignored_names': [child],
                'expected_summary_rows': [[1, 0, 0]]}
    for phase in ('acquire', 'resume', 'newgame', 'empty'):
        text = (directory / (phase + '.log')).read_text()
        check_log(expected, 'result', text, 1, 0)
        assert f'mode={phase}' in text and ' profile=' in text
    if mode == 'syscall':
        # The source exporter already checks negative probes and positive writes.
        # Require every persisted trace as evidence too, including all relaunches.
        for phase in ('readonly-export-smoke', 'acquire', 'resume', 'newgame', 'empty'):
            trace = directory / (phase + '.trace')
            assert trace.is_file() and trace.stat().st_size > 0, ('missing trace', phase)


def main():
    mode, *args = sys.argv[1:]
    if mode == 'log':
        lane, kind, path, passed, ignored = args
        check_log(json.loads(INVENTORY.read_text())[lane], kind, Path(path).read_text(), int(passed), int(ignored))
    elif mode == 'captures':
        check_captures(Path(args[0]))
    elif mode == 'export':
        check_export(Path(args[0]), args[1])
    else:
        raise AssertionError(mode)


if __name__ == '__main__':
    main()
