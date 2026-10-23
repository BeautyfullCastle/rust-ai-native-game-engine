#!/usr/bin/env python3
"""Fresh, source-bound checked package update acceptance. Standard library only."""
import hashlib
import json
import os
import pathlib
import re
import shutil
import sys
import struct
import traceback
import urllib.parse

if not __debug__ or sys.flags.optimize or os.environ.get('PYTHONOPTIMIZE') not in (None, '', '0'):
    raise SystemExit('package update acceptance requires nonoptimized Python')

import static_model_extract_gate as runner
import static_model_cylinder_gate as checked

FEATURE_PATHS = (
    'crates/orr_editor/src/package_manager.rs',
    'crates/orr_editor/src/package_manager/removal.rs',
    'crates/orr_editor/src/package_manager/update.rs',
    'crates/orr_editor/src/package_manager/update_tests.rs',
    'crates/orr_editor/src/package_manager_panel.rs',
    'crates/orr_editor/tests/package_update_preview_ui.rs',
    'docs/package-update-preview.md',
    'tools/package_update_gate.py',
    'tools/test_package_update_gate.py',
    '.github/workflows/package-update.yml',
)
INFRA_PATHS = ('Cargo.toml', 'Cargo.lock', 'crates/orr_editor/Cargo.toml',
    'crates/orr_editor/src/lib.rs', 'tools/static_model_extract_gate.py',
    'tools/test_static_model_extract_gate.py', 'tools/static_model_cylinder_gate.py',
    'tools/test_static_model_cylinder_gate.py')
UI_TARGET = 'package_update_preview_ui'
UI_TEST = 'raw_glb_update_key_preview_save_reopen_collect_restart'
LIB_FEATURES = ['animated-models', 'models', 'package-manager', 'room-animated-models', 'room-project']
UI_FEATURES = ['models', 'package-manager', 'room-project']
WORKFLOW_ROWS = (('cpu', 'ubuntu-22.04'), ('cpu', 'windows-latest'), ('ui', 'ubuntu-22.04'))
WORKFLOW_EVIDENCE = '${{ runner.temp }}/orr-package-update-${{ matrix.mode }}-${{ github.run_id }}-${{ github.run_attempt }}'
WORKFLOW_ARTIFACT = 'package-update-${{ matrix.mode }}-${{ matrix.os }}'
FIXTURE_COUNT = 21
# Explicit new inventories are independent of historical package/removal totals.
BACKEND_TESTS = (
    'package_manager::update::tests::update_preview_is_read_only_and_explicit_apply_preserves_room_and_other_packages',
    'package_manager::update::tests::update_rejects_same_version_changed_bytes_and_noop',
    'package_manager::update::tests::update_rejects_new_name_and_transitive_only_target',
    'package_manager::update::tests::update_rejects_every_saved_static_reference',
    'package_manager::update::tests::update_rejects_every_saved_animated_reference',
    'package_manager::update::tests::update_rejects_dependency_conflict_and_changed_closure',
    'package_manager::update::tests::update_rejects_source_bytes_or_manifest_changed_after_preview',
    'package_manager::update::tests::update_rejects_active_lock_changed_after_preview',
    'package_manager::update::tests::update_rejects_scene_camera_models_and_manifest_changed_after_preview',
    'package_manager::update::tests::update_rejects_replaced_root_and_retargeted_plan',
    'package_manager::update::tests::update_rejects_replaced_source_directory_even_with_identical_bytes',
    'package_manager::update::tests::update_rejects_tampered_candidate_or_retained_package_identity',
    'package_manager::update::tests::update_rejects_writer_contention_without_stealing_guard',
    'package_manager::update::tests::update_rejects_invalid_or_newly_referenced_saved_bindings',
)
PANEL_TESTS = (
    'package_manager_panel::update_tests::update_requires_current_explicit_preview_before_dispatch',
    'package_manager_panel::update_tests::update_preview_completion_rejects_project_source_and_aba_changes',
    'package_manager_panel::update_tests::update_completion_discards_cached_state_for_success_failure_and_stale_identity',
    'package_manager_panel::update_tests::update_disconnect_discards_inventory_and_reports_publication_uncertainty',
    'package_manager_panel::update_tests::update_preview_clears_on_project_source_or_authoring_blocker',
    'package_manager_panel::update_tests::update_dispatch_discards_removal_receipt_inventory_and_model_requests',
    'package_manager_panel::update_tests::update_preview_widgets_show_full_identities_without_publishing',
    'package_manager_panel::update_tests::update_pending_rejects_static_and_animated_assignment_requests',
    'package_manager_panel::update_tests::update_pending_and_completion_freeze_entire_editor_frame',
)
source_root = frozen = mode = None


def source_paths(root):
    return checked.source_paths(root)


def snapshot():
    return {name: {'sha256': runner.digest(source_root / name),
                   'bytes': (source_root / name).stat().st_size,
                   'mode': (source_root / name).stat().st_mode & 0o7777}
            for name in source_paths(source_root)}


def require_sources(value):
    assert set(FEATURE_PATHS + INFRA_PATHS) <= set(value), 'complete current producer sources required'


def source_fence():
    if frozen is not None:
        assert snapshot() == frozen, 'current-run source bytes, modes or paths changed'


def run(label, command, expected=0, seconds=1200):
    source_fence()
    checked.software_fence()
    output = runner.run(label, command, expected=expected, seconds=seconds)
    source_fence()
    checked.software_fence()
    return output


def initialize(root, destination, phase):
    global source_root, frozen, mode
    assert phase in {'cpu', 'ui'}
    source_root = pathlib.Path(root).resolve()
    destination = pathlib.Path(destination)
    assert destination.is_absolute() and destination == destination.resolve()
    assert not destination.is_relative_to(source_root), 'evidence must be outside source'
    frozen = None
    mode = phase
    runner.initialize(source_root, destination, 'cpu' if phase == 'cpu' else 'gpu')
    runner.report.update(mode=phase, coverage={name: 'NOT RUN' for name in ('cpu', 'ui')},
                         legacy_acceptance='NOT RUN here; existing required workflows remain required',
                         feature_paths=list(FEATURE_PATHS))
    runner.record()


def begin():
    global frozen
    assert os.environ.get('PYTHONDONTWRITEBYTECODE') == '1', 'inherited PYTHONDONTWRITEBYTECODE=1 required'
    assert run('clean-source-before', ['git', 'status', '--porcelain=v1', '--untracked-files=all']).strip() == ''
    for key, revision in [('head', 'HEAD'), ('tree', 'HEAD^{tree}')]:
        runner.report[key] = run('checkout-' + key, ['git', 'rev-parse', revision]).strip()
        assert re.fullmatch('[0-9a-f]{40}', runner.report[key])
    assert os.environ.get('GITHUB_SHA', runner.report['head']) == runner.report['head']
    tracked = run('tracked-source', ['git', 'ls-files', '-z']).split('\0')
    assert tracked.pop() == ''
    assert sorted(tracked) == source_paths(source_root), 'untracked/ignored sources are forbidden'
    frozen = snapshot()
    require_sources(frozen)
    runner.report.update(checkout=str(source_root), source_sha256=frozen,
        source_inventory_sha256=hashlib.sha256(json.dumps(frozen, sort_keys=True).encode()).hexdigest())
    runner.record()
    for file, marker in [
        ('test_static_model_extract_gate.py', 'STATIC_PROP_GATE_FIXTURES: 29 passed; 0 failed; 0 skipped'),
        ('test_static_model_cylinder_gate.py', 'STATIC_CYLINDER_GATE_FIXTURES: 28 passed; 0 failed; 0 skipped'),
        ('test_package_update_gate.py', f'PACKAGE_UPDATE_GATE_FIXTURES: {FIXTURE_COUNT} passed; 0 failed; 0 skipped'),
    ]:
        output = run('checker-' + file[:-3], [sys.executable, '-B', str(source_root / 'tools' / file)])
        assert output.rstrip().endswith(marker), 'independent checker fixture census'


def bind_producer(label):
    runner.report['producers'][label].update(
        source_inventory_sha256=runner.report['source_inventory_sha256'], source_root=str(source_root))
    runner.record()


def pin_library(output, label, features):
    """Apply the reviewed runner's producer contract to Cargo library tests."""
    artifacts = [checked.strict_json_text(line) for line in output.splitlines() if line.startswith('{')]
    found = [item for item in artifacts if item.get('reason') == 'compiler-artifact'
             and item.get('target', {}).get('name') == 'orr_editor' and item.get('executable')]
    assert len(found) == 1, 'one exact editor library test producer required'
    producer = found[0]
    crate = source_root / 'crates/orr_editor'
    target = producer['target']
    assert producer.get('fresh') is False, 'freshly compiled producer required'
    assert pathlib.Path(producer['manifest_path']).resolve() == crate / 'Cargo.toml'
    assert producer['package_id'].startswith('path+')
    uri = urllib.parse.urlsplit(producer['package_id'][5:].split('#', 1)[0])
    assert uri.scheme == 'file' and uri.netloc == ''
    path = urllib.parse.unquote(uri.path)
    assert pathlib.Path(path.lstrip('/') if os.name == 'nt' else path).resolve() == crate
    assert pathlib.Path(target['src_path']).resolve() == crate / 'src/lib.rs'
    assert target['kind'] == ['lib'] and target['crate_types'] == ['lib']
    assert producer['profile']['test'] is True and producer['profile']['opt_level'] == '3'
    assert sorted(producer['features']) == sorted(features)
    original = pathlib.Path(producer['executable'])
    assert original.is_absolute() and original == original.resolve() and not original.is_symlink()
    assert original.parent == runner.target_dir / 'release/deps'
    checked.regular(original, 128 * 1024**2)
    assert original.stat().st_size > 0
    copied = runner.evidence / (label + ('.exe' if os.name == 'nt' else ''))
    with original.open('rb') as source, copied.open('xb') as destination:
        shutil.copyfileobj(source, destination, 1024**2)
    shutil.copymode(original, copied)
    assert runner.digest(original) == runner.digest(copied)
    runner.report['producers'][label] = {'producer': producer, 'file': copied.name,
        'sha256': runner.digest(copied), 'bytes': copied.stat().st_size}
    bind_producer(label)
    return str(copied)


def inventory_names(output):
    names = re.findall(r'^([A-Za-z0-9_:]+): test$', output, re.M)
    assert len(names) == len(set(names))
    runner.inventory(output, names)
    return names


def executed(output, names, filtered=0):
    assert type(filtered) is int and filtered >= 0
    normalized = output.replace('\r\n', '\n')
    suffix = f'; 0 ignored; 0 measured; {filtered} filtered out;'
    assert normalized.count(suffix) == 1, 'exact filtered-out inventory required'
    runner.result(normalized.replace(suffix, '; 0 ignored; 0 measured; 0 filtered out;', 1), names)


def test_scope(label, binary, prefix, names, total):
    assert names and len(names) == len(set(names)) and all(name.startswith(prefix) for name in names)
    runner.inventory(run(label + '-inventory', [binary, prefix, '--list']), names)
    runner.inventory(run(label + '-ignored', [binary, prefix, '--ignored', '--list']), [])
    executed(run(label + '-run', [binary, prefix, '--test-threads=1']), names, total - len(names))
    runner.report.setdefault('test_partitions', {})[label] = {'names': names, 'executed': len(names),
                                                            'filtered': total - len(names)}
    runner.record()


def flags():
    return ['--locked', '--release', '-p', 'orr_editor', '--no-default-features']


def cpu():
    enabled = ['--features', 'package-manager,room-animated-models']
    output = run('build-editor-library', ['cargo', 'test', *flags(), *enabled, '--lib',
                 '--no-run', '--message-format=json'], seconds=2400)
    binary = pin_library(output, 'editor-library-test', LIB_FEATURES)
    all_names = inventory_names(run('editor-library-inventory', [binary, '--list']))
    test_scope('new-update', binary, 'package_manager::update::tests::', list(BACKEND_TESTS), len(all_names))
    test_scope('new-panel-update', binary, 'package_manager_panel::update_tests::', list(PANEL_TESTS), len(all_names))
    # Regression partitions are retained separately; they never inflate new coverage.
    for label, prefix, count in [('legacy-removal', 'package_manager::removal::tests::', 20 if os.name == 'nt' else 22),
                                  ('legacy-panel', 'package_manager_panel::tests::', 14)]:
        names = [name for name in all_names if name.startswith(prefix)]
        assert len(names) == count, 'legacy regression census changed'
        test_scope(label, binary, prefix, names, len(all_names))
    for label, command in [
        ('strict-editor-library', ['cargo', 'clippy', *flags(), *enabled, '--lib', '--', '-D', 'warnings']),
        ('default-off-check', ['cargo', 'check', *flags(), '--lib']),
        ('minimal-package-manager-check', ['cargo', 'check', *flags(), '--features', 'package-manager', '--lib']),
        ('animated-without-package-manager-check', ['cargo', 'check', *flags(), '--features', 'room-animated-models', '--lib']),
    ]:
        run(label, command)
    runner.report['coverage']['cpu'] = 'PASSED'
    runner.record()


def check_lock_delta(before, after, name):
    assert set(before) == set(after) == {'schema', 'direct', 'packages'}
    assert type(before['schema']) is int and type(after['schema']) is int and before['schema'] == after['schema'] == 1
    assert set(before['direct']) == set(after['direct']) and set(before['packages']) == set(after['packages'])
    assert name in before['direct'] and name in before['packages']
    old, new = before['packages'][name], after['packages'][name]
    assert old['manifest']['name'] == new['manifest']['name'] == name
    assert before['direct'][name] == old['manifest']['version']
    assert after['direct'][name] == new['manifest']['version']
    assert before['direct'][name] != after['direct'][name], 'different exact selected version required'
    assert old['digest'] != new['digest'], 'different selected identity required'
    assert old['manifest']['dependencies'] == new['manifest']['dependencies'], 'dependency changes forbidden'
    for key in before['direct']:
        if key != name:
            assert before['direct'][key] == after['direct'][key], 'other direct choice changed'
    for key in before['packages']:
        if key != name:
            assert before['packages'][key] == after['packages'][key], 'other package identity changed'


def check_package_object(project, package, source=None):
    assert set(package) == {'manifest', 'digest', 'files'}
    manifest = package['manifest']
    keys = ('schema', 'name', 'version', 'engine', 'capabilities', 'dependencies', 'files')
    assert set(manifest) == set(keys)
    assert manifest['files'] == sorted(package['files'])
    assert manifest['capabilities'] == sorted(set(manifest['capabilities']))
    canonical = {key: manifest[key] for key in keys}
    canonical['dependencies'] = dict(sorted(manifest['dependencies'].items()))
    hashes = dict(sorted(package['files'].items()))
    encoded = json.dumps(['orr-package-v1', canonical, hashes], ensure_ascii=False, separators=(',', ':')).encode()
    assert package['digest'] == hashlib.sha256(encoded).hexdigest(), 'canonical package identity mismatch'
    objects = project / '.orr/packages/objects' / package['digest']
    assert checked.strict_json(objects / 'orr.package.json') == manifest, 'cached package manifest mismatch'
    for name, digest in hashes.items():
        assert type(name) is str and not pathlib.PurePosixPath(name).is_absolute()
        assert all(part not in ('', '.', '..') for part in name.split('/'))
        assert re.fullmatch('[0-9a-f]{64}', digest)
        installed = checked.regular(objects / name)
        assert runner.digest(installed) == digest, 'installed immutable file hash mismatch'
        if source is not None:
            assert checked.regular(source / name).read_bytes() == installed.read_bytes(), 'source/installed bytes mismatch'
    if source is not None:
        assert checked.strict_json(source / 'orr.package.json') == manifest


def glb_chunks(path):
    data = checked.regular(path).read_bytes()
    assert len(data) >= 28 and data[:4] == b'glTF'
    version, length = struct.unpack_from('<II', data, 4)
    assert version == 2 and length == len(data)
    offset = 12; chunks = []
    while offset < len(data):
        assert offset + 8 <= len(data)
        size, kind = struct.unpack_from('<II', data, offset)
        offset += 8
        assert size % 4 == 0 and offset + size <= len(data)
        chunks.append((kind, data[offset:offset+size])); offset += size
    assert len(chunks) == 2 and chunks[0][0] == 0x4e4f534a and chunks[1][0] == 0x004e4942
    return checked.strict_json_text(chunks[0][1].decode('utf-8')), chunks[1][1]


UI_PNGS = ('01-v1-canvas', '01-v1-preview', '02-update-preview', '03-updated',
           '04-v2-canvas', '04-v2-preview', '05-saved-viewport', '05-saved',
           '06-reopened-viewport', '06-reopened', '11-collected-viewport', '11-collected',
           '12-restarted-viewport', '12-restarted')
UI_TRUE_FIELDS = ('raw_import_v1_v2', 'preview_read_only', 'explicit_update', 'other_packages_unchanged',
    'source_change_invalidates_preview', 'retained_candidate_rejected', 'old_undo_rejected', 'dithering',
    'sources_hidden', 'fresh_reopen_binding_exact', 'fresh_reopen_viewport_pixels_exact',
    'actual_play_take_control_keyboard_interact', 'restart_viewport_pixels_exact',
    'presentation_free_frame_bytes_exact', 'presentation_free_checksums_exact', 'source_and_installed_files_preserved')
UI_FIELDS = {'schema', 'status', 'test', 'mandatory_test_count', 'test_source_fnv1a64', 'package',
    'old_version', 'new_version', 'old_digest', 'new_digest', 'raw_v1_fnv1a64', 'raw_v2_fnv1a64',
    'assigned_guid', 'assigned_role', 'assigned_binding', 'source_hash', 'adapter', 'renderer_options',
    'predictable_texture_filtering', 'version_changed_pixels', 'collect_changed_pixels',
    'normalized_viewport_size', 'key_placements_before_collect', 'key_placements_after_collect',
    'key_placements_after_restart', 'key_collected', 'restart_key_collected', 'recorded_ticks',
    'recorded_checksums_checked', 'authoritative_checksum'} | set(UI_TRUE_FIELDS)


def check_ui_receipt(receipt):
    assert set(receipt) == UI_FIELDS, 'exact update acceptance receipt fields'
    assert type(receipt['schema']) is int and receipt['schema'] == 1 and receipt['status'] == 'passed'
    assert receipt['test'] == UI_TEST and type(receipt['mandatory_test_count']) is int and receipt['mandatory_test_count'] == 1
    assert receipt['test_source_fnv1a64'] == checked.fnv1a64((source_root / f'crates/orr_editor/tests/{UI_TARGET}.rs').read_bytes())
    assert type(receipt['adapter']) is str and any(token in receipt['adapter'].lower() for token in ('llvmpipe', 'lavapipe'))
    assert receipt['renderer_options'] == 'default' and receipt['predictable_texture_filtering'] is False
    for field in UI_TRUE_FIELDS:
        assert receipt[field] is True, field
    assert receipt['package'] == 'raw-package-update'
    assert receipt['old_version'] == '1.0.0' and receipt['new_version'] == '2.0.0'
    assert receipt['assigned_role'] == 'KEY' and re.fullmatch('e_[0-9a-f]{8}', receipt['assigned_guid'])
    assert receipt['normalized_viewport_size'] == [900, 850]
    for field, value in [('key_placements_before_collect', 2), ('key_placements_after_collect', 1),
                         ('key_placements_after_restart', 2), ('key_collected', 1), ('restart_key_collected', 0)]:
        assert type(receipt[field]) is int and receipt[field] == value, field
    for field in ('recorded_ticks', 'recorded_checksums_checked'):
        assert type(receipt[field]) is int and 0 < receipt[field] <= 10000
    assert receipt['recorded_checksums_checked'] <= receipt['recorded_ticks'] + 1
    assert re.fullmatch('[0-9a-f]{16}', receipt['authoritative_checksum'])


def check_ui(captures):
    expected = {name + '.png' for name in UI_PNGS} | {'receipt.json', 'room', 'hidden-raw-inputs',
        'hidden-package-sources', 'lock-before-update.json', 'lock-after-update.json', 'actual-gui.orrp',
        'collected-authoritative.frame', 'presentation-free.frame'}
    inventory = checked.capture_files(captures, expected)
    receipt = checked.strict_json(captures / 'receipt.json')
    check_ui_receipt(receipt)
    frames = {}
    for name in UI_PNGS:
        path = captures / (name + '.png'); header = checked.regular(path).read_bytes()[:33]
        assert len(header) == 33 and header[12:16] == b'IHDR'
        width, height = struct.unpack_from('>II', header, 16)
        assert 0 < width <= 2048 and 0 < height <= 2048
        frames[name] = ((width, height), checked.png_pixels(path, width, height))
    for field, first, second in [('version_changed_pixels', '01-v1-canvas', '04-v2-canvas'),
                                 ('collect_changed_pixels', '06-reopened-viewport', '11-collected-viewport')]:
        assert frames[first][0] == frames[second][0]
        observed = checked.difference(frames[first][1], frames[second][1])
        assert type(receipt[field]) is int and receipt[field] == observed and observed > 1000
    assert frames['05-saved-viewport'] == frames['06-reopened-viewport'] == frames['12-restarted-viewport']
    for name in ('05-saved-viewport', '06-reopened-viewport', '11-collected-viewport', '12-restarted-viewport'):
        dimensions, pixels = frames[name]
        assert dimensions == (900, 850)
        assert sum(pixels[index:index+4] != pixels[:4] for index in range(0, len(pixels), 4)) > 1000, 'nonblank production viewport required'
    before = checked.strict_json(captures / 'lock-before-update.json')
    after = checked.strict_json(captures / 'lock-after-update.json')
    project = captures / 'room'
    assert checked.strict_json(project / 'orr.packages.lock.json') == after
    check_lock_delta(before, after, receipt['package'])
    assert set(before['packages']) == {'raw-package-update', 'sample-imported-scene'}
    for lock, version in ((before, '1'), (after, '2')):
        package = lock['packages'][receipt['package']]
        assert package['digest'] == receipt['old_digest' if version == '1' else 'new_digest']
        assert set(package['files']) == {'LICENSE.txt', 'model.glb'}
        check_package_object(project, package, captures / 'hidden-package-sources' / ('v' + version))
        raw = captures / 'hidden-raw-inputs' / ('v' + version + '.glb')
        assert checked.fnv1a64(raw.read_bytes()) == receipt['raw_v' + version + '_fnv1a64']
        assert raw.read_bytes() == (captures / 'hidden-package-sources' / ('v' + version) / 'model.glb').read_bytes()
    check_package_object(project, after['packages']['sample-imported-scene'])
    raw = captures / 'hidden-raw-inputs'
    assert (raw / 'v1.glb').read_bytes() == (source_root / 'assets/imported_scene_demo/foreground.glb').read_bytes()
    first, first_bin = glb_chunks(raw / 'v1.glb'); second, second_bin = glb_chunks(raw / 'v2.glb')
    assert first_bin == second_bin and first != second
    assert second['nodes'][0]['scale'] == [0.5, 1.6, 1.2]
    assert second['materials'][0]['pbrMetallicRoughness']['baseColorFactor'] == [0.15, 0.9, 0.25, 1.0]
    sidecar = checked.strict_json(project / 'room.models.json')
    assert sidecar['version'] == 2 and sidecar['scene'] == 'room.scene.yaml' and sidecar['project'] == '.'
    assert len(sidecar['bindings']) == 2
    binding = sidecar['bindings'][receipt['assigned_guid']]
    assert set(binding) == {'kind', 'package', 'asset', 'package_digest', 'source_hash', 'transform'}
    assert binding == receipt['assigned_binding'] and binding['kind'] == 'static'
    assert binding['package'] == receipt['package'] and binding['asset'] == 'model.glb'
    assert binding['package_digest'] == receipt['new_digest']
    assert binding['source_hash'] == receipt['source_hash'] == runner.digest(raw / 'v2.glb')
    assert binding['transform'] == {'translation': [0., 0., 0.], 'rotation': [0., 0., 0., 1.], 'scale': [1., 1., 1.]}
    for guid, other in sidecar['bindings'].items():
        if guid != receipt['assigned_guid']:
            assert other['package'] == 'sample-imported-scene'
            assert other['package_digest'] == before['packages']['sample-imported-scene']['digest']
    scene = checked.regular(project / 'room.scene.yaml').read_text(encoding='utf-8')
    assert f"  {receipt['assigned_guid']}:\n    RoomEscapeV1::Actor: {{ kind: 2, ordinal: 0 }}\n" in scene
    manifest = checked.strict_json(project / 'orr.project.json')
    assert manifest['entry']['game'] == 'room-escape-v1' and manifest['entry']['models'] == 'room.models.json'
    authoritative = checked.regular(captures / 'collected-authoritative.frame').read_bytes()
    assert len(authoritative) > 32 and authoritative == checked.regular(captures / 'presentation-free.frame').read_bytes()
    assert checked.regular(captures / 'actual-gui.orrp').stat().st_size > 32
    runner.report.update(ui_capture_inventory=inventory, ui_receipt=receipt)
    runner.record()


def ui():
    checked.configure_software()
    captures = runner.evidence / 'captures'
    assert not captures.exists(), 'fresh absent capture directory required'
    os.environ['ORR_PACKAGE_UPDATE_CAPTURE_DIR'] = str(captures)
    output = run('build-editor-ui', ['cargo', 'test', *flags(), '--features', 'package-manager',
                 '--test', UI_TARGET, '--no-run', '--message-format=json'], seconds=2400)
    binary = runner.pin(output, 'editor-ui-test', 'orr_editor', UI_TARGET, 'test', UI_FEATURES)
    bind_producer('editor-ui-test')
    run('strict-editor-ui', ['cargo', 'clippy', *flags(), '--features', 'package-manager',
                            '--test', UI_TARGET, '--', '-D', 'warnings'])
    runner.inventory(run('ui-inventory', [binary, '--list']), [UI_TEST])
    runner.inventory(run('ui-ignored-inventory', [binary, '--ignored', '--list']), [UI_TEST])
    runner.result(run('ui-run', [binary, '--exact', UI_TEST, '--ignored', '--test-threads=1']), [UI_TEST])
    check_ui(captures)
    runner.report['coverage']['ui'] = 'PASSED'
    runner.record()


def check_coverage():
    assert runner.report['coverage'] == {name: 'PASSED' if name == mode else 'NOT RUN' for name in ('cpu', 'ui')}


def finish(error):
    try:
        assert frozen is not None, 'prebuild source snapshot missing'
        source_fence()
        runner.fence()
        checked.software_fence()
        assert run('clean-source-after', ['git', 'status', '--porcelain=v1', '--untracked-files=all']).strip() == ''
        for key, revision in [('head', 'HEAD'), ('tree', 'HEAD^{tree}')]:
            assert run('checkout-' + key + '-after', ['git', 'rev-parse', revision]).strip() == runner.report[key]
        tracked = run('tracked-source-after', ['git', 'ls-files', '-z']).split('\0')
        assert tracked.pop() == '' and sorted(tracked) == list(frozen)
        runner.report['source_unchanged'] = True
        if error is None:
            check_coverage()
    except BaseException as secondary:
        runner.report['final_fence_error'] = repr(secondary)
        error = error or secondary
    runner.report['status'] = 'failed' if error is not None else 'passed'
    if error is not None:
        runner.report['failure'] = ''.join(traceback.format_exception(error))
    runner.record()
    if error is not None:
        raise SystemExit(error.code if isinstance(error, runner.GateFailure) else 1)


def main():
    assert len(sys.argv) == 2 and sys.argv[1] in {'cpu', 'ui'}, 'usage: package_update_gate.py cpu|ui'
    initialize(pathlib.Path(__file__).resolve().parents[1],
               pathlib.Path(os.environ['ORR_PACKAGE_UPDATE_EVIDENCE']), sys.argv[1])
    error = None
    try:
        begin()
        {'cpu': cpu, 'ui': ui}[mode]()
    except BaseException as failure:
        error = failure
    finally:
        finish(error)


if __name__ == '__main__':
    main()
