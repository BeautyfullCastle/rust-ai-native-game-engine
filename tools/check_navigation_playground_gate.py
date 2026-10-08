#!/usr/bin/env python3
"""Exact navigation inventories and persisted evidence, using only the stdlib.

The inventory is hand-reviewed source intent, never regenerated from execution.
Synthetic fixtures validate this checker; they do not certify GPU/CI execution.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sys

import room_template_gate as shared

if not __debug__:
    raise RuntimeError('evidence checking requires Python assertions enabled')

INVENTORY = Path(__file__).with_name('navigation-playground-inventory.json')
APP_CAPTURE_NAMES = ('01-start', '02-mid', '03-paused', '04-reached', '04b-sought-mid', '05-portrait', '06-restarted-without-files', '07-view-controls')
SAMPLE_FEATURES = {'navigation-project', 'project', 'project-create', 'project-export', 'sprites'}
EDITOR_FEATURES = {'navigation-project', 'project-create', 'navigation', 'terrain', 'models'}
ARTIFACTS = {
    'production': [('orr_sample', 'navigation_playground', 'bin', False, 'navigation_playground'),
                   ('orr_sample', 'orr_export_navigation', 'bin', False, 'orr_export_navigation'),
                   ('orr_sample', 'orr_new_navigation', 'bin', False, 'orr_new_navigation')],
    'app': [('orr_sample', 'orr_sample', 'lib', True, 'app-tests')],
    'editor': [('orr_editor', 'navigation_project_editor', 'test', True, 'editor-tests')],
    'export': [('orr_sample', 'navigation_export', 'test', True, 'export-tests')],
}
FORBIDDEN = re.compile(shared.FORBIDDEN.pattern + r'|\bfallback\b|\bGPU unavailable\b', re.I)


def inventories():
    result = shared.load_json(INVENTORY.read_text())
    for name, spec in result.items():
        validate_spec(spec)
        assert spec['run_ignored'] == (name == 'readonly-export'), 'only explicit export may be ignored'
    return result


def validate_spec(spec):
    names = spec['names']
    assert names and all(re.fullmatch(r'[A-Za-z0-9_:]+', name) for name in names)
    assert len(set(names)) == len(names), 'duplicate expected names'
    assert spec['passed'] == len(names) and spec['ignored'] == 0
    assert type(spec['run_ignored']) is bool
    assert spec['ignored_names'] == (names if spec['run_ignored'] else []), 'unexpected ignored tests'
    assert type(spec['exact']) is bool
    if spec['exact']:
        assert len(names) == 1 and spec['filter'] == names[0], 'exact filter mismatch'
    for name in names:
        assert spec['filter'] in name and not any(skip in name for skip in spec['skip'])


def rust_test_names(text, prefix=''):
    """Conservative lexical module scan for these ordinary Rust test sources.

    Ignore comments/literals before tracking module scopes; this is a static
    inventory check, not a replacement for Cargo's exact --list evidence.
    """
    tokens = []
    index = 0
    while index < len(text):
        if text.startswith('//', index):
            end = text.find('\n', index)
            index = len(text) if end < 0 else end
        elif text.startswith('/*', index):
            depth, index = 1, index + 2
            while depth:
                assert index < len(text), 'unterminated Rust block comment'
                if text.startswith('/*', index):
                    depth, index = depth + 1, index + 2
                elif text.startswith('*/', index):
                    depth, index = depth - 1, index + 2
                else:
                    index += 1
        elif (raw := re.match(r'(?:br|cr|r)(#*)"', text[index:])):
            terminator = '"' + raw[1]
            end = text.find(terminator, index + raw.end())
            assert end >= 0, 'unterminated Rust raw string'
            index = end + len(terminator)
        elif text[index] == '"':
            literal = re.match(r'"(?:\\.|[^"\\])*"', text[index:], re.S)
            assert literal, 'unterminated Rust string'
            index += literal.end()
        elif text[index] == "'" and (char := re.match(r"'(?:\\.|[^'\\])'", text[index:], re.S)):
            index += char.end()
        elif (word := re.match(r'[A-Za-z_][A-Za-z0-9_]*', text[index:])):
            tokens.append(word[0])
            index += word.end()
        else:
            if text[index] in '{}#[];':
                tokens.append(text[index])
            index += 1
    scopes, names, test_pending = [], [], False
    for index, token in enumerate(tokens):
        if tokens[index:index + 4] == ['#', '[', 'test', ']']:
            test_pending = True
        elif token == 'fn' and test_pending:
            names.append('::'.join([part for part in (prefix, *scopes, tokens[index + 1]) if part]))
            test_pending = False
        elif token == '{':
            scopes.append(tokens[index - 1] if index >= 2 and tokens[index - 2] == 'mod' else None)
        elif token == '}':
            assert scopes, 'unbalanced Rust scope'
            scopes.pop()
    assert not scopes and not test_pending, 'incomplete Rust test declaration'
    return names


def check_source_inventory(specs, repo):
    sources = {}
    for spec in specs.values():
        validate_spec(spec)
        sources.setdefault(spec['source'], []).append(spec)
    for source, lanes in sources.items():
        path = Path(source)
        assert len(path.parts) == 4 and path.parts[:2] == ('crates', lanes[0]['package'])
        if path.parts[2] == 'src':
            prefix = path.stem
            library = (repo / path.parent / 'lib.rs').read_text()
            assert re.search(r'\bmod\s+' + re.escape(prefix) + r'\s*;', library), ('file module not registered', source)
        else:
            assert path.parts[2] == 'tests'
            prefix = ''
        actual = rust_test_names((repo / path).read_text(), prefix)
        wanted = {name for lane in lanes for name in lane['names']}
        assert set(actual) == wanted and len(actual) == len(wanted), ('fully qualified source inventory', source, actual, wanted)
        for lane in lanes:
            selected = [name for name in actual
                        if (name == lane['filter'] if lane['exact'] else lane['filter'] in name)
                        and not any(skip in name for skip in lane['skip'])]
            assert sorted(selected) == sorted(lane['names']), ('source filter inventory', source)


def command(spec, executable=None):
    validate_spec(spec)
    if executable is None:
        args = ['cargo', 'test', '--release', '--locked', '--no-default-features', '-p', spec['package']]
        if spec['features']:
            args += ['--features', ','.join(spec['features'])]
        args += spec['target'] + ['--']
    else:
        path = Path(executable)
        assert path.is_absolute() and path.is_file() and os.access(path, os.X_OK)
        args = [str(path)]
    if spec['filter']:
        args += [spec['filter']]
    if spec['exact']:
        args += ['--exact']
    for skip in spec['skip']:
        args += ['--skip', skip]
    return args


def check_log(spec, mode, text):
    validate_spec(spec)
    assert not FORBIDDEN.search(text), 'skip/fallback diagnostic'
    if mode in ('list', 'ignored'):
        wanted = spec['names'] if mode == 'list' else spec['ignored_names']
        names = re.findall(r'^([A-Za-z0-9_:]+): test$', text, re.M)
        assert text.count(': test') == len(names), 'unparsed inventory'
        assert sorted(names) == sorted(wanted), ('test inventory', mode, names, wanted)
        rows = re.findall(r'^(\d+) tests?, (\d+) benchmarks?$', text, re.M)
        assert rows == [(str(len(wanted)), '0')], ('list count', rows)
        assert 'test result:' not in text and ': benchmark' not in text
        return
    assert mode == 'result', mode
    rows = re.findall(r'^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; \d+ filtered out; finished in [0-9.]+s$', text, re.M)
    assert text.count('test result:') == 1, 'extra/malformed/missing summary'
    assert rows == [(str(spec['passed']), '0', '0', '0')], ('positive result count', rows)
    running = re.findall(r'^running (\d+) tests?$', text, re.M)
    assert running == [str(spec['passed'])], ('running count', running)
    # Native driver diagnostics can interleave after libtest's "test NAME ..."
    # even with --show-output. Require both exact announced tests and the final
    # positive successes list; a matching summary alone is never sufficient.
    announced = re.findall(r'^test ([A-Za-z0-9_:]+) \.\.\. ([^\n]*)$', text, re.M)
    assert sorted(name for name, _ in announced) == sorted(spec['names']), ('executed test names', announced)
    assert len(re.findall(r'^test .+ \.\.\.', text, re.M)) == len(announced), 'malformed test outcome'
    assert all(not re.match(r'(?:FAILED|ignored)\b', tail) for _, tail in announced), 'nonpassing outcome'
    assert not re.search(r'^(?:FAILED|ignored)(?:, [^\n]*)?$', text, re.M), 'nonpassing interleaved outcome'
    success_blocks = re.findall(r'^successes:\n((?:[ \t]+[A-Za-z0-9_:]+\n)+)\n(?=test result: )', text, re.M)
    assert len(success_blocks) == 1, 'missing/duplicate final successes list'
    succeeded = success_blocks[0].split()
    assert sorted(succeeded) == sorted(spec['names']), ('completed test names', succeeded)


def check_artifacts(rows, repo, group):
    """Pin only exact current-invocation target/package/features/release artifacts."""
    shared.check_cargo_rows(rows)
    assert [row['success'] for row in rows if row['reason'] == 'build-finished'] == [True]
    selected = []
    for package, name, kind, is_test, destination in ARTIFACTS[group]:
        manifest = repo / 'crates' / package / 'Cargo.toml'
        found = [row for row in rows if row['reason'] == 'compiler-artifact'
                 and row['manifest_path'] == str(manifest) and row['target']['name'] == name
                 and row['executable'] is not None]
        assert len(found) == 1, (name, 'missing/duplicate executable')
        row = found[0]
        assert row['target']['kind'] == [kind]
        assert row['target']['crate_types'] == (['lib'] if kind == 'lib' else ['bin'])
        relative = 'src/lib.rs' if kind == 'lib' else f'{"src/bin" if kind == "bin" else "tests"}/{name}.rs'
        assert row['target']['src_path'] == str(manifest.parent / relative)
        assert row['profile']['test'] is is_test and row['profile']['opt_level'] == '3'
        expected_features = EDITOR_FEATURES if package == 'orr_editor' else SAMPLE_FEATURES
        assert set(row['features']) == expected_features and len(row['features']) == len(expected_features)
        prefix, _, version = row['package_id'].rpartition('#')
        assert prefix == 'path+' + manifest.parent.as_uri()
        assert version in ('0.0.1', package + '@0.0.1')
        executable = Path(row['executable'])
        assert str(executable) in row['filenames']
        assert executable.is_absolute() and not executable.is_symlink()
        assert executable.is_file() and os.access(executable, os.X_OK)
        selected.append((destination, row))
    # Cargo integration-test invocations may also emit normal binaries for
    # CARGO_BIN_EXE. Only these exact matching artifacts are ever selected.
    assert len({row['executable'] for _, row in selected}) == len(selected), 'aliased executable paths'
    assert len({(Path(row['executable']).stat().st_dev, Path(row['executable']).stat().st_ino)
                for _, row in selected}) == len(selected), 'aliased executable inodes'
    return selected


def pin_artifacts(evidence, repo, group):
    rows = [shared.load_json(line) for line in (evidence / f'{group}-build.jsonl').read_text().splitlines() if line.strip()]
    records = []
    tools = evidence / 'tools'
    tools.mkdir(exist_ok=True)
    for destination, row in check_artifacts(rows, repo, group):
        source = Path(row['executable'])
        target = tools / destination
        assert not target.exists()
        shutil.copyfile(source, target)
        target.chmod(0o555)
        digest = shared.digest(source)
        assert shared.digest(target) == digest
        records.append({'name': destination, 'source': str(source), 'copy': str(target),
                        'sha256': digest, 'artifact': row})
    (evidence / f'{group}-artifacts.json').write_text(json.dumps(records, indent=2) + '\n')


def check_revision(evidence):
    lines = (evidence / 'source-revision.txt').read_text().splitlines()
    assert len(lines) == 2 and all(re.fullmatch(r'[0-9a-f]{40}', line) for line in lines)
    return lines[0]


def check_app_captures(evidence):
    sha = check_revision(evidence)
    app = evidence / 'app-captures'
    names = APP_CAPTURE_NAMES
    assert sorted(p.name for p in app.iterdir()) == sorted(name + ext for name in names for ext in ('.png', '.json'))
    frames = {}
    metadata = {}
    for name in names:
        frame = shared.read_png(app / (name + '.png'))
        data = shared.load_json((app / (name + '.json')).read_text())
        assert data['source_sha'] == sha and data['software'] is True
        assert data['terrain_models'] == data['overlay_models'] == 1
        assert isinstance(data['adapter'], str) and data['adapter']
        assert re.fullmatch(r'0x[0-9a-f]{16}', data['frame_checksum'])
        assert all(re.fullmatch(r'[0-9a-f]{64}', data[field]) for field in ('terrain_revision', 'graph_revision', 'navigator_checksum'))
        size = (480, 720) if name == '05-portrait' else (800, 600)
        assert frame[:2] == size and data['size'] == list(size)
        frames[name], metadata[name] = frame, data
    for a, b in [('01-start', '06-restarted-without-files'), ('02-mid', '03-paused'), ('02-mid', '04b-sought-mid')]:
        assert frames[a] == frames[b] and all(metadata[a][field] == metadata[b][field] for field in ('frame_checksum', 'navigator_checksum')), ('frame parity', a, b)
    assert metadata['01-start']['tick'] == 0 and metadata['02-mid']['tick'] == 20
    for field in ('tick', 'frame_checksum', 'navigator_checksum', 'position_raw', 'terrain_revision', 'graph_revision', 'status'):
        assert metadata['07-view-controls'][field] == metadata['01-start'][field], ('view changed simulation', field)
    assert shared.different_pixels(frames['01-start'], frames['07-view-controls']) > 0, 'view controls did not change pixels'
    assert metadata['04-reached']['status'] == 'Arrived'
    assert metadata['05-portrait']['frame_checksum'] == metadata['04-reached']['frame_checksum']
    assert shared.different_pixels(frames['01-start'], frames['02-mid']) > 40
    assert shared.different_pixels(frames['02-mid'], frames['04-reached']) > 0


def check_captures(evidence):
    check_app_captures(evidence)
    sha = check_revision(evidence)
    editor = evidence / 'editor-captures'
    assert sorted(p.name for p in editor.iterdir()) == sorted(f'editor-{name}{ext}' for name in ('start', 'mid', 'reached') for ext in ('.png', '.json'))
    editor_frames = []
    for name, tick in [('start', 0), ('mid', 24), ('reached', 300)]:
        editor_frames.append(shared.read_png(editor / f'editor-{name}.png'))
        data = shared.load_json((editor / f'editor-{name}.json').read_text())
        assert data['tick'] == tick and data['source_sha'] == sha and data['adapter']
        assert re.fullmatch(r'0x[0-9a-f]{16}', data['frame_checksum'])
    assert all(shared.different_pixels(a, b) > 0 for a, b in zip(editor_frames, editor_frames[1:]))
    check_export(evidence)


def check_export(evidence):
    sha = check_revision(evidence)
    exported = evidence / 'export-captures'
    wanted_png = [f'{prefix}-{phase}.png' for prefix in ('source', 'export') for phase in ('start', 'mid', 'reached')] + ['export-restarted.png']
    wanted_json = ['bundle-hashes-before.json', 'bundle-hashes-after.json', 'orr.export.json', 'result.json']
    assert sorted(p.name for p in exported.iterdir()) == sorted(wanted_png + wanted_json), 'export evidence inventory'
    for name in wanted_png:
        assert shared.read_png(exported / name)[:2] == (960, 720)
    for phase in ('start', 'mid', 'reached'):
        assert (exported / f'source-{phase}.png').read_bytes() == (exported / f'export-{phase}.png').read_bytes()
    assert (exported / 'export-mid.png').read_bytes() == (exported / 'export-restarted.png').read_bytes()
    hashes = shared.load_json((exported / 'bundle-hashes-before.json').read_text())
    assert hashes == shared.load_json((exported / 'bundle-hashes-after.json').read_text()), 'bundle bytes changed'
    assert {'project/orr.project.json', 'project/terrain.orrt', 'bin/navigation_playground', 'run-navigation-playground', 'orr.export.json'} <= hashes.keys()
    assert all(re.fullmatch(r'[0-9a-f]{64}', digest) for digest in hashes.values())
    result = shared.load_json((exported / 'result.json').read_text())
    for field in ('source_hidden', 'readonly', 'tamper_rejected_before_app', 'missing_rejected_before_app', 'bundle_hashes_unchanged'):
        assert type(result[field]) is bool and result[field], ('export boolean proof', field)
    checksum = result['initial_checksum']
    assert re.fullmatch(r'0x[0-9a-f]{16}', checksum)
    assert result == {'source_sha': sha, 'initial_checksum': checksum, 'source_hidden': True,
                      'readonly': True, 'frame_and_navigation_state_parity_ticks': [0, 24, 300],
                      'restart_after': 37, 'restart_observed_tick': 24, 'tamper_rejected_before_app': True,
                      'missing_rejected_before_app': True, 'bundle_hashes_unchanged': True}, 'export proof contract'
    manifest_path = exported / 'orr.export.json'
    assert shared.digest(manifest_path) == hashes['orr.export.json']
    manifest = shared.load_json(manifest_path.read_text())
    payload = manifest['payload']
    assert manifest['schema'] == 1
    assert payload['profile'] == 'terrain-point-route-authored-linux-x86_64-v1'
    assert payload['declared'] == {'target': 'linux-x86_64', 'source_revision': sha}
    assert payload['initial_checksum'] == checksum and payload['packages'] == {}
    assert payload['entry'] == {'game': 'terrain-point-route-3d-v1', 'scene': 'navigation.scene.yaml'}
    files = payload['files']
    assert len({entry['path'] for entry in files}) == len(files), 'duplicate export file'
    assert {entry['path'] for entry in files} | {'orr.export.json'} == hashes.keys()
    assert all(entry['sha256'] == hashes[entry['path']] for entry in files)
    runtime = [entry for entry in shared.load_json((evidence / 'production-artifacts.json').read_text())
               if entry['name'] == 'navigation_playground']
    assert len(runtime) == 1
    assert payload['runtime']['sha256'] == hashes['bin/navigation_playground'] == runtime[0]['sha256']
    canonical = json.dumps(payload, ensure_ascii=False, separators=(',', ':')).encode()
    assert manifest['content_digest'] == hashlib.sha256(b'orrery.terrain-point-route.export.content.v1\0' + canonical).hexdigest()


def check_creator(evidence):
    def contents(root):
        assert all(not path.is_symlink() for path in root.rglob('*'))
        return {str(path.relative_to(root)): shared.digest(path) for path in root.rglob('*') if path.is_file()}
    first, second = (evidence / name for name in ('created-project-a', 'created-project-b'))
    before = contents(first)
    assert before and before == contents(second), 'creator byte determinism'
    manifest = shared.load_json((first / 'orr.project.json').read_text())
    assert manifest['schema'] == 2 and manifest['entry'] == {'game': 'terrain-point-route-3d-v1', 'scene': 'navigation.scene.yaml'}
    assert {'orr.project.json', 'navigation.scene.yaml', 'terrain.orrt', 'README.md'} <= before.keys()
    for name in ('creator-a', 'creator-b'):
        text = (evidence / (name + '.log')).read_text()
        assert 'template: terrain-point-route-3d-v1' in text
        assert len(re.findall(r'^project initial checksum: 0x[0-9a-f]{16}$', text, re.M)) == 1
    (evidence / 'creator-hashes.json').write_text(json.dumps(before, indent=2) + '\n')


def check_default_graph(package, text):
    assert package in ('orr_sample', 'orr_editor')
    assert re.search(r'^' + package + r' v0\.0\.1(?: |$)', text, re.M), 'missing positive root package'
    assert not re.search(r'^orr_(?:navigation(?:_[a-z0-9]+)*|terrain(?:_[a-z0-9]+)*) v', text, re.M), 'default graph gained navigation/terrain'


def main():
    mode, *args = sys.argv[1:]
    if mode == 'command':
        lane, *executable = args
        assert len(executable) <= 1
        for arg in command(inventories()[lane], *executable):
            sys.stdout.buffer.write(arg.encode() + b'\0')
    elif mode == 'log':
        lane, kind, path = args
        check_log(inventories()[lane], kind, Path(path).read_text())
    elif mode == 'pin':
        evidence, repo, group = args
        pin_artifacts(Path(evidence), Path(repo), group)
    elif mode == 'captures':
        check_captures(Path(args[0]))
    elif mode == 'creator':
        check_creator(Path(args[0]))
    elif mode == 'default-graph':
        package, path = args
        check_default_graph(package, Path(path).read_text())
    else:
        raise AssertionError(mode)


if __name__ == '__main__':
    main()
