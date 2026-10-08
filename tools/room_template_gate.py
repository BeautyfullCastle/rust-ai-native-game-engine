#!/usr/bin/env python3
"""Strict evidence checks for the bounded generated Room acceptance lanes."""
import hashlib
import json
import math
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


def load_json(text):
    """Keep duplicate keys and non-JSON constants observable at every depth."""
    def object_pairs(pairs):
        result = {}
        for key, value in pairs:
            assert key not in result, ('duplicate JSON field', key)
            result[key] = value
        return result

    def invalid_constant(value):
        raise AssertionError(('non-JSON number', value))

    return json.loads(text, object_pairs_hook=object_pairs, parse_constant=invalid_constant)


def fields(value, required, optional=()):
    assert type(value) is dict, 'expected JSON object'
    assert set(required) <= value.keys() <= set(required) | set(optional), ('JSON fields', value.keys())


def strings(value):
    return type(value) is list and all(type(item) is str for item in value)


def cargo_target(target):
    fields(target, ('kind', 'crate_types', 'name', 'src_path', 'edition', 'doc', 'doctest', 'test'),
           ('required-features',))
    for key in ('kind', 'crate_types'):
        assert strings(target[key]) and target[key], ('target', key)
    for key in ('name', 'src_path', 'edition'):
        assert type(target[key]) is str and target[key], ('target', key)
    assert Path(target['src_path']).is_absolute(), 'relative target source'
    for key in ('doc', 'doctest', 'test'):
        assert type(target[key]) is bool, ('target', key)
    if 'required-features' in target:
        assert strings(target['required-features']), 'target required-features'


def check_cargo_rows(rows):
    """Validate all supported Cargo records, including unrelated dependencies."""
    assert type(rows) is list and rows, 'empty Cargo JSON stream'
    for row in rows:
        assert type(row) is dict and type(row.get('reason')) is str, 'malformed Cargo record'
        reason = row['reason']
        if reason == 'build-finished':
            fields(row, ('reason', 'success'))
            assert type(row['success']) is bool, 'build success must be boolean'
        elif reason == 'build-script-executed':
            fields(row, ('reason', 'package_id', 'linked_libs', 'linked_paths', 'cfgs', 'env', 'out_dir'))
            for key in ('linked_libs', 'linked_paths', 'cfgs'):
                assert strings(row[key]), ('build script', key)
            assert type(row['env']) is list and all(strings(pair) and len(pair) == 2 for pair in row['env']), 'build script env'
            assert type(row['out_dir']) is str and Path(row['out_dir']).is_absolute(), 'build script out_dir'
        elif reason in ('compiler-artifact', 'compiler-message'):
            common = ('reason', 'package_id', 'manifest_path', 'target')
            extra = ('profile', 'features', 'filenames', 'executable', 'fresh') if reason == 'compiler-artifact' else ('message',)
            fields(row, common + extra)
            assert type(row['manifest_path']) is str and Path(row['manifest_path']).is_absolute(), 'artifact manifest path'
            cargo_target(row['target'])
            if reason == 'compiler-message':
                message = row['message']
                fields(message, ('message', 'code', 'level', 'spans', 'children'), ('rendered', '$message_type'))
                assert type(message['message']) is str and type(message['level']) is str, 'compiler diagnostic'
                assert type(message['spans']) is list and type(message['children']) is list, 'compiler diagnostic details'
                assert message['code'] is None or type(message['code']) is dict, 'compiler diagnostic code'
                assert message.get('rendered') is None or type(message['rendered']) is str, 'rendered diagnostic'
            else:
                profile = row['profile']
                fields(profile, ('opt_level', 'debuginfo', 'debug_assertions', 'overflow_checks', 'test'))
                assert type(profile['opt_level']) is str, 'profile optimization level'
                debug = profile['debuginfo']
                assert (debug is None or type(debug) is int and debug in (0, 1, 2)
                        or type(debug) is str and debug in ('line-directives-only', 'line-tables-only')), 'profile debuginfo'
                for key in ('debug_assertions', 'overflow_checks', 'test'):
                    assert type(profile[key]) is bool, ('profile', key)
                assert strings(row['features']) and len(set(row['features'])) == len(row['features']), 'artifact features'
                assert strings(row['filenames']) and all(Path(path).is_absolute() for path in row['filenames']), 'artifact filenames'
                assert row['executable'] is None or type(row['executable']) is str and Path(row['executable']).is_absolute(), 'artifact executable'
                assert type(row['fresh']) is bool, 'artifact fresh'
        else:
            raise AssertionError(('unknown Cargo record', reason))
        if reason != 'build-finished':
            assert type(row['package_id']) is str and row['package_id'], 'Cargo package id'
    assert rows[-1]['reason'] == 'build-finished', 'records after build-finished'


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


def check_artifacts(rows, repo, features=FEATURES):
    check_cargo_rows(rows)
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
        assert set(row['features']) == features and len(row['features']) == len(features), (name, row['features'])
        package, _, version = row['package_id'].rpartition('#')
        assert package == 'path+' + manifest.parent.as_uri() and version in ('0.0.1', 'orr_sample@0.0.1'), row['package_id']
        source = Path(row['executable'])
        assert str(source) in row['filenames'], 'executable missing from artifact filenames'
        assert source.is_absolute() and not source.is_symlink() and source.is_file() and os.access(source, os.X_OK), source
        selected.append(row)
    assert len({row['executable'] for row in selected}) == 3, 'tools alias one executable'
    assert len({(Path(row['executable']).stat().st_dev, Path(row['executable']).stat().st_ino) for row in selected}) == 3, 'tools alias one inode'
    return selected


def pin_tools(evidence, repo, features=FEATURES, requested_features=None):
    rows = [load_json(line) for line in (evidence / 'production-tools.jsonl').read_text().splitlines() if line.strip()]
    selected = check_artifacts(rows, repo, features)
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
                        'requested_features': requested_features or ['project-create', 'project-export', 'room-project']})
    (evidence / 'production-tools-artifacts.json').write_text(json.dumps(records, indent=2) + '\n')
    (evidence / 'tools.sha256').write_text(''.join(f"{item['sha256']}  {item['copy']}\n" for item in records))


def verify_tools(evidence):
    records = load_json((evidence / 'production-tools-artifacts.json').read_text())
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


def check_camera(data):
    """Validate the persisted schema-1 sidecar, using the authored f32 bounds."""
    assert 0 < len(data) <= 4096, 'camera byte limit'
    document = load_json(data.decode('utf-8'))
    fields(document, ('schema', 'target', 'yaw', 'pitch', 'distance', 'projection'))
    assert type(document['schema']) is int and document['schema'] == 1, 'camera schema'

    def bounded(value, low, high):
        assert type(value) in (int, float), 'camera number'
        # Rust reads these JSON numbers directly into f32 before validation.
        def f32(number):
            try:
                return struct.unpack('f', struct.pack('f', number))[0]
            except (OverflowError, struct.error):
                raise AssertionError('camera number exceeds f32') from None
        value = f32(value)
        assert math.isfinite(value) and f32(low) <= value <= f32(high), 'camera bounds'

    assert type(document['target']) is list and len(document['target']) == 3, 'camera target'
    for value in document['target']:
        bounded(value, -64, 64)
    bounded(document['yaw'], -math.pi, math.pi)
    bounded(document['pitch'], -1.4, 1.4)
    bounded(document['distance'], 1, 128)
    projection = document['projection']
    assert type(projection) is dict and type(projection.get('type')) is str, 'camera projection object/tag'
    if projection['type'] == 'orthographic':
        fields(projection, ('type', 'half_height'))
        bounded(projection['half_height'], 1, 64)
    else:
        assert projection['type'] == 'perspective', 'camera projection type'
        fields(projection, ('type', 'fov_y_degrees'))
        bounded(projection['fov_y_degrees'], 20, 100)
    return document


def check_export(evidence, directory, require_camera=True, ui=None):
    """Check saved export/source closure independently of capture layout."""
    records = verify_tools(evidence)
    export = load_json((directory / 'orr.export.json').read_text())
    assert type(export) is dict and type(export['schema']) is int and export['schema'] == 1
    payload = export['payload']
    assert payload['profile'] == 'room-escape-authored-linux-x86_64-v1'
    entry = {'game': 'room-escape-v1', 'scene': 'room.scene.yaml', 'models': 'room.models.json'}
    if require_camera:
        entry['camera'] = 'room.camera.json'
    if ui is not None:
        entry['ui'] = ui
    assert payload['entry'] == entry, 'export entry'
    runtime_record = next(item for item in records if item['name'] == 'room_escape')
    fields(payload['runtime'], ('sha256', 'bytes'))
    assert payload['runtime']['sha256'] == runtime_record['sha256'], 'exported runtime differs from pinned binary'
    assert type(payload['runtime']['bytes']) is int and payload['runtime']['bytes'] == Path(runtime_record['copy']).stat().st_size
    assert type(payload['initial_checksum']) is str and re.fullmatch(r'0x[0-9a-f]{16}', payload['initial_checksum'])
    files = payload['files']
    assert type(files) is list and files, 'export files'
    for item in files:
        fields(item, ('path', 'role', 'mode', 'bytes', 'sha256'))
        assert type(item['path']) is str and item['path'], 'export path'
        relative = Path(item['path'])
        assert (not relative.is_absolute() and '\\' not in item['path']
                and all(part not in ('', '.', '..') for part in item['path'].split('/'))), 'export path traversal/alias'
        assert type(item['role']) is str and item['role'], 'export file role'
        assert type(item['mode']) is int and item['mode'] in (0o644, 0o755), 'export file mode'
        assert type(item['bytes']) is int and item['bytes'] >= 0, 'export file bytes'
        assert type(item['sha256']) is str and re.fullmatch(r'[0-9a-f]{64}', item['sha256']), 'export file digest'
        if item['path'] == 'project/room.camera.json' or item['role'] == 'camera_sidecar':
            assert 0 < item['bytes'] <= 4096, 'camera byte limit'
    assert len({item['path'] for item in files}) == len(files), 'duplicate export path'
    project = directory / 'authored-project'
    assert not project.is_symlink() and project.is_dir(), 'saved authored project'
    for item in files:
        if item['path'].startswith('project/'):
            relative = Path(item['path']).relative_to('project')
            path = project / relative
            assert all(not (project / Path(*relative.parts[:n])).is_symlink()
                       for n in range(1, len(relative.parts) + 1)), 'symlink in saved source'
            assert path.is_file() and path.stat().st_size == item['bytes'] and digest(path) == item['sha256'], relative
    manifest_records = [item for item in files if item['path'] == 'project/orr.project.json' or item['role'] == 'project_manifest']
    assert len(manifest_records) == 1 and manifest_records[0]['path'] == 'project/orr.project.json' and manifest_records[0]['role'] == 'project_manifest', 'project manifest record'
    project_manifest = load_json((project / 'orr.project.json').read_text())
    assert type(project_manifest['schema']) is int and project_manifest['schema'] == 2, 'saved project schema'
    assert project_manifest['entry'] == entry, 'saved project entry differs from export'
    camera_records = [item for item in files if item['path'] == 'project/room.camera.json' or item['role'] == 'camera_sidecar']
    if require_camera:
        assert len(camera_records) == 1, 'missing/duplicate camera sidecar record'
        camera_record = camera_records[0]
        assert camera_record['path'] == 'project/room.camera.json' and camera_record['role'] == 'camera_sidecar', 'camera path/role'
        assert 0 < camera_record['bytes'] <= 4096 and camera_record['mode'] == 0o644, 'camera byte limit/mode'
        camera_path = project / 'room.camera.json'
        with camera_path.open('rb') as stream:
            data = stream.read(4097)
        assert len(data) == camera_record['bytes'] and hashlib.sha256(data).hexdigest() == camera_record['sha256'], 'saved camera bytes'
        check_camera(data)
    else:
        assert not camera_records, 'unexpected legacy camera sidecar'
    return export


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
    check_export(evidence, evidence / 'export')
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
