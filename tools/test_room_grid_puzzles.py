"""Validate fixed starter assets; explicit integration retains real production receipts."""
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import sys
import tempfile
import time
import unittest

import create_room_grid as grid

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / 'examples/room-grid-puzzles'
TOOL = Path(grid.__file__).resolve()
# Metadata is documentation, never a command source. Commands use this fixed catalog.
CATALOG = {
    'courtyard': ((), 202, 122),
    'switchback': ((), 362, 202),
    'passage': (('south:0', 'south:6'), 242, 122),
}
FILES = ('grid.txt', 'route.json', 'shortcut.json', 'puzzle.json')
CHECKS = ['fixed-asset-admission', 'creator-admission', 'winning-route',
          'byte-identical-repeat', 'shortcut-blocked', 'project-unchanged']
MAX_OUTPUT = 256 * 1024


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inventory(path):
    return {str(p.relative_to(path)): digest(p) for p in sorted(path.rglob('*')) if p.is_file()}


def strict_json(raw):
    if not isinstance(raw, bytes) or not 1 <= len(raw) <= 65536:
        raise ValueError('JSON must contain 1..65536 bytes')
    def pairs(items):
        out = {}
        for key, value in items:
            if key in out:
                raise ValueError('duplicate JSON field')
            out[key] = value
        return out
    def constant(value):
        raise ValueError('nonfinite JSON constant')
    return json.loads(raw, object_pairs_hook=pairs, parse_constant=constant)


def validate_route(raw, ticks):
    value = strict_json(raw)
    if (not isinstance(value, dict) or set(value) != {'schema', 'steps'}
            or type(value['schema']) is not int or value['schema'] != 1
            or not isinstance(value['steps'], list) or not 1 <= len(value['steps']) <= 256):
        raise ValueError('invalid route envelope')
    total = 0
    fields = {'ticks', 'move_x', 'move_z', 'interact', 'expect_key', 'expect_won'}
    for item in value['steps']:
        if not isinstance(item, dict) or set(item) != fields:
            raise ValueError('invalid route step fields')
        if type(item['ticks']) is not int or not 0 <= item['ticks'] <= 6000:
            raise ValueError('invalid tick count')
        for field in ('move_x', 'move_z'):
            if type(item[field]) is not int or item[field] not in (-1, 0, 1):
                raise ValueError('invalid semantic axis')
        for field in ('interact', 'expect_key', 'expect_won'):
            if type(item[field]) is not bool:
                raise ValueError('invalid boolean')
        total += item['ticks']
    if total != ticks or total > 6000:
        raise ValueError('route tick total differs from fixed catalog')
    if value['steps'][0] != {'ticks': 1, 'move_x': 0, 'move_z': 0, 'interact': True,
                              'expect_key': True, 'expect_won': False}:
        raise ValueError('route must first collect the adjacent key')
    if value['steps'][-1] != {'ticks': 1, 'move_x': 0, 'move_z': 0, 'interact': True,
                               'expect_key': True, 'expect_won': True}:
        raise ValueError('route must explicitly assert final victory')
    if any(not item['expect_key'] or item['expect_won'] or item['interact']
           for item in value['steps'][1:-1]):
        raise ValueError('movement must release interaction and retain key before victory')
    return value


def load_assets():
    result = {}
    for name, (openings, win_ticks, shortcut_ticks) in CATALOG.items():
        directory = ASSETS / name
        if not directory.is_dir() or directory.is_symlink():
            raise ValueError('starter directory must be a normal directory')
        if {p.name for p in directory.iterdir()} != set(FILES):
            raise ValueError('starter asset inventory differs')
        paths = {filename: directory / filename for filename in FILES}
        if any(p.is_symlink() or not p.is_file() or p.stat().st_size > 65536 for p in paths.values()):
            raise ValueError('starter assets must be bounded regular nonsymlink files')
        data = paths['grid.txt'].read_bytes()
        recipe = grid.encode(data, border=True, merge_walls=True, openings=openings)
        metadata = strict_json(paths['puzzle.json'].read_bytes())
        expected = {'schema': 1, 'id': name, 'border': True, 'merge_walls': True,
                    'openings': list(openings), 'winning_ticks': win_ticks, 'shortcut_ticks': shortcut_ticks}
        # Python bool/int equality is not a substitute for exact metadata types.
        if (metadata != expected or any(type(metadata[k]) is not type(v) for k, v in expected.items())
                or any(type(v) is not str for v in metadata['openings'])):
            raise ValueError('metadata differs from fixed catalog')
        winning = validate_route(paths['route.json'].read_bytes(), win_ticks)
        shortcut = validate_route(paths['shortcut.json'].read_bytes(), shortcut_ticks)
        if len(shortcut['steps']) != 3 or shortcut['steps'][1]['move_x'] != 1 or shortcut['steps'][1]['move_z'] != 0:
            raise ValueError('shortcut must be the direct positive-X attempt')
        result[name] = {'paths': paths, 'recipe': recipe, 'winning': winning, 'shortcut': shortcut}
    return result


def run_bounded(command, directory, label, expected=0, timeout=90):
    """Retain each command; drain both pipes with strict per-stream output limits."""
    command = list(map(str, command))
    (directory / (label + '.command.json')).write_text(json.dumps(command) + '\n')
    outputs = {'stdout': bytearray(), 'stderr': bytearray()}
    problem = None
    start = time.monotonic()
    with subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE) as child:
        with selectors.DefaultSelector() as selector:
            selector.register(child.stdout, selectors.EVENT_READ, 'stdout')
            selector.register(child.stderr, selectors.EVENT_READ, 'stderr')
            while selector.get_map() or child.poll() is None:
                if time.monotonic() - start > timeout:
                    problem = 'subprocess timeout'
                    child.kill()
                    break
                for key, _ in selector.select(0.05):
                    channel = key.data
                    chunk = os.read(key.fileobj.fileno(), min(4096, MAX_OUTPUT - len(outputs[channel]) + 1))
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    remaining = MAX_OUTPUT - len(outputs[channel])
                    outputs[channel].extend(chunk[:remaining])
                    if len(chunk) > remaining:
                        problem = 'subprocess output limit exceeded'
                        child.kill()
                        break
                if problem:
                    break
        child.wait(timeout=5)
        code = child.returncode
    for channel, data in outputs.items():
        (directory / (label + '.' + channel)).write_bytes(data)
    (directory / (label + '.exit')).write_text(str(code) + '\n')
    if problem:
        (directory / (label + '.limit')).write_text(problem + '\n')
        raise AssertionError(problem)
    if code != expected:
        raise AssertionError(f'{label}: expected exit {expected}, got {code}; see retained output')
    return bytes(outputs['stdout'])


def run_integration(binary_dir, output_dir):
    """Explicit existing producer directory, new receipt directory; no builds/downloads."""
    assets = load_assets()  # Validate every fixed asset before writing evidence.
    binaries = Path(binary_dir).resolve(strict=True)
    hashes = {name: digest(binaries / name) for name in ('orr_new_arena', 'orr_playtest_room')}
    asset_hashes = {f'{name}/{filename}': digest(item['paths'][filename])
                    for name, item in assets.items() for filename in FILES}
    tool_hash = digest(TOOL)
    harness_hash = digest(Path(__file__))
    root = Path(output_dir).absolute()
    root.mkdir()  # Refuse reuse and preserve all earlier receipts.
    root = root.resolve(strict=True)
    results = {}
    for name, item in assets.items():
        out = root / name
        out.mkdir()
        openings, win_ticks, shortcut_ticks = CATALOG[name]
        options = ['--border', '--merge-walls']
        for opening in openings:
            options += ['--opening', opening]
        recipe = run_bounded([sys.executable, '-B', '-E', TOOL, '--input', item['paths']['grid.txt'], *options], out, 'compile')
        if recipe != item['recipe']:
            raise AssertionError('CLI recipe differs from validated compiler output')
        recipe_path = out / 'layout.json'
        recipe_path.write_bytes(recipe)
        project = out / (name + ' Project Ω')
        run_bounded([binaries / 'orr_new_arena', '--output', project, '--template', 'room-escape-3d-v1',
                     '--seed', 'starter-' + name, '--room-layout', recipe_path], out, 'create')
        before = inventory(project)
        def play(label, script, expected=0):
            return run_bounded([binaries / 'orr_playtest_room', '--project', project, '--script', script, '--json'], out, label, expected)
        first = play('win', item['paths']['route.json'])
        if first != play('repeat', item['paths']['route.json']):
            raise AssertionError('winning route does not repeat byte-identically')
        winning = strict_json(first)
        if (winning.get('status') != 'passed' or winning.get('ticks') != win_ticks
                or winning['steps'][-1]['observed'] != {'key': True, 'won': True}):
            raise AssertionError('winning route did not reach the promised victory')
        blocked = strict_json(play('shortcut', item['paths']['shortcut.json'], 1))
        if (blocked.get('status') != 'error' or blocked.get('error', {}).get('kind') != 'assertion'
                or blocked['steps'][-1]['tick'] != shortcut_ticks
                or blocked['steps'][-1]['observed'] != {'key': True, 'won': False}):
            raise AssertionError('shortcut must fail its victory assertion while retaining key')
        after = inventory(project)
        (out / 'inventory.json').write_text(json.dumps({'before': before, 'after': after}, indent=2) + '\n')
        if before != after:
            raise AssertionError('playtest changed project contents')
        results[name] = {'status': 'passed', 'checks': CHECKS, 'winning_ticks': win_ticks,
                         'shortcut_ticks': shortcut_ticks, 'repeat_identical': True}
    if asset_hashes != {f'{name}/{filename}': digest(item['paths'][filename])
                        for name, item in assets.items() for filename in FILES}:
        raise AssertionError('source assets changed during acceptance')
    if tool_hash != digest(TOOL) or harness_hash != digest(Path(__file__)):
        raise AssertionError('compiler/harness changed during acceptance')
    if hashes != {name: digest(binaries / name) for name in hashes}:
        raise AssertionError('producer binaries changed during acceptance')
    result = {'status': 'passed', 'checks': list(CATALOG), 'checks_per_puzzle': CHECKS,
              'puzzles': results, 'binaries': hashes, 'tool_sha256': tool_hash,
              'harness_sha256': harness_hash, 'asset_sha256': asset_hashes,
              'scope': 'scripted CPU using explicit production binaries; no native/GPU/export claim'}
    (root / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))


class PuzzleTests(unittest.TestCase):
    def test_exact_three_starter_assets_are_admitted(self):
        self.assertEqual(list(load_assets()), list(CATALOG))

    def test_layouts_are_distinct_and_all_have_obstacles(self):
        assets = load_assets()
        self.assertEqual(len({value['recipe'] for value in assets.values()}), 3)
        self.assertTrue(all(json.loads(item['recipe'])['walls'] for item in assets.values()))

    def test_winning_routes_have_distinct_authored_lengths(self):
        assets = load_assets()
        self.assertEqual([sum(s['ticks'] for s in item['winning']['steps']) for item in assets.values()], [202, 362, 242])

    def test_shortcuts_assert_victory_and_are_not_fake_successes(self):
        for item in load_assets().values():
            self.assertTrue(item['shortcut']['steps'][-1]['expect_won'])
            self.assertNotEqual(item['winning'], item['shortcut'])

    def test_strict_json_rejects_duplicates_constants_and_size(self):
        for raw in (b'{"x":1,"x":2}', b'NaN', b'Infinity', b'-Infinity', b'', b' '*65537):
            with self.assertRaises(ValueError):
                strict_json(raw)

    def test_route_rejects_wrong_schema_types_and_fields(self):
        base = load_assets()['courtyard']['winning']
        for change in ({'schema': True}, {'schema': 2}, {'extra': 0}, {'steps': []}):
            with self.assertRaises(ValueError):
                validate_route(json.dumps({**base, **change}).encode(), 202)
        for field, value in [('ticks', True), ('ticks', -1), ('move_x', True), ('move_z', 2), ('interact', 1), ('expect_won', 1)]:
            candidate = json.loads(json.dumps(base)); candidate['steps'][1][field] = value
            with self.assertRaises(ValueError):
                validate_route(json.dumps(candidate).encode(), 202)

    def test_route_rejects_missing_final_victory_and_unreleased_input(self):
        base = load_assets()['courtyard']['winning']
        for index, field, value in [(-1, 'expect_won', False), (1, 'interact', True), (1, 'expect_key', False)]:
            candidate = json.loads(json.dumps(base)); candidate['steps'][index][field] = value
            with self.assertRaises(ValueError):
                validate_route(json.dumps(candidate).encode(), 202)
        with self.assertRaises(ValueError):
            validate_route(json.dumps(base).encode(), 203)

    def test_readme_and_fixed_catalog_do_not_execute_metadata(self):
        text = (ASSETS / 'README.md').read_text()
        self.assertIn('set -C', text)
        for name in CATALOG:
            self.assertIn(name, text)
        self.assertEqual(set(CATALOG), {'courtyard', 'switchback', 'passage'})

    def test_subprocess_exit_and_both_output_streams_are_retained(self):
        out = Path(tempfile.mkdtemp(prefix='room-puzzles-unit-'))
        raw = run_bounded([sys.executable, '-c', 'import sys;print("ok");print("note",file=sys.stderr)'], out, 'streams')
        self.assertEqual(raw, b'ok\n')
        self.assertEqual((out/'streams.stderr').read_bytes(), b'note\n')
        with self.assertRaises(AssertionError):
            run_bounded([sys.executable, '-c', 'raise SystemExit(3)'], out, 'exit')
        self.assertEqual((out/'exit.exit').read_text(), '3\n')

    def test_subprocess_output_and_time_are_bounded(self):
        out = Path(tempfile.mkdtemp(prefix='room-puzzles-limits-'))
        with self.assertRaisesRegex(AssertionError, 'output limit'):
            run_bounded([sys.executable, '-c', 'import sys;sys.stdout.write("x"*300000)'], out, 'limit')
        self.assertEqual((out/'limit.stdout').stat().st_size, MAX_OUTPUT)
        with self.assertRaisesRegex(AssertionError, 'timeout'):
            run_bounded([sys.executable, '-c', 'import time;time.sleep(2)'], out, 'timeout', timeout=0.1)
        with self.assertRaisesRegex(AssertionError, 'timeout'):
            run_bounded([sys.executable, '-c', 'import os,time;os.close(1);os.close(2);time.sleep(2)'], out, 'closed-pipes', timeout=0.1)


if __name__ == '__main__':
    unittest.main()
