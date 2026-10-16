#!/usr/bin/env python3
"""Fail-closed private Room wall authoring acceptance, standard library only.

The cpu, gpu and export routes independently build and retain current-source
producers. No historical artifact, ignored inventory or semantic Python fixture
is execution credit. The reviewed static-prop runner owns bounded output and
process-tree reaping; it and its independent fixtures are inside every source
fence. Missing or changed workflow wiring intentionally blocks hosted acceptance.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import stat
import struct
import sys
import traceback


def python_runtime():
    return {'optimize': sys.flags.optimize, '__debug__': __debug__,
            'PYTHONOPTIMIZE': os.environ.get('PYTHONOPTIMIZE'),
            'dont_write_bytecode': sys.dont_write_bytecode,
            'ignore_environment': sys.flags.ignore_environment,
            'isolated': sys.flags.isolated,
            'PYTHONDONTWRITEBYTECODE': os.environ.get('PYTHONDONTWRITEBYTECODE'),
            'executable': sys.executable, 'version': sys.version}


def require_python():
    """Run before importing assertion-based infrastructure or touching evidence."""
    value = python_runtime()
    if (value['optimize'] != 0 or value['__debug__'] is not True
            or value['PYTHONOPTIMIZE'] is not None
            or value['dont_write_bytecode'] is not True
            or value['PYTHONDONTWRITEBYTECODE'] != '1'):
        print('ROOM_WALL_GATE_PYTHON_REJECTED ' + json.dumps(value, sort_keys=True),
              file=sys.stderr, flush=True)
        raise SystemExit(2)
    return value


require_python()
import static_model_extract_gate as runner
from static_model_uv_gate import png_pixels

FEATURE_PATHS = (
    'crates/orr_remote/examples/room_wall.rs',
    'crates/orr_remote/examples/room_wall/authoring.rs',
    'crates/orr_remote/tests/room_wall.rs',
    'crates/orr_sample/tests/room_wall_gpu.rs',
    'crates/orr_sample/tests/room_wall_export.rs',
    'docs/room-wall-authoring.md',
    'tools/room_wall_gate.py',
    'tools/test_room_wall_gate.py',
)
INFRA_PATHS = (
    'Cargo.toml', 'Cargo.lock', '.github/workflows/determinism.yml',
    'tools/static_model_extract_gate.py', 'tools/test_static_model_extract_gate.py',
    'tools/room_project_gate.py', 'tools/test_room_project_gate.py',
    'tools/room_template_gate.py', 'tools/test_room_template_gate.py',
    'tools/static_model_uv_gate.py', 'tools/test_static_model_uv_gate.py',
)
CPU_TESTS = tuple(sorted((
    'room_wall_cli_cardinality_and_token_bounds',
    'room_wall_fixed_point_envelope_boundaries',
    'room_wall_absent_guid_and_canonical_spawn',
    'room_wall_inclusive_64_wall_budget',
    'room_wall_publication_no_clobber_and_failure_preservation',
    'room_wall_platform_native_paths_and_unsafe_ancestry',
    'room_wall_erp_verified_adoption_undo_redo_save_reopen',
    'room_wall_live_document_stale_revalidation',
    'room_wall_collision_los_controls_and_restart',
    'room_wall_real_cli_ops_file_no_overwrite',
)))
CPU_IGNORED = ('room_wall_real_cli_ops_file_no_overwrite',)
GPU_TEST = 'room_wall_real_software_gpu_appearance_and_controls'
EXPORT_TEST = 'room_wall_real_cli_source_hidden_room_export'
PHASES = ('cpu', 'gpu', 'export')
SEMANTIC_FIXTURE_COUNT = 34
FIXTURE_COUNT = 35
WORKFLOW_JOB = '  room-wall-authoring:\n    # Each mandatory wall authoring route owns its runner, target and evidence.\n    name: room wall authoring (${{ matrix.mode }}, ${{ matrix.os }})\n    runs-on: ${{ matrix.os }}\n    timeout-minutes: 90\n    env:\n      PYTHONDONTWRITEBYTECODE: "1"\n    strategy:\n      fail-fast: false\n      matrix:\n        include:\n          - mode: cpu\n            os: ubuntu-22.04\n            timeout: 45\n          - mode: cpu\n            os: windows-latest\n            timeout: 45\n          - mode: gpu\n            os: ubuntu-22.04\n            timeout: 45\n          - mode: export\n            os: ubuntu-22.04\n            timeout: 60\n    steps:\n      - uses: actions/checkout@v4\n      - uses: dtolnay/rust-toolchain@stable\n      - uses: Swatinem/rust-cache@v2\n      - name: Install Room wall software Vulkan dependency\n        if: matrix.mode != \'cpu\'\n        run: sudo apt-get update -q && sudo apt-get install -y -q mesa-vulkan-drivers\n      - name: Install Room wall namespace acceptance dependency\n        if: matrix.mode == \'export\'\n        run: sudo apt-get update -q && sudo apt-get install -y -q bubblewrap\n      - name: Required independent Room wall authoring acceptance\n        shell: bash\n        timeout-minutes: ${{ matrix.timeout }}\n        env:\n          ORR_ROOM_WALL_GATE_EVIDENCE: ${{ matrix.mode == \'export\' && \'/tmp\' || runner.temp }}/orr-room-wall-${{ matrix.mode }}-${{ github.run_id }}-${{ github.run_attempt }}\n        run: |\n          unset PYTHONOPTIMIZE\n          python -B tools/room_wall_gate.py ${{ matrix.mode }}\n      - name: Retain Room wall and partial failure evidence\n        if: always()\n        uses: actions/upload-artifact@v4\n        with:\n          name: room-wall-${{ matrix.mode }}-${{ matrix.os }}\n          path: ${{ matrix.mode == \'export\' && \'/tmp\' || runner.temp }}/orr-room-wall-${{ matrix.mode }}-${{ github.run_id }}-${{ github.run_attempt }}\n          include-hidden-files: true\n          if-no-files-found: error\n          retention-days: 7\n\n'
WORKFLOW_AGGREGATE_STEP = '      - name: Require every independent Room wall authoring matrix row to pass\n        if: always()\n        shell: bash\n        env:\n          ROOM_WALL_RESULT: ${{ needs[\'room-wall-authoring\'].result }}\n        run: |\n          echo "ROOM_WALL_RESULT=$ROOM_WALL_RESULT"\n          if [[ "$ROOM_WALL_RESULT" != "success" ]]; then\n            echo "::error::ROOM_WALL_RESULT must be success, got \'$ROOM_WALL_RESULT\'"\n            exit 1\n          fi\n          echo "All four independent Room wall authoring routes passed."\n'
STATIC_AUTHORING_JOB_SHA256 = '9bde6e212d59b60f5edf76af5e9ac322ea599d00607b7be96bfb0657167979ab'
PRIOR_AGGREGATE_STEPS_SHA256 = '384293306b4deb4b46946e7c20bf7e00388b4409f111922668e00af3f5d01aac'
source_root = frozen = mode = None


def require(value, message='Room wall acceptance contract failed'):
    if not value:
        raise AssertionError(message)


def integer_fields(value, names):
    require(all(type(value[name]) is int for name in names), 'exact integer fields required')


def reject_boolean_numbers(value):
    """JSON bool must never compare equal to a numerical production field."""
    require(type(value) is not bool, 'boolean in numerical component data')
    if isinstance(value, dict):
        for child in value.values():
            reject_boolean_numbers(child)
    elif isinstance(value, list):
        for child in value:
            reject_boolean_numbers(child)


def inventory_numbers(value):
    for entry in value.values():
        require(isinstance(entry, list) and len(entry) == 2 and type(entry[0]) is int
                and entry[0] > 0, 'exact inventory byte count')


def source_paths(root):
    root = Path(root)
    paths = []
    for directory, folders, files in os.walk(root, followlinks=False):
        parent = Path(directory)
        if parent == root:
            folders[:] = [name for name in folders if name not in {'.git', 'target'}]
        for name in folders:
            require(not (parent / name).is_symlink(), 'symlink source directory')
        for name in files:
            path = parent / name
            require(path.is_file() and not path.is_symlink(), 'nonregular source file')
            paths.append(path.relative_to(root).as_posix())
    require(len(paths) == len(set(paths)), 'duplicate source paths')
    return sorted(paths)


def snapshot():
    return {name: {'sha256': runner.digest(source_root / name),
                   'bytes': (source_root / name).stat().st_size,
                   'mode': stat.S_IMODE((source_root / name).stat().st_mode)}
            for name in source_paths(source_root)}


def require_sources(value):
    required = set(FEATURE_PATHS + INFRA_PATHS)
    for package in ('orr_remote', 'orr_sample', 'orr_edit', 'orr_reflect', 'orr_games'):
        required.update(f'crates/{package}/{name}' for name in ('Cargo.toml', 'src/lib.rs'))
    require(len(FEATURE_PATHS) == len(set(FEATURE_PATHS)) == 8)
    require(required <= set(value), 'complete authoring/consumer/lifecycle source required')


def check_source_inventory(tracked, phase):
    observed = source_paths(source_root)
    expected = sorted(tracked)
    item = {'tracked': expected, 'observed': observed,
            'extra': sorted(set(observed) - set(expected)),
            'missing': sorted(set(expected) - set(observed)),
            'matches': expected == observed and len(set(tracked)) == len(tracked)}
    runner.report.setdefault('source_path_inventories', {})[phase] = item
    runner.record()
    require(item['matches'], 'untracked, ignored, duplicate or missing source')


def source_fence():
    if frozen is not None:
        current = snapshot()
        if list(current) != list(frozen):
            check_source_inventory(list(frozen), 'source-fence')
        require(current == frozen, 'source bytes, modes or paths changed')


def run(label, command, expected=0, seconds=1200):
    require_python()
    source_fence()
    output = runner.run(label, command, expected=expected, seconds=seconds)
    source_fence()
    return output


def initialize(root, destination, phase):
    global source_root, frozen, mode
    runtime = require_python()
    require(phase in PHASES)
    source_root, frozen, mode = Path(root).resolve(), None, phase
    destination = Path(destination)
    require(destination.is_absolute() and destination == destination.resolve(),
            'canonical absolute evidence path required')
    require(not destination.is_relative_to(source_root), 'evidence must be outside checkout')
    require(not source_root.is_relative_to(destination), 'evidence cannot contain checkout')
    require(not destination.exists(), 'exclusive fresh evidence required')
    try:
        runner.initialize(source_root, destination, phase)
        runner.report.update(python_runtime=runtime, mode=phase,
                             coverage={name: 'NOT RUN' for name in PHASES},
                             physical_gpu_claim=False, workflow_contract='PENDING')
        temporary = destination / 'temporary'
        temporary.mkdir(mode=0o700)
        os.environ.update(TMPDIR=str(temporary), TMP=str(temporary), TEMP=str(temporary))
        runner.report['private_temporary'] = str(temporary)
        runner.record()
    except BaseException as error:
        # A failure after exclusive evidence creation must not erase the first
        # failure or manufacture a successful setup receipt.
        if destination.is_dir() and runner.evidence == destination:
            runner.report.update(status='failed', initialization_error=repr(error))
            runner.record()
        raise


def fixture_result(output, count, marker):
    """The marker alone cannot launder skipped, repeated or failing fixtures."""
    require(type(count) is int and count > 0, 'fixture census is not frozen')
    require(re.findall(r'^Ran (\d+) tests in [0-9.]+s$', output, re.M) == [str(count)],
            'single exact Python fixture total')
    require(len(re.findall(r'^test_\w+ \(.+\) \.\.\. ok$', output, re.M)) == count,
            'exact all-pass Python fixture records')
    require(not re.search(r'(?im)(?:\.\.\. (?:skipped|FAIL|ERROR|expected failure|unexpected success)|^FAILED|^ERROR:|^FAIL:)', output),
            'fixture failed or skipped')
    require(output.rstrip().endswith(marker), 'fixture completion marker')


def workflow_job(text, name):
    headers = list(re.finditer(r'^  ([A-Za-z0-9_-]+):\n', text, re.M))
    selected = [index for index, item in enumerate(headers) if item[1] == name]
    require(len(selected) == 1, 'one exact workflow job: ' + name)
    index = selected[0]
    end = headers[index + 1].start() if index + 1 < len(headers) else len(text)
    return text[headers[index].start():end]


def workflow_contract(text):
    require(WORKFLOW_JOB is not None and WORKFLOW_AGGREGATE_STEP is not None,
            'Room wall workflow wiring is pending integration review')
    # These literals are integration-owned review results, never learned from
    # whatever workflow happens to be checked out or from receipt assertions.
    job = workflow_job(text, 'room-wall-authoring')
    require(job == WORKFLOW_JOB, 'exact mandatory four-row Room wall matrix')
    aggregate = workflow_job(text, 'cross_platform_checksum_compare')
    require('needs: [native, wasm32, wasm32-browser, android-arm64, sample-build, sample-linked-prefabs, sample-room, static-model-authoring, room-wall-authoring]' in aggregate,
            'required Room wall aggregate dependency')
    require(aggregate.endswith(WORKFLOW_AGGREGATE_STEP), 'fail-closed Room wall aggregate result')
    require(text.count(WORKFLOW_AGGREGATE_STEP) == 1, 'one Room wall aggregate result owner')
    require(hashlib.sha256(workflow_job(text, 'static-model-authoring').encode()).hexdigest() == STATIC_AUTHORING_JOB_SHA256,
            'existing eight-row static authoring job must survive unchanged')
    previous = aggregate[:-len(WORKFLOW_AGGREGATE_STEP)].split('    steps:\n', 1)[1]
    require(hashlib.sha256(previous.encode()).hexdigest() == PRIOR_AGGREGATE_STEPS_SHA256,
            'prior aggregate result checks must survive byte-identically')


def begin():
    global frozen
    require_python()
    require(run('clean-source-before', ['git', 'status', '--porcelain=v1', '--untracked-files=all']).strip() == '',
            'clean committed source required')
    for key, revision in (('head', 'HEAD'), ('tree', 'HEAD^{tree}')):
        value = run('checkout-' + key, ['git', 'rev-parse', revision]).strip()
        require(re.fullmatch('[0-9a-f]{40}', value) is not None, 'full Git source identity')
        runner.report[key] = value
    runner.report['github_sha'] = os.environ.get('GITHUB_SHA')
    if runner.report['github_sha'] is not None:
        require(runner.report['github_sha'] == runner.report['head'], 'hosted checkout SHA mismatch')
    tracked = run('tracked-source', ['git', 'ls-files', '-z']).split('\0')
    require(tracked.pop() == '' and bool(tracked), 'NUL-delimited complete Git inventory')
    check_source_inventory(tracked, 'before')
    frozen = snapshot()
    require_sources(frozen)
    runner.report.update(checkout=str(source_root), source_sha256=frozen,
                         source_inventory_sha256=hashlib.sha256(json.dumps(frozen, sort_keys=True).encode()).hexdigest(),
                         feature_paths=list(FEATURE_PATHS))
    runner.record()
    workflow_contract((source_root / '.github/workflows/determinism.yml').read_text())
    runner.report['workflow_contract'] = 'PASSED'
    runner.record()
    for name, count, marker in (
        ('test_static_model_extract_gate.py', 29, 'STATIC_PROP_GATE_FIXTURES: 29 passed; 0 failed; 0 skipped'),
        ('test_room_wall_gate.py', FIXTURE_COUNT,
         f'ROOM_WALL_GATE_FIXTURES: {FIXTURE_COUNT} passed; 0 failed; 0 skipped'),
    ):
        output = run('checker-' + name[:-3], [sys.executable, '-B', str(source_root / 'tools' / name)])
        fixture_result(output, count, marker)


def pin(output, label, package, name, kind, features):
    source_fence()
    binary = runner.pin(output, label, package, name, kind, features)
    runner.report['producers'][label].update(
        source_inventory_sha256=runner.report['source_inventory_sha256'],
        source_root=str(source_root), source_head=runner.report['head'],
        source_tree=runner.report['tree'])
    runner.record()
    return binary


def test(label, binary, names, ignored=()):
    names, ignored = tuple(names), tuple(ignored)
    require(set(ignored) <= set(names) and len(set(names)) == len(names), 'test census')
    runner.inventory(run(label + '-inventory', [binary, '--list']), names)
    runner.inventory(run(label + '-ignored-inventory', [binary, '--ignored', '--list']), ignored)
    flags = ['--include-ignored'] if ignored else []
    runner.result(run(label + '-run', [binary, *flags, '--test-threads=1']), names)


def cargo_flags(package):
    return ['--locked', '--release', '-p', package, '--no-default-features']


def build_cli():
    flags = cargo_flags('orr_remote')
    output = run('build-room-wall-cli', ['cargo', 'build', *flags, '--features', 'room-project',
                                       '--example', 'room_wall', '--message-format=json'])
    binary = pin(output, 'room-wall-cli', 'orr_remote', 'room_wall', 'example',
                 ['room-project', 'sample-host'])
    os.environ['ORR_ROOM_WALL_BIN'] = binary
    return binary


def strict_json(path, cap=8 * 1024**2):
    path = Path(path)
    require(path.is_file() and not path.is_symlink() and 0 < path.stat().st_size <= cap,
            'bounded regular JSON file')
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, 'duplicate JSON object field')
            result[key] = value
        return result
    def invalid(value):
        raise AssertionError('nonfinite JSON number: ' + value)
    value = json.loads(path.read_text(encoding='utf-8'), object_pairs_hook=pairs, parse_constant=invalid)
    def finite(node, depth=0):
        require(depth <= 64, 'bounded JSON depth')
        if isinstance(node, dict):
            for child in node.values():
                finite(child, depth + 1)
        elif isinstance(node, list):
            for child in node:
                finite(child, depth + 1)
        elif isinstance(node, float):
            require(math.isfinite(node), 'nonfinite JSON exponent')
    finite(value)
    return value


def capture_files(root, expected):
    root = Path(root)
    require(root.is_dir() and not root.is_symlink(), 'regular capture root')
    observed, total, directories = {}, 0, set()
    for directory, folders, files in os.walk(root, followlinks=False):
        for name in folders:
            path = Path(directory) / name
            require(not path.is_symlink(), 'symlink capture directory')
            directories.add(path.relative_to(root).as_posix())
        for name in files:
            path = Path(directory) / name
            require(path.is_file() and not path.is_symlink(), 'nonregular capture')
            size = path.stat().st_size
            require(size <= 128 * 1024**2, 'individual capture budget')
            total += size
            require(total <= 512 * 1024**2 and len(observed) < 256, 'total capture budget')
            observed[path.relative_to(root).as_posix()] = [size, runner.digest(path)]
    require(set(observed) == set(expected), 'exact retained capture inventory')
    expected_directories = {parent.as_posix() for name in expected for parent in Path(name).parents if parent != Path('.')}
    require(directories == expected_directories, 'exact capture directory inventory')
    return dict(sorted(observed.items()))


def file_reference(root, entry, expected):
    require(isinstance(entry, dict) and set(entry) == {'file', 'bytes', 'sha256'}, 'file reference schema')
    require(entry['file'] == expected, 'exact retained file path')
    path = Path(root) / expected
    require(path.is_file() and not path.is_symlink(), 'retained reference regular file')
    require(type(entry['bytes']) is int and entry['bytes'] == path.stat().st_size,
            'retained reference byte count')
    require(entry['sha256'] == runner.digest(path), 'retained reference SHA256')


def source_provenance(receipt, test_path, producer_names):
    expected_sources = {'test': test_path, 'cli': FEATURE_PATHS[0], 'authoring': FEATURE_PATHS[1]}
    require(set(receipt['source_sha256']) == set(expected_sources), 'exact source provenance keys')
    for key, name in expected_sources.items():
        require(receipt['source_sha256'][key] == runner.digest(source_root / name), 'current source provenance')
    require(set(receipt['producer_sha256']) == set(producer_names), 'exact producer provenance keys')
    for name in producer_names:
        require(receipt['producer_sha256'][name] == runner.report['producers'][name]['sha256'],
                'fresh retained producer provenance')


def check_coverage():
    require(runner.report['coverage'] == {name: ('PASSED' if name == mode else 'NOT RUN')
                                         for name in PHASES}, 'exact mode coverage')
    require(runner.report['workflow_contract'] == 'PASSED', 'mandatory workflow contract')


def finish(error):
    try:
        require(frozen is not None, 'prebuild source snapshot missing')
        source_fence()
        runner.fence()
        require(run('clean-source-after', ['git', 'status', '--porcelain=v1', '--untracked-files=all']).strip() == '',
                'source worktree changed')
        for key, revision in (('head', 'HEAD'), ('tree', 'HEAD^{tree}')):
            require(run('checkout-' + key + '-after', ['git', 'rev-parse', revision]).strip() == runner.report[key],
                    'Git source identity changed')
        tracked = run('tracked-source-after', ['git', 'ls-files', '-z']).split('\0')
        require(tracked.pop() == '')
        check_source_inventory(tracked, 'after')
        require(sorted(tracked) == list(frozen), 'source path identity changed')
        runner.report['source_unchanged'] = True
        if error is None:
            check_coverage()
    except BaseException as final_error:
        runner.report['final_fence_error'] = repr(final_error)
        error = error or final_error
    runner.report['status'] = 'failed' if error is not None else 'passed'
    if error is not None:
        runner.report['failure'] = ''.join(traceback.format_exception(error))
    runner.record()
    if error is not None:
        raise SystemExit(error.code if isinstance(error, runner.GateFailure) else 1)


def fnv1a64(data):
    value = 0xcbf29ce484222325
    for byte in data:
        value = ((value ^ byte) * 0x100000001b3) & 0xffffffffffffffff
    return f'{value:016x}'


def fnv_reference(root, entry, expected):
    require(isinstance(entry, dict) and set(entry) == {'file', 'bytes', 'fnv1a64'}, 'FNV reference schema')
    require(entry['file'] == expected, 'exact capture file reference')
    path = Path(root) / expected
    require(path.is_file() and not path.is_symlink(), 'regular FNV reference')
    require(type(entry['bytes']) is int and entry['bytes'] == path.stat().st_size)
    require(entry['fnv1a64'] == fnv1a64(path.read_bytes()), 'capture FNV byte identity')


def fnv_provenance(receipt, test_path, producer_names):
    expected = {'test': test_path, 'cli': FEATURE_PATHS[0], 'authoring': FEATURE_PATHS[1]}
    require(set(receipt['source_fnv1a64']) == set(expected), 'source FNV provenance keys')
    for key, name in expected.items():
        require(receipt['source_fnv1a64'][key] == fnv1a64((source_root / name).read_bytes()),
                'current source FNV provenance')
    require(set(receipt['producer_fnv1a64']) == set(producer_names), 'producer FNV provenance keys')
    for name in producer_names:
        item = runner.report['producers'][name]
        path = runner.evidence / item['file']
        require(runner.digest(path) == item['sha256'], 'retained SHA256 producer identity')
        require(receipt['producer_fnv1a64'][name] == fnv1a64(path.read_bytes()),
                'fresh retained FNV producer identity')


def difference(first, second, tolerance=3):
    require(len(first) == len(second) and len(first) % 4 == 0, 'equal RGBA frame lengths')
    return sum(any(abs(first[offset + channel] - second[offset + channel]) > tolerance
                   for channel in range(4)) for offset in range(0, len(first), 4))


GPU_FRAMES = ('baseline', 'authored', 'wrong-position', 'models-off')
GPU_WIDTH = GPU_HEIGHT = 512
WALL_RECT = (384, 80, 448, 176)


def room_wall_pixel_oracle(frames):
    """Independently project the specified top-down Room wall, not receipt geometry.

    Eye=[0,20,0], up=[0,0,-1], orthographic half-height=4. The 512px
    viewport gives 64px/world-unit. Center=(2.5,-2), half=(.5,.75).
    Production Room shadows are disabled for this isolated shape assertion.
    """
    require(set(frames) == set(GPU_FRAMES), 'exact GPU frame names')
    size = GPU_WIDTH * GPU_HEIGHT * 4
    require(all(len(value) == size and value[3::4] == b'\xff' * (size // 4)
                for value in frames.values()), 'opaque bounded GPU frames')
    baseline, authored = frames['baseline'], frames['authored']
    wrong, absent = frames['wrong-position'], frames['models-off']
    left, top, right, bottom = WALL_RECT
    interior = exterior = model = visible_model = 0
    for y in range(GPU_HEIGHT):
        for x in range(GPU_WIDTH):
            offset = (y * GPU_WIDTH + x) * 4
            before = baseline[offset:offset + 4]
            after = authored[offset:offset + 4]
            if left + 2 <= x < right - 2 and top + 2 <= y < bottom - 2:
                require(any(abs(a - b) > 3 for a, b in zip(before, after)),
                        'numeric wall interior missing')
                require(wrong[offset:offset + 4] == before, 'wrong-position wall occupies authored interior')
                interior += 1
            elif not (left - 2 <= x < right + 2 and top - 2 <= y < bottom + 2):
                require(after == before, 'numeric wall exterior changed')
                exterior += 1
                missing = absent[offset:offset + 4]
                if before != missing:
                    require(after == before, 'existing model pixels changed')
                    model += 1
                    visible_model += int(any(abs(a - b) > 3 for a, b in zip(before, missing)))
    require(interior == 60 * 92 and exterior == 512 * 512 - 68 * 100,
            'independent exact oracle coverage')
    require(model > 20 and visible_model > 20, 'nonvacuous imported model preservation control')
    measured = {name: difference(authored, frames[name])
                for name in ('baseline', 'wrong-position', 'models-off')}
    require(measured['baseline'] >= interior and measured['wrong-position'] >= interior,
            'nonvacuous authored wall controls')
    require(measured['models-off'] > 20, 'nonvacuous models-off control')
    return {'wall_interior_pixels': interior, 'wall_exterior_pixels': exterior,
            'preserved_model_pixels': model, 'visible_model_pixels_over_tolerance': visible_model,
            'control_pixel_differences': measured}


def read_rgba_ppm(captures, stem):
    raw = captures / (stem + '.rgba')
    ppm = captures / (stem + '.ppm')
    require(raw.stat().st_size == GPU_WIDTH * GPU_HEIGHT * 4, 'exact raw GPU frame size')
    pixels = raw.read_bytes()
    expected = b'P6\n512 512\n255\n' + bytes(value for index, value in enumerate(pixels) if index % 4 != 3)
    require(ppm.read_bytes() == expected, 'PPM must encode the retained raw RGBA')
    return pixels


def validate_ops(path, guid=None):
    operations = strict_json(path, cap=16 * 1024)
    require(isinstance(operations, list) and len(operations) == 1, 'exactly one ordinary operation')
    operation = operations[0]
    require(isinstance(operation, dict) and operation.get('op') == 'spawn', 'ordinary spawn operation')
    require(set(operation) <= {'op', 'guid', 'name', 'components'}, 'no private operation metadata')
    require(set(operation) == {'op', 'guid', 'name', 'components'}, 'explicit spawn identity and components')
    require(isinstance(operation['name'], str) and 0 < len(operation['name'].encode()) <= 128)
    if guid is not None:
        require(operation['guid'] == guid, 'exact authored stable GUID')
    require(isinstance(operation['components'], dict) and operation['components'], 'spawn component map')
    return operation


def check_feature_off(binary, label):
    root = runner.evidence / ('feature-off-' + label)
    root.mkdir(mode=0o700)
    project, output = root / 'unopened-project', root / 'unpublished.ops.json'
    diagnostic = 'room_wall requires --features room-project; no project or output was opened'
    for suffix, args in (('help', ['--help']), ('valid-options', [
            '--project', str(project), '--guid', 'e_abcdef0123456789', '--name', 'authored wall',
            '--center', '0.625', '0', '--half-extents', '0.125', '0.75', '--output', str(output)])):
        require(run('feature-off-' + label + '-' + suffix, [binary, *args], expected=1).strip() == diagnostic,
                'feature-off CLI must reject before project/output access')
        require(list(root.iterdir()) == [], 'feature-off CLI touched project/output paths')


def cpu():
    flags = cargo_flags('orr_remote')
    output = run('build-feature-off', ['cargo', 'build', *flags, '--example', 'room_wall', '--message-format=json'])
    disabled = pin(output, 'feature-off-cli', 'orr_remote', 'room_wall', 'example', [])
    check_feature_off(disabled, 'no-default')
    default_flags = ['--locked', '--release', '-p', 'orr_remote']
    output = run('build-default-feature-off', ['cargo', 'build', *default_flags,
                                              '--example', 'room_wall', '--message-format=json'])
    default = pin(output, 'default-feature-off-cli', 'orr_remote', 'room_wall', 'example', ['default', 'sample-host'])
    check_feature_off(default, 'default')
    cli = build_cli()
    require('usage: room_wall' in run('actual-room-wall-usage-exit1', [cli, '--help'], expected=1),
            'strict enabled CLI usage rejection')
    output = run('build-cpu-tests', ['cargo', 'test', *flags, '--features', 'room-project',
                                   '--test', 'room_wall', '--no-run', '--message-format=json'])
    binary = pin(output, 'cpu-test', 'orr_remote', 'room_wall', 'test', ['room-project', 'sample-host'])
    captures = runner.evidence / 'captures'
    captures.mkdir(mode=0o700)
    os.environ['ORR_ROOM_WALL_CPU_EVIDENCE'] = str(captures)
    test('cpu-acceptance', binary, CPU_TESTS, CPU_IGNORED)
    run('strict-room-wall-enabled', ['cargo', 'clippy', *flags, '--features', 'room-project',
                                    '--example', 'room_wall', '--test', 'room_wall', '--', '-D', 'warnings'])
    run('strict-room-wall-disabled', ['cargo', 'clippy', *flags, '--example', 'room_wall',
                                     '--test', 'room_wall', '--', '-D', 'warnings'])
    run('strict-room-wall-default-disabled', ['cargo', 'clippy', *default_flags,
                                             '--example', 'room_wall', '--test', 'room_wall', '--', '-D', 'warnings'])
    check_cpu(captures)
    runner.report['coverage']['cpu'] = 'PASSED'
    runner.record()


def gpu():
    require(sys.platform == 'linux', 'GPU acceptance requires hosted software Vulkan Linux')
    runner.software()
    build_cli()
    flags = cargo_flags('orr_sample')
    output = run('build-gpu-test', ['cargo', 'test', *flags, '--features', 'room-project',
                                  '--test', 'room_wall_gpu', '--no-run', '--message-format=json'])
    binary = pin(output, 'gpu-test', 'orr_sample', 'room_wall_gpu', 'test', ['project', 'room-project', 'sprites'])
    run('strict-room-wall-gpu', ['cargo', 'clippy', *flags, '--features', 'room-project',
                                '--test', 'room_wall_gpu', '--', '-D', 'warnings'])
    captures = runner.evidence / 'captures'
    require(not captures.exists(), 'fresh GPU captures')
    os.environ['ORR_ROOM_WALL_CAPTURE_DIR'] = str(captures)
    test('gpu-acceptance', binary, [GPU_TEST], [GPU_TEST])
    check_gpu(captures)
    runner.report['coverage']['gpu'] = 'PASSED'
    runner.record()


def export():
    require(sys.platform == 'linux' and os.uname().machine == 'x86_64' and os.getuid() != 0,
            'source-hidden export requires non-root Linux x86_64')
    require(not runner.evidence.is_relative_to(source_root.parent), 'evidence must survive workspace hiding')
    runner.software()
    run('namespace-preflight', ['bwrap', '--die-with-parent', '--ro-bind', '/', '/', '--', '/bin/true'])
    os.environ['ORR_REQUIRE_PROJECT_ISOLATION'] = '1'
    build_cli()
    flags = [*cargo_flags('orr_sample'), '--features', 'room-project,project-export']
    features = ['project', 'project-export', 'room-project', 'sprites']
    for name, variable in (('room_escape', 'ORR_ROOM_RUNTIME'), ('orr_export_room', 'ORR_ROOM_EXPORTER')):
        output = run('build-' + name, ['cargo', 'build', *flags, '--bin', name, '--message-format=json'])
        os.environ[variable] = pin(output, name, 'orr_sample', name, 'bin', features)
    output = run('build-export-test', ['cargo', 'test', *flags, '--test', 'room_wall_export',
                                     '--no-run', '--message-format=json'])
    binary = pin(output, 'export-test', 'orr_sample', 'room_wall_export', 'test', features)
    run('strict-room-wall-export', ['cargo', 'clippy', *flags, '--test', 'room_wall_export', '--', '-D', 'warnings'])
    captures = runner.evidence / 'captures'
    captures.mkdir(mode=0o700)
    os.environ['ORR_ROOM_WALL_EVIDENCE'] = str(captures)
    test('source-hidden-export', binary, [EXPORT_TEST], [EXPORT_TEST])
    check_export(captures)
    runner.report['coverage']['export'] = 'PASSED'
    runner.record()


PACKAGE_NAME = 'sample-imported-scene'
PACKAGE_DIGEST = '9c7708914c9e5fbe61dad5cc717dff6df0d811d0e14f3aa9a967896785522cf0'
PACKAGE_FILES = ('LICENSE.txt', 'background.glb', 'foreground.glb', 'generate.py')
PROJECT_FILES = ('orr.project.json', 'room.scene.yaml', 'room.models.json',
                 'room.camera.json', 'orr.packages.lock.json')
WALL_GUID = 'e_abcdef0123456789'


def project_files():
    prefix = f'.orr/packages/objects/{PACKAGE_DIGEST}/'
    return set(PROJECT_FILES) | {prefix + name for name in ('orr.package.json', *PACKAGE_FILES)}


def check_project(root):
    """Check production-admitted retained closure against current fixture assets."""
    root = Path(root)
    inventory = capture_files(root, project_files())
    manifest = strict_json(root / 'orr.project.json')
    integer_fields(manifest, ('schema',))
    require(manifest == {'schema': 2, 'engine': '*', 'entry': {
        'game': 'room-escape-v1', 'scene': 'room.scene.yaml',
        'models': 'room.models.json', 'camera': 'room.camera.json'}}, 'exact retained project manifest')
    lock = strict_json(root / 'orr.packages.lock.json')
    integer_fields(lock, ('schema',))
    require(set(lock) == {'schema', 'direct', 'packages'} and lock['schema'] == 1)
    require(lock['direct'] == {PACKAGE_NAME: '1.0.0'} and set(lock['packages']) == {PACKAGE_NAME})
    package = lock['packages'][PACKAGE_NAME]
    require(set(package) == {'manifest', 'digest', 'files'})
    manifest = package['manifest']
    integer_fields(manifest, ('schema',))
    canonical = {key: manifest[key] for key in ('schema', 'name', 'version', 'engine', 'capabilities', 'dependencies', 'files')}
    require(set(canonical) == set(manifest))
    require(manifest == {'schema': 1, 'name': PACKAGE_NAME, 'version': '1.0.0', 'engine': '^0.0.1',
                         'capabilities': ['models'], 'dependencies': {}, 'files': list(PACKAGE_FILES)})
    hashes = dict(sorted(package['files'].items()))
    require(set(hashes) == set(PACKAGE_FILES), 'exact package asset census')
    identity = hashlib.sha256(json.dumps(['orr-package-v1', canonical, hashes], separators=(',', ':'), ensure_ascii=False).encode()).hexdigest()
    require(identity == package['digest'] == PACKAGE_DIGEST, 'independent package identity')
    prefix = root / '.orr/packages/objects' / identity
    for name in ('orr.package.json', *PACKAGE_FILES):
        require((prefix / name).read_bytes() == (source_root / 'assets/imported_scene_demo' / name).read_bytes(),
                'existing model/source asset changed')
        if name != 'orr.package.json':
            require(hashes[name] == runner.digest(prefix / name), 'package asset hash')
    sidecar = strict_json(root / 'room.models.json')
    integer_fields(sidecar, ('version',))
    require(set(sidecar) == {'version', 'scene', 'project', 'bindings'})
    require(sidecar['version'] == 2 and sidecar['scene'] == 'room.scene.yaml' and sidecar['project'] == '.')
    require(len(sidecar['bindings']) == 2, 'two preserved model bindings')
    for guid, binding in sidecar['bindings'].items():
        require(re.fullmatch(r'e_[0-9a-f]{8,32}', guid) is not None and guid != WALL_GUID)
        require(set(binding) == {'kind', 'package', 'asset', 'package_digest', 'source_hash', 'transform'})
        require(binding['kind'] == 'static' and binding['package'] == PACKAGE_NAME
                and binding['package_digest'] == identity and binding['asset'] == 'foreground.glb'
                and binding['source_hash'] == hashes['foreground.glb'])
        require(set(binding['transform']) == {'translation', 'rotation', 'scale'})
        reject_boolean_numbers(binding['transform'])
        require(binding['transform']['rotation'] == [0, 0, 0, 1]
                and binding['transform']['scale'] == [1, 1, 1])
    return inventory, sidecar


def check_wall_operation(path, center, half):
    operation = validate_ops(path, WALL_GUID)
    components = operation['components']
    reject_boolean_numbers(components)
    require(set(components) == {'RoomEscapeV1::Actor', 'orr_physics3d::Body', 'orr_physics3d::Collider'},
            'canonical three wall components')
    integer_fields(components['RoomEscapeV1::Actor'], ('kind', 'ordinal'))
    require(components['RoomEscapeV1::Actor'] == {'kind': 4, 'ordinal': 0}, 'canonical wall identity')
    body = components['orr_physics3d::Body']
    require(set(body) == {'pos', 'rot', 'vel', 'omega', 'inv_mass', 'inv_inertia',
                          'linear_damping', 'angular_damping', 'kind'}, 'canonical complete Body')
    require(body['pos'] == [center[0], .5, center[1]] and body['kind'] == 'static')
    require(body['rot'] == {'kind': 'unit', 'x': 0, 'y': 0, 'z': 0, 'w': 1}, 'identity wall rotation')
    require(all(body[name] == [0, 0, 0] for name in ('vel', 'omega', 'inv_inertia')))
    require(all(body[name] == 0 for name in ('inv_mass', 'linear_damping', 'angular_damping')))
    collider = components['orr_physics3d::Collider']
    integer_fields(collider, ('layer', 'mask'))
    require(collider == {'shape': {'kind': 'box', 'half_extents': [half[0], 1, half[1]]},
                         'restitution': 0, 'friction': .5, 'layer': 1, 'mask': 0}, 'canonical box wall collider')
    return operation


def compare_projects(before, after, changes):
    first, before_models = check_project(before)
    second, after_models = check_project(after)
    require(set(first) == set(second))
    for name in first:
        if name not in changes:
            require(first[name] == second[name], 'unrelated project data changed: ' + name)
        else:
            require(first[name] != second[name], 'required control did not change: ' + name)
    return before_models, after_models


def check_cpu(captures):
    adoption = captures / 'adoption'
    names = {'cpu-receipt.json', 'real-cli.ops.json', 'real-cli.stdout', 'real-cli-no-overwrite.stderr'}
    names.update('adoption/' + name for name in ('before.scene.yaml', 'after.scene.yaml', 'reopened.scene.yaml',
                  'wall.ops.json', 'initial.frame', 'reopened.frame', 'verified-report.json', 'history.json'))
    for phase in ('before', 'after'):
        names.update('adoption/' + phase + '-project/' + name for name in project_files())
    inventory = capture_files(captures, names)
    receipt = strict_json(captures / 'cpu-receipt.json')
    require(type(receipt['schema']) is int and receipt['schema'] == 1 and receipt['status'] == 'passed')
    fnv_provenance(receipt, FEATURE_PATHS[2], ['room-wall-cli'])
    rejections = receipt['cli_rejections']
    require(set(rejections) == {'malformed-number', 'duplicate-option', 'overlong-number', 'extra-arguments', 'missing-output'},
            'actual CLI cardinality/token rejection census')
    for item in rejections.values():
        require(set(item) == {'exit', 'stdout', 'stderr'} and type(item['exit']) is int and item['exit'] == 1)
        require(isinstance(item['stdout'], str) and isinstance(item['stderr'], str)
                and item['stderr'].startswith('room_wall:'), 'actual CLI rejection diagnostic')
    compare_projects(adoption / 'before-project', adoption / 'after-project', {'room.scene.yaml'})
    before = (adoption / 'before.scene.yaml').read_bytes()
    after = (adoption / 'after.scene.yaml').read_bytes()
    require(before and after and before != after, 'actual ERP adoption must alter scene')
    require(before == (adoption / 'before-project/room.scene.yaml').read_bytes())
    require(after == (adoption / 'after-project/room.scene.yaml').read_bytes()
            == (adoption / 'reopened.scene.yaml').read_bytes(), 'save/reopen byte identity')
    first = (adoption / 'initial.frame').read_bytes()
    reopened = (adoption / 'reopened.frame').read_bytes()
    require(first and reopened and first != reopened, 'nonvacuous adopted frame')
    report = strict_json(adoption / 'verified-report.json')
    for field in ('base_start', 'candidate_start'):
        checksum = report['checksums'][field]
        require(isinstance(checksum, str) and re.fullmatch(r'0x[0-9a-f]{16}', checksum) is not None,
                'actual ERP starting checksum required')
    check_frame(first, 0, report['checksums']['base_start'], None, 0)
    check_frame(reopened, 0, report['checksums']['candidate_start'], (40960, 32768, 0), 0)
    check_wall_operation(adoption / 'wall.ops.json', [.625, 0], [.125, .75])
    check_wall_operation(captures / 'real-cli.ops.json', [.625, 0], [.125, .75])
    require((adoption / 'wall.ops.json').read_bytes() == (captures / 'real-cli.ops.json').read_bytes(),
            'actual CLI and private production admission output differ')
    require('output already exists' in (captures / 'real-cli-no-overwrite.stderr').read_text(),
            'actual CLI no-overwrite rejection')
    integer_fields(report, ('ticks', 'first_divergence'))
    require(report['ticks'] == 4 and report['passed'] is True and report['first_divergence'] == 0
            and report['identical'] is False, 'actual ERP verification outcome')
    integer_fields(report['metric_sampling'], ('requested_interval', 'sample_count'))
    require(report['metric_sampling']['every_tick_boundary_observed'] is True)
    require(report['metric_sampling'] == {'requested_interval': 1, 'sample_count': 5,
            'scope': 'sampled_tick_boundaries', 'every_tick_boundary_observed': True})
    checks = report['checks']
    require(checks['passed'] is True and len(checks['results']) == 1)
    require(checks['results'][0]['passed'] is True and checks['results'][0]['check'] == 'entities.delta == 1')
    require(set(report['verified_state']) == {'document_id', 'id', 'document_revision', 'proposal_revision'})
    integer_fields(report['verified_state'], ('document_revision', 'proposal_revision'))
    require(all(0 <= report['verified_state'][name] <= 0xffffffffffffffff
                for name in ('document_revision', 'proposal_revision')), 'bounded verified revisions')
    history = strict_json(adoption / 'history.json')
    require(len(history['entries']) == 1)
    integer_fields(history['entries'][0], ('op_count',))
    require(history['entries'][0]['op_count'] == 1)
    runner.report['captures'] = inventory
    runner.report['erp_adoption'] = {'verified_ticks': 4, 'sample_count': 5, 'one_undo_entry': True,
                                   'saved_reopened_scene_sha256': runner.digest(adoption / 'reopened.scene.yaml')}


def check_gpu(captures):
    names = {'receipt.json', 'adapter.json', 'wall.ops.json', 'wrong-position.ops.json'}
    names.update(stem + suffix for stem in GPU_FRAMES for suffix in ('.rgba', '.ppm'))
    for directory in ('baseline-project', 'authored-project', 'wrong-position-project', 'models-off-project'):
        names.update(directory + '/' + name for name in project_files())
    inventory = capture_files(captures, names)
    receipt, adapter = strict_json(captures / 'receipt.json'), strict_json(captures / 'adapter.json')
    require(type(receipt['schema']) is int and receipt['schema'] == 1 and receipt['status'] == 'passed')
    integer_fields(receipt, ('width', 'height', 'interior_pixels', 'unchanged_model_pixels', 'wrong_position_pixels'))
    require(receipt['width'] == receipt['height'] == 512)
    require(receipt['adapter'] == adapter and adapter['software'] is True and receipt['adapter']['software'] is True
            and adapter['backend'] == 'Vulkan' and bool(adapter['name']), 'observed software Vulkan adapter')
    fnv_provenance(receipt, FEATURE_PATHS[3], ['room-wall-cli'])
    for name in ('wall_rect', 'interior_rect', 'exterior_exclusion_rect', 'wrong_position_rect'):
        require(isinstance(receipt[name], list) and all(type(value) is int for value in receipt[name]),
                'exact integer rectangle dimensions')
    require(receipt['wall_rect'] == [384, 80, 448, 176]
            and receipt['interior_rect'] == [386, 82, 446, 174]
            and receipt['exterior_exclusion_rect'] == [382, 78, 450, 178]
            and receipt['wrong_position_rect'] == [384, 336, 448, 432], 'independent numeric wall projection')
    require(set(receipt['frames']) == {name.replace('-', '_') for name in GPU_FRAMES})
    frames = {}
    for stem in GPU_FRAMES:
        entry = receipt['frames'][stem.replace('-', '_')]
        require(set(entry) == {'raw', 'image'})
        fnv_reference(captures, entry['raw'], stem + '.rgba')
        fnv_reference(captures, entry['image'], stem + '.ppm')
        frames[stem] = read_rgba_ppm(captures, stem)
    observations = room_wall_pixel_oracle(frames)
    require(receipt['interior_pixels'] == observations['wall_interior_pixels'])
    require(receipt['unchanged_model_pixels'] == observations['preserved_model_pixels'])
    wrong = sum(frames['baseline'][(y * 512 + x) * 4:(y * 512 + x) * 4 + 4]
                != frames['wrong-position'][(y * 512 + x) * 4:(y * 512 + x) * 4 + 4]
                for y in range(338, 430) for x in range(386, 446))
    require(receipt['wrong_position_pixels'] == wrong and wrong > 500, 'numerical wrong-position control')
    check_wall_operation(captures / 'wall.ops.json', [2.5, -2], [.5, .75])
    check_wall_operation(captures / 'wrong-position.ops.json', [2.5, 2], [.5, .75])
    compare_projects(captures / 'baseline-project', captures / 'authored-project', {'room.scene.yaml'})
    compare_projects(captures / 'baseline-project', captures / 'wrong-position-project', {'room.scene.yaml'})
    before, after = compare_projects(captures / 'baseline-project', captures / 'models-off-project', {'room.models.json'})
    expected = json.loads(json.dumps(before))
    for binding in expected['bindings'].values():
        binding['transform']['translation'] = [1000, 1000, 1000]
    require(after == expected, 'models-off control changes translation only')
    runner.report.update(captures=inventory, gpu_oracle=observations)


EXPORT_STAGES = (('initial', 0, ''), ('moved', 1, 'right'), ('collision', 120, 'right'),
                 ('interacted', 1, 'interact'), ('restarted', 0, ''))
CONTROL_STAGES = (('initial', 0, ''), ('collision', 120, 'right'), ('interacted', 1, 'interact'))
NAMESPACE_MARKER = 'Room wall namespace: verified; project read-only'


FRAME_SCHEMA = (
    (('orr_physics3d::Body', 168), ('orr_physics3d::Collider', 72), ('RoomEscapeV1::Actor', 8)),
    (('orr_sim::FrameRng', 16), ('orr_physics3d::PhysicsState', 128), ('RoomEscapeV1::Run', 16)),
    (('orr_physics3d::ContactCache', 48),),
)


def room_frame(data):
    """Bounded independent layout/behavior reader for source-bound Room frames.

    This checks v2 framing and exact registered Pod layouts, not an independent
    implementation of XXH3. The trailing checksum is matched to the separate
    fresh production runtime; actual PreparedProject performs full admission.
    """
    require(0 < len(data) <= 1024 * 1024, 'bounded Room frame')
    offset = 0
    def take(count):
        nonlocal offset
        require(0 <= count <= len(data) - offset, 'truncated Room frame')
        result = data[offset:offset + count]
        offset += count
        return result
    def u32():
        return struct.unpack('<I', take(4))[0]
    def u64():
        return struct.unpack('<Q', take(8))[0]
    require(take(4) == b'ORRF' and u32() == 2, 'exact Room frame v2')
    tick = u64()
    require(tick <= 120, 'bounded Room frame tick')
    for section in FRAME_SCHEMA:
        require(u32() == len(section), 'Room frame schema count')
        for name, size in section:
            length = u32()
            require(length == len(name) and take(length) == name.encode() and u32() == size,
                    'exact Room component/singleton/list schema')
    alive, slots = u32(), u32()
    require(alive == slots and slots in (7, 8), 'exact live Room actor count')
    versions = take(slots * 4)
    require(versions == bytes(slots * 4) and take(slots) == b'\x01' * slots and u32() == 0,
            'canonical nonreused Room entities')
    stores = {}
    for name, size in FRAME_SCHEMA[0]:
        count = u32()
        require(count == slots, 'every Room actor has all three components')
        identities = [struct.unpack('<II', take(8)) for _ in range(count)]
        require(set(identities) == {(index, 0) for index in range(slots)}, 'complete unique live entity store')
        stores[name] = {identity[0]: take(size) for identity in identities}
    singletons = {name: take(size) for name, size in FRAME_SCHEMA[1]}
    require(singletons['orr_physics3d::PhysicsState'][-8:] == struct.pack('<II', 0, 1),
            'canonical live Room contact-cache handle')
    require(u32() == 1 and u32() == 1 and take(1) == b'\x01' and u32() == 0 and u32() == 0,
            'canonical empty Room contact-cache pool')
    checksum = f'0x{u64():016x}'
    require(offset == len(data), 'trailing Room frame bytes')
    actors = {entity: struct.unpack('<II', value) for entity, value in stores['RoomEscapeV1::Actor'].items()}
    require(all(ordinal == 0 for _, ordinal in actors.values()), 'canonical Room actor ordinals')
    kinds = [kind for kind, _ in actors.values()]
    require(sorted(kinds) == [1, 2, 3] + [4] * (slots - 3), 'exact Room actor kinds')
    player = next(entity for entity, (kind, _) in actors.items() if kind == 1)
    player_pos = struct.unpack_from('<3q', stores['orr_physics3d::Body'][player])
    walls = []
    for entity, (kind, _) in actors.items():
        if kind != 4:
            continue
        body, collider = stores['orr_physics3d::Body'][entity], stores['orr_physics3d::Collider'][entity]
        require(struct.unpack_from('<4q', body, 24) == (0, 0, 0, 65536))
        require(body[56:] == bytes(112), 'static wall canonical dynamics')
        require(struct.unpack_from('<IIq', collider) == (2, 0, 0))
        require(struct.unpack_from('<qqIIII', collider, 40) == (0, 32768, 1, 0, 0, 0))
        walls.append({'center': struct.unpack_from('<3q', body), 'half': struct.unpack_from('<3q', collider, 16)})
    key, won, previous, reserved = struct.unpack('<4I', singletons['RoomEscapeV1::Run'])
    require(key in (0, 1) and won == 0 and previous in (0, 1) and reserved == 0, 'Room run flags')
    require(player_pos[1] == 32768 and player_pos[2] == 0, 'flat Room control motion')
    return {'tick': tick, 'checksum': checksum, 'actors': slots, 'player': player_pos,
            'key_collected': key, 'walls': walls}


def check_frame(data, ticks, checksum, wall, key):
    require(type(ticks) is int and type(key) is int, 'exact integer Frame control inputs')
    frame = room_frame(data)
    require(frame['tick'] == ticks and frame['checksum'] == checksum, 'Frame and actual runtime identity')
    require(frame['key_collected'] == key, 'independent Frame key line-of-sight state')
    require(frame['actors'] == (8 if wall is not None else 7))
    if wall is not None:
        require(frame['walls'].count({'center': wall, 'half': (8192, 65536, 49152)}) == 1,
                'independent authored wall center/box extents')
    if ticks == 0:
        require(frame['player'] == (0, 32768, 0), 'initial/restarted player frame')
    if ticks == 120:
        require(0 < frame['player'][0] < 8192 if wall == (40960, 32768, 0)
                else frame['player'][0] > 3 * 65536, 'independent wall-blocked/reachable control motion')
    return frame


def check_runtime_output(output, ticks, checksum, key, initial=None):
    require(type(ticks) is int and type(key) is int, 'exact integer runtime control inputs')
    require(isinstance(output, str) and len(output) <= 1024 * 1024)
    require(re.fullmatch(r'0x[0-9a-f]{16}', checksum) is not None, 'runtime checksum shape')
    require(not re.search(r'SKIP:|\bskipping\b|adapter unavailable|without source hiding', output, re.I))
    require(re.findall(r'^room tick: (\d+) checksum: (0x[0-9a-f]{16})$', output, re.M)
            == [(str(ticks), checksum)], 'exact current runtime tick/checksum')
    require(re.findall(r'^room key: (\d+) won: (\d+)$', output, re.M) == [(str(key), '0')],
            'actual Room key/LOS state')
    adapters = re.findall(r'^room capture adapter: (.+)$', output, re.M)
    require(len(adapters) == 1 and adapters[0].strip(), 'actual runtime GPU capture adapter')
    if initial is not None:
        require(re.findall(r'^room initial checksum: (0x[0-9a-f]{16})$', output, re.M) == [initial])


def check_probe_output(output, ticks, checksum):
    require(type(ticks) is int, 'exact integer probe tick')
    runner.result(output, [EXPORT_TEST])
    require(output.count(NAMESPACE_MARKER) == 1, 'actual source-hidden read-only namespace proof')
    require(output.count(f'PreparedProject frame tick={ticks} checksum={checksum}') == 1,
            'actual independent PreparedProject frame probe')


def check_export_manifest(captures, receipt):
    manifest = strict_json(captures / 'orr.export.json')
    integer_fields(manifest, ('schema',))
    require(set(manifest) == {'schema', 'payload', 'content_digest'} and manifest['schema'] == 1)
    payload = manifest['payload']
    require(set(payload) == {'exporter_version', 'profile', 'entry', 'packages', 'files',
                             'runtime', 'declared', 'initial_checksum'})
    require(payload['profile'] == 'room-escape-authored-linux-x86_64-v1')
    require(payload['entry'] == {'game': 'room-escape-v1', 'scene': 'room.scene.yaml',
                                 'models': 'room.models.json', 'camera': 'room.camera.json'})
    require(payload['packages'] == {PACKAGE_NAME: PACKAGE_DIGEST})
    runtime = runner.report['producers']['room_escape']
    integer_fields(payload['runtime'], ('bytes',))
    require(payload['runtime'] == {'sha256': runtime['sha256'], 'bytes': runtime['bytes']})
    require(payload['initial_checksum'] == receipt['observations']['initial']['checksum'])
    require(payload['declared']['target'] == 'linux-x86_64')
    canonical = {key: payload[key] for key in ('exporter_version', 'profile', 'entry', 'packages',
                                              'files', 'runtime', 'declared', 'initial_checksum')}
    content = b'orrery.room-escape.export.content.v1\0' + json.dumps(canonical, separators=(',', ':'), ensure_ascii=False).encode()
    require(hashlib.sha256(content).hexdigest() == manifest['content_digest'], 'export content digest')
    exported = receipt['exported_file_inventory']
    inventory_numbers(exported)
    expected = {'project/' + name for name in project_files()} | {'bin/room_escape', 'run-room-escape', 'orr.export.json'}
    require(set(exported) == expected, 'exact relocated exported file inventory')
    files = payload['files']
    require(isinstance(files, list) and len(files) == len(expected) - 1)
    require({entry['path'] for entry in files} == expected - {'orr.export.json'}, 'manifest file closure')
    for entry in files:
        require(set(entry) == {'path', 'role', 'mode', 'bytes', 'sha256'})
        name = entry['path']
        require(type(entry['bytes']) is int and entry['bytes'] > 0)
        require(re.fullmatch('[0-9a-f]{64}', entry['sha256']) is not None)
        require(exported[name] == [entry['bytes'], entry['sha256']])
        require(type(entry['mode']) is int and entry['mode'] in (0o644, 0o755))
        if name.startswith('project/'):
            retained = captures / 'authored-project' / name[len('project/'):]
            require(exported[name] == [retained.stat().st_size, runner.digest(retained)])
        elif name == 'bin/room_escape':
            require(exported[name] == [runtime['bytes'], runtime['sha256']])
    require(exported['orr.export.json'] == [(captures / 'orr.export.json').stat().st_size,
                                           runner.digest(captures / 'orr.export.json')])


def check_export(captures):
    names = {'production-receipt.json', 'capture-inventory.json', 'negative-controls.json',
             'wall.ops.json', 'wrong-position.ops.json', 'orr.export.json'}
    for name, _, _ in EXPORT_STAGES:
        for route in ('source', 'export'):
            names.update((f'runtime-{name}-{route}.png', f'prepared-project-{name}-{route}.frame'))
    for control in ('baseline', 'wrong-position'):
        for stage, _, _ in CONTROL_STAGES:
            names.update((f'control-{control}-{stage}.png', f'control-{control}-{stage}.frame'))
    for directory in ('authored-project', 'baseline-project', 'wrong-position-project'):
        names.update(directory + '/' + name for name in project_files())
    inventory = capture_files(captures, names)
    expected_inventory = dict(inventory)
    del expected_inventory['capture-inventory.json']
    retained_inventory = strict_json(captures / 'capture-inventory.json')
    inventory_numbers(retained_inventory)
    require(retained_inventory == expected_inventory,
            'independent full export capture SHA256 inventory')
    receipt = strict_json(captures / 'production-receipt.json')
    require(type(receipt['schema']) is int and receipt['schema'] == 1 and receipt['status'] == 'passed')
    source_provenance(receipt, FEATURE_PATHS[4], ['room-wall-cli', 'room_escape', 'orr_export_room', 'export-test'])
    require(set(receipt['observations']) == {name for name, _, _ in EXPORT_STAGES})
    require(set(receipt['controls']) == {name + '-' + stage for name in ('baseline', 'wrong-position')
                                       for stage, _, _ in CONTROL_STAGES})
    initial_checksum = receipt['observations']['initial']['checksum']
    images, frames = {}, {}
    for name, ticks, held in EXPORT_STAGES:
        item = receipt['observations'][name]
        integer_fields(item, ('ticks',))
        require(item['ticks'] == ticks and item['held'] == held, 'exact exported runtime stage')
        require(item['source_stdout'] == item['export_stdout'], 'source/export actual runtime parity')
        check_runtime_output(item['source_stdout'], ticks, item['checksum'], 0, initial_checksum)
        for route in ('source', 'export'):
            check_probe_output(item[route + '_probe_stdout'], ticks, item['checksum'])
        for extension, prefix in (('png', 'runtime'), ('frame', 'prepared-project')):
            first = captures / f'{prefix}-{name}-source.{extension}'
            second = captures / f'{prefix}-{name}-export.{extension}'
            require(first.stat().st_size > 0 and first.read_bytes() == second.read_bytes(),
                    'source-hidden read-only byte parity: ' + name + '.' + extension)
            for route, path in (('source', first), ('export', second)):
                require(item[route + '_' + extension + '_sha256'] == runner.digest(path))
        images[name] = png_pixels(captures / f'runtime-{name}-source.png')
        frames[name] = (captures / f'prepared-project-{name}-source.frame').read_bytes()
        check_frame(frames[name], ticks, item['checksum'], (40960, 32768, 0), 0)
    require(images['restarted'] == images['initial'] and frames['restarted'] == frames['initial'],
            'fresh reopened initial/restarted parity')
    require(difference(images['initial'], images['collision'], tolerance=0) > 20,
            'actual wall-limited movement must be visible')
    require(frames['initial'] != frames['collision'], 'actual collision run frame changed')
    for control in ('baseline', 'wrong-position'):
        for stage, ticks, held in CONTROL_STAGES:
            item = receipt['controls'][control + '-' + stage]
            integer_fields(item, ('ticks',))
            require(item['ticks'] == ticks and item['held'] == held)
            check_runtime_output(item['stdout'], ticks, item['checksum'], int(stage == 'interacted'))
            check_probe_output(item['probe_stdout'], ticks, item['checksum'])
            png = captures / f'control-{control}-{stage}.png'
            frame = captures / f'control-{control}-{stage}.frame'
            require(item['png_sha256'] == runner.digest(png) and item['frame_sha256'] == runner.digest(frame))
            pixels = png_pixels(png)
            check_frame(frame.read_bytes(), ticks, item['checksum'],
                        (-131072, 32768, 0) if control == 'wrong-position' else None, int(stage == 'interacted'))
            require(frame.stat().st_size > 0 and frame.read_bytes() != frames[stage],
                    'wall collision/LOS negative-control frame')
            if stage == 'initial':
                require(difference(pixels, images['initial'], tolerance=0) > 20,
                        'authored wall runtime appearance negative control')
    negative = strict_json(captures / 'negative-controls.json')
    diagnostics = {'capture-no-overwrite': 'already exists', 'runtime-read-only': 'Read-only file system',
                   'exporter-read-only': 'Read-only file system', 'export-no-overwrite': 'output destination already exists'}
    require(set(negative) == set(diagnostics), 'exact negative command controls')
    for name, diagnostic in diagnostics.items():
        item = negative[name]
        require(set(item) == {'exit', 'stdout', 'stderr'} and type(item['exit']) is int and item['exit'] != 0)
        require(isinstance(item['stdout'], str) and isinstance(item['stderr'], str)
                and diagnostic in item['stderr'], 'actual expected control rejection')
    compare_projects(captures / 'baseline-project', captures / 'authored-project', {'room.scene.yaml'})
    compare_projects(captures / 'baseline-project', captures / 'wrong-position-project', {'room.scene.yaml'})
    check_wall_operation(captures / 'wall.ops.json', [.625, 0], [.125, .75])
    check_wall_operation(captures / 'wrong-position.ops.json', [-2, 0], [.125, .75])
    check_export_manifest(captures, receipt)
    runner.report.update(captures=inventory, export_boundary=receipt['frame_evidence_boundary'],
                         namespace_source_hidden=True, readonly_negative_controls=True)


def main():
    require_python()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=PHASES)
    args = parser.parse_args()
    initialize(Path(__file__).resolve().parents[1], Path(os.environ['ORR_ROOM_WALL_GATE_EVIDENCE']), args.mode)
    error = None
    try:
        begin()
        {'cpu': cpu, 'gpu': gpu, 'export': export}[mode]()
    except BaseException as failure:
        error = failure
    finally:
        finish(error)


if __name__ == '__main__':
    main()
