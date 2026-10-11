#!/usr/bin/env python3
"""Exact Room/camera acceptance evidence, including attributable unified harnesses."""
import json
from pathlib import Path
import re
import sys

import room_template_gate as shared

INVENTORY = Path(__file__).with_name('room-project-inventories.json')
CAMERA_CAPTURES = ('landscape', 'invalid-retained', 'manual', 'reset', 'projection-error', 'portrait', 'resize-return')


def check_log(expected, mode, text):
    if 'harness_rows' not in expected:
        shared.check_log(expected, mode, text)
        if mode == 'result':
            summaries = re.findall(r'^test result: ok\. \d+ passed; \d+ failed; \d+ ignored; \d+ measured; \d+ filtered out; finished in [0-9.]+s$', text, re.M)
            assert len(summaries) == len(expected['expected_summary_rows']), 'malformed full summary'
            assert len(re.findall(r'^test ', text, re.M)) == len(expected['names']) + len(summaries), 'unparsed test outcome'
        return
    # Cargo identifies each library harness before its rows. The editor zero is
    # legal only in this exact feature-unified filter, never as a generic zero.
    pattern = r'^\s*Running unittests src/lib\.rs \(([^\n]+)\)[ \t]*$'
    headers = list(re.finditer(pattern, text, re.M))
    assert len(re.findall(r'^\s*Running ', text, re.M)) == 2, 'extra/malformed harness'
    assert len(headers) == len(expected['harness_rows']) == 2, 'missing/extra/unattributed library harness'
    preamble = text[:headers[0].start()]
    assert not re.search(r'^test |test result:|: test$', preamble, re.M), 'outcome before first harness'
    assert not shared.FORBIDDEN.search(text), 'skip/fallback diagnostic'
    for index, (header, harness) in enumerate(zip(headers, expected['harness_rows'])):
        assert re.fullmatch(re.escape(harness['package']) + r'-[0-9a-f]+', Path(header[1]).name), 'wrong/reordered harness'
        block = text[header.end():headers[index + 1].start() if index + 1 < len(headers) else len(text)]
        assert not re.search(r'^\s*Running ', block, re.M), 'extra harness'
        one = {'names': harness['names'], 'ignored_names': [], 'run_ignored': False,
               'expected_summary_rows': [harness['summary']]}
        check_log(one, mode, block)


def runtime_captures(directory):
    names = [f'{phase}-{kind}.png' for phase in ('initial', 'moved', 'interacted') for kind in ('source', 'export')]
    names.append('no-models.png')
    assert sorted(path.name for path in directory.glob('*.png')) == sorted(names), 'runtime PNG inventory'
    frames = {name: shared.read_png(directory / name) for name in names}
    assert all(frame[:2] == (1024, 768) for frame in frames.values()), 'runtime capture dimensions'
    for phase in ('initial', 'moved', 'interacted'):
        assert (directory / f'{phase}-source.png').read_bytes() == (directory / f'{phase}-export.png').read_bytes(), 'source/export PNG parity'
    for phase in ('moved', 'interacted'):
        assert shared.different_pixels(frames['initial-source.png'], frames[f'{phase}-source.png']) > 0, phase
    assert shared.different_pixels(frames['initial-source.png'], frames['no-models.png']) > 20, 'runtime model negative control'
    return {directory / name: frame for name, frame in frames.items()}


def camera_captures(directory):
    names = ['authored-camera-' + name + '.png' for name in CAMERA_CAPTURES]
    assert sorted(path.name for path in directory.glob('*.png')) == sorted(names), 'camera PNG inventory'
    frames = {name: shared.read_png(directory / ('authored-camera-' + name + '.png')) for name in CAMERA_CAPTURES}
    first = frames['landscape']
    # The landscape *window* includes editor sidebars, so its captured viewport
    # can still be taller than wide. The portrait resize must narrow and lengthen it.
    portrait = frames['portrait']
    assert portrait[1] > portrait[0], 'portrait viewport expected'
    assert portrait[0] < first[0] and portrait[1] > first[1], 'portrait resize dimensions'
    for name in ('invalid-retained', 'reset', 'projection-error', 'resize-return'):
        assert frames[name] == first, (name, 'last valid camera pixels not preserved')
    assert shared.different_pixels(first, frames['manual']) > 0, 'manual camera must change pixels'
    return {directory / ('authored-camera-' + name + '.png'): frame for name, frame in frames.items()}


def check_captures(evidence):
    shared.verify_tools(evidence)
    decoded = runtime_captures(evidence / 'export/captures')
    decoded.update(runtime_captures(evidence / 'camera-export/captures'))
    decoded.update(camera_captures(evidence / 'captures/camera-editor'))
    editor = evidence / 'captures/legacy-editor'
    names = ['room-editor-bound.png', 'room-editor-models-offscreen.png']
    assert sorted(path.name for path in editor.glob('*.png')) == names, 'legacy editor PNG inventory'
    a, b = (shared.read_png(editor / name) for name in names)
    assert shared.different_pixels(a, b) > 20, 'legacy editor model negative control'
    decoded.update({editor / names[0]: a, editor / names[1]: b})
    shared.check_export(evidence, evidence / 'export', require_camera=False)
    shared.check_export(evidence, evidence / 'camera-export', require_camera=True)
    assert len(decoded) == 23
    records = [{'path': str(path.relative_to(evidence)), 'width': frame[0], 'height': frame[1], 'sha256': shared.digest(path)}
               for path, frame in sorted(decoded.items())]
    (evidence / 'capture-manifest.json').write_text(json.dumps(records, indent=2) + '\n')


def main():
    mode, *args = sys.argv[1:]
    if mode == 'log':
        lane, kind, path = args
        check_log(json.loads(INVENTORY.read_text())[lane], kind, Path(path).read_text())
    elif mode == 'pin':
        shared.pin_tools(Path(args[0]).resolve(), Path.cwd().resolve())
    elif mode == 'verify-tools':
        shared.verify_tools(Path(args[0]).resolve())
    elif mode == 'captures':
        check_captures(Path(args[0]).resolve())
    else:
        raise AssertionError(mode)


if __name__ == '__main__':
    main()
