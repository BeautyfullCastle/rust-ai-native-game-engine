#!/usr/bin/env python3
"""Strict Room UI evidence checks. Source results never certify the union."""
import json
from pathlib import Path
import re
import sys

import room_project_gate as room
import room_template_gate as shared

INVENTORY = Path(__file__).with_name('room-ui-inventories.json')
FEATURES = {'default', 'project', 'project-create', 'project-export', 'room-project',
            'room-ui', 'game-ui', 'input-actions', 'sprites'}
REQUESTED_FEATURES = ['project-create', 'project-export', 'room-ui']
UI = {'profile': 'room-authored-v1', 'document': 'room.ui.json',
      'font': {'package': 'korean-game-ui', 'asset': 'OrreryKoreanUI.otf'}}
STATES = ('title', 'playing', 'key', 'won')
SIZES = ((1024, 768), (480, 800))
check_log = room.check_log


def check_ui(data):
    assert 0 < len(data) <= 65536, 'UI byte limit'
    document = shared.load_json(data.decode('utf-8'))
    shared.fields(document, ('schema', 'nodes'))
    assert type(document['schema']) is int and document['schema'] == 1, 'UI schema'
    assert type(document['nodes']) is list and len(document['nodes']) <= 32, 'UI node count'
    previous = {}
    for node in document['nodes']:
        shared.fields(node, ('id', 'kind', 'screen', 'anchor', 'offset', 'size'), ('parent',))
        identity = node['id']
        assert type(identity) is str and re.fullmatch(r'[A-Za-z0-9_-]{1,32}', identity), 'UI id'
        assert identity not in previous, 'duplicate UI id'
        assert type(node['screen']) is str and node['screen'] in ('title', 'playing', 'menu', 'terminal'), 'UI screen'
        for field, low, high in [('anchor', 0, 1000), ('offset', -4096, 4096), ('size', 1, 4096)]:
            value = node[field]
            assert type(value) is list and len(value) == 2 and all(type(n) is int and low <= n <= high for n in value), ('UI geometry', field)
        kind = node['kind']
        assert type(kind) is dict and type(kind.get('type')) is str, 'UI kind object'
        if kind['type'] == 'container':
            shared.fields(kind, ('type',))
        else:
            assert kind['type'] in ('label', 'button'), 'UI kind'
            shared.fields(kind, ('type', 'text') + (('action',) if kind['type'] == 'button' else ()), ('binding',) if kind['type'] == 'label' else ())
            text = kind['text']
            assert type(text) is str and len(text) <= 128 and all(not (ord(c) <= 31 or 127 <= ord(c) <= 159 or 0xD800 <= ord(c) <= 0xDFFF) for c in text), 'UI text'
            if kind['type'] == 'label':
                binding = kind.get('binding')
                assert binding is None or type(binding) is str and binding in ('key_acquired', 'exit_state', 'room_phase'), 'Room binding'
            else:
                assert type(kind['action']) is str and kind['action'] in ('play', 'menu', 'continue', 'restart', 'quit'), 'UI action'
        parent = node.get('parent')
        depth = 1
        if parent is not None:
            assert type(parent) is str and parent in previous, 'UI parent order'
            ancestor, parent_depth = previous[parent]
            assert ancestor['kind']['type'] == 'container' and ancestor['screen'] == node['screen'], 'UI parent kind/screen'
            depth = parent_depth + 1
        assert depth <= 4, 'UI hierarchy depth'
        previous[identity] = (node, depth)
    return document


def ui_captures(directory):
    names = [f'{state}-{width}x{height}.png' for width, height in SIZES for state in STATES]
    assert sorted(path.name for path in directory.glob('*.png')) == sorted(names), 'UI PNG inventory'
    frames = {}
    for size in SIZES:
        width, height = size
        current = {}
        for state in STATES:
            path = directory / f'{state}-{width}x{height}.png'
            frame = shared.read_png(path)
            assert frame[:2] == size, 'UI PNG dimensions'
            frames[path] = current[state] = frame
        for before, after in [('title', 'playing'), ('playing', 'key'), ('key', 'won')]:
            assert shared.different_pixels(current[before], current[after]) > 0, ('UI state unchanged', before, after)
    return frames


def check_ui_export(evidence, directory):
    export = shared.check_export(evidence, directory, ui=UI)
    files = export['payload']['files']
    ui_records = [item for item in files if item['path'] == 'project/room.ui.json' or item['role'] == 'ui_document']
    assert len(ui_records) == 1, 'missing/duplicate UI document'
    ui = ui_records[0]
    assert ui['path'] == 'project/room.ui.json' and ui['role'] == 'ui_document' and ui['mode'] == 0o644, 'UI path/role/mode'
    assert 0 < ui['bytes'] <= 65536, 'UI byte limit'
    document = check_ui((directory / 'authored-project/room.ui.json').read_bytes())
    # These captures are the bounded starter. Require the three real Room labels.
    bindings = {node['kind'].get('binding') for node in document['nodes'] if node['kind']['type'] == 'label'}
    assert {'key_acquired', 'exit_state', 'room_phase'} <= bindings, 'missing Room HUD bindings'
    packages = export['payload']['packages']
    shared.fields(packages, ('korean-game-ui', 'sample-imported-scene'))
    for digest in packages.values():
        assert type(digest) is str and re.fullmatch(r'[0-9a-f]{64}', digest), 'package digest'
    prefix = 'project/.orr/packages/objects/' + packages['korean-game-ui'] + '/'
    actual = {item['path'][len(prefix):]: item for item in files if item['path'].startswith(prefix)}
    expected = {'COPYRIGHT.txt', 'OFL.txt', 'OrreryKoreanUI.otf', 'corpus.txt', 'font-manifest.json', 'orr.package.json'}
    assert actual.keys() == expected, 'font closure'
    for name, item in actual.items():
        assert item['role'] == ('package_manifest' if name == 'orr.package.json' else 'package_asset') and item['mode'] == 0o644 and item['bytes'] > 0, 'font closure role/mode/bytes'
    font = actual['OrreryKoreanUI.otf']
    assert font['bytes'] == 1891888 and font['sha256'] == '91c7e75ac1b54a3571a305d259a2f486b88289853baef90753506855f5c5dd08', 'allowlisted font bytes'
    return export


def check_captures(evidence):
    shared.verify_tools(evidence)
    decoded = ui_captures(evidence / 'captures/room-ui')
    decoded.update(room.runtime_captures(evidence / 'export/captures'))
    check_ui_export(evidence, evidence / 'export')
    assert len(decoded) == 15
    rows = [{'path': str(path.relative_to(evidence)), 'width': frame[0], 'height': frame[1], 'sha256': shared.digest(path)}
            for path, frame in sorted(decoded.items())]
    (evidence / 'capture-manifest.json').write_text(json.dumps(rows, indent=2) + '\n')


def main():
    mode, *args = sys.argv[1:]
    if mode == 'log':
        lane, kind, path = args
        check_log(shared.load_json(INVENTORY.read_text())[lane], kind, Path(path).read_text())
    elif mode == 'pin':
        shared.pin_tools(Path(args[0]).resolve(), Path.cwd().resolve(), FEATURES, REQUESTED_FEATURES)
    elif mode == 'verify-tools':
        shared.verify_tools(Path(args[0]).resolve())
    elif mode == 'captures':
        check_captures(Path(args[0]).resolve())
    else:
        raise AssertionError(mode)


if __name__ == '__main__':
    main()
