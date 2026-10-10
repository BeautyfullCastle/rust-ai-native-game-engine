#!/usr/bin/env python3
"""Strict evidence checks for the bounded generated Room acceptance lanes."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import struct
import sys
import zlib

INVENTORY = Path(__file__).with_name('room-template-inventories.json')
TOOLS = ('orr_new_arena', 'room_escape', 'orr_export_room')
FEATURES = {'default', 'project', 'project-create', 'project-export', 'room-project', 'sprites'}
FORBIDDEN = re.compile(
    r'SKIP:|negative skipped|without capture|direct relocated execution used|'
    r'source-hiding acceptance not requested|without source hiding|source_hiding=false|'
    r'software GPU unavailable|\bskipping\b', re.I)


def digest(path):
    result = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            result.update(block)
    return result.hexdigest()


def check_log(expected, mode, text):
    assert not FORBIDDEN.search(text), 'skip/fallback diagnostic'
    if mode in ('list', 'ignored'):
        names = re.findall(r'^([A-Za-z0-9_:]+): test$', text, re.M)
        wanted = expected['names'] if mode == 'list' else expected['ignored_names']
        assert text.count(': test') == len(names), 'unparsed test inventory line'
        assert sorted(names) == sorted(wanted), (mode, names, wanted)
        return
    assert mode == 'result', mode
    summaries = re.findall(r'^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', text, re.M)
    assert text.count('test result:') == len(expected['expected_summary_rows']), 'extra/malformed/missing summary'
    assert [list(map(int, row)) for row in summaries] == expected['expected_summary_rows'], summaries
    outcomes = re.findall(r'^test ([A-Za-z0-9_:]+) \.\.\. (ok|ignored)(?:, [^\n]*)?$', text, re.M)
    wanted = [(name, 'ignored' if name in expected['ignored_names'] and not expected['run_ignored'] else 'ok')
              for name in expected['names']]
    assert sorted(outcomes) == sorted(wanted), ('executed test names/outcomes', outcomes, wanted)


def check_artifacts(rows, repo):
    manifest = (repo / 'crates/orr_sample/Cargo.toml').resolve()
    finished = [row for row in rows if row.get('reason') == 'build-finished']
    assert len(finished) == 1 and finished[0].get('success') is True, 'build did not finish successfully'
    selected = []
    for name in TOOLS:
        found = [row for row in rows if row.get('reason') == 'compiler-artifact'
                 and Path(row['manifest_path']).resolve() == manifest
                 and row['target']['name'] == name and row.get('executable')]
        assert len(found) == 1, (name, 'missing/duplicate artifact', found)
        row = found[0]
        assert row['target']['kind'] == ['bin'] and row['target']['crate_types'] == ['bin'], name
        assert Path(row['target']['src_path']).resolve() == manifest.parent / 'src/bin' / (name + '.rs'), name
        assert row['profile']['test'] is False and row['profile']['opt_level'] == '3', name
        assert set(row['features']) == FEATURES and len(row['features']) == len(FEATURES), (name, row['features'])
        package, _, version = row['package_id'].rpartition('#')
        assert package == 'path+' + manifest.parent.as_uri() and version in ('0.0.1', 'orr_sample@0.0.1'), row['package_id']
        source = Path(row['executable'])
        assert source.is_absolute() and source.is_file() and os.access(source, os.X_OK), source
        selected.append(row)
    assert len({row['executable'] for row in selected}) == 3, 'tools alias one executable'
    assert len({(Path(row['executable']).stat().st_dev, Path(row['executable']).stat().st_ino) for row in selected}) == 3, 'tools alias one inode'
    return selected


def pin_tools(evidence, repo):
    rows = [json.loads(line) for line in (evidence / 'production-tools.jsonl').read_text().splitlines() if line.strip()]
    selected = check_artifacts(rows, repo)
    copies = evidence / 'tools'
    copies.mkdir()
    records = []
    for row in selected:
        source = Path(row['executable']).resolve(strict=True)
        copy = copies / row['target']['name']
        assert not copy.exists()
        shutil.copy2(source, copy)
        copy.chmod(0o555)
        sha = digest(source)
        assert digest(copy) == sha and os.access(copy, os.X_OK)
        records.append({'name': row['target']['name'], 'source': str(source), 'copy': str(copy), 'sha256': sha,
                        'manifest': row['manifest_path'], 'package_id': row['package_id'], 'target': row['target'],
                        'features': sorted(row['features']), 'profile': row['profile'],
                        'requested_features': ['project-create', 'project-export', 'room-project']})
    (evidence / 'production-tools-artifacts.json').write_text(json.dumps(records, indent=2) + '\n')
    (evidence / 'tools.sha256').write_text(''.join(f"{item['sha256']}  {item['copy']}\n" for item in records))


def verify_tools(evidence):
    records = json.loads((evidence / 'production-tools-artifacts.json').read_text())
    assert sorted(item['name'] for item in records) == sorted(TOOLS)
    for item in records:
        path = Path(item['copy'])
        assert path.resolve() == evidence / 'tools' / item['name']
        assert not path.is_symlink() and path.is_file() and os.access(path, os.X_OK)
        assert path.stat().st_mode & (stat.S_IWUSR | stat.S_IWGRP | stat.S_IWOTH) == 0
        assert digest(path) == item['sha256'], item['name']
    return records


def read_png(path):
    """Decode the exact RGBA8/noninterlaced PNG produced by these Rust tests."""
    data = path.read_bytes()
    assert data[:8] == b'\x89PNG\r\n\x1a\n', path
    offset = 8
    chunks = []
    while offset < len(data):
        length = struct.unpack('>I', data[offset:offset + 4])[0]
        kind = data[offset + 4:offset + 8]
        payload = data[offset + 8:offset + 8 + length]
        assert len(payload) == length
        crc = struct.unpack('>I', data[offset + 8 + length:offset + 12 + length])[0]
        assert zlib.crc32(kind + payload) & 0xffffffff == crc, 'PNG CRC'
        chunks.append((kind, payload))
        offset += length + 12
    assert offset == len(data) and chunks[0][0] == b'IHDR' and chunks[-1] == (b'IEND', b'')
    assert sum(kind == b'IHDR' for kind, _ in chunks) == 1
    width, height, depth, color, compression, filtering, interlace = struct.unpack('>IIBBBBB', chunks[0][1])
    assert 0 < width <= 4096 and 0 < height <= 4096
    assert (depth, color, compression, filtering, interlace) == (8, 6, 0, 0, 0)
    raw = zlib.decompress(b''.join(payload for kind, payload in chunks if kind == b'IDAT'))
    stride = width * 4
    assert len(raw) == height * (stride + 1)
    pixels = bytearray()
    previous = bytearray(stride)
    for y in range(height):
        start = y * (stride + 1)
        method = raw[start]
        assert method in range(5)
        current = bytearray(raw[start + 1:start + 1 + stride])
        if method == 0:
            pixels.extend(current)
            previous = current
            continue
        for x in range(stride):
            left = current[x - 4] if x >= 4 else 0
            up = previous[x]
            corner = previous[x - 4] if x >= 4 else 0
            if method == 0:
                predictor = 0
            elif method == 1:
                predictor = left
            elif method == 2:
                predictor = up
            elif method == 3:
                predictor = (left + up) // 2
            else:
                estimate = left + up - corner
                distances = (abs(estimate - left), abs(estimate - up), abs(estimate - corner))
                predictor = (left, up, corner)[distances.index(min(distances))]
            current[x] = (current[x] + predictor) & 255
        pixels.extend(current)
        previous = current
    assert sum(pixels[n:n + 4] != pixels[:4] for n in range(0, len(pixels), 4)) > 100, 'blank PNG'
    return width, height, pixels


def different_pixels(a, b):
    assert a[:2] == b[:2]
    return sum(a[2][i:i + 4] != b[2][i:i + 4] for i in range(0, len(a[2]), 4))


def check_captures(evidence):
    runtime = evidence / 'export/captures'
    editor = evidence / 'captures/editor'
    names = [f'{phase}-{kind}.png' for phase in ('initial', 'moved', 'interacted') for kind in ('source', 'export')]
    names.append('no-models.png')
    assert sorted(path.name for path in runtime.glob('*.png')) == sorted(names)
    assert sorted(path.name for path in editor.glob('*.png')) == ['room-editor-bound.png', 'room-editor-models-offscreen.png']
    frames = {name: read_png(runtime / name) for name in names}
    assert all(frame[:2] == (1024, 768) for frame in frames.values())
    for phase in ('initial', 'moved', 'interacted'):
        assert (runtime / f'{phase}-source.png').read_bytes() == (runtime / f'{phase}-export.png').read_bytes(), (phase, 'source/export PNG parity')
    for phase in ('moved', 'interacted'):
        assert different_pixels(frames['initial-source.png'], frames[f'{phase}-source.png']) > 0, phase
    assert different_pixels(frames['initial-source.png'], frames['no-models.png']) > 20, 'runtime model negative control'
    a, b = (read_png(editor / name) for name in ('room-editor-bound.png', 'room-editor-models-offscreen.png'))
    assert different_pixels(a, b) > 20, 'editor model negative control'
    records = verify_tools(evidence)
    export = json.loads((evidence / 'export/orr.export.json').read_text())
    assert export['schema'] == 1
    payload = export['payload']
    assert payload['profile'] == 'room-escape-authored-linux-x86_64-v1'
    assert payload['entry'] == {'game': 'room-escape-v1', 'scene': 'room.scene.yaml', 'models': 'room.models.json'}
    runtime_record = next(item for item in records if item['name'] == 'room_escape')
    assert payload['runtime']['sha256'] == runtime_record['sha256'], 'exported runtime differs from pinned binary'
    assert payload['runtime']['bytes'] == Path(runtime_record['copy']).stat().st_size
    assert re.fullmatch(r'0x[0-9a-f]{16}', payload['initial_checksum'])
    files = payload['files']
    assert len({item['path'] for item in files}) == len(files)
    for item in files:
        if item['path'].startswith('project/'):
            relative = Path(item['path']).relative_to('project')
            assert '..' not in relative.parts
            path = evidence / 'export/authored-project' / relative
            assert path.is_file() and path.stat().st_size == item['bytes'] and digest(path) == item['sha256'], relative
    decoded = {runtime / name: frame for name, frame in frames.items()}
    decoded.update({editor / 'room-editor-bound.png': a, editor / 'room-editor-models-offscreen.png': b})
    entries = [{'path': str(path.relative_to(evidence)), 'width': frame[0], 'height': frame[1], 'sha256': digest(path)}
               for path, frame in sorted(decoded.items())]
    (evidence / 'capture-manifest.json').write_text(json.dumps(entries, indent=2) + '\n')


def main():
    mode, *args = sys.argv[1:]
    if mode == 'log':
        lane, kind, path = args
        check_log(json.loads(INVENTORY.read_text())[lane], kind, Path(path).read_text())
    elif mode == 'pin':
        pin_tools(Path(args[0]).resolve(), Path.cwd().resolve())
    elif mode == 'verify-tools':
        verify_tools(Path(args[0]).resolve())
    elif mode == 'captures':
        check_captures(Path(args[0]).resolve())
    else:
        raise AssertionError(mode)


if __name__ == '__main__':
    main()
