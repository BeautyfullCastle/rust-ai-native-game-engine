#!/usr/bin/env python3
"""Additive, source-bound Room wall local-X mirror CPU/Linux Vulkan acceptance.

This gate cannot certify Windows hardware UI or merge eligibility. Every route
requires a clean, complete checkout, fresh retained producer and actual execution.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import shutil
import stat
import struct
import sys
import traceback
import urllib.parse


def require(value, message='Room wall mirror acceptance contract failed'):
    if not value:
        raise AssertionError(message)


def require_python():
    value = {'optimize': sys.flags.optimize, '__debug__': __debug__,
             'dont_write_bytecode': sys.dont_write_bytecode,
             'PYTHONOPTIMIZE': os.environ.get('PYTHONOPTIMIZE'),
             'PYTHONDONTWRITEBYTECODE': os.environ.get('PYTHONDONTWRITEBYTECODE')}
    if (value['optimize'] != 0 or value['__debug__'] is not True
            or value['dont_write_bytecode'] is not True
            or value['PYTHONOPTIMIZE'] is not None
            or value['PYTHONDONTWRITEBYTECODE'] != '1'):
        print('ROOM_WALL_MIRROR_PYTHON_REJECTED ' + json.dumps(value), file=sys.stderr)
        raise SystemExit(2)
    return value


# The runner and PNG decoder use assertions; reject optimization before import.
require_python()
import static_model_extract_gate as runner
from static_model_uv_gate import png_pixels
from room_wall_gate import fixture_result

ROOT = Path(__file__).resolve().parents[1]
CPU_PREFIX = 'room_wall_stamp::mirror_tests::'
CPU_TESTS = tuple(CPU_PREFIX + name for name in ('all_eight_orientations_match_literal_asymmetric_nonorigin_oracle', 'all_orientations_leave_source_encoding_unchanged', 'final_i64_overflow_is_rejected_without_mutating_source', 'final_serialized_size_is_checked_after_mirroring_without_source_mutation', 'host_invalid_final_scene_rolls_back_whole_group_and_preserves_redo', 'host_invalid_mirror_preserves_scene_history_and_pending_redo', 'host_mirrored_group_preserves_existing_entities_and_is_one_undoable_edit', 'invalid_turns_and_late_invalid_source_wall_leave_source_unchanged', 'mirror_fixed_axis_retains_original_decimal_tokens', 'no_mirror_matches_legacy_four_turns_bit_exact', 'preserves_names_y_heights_identity_rotation_and_every_unowned_field', 'raw_fixed_point_reflections_roundtrip_without_drift', 'reflecting_twice_restores_geometry_and_encoded_bytes', 'room_boundaries_are_exact_and_one_raw_step_past_is_atomic', 'wide_intermediate_differences_cancel_before_final_i64_conversion', 'wide_pivot_restoration_can_cancel_with_offset_before_bounds_check'))
GPU_TARGET = 'room_wall_stamp_mirror_ui'
GPU_TEST = 'linux_room_wall_mirror_quarterturns_history_reopen_and_collision'
FIXTURE_COUNT = 29
FEATURES = {
    'Linux': ('room-authoring', ('default', 'models', 'room-authoring', 'room-project')),
    'Windows': ('windows-projects', ('default', 'models', 'package-manager', 'room-authoring',
                                     'room-project', 'windows-export', 'windows-projects')),
}
REQUIRED_PATHS = (
    'Cargo.toml', 'Cargo.lock', 'crates/orr_editor/Cargo.toml',
    'crates/orr_editor/src/lib.rs', 'crates/orr_editor/src/room_wall_stamp.rs',
    'crates/orr_editor/src/room_wall_stamp/placement.rs',
    'crates/orr_editor/src/room_wall_stamp/placement/mirror_tests.rs',
    'crates/orr_editor/src/room_wall_stamp_panel.rs',
    'crates/orr_editor/src/room_authoring.rs',
    'crates/orr_editor/tests/room_wall_stamp_mirror_ui.rs',
    'tools/room_wall_mirror_gate.py', 'tools/test_room_wall_mirror_gate.py',
    'tools/static_model_extract_gate.py', 'tools/test_static_model_extract_gate.py',
    'tools/static_model_uv_gate.py', 'tools/room_wall_gate.py',
    'docs/room-wall-group-mirror.md', '.github/workflows/room-wall-mirror.yml',
)
# Pin existing acceptance infrastructure without changing its counts or sources.
PRESERVED_SHA256 = {'tools/check-room-wall-groups.py': 'b549a68b36ffa3616d26d4f48788c75118d2ac9c54af0c53528c52d6109ea472', 'tools/check-room-wall-group-rotation.py': '875930f0e01d117e7d4ba2a265a4a25c949aece11afccd96e638d6a569cf1b9d', '.github/workflows/room-wall-groups.yml': 'a8b1335f31037b75405487762a633c6818b64b256664face9d84acdf3bdbc5ac', '.github/workflows/room-wall-group-rotation.yml': 'a9e20c663b4f9f7bfc849e99698e3b7e319d4bfca47ccd6a88728dca280aeffc', 'crates/orr_editor/tests/room_wall_stamp_ui.rs': '39e082dd0d010a642550a0b6228ae05023f175cabf8e5720d7d237047f707e18', 'crates/orr_editor/tests/room_wall_stamp_rotation_ui.rs': '396d9937568987d10cd3ace9f9fb6b032c550ea540eb9294639d97906a19ef3a', 'crates/orr_editor/tests/room_wall_stamp_linux_ui.rs': 'dbb830eff471ffe8e4218046d96d28872039a32ef6d72be537d696bd2d95f730', 'crates/orr_editor/src/room_wall_stamp/placement/tests.rs': '8b73411e5ec2ef5e4c2e64f42e21f3da8653d519d89f1efa6effb6ec8e292f75', 'crates/orr_editor/src/room_wall_stamp/tests.rs': 'ff6e2fde8d6ea233ea880e06f5c5615c9aa6d770b879e2d0a23d3d7a8c7fa212', 'crates/orr_editor/src/room_wall_stamp_store/tests.rs': '26f7c77e37c560837c6770c8f3dfd6dadebd14ae6258699500c6c9a1eefb69b4', 'crates/orr_editor/src/room_wall_stamp_store/linux_tests.rs': '78cd6f9aad4e2e923dd69d05144ea6513cf4f00c99f3d3cab206baa12ba03cdc', 'docs/room-wall-group-rotation.md': '5ea783c8ef3af960b987bb7cae6a714da1a5c7af175300fd83246b4a9631fef7'}
WORKFLOW_SHA256 = '6d9e952607c7bd0e8c5be2d1fb6a298886d39d0d06981a20e79b27e0bf02e526'
CAPTURES = ('01-source-composed.png', '01-source-viewport.png', '02-controls-mirror-false-90-degrees.png', '02-controls-mirror-true-0-degrees.png', '02-controls-mirror-true-180-degrees.png', '02-controls-mirror-true-270-degrees.png', '02-controls-mirror-true-90-degrees.png', '03-gameplay-mirror-false-90-degrees.png', '03-gameplay-mirror-true-0-degrees.png', '03-gameplay-mirror-true-180-degrees.png', '03-gameplay-mirror-true-270-degrees.png', '03-gameplay-mirror-true-90-degrees.png', '04-mirrored-saved-composed.png', '04-mirrored-saved-viewport.png', '05-fresh-source-hidden-composed.png', '05-fresh-source-hidden-viewport.png', '06-fresh-source-hidden-gameplay.png')
TRUE_FIELDS = (
    'production_editor_app', 'mirror_default_false', 'actual_mirror_control',
    'mirror_before_quarterturn_exact_geometry', 'capture_save_load_check_instantiate',
    'fresh_guids_and_redo_identity', 'one_atomic_history_entry',
    'invalid_placement_preserves_redo', 'loaded_template_bytes_unchanged',
    'source_hidden_during_fresh_reopen', 'reopen_pixels_equal',
    'all_routes_same_xz_start', 'mirror_toggle_preserves_redo',
    'all_project_sidecars_and_installed_assets_unchanged', 'source_geometry_unchanged',
    'windows_hardware_ui_required_before_merge',
)
source_root = frozen = mode = None


def regular(path, limit=32 * 1024**2):
    require(path.is_file() and not path.is_symlink() and 0 < path.stat().st_size <= limit,
            'bounded regular file required: ' + str(path))
    return path


def strict_json(path):
    regular(path)
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, 'duplicate JSON key')
            result[key] = value
        return result
    def reject(value):
        raise AssertionError('nonfinite JSON number: ' + value)
    value = json.loads(path.read_text(encoding='utf-8'), object_pairs_hook=pairs,
                       parse_constant=reject)
    def finite(node):
        if isinstance(node, dict):
            for child in node.values():
                finite(child)
        elif isinstance(node, list):
            for child in node:
                finite(child)
        elif isinstance(node, float):
            require(math.isfinite(node), 'nonfinite JSON exponent')
    finite(value)
    return value


def source_paths(root):
    result = []
    for directory, folders, files in os.walk(root, followlinks=False):
        parent = Path(directory)
        if parent == root:
            folders[:] = [name for name in folders if name not in {'.git', 'target'}]
        for name in folders:
            require(not (parent / name).is_symlink(), 'source directory symlink')
        for name in files:
            if parent == root and name == '.git':
                continue  # Git worktree metadata is not a source file.
            path = parent / name
            require(path.is_file() and not path.is_symlink(), 'source file is not regular')
            result.append(path.relative_to(root).as_posix())
    return sorted(result)


def snapshot():
    return {name: {'bytes': (source_root / name).stat().st_size,
                   'mode': stat.S_IMODE((source_root / name).stat().st_mode),
                   'sha256': runner.digest(source_root / name)}
            for name in source_paths(source_root)}


def check_inventory(tracked, phase):
    actual = source_paths(source_root)
    item = {'expected': sorted(tracked), 'observed': actual,
            'extra': sorted(set(actual) - set(tracked)),
            'missing': sorted(set(tracked) - set(actual)),
            'matches': actual == sorted(tracked) and len(set(tracked)) == len(tracked)}
    runner.report.setdefault('source_path_inventories', {})[phase] = item
    runner.record()
    require(item['matches'], 'untracked, ignored, duplicate or missing source path')


def source_fence():
    if frozen is not None:
        current = snapshot()
        if list(current) != list(frozen):
            check_inventory(list(frozen), 'fence')
        require(current == frozen, 'source bytes, modes or inventory changed')


def run(label, command, seconds=1200):
    require_python()
    source_fence()
    result = runner.run(label, command, seconds=seconds)
    source_fence()
    return result


def initialize(root, destination, phase):
    global source_root, frozen, mode
    require_python()
    require(phase in ('cpu', 'gpu'), 'unknown gate route')
    source_root, frozen, mode = Path(root).resolve(), None, phase
    destination = Path(destination)
    require(destination.is_absolute() and destination == destination.resolve(),
            'canonical absolute evidence path required')
    require(not destination.is_relative_to(source_root)
            and not source_root.is_relative_to(destination), 'evidence must be outside checkout')
    require(not destination.exists(), 'fresh exclusive evidence required')
    runner.initialize(source_root, destination, phase)
    runner.report.update(mode=phase, python_runtime=require_python(),
                         coverage={'cpu': 'NOT RUN', 'linux_software_vulkan_ui': 'NOT RUN',
                                   'windows_hardware_vulkan_ui': 'NOT RUN'},
                         merge_eligibility=False, physical_os_input=False)
    temporary = destination / 'temporary'
    temporary.mkdir(mode=0o700)
    os.environ.update(TMPDIR=str(temporary), TMP=str(temporary), TEMP=str(temporary))
    runner.report['private_temporary'] = str(temporary)
    runner.report['cargo_environment'] = {key: value for key, value in os.environ.items()
                                         if key.startswith('CARGO_PROFILE_') or key in ('CARGO_BUILD_JOBS', 'CARGO_INCREMENTAL')}
    runner.record()


def workflow_contract():
    for name, digest in PRESERVED_SHA256.items():
        require(runner.digest(source_root / name) == digest, 'changed preserved acceptance: ' + name)
    require(WORKFLOW_SHA256 and runner.digest(source_root / '.github/workflows/room-wall-mirror.yml')
            == WORKFLOW_SHA256, 'changed mirror workflow wiring')
    runner.report['workflow_contract'] = 'passed'
    runner.record()


def begin():
    global frozen
    require(run('clean-source-before', ['git', 'status', '--porcelain=v1', '--untracked-files=all']).strip() == '',
            'clean committed source required')
    for key, revision in (('head', 'HEAD'), ('tree', 'HEAD^{tree}')):
        value = run('source-' + key, ['git', 'rev-parse', revision]).strip()
        require(re.fullmatch('[0-9a-f]{40}', value), 'exact Git source identity')
        runner.report[key] = value
    if 'GITHUB_SHA' in os.environ:
        require(os.environ['GITHUB_SHA'] == runner.report['head'], 'hosted source SHA mismatch')
    tracked = run('tracked-source', ['git', 'ls-files', '-z']).split('\0')
    require(tracked.pop() == '' and tracked, 'Git source inventory')
    check_inventory(tracked, 'before')
    frozen = snapshot()
    require(set(REQUIRED_PATHS) <= set(frozen), 'complete mirror implementation and acceptance source')
    runner.report.update(source_sha256=frozen,
                         source_inventory_sha256=hashlib.sha256(json.dumps(frozen, sort_keys=True).encode()).hexdigest())
    runner.record()
    workflow_contract()
    for file, count, marker in (
        ('test_static_model_extract_gate.py', 29, 'STATIC_PROP_GATE_FIXTURES'),
        ('test_room_wall_mirror_gate.py', FIXTURE_COUNT, 'ROOM_WALL_MIRROR_GATE_FIXTURES'),
    ):
        output = run('selftest-' + file[:-3], [sys.executable, '-B', str(source_root / 'tools' / file)])
        fixture_result(output, count, f'{marker}: {count} passed; 0 failed; 0 skipped')


def pin(output, label, target, kind, features):
    """Retain only a freshly compiled, exact current-checkout test producer."""
    source_fence()
    records = [json.loads(line) for line in output.splitlines() if line.startswith('{')]
    found = [item for item in records if item.get('reason') == 'compiler-artifact'
             and item.get('target', {}).get('name') == target and item.get('executable')]
    require(len(found) == 1, 'exactly one executable producer')
    producer = found[0]
    crate = source_root / 'crates/orr_editor'
    require(producer.get('fresh') is False, 'freshly compiled producer required')
    require(Path(producer['manifest_path']).resolve() == crate / 'Cargo.toml', 'producer manifest')
    require(producer['package_id'].startswith('path+'), 'local producer package')
    uri = urllib.parse.urlsplit(producer['package_id'][5:].split('#', 1)[0])
    require(uri.scheme == 'file' and uri.netloc == '', 'local producer URI')
    package_path = urllib.parse.unquote(uri.path)
    if os.name == 'nt':
        package_path = package_path.lstrip('/')
    require(Path(package_path).resolve() == crate, 'current checkout package')
    source = crate / ('src/lib.rs' if kind == 'lib' else 'tests/' + target + '.rs')
    require(Path(producer['target']['src_path']).resolve() == source, 'producer source identity')
    require(producer['target']['kind'] == [kind]
            and producer['target']['crate_types'] == (['lib'] if kind == 'lib' else ['bin']),
            'producer target identity')
    require(producer['profile']['test'] is True and producer['profile']['opt_level'] == '0'
            and producer['profile'].get('debug_assertions') is True,
            'normal test profile required')
    require(sorted(producer['features']) == sorted(features), 'exact enabled feature closure')
    original = Path(producer['executable'])
    regular(original, 512 * 1024**2)
    require(original.is_absolute() and original == original.resolve()
            and original.parent == runner.target_dir / 'debug/deps', 'private fresh target authority')
    retained = runner.evidence / (label + ('.exe' if os.name == 'nt' else ''))
    with original.open('rb') as source_file, retained.open('xb') as output_file:
        shutil.copyfileobj(source_file, output_file, 1024**2)
    shutil.copymode(original, retained)
    require(runner.digest(original) == runner.digest(retained), 'retained producer integrity')
    runner.report['producers'][label] = {
        'producer': producer, 'file': retained.name, 'sha256': runner.digest(retained),
        'bytes': retained.stat().st_size, 'source_head': runner.report['head'],
        'source_tree': runner.report['tree'],
        'source_inventory_sha256': runner.report['source_inventory_sha256'],
    }
    runner.record()
    source_fence()
    return str(retained)


def inventory(output):
    value = output.replace('\r\n', '\n')
    require('\r' not in value, 'bare carriage return in inventory')
    names = re.findall(r'^([A-Za-z0-9_:]+): test$', value, re.M)
    require(len(names) == len(set(names)), 'duplicate listed test')
    require(re.findall(r'^(\d+) tests?, 0 benchmarks$', value, re.M) == [str(len(names))],
            'single exact inventory total')
    require(all(re.fullmatch(r'[A-Za-z0-9_:]+: test|\d+ tests?, 0 benchmarks', line)
                for line in value.splitlines() if line), 'unexpected test inventory output')
    return sorted(names)


def executed(output, names, filtered=0):
    require(names and len(names) == len(set(names)), 'nonempty distinct executed census')
    require(type(filtered) is int and filtered >= 0, 'known unrelated filtered census')
    value = output.replace('\r\n', '\n')
    summaries = re.findall(r'^test result: ok\. \d+ passed; 0 failed; 0 ignored; 0 measured; (\d+) filtered out; finished in [0-9]+(?:\.[0-9]+)?s$', value, re.M)
    require(summaries == [str(filtered)], 'exact filtered census and successful summary')
    # The shared strict record parser requires zero filtering; admit only the
    # already checked count of unrelated library tests before parsing records.
    value = re.sub(r'(?m)^(test result: ok\..*?; )\d+( filtered out;)', r'\g<1>0\2', value)
    runner.result(value, names)


def run_tests(label, binary, names, prefix=None, ignored=False):
    all_names = inventory(run(label + '-inventory', [binary, '--list']))
    ignored_names = inventory(run(label + '-ignored-inventory', [binary, '--ignored', '--list']))
    selected = [name for name in all_names if prefix is None or name.startswith(prefix)]
    require(selected == sorted(names), 'exact selected test inventory')
    require([name for name in ignored_names if prefix is None or name.startswith(prefix)]
            == (sorted(names) if ignored else []), 'exact ignored test inventory')
    args = [binary]
    if prefix is not None:
        args.append(prefix)
    if ignored:
        require(len(names) == 1 and prefix is None, 'exact single ignored GPU test')
        args += ['--ignored', '--exact', names[0]]
    args += ['--format=pretty', '--color=never', '--test-threads=1']
    executed(run(label + '-run', args), names, len(all_names) - len(names))


def cpu():
    system = platform.system()
    require(system in FEATURES, 'Linux or Windows CPU required')
    if system == 'Linux':
        require(platform.machine() == 'x86_64', 'Linux x86_64 required')
    features, closure = FEATURES[system]
    flags = ['--locked', '-p', 'orr_editor', '--features', features]
    output = run('mirror-cpu-build', ['cargo', 'test', *flags, '--lib', '--no-run', '--message-format=json'])
    binary = pin(output, 'mirror-cpu-producer', 'orr_editor', 'lib', closure)
    run_tests('mirror-cpu', binary, CPU_TESTS, prefix=CPU_PREFIX)
    # Existing counts, filesystem/socket rejection and feature boundaries stay
    # mandatory. Their original failure is retained; there is no skip fallback.
    run('retained-rotation-cpu', [sys.executable, '-B', str(source_root / 'tools/check-room-wall-group-rotation.py'),
                                '--evidence', str(runner.evidence / 'rotation-regression')], seconds=2400)
    runner.report['coverage']['cpu'] = 'passed'
    runner.record()


def configure_software():
    require(platform.system() == 'Linux' and platform.machine() == 'x86_64', 'Linux x86_64 GPU required')
    selected = [os.environ[key] for key in ('VK_DRIVER_FILES', 'VK_ICD_FILENAMES') if key in os.environ]
    if selected:
        require(all(value == selected[0] for value in selected), 'conflicting explicit Vulkan manifests')
        require(selected[0] and os.pathsep not in selected[0], 'one explicit Vulkan manifest required')
        path = Path(selected[0])
        require(path.is_absolute(), 'absolute configured Vulkan manifest')
    else:
        matches = sorted(Path('/usr/share/vulkan/icd.d').glob('lvp_icd*.json'))
        require(len(matches) == 1, 'set VK_DRIVER_FILES to one actual Lavapipe ICD manifest')
        path = matches[0]
    regular(path, 1024**2)
    manifest = strict_json(path)
    library = manifest.get('ICD', {}).get('library_path')
    require(isinstance(library, str) and ('lvp' in library.lower() or 'lavapipe' in library.lower()),
            'configured ICD must name Lavapipe')
    runtime = runner.evidence / 'xdg-runtime'
    runtime.mkdir(mode=0o700)
    os.environ.update(VK_DRIVER_FILES=str(path), VK_ICD_FILENAMES=str(path),
                      WGPU_BACKEND='vulkan', ORR_REQUIRE_GPU='1', XDG_RUNTIME_DIR=str(runtime))
    runner.report['gpu_environment'] = {key: os.environ[key] for key in (
        'VK_DRIVER_FILES', 'VK_ICD_FILENAMES', 'WGPU_BACKEND', 'ORR_REQUIRE_GPU', 'XDG_RUNTIME_DIR')}
    runner.report['icd_sha256'] = runner.digest(path)
    runner.record()
    return path


def validate_route(route, blocked, require_outcome=True):
    require(isinstance(route, dict), 'route object required')
    if require_outcome:
        require(type(route.get('blocked')) is bool and route['blocked'] is blocked,
                'route outcome identity')
    for key in ('start', 'end'):
        vector = route.get(key)
        require(isinstance(vector, list) and len(vector) == 3
                and all(type(value) in (int, float) and math.isfinite(value) for value in vector),
                'finite actual gameplay coordinates')
    start, end = route['start'], route['end']
    if blocked:
        require(start[0] + .05 < end[0] < 1.7, 'mirrored wall must block actual movement')
    else:
        require(end[0] > 2.5, 'negative control must cross barrier position')


def validate_receipt(receipt, expected_source_sha256=None):
    expected = {'schema': 1, 'platform': 'linux-x86_64', 'physical_os_input': False,
                'windows_hardware_ui_tested': False}
    expected.update(dict.fromkeys(TRUE_FIELDS, True))
    require(isinstance(receipt, dict), 'receipt object required')
    for key, value in expected.items():
        require(type(receipt.get(key)) is type(value) and receipt.get(key) == value,
                'missing or invalid actual EditorApp evidence: ' + key)
    require(isinstance(receipt.get('adapter'), str) and 'llvmpipe' in receipt['adapter'].lower(),
            'actual software Vulkan llvmpipe adapter required')
    require(re.fullmatch(r'0x[0-9a-f]{16}', str(receipt.get('scene_checksum', ''))), 'final scene checksum')
    source_hash = receipt.get('test_source_sha256')
    require(isinstance(source_hash, str) and re.fullmatch('[0-9a-f]{64}', source_hash),
            'compiled UI test source hash')
    if expected_source_sha256 is None:
        expected_source_sha256 = runner.digest(ROOT / 'crates/orr_editor/tests/room_wall_stamp_mirror_ui.rs')
    require(source_hash == expected_source_sha256, 'UI producer compiled a different source revision')
    controls = receipt.get('gameplay_controls')
    identities = [(False, 90), (True, 0), (True, 180), (True, 270), (True, 90)]
    require(isinstance(controls, list) and len(controls) == len(identities), 'all mirror and order negative controls')
    for route, (mirror, degrees) in zip(controls, identities):
        require(type(route.get('mirror_x')) is bool and route['mirror_x'] is mirror
                and type(route.get('degrees')) is int and route['degrees'] == degrees, 'route placement identity')
        validate_route(route, mirror and degrees == 90)
        require(type(route.get('wrong_axis_and_wrong_order_control')) is bool
                and route['wrong_axis_and_wrong_order_control'] == (mirror and degrees == 270)
                and type(route.get('unchecked_compatibility_control')) is bool
                and route['unchecked_compatibility_control'] == (not mirror), 'negative-control purpose')
    require(all(route['start'][::2] == controls[0]['start'][::2] for route in controls), 'identical negative-control X/Z starting positions')
    reopened = receipt.get('reopened_gameplay')
    validate_route(reopened, True, require_outcome=False)
    require(reopened['start'][::2] == controls[0]['start'][::2], 'fresh reopened route X/Z start')
    require(receipt.get('wrong_axis_and_order_equivalence') ==
            'R90 Mz = Mx R90 = R270 Mx about the same pivot, offset last', 'mirror order negative-control identity')
    require(receipt.get('mirrored_centers') == [
        [['2', '6'], ['0', '7.5'], ['3', '9']],
        [['2', '0'], ['0.5', '-2'], ['-1', '1']],
        [['8', '0'], ['10', '-1.5'], ['7', '-3']],
        [['8', '6'], ['9.5', '8'], ['11', '5']]], 'independent literal mirrored center oracle')
    require(receipt.get('unchecked_90_centers') == [['2', '6'], ['0.5', '8'], ['-1', '5']],
            'unchecked compatibility oracle')
    require(receipt.get('renderer_options') == 'native egui_wgpu::RendererOptions::default',
            'production-compatible native texture renderer')
    require(receipt.get('pixel_source') ==
            'full composed EditorApp framebuffer; exact interior viewport ROI 890x797',
            'composed EditorApp visual proof')


def validate_captures(directory, receipt):
    require(directory.is_dir() and not directory.is_symlink(), 'fresh capture directory identity')
    captures = receipt.get('captures')
    require(isinstance(captures, list) and [item.get('name') for item in captures] == list(CAPTURES),
            'exact ordered capture receipt')
    require({path.name for path in directory.iterdir()} == set(CAPTURES) | {'acceptance.json', 'Mirrored Room fixture', 'source-hidden.room-stamp.json'},
            'exact capture directory inventory')
    decoded = {}
    for item in captures:
        name = item['name']
        path = regular(directory / name, 16 * 1024**2)
        require(type(item.get('bytes')) is int and item['bytes'] == path.stat().st_size
                and item.get('sha256') == runner.digest(path), 'capture byte/hash binding')
        data = path.read_bytes()
        require(len(data) >= 33, 'complete PNG header')
        width, height = struct.unpack('>II', data[16:24])
        require(0 < width <= 4096 and 0 < height <= 4096, 'bounded PNG dimensions')
        decoded[name] = (width, height, png_pixels(path, width, height))
    for name in ('01-source-viewport.png', '04-mirrored-saved-viewport.png',
                 '05-fresh-source-hidden-viewport.png'):
        require(decoded[name][:2] == (890, 797), 'exact composed viewport ROI dimensions')
    require(decoded['04-mirrored-saved-viewport.png'] == decoded['05-fresh-source-hidden-viewport.png'],
            'fresh source-hidden reopen must preserve actual viewport pixels')
    before = decoded['01-source-viewport.png'][2]
    after = decoded['04-mirrored-saved-viewport.png'][2]
    changed = sum(before[at:at + 4] != after[at:at + 4] for at in range(0, len(before), 4))
    require(type(receipt.get('changed_viewport_pixels')) is int
            and receipt['changed_viewport_pixels'] == changed and changed > 1000,
            'substantial independently counted displayed geometry change')
    for name in ('01-source-viewport.png', '04-mirrored-saved-viewport.png',
                 '05-fresh-source-hidden-viewport.png'):
        pixels = decoded[name][2]
        require(sum(pixels[at:at + 4] != pixels[:4] for at in range(0, len(pixels), 4)) > 1000,
                'nonvacuous rendered viewport detail')
    validate_regions(decoded, receipt.get('viewport_regions'))
    validate_retained_files(directory, receipt.get('retained_files'))
    return decoded


def validate_regions(decoded, regions):
    expected = ['01-source-viewport.png', '04-mirrored-saved-viewport.png',
                '05-fresh-source-hidden-viewport.png']
    require(isinstance(regions, list) and [item.get('name') for item in regions] == expected,
            'exact composed framebuffer region inventory')
    for region in regions:
        require(set(region) == {'name', 'composed_name', 'x', 'y', 'width', 'height',
                                'viewport_rect', 'framebuffer_width', 'framebuffer_height', 'panel_widths'},
                'exact viewport region fields')
        name = region['name']
        composed = name.replace('-viewport.png', '-composed.png')
        require(region['composed_name'] == composed, 'matching composed framebuffer')
        require(all(type(region[key]) is int for key in ('x', 'y', 'width', 'height')),
                'integer viewport region coordinates')
        x, y, width, height = (region[key] for key in ('x', 'y', 'width', 'height'))
        full_width, full_height, full = decoded[composed]
        require(type(region['framebuffer_width']) is int and type(region['framebuffer_height']) is int
                and (region['framebuffer_width'], region['framebuffer_height']) == (full_width, full_height) == (1730, 1009),
                'exact composed framebuffer geometry')
        require(region['panel_widths'] == {'hierarchy': 230, 'inspector': 600},
                'exact persisted application panel widths')
        rect = region['viewport_rect']
        require(isinstance(rect, list) and len(rect) == 4
                and all(type(value) in (int, float) and math.isfinite(value) for value in rect)
                and 0 <= rect[0] < rect[2] <= full_width and 0 <= rect[1] < rect[3] <= full_height,
                'finite logical viewport rectangle')
        require((math.floor(rect[2] - rect[0] + .5), math.floor(rect[3] - rect[1] + .5)) == (900, 850)
                and (x, y) == (math.floor(rect[0] + .5) + 5, math.floor(rect[1] + .5) + 48),
                'viewport region matches actual logical rectangle')
        require(all(region[key] == regions[0][key] for key in ('x', 'y', 'width', 'height', 'viewport_rect',
                    'framebuffer_width', 'framebuffer_height', 'panel_widths')),
                'identical viewport and composed layout across source/save/reopen')
        require((width, height) == decoded[name][:2] == (890, 797)
                and x >= 0 and y >= 0 and x + width <= full_width and y + height <= full_height,
                'viewport region within composed framebuffer')
        roi = b''.join(full[((y + row) * full_width + x) * 4:
                           ((y + row) * full_width + x + width) * 4] for row in range(height))
        require(roi == decoded[name][2], 'viewport pixels must come from actual composed framebuffer')


def validate_retained_files(directory, items):
    require(isinstance(items, list) and items, 'retained fixture/source file inventory')
    root = directory / 'Mirrored Room fixture'
    require(root.is_dir() and not root.is_symlink(), 'actual project fixture directory')
    actual = ['source-hidden.room-stamp.json']
    for at, folders, files in os.walk(root, followlinks=False):
        for name in folders:
            require(not (Path(at) / name).is_symlink(), 'retained directory symlink')
        for name in files:
            path = regular(Path(at) / name)
            actual.append(path.relative_to(directory).as_posix())
    require([item.get('name') for item in items] == sorted(actual), 'exact retained file census')
    required = {'source-hidden.room-stamp.json', 'Mirrored Room fixture/orr.project.json',
                'Mirrored Room fixture/room.scene.yaml', 'Mirrored Room fixture/room.models.json',
                'Mirrored Room fixture/room.camera.json'}
    require(required <= set(actual), 'retained ordinary project and source stamp')
    for item in items:
        require(set(item) == {'name', 'bytes', 'sha256'}, 'retained file metadata fields')
        path = regular(directory / item['name'])
        require(type(item.get('bytes')) is int and item['bytes'] == path.stat().st_size
                and item.get('sha256') == runner.digest(path), 'retained fixture/source bytes changed')


def gpu():
    configure_software()
    capture = runner.evidence / 'linux-ui'
    require(not capture.exists(), 'fresh absent GPU capture directory')
    os.environ['ORR_ROOM_WALL_MIRROR_LINUX_DIR'] = str(capture)
    features, closure = FEATURES['Linux']
    flags = ['--locked', '-p', 'orr_editor', '--features', features, '--test', GPU_TARGET]
    run('mirror-ui-lint', ['cargo', 'clippy', *flags, '--', '-D', 'warnings'])
    output = run('mirror-ui-build', ['cargo', 'test', *flags, '--no-run', '--message-format=json'])
    binary = pin(output, 'mirror-ui-producer', GPU_TARGET, 'test', closure)
    run_tests('mirror-ui', binary, [GPU_TEST], ignored=True)
    receipt = strict_json(capture / 'acceptance.json')
    validate_receipt(receipt, frozen['crates/orr_editor/tests/room_wall_stamp_mirror_ui.rs']['sha256'])
    validate_captures(capture, receipt)
    runner.report['coverage']['linux_software_vulkan_ui'] = 'passed'
    runner.report['ui_receipt_sha256'] = runner.digest(capture / 'acceptance.json')
    runner.record()


def finish(error):
    runner.report['status'] = 'failed' if error else 'passed'
    if error:
        runner.report['failure'] = ''.join(traceback.format_exception(error))
    try:
        source_fence()
        runner.fence()
        require(frozen is not None, 'source admission did not finish')
        require(run('clean-source-after', ['git', 'status', '--porcelain=v1', '--untracked-files=all']).strip() == '',
                'checkout dirty after acceptance')
        for key, revision in (('head', 'HEAD'), ('tree', 'HEAD^{tree}')):
            require(run('source-' + key + '-after', ['git', 'rev-parse', revision]).strip() == runner.report[key],
                    'checkout source authority changed')
        tracked = run('tracked-source-after', ['git', 'ls-files', '-z']).split('\0')
        require(tracked.pop() == '', 'final Git source inventory')
        check_inventory(tracked, 'after')
        require(sorted(tracked) == list(frozen), 'tracked paths changed')
        runner.report['source_unchanged'] = True
    except BaseException as secondary:
        runner.report['status'] = 'failed'
        runner.report['final_fence_error'] = repr(secondary)
        if error is None:
            error = secondary
    runner.record()
    if error is not None:
        raise SystemExit(error.code if isinstance(error, runner.GateFailure) else 1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('cpu', 'gpu'))
    parser.add_argument('--evidence', required=True, type=Path)
    args = parser.parse_args()
    initialize(ROOT, args.evidence, args.mode)
    error = None
    try:
        begin()
        {'cpu': cpu, 'gpu': gpu}[args.mode]()
    except BaseException as failure:
        error = failure
    finish(error)


if __name__ == '__main__':
    main()
