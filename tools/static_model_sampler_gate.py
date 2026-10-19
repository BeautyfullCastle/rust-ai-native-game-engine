#!/usr/bin/env python3
"""Mandatory current-source sampler-default acceptance; standard library only.

Hosted cpu/gpu/ui each compile independent, retained producers in a fresh
target. Separate local model-only acceptance cannot certify bindings, GPU
pixels, or source-hidden editor preview. The reviewed static
prop runner supplies bounded output, process-tree cleanup and producer identity;
its source and independent fixtures are part of every prebuild source fence.
"""
import copy
import math
import functools
import hashlib
import json
import os
import pathlib
import re
import sys
import traceback
import struct
import zlib

def require_unoptimized_python():
    if sys.flags.optimize or not __debug__ or os.environ.get('PYTHONOPTIMIZE') not in (None, '', '0'):
        raise SystemExit('sampler gate requires unoptimized Python')


require_unoptimized_python()

import static_model_extract_gate as runner

FEATURE_PATHS = (
    'crates/orr_model/examples/static_model_sampler.rs',
    'crates/orr_model/examples/static_model_sampler/authoring.rs',
    'crates/orr_model/tests/static_model_sampler.rs',
    'crates/orr_model_bindings/tests/static_model_sampler.rs',
    'crates/orr_render/tests/static_model_sampler.rs',
    'crates/orr_editor/tests/static_model_sampler_package_preview_ui.rs',
    'docs/static-model-sampler.md',
    'tools/static_model_sampler_gate.py',
    'tools/test_static_model_sampler_gate.py',
    '.github/workflows/static-model-sampler.yml',
)
INFRA_PATHS = ('Cargo.toml', 'Cargo.lock', 'tools/static_model_extract_gate.py',
               'tools/test_static_model_extract_gate.py')
BINDINGS_TEST = 'installed_static_sampler_variants_keep_distinct_bindings_and_source_geometry'
GPU_TEST = 'static_sampler_variants_match_independent_gpu_oracle'
UI_TEST = 'installed_sampler_variants_preview_read_only_and_reopen_identity'
FIXTURE_COUNT = 12
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
    for package in ('orr_model', 'orr_model_bindings', 'orr_render', 'orr_editor'):
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
    assert phase in {'cpu', 'gpu', 'ui'}
    source_root = pathlib.Path(root).resolve()
    frozen = None
    mode = phase
    destination = pathlib.Path(destination)
    assert not destination.is_relative_to(source_root), 'evidence must be outside source'
    runner.initialize(source_root, destination, 'gpu' if phase == 'ui' else phase)
    runner.report['mode'] = phase
    runner.report['coverage'] = {name: 'NOT RUN' for name in ('model', 'bindings', 'gpu', 'ui')}
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
        ('test_static_model_sampler_gate.py', f'STATIC_SAMPLER_GATE_FIXTURES: {FIXTURE_COUNT} passed; 0 failed; 0 skipped'),
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
    assert names and len(names) == len(set(names)), 'nonempty exact executed test census'
    runner.inventory(run(label + '-inventory', [binary, '--list']), names)
    runner.inventory(run(label + '-ignored', [binary, '--ignored', '--list']), names if ignored else [])
    flags = ['--ignored', '--exact', names[0]] if ignored and len(names) == 1 else (['--ignored'] if ignored else [])
    runner.result(run(label + '-run', [binary, *flags, '--test-threads=1']), names)


def cargo_flags(package='orr_model'):
    return ['--locked', '--release', '-p', package, '--no-default-features']


def build_cli():
    flags = cargo_flags()
    output = run('build-cli', ['cargo', 'build', *flags, '--features', 'import',
                              '--example', 'static_model_sampler', '--message-format=json'])
    binary = pin(output, 'sampler-cli', 'orr_model', 'static_model_sampler', 'example', ['import'])
    os.environ['ORR_STATIC_MODEL_SAMPLER_BIN'] = binary
    return binary


CLI_TEST = 'pinned_cli_publishes_exact_bytes_and_rejects_before_output'


def model_tests(platform):
    assert platform in {'linux', 'darwin', 'win32'}
    names = ['strict_recipe_types_duplicates_shapes_and_exact_enums', 'portable_paths_and_identity_reject_before_resolution', 'full_source_admission_used_slot_and_bounded_resolution', 'replacement_preserves_every_unowned_float_bit_including_signed_zero', 'provenance_exact_bytes_distinct_ids_same_sampler_and_nested_variants', 'all_eighteen_filter_wrap_combinations_preserve_every_other_bit', 'exact_recipe_path_and_model_table_budgets', 'cli_publishes_once_without_overwrite_and_preserves_all_inputs', 'cli_help_and_three_unique_pairs_are_bounded', 'cli_bad_arguments_fail_without_output_or_staging', 'cli_portable_output_names_and_package_store_rejection', 'cli_missing_nonregular_and_oversized_inputs_leave_no_output', 'cli_resolver_is_anchored_to_recipe_parent', 'cli_parent_traversal_preserves_raw_spelling_and_never_publishes', 'exact_aggregate_input_budget_accepts_boundary_and_rejects_next_byte', 'sampler_composes_after_validated_retexture_and_uv_outputs', 'selecting_used_nonzero_material_preserves_slot_zero_and_shared_image']
    if platform == 'win32':
        names.append('cli_windows_nonlocal_prefixes_reject')
    else:
        names.extend(['cli_symlink_inputs_ancestors_and_destinations_reject', 'cli_non_utf8_option_names_and_logical_ids_reject'])
        if platform == 'linux': names.append('cli_non_utf8_filesystem_inputs_work')
    return names


def model():
    flags = cargo_flags()
    output = run('build-feature-off', ['cargo', 'build', *flags, '--example',
                                      'static_model_sampler', '--message-format=json'])
    disabled = pin(output, 'feature-off-cli', 'orr_model', 'static_model_sampler', 'example', [])
    assert 'requires --features import' in run('feature-off-exit2', [disabled, '--help'], expected=2)
    cli = build_cli()
    assert 'Usage: static_model_sampler' in run('actual-help', [cli, '--help'])
    output = run('build-model-test', ['cargo', 'test', *flags, '--features', 'import',
                                    '--test', 'static_model_sampler', '--no-run', '--message-format=json'])
    binary = pin(output, 'model-test', 'orr_model', 'static_model_sampler', 'test', ['import'])
    names = model_tests(sys.platform)
    runner.inventory(run('model-inventory', [binary, '--list']), names + [CLI_TEST])
    runner.inventory(run('model-ignored', [binary, '--ignored', '--list']), [CLI_TEST])
    for name in names:
        runner.result(run('model-' + name, [binary, '--exact', name, '--test-threads=1']), [name])
    runner.result(run('model-real-cli', [binary, '--ignored', '--exact', CLI_TEST, '--test-threads=1']), [CLI_TEST])
    # Full original model regression suites remain mandatory, including Unix
    # socket refusal fixtures; restricted local execution is not a waiver.
    run('model-full-regressions', ['cargo', 'test', *flags, '--features', 'import', '--tests', '--', '--test-threads=1'])
    run('strict-model-import', ['cargo', 'clippy', *flags, '--features', 'import',
                               '--lib', '--example', 'static_model_sampler', '--test',
                               'static_model_sampler', '--', '-D', 'warnings'])
    run('strict-model-default', ['cargo', 'clippy', *flags, '--lib', '--example',
                                'static_model_sampler', '--', '-D', 'warnings'])
    runner.report['coverage']['model'] = 'PASSED'
    runner.record()


def cpu():
    model()
    flags = cargo_flags('orr_model_bindings')
    output = run('build-bindings-test', ['cargo', 'test', *flags, '--test',
                                       'static_model_sampler', '--no-run', '--message-format=json'])
    binary = pin(output, 'bindings-test', 'orr_model_bindings', 'static_model_sampler', 'test', [])
    test('bindings', binary, [BINDINGS_TEST], ignored=True)
    run('strict-bindings', ['cargo', 'clippy', *flags, '--test', 'static_model_sampler', '--', '-D', 'warnings'])
    run('bindings-full-regressions', ['cargo', 'test', *flags, '--tests', '--', '--test-threads=1'])
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
    assert sum(path.stat().st_size for path in files) <= 512 * 1024**2
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


def fnv1a64(data):
    value = 0xcbf29ce484222325
    for byte in data:
        value = ((value ^ byte) * 0x100000001b3) & 0xffffffffffffffff
    return f'{value:016x}'


def uv_sample(uv, linear, wrap_s, wrap_t):
    """Independent normalized 2x2 sampler; interpolation is in linear light."""
    colors = ((239, 31, 71, 255), (17, 211, 89, 255), (53, 97, 229, 255), (179, 137, 43, 255))
    def address(value, wrap):
        return max(0, min(1, value)) if wrap == 'clamp' else (value % 2 if wrap == 'repeat' else (0, 1, 1, 0)[value % 4])
    assert wrap_s in {'repeat', 'mirror', 'clamp'} and wrap_t in {'repeat', 'mirror', 'clamp'}
    scaled = [value * 2 for value in uv]
    slot = address(math.floor(scaled[1]), wrap_t) * 2 + address(math.floor(scaled[0]), wrap_s)
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
                color = colors[address(base[1] + dy, wrap_t) * 2 + address(base[0] + dx, wrap_s)][channel] / 255
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



def check_variant(captures, source_file, recipe_file, cooked_file, asset_id, shared_users=True):
    source = strict_json(captures / source_file)
    recipe = strict_json(captures / recipe_file)
    cooked = strict_json(captures / cooked_file)
    assert (captures / recipe_file).stat().st_size <= 16 * 1024
    assert sum((captures / path).stat().st_size for path in (source_file, recipe_file)) <= 64 * 1024**2
    assert set(recipe) == {'format', 'version', 'source', 'material', 'linear_filter', 'wrap_s', 'wrap_t'}
    assert recipe['format'] == 'orr_static_material_sampler'
    assert type(recipe['version']) is int and recipe['version'] == 1
    assert type(recipe['material']) is int and recipe['material'] == 0
    assert type(recipe['linear_filter']) is bool
    assert all(recipe[key] in ('clamp', 'repeat', 'mirror') for key in ('wrap_s', 'wrap_t'))
    assert asset_id.endswith('.sampler.json') and asset_id.startswith('variants/')
    assert source['format'] == 'orr_static_model' and source['version'] == 1
    expected = copy.deepcopy(source)
    expected['asset_id'] = asset_id
    for primitive in expected['primitives']:
        assert primitive['id'].startswith(source['asset_id'] + '#node=')
        primitive['id'] = asset_id + primitive['id'][len(source['asset_id']):]
    for field in ('linear_filter', 'wrap_s', 'wrap_t'):
        expected['materials'][0][field] = recipe[field]
    expected['dependencies'] = [
        {'uri': '$source', 'sha256': runner.digest(captures / recipe_file)},
        {'uri': recipe['source'], 'sha256': runner.digest(captures / source_file)}]
    assert bit_equal(cooked, expected), 'only selected sampler and identity/provenance may change'
    assert sum(p['material'] == 0 for p in source['primitives']) >= (2 if shared_users else 1)
    assert any(p['material'] != 0 for p in source['primitives'])
    if shared_users:
        assert source['materials'][0]['image'] == source['materials'][1]['image']
    return cooked


def oracle_pixels(placements, samplers):
    selected = [[0, 0] for _ in placements]; markers = [[0, 0] for _ in placements]
    quadrants = [0] * 4; regions = [[0]*3 for _ in range(2)]; clear = 0; pixels = []
    for offset, interior in oracle_geometry(tuple(center for center, _ in placements)):
        if interior is None:
            clear += 1; pixels.append((offset, (0, 0, 0, 255))); continue
        model, node, marker, u, v = interior
        mode = placements[model][1]
        uv = (1.5 * (2*u-1) + .25, 1.25 * (1-2*v) + .4)
        spec = (False, 'clamp', 'clamp') if marker or mode == 0 else samplers[mode-1]
        sampled = uv_sample(uv, *spec)
        if sampled is None: continue
        color, quadrant = sampled
        (markers if marker else selected)[model][node] += 1
        if not marker:
            for axis in range(2): regions[axis][0 if uv[axis] < 0 else 1 if uv[axis] <= 1 else 2] += 1
            if mode == 0: quadrants[quadrant] += 1
        pixels.append((offset, color))
    assert all(n > 500 for pair in selected for n in pair)
    assert all(n > 40 for pair in markers for n in pair) and clear > 40000
    assert all(n > 100 for pair in regions for n in pair)
    return pixels, {'selected_pixels_per_model_node':selected, 'unaffected_pixels_per_model_node':markers,
                    'source_checker_pixels':quadrants,'clear_checked':clear,'uv_regions_below_inside_above':regions}


GPU_CASES = ('repeat-mirror', 'mirror-clamp', 'clamp-repeat')
GPU_FAULTS = ('wrong_filter', 'swapped_axes', 'wrong_wrap', 'all_material', 'world_transform')
GPU_FRAMES = ('source', 'nearest', 'linear', 'together', 'reversed') + tuple(f'variant{i}-{fault}' for i in range(2) for fault in GPU_FAULTS)


def check_gpu(captures):
    names = {'source.glb', 'adapter.json', 'receipt.json'}
    for case in GPU_CASES:
        names.add(case + '-source.model.json')
        names.update(f'{case}-{name}{suffix}' for name in ('nearest','linear') for suffix in ('.sampler.json','.model.json'))
        names.update(f'{case}-{name}{suffix}' for name in GPU_FRAMES for suffix in ('.rgba','.ppm'))
    assert len(names) == 108
    inventory = capture_files(captures, names)
    receipt, adapter = strict_json(captures/'receipt.json'), strict_json(captures/'adapter.json')
    assert receipt['schema'] == 1 and receipt['status'] == 'passed'
    assert (receipt['case_count'], receipt['frame_count'], receipt['artifact_count']) == (3,45,108)
    assert receipt['width'] == receipt['height'] == 256
    assert receipt['adapter'] == adapter and adapter['software'] is True and adapter['backend'] == 'Vulkan'
    assert receipt['tolerances'] == {'color_bytes':3,'minimum_control_differences':500,'minimum_unaffected_differences':30}
    for field, path in [('test_source_fnv1a64',source_root/'crates/orr_render/tests/static_model_sampler.rs'),
                        ('cli_source_fnv1a64',source_root/'crates/orr_model/examples/static_model_sampler.rs'),
                        ('authoring_source_fnv1a64',source_root/'crates/orr_model/examples/static_model_sampler/authoring.rs'),
                        ('producer_fnv1a64',runner.evidence/runner.report['producers']['sampler-cli']['file'])]:
        assert receipt[field] == fnv1a64(path.read_bytes()), 'source-bound GPU producer'
    assert (captures/'source.glb').read_bytes() == (source_root/'crates/orr_model/tests/fixtures/model.glb').read_bytes()
    assert set(receipt['cases']) == set(GPU_CASES)
    for case in GPU_CASES:
        item = receipt['cases'][case]; s,t = case.split('-')
        samplers = ((False,s,t),(True,t,s))
        for key in ('same_image_unselected_material','reversed_batch_order_byte_identical','opaque_alpha_checked'):
            assert item[key] is True
        frames = item['frames']; assert set(frames) == set(GPU_FRAMES)
        pixels = {}
        for name in GPU_FRAMES:
            frame = frames[name]; assert set(frame) == {'raw','image'}
            for field,suffix in [('raw','.rgba'),('image','.ppm')]:
                assert frame[field]['file'] == case+'-'+name+suffix
                capture_reference(captures,frame[field])
            raw = (captures/(case+'-'+name+'.rgba')).read_bytes(); pixels[name]=raw
            assert len(raw) == 256*256*4 and raw[3::4] == b'\xff'*65536
            assert (captures/(case+'-'+name+'.ppm')).read_bytes() == b'P6\n256 256\n255\n'+bytes(v for i,v in enumerate(raw) if i%4!=3)
        assert pixels['together'] == pixels['reversed']
        markers = marker_offsets(); assert item['unaffected_material_pixels_checked'] == len(markers)
        for offset in markers:
            assert pixels['source'][offset:offset+4] == pixels['nearest'][offset:offset+4] == pixels['linear'][offset:offset+4]
        pairs = {'unchanged_to_nearest':('source','nearest'),'unchanged_to_linear':('source','linear'),'distinct_variants':('nearest','linear')}
        pairs.update({f'variant{i}-{fault}':(f'variant{i}-{fault}',name) for i,name in enumerate(('nearest','linear')) for fault in GPU_FAULTS})
        assert set(item['control_pixel_differences']) == set(pairs)
        for name,(a,b) in pairs.items():
            measured = difference(pixels[a],pixels[b])
            assert measured == item['control_pixel_differences'][name] and measured > (30 if name.endswith('all_material') else 500)
        placements = {'source':((0,0),),'nearest':((0,1),),'linear':((0,2),),'together':((-2,1),(2,2))}
        assert set(item['coverage']) == set(placements)
        for name,placement in placements.items():
            expected,coverage = oracle_pixels(placement,samplers)
            assert item['coverage'][name] == coverage, 'independent exact coverage'
            for offset,color in expected:
                assert all(abs(a-b)<=3 for a,b in zip(pixels[name][offset:offset+4],color)), 'independent sampler pixel oracle'
        assets=item['assets']
        assert assets['source_hidden_after_install'] is True and assets['cooked_reload_before_render'] is True and assets['installed_files']==2
        for field,file in [('original_glb','source.glb'),('source',case+'-source.model.json')]:
            assert assets[field]['file']==file; capture_reference(captures,assets[field])
        assert len(assets['variants'])==2
        for i,(entry,name) in enumerate(zip(assets['variants'],('nearest','linear'))):
            identity='variants/'+name+'.sampler.json'
            assert entry['asset_id']==identity and set(entry)=={'asset_id','recipe','cooked'}
            for field,suffix in [('recipe','.sampler.json'),('cooked','.model.json')]:
                assert entry[field]['file']==case+'-'+name+suffix;capture_reference(captures,entry[field])
            cooked=check_variant(captures,case+'-source.model.json',case+'-'+name+'.sampler.json',case+'-'+name+'.model.json',identity)
            assert tuple(cooked['materials'][0][key] for key in ('linear_filter','wrap_s','wrap_t'))==samplers[i]
    runner.report['captures']=inventory

def gpu():
    runner.software()
    build_cli()
    captures = runner.evidence / 'captures'
    assert not captures.exists()
    os.environ['ORR_STATIC_MODEL_SAMPLER_CAPTURE_DIR'] = str(captures)
    flags = cargo_flags('orr_render')
    output = run('build-render-test', ['cargo', 'test', *flags, '--features', 'models,imported-scene',
                                     '--test', 'static_model_sampler', '--no-run', '--message-format=json'])
    binary = pin(output, 'render-test', 'orr_render', 'static_model_sampler', 'test', ['models', 'imported-scene'])
    run('strict-render', ['cargo', 'clippy', *flags, '--features', 'models,imported-scene',
                         '--test', 'static_model_sampler', '--', '-D', 'warnings'])
    test('render', binary, [GPU_TEST], ignored=True)
    check_gpu(captures)
    runner.report['coverage']['gpu'] = 'PASSED'


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


UI_PNGS = ('01-verified','02-original','02-original-canvas','03-nearest','03-nearest-canvas',
           '03-linear','03-linear-canvas','04-orbit','04-orbit-canvas','05-zoom-canvas',
           '06-reset','06-reset-canvas','07-closed','08-corrupt-refused','08-removed-refused',
           '09-assigned-saved','09-saved-viewport','10-reopened','10-reopened-viewport')
UI_LOGS = ('cli-nearest.stdout.log','cli-nearest.stderr.log','cli-linear.stdout.log','cli-linear.stderr.log')


def check_ui(captures):
    inventory = capture_files(captures, {name+'.png' for name in UI_PNGS} | set(UI_LOGS) |
                              {'receipt.json','room','hidden-package-source','hidden-authoring'})
    receipt = strict_json(captures/'receipt.json')
    assert receipt['test'] == UI_TEST and type(receipt['mandatory_test_count']) is int and receipt['mandatory_test_count']==1
    assert any(word in receipt['adapter'].lower() for word in ('llvmpipe','lavapipe'))
    for key in ('authoring_and_package_source_hidden','geometry_uv_images_color_unselected_materials_preserved',
                'removed_cached_asset_refused','default_transform_and_full_binding_identity_reopened','reset_exact',
                'preview_without_selection','preview_preserves_project_source_files_frame_history_bindings_and_editor_camera',
                'corrupt_cached_asset_refused','use_model_save_reopen_preserved','fresh_reopen_viewport_pixels_exact'):
        assert receipt[key] is True, key
    assert receipt['real_cli_logs'] == list(UI_LOGS)
    frames={}
    for name in UI_PNGS:
        path=captures/(name+'.png');header=path.read_bytes()[:33]
        assert len(header)==33 and header[12:16]==b'IHDR'
        width,height=struct.unpack_from('>II',header,16)
        assert 0<width<=2048 and 0<height<=2048
        frames[name]=((width,height),png_pixels(path,width,height))
    for field,a,b in [('wrap_changed_pixels','02-original-canvas','03-nearest-canvas'),
                      ('filter_changed_pixels','03-nearest-canvas','03-linear-canvas'),
                      ('orbit_changed_pixels','03-nearest-canvas','04-orbit-canvas'),
                      ('zoom_changed_pixels','04-orbit-canvas','05-zoom-canvas')]:
        assert frames[a][0]==frames[b][0]
        observed=difference(frames[a][1],frames[b][1],0)
        assert type(receipt[field]) is int and observed==receipt[field] and observed>1000
    assert frames['03-nearest-canvas']==frames['06-reset-canvas']
    assert receipt['normalized_viewport_size']==[900,850]
    saved=frames['09-saved-viewport'];assert saved==frames['10-reopened-viewport'] and saved[0]==(900,850)
    assert len(set(saved[1][i:i+4] for i in range(0,len(saved[1]),4)))>1, 'nonblank reopened viewport'
    hidden=captures/'hidden-package-source';authoring=captures/'hidden-authoring'
    assert (hidden/'original.orrmodel').read_bytes()==(authoring/'source.model.json').read_bytes()
    for name,linear in [('nearest',False),('linear',True)]:
        cooked=check_variant(captures,'hidden-authoring/source.model.json','hidden-authoring/'+name+'.json',
                             'hidden-package-source/'+name+'.orrmodel','variants/'+name+'.sampler.json', shared_users=False)
        assert tuple(cooked['materials'][0][key] for key in ('linear_filter','wrap_s','wrap_t'))==(linear,'repeat','mirror')
    nearest=strict_json(hidden/'nearest.orrmodel')
    assert receipt['variant_asset_id']==nearest['asset_id'] and receipt['variant_sampler']==nearest['materials'][0]
    sidecar=strict_json(captures/'room/room.models.json')
    binding=sidecar['bindings'][receipt['assigned_guid']]
    assert binding==receipt['assigned_binding'] and binding['asset']=='nearest.orrmodel'
    assert binding['source_hash']==receipt['variant_source_hash']==runner.digest(hidden/'nearest.orrmodel')
    assert binding['kind']=='static' and binding['package']=='static-sampler-preview'
    lock=strict_json(captures/'room/orr.packages.lock.json')['packages'][binding['package']]
    assert binding['package_digest']==receipt['installed_package_digest']==lock['digest']
    assert re.fullmatch('[0-9a-f]{64}',lock['digest'])
    installed=captures/'room/.orr/packages/objects'/lock['digest']
    for name in ('original','nearest','linear'):
        filename=name+'.orrmodel'
        assert (hidden/filename).read_bytes()==(installed/filename).read_bytes()
        assert runner.digest(hidden/filename)==lock['files'][filename]
        if name!='original': assert (authoring/(name+'.model.json')).read_bytes()==(hidden/filename).read_bytes()
    assert binding['transform']=={'translation':[0.0,0.0,0.0],'rotation':[0.0,0.0,0.0,1.0],'scale':[1.0,1.0,1.0]}
    runner.report['ui_capture_inventory']=inventory
    runner.report['ui_receipt']=receipt
    runner.record()

def ui():
    runner.software()
    build_cli()
    captures = runner.evidence / 'captures'
    os.environ['ORR_STATIC_MODEL_SAMPLER_PREVIEW_CAPTURE_DIR'] = str(captures)
    flags = cargo_flags('orr_editor')
    target = 'static_model_sampler_package_preview_ui'
    output = run('build-editor-test', ['cargo', 'test', *flags, '--features', 'package-manager', '--test', target, '--no-run', '--message-format=json'], seconds=2400)
    binary = pin(output, 'editor-test', 'orr_editor', target, 'test', ['models', 'package-manager', 'room-project'])
    run('strict-editor-test', ['cargo', 'clippy', *flags, '--features', 'package-manager', '--test', target, '--', '-D', 'warnings'])
    test('editor', binary, [UI_TEST], ignored=True)
    check_ui(captures)
    runner.report['coverage']['ui'] = 'PASSED'


def check_coverage():
    expected = {'cpu': {'model', 'bindings'}, 'gpu': {'gpu'}, 'ui': {'ui'}}[mode]
    assert runner.report['coverage'] == {name: ('PASSED' if name in expected else 'NOT RUN')
                                         for name in ('model', 'bindings', 'gpu', 'ui')}


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
    assert len(sys.argv) == 2 and sys.argv[1] in {'cpu', 'gpu', 'ui'}, (
        'usage: static_model_sampler_gate.py cpu|gpu|ui')
    initialize(pathlib.Path(__file__).resolve().parents[1],
               pathlib.Path(os.environ['ORR_STATIC_SAMPLER_EVIDENCE']), sys.argv[1])
    error = None
    try:
        begin()
        {'cpu': cpu, 'gpu': gpu, 'ui': ui}[mode]()
    except BaseException as failure:
        error = failure
    finally:
        finish(error)


if __name__ == '__main__':
    main()
