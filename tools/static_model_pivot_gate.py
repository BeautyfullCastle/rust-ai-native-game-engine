#!/usr/bin/env python3
"""Mandatory current-source pivot-variant acceptance; standard library only.

Hosted cpu/gpu/export each compile independent, retained producers in a fresh
target. Separate local model-only acceptance cannot certify bindings, GPU
pixels, or source-hidden production export. The reviewed static
prop runner supplies bounded output, process-tree cleanup and producer identity;
its source and independent fixtures are part of every prebuild source fence.
"""
import copy
import math
import hashlib
import json
import os
import pathlib
import re
import sys
import traceback
import struct
import zlib

if not __debug__ or sys.flags.optimize or os.environ.get('PYTHONOPTIMIZE') not in (None, '', '0'):
    raise SystemExit("static pivot acceptance requires nonoptimized Python")

import static_model_extract_gate as runner

FEATURE_PATHS = (
    'crates/orr_model/examples/static_model_pivot.rs',
    'crates/orr_model/examples/static_model_pivot/authoring.rs',
    'crates/orr_model/tests/static_model_pivot.rs',
    'crates/orr_model_bindings/tests/static_model_pivot.rs',
    'crates/orr_render/tests/static_model_pivot.rs',
    'crates/orr_sample/tests/static_model_pivot_export.rs',
    'docs/static-model-pivot.md',
    'tools/static_model_pivot_gate.py',
    'tools/test_static_model_pivot_gate.py',
)
INFRA_PATHS = ('Cargo.toml', 'Cargo.lock', 'tools/static_model_extract_gate.py',
               'tools/test_static_model_extract_gate.py',
               'tools/static_model_retexture_gate.py', 'tools/test_static_model_retexture_gate.py',
               'tools/static_model_uv_gate.py', 'tools/test_static_model_uv_gate.py',
               '.github/workflows/determinism.yml')
BINDINGS_TEST = 'installed_static_pivot_variants_keep_distinct_bindings_and_source_geometry'
GPU_TEST = 'static_pivot_variants_match_independent_compensated_original_oracle'
EXPORT_TEST = 'static_model_pivot_real_cli_source_hidden_room_export'
FIXTURE_COUNT = 24
WORKFLOW_JOB = 'static-model-authoring'
WORKFLOW_STEP = 'Required independent static model authoring acceptance'
WORKFLOW_RETENTION = 'Retain static authoring and partial failure evidence'
WORKFLOW_ROWS = (
    ('uv', 'cpu', 'ubuntu-22.04', 45),
    ('uv', 'cpu', 'windows-latest', 45),
    ('uv', 'gpu', 'ubuntu-22.04', 45),
    ('uv', 'export', 'ubuntu-22.04', 60),
    ('pivot', 'cpu', 'ubuntu-22.04', 45),
    ('pivot', 'cpu', 'windows-latest', 45),
    ('pivot', 'gpu', 'ubuntu-22.04', 45),
    ('pivot', 'export', 'ubuntu-22.04', 60),
)
WORKFLOW_EVIDENCE = "${{ matrix.mode == 'export' && '/tmp' || runner.temp }}/orr-static-${{ matrix.feature }}-${{ matrix.mode }}-${{ github.run_id }}-${{ github.run_attempt }}"
WORKFLOW_ARTIFACT = 'static-${{ matrix.feature }}-${{ matrix.mode }}-${{ matrix.os }}'


source_root = None
frozen = None
mode = None


def source_paths(root):
    """Complete source inventory, including new files in a non-Git local copy."""
    result = []
    for directory, folders, files in os.walk(root, followlinks=False):
        parent = pathlib.Path(directory)
        if parent == root:
            folders[:] = [name for name in folders if name not in {'.git', 'target'}]
        for name in folders:
            assert not (parent / name).is_symlink(), 'symlink source directory'
        for name in files:
            path = parent / name
            assert path.is_file() and not path.is_symlink(), 'non-regular source'
            result.append(path.relative_to(root).as_posix())
    return sorted(result)


def check_source_inventory(tracked, phase):
    observed = source_paths(source_root)
    expected = sorted(tracked)
    entry = {'tracked': expected, 'observed': observed,
             'extra': sorted(set(observed) - set(expected)),
             'missing': sorted(set(expected) - set(observed)),
             'matches': expected == observed}
    runner.report.setdefault('source_path_inventories', {})[phase] = entry
    # Preserve the actual path difference even when Git ignores an unexpected
    # file and therefore reports a clean worktree. Do not exempt or remove it.
    runner.record()
    assert expected == observed, 'untracked/ignored source cannot enter producers'


def snapshot():
    result = {}
    for name in source_paths(source_root):
        path = source_root / name
        result[name] = {'sha256': runner.digest(path), 'bytes': path.stat().st_size,
                        'mode': path.stat().st_mode & 0o7777}
    return result


def require_sources(value):
    assert len(FEATURE_PATHS) == len(set(FEATURE_PATHS)) == 9
    required = set(FEATURE_PATHS + INFRA_PATHS)
    for package in ('orr_model', 'orr_model_bindings', 'orr_render', 'orr_sample'):
        required.add(f'crates/{package}/Cargo.toml')
        required.add(f'crates/{package}/src/lib.rs')
    assert required <= set(value), 'complete feature and producer sources required'


def source_fence():
    if frozen is not None:
        current = snapshot()
        if list(current) != list(frozen):
            check_source_inventory(list(frozen), 'source-fence')
        assert current == frozen, 'source bytes, modes or paths changed'


def run(label, command, expected=0, seconds=1200):
    source_fence()
    output = runner.run(label, command, expected=expected, seconds=seconds)
    source_fence()
    return output


def initialize(root, destination, phase):
    global source_root, frozen, mode
    assert phase in {'cpu', 'gpu', 'export'}
    source_root = pathlib.Path(root).resolve()
    frozen = None
    mode = phase
    destination = pathlib.Path(destination)
    assert not destination.is_relative_to(source_root), 'evidence must be outside source'
    runner.initialize(source_root, destination, phase)
    runner.report['mode'] = phase
    runner.report['coverage'] = {name: 'NOT RUN' for name in ('model', 'bindings', 'gpu', 'export')}
    runner.report['local_model_only_cannot_certify_hosted_acceptance'] = True
    runner.report['workflow_integration'] = 'PENDING current-source matrix contract check'
    runner.report['workflow_job'] = WORKFLOW_JOB
    runner.report['workflow_rows'] = WORKFLOW_ROWS
    runner.report['workflow_evidence'] = WORKFLOW_EVIDENCE
    runner.report['workflow_artifact'] = WORKFLOW_ARTIFACT
    runner.record()


def require_inherited_bytecode_prevention():
    observed = {'PYTHONDONTWRITEBYTECODE': os.environ.get('PYTHONDONTWRITEBYTECODE'),
                'sys_dont_write_bytecode': sys.dont_write_bytecode}
    runner.report['python_bytecode_prevention'] = observed
    # Record the inherited state before rejection. A local -B flag alone does
    # not protect Python children launched by earlier workflow producers.
    runner.record()
    assert observed['PYTHONDONTWRITEBYTECODE'] == '1', 'inherited PYTHONDONTWRITEBYTECODE=1 required'


def begin():
    global frozen
    require_inherited_bytecode_prevention()
    assert run('clean-source-before', ['git', 'status', '--porcelain=v1', '--untracked-files=all']).strip() == ''
    for key, revision in [('head', 'HEAD'), ('tree', 'HEAD^{tree}')]:
        runner.report[key] = run('checkout-' + key, ['git', 'rev-parse', revision]).strip()
        assert re.fullmatch('[0-9a-f]{40}', runner.report[key])
    if 'GITHUB_SHA' in os.environ:
        assert os.environ['GITHUB_SHA'] == runner.report['head']
    tracked = run('tracked-source', ['git', 'ls-files', '-z']).split('\0')
    assert tracked.pop() == ''
    check_source_inventory(tracked, 'before')
    frozen = snapshot()
    require_sources(frozen)
    runner.report.update(checkout=str(source_root), source_sha256=frozen,
                         source_inventory_sha256=hashlib.sha256(json.dumps(frozen, sort_keys=True).encode()).hexdigest(),
                         feature_paths=list(FEATURE_PATHS))
    runner.record()
    for file, marker in [
        ('test_static_model_extract_gate.py', 'STATIC_PROP_GATE_FIXTURES: 29 passed; 0 failed; 0 skipped'),
        ('test_static_model_pivot_gate.py', f'STATIC_PIVOT_GATE_FIXTURES: {FIXTURE_COUNT} passed; 0 failed; 0 skipped'),
    ]:
        output = run('checker-' + file[:-3], [sys.executable, '-B', str(source_root / 'tools' / file)])
        assert output.rstrip().endswith(marker), 'independent gate fixture census'
    runner.report['workflow_integration'] = 'CURRENT_SOURCE_MATRIX_CONTRACT_PASSED; hosted acceptance remains mode-specific'
    runner.record()


def pin(output, label, package, target, kind, features):
    source_fence()
    binary = runner.pin(output, label, package, target, kind, features)
    producer = runner.report['producers'][label]
    producer['source_inventory_sha256'] = runner.report['source_inventory_sha256']
    producer['source_root'] = str(source_root)
    runner.record()
    return binary


def test(label, binary, names, ignored=False):
    runner.inventory(run(label + '-inventory', [binary, '--list']), names)
    runner.inventory(run(label + '-ignored', [binary, '--ignored', '--list']), names if ignored else [])
    flags = ['--ignored', '--exact', names[0]] if ignored and len(names) == 1 else (['--ignored'] if ignored else [])
    runner.result(run(label + '-run', [binary, *flags, '--test-threads=1']), names)


def cargo_flags(package='orr_model'):
    return ['--locked', '--release', '-p', package, '--no-default-features']


def build_cli():
    flags = cargo_flags()
    output = run('build-cli', ['cargo', 'build', *flags, '--features', 'import',
                              '--example', 'static_model_pivot', '--message-format=json'])
    binary = pin(output, 'pivot-cli', 'orr_model', 'static_model_pivot', 'example', ['import'])
    os.environ['ORR_STATIC_MODEL_PIVOT_BIN'] = binary
    return binary


def model_tests(platform):
    assert platform in {'linux', 'darwin', 'win32'}, 'unsupported model test platform'
    names = """rebase_is_model_space_translation_for_all_composed_origins_and_preserves_bits
zero_pivots_preserve_positive_and_negative_zero_translation_bits
pivot_numbers_quantize_to_f32_and_subtraction_uses_f32
translation_and_pivot_bounds_reject_overflow_without_saturation
rebase_revalidates_world_vertex_bounds_after_translation
recipe_accepts_exact_object_whitespace_and_16kib_boundary
recipe_rejects_missing_duplicate_unknown_null_and_positional_aliases
recipe_rejects_wrong_types_versions_pivot_lengths_and_byte_budget
logical_paths_reject_nonportable_unnormalized_suffixes_and_aliases
logical_paths_accept_240_byte_boundary
source_resolves_once_no_historical_dependencies_and_errors_propagate
entire_cooked_source_is_admitted_before_transforming
provenance_binds_exact_bytes_and_roundtrip_is_deterministic
combined_input_and_independent_cooked_output_budgets_are_enforced
cli_publishes_once_without_overwrite_and_preserves_all_inputs
cli_help_and_three_unique_pairs_are_bounded
cli_bad_arguments_fail_without_output_or_staging
cli_portable_output_names_and_package_store_rejection
cli_missing_nonregular_and_oversized_inputs_leave_no_output
cli_resolver_is_anchored_to_recipe_parent
cli_parent_traversal_preserves_raw_spelling_and_never_publishes""".split()
    if platform == 'win32':
        names.append('cli_windows_nonlocal_prefixes_reject')
    else:
        names.extend(['cli_symlink_inputs_ancestors_and_destinations_reject',
                      'cli_non_utf8_option_names_and_logical_ids_reject',
                      'cli_unix_socket_inputs_reject_without_opening_or_publishing'])
        if platform == 'linux':
            names.append('cli_non_utf8_filesystem_inputs_work')
    assert len(names) == {'linux': 25, 'darwin': 24, 'win32': 22}[platform]
    assert len(names) == len(set(names))
    return names


def model():
    flags = cargo_flags()
    output = run('build-feature-off', ['cargo', 'build', *flags, '--example',
                                      'static_model_pivot', '--message-format=json'])
    disabled = pin(output, 'feature-off-cli', 'orr_model', 'static_model_pivot', 'example', [])
    assert 'requires --features import' in run('feature-off-exit2', [disabled, '--help'], expected=2)
    cli = build_cli()
    assert 'Usage: static_model_pivot' in run('actual-help', [cli, '--help'])
    output = run('build-model-test', ['cargo', 'test', *flags, '--features', 'import',
                                    '--test', 'static_model_pivot', '--no-run', '--message-format=json'])
    binary = pin(output, 'model-test', 'orr_model', 'static_model_pivot', 'test', ['import'])
    test('model', binary, model_tests(sys.platform))
    # Target only model library and the new source-bound fixture. Other workspace
    # acceptance (including existing socket fixtures) has its own required gates.
    run('model-library-regressions', ['cargo', 'test', *flags, '--features', 'import', '--lib'])
    run('strict-model-import', ['cargo', 'clippy', *flags, '--features', 'import',
                               '--lib', '--example', 'static_model_pivot', '--test',
                               'static_model_pivot', '--', '-D', 'warnings'])
    run('strict-model-default', ['cargo', 'clippy', *flags, '--lib', '--example',
                                'static_model_pivot', '--', '-D', 'warnings'])
    runner.report['coverage']['model'] = 'PASSED'
    runner.record()


def cpu():
    model()
    flags = cargo_flags('orr_model_bindings')
    output = run('build-bindings-test', ['cargo', 'test', *flags, '--test',
                                       'static_model_pivot', '--no-run', '--message-format=json'])
    binary = pin(output, 'bindings-test', 'orr_model_bindings', 'static_model_pivot', 'test', [])
    test('bindings', binary, [BINDINGS_TEST], ignored=True)
    run('strict-bindings', ['cargo', 'clippy', *flags, '--test', 'static_model_pivot', '--', '-D', 'warnings'])
    runner.report['coverage']['bindings'] = 'PASSED'
    runner.record()


def capture_files(captures, expected):
    assert captures.is_dir() and not captures.is_symlink()
    assert {path.name for path in captures.iterdir()} == expected, 'exact capture inventory'
    files = []
    for directory, folders, names in os.walk(captures, followlinks=False):
        for name in folders:
            assert not (pathlib.Path(directory) / name).is_symlink()
        for name in names:
            path = pathlib.Path(directory) / name
            assert path.is_file() and not path.is_symlink() and path.stat().st_size <= 64 * 1024**2
            files.append(path)
            assert len(files) < 128, 'bounded capture inventory'
    return {path.relative_to(captures).as_posix(): [path.stat().st_size, runner.digest(path)]
            for path in sorted(files)}


def capture_reference(captures, entry):
    assert set(entry) == {'file', 'bytes', 'fnv1a64'}
    assert entry['file'] == pathlib.Path(entry['file']).name
    data = (captures / entry['file']).read_bytes()
    value = 0xcbf29ce484222325
    for byte in data:
        value = ((value ^ byte) * 0x100000001b3) & 0xffffffffffffffff
    assert entry['bytes'] == len(data) and entry['fnv1a64'] == f'{value:016x}'


def f32(value):
    assert type(value) in {int, float}, 'f32 numeric value required'
    result = struct.unpack('<f', struct.pack('<f', value))[0]
    assert math.isfinite(result), 'finite f32 required'
    return result


def check_variant(captures, source_file, recipe_file, cooked_file, asset_id):
    source = json.loads((captures / source_file).read_text())
    recipe = json.loads((captures / recipe_file).read_text())
    cooked = json.loads((captures / cooked_file).read_text())
    assert set(recipe) == {'format', 'version', 'source', 'pivot'}
    assert recipe['format'] == 'orr_static_model_pivot' and type(recipe['version']) is int and recipe['version'] == 1
    pivot = recipe['pivot']
    assert type(pivot) is list and len(pivot) == 3
    assert all(abs(f32(value)) <= 1e6 for value in pivot)
    assert set(cooked) == set(source)
    assert cooked['format'] == source['format'] == 'orr_static_model'
    assert cooked['version'] == source['version'] == 1
    assert cooked['asset_id'] == asset_id and asset_id.endswith('.pivot.json')
    assert cooked['dependencies'] == [
        {'uri': '$source', 'sha256': runner.digest(captures / recipe_file)},
        {'uri': recipe['source'], 'sha256': runner.digest(captures / source_file)},
    ], 'exact ordered recipe/source byte provenance without historical resolution'
    assert len(cooked['primitives']) == len(source['primitives']) >= 4
    for original, variant in zip(source['primitives'], cooked['primitives']):
        assert original['id'].startswith(source['asset_id'] + '#node=')
        assert type(original['transform']) is list and len(original['transform']) == 4, 'source transform shape'
        for column in original['transform']:
            assert type(column) is list and len(column) == 4, 'source transform shape'
            for value in column:
                f32(value)
        expected = copy.deepcopy(original)
        expected['id'] = asset_id + original['id'][len(source['asset_id']):]
        for axis, value in enumerate(pivot):
            if f32(value) != 0:
                expected['transform'][3][axis] = f32(f32(original['transform'][3][axis]) - f32(value))
        assert type(variant) is dict and set(variant) == set(expected), 'exact primitive fields'
        assert {key: value for key, value in variant.items() if key != 'transform'} == {
            key: value for key, value in expected.items() if key != 'transform'
        }, 'only identity and translation column may change'
        # Rust serializes f32 values using their shortest round-trip decimals.
        # Python's expanded arithmetic result can differ as an f64 while naming
        # exactly the same f32. Compare every transform bit, never a tolerance.
        assert type(variant['transform']) is list and len(variant['transform']) == len(expected['transform']) == 4
        for actual_column, expected_column in zip(variant['transform'], expected['transform']):
            assert type(actual_column) is list and len(actual_column) == len(expected_column) == 4
            assert b''.join(struct.pack('<f', f32(v)) for v in actual_column) == b''.join(
                struct.pack('<f', f32(v)) for v in expected_column), 'preserve all transform bits including signed zero'
    assert cooked['materials'] == source['materials'], 'all material factors, slots and sampler preserved'
    assert cooked['images'] == source['images'], 'all image bytes and slots preserved'
    assert {item['material'] for item in source['primitives']} >= {0, 1}, 'multiple material controls'
    return cooked


def difference(first, second, tolerance=3):
    assert len(first) == len(second) and len(first) % 4 == 0
    return sum(any(abs(first[index + channel] - second[index + channel]) > tolerance for channel in range(4))
               for index in range(0, len(first), 4))


POSITIONS = ((-1,-1),(1,-1),(1,0.4),(0.4,1),(-1,1),(1.1,-0.75),(1.8,-0.75),(1.4,0.4))
TRIANGLES = ((0,1,2),(0,2,3),(0,3,4),(5,6,7))
CHECKER = ((255,0,0,255),(0,255,0,255),(0,0,255,255),(128,128,128,255))
PIVOTS = ((0.625,-0.375,0.125),(-0.5,0.75,0.0))


def oracle_samples(placements):
    """Independent numeric oracle; never consumes producer geometry or receipts."""
    panels = [[0,0] for _ in placements]; markers = [[0,0] for _ in placements]
    checker = [0]*4; clear = 0; expected = {}
    for y in range(256):
        for x in range(256):
            world = ((x+0.5-128)/32,(128-y-0.5)/32)
            near = False; color = None
            for placement, (instance, pivot) in enumerate(placements):
                tx,sx,sy = (-2,0.75,0.5) if instance == 0 else (2,0.5,0.75)
                model = (-(world[0]-tx)/sx+pivot[0],-world[1]/sy+pivot[1])
                for node in range(2):
                    local = (model[0]/-0.5,(model[1]-1.25)/0.75) if node == 0 else ((model[1]+1.25)/0.5,model[0]/-0.75)
                    for triangle, ids in enumerate(TRIANGLES):
                        a,b,c = (POSITIONS[i] for i in ids)
                        d = (b[1]-c[1])*(a[0]-c[0])+(c[0]-b[0])*(a[1]-c[1])
                        u = ((b[1]-c[1])*(local[0]-c[0])+(c[0]-b[0])*(local[1]-c[1]))/d
                        v = ((c[1]-a[1])*(local[0]-c[0])+(a[0]-c[0])*(local[1]-c[1]))/d
                        w = (u,v,1-u-v)
                        near |= min(w) >= -0.08
                        if min(w) <= 0.08: continue
                        if triangle == 3:
                            markers[placement][node] += 1
                            value = (188,255,0,255)
                        else:
                            texel = (local[0]+1,1-local[1])
                            if any(not 0.02 <= t-math.trunc(t) <= 0.98 for t in texel): continue
                            slot = math.floor(texel[1])*2+math.floor(texel[0])
                            assert 0 <= slot < 4
                            value = CHECKER[slot]
                            checker[slot] += 1; panels[placement][node] += 1
                        assert color is None, 'fixture surfaces overlap'
                        color = value
            if color is None and not near:
                color = (0,0,0,255); clear += 1
            if color is not None: expected[(y*256+x)*4] = bytes(color)
    assert all(n>120 for pair in panels for n in pair)
    assert all(n>10 for pair in markers for n in pair)
    assert all(n>100 for n in checker) and clear>50000
    return expected, {'panel_pixels':panels,'marker_pixels':markers,'checker_pixels':checker,'clear_checked':clear}


def png_pixels(path, width=1024, height=768):
    """Bounded independent decoder for the required non-interlaced RGBA8 proof."""
    assert 0 < path.stat().st_size <= 16 * 1024**2
    data = path.read_bytes()
    assert data[:8] == b'\x89PNG\r\n\x1a\n'
    offset = 8; compressed = bytearray(); state = 'header'; chunks = 0
    while offset < len(data):
        chunks += 1
        assert chunks < 4096 and offset + 12 <= len(data)
        length = struct.unpack_from('>I', data, offset)[0]
        assert length <= 16 * 1024**2 and offset + length + 12 <= len(data)
        kind = data[offset + 4:offset + 8]
        payload = data[offset + 8:offset + 8 + length]
        crc = struct.unpack_from('>I', data, offset + 8 + length)[0]
        assert crc == zlib.crc32(kind + payload) & 0xffffffff, 'PNG CRC'
        offset += length + 12
        assert kind not in {b'acTL', b'fcTL', b'fdAT'}, 'APNG is not a frame proof'
        if state == 'header':
            assert kind == b'IHDR' and length == 13
            assert struct.unpack('>IIBBBBB', payload) == (width, height, 8, 6, 0, 0, 0)
            state = 'before-data'
        elif kind == b'IDAT':
            assert state in {'before-data', 'data'}, 'contiguous PNG IDAT'
            state = 'data'; compressed.extend(payload)
            assert len(compressed) <= 16 * 1024**2
        elif kind == b'IEND':
            assert state in {'data', 'after-data'} and length == 0 and offset == len(data)
            state = 'ended'
        else:
            assert kind[0] & 32 and state != 'ended', 'unexpected critical PNG chunk'
            if state == 'data':
                state = 'after-data'
    assert state == 'ended' and compressed
    row_bytes = width * 4; expected = (row_bytes + 1) * height
    decoder = zlib.decompressobj()
    raw = decoder.decompress(compressed, expected + 1)
    assert len(raw) == expected and decoder.eof and not decoder.unused_data and not decoder.unconsumed_tail
    output = bytearray(); previous = bytearray(row_bytes)
    for y in range(height):
        start = y * (row_bytes + 1); filter_type = raw[start]
        assert filter_type <= 4
        row = bytearray(raw[start + 1:start + 1 + row_bytes])
        if filter_type:
            for x in range(row_bytes):
                left = row[x - 4] if x >= 4 else 0
                above = previous[x]
                corner = previous[x - 4] if x >= 4 else 0
                if filter_type == 1:
                    predictor = left
                elif filter_type == 2:
                    predictor = above
                elif filter_type == 3:
                    predictor = (left + above) // 2
                else:
                    predicted = left + above - corner
                    distances = (abs(predicted - left), abs(predicted - above), abs(predicted - corner))
                    predictor = (left, above, corner)[distances.index(min(distances))]
                row[x] = (row[x] + predictor) & 255
        output.extend(row); previous = row
    assert len(output) == width * height * 4
    return bytes(output)


GPU_FRAMES = ('offset-a','offset-b','compensated-original-a','compensated-original-b','no-pivot',
              'wrong-sign','wrong-order','wrong-uv','wrong-material','variants-together','variants-reversed')


def check_gpu(captures):
    names = {'source.glb','source.model.json','adapter.json','receipt.json'}
    names.update(stem+suffix for stem in ('offset-a','offset-b') for suffix in ('.pivot.json','.model.json'))
    names.update(stem+suffix for stem in GPU_FRAMES for suffix in ('.rgba','.ppm'))
    inventory = capture_files(captures,names)
    accepted = json.loads((captures/'receipt.json').read_text())
    adapter = json.loads((captures/'adapter.json').read_text())
    assert accepted['schema'] == 1 and accepted['status'] == 'passed'
    assert accepted['width'] == accepted['height'] == 256
    assert accepted['adapter'] == adapter and adapter['software'] is True and adapter['backend'] == 'Vulkan'
    for field in ('immutable_assets','distinct_recipe_namespaces','reversed_batch_order_byte_identical'):
        assert accepted[field] is True
    assert accepted['oracle'] == 'independent compensated original E*T(-pivot)*M and numeric fixture-space interior/background'
    assert accepted['external_instances'] == 'two Rz(pi) rotations with nonuniform scales; node 1 mirrored, node 7 rotated'
    assert accepted['tolerances'] == {'color_bytes':3,'minimum_control_differences':200}
    frames = accepted['frames']; pixels = {}
    assert set(frames) == {name.replace('-','_') for name in GPU_FRAMES}
    for name in GPU_FRAMES:
        key = name.replace('-','_'); frame = frames[key]
        assert set(frame) == {'raw','image'}
        assert frame['raw']['file'] == name+'.rgba' and frame['image']['file'] == name+'.ppm'
        for entry in frame.values(): capture_reference(captures,entry)
        raw = (captures/(name+'.rgba')).read_bytes(); pixels[key] = raw
        assert len(raw) == 256*256*4 and raw[3::4] == b'\xff'*(256*256)
        assert (captures/(name+'.ppm')).read_bytes() == b'P6\n256 256\n255\n'+bytes(value for index,value in enumerate(raw) if index%4 != 3)
    assert pixels['offset_a'] == pixels['compensated_original_a'], 'independent original externally compensated A'
    assert pixels['offset_b'] == pixels['compensated_original_b'], 'independent original externally compensated B'
    assert pixels['variants_together'] == pixels['variants_reversed']
    placements = {'offset_a':[(0,PIVOTS[0]),(1,PIVOTS[0])],
                  'offset_b':[(0,PIVOTS[1]),(1,PIVOTS[1])],
                  'variants_together':[(0,PIVOTS[0]),(1,PIVOTS[1])]}
    assert set(accepted['coverage']) == set(placements)
    for name, items in placements.items():
        samples, coverage = oracle_samples(items)
        assert accepted['coverage'][name] == coverage
        for offset,expected in samples.items():
            assert all(abs(a-b)<=3 for a,b in zip(pixels[name][offset:offset+4],expected)), 'independent numeric pixel oracle'
    pairs = {'no_pivot':'no_pivot','wrong_sign':'wrong_sign','wrong_order':'wrong_order',
             'wrong_uv':'wrong_uv','wrong_material':'wrong_material','distinct_pivots':'offset_b'}
    assert set(accepted['control_pixel_differences']) == set(pairs)
    for key,second in pairs.items():
        measured = difference(pixels['offset_a'],pixels[second])
        assert accepted['control_pixel_differences'][key] == measured and measured>200
    assets = accepted['assets']
    assert assets['source_hidden_after_install'] is True and assets['cooked_reload_before_render'] is True
    assert assets['installed_files'] == 2 and len(assets['variants']) == 2
    for field,file in [('original_glb','source.glb'),('source','source.model.json')]:
        assert assets[field]['file'] == file; capture_reference(captures,assets[field])
    for index,(entry,stem) in enumerate(zip(assets['variants'],('offset-a','offset-b'))):
        asset_id = f'variants/{stem}.pivot.json'
        assert entry['asset_id'] == asset_id
        for field,suffix in [('recipe','.pivot.json'),('cooked','.model.json')]:
            assert entry[field]['file'] == stem+suffix; capture_reference(captures,entry[field])
        check_variant(captures,'source.model.json',stem+'.pivot.json',stem+'.model.json',asset_id)
        assert json.loads((captures/(stem+'.pivot.json')).read_text())['pivot'] == list(PIVOTS[index])
    runner.report['captures'] = inventory


def gpu():
    runner.software()
    build_cli()
    captures = runner.evidence / 'captures'
    assert not captures.exists()
    os.environ['ORR_STATIC_MODEL_PIVOT_CAPTURE_DIR'] = str(captures)
    flags = cargo_flags('orr_render')
    output = run('build-render-test', ['cargo', 'test', *flags, '--features', 'models,imported-scene',
                                     '--test', 'static_model_pivot', '--no-run', '--message-format=json'])
    binary = pin(output, 'render-test', 'orr_render', 'static_model_pivot', 'test', ['models', 'imported-scene'])
    run('strict-render', ['cargo', 'clippy', *flags, '--features', 'models,imported-scene',
                         '--test', 'static_model_pivot', '--', '-D', 'warnings'])
    test('render', binary, [GPU_TEST], ignored=True)
    check_gpu(captures)
    runner.report['coverage']['gpu'] = 'PASSED'


def check_export(captures):
    names = {'acceptance-source.json', 'production-receipt.json', 'capture-inventory.json',
             'orr.export.json', 'authored-project', 'scene.glb', 'authoring-source.model.json',
             'control-offscreen.png', 'control-original.png'}
    names.update(color + suffix for color in ('red', 'blue') for suffix in ('.model.json', '.pivot.json'))
    stages = [('initial', 0, ''), ('moved', 18, 'right'), ('collision', 180, 'right'), ('interacted', 18, 'right,interact')]
    for name, _, _ in stages:
        for route in ('source', 'export'):
            names.update((f'runtime-{name}-{route}.png', f'prepared-project-{name}-{route}.frame'))
    inventory = capture_files(captures, names)
    del inventory['capture-inventory.json']
    assert json.loads((captures / 'capture-inventory.json').read_text()) == inventory
    pixels = {path.name: png_pixels(path) for path in
              list(captures.glob('runtime-*.png')) + list(captures.glob('control-*.png'))}
    assert len(pixels) == 10
    accepted = json.loads((captures / 'production-receipt.json').read_text())
    assert accepted['source_hidden'] is True and accepted['read_only_bundle'] is True
    assert accepted['rejection_before_output_preserved'] is True
    assert accepted['limits'] == 'explicit offline pivot authoring only; unchanged Room simulation/runtime/exporter; no automatic centering, transform authoring, glTF or collision claims'
    assert accepted['visible_changed_pixels'] > 20 and accepted['pivot_changed_pixels'] > 20
    for field, label in [('runtime_sha256', 'room_escape'), ('exporter_sha256', 'orr_export_room'),
                         ('pivot_cli_sha256', 'pivot-cli'), ('acceptance_executable_sha256', 'export-test')]:
        assert accepted[field] == runner.report['producers'][label]['sha256']
    assert set(accepted['observations']) == {name for name, _, _ in stages}
    for name, ticks, held in stages:
        item = accepted['observations'][name]
        assert item['ticks'] == ticks and item['held'] == held
        assert item['source_stdout'] == item['export_stdout']
        assert 'room capture adapter:' in item['source_stdout']
        assert f"room tick: {ticks} checksum: {item['checksum']}" in item['source_stdout']
        if name == 'interacted':
            assert 'room key: 1 won: 0' in item['source_stdout']
        for suffix, stem in [('png', 'runtime'), ('frame', 'prepared-project')]:
            first = captures / f'{stem}-{name}-source.{suffix}'
            second = captures / f'{stem}-{name}-export.{suffix}'
            assert first.stat().st_size > 0 and first.read_bytes() == second.read_bytes()
            assert item[f'source_{suffix}_sha256'] == runner.digest(first)
            assert item[f'export_{suffix}_sha256'] == runner.digest(second)
        if name != 'initial':
            assert difference(pixels['runtime-initial-source.png'], pixels[f'runtime-{name}-source.png'], tolerance=0) > 20
    assert len(accepted['control_stdout']) == 2
    for output in accepted['control_stdout']:
        assert 'room capture adapter:' in output
        assert f"room tick: 0 checksum: {accepted['observations']['initial']['checksum']}" in output
    initial = pixels['runtime-initial-source.png']
    assert accepted['visible_changed_pixels'] == difference(initial, pixels['control-offscreen.png'], tolerance=0)
    assert accepted['pivot_changed_pixels'] == difference(initial, pixels['control-original.png'], tolerance=0)
    source = json.loads((captures / 'acceptance-source.json').read_text())
    assert source['test'] == EXPORT_TEST
    for field, path in [('test_source_sha256', 'crates/orr_sample/tests/static_model_pivot_export.rs'),
                        ('cli_source_sha256', 'crates/orr_model/examples/static_model_pivot.rs'),
                        ('authoring_source_sha256', 'crates/orr_model/examples/static_model_pivot/authoring.rs')]:
        assert source[field] == runner.digest(source_root / path)
    assert source['source_asset_id'] == 'models/scene.glb' and source['source_path'] == 'inputs/source.model.json'
    assert source['glb_sha256'] == runner.digest(captures / 'scene.glb')
    assert source['source_cooked_sha256'] == runner.digest(captures / 'authoring-source.model.json')
    assert source['original_source_exported'] is False
    assert source['initial_frame_sha256'] == runner.digest(captures / 'prepared-project-initial-source.frame')
    assert len(source['variants']) == 2
    for entry, color, pivot in zip(source['variants'], ('red', 'blue'), ([1.0,-0.5,0.25],[-1.0,0.75,0.0])):
        asset_id = f'variants/{color}.pivot.json'
        assert entry['name'] == color and entry['asset_id'] == asset_id and entry['pivot'] == pivot
        for field,suffix in [('recipe_sha256','.pivot.json'),('cooked_sha256','.model.json')]:
            assert entry[field] == runner.digest(captures/(color+suffix))
        check_variant(captures,'authoring-source.model.json',color+'.pivot.json',color+'.model.json',asset_id)
        assert json.loads((captures/(color+'.pivot.json')).read_text())['pivot'] == pivot
    manifest = json.loads((captures / 'orr.export.json').read_text())
    payload = manifest['payload']
    assert manifest['schema'] == 1 and payload['profile'] == 'room-escape-authored-linux-x86_64-v1'
    assert payload['runtime']['sha256'] == runner.report['producers']['room_escape']['sha256']
    assert set(payload['packages']) == {'pivoted-static-props'}
    package = payload['packages']['pivoted-static-props']
    assert re.fullmatch('[0-9a-f]{64}', package)
    prefix = f'project/.orr/packages/objects/{package}/'
    project_files = {'project/' + name for name in ('orr.project.json', 'room.scene.yaml', 'room.models.json',
                     'room.camera.json', 'orr.packages.lock.json')}
    project_files.update(prefix + name for name in ('orr.package.json', 'red.model.json', 'blue.model.json'))
    exported = accepted['exported_file_inventory']
    assert set(exported) == project_files | {'bin/room_escape', 'run-room-escape', 'orr.export.json'}
    files = {entry['path']: [entry['bytes'], entry['sha256']] for entry in payload['files']}
    assert len(files) == len(payload['files']) == 10
    assert files == {name: value for name, value in exported.items() if name != 'orr.export.json'}
    assert exported['orr.export.json'] == [len((captures / 'orr.export.json').read_bytes()), runner.digest(captures / 'orr.export.json')]
    for name in project_files:
        actual = captures / 'authored-project' / name[len('project/'):]
        assert [actual.stat().st_size, runner.digest(actual)] == exported[name]
    for color in ('red', 'blue'):
        assert (captures / 'authored-project' / (prefix + color + '.model.json')[len('project/'):]).read_bytes() == (captures / (color + '.model.json')).read_bytes()
    assert not any(name.endswith(('.glb', '.png', '.pivot.json', 'original.model.json', 'source.model.json'))
                   for name in exported)
    runner.report['captures'] = inventory


def export():
    assert sys.platform == 'linux' and os.uname().machine == 'x86_64' and os.getuid() != 0
    assert not runner.evidence.is_relative_to(source_root.parent), 'evidence must survive workspace hiding'
    runner.software()
    run('namespace-preflight', ['bwrap', '--die-with-parent', '--ro-bind', '/', '/', '--', '/bin/true'])
    temporary = runner.evidence / 'temporary'; temporary.mkdir()
    os.environ.update(TMPDIR=str(temporary), ORR_REQUIRE_PROJECT_ISOLATION='1')
    build_cli()
    flags = [*cargo_flags('orr_sample'), '--features', 'room-project,project-export']
    features = ['project', 'project-export', 'room-project', 'sprites']
    for target, variable in [('room_escape', 'ORR_ROOM_RUNTIME'), ('orr_export_room', 'ORR_ROOM_EXPORTER')]:
        output = run('build-' + target, ['cargo', 'build', *flags, '--bin', target, '--message-format=json'])
        os.environ[variable] = pin(output, target, 'orr_sample', target, 'bin', features)
    output = run('build-export-test', ['cargo', 'test', *flags, '--test',
                                     'static_model_pivot_export', '--no-run', '--message-format=json'])
    binary = pin(output, 'export-test', 'orr_sample', 'static_model_pivot_export', 'test', features)
    run('strict-export', ['cargo', 'clippy', *flags, '--test', 'static_model_pivot_export', '--', '-D', 'warnings'])
    captures = runner.evidence / 'captures'; captures.mkdir()
    os.environ['ORR_STATIC_MODEL_PIVOT_EVIDENCE'] = str(captures)
    test('source-hidden-export', binary, [EXPORT_TEST], ignored=True)
    check_export(captures)
    runner.report['coverage']['export'] = 'PASSED'


def check_coverage():
    expected = {'cpu': {'model', 'bindings'}, 'gpu': {'gpu'}, 'export': {'export'}}[mode]
    assert runner.report['coverage'] == {name: ('PASSED' if name in expected else 'NOT RUN')
                                         for name in ('model', 'bindings', 'gpu', 'export')}


def finish(error):
    try:
        assert frozen is not None, 'prebuild source snapshot missing'
        source_fence()
        runner.fence()
        assert run('clean-source-after', ['git', 'status', '--porcelain=v1', '--untracked-files=all']).strip() == ''
        for key, revision in [('head', 'HEAD'), ('tree', 'HEAD^{tree}')]:
            assert run('checkout-' + key + '-after', ['git', 'rev-parse', revision]).strip() == runner.report[key]
        tracked = run('tracked-source-after', ['git', 'ls-files', '-z']).split('\0')
        assert tracked.pop() == ''
        check_source_inventory(tracked, 'after')
        assert sorted(tracked) == list(frozen)
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


def main():
    assert len(sys.argv) == 2 and sys.argv[1] in {'cpu', 'gpu', 'export'}, (
        'usage: static_model_pivot_gate.py cpu|gpu|export')
    initialize(pathlib.Path(__file__).resolve().parents[1],
               pathlib.Path(os.environ['ORR_STATIC_PIVOT_EVIDENCE']), sys.argv[1])
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
