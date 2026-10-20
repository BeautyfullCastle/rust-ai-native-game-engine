#!/usr/bin/env python3
"""Source-bound grouped terrain ramp acceptance. Python checks file/pixel integrity;
Rust checks actual Frame decoding/checksum and independent authoritative navigation.
No retry or replacement success after an original failure.
"""
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import sys
import traceback
import urllib.parse
import animation_pose_bake_gate as source
import static_model_extract_gate as runner
import terrain_ramp_contract as contract
from tilemap_authoring_gate import capture

PHASES = ['01-before', '01b-picked-endpoints', '02-stage', '03-applied',
          '03b-undo', '03c-redo', 'negative-wrong-axis', 'negative-wrong-slope',
          '04-built', '04b-reopened', '05-play-paused',
          '05b-middle', '06-arrival', '07-stale', '08-stale-play-refused']
CORRIDOR = list(range(66, 79))
WAYPOINTS = [[-196608, 0, 0], [-163840, 8192, 32768],
             [-131072, 16384, 32768], [-98304, 24576, 32768],
             [-65536, 32768, 32768], [-32768, 40960, 32768],
             [0, 49152, 32768], [32768, 57344, 32768],
             [65536, 65536, 32768], [98304, 73728, 32768],
             [131072, 81920, 32768], [163840, 90112, 32768],
             [196608, 98304, 32768], [196608, 98304, 0]]
SUFFIXES = ['.png', '.rgba', '-app.png', '-app.rgba', '.frame', '.json']
SCOPE = ('existing point-agent heightfield navigation, not polygon navmesh or rigid-body collision; '
         'paused deterministic steps, not timing/performance evidence')
require = contract.require


def read(path):
    require(path.is_file() and not path.is_symlink(), f'Missing regular evidence: {path}')
    require(0 < path.stat().st_size <= 64 * 1024**2, f'Invalid evidence size: {path}')
    return path.read_bytes()


def document(path):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, 'Duplicate JSON key')
            result[key] = value
        return result
    return json.loads(read(path), object_pairs_hook=unique)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def fields(value, expected):
    require(isinstance(value, dict) and all(type(value.get(k)) is type(v) and value[k] == v
                                          for k, v in expected.items()), 'Receipt fields mismatch')


def position(value):
    p = value.get('position_raw')
    require(isinstance(p, list) and len(p) == 3 and all(type(n) is int for n in p), 'Invalid typed position')
    x, y, z = p
    require(-196608 <= x <= 196608 and 0 <= z <= 32768 and y == (x + 196608) // 4,
            'Reported navigation profile mismatch')
    path = value.get('path')
    fields(path, {'corridor': CORRIDOR, 'waypoints_raw': WAYPOINTS})
    require(all(type(k) is int for k in path['corridor']) and
            all(isinstance(point, list) and len(point) == 3 and all(type(n) is int for n in point)
                for point in path['waypoints_raw']), 'Invalid typed route geometry')
    index = value.get('next_waypoint')
    require(type(index) is int and 1 <= index <= len(WAYPOINTS) and
            (index == len(WAYPOINTS)) == (value.get('status') == 'Arrived'), 'Invalid next waypoint')
    require(value.get('status') != 'Arrived' or p == WAYPOINTS[-1], 'Arrived away from exact endpoint')
    key = CORRIDOR[min(index - 1, len(CORRIDOR) - 1)]
    cell_x = (key // 2) % 8
    u, v = x - (cell_x - 4) * 65536, z
    require(0 <= u <= 65536 and 0 <= v <= 65536 and
            (u <= v if key % 2 == 0 else v <= u), 'Reported point outside canonical corridor triangle')
    return p


def identity(value):
    require(type(value.get('tick')) is int and value['tick'] >= 0 and
            isinstance(value.get('checksum'), str) and re.fullmatch('[0-9a-f]{16}', value['checksum']),
            'Invalid Frame identity')


def receipts(root, head):
    complete = document(root / 'complete.json')
    fields(complete, {'complete': True, 'source_sha': head, 'scope': SCOPE,
        'phases': PHASES, 'expected_corridor': CORRIDOR, 'expected_waypoints_raw': WAYPOINTS,
        'independent_raw_profile': [0, 16384, 32768, 49152, 65536, 81920, 98304],
        'pointer_endpoints': True, 'stage_cancel_apply': True, 'single_undo_redo': True,
        'save_fresh_reopen': True, 'stale_play_refused': True})
    require(all(type(k) is int for k in complete['expected_corridor']) and
            all(isinstance(p, list) and all(type(n) is int for n in p)
                for p in complete['expected_waypoints_raw']), 'Invalid typed expected route geometry')
    trace = document(root / 'traversal-partial.json')
    require(isinstance(trace, list) and 3 <= len(trace) <= 64 and complete.get('authoritative_frame_trace') == trace,
            'Missing or inconsistent complete traversal')
    drains = document(root / 'render-drains-partial.json')
    require(isinstance(drains, list) and len(drains) == 2 * len(trace) and
            complete.get('render_drains') == drains, 'Missing or inconsistent completed render drains')
    for i, row in enumerate(trace):
        before, after = drains[2*i:2*i+2]
        require(isinstance(before, dict) and set(before) == {'step', 'tick', 'phase'} and
                isinstance(after, dict) and set(after) == {'step', 'tick', 'phase', 'ok', 'dimensions'},
                'Unexpected render drain marker fields')
        fields(before, {'step': i, 'tick': row['tick'], 'phase': 'before-render'})
        fields(after, {'step': i, 'tick': row['tick'], 'phase': 'after-render',
                       'ok': True, 'dimensions': [1440, 1200]})
        require(all(type(n) is int for n in after['dimensions']), 'Invalid render drain dimensions')
    expected = {'complete.json', 'adapter.json', 'reopen-adapter.json', 'traversal-partial.json',
                'render-drains-partial.json'}
    expected |= {phase + ext for phase in PHASES for ext in SUFFIXES}
    expected |= {f'step-{i:02}.frame' for i in range(len(trace))}
    require({p.name for p in root.iterdir()} == expected, 'Unexpected capture census')
    manifest = complete.get('files')
    require(isinstance(manifest, dict) and set(manifest) == expected - {'complete.json'}, 'Manifest census mismatch')
    for name in sorted(expected):
        data = read(root / name)
        if name != 'complete.json':
            fields(manifest[name], {'bytes': len(data), 'sha256': sha(data)})
    first = document(root / 'adapter.json')
    reopened = document(root / 'reopen-adapter.json')
    for value in (first, reopened):
        contract.hosted_adapter(value, head)
        fields(value, {'phase': 'adapter-created'})
    require(first == reopened, 'Reopen adapter identity differs')
    phases, raw = {}, {}
    for phase in PHASES:
        value = document(root / (phase + '.json'))
        phases[phase] = value
        fields(value, {'phase': phase, 'partial': True, 'source_sha': head,
                       'adapter': first['adapter']})
        identity(value)
        fields(value, {'display_tick': value['tick'], 'display_checksum': value['checksum']})
        require(value.get('mode') == ('Play' if phase in ('05-play-paused', '05b-middle', '06-arrival') else 'Edit'), 'Unexpected phase mode')
        require(type(value.get('route_stale')) is bool and isinstance(value.get('terrain_revision'), list) and len(value['terrain_revision']) == 32 and all(type(n) is int and 0 <= n <= 255 for n in value['terrain_revision']), 'Invalid terrain state')
        require(isinstance(value.get('terrain_bytes_sha256'), str) and
                re.fullmatch('[0-9a-f]{64}', value['terrain_bytes_sha256']), 'Invalid terrain bytes digest')
        require(all(type(value.get(k)) is bool for k in
                    ('terrain_can_undo', 'terrain_can_redo', 'terrain_dirty')), 'Invalid terrain history types')
        fields(value, {'terrain_error': None})
        require(isinstance(value.get('diagnostics'), list) and all(isinstance(d, dict) and type(d.get('error')) is bool and isinstance(d.get('text'), str) for d in value['diagnostics']), 'Missing diagnostic census')
        require(value.get('navigation_error') is None or isinstance(value['navigation_error'], str), 'Invalid navigation diagnostic')
        require(sha(read(root / (phase + '.frame'))) == value.get('frame_sha256'), 'Frame bytes hash mismatch')
        for suffix, dimensions, digest in [('', 'viewport', 'rgba_sha256'), ('-app', 'composed_size', 'composed_rgba_sha256')]:
            size = value.get(dimensions)
            require(isinstance(size, list) and len(size) == 2 and all(type(n) is int and 1 <= n <= 4096 for n in size), 'Invalid bounded capture dimensions')
            pixels = read(root / (phase + suffix + '.rgba'))
            require(len(pixels) == size[0] * size[1] * 4 and sha(pixels) == value.get(digest), 'Raw RGBA integrity mismatch')
            require(capture(root / (phase + suffix + '.png'), size)[1] == pixels, 'Decoded PNG/raw pixels differ')
            if not suffix:
                raw[phase] = pixels
    # Reported terrain identities/history are source-bound receipts, not independently
    # decoded terrain assets. Rust still proves actual bytes and single transaction.
    initial, authored = phases['01-before'], phases['03-applied']
    require(initial['terrain_bytes_sha256'] != authored['terrain_bytes_sha256'] and
            initial['terrain_revision'] != authored['terrain_revision'], 'Vacuous terrain change')
    for group, reference in [(['01b-picked-endpoints', '02-stage', '03b-undo'], initial),
                             (['03c-redo', '04-built', '04b-reopened', '05-play-paused',
                               '05b-middle', '06-arrival'], authored)]:
        for phase in group:
            fields(phases[phase], {key: reference[key] for key in
                                  ('terrain_bytes_sha256', 'terrain_revision')})
    history = {
        '01-before': (False, False, False), '01b-picked-endpoints': (False, False, False),
        '02-stage': (False, False, False), '03-applied': (True, False, True),
        '03b-undo': (False, True, False), '03c-redo': (True, False, True),
        'negative-wrong-axis': (True, False, True), 'negative-wrong-slope': (True, False, True),
        '04-built': (True, True, False), '04b-reopened': (False, False, False),
        '05-play-paused': (False, False, False), '05b-middle': (False, False, False),
        '06-arrival': (False, False, False), '07-stale': (True, False, True),
        '08-stale-play-refused': (True, False, True),
    }
    for phase, flags in history.items():
        fields(phases[phase], dict(zip(('terrain_can_undo', 'terrain_can_redo', 'terrain_dirty'), flags)))
    for phase in ('negative-wrong-axis', 'negative-wrong-slope', '07-stale'):
        require(phases[phase]['terrain_bytes_sha256'] != authored['terrain_bytes_sha256'] and
                phases[phase]['terrain_revision'] != authored['terrain_revision'], 'Missing terrain control mutation')
    fields(phases['08-stale-play-refused'], {key: phases['07-stale'][key] for key in
                                          ('terrain_bytes_sha256', 'terrain_revision')})
    require(raw['01-before'] != raw['03-applied'], 'Vacuous applied capture')
    after = raw['03-applied']
    for phase in ('negative-wrong-axis', 'negative-wrong-slope'):
        other = raw[phase]
        require(len(after) == len(other), 'Negative capture dimensions differ')
        changed = sum(any(abs(after[i+j] - other[i+j]) > 12 for j in range(3)) for i in range(0, len(after), 4))
        require(changed > 100, 'Vacuous wrong-axis/slope pixel discriminator')
    require(phases['07-stale']['route_stale'] and phases['08-stale-play-refused']['route_stale'], 'Stale admission not retained')
    for i, row in enumerate(trace):
        fields(row, {'step': i, 'tick': i, 'file': f'step-{i:02}.frame'})
        identity(row)
        require(sha(read(root / row['file'])) == row.get('sha256'), 'Traversal Frame hash mismatch')
        p = position(row)
        require(row.get('status') in ('Moving', 'Arrived') and (row['status'] == 'Arrived') == (i == len(trace)-1), 'Invalid arrival sequence')
        if i:
            previous = trace[i-1]['position_raw']
            require(p[0] >= previous[0] and p != previous, 'Traversal did not advance')
            require(row['next_waypoint'] >= trace[i-1]['next_waypoint'], 'Waypoint progress regressed')
            if previous[0] == 196608:
                require(p[2] < previous[2], 'Final vertical leg did not approach goal')
    require(trace[0]['position_raw'] == [-196608, 0, 0] and trace[-1]['position_raw'] == [196608, 98304, 0], 'Traversal endpoints differ')
    for phase, index in [('05-play-paused', 0), ('05b-middle', 1), ('06-arrival', len(trace)-1)]:
        value, row = phases[phase], trace[index]
        for key in ('tick', 'checksum', 'position_raw', 'status', 'next_waypoint', 'path'):
            require(value[key] == row[key], 'Phase/trace identity differs')
        require(read(root / (phase + '.frame')) == read(root / row['file']), 'Phase/trace actual Frame bytes differ')
        position(value)
    # These checks establish retained-file/receipt consistency, NOT independent Frame decoding.
    return {'phases': len(PHASES), 'frames_in_traversal': len(trace), 'files': len(expected)}


def run(label, argv):
    source.source_fence()
    output = runner.run(label, argv, seconds=2400)
    source.source_fence()
    return output


def pin(output, root):
    records = [json.loads(line) for line in output.splitlines() if line.startswith('{')]
    rows = [v for v in records if v.get('reason') == 'compiler-artifact' and v.get('profile', {}).get('test') and v.get('executable')]
    require(len(rows) == 2, 'Exactly two grouped producer executables required')
    bins = {}
    for value in rows:
        target = value['target']; name = target['name']
        require(name in ('orr_editor', 'terrain_ramp') and name not in bins, 'Unexpected/duplicate producer')
        kind, src, crate_types = ('lib', 'src/lib.rs', ['lib']) if name == 'orr_editor' else ('test', 'tests/terrain_ramp.rs', ['bin'])
        crate = root / 'crates/orr_editor'
        fields(target, {'kind': [kind], 'crate_types': crate_types, 'src_path': str(crate / src)})
        fields(value, {'manifest_path': str(crate / 'Cargo.toml'), 'fresh': False})
        require(sorted(value['features']) == ['models', 'navigation', 'terrain'], 'Unexpected producer feature closure')
        require(value['profile']['test'] is True and value['profile']['opt_level'] == '3', 'Wrong test profile')
        package = value.get('package_id', '')
        require(package.startswith('path+'), 'Nonlocal package producer')
        uri = urllib.parse.urlsplit(package[5:].split('#', 1)[0])
        require(uri.scheme == 'file' and not uri.netloc and Path(urllib.parse.unquote(uri.path)) == crate, 'Wrong package identity')
        original = Path(value['executable'])
        require(original.is_absolute() and original == original.resolve() and original.parent == runner.target_dir / 'release/deps' and not original.is_symlink(), 'Wrong producer path')
        require(original.is_file() and 0 < original.stat().st_size <= 128 * 1024**2, 'Invalid producer size')
        data = original.read_bytes()
        copied = runner.evidence / (name + '-test')
        with copied.open('xb') as stream:
            stream.write(data)
        shutil.copymode(original, copied)
        require(os.access(copied, os.X_OK) and copied.read_bytes() == data, 'Producer copy failed')
        runner.report['producers'][name] = {'producer': value, 'file': copied.name, 'sha256': sha(data), 'bytes': len(data)}
        bins[name] = str(copied)
    runner.record()
    return bins


def main():
    require(__debug__ and not sys.flags.optimize and sys.platform == 'linux' and platform.machine() == 'x86_64', 'Nonoptimized Linux x86_64 gate required')
    require(os.environ.get('PYTHONDONTWRITEBYTECODE') == '1', 'No-bytecode environment required')
    root = Path(__file__).resolve().parents[1]
    destination = Path(os.environ['TERRAIN_RAMP_EVIDENCE'])
    require(destination.is_absolute() and not destination.resolve().is_relative_to(root), 'Evidence must be outside source')
    source.source_root, source.frozen = root, None
    runner.initialize(root, destination, 'gpu')
    os.environ.update(CARGO_BUILD_JOBS='2', CARGO_PROFILE_RELEASE_DEBUG='0')
    error = None
    try:
        require(run('clean-before', ['git', 'status', '--porcelain=v1', '--untracked-files=all']).strip() == '', 'Committed clean source required')
        for key, rev in [('head', 'HEAD'), ('tree', 'HEAD^{tree}')]:
            runner.report[key] = run(key, ['git', 'rev-parse', rev]).strip()
        require(os.environ.get('GITHUB_SHA', runner.report['head']) == runner.report['head'], 'Checkout mismatch')
        tracked = run('tracked', ['git', 'ls-files', '-z']).split('\0')
        require(tracked.pop() == '', 'Invalid tracked census')
        source.check_source_inventory(tracked, 'before')
        source.frozen = source.snapshot()
        required = ['crates/orr_editor/src/terrain_ramp.rs', 'crates/orr_editor/src/app/terrain_ramp.rs', 'crates/orr_editor/src/app/terrain_ramp_tests.rs', 'crates/orr_editor/tests/terrain_ramp.rs', 'tools/terrain_ramp_gate.py', 'tools/terrain_ramp_contract.py', 'tools/test_terrain_ramp_gate.py', 'tools/test_terrain_ramp_contract.py']
        require(set(required) <= set(source.frozen), 'Missing feature/gate sources')
        runner.report['source_sha256'] = source.frozen
        runner.record()
        run('gate-tests', [sys.executable, '-B', '-m', 'unittest', 'discover', '-s', 'tools', '-p', 'test_terrain_ramp*.py', '-v'])
        common = ['--locked', '--release', '-p', 'orr_editor', '--no-default-features', '--features', 'navigation', '--lib', '--test', 'terrain_ramp']
        run('strict-first', ['cargo', 'clippy', *common, '--', '-D', 'warnings'])
        bins = pin(run('grouped-build', ['cargo', 'test', *common, '--no-run', '--message-format=json']), root)
        lists = []
        for name in ('terrain_ramp', 'orr_editor'):
            for label, flag in [('all', []), ('ignored', ['--ignored'])]:
                lists.append(run(name + '-' + label, [bins[name], *flag, '--list']))
        total = contract.inventories(*lists)
        for label, name, selected, flags, count in [
            ('cpu-candidate-session', 'terrain_ramp', contract.CPU, [], 3),
            ('cpu-app-context', 'orr_editor', [contract.APP_CPU], ['--exact', contract.APP_CPU], total),
            ('gpu-actual-app', 'orr_editor', [contract.APP_GPU], ['--ignored', '--exact', contract.APP_GPU], total)]:
            if label == 'gpu-actual-app':
                runner.software()
                os.environ.update(TERRAIN_RAMP_SOURCE_SHA=runner.report['head'], TERRAIN_RAMP_CAPTURE_DIR=str(destination / 'captures'))
            runner.fence()
            output = run(label, [bins[name], *flags, '--test-threads=1', '--show-output', '--color=never'])
            contract.result(output, selected, count)
            runner.report.setdefault('coverage', {})[label] = len(selected)
            runner.record()
        runner.report['validated_captures'] = receipts(destination / 'captures', runner.report['head'])
    except BaseException as failure:
        error = failure
    finally:
        try:
            require(source.frozen is not None, 'Missing prebuild source snapshot')
            source.source_fence(); runner.fence()
            require(run('clean-after', ['git', 'status', '--porcelain=v1', '--untracked-files=all']).strip() == '', 'Source dirtied')
            for key, rev in [('head', 'HEAD'), ('tree', 'HEAD^{tree}')]:
                require(run(key + '-after', ['git', 'rev-parse', rev]).strip() == runner.report[key], 'Revision changed')
            runner.report['source_unchanged'] = True
        except BaseException as failure:
            runner.report['final_fence_failure'] = repr(failure)
            error = error or failure
        runner.report['capture_sha256'] = {str(p.relative_to(destination)): runner.digest(p) for p in (destination / 'captures').glob('*') if p.is_file() and not p.is_symlink()}
        runner.report['status'] = 'failed' if error else 'passed'
        if error:
            runner.report['failure'] = ''.join(traceback.format_exception(error))
        runner.record()
    if error:
        raise SystemExit(error.code if isinstance(error, runner.GateFailure) else 1)


if __name__ == '__main__':
    main()
