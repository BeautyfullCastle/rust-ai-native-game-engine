#!/usr/bin/env python3
"""Mandatory current-source UV-variant acceptance; standard library only.

Hosted cpu/gpu/export each compile independent, retained producers in a fresh
target. Separate local model-only acceptance cannot certify bindings, GPU
pixels, or source-hidden production export. The reviewed static
prop runner supplies bounded output, process-tree cleanup and producer identity;
its source and independent fixtures are part of every prebuild source fence.
"""
import hashlib
import functools
import math
import json
import os
import pathlib
import re
import sys
import traceback
import struct
import zlib


def python_runtime():
    return {'optimize': sys.flags.optimize, '__debug__': __debug__,
            'PYTHONOPTIMIZE': os.environ.get('PYTHONOPTIMIZE'),
            'ignore_environment': sys.flags.ignore_environment,
            'dont_write_bytecode': sys.dont_write_bytecode,
            'executable': sys.executable}


def require_unoptimized_python():
    """Reject before assertion-based admission, even when Python removes asserts."""
    observed = python_runtime()
    # Check the inherited environment as well: -E/-I can hide optimization from
    # this interpreter while a later Python child still inherits the setting.
    if (observed['optimize'] != 0 or not observed['__debug__']
            or observed['PYTHONOPTIMIZE'] not in (None, '', '0')):
        print('STATIC_UV_GATE_PYTHON_REJECTED ' + json.dumps(observed, sort_keys=True),
              file=sys.stderr, flush=True)
        raise SystemExit(2)
    return observed


# This call intentionally precedes the assertion-based lifecycle import and all
# source/evidence/target operations. Rejection leaves only the caller's stderr;
# it must not manufacture a gate receipt or suggest that acceptance began.
require_unoptimized_python()

import static_model_extract_gate as runner

FEATURE_PATHS = (
    'crates/orr_model/examples/static_model_uv.rs',
    'crates/orr_model/examples/static_model_uv/authoring.rs',
    'crates/orr_model/tests/static_model_uv.rs',
    'crates/orr_model_bindings/tests/static_model_uv.rs',
    'crates/orr_render/tests/static_model_uv.rs',
    'crates/orr_sample/tests/static_model_uv_export.rs',
    'docs/static-model-uv.md',
    'tools/static_model_uv_gate.py',
    'tools/test_static_model_uv_gate.py',
    '.github/workflows/determinism.yml',
)
INFRA_PATHS = ('Cargo.toml', 'Cargo.lock', 'tools/static_model_extract_gate.py',
               'tools/test_static_model_extract_gate.py')
BINDINGS_TEST = 'installed_static_uv_variants_keep_distinct_bindings_and_source_geometry'
GPU_TEST = 'static_uv_variants_match_independent_gpu_oracle'
EXPORT_TEST = 'static_model_uv_real_cli_source_hidden_room_export'
FIXTURE_COUNT = 25
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
    assert len(FEATURE_PATHS) == len(set(FEATURE_PATHS)) == 10
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
    require_unoptimized_python()
    source_fence()
    output = runner.run(label, command, expected=expected, seconds=seconds)
    source_fence()
    return output


def initialize(root, destination, phase):
    global source_root, frozen, mode
    runtime = require_unoptimized_python()
    assert phase in {'cpu', 'gpu', 'export'}
    source_root = pathlib.Path(root).resolve()
    frozen = None
    mode = phase
    destination = pathlib.Path(destination)
    assert not destination.is_relative_to(source_root), 'evidence must be outside source'
    runner.initialize(source_root, destination, phase)
    runner.report['python_runtime'] = runtime
    runner.report['mode'] = phase
    runner.report['coverage'] = {name: 'NOT RUN' for name in ('model', 'bindings', 'gpu', 'export')}
    runner.report['local_model_only_cannot_certify_hosted_acceptance'] = True
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
    require_unoptimized_python()
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
        ('test_static_model_uv_gate.py', f'STATIC_UV_GATE_FIXTURES: {FIXTURE_COUNT} passed; 0 failed; 0 skipped'),
    ]:
        output = run('checker-' + file[:-3], [sys.executable, '-B', str(source_root / 'tools' / file)])
        assert output.rstrip().endswith(marker), 'independent gate fixture census'


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
                              '--example', 'static_model_uv', '--message-format=json'])
    binary = pin(output, 'uv-cli', 'orr_model', 'static_model_uv', 'example', ['import'])
    os.environ['ORR_STATIC_MODEL_UV_BIN'] = binary
    return binary


def model_tests(platform):
    # A fixed explicit census is compared with actual Rust source by fixtures.
    assert platform in {'linux', 'darwin', 'win32'}, 'unsupported model test platform'
    names = '''all_turns_at_exact_uv_limit_are_accepted_without_clamping
cli_bad_arguments_fail_without_output_or_staging
cli_help_and_three_unique_pairs_are_bounded
cli_missing_nonregular_and_oversized_inputs_leave_no_output
cli_parent_traversal_preserves_raw_spelling_and_never_publishes
cli_portable_output_names_and_package_store_rejection
cli_publishes_once_without_overwrite_and_preserves_all_inputs
cli_resolver_is_anchored_to_recipe_parent
cooked_roundtrip_is_byte_deterministic_and_recipe_ids_remain_distinct
dependencies_bind_exact_recipe_and_source_without_chasing_old_uris
final_overflow_and_one_ulp_beyond_both_limits_reject_without_clamping
finite_intermediates_above_uv_limit_may_return_inside_final_bounds
identity_preserves_all_uv_signed_zero_bits
paths_and_identity_are_normalized_portable_and_rejected_before_resolution
portable_logical_paths_accept_the_240_byte_boundary
recipe_accepts_exact_required_object_and_whitespace
recipe_accepts_positive_f32_extremes_and_finite_offsets
recipe_and_source_combined_budget_boundary_is_enforced
recipe_rejects_format_version_types_and_byte_budget_before_resolution
recipe_rejects_nonfinite_nonpositive_scale_and_quarter_turn_boundaries_before_resolution
recipe_rejects_unknown_duplicate_missing_null_and_sequence_fields_before_resolution
resolution_is_exactly_once_and_failures_are_preserved
scale_intermediate_infinity_rejects_before_offset_cancellation
scale_then_origin_quarter_turn_then_offset_matches_hand_computed_coordinates
selection_requires_existing_used_material
source_is_fully_admitted_before_material_selection
transform_preserves_all_unrelated_bits_shared_images_and_multiple_primitive_users
transformed_cooked_output_budget_is_enforced'''.split()
    if platform == 'win32':
        names.append('cli_windows_nonlocal_prefixes_reject')
    else:
        names.extend(['cli_symlink_inputs_ancestors_and_destinations_reject',
                      'cli_non_utf8_option_names_and_logical_ids_reject'])
        if platform == 'linux':
            names.append('cli_non_utf8_filesystem_inputs_work')
    assert len(names) == {'linux': 31, 'darwin': 30, 'win32': 29}[platform]
    assert len(names) == len(set(names))
    return names


def model():
    flags = cargo_flags()
    output = run('build-feature-off', ['cargo', 'build', *flags, '--example',
                                      'static_model_uv', '--message-format=json'])
    disabled = pin(output, 'feature-off-cli', 'orr_model', 'static_model_uv', 'example', [])
    assert 'requires --features import' in run('feature-off-exit2', [disabled, '--help'], expected=2)
    cli = build_cli()
    assert 'Usage: static_model_uv' in run('actual-help', [cli, '--help'])
    output = run('build-model-test', ['cargo', 'test', *flags, '--features', 'import',
                                    '--test', 'static_model_uv', '--no-run', '--message-format=json'])
    binary = pin(output, 'model-test', 'orr_model', 'static_model_uv', 'test', ['import'])
    test('model', binary, model_tests(sys.platform))
    # Target only model library and the new source-bound fixture. Other workspace
    # acceptance (including existing socket fixtures) has its own required gates.
    run('model-library-regressions', ['cargo', 'test', *flags, '--features', 'import', '--lib'])
    run('strict-model-import', ['cargo', 'clippy', *flags, '--features', 'import',
                               '--lib', '--example', 'static_model_uv', '--test',
                               'static_model_uv', '--', '-D', 'warnings'])
    run('strict-model-default', ['cargo', 'clippy', *flags, '--lib', '--example',
                                'static_model_uv', '--', '-D', 'warnings'])
    runner.report['coverage']['model'] = 'PASSED'
    runner.record()


def cpu():
    model()
    flags = cargo_flags('orr_model_bindings')
    output = run('build-bindings-test', ['cargo', 'test', *flags, '--test',
                                       'static_model_uv', '--no-run', '--message-format=json'])
    binary = pin(output, 'bindings-test', 'orr_model_bindings', 'static_model_uv', 'test', [])
    test('bindings', binary, [BINDINGS_TEST], ignored=True)
    run('strict-bindings', ['cargo', 'clippy', *flags, '--test', 'static_model_uv', '--', '-D', 'warnings'])
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
            assert len(files) <= 192, 'bounded capture inventory'
    assert sum(path.stat().st_size for path in files) <= 512 * 1024**2, 'bounded total capture bytes'
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


def strict_json(path):
    def pairs(values):
        result = {}
        for key, value in values:
            assert key not in result, 'duplicate receipt or capture object field'
            result[key] = value
        return result
    def reject(value):
        raise AssertionError('nonfinite JSON number: ' + value)
    value = json.loads(path.read_text(encoding='utf-8'), object_pairs_hook=pairs, parse_constant=reject)
    def finite(node):
        if isinstance(node, dict):
            for child in node.values():
                finite(child)
        elif isinstance(node, list):
            for child in node:
                finite(child)
        elif isinstance(node, float):
            assert math.isfinite(node), 'nonfinite JSON exponent'
    finite(value)
    return value


def f32(value):
    assert type(value) in {int, float} and math.isfinite(value)
    rounded = struct.unpack('<f', struct.pack('<f', value))[0]
    assert math.isfinite(rounded), 'nonfinite f32 intermediate'
    return rounded


def bit_equal(first, second):
    """Compare presentation numbers as stored f32s, including signed zero."""
    if type(first) is float and type(second) is float:
        return struct.pack('<f', f32(first)) == struct.pack('<f', f32(second))
    if type(first) is not type(second):
        return False
    if isinstance(first, dict):
        return set(first) == set(second) and all(bit_equal(first[key], second[key]) for key in first)
    if isinstance(first, list):
        return len(first) == len(second) and all(bit_equal(a, b) for a, b in zip(first, second))
    return first == second


def transformed_uv(uv, recipe):
    assert len(recipe['scale']) == len(recipe['offset']) == len(uv) == 2
    scale, offset = [f32(value) for value in recipe['scale']], [f32(value) for value in recipe['offset']]
    turns = recipe['quarter_turns']
    assert all(value > 0 for value in scale), 'strictly positive admitted f32 scales'
    assert type(turns) is int and 0 <= turns <= 3
    uv = [f32(value) for value in uv]
    assert all(abs(value) <= 65536 for value in uv), 'admitted source UV bounds'
    if scale == [1, 1] and turns == 0 and offset == [0, 0]:
        return uv[:]
    x, y = (f32(f32(value) * f32(factor)) for value, factor in zip(uv, scale))
    rotated = ((x, y), (-y, x), (-x, -y), (y, -x))[turns]
    result = [f32(value + f32(shift)) for value, shift in zip(rotated, offset)]
    assert all(abs(value) <= 65536 for value in result), 'UV storage bound'
    return result


def check_variant(captures, source_file, recipe_file, cooked_file, asset_id):
    source = strict_json(captures / source_file)
    recipe = strict_json(captures / recipe_file)
    cooked = strict_json(captures / cooked_file)
    assert (captures / recipe_file).stat().st_size <= 16 * 1024
    assert (captures / recipe_file).stat().st_size + (captures / source_file).stat().st_size <= 64 * 1024**2
    assert set(recipe) == {'format', 'version', 'source', 'material', 'offset', 'scale', 'quarter_turns'}
    assert recipe['format'] == 'orr_static_material_uv' and type(recipe['version']) is int and recipe['version'] == 1
    assert type(recipe['material']) is int and recipe['material'] == 0
    assert type(recipe['quarter_turns']) is int and 0 <= recipe['quarter_turns'] <= 3
    for field in ('offset', 'scale'):
        assert isinstance(recipe[field], list) and len(recipe[field]) == 2
        for value in recipe[field]:
            assert math.isfinite(f32(value)) and (field != 'scale' or f32(value) > 0)
    assert isinstance(recipe['source'], str) and recipe['source'] == 'inputs/source.model.json'
    assert asset_id in {'variants/rotate.uv.json', 'variants/counter.uv.json'}
    offset, scale, turns = (([0.25, 0.125], [1.5, 0.5], 1) if asset_id == 'variants/rotate.uv.json'
                            else ([-0.375, 0.75], [0.5, 1.5], 3))
    assert recipe['offset'] == offset and recipe['scale'] == scale and recipe['quarter_turns'] == turns
    assert set(cooked) == set(source) == {'format', 'version', 'asset_id', 'dependencies', 'primitives', 'materials', 'images'}
    assert cooked['format'] == source['format'] == 'orr_static_model'
    assert type(cooked['version']) is int and cooked['version'] == source['version'] == 1
    assert cooked['asset_id'] == asset_id and asset_id.endswith('.uv.json')
    assert cooked['dependencies'] == [
        {'uri': '$source', 'sha256': runner.digest(captures / recipe_file)},
        {'uri': recipe['source'], 'sha256': runner.digest(captures / source_file)},
    ], 'exact ordered recipe and cooked source byte provenance'
    assert len(cooked['primitives']) == len(source['primitives'])
    for original, variant in zip(source['primitives'], cooked['primitives']):
        assert original['id'].startswith(source['asset_id'] + '#node=')
        expected = dict(original, id=asset_id + original['id'][len(source['asset_id']):])
        assert original['vertices'], 'nonvacuous UV vertex contract'
        if original['material'] == recipe['material']:
            expected['vertices'] = [dict(vertex, uv=transformed_uv(vertex['uv'], recipe))
                                    for vertex in original['vertices']]
        assert bit_equal(variant, expected), 'only selected UVs and source namespace may change'
    assert bit_equal(cooked['materials'], source['materials']), 'all material fields and slots preserved'
    assert cooked['images'] == source['images'], 'all source image slots and bytes preserved'
    assert sum(item['material'] == 0 for item in source['primitives']) >= 2, 'shared material users'
    assert any(item['material'] != 0 for item in source['primitives']), 'unaffected shared-image material control'
    assert source['materials'][0]['image'] == source['materials'][1]['image'], 'shared source image control'
    return cooked


def difference(first, second, tolerance=3):
    assert len(first) == len(second) and len(first) % 4 == 0
    return sum(any(abs(first[index + channel] - second[index + channel]) > tolerance for channel in range(4))
               for index in range(0, len(first), 4))


def marker_offsets():
    """Independent interior mask for the two fixture marker triangles."""
    a, b, c = (1.1, -0.75), (1.8, -0.75), (1.4, 0.4)
    denominator = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1])
    selected = []
    for y in range(256):
        for x in range(256):
            world = ((x + 0.5 - 128) / 32, (128 - y - 0.5) / 32)
            for sign, center in ((-1, 1.25), (1, -1.25)):
                local = (world[0] / (0.75 * sign), (world[1] - center) / 0.75)
                u = ((b[1] - c[1]) * (local[0] - c[0]) + (c[0] - b[0]) * (local[1] - c[1])) / denominator
                v = ((c[1] - a[1]) * (local[0] - c[0]) + (a[0] - c[0]) * (local[1] - c[1])) / denominator
                if min(u, v, 1 - u - v) > 0.08:
                    selected.append((y * 256 + x) * 4)
    assert len(selected) > 100 and len(selected) == len(set(selected))
    return selected


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


def fnv1a64(data):
    value = 0xcbf29ce484222325
    for byte in data:
        value = ((value ^ byte) * 0x100000001b3) & 0xffffffffffffffff
    return f'{value:016x}'


def uv_sample(uv, linear, wrap):
    """Independent normalized 2x2 sampler; interpolation is in linear light."""
    colors = ((255, 0, 0, 255), (0, 255, 0, 255), (0, 0, 255, 255), (128, 128, 128, 255))
    def address(value):
        return max(0, min(1, value)) if wrap == 'clamp' else (value % 2 if wrap == 'repeat' else (0, 1, 1, 0)[value % 4])
    assert wrap in {'repeat', 'mirror', 'clamp'}
    scaled = [value * 2 for value in uv]
    slot = address(math.floor(scaled[1])) * 2 + address(math.floor(scaled[0]))
    if not linear:
        if any(abs(value - round(value)) < .02 for value in scaled):
            return None
        return colors[slot], slot
    positions = [value - .5 for value in scaled]
    base = [math.floor(value) for value in positions]
    fractions = [value - origin for value, origin in zip(positions, base)]
    out = []
    for channel in range(3):
        value = 0.0
        for dy in range(2):
            for dx in range(2):
                color = colors[address(base[1] + dy) * 2 + address(base[0] + dx)][channel] / 255
                decoded = color / 12.92 if color <= .04045 else ((color + .055) / 1.055) ** 2.4
                weight = (fractions[0] if dx else 1 - fractions[0]) * (fractions[1] if dy else 1 - fractions[1])
                value += weight * decoded
        encoded = 12.92 * value if value <= .0031308 else 1.055 * value ** (1 / 2.4) - .055
        out.append(math.floor(max(0, min(1, encoded)) * 255 + .5))
    return (*out, 255), slot


@functools.lru_cache(maxsize=2)
def oracle_geometry(centers):
    # Constants are independent from captured geometry, compiled model helpers,
    # production camera matrices, image bytes and all reported coverage claims.
    points = ((-1, -1), (1, -1), (1, .4), (.4, 1), (-1, 1), (1.1, -.75), (1.8, -.75), (1.4, .4))
    triangles = ((0, 1, 2), (0, 2, 3), (0, 3, 4), (5, 6, 7))
    records = []
    for y in range(256):
        for x in range(256):
            world = ((x + .5 - 128) / 32, (128 - y - .5) / 32)
            near, inside = False, None
            for model, center in enumerate(centers):
                for node, (sign, center_y) in enumerate(((-1, 1.25), (1, -1.25))):
                    local = ((world[0] - center) / (.75 * sign), (world[1] - center_y) / .75)
                    for triangle, ids in enumerate(triangles):
                        a, b, c = (points[index] for index in ids)
                        denominator = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1])
                        u = ((b[1] - c[1]) * (local[0] - c[0]) + (c[0] - b[0]) * (local[1] - c[1])) / denominator
                        v = ((c[1] - a[1]) * (local[0] - c[0]) + (a[0] - c[0]) * (local[1] - c[1])) / denominator
                        weights = (u, v, 1 - u - v)
                        near |= min(weights) >= -.08
                        if min(weights) > .08:
                            assert inside is None, 'nonoverlapping oracle geometry'
                            inside = (model, node, triangle == 3, (local[0] + 1) * .5, (1 - local[1]) * .5)
            offset = (y * 256 + x) * 4
            if inside is not None:
                records.append((offset, inside))
            elif not near:
                records.append((offset, None))
    return records


def oracle_pixels(placements, linear, wrap):
    selected = [[0, 0] for _ in placements]
    markers = [[0, 0] for _ in placements]
    quadrants = [0] * 4
    clear, pixels = 0, []
    for offset, interior in oracle_geometry(tuple(center for center, _ in placements)):
        if interior is None:
            clear += 1; pixels.append((offset, (0, 0, 0, 255))); continue
        model, node, marker, u, v = interior
        mode = placements[model][1]
        uv = (u, v) if marker or mode == 0 else ((.25 - .5 * v, .125 + 1.5 * u) if mode == 1 else (-.375 + 1.5 * v, .75 - .5 * u))
        sampled = uv_sample(uv, linear, wrap)
        if sampled is None:
            continue
        color, quadrant = sampled
        (markers if marker else selected)[model][node] += 1
        if not marker and mode == 0:
            quadrants[quadrant] += 1
        pixels.append((offset, color))
    assert all(value > 500 for pair in selected for value in pair)
    assert all(value > 40 for pair in markers for value in pair)
    assert clear > 40000
    if any(mode == 0 for _, mode in placements):
        assert all(value > 100 for value in quadrants)
    return pixels, {'selected_pixels_per_model_node': selected, 'unaffected_pixels_per_model_node': markers,
                    'source_checker_pixels': quadrants, 'clear_checked': clear}


GPU_FRAMES = ('source', 'rotate', 'counter', 'variants_together', 'variants_reversed',
              'wrong_order', 'all_material', 'only_first_user', 'wrong_transform')
GPU_CASES = tuple(f'{filter_}-{wrap}' for filter_ in ('nearest', 'linear') for wrap in ('repeat', 'mirror', 'clamp'))
CONTROL_PAIRS = {'unchanged_to_rotate': ['source', 'rotate'], 'unchanged_to_counter': ['source', 'counter'],
                 'distinct_variants': ['rotate', 'counter'], 'wrong_order': ['wrong_order', 'rotate'],
                 'all_material': ['all_material', 'rotate'], 'shared_material_user': ['only_first_user', 'rotate'],
                 'world_transform': ['wrong_transform', 'rotate']}


def check_gpu(captures):
    names = {'source.glb', 'adapter.json', 'receipt.json'}
    for case in GPU_CASES:
        names.add(case + '-source.model.json')
        names.update(f'{case}-{name}{suffix}' for name in ('rotate', 'counter') for suffix in ('.uv.json', '.model.json'))
        names.update(f'{case}-{name.replace("_", "-")}{suffix}' for name in GPU_FRAMES for suffix in ('.rgba', '.ppm'))
    assert len(names) == 141
    inventory = capture_files(captures, names)
    accepted, adapter = strict_json(captures / 'receipt.json'), strict_json(captures / 'adapter.json')
    assert accepted['schema'] == 1 and accepted['status'] == 'passed'
    assert accepted['width'] == accepted['height'] == 256
    assert accepted['adapter'] == adapter and adapter['software'] is True and adapter['backend'] == 'Vulkan'
    assert accepted['tolerances'] == {'color_bytes': 3, 'minimum_control_differences': 500, 'minimum_unaffected_differences': 30}
    for field, path in [('test_source_fnv1a64', source_root / 'crates/orr_render/tests/static_model_uv.rs'),
                        ('cli_source_fnv1a64', source_root / 'crates/orr_model/examples/static_model_uv.rs'),
                        ('authoring_source_fnv1a64', source_root / 'crates/orr_model/examples/static_model_uv/authoring.rs'),
                        ('producer_fnv1a64', runner.evidence / runner.report['producers']['uv-cli']['file'])]:
        assert accepted[field] == fnv1a64(path.read_bytes()), 'GPU source and producer identity'
    assert (captures / 'source.glb').read_bytes() == (source_root / 'crates/orr_model/tests/fixtures/model.glb').read_bytes()
    assert set(accepted['cases']) == set(GPU_CASES)
    markers = marker_offsets()
    for case in GPU_CASES:
        item = accepted['cases'][case]
        filter_, wrap = case.split('-'); linear = filter_ == 'linear'
        assert item['linear_filter'] is linear and item['wrap_s'] == item['wrap_t'] == wrap
        for field in ('shared_material_users_checked', 'unaffected_material_byte_identical',
                      'same_image_unselected_material', 'distinct_recipe_namespaces',
                      'reversed_batch_order_byte_identical', 'opaque_alpha_checked'):
            assert item[field] is True
        assert item['unaffected_material_pixels_checked'] == len(markers)
        frames, pixels = item['frames'], {}
        assert set(frames) == set(GPU_FRAMES)
        for name in GPU_FRAMES:
            frame, stem = frames[name], f'{case}-{name.replace("_", "-")}'
            assert set(frame) == {'raw', 'image'}
            assert frame['raw']['file'] == stem + '.rgba' and frame['image']['file'] == stem + '.ppm'
            for entry in frame.values():
                capture_reference(captures, entry)
            pixels[name] = (captures / (stem + '.rgba')).read_bytes()
            assert len(pixels[name]) == 256 * 256 * 4 and pixels[name][3::4] == b'\xff' * 65536
            ppm = b'P6\n256 256\n255\n' + bytes(value for index, value in enumerate(pixels[name]) if index % 4 != 3)
            assert (captures / (stem + '.ppm')).read_bytes() == ppm
        assert pixels['variants_together'] == pixels['variants_reversed'], 'batch order changed pixels'
        for offset in markers:
            assert pixels['source'][offset:offset + 4] == pixels['rotate'][offset:offset + 4] == pixels['counter'][offset:offset + 4]
        assert item['control_pairs'] == CONTROL_PAIRS and set(item['control_pixel_differences']) == set(CONTROL_PAIRS)
        for control, (first, second) in CONTROL_PAIRS.items():
            measured = difference(pixels[first], pixels[second])
            assert item['control_pixel_differences'][control] == measured
            assert measured > (30 if control == 'all_material' else 500), 'nonvacuous independent output control'
        placements = {'source': ((0, 0),), 'rotate': ((0, 1),), 'counter': ((0, 2),),
                      'variants_together': ((-2, 1), (2, 2))}
        assert set(item['coverage']) == set(placements)
        for name, arrangement in placements.items():
            expected, coverage = oracle_pixels(arrangement, linear, wrap)
            assert item['coverage'][name] == coverage, 'independent exact coverage census'
            for offset, color in expected:
                assert all(abs(actual - wanted) <= 3 for actual, wanted in zip(pixels[name][offset:offset + 4], color)), 'independent UV pixel oracle'
        assets = item['assets']
        assert assets['source_hidden_after_install'] is True and assets['cooked_reload_before_render'] is True
        assert assets['installed_files'] == 2
        for field, file in [('original_glb', 'source.glb'), ('source', case + '-source.model.json')]:
            assert assets[field]['file'] == file; capture_reference(captures, assets[field])
        assert len(assets['variants']) == 2
        source = strict_json(captures / (case + '-source.model.json'))
        assert len(source['materials']) == len(source['images']) == 3 and len(source['primitives']) == 4
        assert source['images'][0] == {'width': 2, 'height': 2, 'rgba8': [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 128, 128, 128, 255]}
        for material in source['materials'][:2]:
            assert material['linear_filter'] is linear and material['wrap_s'] == material['wrap_t'] == wrap
            assert material['image'] == 0 and material['base_color'][:3] == [1, 1, 1]
        for entry, name in zip(assets['variants'], ('rotate', 'counter')):
            asset_id = f'variants/{name}.uv.json'
            assert entry['asset_id'] == asset_id and set(entry) == {'asset_id', 'recipe', 'cooked'}
            for field, suffix in [('recipe', '.uv.json'), ('cooked', '.model.json')]:
                assert entry[field]['file'] == case + '-' + name + suffix; capture_reference(captures, entry[field])
            check_variant(captures, case + '-source.model.json', case + '-' + name + '.uv.json', case + '-' + name + '.model.json', asset_id)
    runner.report['captures'] = inventory


def gpu():
    runner.software()
    build_cli()
    captures = runner.evidence / 'captures'
    assert not captures.exists()
    os.environ['ORR_STATIC_MODEL_UV_CAPTURE_DIR'] = str(captures)
    flags = cargo_flags('orr_render')
    output = run('build-render-test', ['cargo', 'test', *flags, '--features', 'models,imported-scene',
                                     '--test', 'static_model_uv', '--no-run', '--message-format=json'])
    binary = pin(output, 'render-test', 'orr_render', 'static_model_uv', 'test', ['models', 'imported-scene'])
    run('strict-render', ['cargo', 'clippy', *flags, '--features', 'models,imported-scene',
                         '--test', 'static_model_uv', '--', '-D', 'warnings'])
    test('render', binary, [GPU_TEST], ignored=True)
    check_gpu(captures)
    runner.report['coverage']['gpu'] = 'PASSED'


def project_inventory(root):
    files = {path.relative_to(root).as_posix(): [path.stat().st_size, runner.digest(path)]
             for path in root.rglob('*') if path.is_file()}
    expected_directories = {parent.as_posix() for name in files for parent in pathlib.PurePosixPath(name).parents if str(parent) != '.'}
    actual_directories = {path.relative_to(root).as_posix() for path in root.rglob('*') if path.is_dir()}
    assert actual_directories == expected_directories, 'unexpected empty capture directories'
    return files


def check_project(root, package_name, expected_assets):
    lock = strict_json(root / 'orr.packages.lock.json')
    assert set(lock) == {'schema', 'direct', 'packages'} and lock['schema'] == 1
    assert set(lock['packages']) == set(lock['direct']) == {package_name}
    package = lock['packages'][package_name]
    assert set(package) == {'manifest', 'digest', 'files'}
    manifest = package['manifest']
    assert set(manifest) == {'schema', 'name', 'version', 'engine', 'capabilities', 'dependencies', 'files'}
    assert manifest['schema'] == 1 and manifest['name'] == package_name and manifest['version'] == '1.0.0'
    assert manifest['engine'] == '*' and manifest['capabilities'] == ['models'] and manifest['dependencies'] == {}
    assert lock['direct'][package_name] == '1.0.0'
    assert manifest['files'] == sorted(expected_assets) and set(package['files']) == set(expected_assets)
    canonical = {key: manifest[key] for key in ('schema', 'name', 'version', 'engine', 'capabilities', 'dependencies', 'files')}
    hashes = dict(sorted(package['files'].items()))
    identity = hashlib.sha256(json.dumps(['orr-package-v1', canonical, hashes], separators=(',', ':'), ensure_ascii=False).encode()).hexdigest()
    assert package['digest'] == identity, 'independently recomputed package identity'
    directory = f'.orr/packages/objects/{identity}/'
    assert strict_json(root / directory / 'orr.package.json') == manifest
    required = {'orr.project.json', 'room.scene.yaml', 'room.models.json', 'room.camera.json', 'orr.packages.lock.json'}
    required.update(directory + name for name in ['orr.package.json', *expected_assets])
    inventory = project_inventory(root)
    assert set(inventory) == required, 'exact self-contained package project inventory'
    for name, expected in expected_assets.items():
        data = (root / directory / name).read_bytes()
        assert data == expected and package['files'][name] == hashlib.sha256(data).hexdigest()
    sidecar = strict_json(root / 'room.models.json')
    assert set(sidecar) == {'version', 'scene', 'project', 'bindings'}
    assert sidecar['version'] == 2 and sidecar['scene'] == 'room.scene.yaml' and sidecar['project'] == '.'
    assert len(sidecar['bindings']) == 2
    for key, binding in sidecar['bindings'].items():
        assert re.fullmatch(r'e_[0-9a-f]{8,32}', key), 'persistent scene GUID'
        assert set(binding) == {'kind', 'package', 'asset', 'package_digest', 'source_hash', 'transform'}
        assert binding['kind'] == 'static' and binding['package'] == package_name and binding['package_digest'] == identity
        assert binding['asset'] in expected_assets and binding['source_hash'] == package['files'][binding['asset']]
        assert set(binding['transform']) == {'translation', 'rotation', 'scale'}
        assert binding['transform']['rotation'] == [0, 0, 0, 1] and binding['transform']['scale'] == [1, 1, 1]
    return inventory, sidecar, identity


def check_control_projects(captures, accepted):
    models = {name + '.model.json': (captures / (name + '.model.json')).read_bytes() for name in ('rotate', 'counter')}
    authored = captures / 'authored-project'
    normal_inventory, normal, _ = check_project(authored, 'uv-static-props', models)
    offscreen_root, original_root = captures / 'control-offscreen-project', captures / 'control-original-project'
    offscreen_inventory, offscreen, _ = check_project(offscreen_root, 'uv-static-props', models)
    original_inventory, original, _ = check_project(original_root, 'original-uv-control',
                                                  {'original.model.json': (captures / 'authoring-source.model.json').read_bytes()})
    assert accepted['control_project_inventories'] == {'offscreen': offscreen_inventory, 'original': original_inventory}
    assert set(normal['bindings']) == set(offscreen['bindings']) == set(original['bindings'])
    assert {binding['asset'] for binding in normal['bindings'].values()} == set(models)
    for key, binding in normal['bindings'].items():
        expected_translation = [-2, 1, 0] if binding['asset'] == 'rotate.model.json' else [2, 1, 0]
        assert binding['transform']['translation'] == expected_translation
        moved = dict(binding, transform=dict(binding['transform'], translation=[1000, 1000, 1000]))
        assert offscreen['bindings'][key] == moved, 'offscreen control changes translation only'
        assert original['bindings'][key]['transform'] == binding['transform'], 'original control preserves visible transform'
        assert original['bindings'][key]['asset'] == 'original.model.json'
    for name in ('orr.project.json', 'room.scene.yaml', 'room.camera.json'):
        assert (authored / name).read_bytes() == (offscreen_root / name).read_bytes() == (original_root / name).read_bytes()
    assert {name for name in normal_inventory if name != 'room.models.json'} == {name for name in offscreen_inventory if name != 'room.models.json'}
    for name in normal_inventory:
        if name != 'room.models.json':
            assert normal_inventory[name] == offscreen_inventory[name]


def check_export(captures):
    names = {'acceptance-source.json', 'production-receipt.json', 'capture-inventory.json',
             'orr.export.json', 'authored-project', 'scene.glb', 'authoring-source.model.json',
             'control-offscreen.png', 'control-original.png', 'control-offscreen-project', 'control-original-project'}
    names.update(color + suffix for color in ('rotate', 'counter') for suffix in ('.model.json', '.uv.json'))
    stages = [('initial', 0, ''), ('moved', 18, 'right'), ('collision', 180, 'right'), ('interacted', 18, 'right,interact')]
    for name, _, _ in stages:
        for route in ('source', 'export'):
            names.update((f'runtime-{name}-{route}.png', f'prepared-project-{name}-{route}.frame'))
    inventory = capture_files(captures, names)
    del inventory['capture-inventory.json']
    assert strict_json(captures / 'capture-inventory.json') == inventory
    pixels = {path.name: png_pixels(path) for path in
              list(captures.glob('runtime-*.png')) + list(captures.glob('control-*.png'))}
    assert len(pixels) == 10
    accepted = strict_json(captures / 'production-receipt.json')
    assert accepted['source_hidden'] is True and accepted['read_only_bundle'] is True
    assert accepted['no_visible_models_control'] is True and accepted['offscreen_control_preserves_bindings'] is True
    assert accepted['visible_changed_pixels'] > 20 and accepted['uv_changed_pixels'] > 20
    for field, label in [('runtime_sha256', 'room_escape'), ('exporter_sha256', 'orr_export_room'),
                         ('uv_cli_sha256', 'uv-cli'), ('acceptance_executable_sha256', 'export-test')]:
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
    assert accepted['uv_changed_pixels'] == difference(initial, pixels['control-original.png'], tolerance=0)
    source = strict_json(captures / 'acceptance-source.json')
    assert source['test'] == EXPORT_TEST
    for field, path in [('test_source_sha256', 'crates/orr_sample/tests/static_model_uv_export.rs'),
                        ('cli_source_sha256', 'crates/orr_model/examples/static_model_uv.rs'),
                        ('authoring_source_sha256', 'crates/orr_model/examples/static_model_uv/authoring.rs')]:
        assert source[field] == runner.digest(source_root / path)
    assert source['source_asset_id'] == 'models/scene.glb' and source['source_path'] == 'inputs/source.model.json'
    assert source['glb_sha256'] == runner.digest(captures / 'scene.glb')
    assert source['source_cooked_sha256'] == runner.digest(captures / 'authoring-source.model.json')
    assert source['original_source_exported'] is False
    assert source['initial_frame_sha256'] == runner.digest(captures / 'prepared-project-initial-source.frame')
    assert len(source['variants']) == 2
    for entry, name in zip(source['variants'], ('rotate', 'counter')):
        asset_id = f'variants/{name}.uv.json'
        assert entry['name'] == name and entry['asset_id'] == asset_id and entry['material'] == 0
        recipe = strict_json(captures / (name + '.uv.json'))
        for field in ('offset', 'scale', 'quarter_turns'):
            assert entry[field] == recipe[field]
        for field, suffix in [('recipe_sha256', '.uv.json'), ('cooked_sha256', '.model.json')]:
            assert entry[field] == runner.digest(captures / (name + suffix))
        check_variant(captures, 'authoring-source.model.json', name + '.uv.json', name + '.model.json', asset_id)
    manifest = strict_json(captures / 'orr.export.json')
    payload = manifest['payload']
    assert manifest['schema'] == 1 and payload['profile'] == 'room-escape-authored-linux-x86_64-v1'
    assert payload['runtime']['sha256'] == runner.report['producers']['room_escape']['sha256']
    assert set(payload['packages']) == {'uv-static-props'}
    package = payload['packages']['uv-static-props']
    assert re.fullmatch('[0-9a-f]{64}', package)
    prefix = f'project/.orr/packages/objects/{package}/'
    project_files = {'project/' + name for name in ('orr.project.json', 'room.scene.yaml', 'room.models.json',
                     'room.camera.json', 'orr.packages.lock.json')}
    project_files.update(prefix + name for name in ('orr.package.json', 'rotate.model.json', 'counter.model.json'))
    exported = accepted['exported_file_inventory']
    assert set(exported) == project_files | {'bin/room_escape', 'run-room-escape', 'orr.export.json'}
    files = {entry['path']: [entry['bytes'], entry['sha256']] for entry in payload['files']}
    assert len(files) == len(payload['files']) == 10
    assert files == {name: value for name, value in exported.items() if name != 'orr.export.json'}
    assert exported['orr.export.json'] == [len((captures / 'orr.export.json').read_bytes()), runner.digest(captures / 'orr.export.json')]
    for name in project_files:
        actual = captures / 'authored-project' / name[len('project/'):]
        assert [actual.stat().st_size, runner.digest(actual)] == exported[name]
    for color in ('rotate', 'counter'):
        assert (captures / 'authored-project' / (prefix + color + '.model.json')[len('project/'):]).read_bytes() == (captures / (color + '.model.json')).read_bytes()
    assert not any(name.endswith(('.glb', '.png', '.uv.json', 'original.model.json', 'source.model.json'))
                   for name in exported)
    check_control_projects(captures, accepted)
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
                                     'static_model_uv_export', '--no-run', '--message-format=json'])
    binary = pin(output, 'export-test', 'orr_sample', 'static_model_uv_export', 'test', features)
    run('strict-export', ['cargo', 'clippy', *flags, '--test', 'static_model_uv_export', '--', '-D', 'warnings'])
    captures = runner.evidence / 'captures'; captures.mkdir()
    os.environ['ORR_STATIC_MODEL_UV_EVIDENCE'] = str(captures)
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
    require_unoptimized_python()
    assert len(sys.argv) == 2 and sys.argv[1] in {'cpu', 'gpu', 'export'}, (
        'usage: static_model_uv_gate.py cpu|gpu|export')
    initialize(pathlib.Path(__file__).resolve().parents[1],
               pathlib.Path(os.environ['ORR_STATIC_UV_EVIDENCE']), sys.argv[1])
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
