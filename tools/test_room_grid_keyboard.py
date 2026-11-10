"""Fixed keyboard starter assets and explicit retained-production acceptance."""
import copy
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

import test_room_grid_puzzles as base

ASSETS = Path(__file__).resolve().parents[1] / 'examples/room-grid-puzzles'
ROUTES = ('route-keys.json', 'shortcut-keys.json')
BINDINGS = (('move_left', 'arrow_left'), ('move_right', 'arrow_right'),
            ('move_forward', 'arrow_up'), ('move_backward', 'arrow_down'),
            ('interact', 'space'))
CHECKS = ['fixed-keyboard-assets', 'creator-admission', 'semantic-baseline',
          'safe-controls-attachment', 'exact-backup-and-sidecar',
          'authored-key-victory', 'resolved-input-and-checksum-parity',
          'byte-identical-repeat', 'shortcut-blocked', 'schema1-refused',
          'repeat-attachment-refused', 'project-unchanged']


def expected_controls():
    return {'version': 1, 'actions': [
        {'name': name, 'bindings': [{'device': 'Keyboard', 'key': key}]}
        for name, key in BINDINGS]}


def translated_route(logical):
    steps = []
    for step in logical['steps']:
        held = []
        if step['move_x']:
            held.append('arrow_right' if step['move_x'] > 0 else 'arrow_left')
        if step['move_z']:
            held.append('arrow_down' if step['move_z'] > 0 else 'arrow_up')
        if step['interact']:
            held.append('space')
        steps.append({'ticks': step['ticks'], 'held': held,
                      'expect_key': step['expect_key'], 'expect_won': step['expect_won']})
    return {'schema': 2, 'steps': steps}


def read_asset(path):
    if path.is_symlink() or not path.is_file() or not 1 <= path.stat().st_size <= 65536:
        raise ValueError('keyboard asset must be a bounded regular nonsymlink file')
    return path.read_bytes()


def exact_json(raw, expected):
    value = base.strict_json(raw)
    # Canonical JSON comparison preserves bool/int and string/number distinctions.
    if json.dumps(value, sort_keys=True) != json.dumps(expected, sort_keys=True):
        raise ValueError('keyboard asset differs from fixed preset or semantic route')
    return value


def load_assets():
    logical = base.load_assets()
    controls_path = ASSETS / 'controls/arrows.json'
    if controls_path.parent.is_symlink() or not controls_path.parent.is_dir():
        raise ValueError('controls directory must be normal')
    controls = read_asset(controls_path)
    exact_json(controls, expected_controls())
    root = ASSETS / 'keyboard'
    if root.is_symlink() or not root.is_dir() or {p.name for p in root.iterdir()} != set(base.CATALOG):
        raise ValueError('keyboard directory inventory differs')
    paths = {'controls/arrows.json': controls_path}
    for name in base.CATALOG:
        directory = root / name
        if directory.is_symlink() or not directory.is_dir() or {p.name for p in directory.iterdir()} != set(ROUTES):
            raise ValueError('keyboard route inventory differs')
        for filename, role in zip(ROUTES, ('winning', 'shortcut')):
            path = directory / filename
            exact_json(read_asset(path), translated_route(logical[name][role]))
            paths[f'keyboard/{name}/{filename}'] = path
    return logical, controls, paths


def assert_key_report(report, logical, controls_hash, ticks, won):
    expected_status = 'passed' if won else 'error'
    if (report.get('schema') != 2 or report.get('status') != expected_status
            or report.get('input_mode') != 'supplied_physical_key_identities'
            or report.get('controls_sha256') != controls_hash
            or len(report.get('steps', [])) != len(logical['steps'])
            or report['steps'][-1]['tick'] != ticks
            or report['steps'][-1]['observed'] != {'key': True, 'won': won}):
        raise AssertionError('keyboard report identity or gameplay outcome differs')
    if won:
        if report.get('ticks') != ticks:
            raise AssertionError('winning tick total differs')
    elif report.get('error', {}).get('kind') != 'assertion':
        raise AssertionError('shortcut must fail its victory assertion')
    for observed, source in zip(report['steps'], logical['steps']):
        expected = {key: source[key] for key in ('move_x', 'move_z', 'interact')}
        if observed.get('input') != expected:
            raise AssertionError('authored keys did not resolve to semantic inputs')


def run_integration(binary_dir, output_dir):
    """Create new fixtures only; use explicit trusted binaries without building."""
    assets, controls, paths = load_assets()
    binaries = Path(binary_dir).resolve(strict=True)
    hashes = {name: base.digest(binaries / name) for name in
              ('orr_new_arena', 'orr_attach_room_controls', 'orr_playtest_room')}
    asset_hashes = {name: base.digest(path) for name, path in paths.items()}
    original_hashes = {f'{name}/{filename}': base.digest(item['paths'][filename])
                       for name, item in assets.items() for filename in base.FILES}
    tool_hash, harness_hash = base.digest(base.TOOL), base.digest(Path(__file__))
    starter_hash = base.digest(Path(base.__file__))
    controls_hash = hashlib.sha256(controls).hexdigest()
    root = Path(output_dir).absolute()
    root.mkdir()
    root = root.resolve(strict=True)
    results = {}
    for name, item in assets.items():
        out = root / name
        out.mkdir()
        openings, win_ticks, shortcut_ticks = base.CATALOG[name]
        options = ['--border', '--merge-walls']
        for opening in openings:
            options += ['--opening', opening]
        recipe = base.run_bounded([sys.executable, '-B', '-E', base.TOOL, '--input',
                                   item['paths']['grid.txt'], *options], out, 'compile')
        if recipe != item['recipe']:
            raise AssertionError('compiler recipe differs from admitted assets')
        recipe_path = out / 'layout.json'
        recipe_path.write_bytes(recipe)
        project = out / (name + ' Keyboard Ω')
        base.run_bounded([binaries / 'orr_new_arena', '--output', project,
                          '--template', 'room-escape-3d-v1', '--seed', 'starter-' + name,
                          '--room-layout', recipe_path], out, 'create')
        def play(label, script, expected=0):
            return base.run_bounded([binaries / 'orr_playtest_room', '--project', project,
                                     '--script', script, '--json'], out, label, expected)
        before = base.inventory(project)
        semantic = base.strict_json(play('semantic-baseline', item['paths']['route.json']))
        if semantic.get('status') != 'passed' or semantic.get('ticks') != win_ticks or base.inventory(project) != before:
            raise AssertionError('semantic baseline failed or changed the project')
        manifest_path = project / 'orr.project.json'
        original = manifest_path.read_bytes()
        with (project / 'arrows.json').open('xb') as stream:
            stream.write(controls)
        pre_attach = base.inventory(project)
        attach_command = [binaries / 'orr_attach_room_controls', '--project', project, '--controls', 'arrows.json']
        base.run_bounded(attach_command, out, 'attach')
        expected_manifest = base.strict_json(original)
        expected_manifest['entry']['controls'] = 'arrows.json'
        if base.strict_json(manifest_path.read_bytes()) != expected_manifest:
            raise AssertionError('attachment changed unrelated manifest values')
        backup = project / 'orr.project.before-controls.json'
        if backup.read_bytes() != original or (project / 'arrows.json').read_bytes() != controls:
            raise AssertionError('backup or sidecar identity changed')
        after_attach = base.inventory(project)
        if set(after_attach) != set(pre_attach) | {'orr.project.before-controls.json'}:
            raise AssertionError('attachment file inventory differs')
        if any(after_attach[p] != h for p, h in pre_attach.items() if p != 'orr.project.json'):
            raise AssertionError('attachment changed unrelated files')
        first = play('key-win', paths[f'keyboard/{name}/route-keys.json'])
        if first != play('key-repeat', paths[f'keyboard/{name}/route-keys.json']):
            raise AssertionError('key route did not repeat byte-identically')
        winning = base.strict_json(first)
        assert_key_report(winning, item['winning'], controls_hash, win_ticks, True)
        if (winning['initial_checksum'] != semantic['initial_checksum']
                or winning['scene_sha256'] != semantic['scene_sha256']
                or [s['checksum'] for s in winning['steps']] != [s['checksum'] for s in semantic['steps']]):
            raise AssertionError('semantic and authored-key frame checksums differ')
        blocked = base.strict_json(play('key-shortcut', paths[f'keyboard/{name}/shortcut-keys.json'], 1))
        assert_key_report(blocked, item['shortcut'], controls_hash, shortcut_ticks, False)
        refused = base.strict_json(play('schema1-refused', item['paths']['route.json'], 1))
        if refused.get('error', {}).get('kind') != 'unsupported_project' or refused.get('steps') != []:
            raise AssertionError('schema1 must explicitly refuse authored controls')
        base.run_bounded(attach_command, out, 'repeat-attach', 1)
        if b'controls already present' not in (out / 'repeat-attach.stderr').read_bytes():
            raise AssertionError('repeated attachment failed for an unexpected reason')
        after = base.inventory(project)
        if after != after_attach:
            raise AssertionError('playtest or refused attachment changed the project')
        (out / 'inventory.json').write_text(json.dumps({'before': before, 'pre_attach': pre_attach,
            'after_attach': after_attach, 'after': after}, indent=2) + '\n')
        results[name] = {'status': 'passed', 'checks': CHECKS, 'winning_ticks': win_ticks,
                         'shortcut_ticks': shortcut_ticks, 'repeat_identical': True,
                         'semantic_checksums_equal': True}
    if asset_hashes != {name: base.digest(path) for name, path in paths.items()}:
        raise AssertionError('keyboard source assets changed')
    if original_hashes != {f'{name}/{filename}': base.digest(item['paths'][filename])
                          for name, item in assets.items() for filename in base.FILES}:
        raise AssertionError('original starter assets changed')
    if hashes != {name: base.digest(binaries / name) for name in hashes}:
        raise AssertionError('producer binaries changed')
    if (tool_hash != base.digest(base.TOOL) or harness_hash != base.digest(Path(__file__))
            or starter_hash != base.digest(Path(base.__file__))):
        raise AssertionError('compiler or harness changed')
    result = {'status': 'passed', 'checks': list(base.CATALOG), 'checks_per_puzzle': CHECKS,
              'puzzles': results, 'binaries': hashes, 'tool_sha256': tool_hash,
              'harness_sha256': harness_hash, 'starter_harness_sha256': starter_hash,
              'asset_sha256': asset_hashes, 'original_asset_sha256': original_hashes,
              'controls_sha256': controls_hash,
              'scope': 'CPU translation of supplied key identities, not hardware/window/GPU/export'}
    (root / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))


class KeyboardTests(unittest.TestCase):
    def test_all_seven_keyboard_assets_match_logical_routes(self):
        logical, controls, paths = load_assets()
        self.assertEqual(len(paths), 7)
        self.assertEqual(list(logical), list(base.CATALOG))
        self.assertEqual(json.loads(controls), expected_controls())

    def test_arrow_mapping_and_space_are_exact(self):
        self.assertEqual([key for _, key in BINDINGS],
                         ['arrow_left', 'arrow_right', 'arrow_up', 'arrow_down', 'space'])
        self.assertEqual(len(set(name for name, _ in BINDINGS)), 5)

    def test_translation_preserves_every_semantic_step(self):
        for item in base.load_assets().values():
            for role in ('winning', 'shortcut'):
                for key, logical in zip(translated_route(item[role])['steps'], item[role]['steps']):
                    held = key['held']
                    self.assertEqual(int('arrow_right' in held) - int('arrow_left' in held), logical['move_x'])
                    self.assertEqual(int('arrow_down' in held) - int('arrow_up' in held), logical['move_z'])
                    self.assertEqual('space' in held, logical['interact'])
                    self.assertEqual(key['ticks'], logical['ticks'])

    def test_interaction_release_and_tick_totals_preserved(self):
        for name, item in base.load_assets().items():
            for role, ticks in [('winning', base.CATALOG[name][1]), ('shortcut', base.CATALOG[name][2])]:
                route = translated_route(item[role])
                self.assertEqual(sum(s['ticks'] for s in route['steps']), ticks)
                self.assertEqual(route['steps'][0]['held'], ['space'])
                self.assertEqual(route['steps'][-1]['held'], ['space'])
                self.assertTrue(all('space' not in s['held'] and s['ticks'] > 0 for s in route['steps'][1:-1]))

    def test_exact_json_rejects_wrong_types_and_extra_fields(self):
        expected = expected_controls()
        for mutation in [dict(expected, version=True), dict(expected, version=1.0),
                         dict(expected, extra=1), {'version': 1, 'actions': []}]:
            with self.assertRaises(ValueError):
                exact_json(json.dumps(mutation).encode(), expected)

    def test_duplicate_keys_and_nonfinite_values_rejected(self):
        for raw in (b'{"version":1,"version":1}', b'{"version":NaN}', b' ' * 65537):
            with self.assertRaises(ValueError):
                exact_json(raw, expected_controls())

    def test_wrong_key_order_or_identity_is_not_silently_repaired(self):
        expected = translated_route(next(iter(base.load_assets().values()))['winning'])
        for key in ('r', 'escape', 'unknown', 'arrow_left'):
            bad = copy.deepcopy(expected)
            bad['steps'][0]['held'] = [key]
            with self.assertRaises(ValueError):
                exact_json(json.dumps(bad).encode(), expected)

    def test_asset_reader_refuses_symlinks_directories_and_oversize(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / 'target'
            target.write_bytes(b'{}')
            link = root / 'link'
            link.symlink_to(target)
            large = root / 'large'
            large.write_bytes(b'x' * 65537)
            for path in (link, root, large, root / 'missing'):
                with self.assertRaises(ValueError):
                    read_asset(path)


if __name__ == '__main__':
    unittest.main()
