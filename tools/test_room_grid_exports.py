"""Explicit trusted-producer export acceptance for the fixed Room starters."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import shutil
import stat
import sys
import tempfile
import unittest

import test_room_grid_keyboard as keys

base = keys.base
CHECKS = ['fixed-assets', 'creator-admission', 'controls-attachment', 'source-victory',
          'export-manifest-and-sidecars', 'backup-exclusion', 'source-hidden-relocation',
          'exported-project-victory', 'exported-repeat', 'shortcut-blocked',
          'launcher-cpu-parity', 'existing-output-refused', 'inputs-unchanged']
FLOOR = 1024 ** 3
DOMAIN = b'orrery.room-escape.export.content.v1\0'
LAUNCHER = b'#!/bin/sh\nset -eu\nroot=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)\nexec "$root/bin/room_escape" --project "$root/project" "$@"\n'


def require_budget(free, runtime_bytes):
    if type(runtime_bytes) is not int or not 1 <= runtime_bytes <= 128 * 1024 ** 2:
        raise ValueError('runtime size outside bounded export budget')
    # Three retained copies, one temporary exporter stage, and small projects/logs.
    required = 4 * runtime_bytes + 16 * 1024 ** 2
    if free - required < FLOOR:
        raise ValueError('export budget would cross the 1 GiB free-space floor')
    return required


def checked_inventory(root):
    if root.is_symlink() or not root.is_dir():
        raise ValueError('inventory root must be a normal directory')
    result = {}
    for p in sorted(root.rglob('*')):
        if p.is_symlink() or not (p.is_file() or p.is_dir()):
            raise ValueError('inventory contains a link or special file')
        if p.is_file():
            result[p.relative_to(root).as_posix()] = base.digest(p)
    return result


def validate_bundle(bundle, source, runtime_hash, runtime_size):
    inventory = checked_inventory(bundle)
    raw = keys.read_asset(bundle / 'orr.export.json')
    manifest = base.strict_json(raw)
    if (set(manifest) != {'schema', 'payload', 'content_digest'}
            or type(manifest['schema']) is not int or manifest['schema'] != 1):
        raise ValueError('export manifest envelope differs')
    payload = manifest['payload']
    if (payload['profile'] != 'room-escape-authored-linux-x86_64-v1'
            or payload['runtime'] != {'sha256': runtime_hash, 'bytes': runtime_size}
            or payload['entry'] != base.strict_json((source / 'orr.project.json').read_bytes())['entry']):
        raise ValueError('export profile, runtime or entry differs')
    canonical = json.dumps(payload, ensure_ascii=False, separators=(',', ':')).encode()
    if hashlib.sha256(DOMAIN + canonical).hexdigest() != manifest['content_digest']:
        raise ValueError('export content digest differs')
    expected = {'orr.export.json'}
    for item in payload['files']:
        path = item['path']
        if (not isinstance(path, str) or not path or '\\' in path
                or PurePosixPath(path).is_absolute() or '..' in PurePosixPath(path).parts
                or PurePosixPath(path).as_posix() != path or path in expected):
            raise ValueError('unsafe or duplicate manifest path')
        expected.add(path)
        p = bundle / path
        if (inventory.get(path) != item['sha256'] or p.stat().st_size != item['bytes']
                or stat.S_IMODE(p.stat().st_mode) != item['mode']):
            raise ValueError('export file bytes or mode differ')
        if path.startswith('project/'):
            original = source / path[len('project/'):]
            if not original.is_file() or base.digest(original) != item['sha256']:
                raise ValueError('exported project content differs from source')
    if set(inventory) != expected:
        raise ValueError('bundle inventory differs from manifest')
    if not {'project/arrows.json', 'project/orr.project.json', 'project/room.scene.yaml',
            'project/room.models.json', 'project/room.camera.json',
            'bin/room_escape', 'run-room-escape'} <= expected:
        raise ValueError('required starter files missing')
    if 'project/orr.project.before-controls.json' in expected:
        raise ValueError('authoring backup leaked into bundle')
    validate_executables(bundle, runtime_hash, runtime_size)
    return inventory


def validate_executables(bundle, runtime_hash, runtime_size):
    runtime = bundle / 'bin/room_escape'
    launcher = bundle / 'run-room-escape'
    if (runtime.is_symlink() or not runtime.is_file()
            or base.digest(runtime) != runtime_hash or runtime.stat().st_size != runtime_size
            or stat.S_IMODE(runtime.stat().st_mode) != 0o755):
        raise ValueError('exported runtime does not match the selected producer')
    if (launcher.is_symlink() or not launcher.is_file()
            or launcher.read_bytes() != LAUNCHER
            or stat.S_IMODE(launcher.stat().st_mode) != 0o755):
        raise ValueError('launcher does not use the fixed relative-path contract')


def run_integration(binary_dir, output_dir, runtime=None):
    assets, controls, paths = keys.load_assets()
    binaries = Path(binary_dir).resolve(strict=True)
    runtime = Path(runtime).resolve(strict=True) if runtime else binaries / 'room_escape'
    producers = {name: binaries / name for name in
                 ('orr_new_arena', 'orr_attach_room_controls', 'orr_playtest_room', 'orr_export_room')}
    producers['room_escape'] = runtime
    hashes = {name: base.digest(path) for name, path in producers.items()}
    runtime_size = runtime.stat().st_size
    root = Path(output_dir).absolute()
    # Parent must exist; never reuse/overwrite any pre-existing output root.
    budget = require_budget(shutil.disk_usage(root.parent).free, runtime_size)
    root.mkdir()
    root = root.resolve(strict=True)
    controls_hash = hashlib.sha256(controls).hexdigest()
    asset_paths = dict(paths)
    asset_paths.update({f'{name}/{filename}': item['paths'][filename]
                       for name, item in assets.items() for filename in base.FILES})
    asset_hashes = {name: base.digest(path) for name, path in asset_paths.items()}
    source_paths = {'harness': Path(__file__), 'keyboard_harness': Path(keys.__file__),
                    'starter_harness': Path(base.__file__), 'compiler': base.TOOL}
    source_hashes = {name: base.digest(path) for name, path in source_paths.items()}
    results = {}
    for name, item in assets.items():
        if shutil.disk_usage(root).free < FLOOR + runtime_size + 4 * 1024 ** 2:
            raise ValueError('insufficient remaining export capacity')
        out = root / name
        out.mkdir()
        project = out / (name + ' Source Ω')
        layout = out / 'layout.json'
        layout.write_bytes(item['recipe'])
        base.run_bounded([producers['orr_new_arena'], '--output', project, '--template',
                          'room-escape-3d-v1', '--seed', 'export-' + name, '--room-layout', layout], out, 'create')
        original = (project / 'orr.project.json').read_bytes()
        (project / 'arrows.json').write_bytes(controls)
        base.run_bounded([producers['orr_attach_room_controls'], '--project', project,
                          '--controls', 'arrows.json'], out, 'attach')
        if (project / 'orr.project.before-controls.json').read_bytes() != original:
            raise AssertionError('attachment backup changed')
        before = checked_inventory(project)
        win_ticks, shortcut_ticks = base.CATALOG[name][1:]
        def play(project_path, label, shortcut=False):
            script = paths[f'keyboard/{name}/' + ('shortcut-keys.json' if shortcut else 'route-keys.json')]
            raw = base.run_bounded([producers['orr_playtest_room'], '--project', project_path,
                                   '--script', script, '--json'], out, label, int(shortcut))
            keys.assert_key_report(base.strict_json(raw), item['shortcut' if shortcut else 'winning'],
                                   controls_hash, shortcut_ticks if shortcut else win_ticks, not shortcut)
            return raw
        source_report = base.strict_json(play(project, 'source-win'))
        cpu_args = ['--headless', '--ticks', '42', '--hold', 'left,right,up,down,interact', '--check-model-poses']
        cpu = base.run_bounded([runtime, '--project', project, *cpu_args], out, 'source-cpu')
        bundle = out / 'exported'
        command = [producers['orr_export_room'], '--project', project, '--runtime', runtime,
                   '--runtime-sha256', hashes['room_escape'], '--output', bundle, '--trusted-runtime']
        base.run_bounded(command, out, 'export')
        exported = validate_bundle(bundle, project, hashes['room_escape'], runtime_size)
        base.run_bounded(command, out, 'repeat-export-refused', 1)
        if b'exist' not in (out / 'repeat-export-refused.stderr').read_bytes():
            raise AssertionError('repeat export failed for unexpected reason')
        if checked_inventory(project) != before or checked_inventory(bundle) != exported:
            raise AssertionError('export/refusal mutated project or published bundle')
        hidden = out / 'source-hidden'
        project.rename(hidden)
        relocated = out / 'Relocated Bundle Ω'
        bundle.rename(relocated)
        if project.exists() or bundle.exists():
            raise AssertionError('original source/bundle locations still exist')
        first = play(relocated / 'project', 'relocated-win')
        if first != play(relocated / 'project', 'relocated-repeat'):
            raise AssertionError('relocated playtest differs on repeat')
        after_report = base.strict_json(first)
        for field in ('initial_checksum', 'scene_sha256', 'controls_sha256', 'steps'):
            if source_report[field] != after_report[field]:
                raise AssertionError('relocated route identity/checksums differ')
        play(relocated / 'project', 'relocated-shortcut', True)
        for label in ('relocated-cpu', 'relocated-cpu-repeat'):
            actual = base.run_bounded([relocated / 'run-room-escape', *cpu_args], out, label)
            if actual != cpu:
                raise AssertionError('relocated launcher CPU output differs')
        if (checked_inventory(hidden) != before
                or validate_bundle(relocated, hidden, hashes['room_escape'], runtime_size) != exported):
            raise AssertionError('relocation/playtest/launcher changed content')
        results[name] = {'status': 'passed', 'checks': CHECKS, 'winning_ticks': win_ticks,
                         'shortcut_ticks': shortcut_ticks, 'source_hidden': True,
                         'launcher_cpu_ticks': 42, 'bundle_bytes': sum(p.stat().st_size for p in relocated.rglob('*') if p.is_file()),
                         'export_manifest_sha256': base.digest(relocated / 'orr.export.json')}
        (out / 'inventory.json').write_text(json.dumps({'source': before, 'bundle': exported}, indent=2) + '\n')
    if hashes != {name: base.digest(path) for name, path in producers.items()}:
        raise AssertionError('producer identity changed')
    if asset_hashes != {name: base.digest(path) for name, path in asset_paths.items()}:
        raise AssertionError('source assets changed')
    if source_hashes != {name: base.digest(path) for name, path in source_paths.items()}:
        raise AssertionError('source helper changed')
    free = shutil.disk_usage(root).free
    if free < FLOOR:
        raise AssertionError('free-space floor crossed')
    result = {'status': 'passed', 'checks': list(base.CATALOG), 'checks_per_puzzle': CHECKS,
              'puzzles': results, 'binaries': hashes, 'source_sha256': source_hashes,
              'asset_sha256': asset_hashes, 'runtime_bytes': runtime_size,
              'reserved_budget_bytes': budget, 'free_bytes_after': free,
              'scope': 'Exported-project supplied-key CPU routes plus relocated launcher constant semantic input/model-pose parity; no native window, hardware keys or GPU coverage.'}
    (root / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))


class ExportTests(unittest.TestCase):
    def test_budget_preserves_hard_floor(self):
        self.assertEqual(require_budget(FLOOR + 32 * 1024 ** 2, 1024), 16781312)
        with self.assertRaises(ValueError):
            require_budget(FLOOR, 1024)

    def test_budget_rejects_unbounded_or_boolean_size(self):
        for size in (0, -1, True, 128 * 1024 ** 2 + 1):
            with self.assertRaises(ValueError):
                require_budget(10 * FLOOR, size)

    def test_inventory_rejects_symlinks(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'data').write_bytes(b'ok')
            (root / 'link').symlink_to('data')
            with self.assertRaises(ValueError):
                checked_inventory(root)

    def test_inventory_is_stable_and_content_sensitive(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'data').write_bytes(b'ok')
            first = checked_inventory(root)
            self.assertEqual(first, checked_inventory(root))
            (root / 'data').write_bytes(b'changed')
            self.assertNotEqual(first, checked_inventory(root))

    def test_all_fixed_starters_have_keyboard_routes(self):
        assets, controls, paths = keys.load_assets()
        self.assertEqual(set(assets), {'courtyard', 'switchback', 'passage'})
        self.assertEqual(len(paths), 7)
        keys.exact_json(controls, keys.expected_controls())

    def test_executables_bind_actual_runtime_identity_size_and_mode(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'bin').mkdir()
            runtime = root / 'bin/room_escape'
            runtime.write_bytes(b'known-runtime')
            runtime.chmod(0o755)
            launcher = root / 'run-room-escape'
            launcher.write_bytes(LAUNCHER)
            launcher.chmod(0o755)
            digest = base.digest(runtime)
            validate_executables(root, digest, 13)
            for wrong_hash, wrong_size in [('0' * 64, 13), (digest, 1)]:
                with self.assertRaises(ValueError):
                    validate_executables(root, wrong_hash, wrong_size)
            runtime.chmod(0o644)
            with self.assertRaises(ValueError):
                validate_executables(root, digest, 13)

    def test_launcher_contract_rejects_absolute_source_and_nonexecutable(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'bin').mkdir()
            runtime = root / 'bin/room_escape'
            runtime.write_bytes(b'known-runtime')
            runtime.chmod(0o755)
            launcher = root / 'run-room-escape'
            launcher.write_bytes(b'#!/bin/sh\nexec /original/source/runtime\n')
            launcher.chmod(0o755)
            with self.assertRaises(ValueError):
                validate_executables(root, base.digest(runtime), 13)
            launcher.write_bytes(LAUNCHER)
            launcher.chmod(0o644)
            with self.assertRaises(ValueError):
                validate_executables(root, base.digest(runtime), 13)

    def test_manifest_rejects_wrong_envelope(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'orr.export.json').write_text('{"schema":true,"payload":{},"content_digest":"x"}')
            with self.assertRaises(ValueError):
                validate_bundle(root, root, '0' * 64, 1)


if __name__ == '__main__':
    if '--binary-dir' in sys.argv:
        parser = argparse.ArgumentParser(description=__doc__)
        parser.add_argument('--binary-dir', required=True)
        parser.add_argument('--output-dir', required=True)
        parser.add_argument('--runtime', help='Explicit known trusted runtime; default is binary-dir/room_escape')
        args = parser.parse_args()
        run_integration(args.binary_dir, args.output_dir, args.runtime)
    else:
        unittest.main()
