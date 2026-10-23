#!/usr/bin/env python3
"""Mandatory source-bound cylinder acceptance, standard library only.

Hosted CPU and GPU modes build retained producers in fresh targets. The reviewed
static prop runner supplies bounded output, process-tree cleanup and producer
identity; its source and independent fixtures join every source fence.
"""
import hashlib
import functools
import json
import math
import os
import pathlib
import re
import sys
import struct
import zlib
import traceback

if not __debug__ or sys.flags.optimize or os.environ.get('PYTHONOPTIMIZE') not in (None, '', '0'):
    raise SystemExit("static cylinder acceptance requires nonoptimized Python")

import static_model_extract_gate as runner

FEATURE_PATHS = (
 'crates/orr_model/examples/static_model_cylinder.rs',
 'crates/orr_model/examples/static_model_cylinder/authoring.rs',
 'crates/orr_model/tests/static_model_cylinder.rs',
 'crates/orr_model_bindings/tests/static_model_cylinder.rs',
 'crates/orr_render/tests/static_model_cylinder.rs',
 'crates/orr_editor/tests/static_model_cylinder_package_preview_ui.rs',
 'docs/static-model-cylinder.md',
 'tools/static_model_cylinder_gate.py',
 'tools/test_static_model_cylinder_gate.py',
 '.github/workflows/static-model-cylinder.yml',
)
INFRA_PATHS = ('Cargo.toml','Cargo.lock','tools/static_model_extract_gate.py',
 'tools/test_static_model_extract_gate.py')
CLI_TEST = 'actual_cli_publishes_validated_output_and_rejects_second_write'
BINDINGS_TEST = 'installed_static_cylinder_variants_keep_distinct_bindings_and_geometry'
GPU_TEST = 'static_cylinder_variants_match_independent_gpu_oracle'
UI_TEST = 'installed_cylinder_key_preview_undo_reopen_collect_restart'
FIXTURE_COUNT = 28
WORKFLOW_JOB = 'acceptance'
WORKFLOW_ROWS = (('cpu','ubuntu-22.04'),('cpu','windows-latest'),('gpu','ubuntu-22.04'),('ui','ubuntu-22.04'))
WORKFLOW_EVIDENCE = '${{ runner.temp }}/orr-static-cylinder-${{ matrix.mode }}-${{ github.run_id }}-${{ github.run_attempt }}'
WORKFLOW_ARTIFACT = 'static-model-cylinder-${{ matrix.mode }}-${{ matrix.os }}'

LEGACY_PATHS = ('.github/workflows/static-model-assemble.yml', '.github/workflows/static-model-recolor.yml', '.github/workflows/static-model-sampler.yml', 'tools/static_model_assemble_gate.py', 'tools/static_model_extract_gate.py', 'tools/static_model_pivot_gate.py', 'tools/static_model_recolor_gate.py', 'tools/static_model_retexture_gate.py', 'tools/static_model_sampler_gate.py', 'tools/static_model_uv_gate.py', 'tools/test_static_model_assemble_gate.py', 'tools/test_static_model_extract_gate.py', 'tools/test_static_model_pivot_gate.py', 'tools/test_static_model_recolor_gate.py', 'tools/test_static_model_retexture_gate.py', 'tools/test_static_model_sampler_gate.py', 'tools/test_static_model_uv_gate.py')

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
    required.update(LEGACY_PATHS)
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
    runner.report['legacy_acceptance'] = 'NOT RUN here; existing mandatory workflows remain required'
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
        ('test_static_model_cylinder_gate.py', f'STATIC_CYLINDER_GATE_FIXTURES: {FIXTURE_COUNT} passed; 0 failed; 0 skipped'),
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
    assert names and len(names) == len(set(names)), 'exact nonempty test census required'
    runner.inventory(run(label + '-inventory', [binary, '--list']), names)
    runner.inventory(run(label + '-ignored', [binary, '--ignored', '--list']), names if ignored else [])
    flags = ['--ignored', '--exact', names[0]] if ignored and len(names) == 1 else (['--ignored'] if ignored else [])
    runner.result(run(label + '-run', [binary, *flags, '--test-threads=1']), names)


def executed(output, names, filtered=0, ignored_names=()):
    """Validate an explicit libtest partition without accepting ignored as pass."""
    assert set(names).isdisjoint(ignored_names)
    assert len(names) == len(set(names)) and len(ignored_names) == len(set(ignored_names))
    normalized = output.replace('\r\n', '\n')
    expected_header = f'running {len(names)+len(ignored_names)} test' + ('' if len(names)+len(ignored_names)==1 else 's')
    lines = normalized.splitlines()
    assert lines.count(expected_header) == 1
    ignored = re.findall(r'^test ([A-Za-z0-9_:]+) \.\.\. ignored(?:, [^\n]+)?$', normalized, re.M)
    assert sorted(ignored) == sorted(ignored_names), 'only explicitly partitioned ignored fixtures allowed'
    lines = [line for line in lines if not re.fullmatch(r'test ([A-Za-z0-9_:]+) \.\.\. ignored(?:, [^\n]+)?', line)]
    normalized = '\n'.join(lines)
    normalized = normalized.replace(expected_header, f'running {len(names)} test' + ('' if len(names)==1 else 's'), 1)
    summaries = re.findall(r'^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out; finished in ([0-9]+(?:\.[0-9]+)?)s$', normalized, re.M)
    assert len(summaries) == 1 and tuple(map(int,summaries[0][:5])) == (len(names),0,len(ignored_names),0,filtered), 'exact partition summary'
    normalized = normalized.replace(f'; {len(ignored_names)} ignored; 0 measured; {filtered} filtered out;', '; 0 ignored; 0 measured; 0 filtered out;', 1)
    runner.result(normalized,names)


def cargo_flags(package='orr_model'):
    return ['--locked', '--release', '-p', package, '--no-default-features']


def build_cli():
    flags = cargo_flags()
    output = run('build-cli', ['cargo', 'build', *flags, '--features', 'import',
                              '--example', 'static_model_cylinder', '--message-format=json'])
    binary = pin(output, 'cylinder-cli', 'orr_model', 'static_model_cylinder', 'example', ['import'])
    os.environ['ORR_STATIC_MODEL_CYLINDER_BIN'] = binary
    return binary


def model_tests(platform):
    assert platform in {'linux', 'darwin', 'win32'}, 'unsupported model test platform'
    names = ['independent_geometry_oracle_covers_all_segments_and_extreme_proportions',
             'fixed_table_contract_has_exact_cardinals_diagonals_and_positive_zero',
             'exact_recipe_provenance_stable_identity_and_cooked_roundtrip',
             'strict_recipe_rejects_missing_duplicate_unknown_shapes_and_numeric_types',
             'dimensions_are_bounded_in_f64_before_rounding_to_f32',
             'logical_identity_rejects_nonportable_reserved_and_traversal_paths',
             'cli_publishes_once_without_overwrite_and_preserves_recipe',
             'cli_three_unique_pairs_help_and_bounded_argument_stream',
             'cli_portable_output_names_and_package_store_rejection',
             'cli_missing_nonregular_malformed_and_oversized_inputs_never_publish',
             'cli_parent_traversal_preserves_raw_spelling_and_never_publishes']
    if platform == 'win32':
        names.append('cli_windows_nonlocal_prefixes_reject')
    else:
        names.extend(['cli_symlink_inputs_ancestors_and_destinations_reject',
                      'cli_non_utf8_option_names_and_logical_ids_reject',
                      'cli_unix_fifo_inputs_reject_without_opening_or_publishing'])
        if platform == 'linux':
            names.append('cli_non_utf8_filesystem_inputs_work')
    assert len(names) == {'linux':15, 'darwin':14, 'win32':12}[platform]
    assert len(names) == len(set(names)), 'mandatory exact test census required'
    return names


def model():
    flags = cargo_flags()
    output = run('build-feature-off', ['cargo', 'build', *flags, '--example',
                                      'static_model_cylinder', '--message-format=json'])
    disabled = pin(output, 'feature-off-cli', 'orr_model', 'static_model_cylinder', 'example', [])
    assert 'requires --features import' in run('feature-off-exit2', [disabled, '--help'], expected=2)
    cli = build_cli()
    assert 'Usage: static_model_cylinder' in run('actual-help', [cli, '--help'])
    output = run('build-model-test', ['cargo', 'test', *flags, '--features', 'import',
                                    '--test', 'static_model_cylinder', '--no-run', '--message-format=json'])
    binary = pin(output, 'model-test', 'orr_model', 'static_model_cylinder', 'test', ['import'])
    names = model_tests(sys.platform)
    runner.inventory(run('model-inventory', [binary, '--list']), names + [CLI_TEST])
    runner.inventory(run('model-ignored', [binary, '--ignored', '--list']), [CLI_TEST])
    executed(run('model-ordinary', [binary, '--test-threads=1']), names, ignored_names=[CLI_TEST])
    executed(run('model-required-cli', [binary, '--ignored', '--exact', CLI_TEST, '--test-threads=1']),
             [CLI_TEST], filtered=len(names))
    # Target only model library and the new source-bound fixture. Other workspace
    # acceptance (including existing socket fixtures) has its own required gates.
    run('model-library-regressions', ['cargo', 'test', *flags, '--features', 'import', '--lib'])
    run('strict-model-import', ['cargo', 'clippy', *flags, '--features', 'import',
                               '--lib', '--example', 'static_model_cylinder', '--test',
                               'static_model_cylinder', '--', '-D', 'warnings'])
    run('strict-model-default', ['cargo', 'clippy', *flags, '--lib', '--example',
                                'static_model_cylinder', '--', '-D', 'warnings'])
    runner.report['coverage']['model'] = 'PASSED'
    runner.record()



def regular(path, limit=64 * 1024**2):
    assert path.is_file() and not path.is_symlink(), 'regular retained file required'
    assert 0 <= path.stat().st_size <= limit, 'bounded retained file required'
    return path


def strict_json_text(text):
    def pairs(values):
        result = {}
        for key, value in values:
            assert key not in result, 'duplicate receipt/model object field'
            result[key] = value
        return result
    def reject(value):
        raise AssertionError('nonfinite JSON number: ' + value)
    value = json.loads(text, object_pairs_hook=pairs, parse_constant=reject)
    def finite(node):
        if isinstance(node, dict):
            for child in node.values(): finite(child)
        elif isinstance(node, list):
            for child in node: finite(child)
        elif isinstance(node, float):
            assert math.isfinite(node), 'nonfinite JSON exponent'
    finite(value)
    return value


def strict_json(path):
    return strict_json_text(regular(path).read_text(encoding='utf-8'))


def software_manifest(environment, system_directory=pathlib.Path('/usr/share/vulkan/icd.d')):
    """Select one installed Lavapipe ICD without changing system configuration."""
    configured=[environment[key] for key in ('VK_DRIVER_FILES','VK_ICD_FILENAMES') if key in environment]
    if configured:
        for value in configured:
            assert type(value) is str and value and '\0' not in value, 'one nonempty Vulkan manifest required'
            path=pathlib.Path(value)
            # A Windows CPU fixture may use a normal C:\ path. Only its drive
            # colon is exempt; colon/semicolon-separated path lists are refused.
            assert ';' not in value and ':' not in value[len(path.drive):], 'Vulkan path lists are forbidden'
        assert len(set(configured))==1, 'Vulkan manifest environment variables disagree'
        path=pathlib.Path(configured[0])
    else:
        candidates=sorted(system_directory.glob('lvp_icd*.json'))
        assert len(candidates)==1, 'one configured or system Lavapipe manifest required'
        path=candidates[0]
    assert path.is_absolute() and path==path.resolve() and not path.is_symlink(), 'absolute ordinary Vulkan manifest required'
    assert re.fullmatch(r'lvp_icd[^/]*\.json',path.name), 'Lavapipe manifest filename required'
    regular(path,65536)
    with path.open('rb') as stream:
        raw=stream.read(65537)
    assert 0<len(raw)<=65536, 'bounded Vulkan manifest bytes required'
    value=strict_json_text(raw.decode('utf-8'))
    assert type(value) is dict and value.get('file_format_version')=='1.0.0', 'supported Vulkan manifest format required'
    icd=value.get('ICD')
    assert type(icd) is dict and type(icd.get('library_path')) is str
    assert re.fullmatch(r'libvulkan_lvp\.so(?:\.[0-9]+)*',pathlib.Path(icd['library_path']).name), 'Lavapipe library required'
    assert type(icd.get('api_version')) is str and re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+',icd['api_version']), 'Vulkan API version required'
    return path,raw


def configure_software():
    manifest,raw=software_manifest(os.environ)
    runtime=runner.evidence/'xdg-runtime'
    runtime.mkdir(mode=0o700,exist_ok=False)
    os.environ.update(VK_DRIVER_FILES=str(manifest),VK_ICD_FILENAMES=str(manifest),
                      WGPU_BACKEND='vulkan',ORR_REQUIRE_GPU='1',XDG_RUNTIME_DIR=str(runtime))
    with (runner.evidence/'lavapipe-icd.json').open('xb') as stream:
        stream.write(raw)
    runner.report['gpu_environment']={key:os.environ[key] for key in (
        'VK_DRIVER_FILES','VK_ICD_FILENAMES','WGPU_BACKEND','ORR_REQUIRE_GPU','XDG_RUNTIME_DIR')}
    digest=hashlib.sha256(raw).hexdigest()
    runner.report['icd_sha256']=digest
    runner.report['icd_manifest']={'path':str(manifest),'file':'lavapipe-icd.json','bytes':len(raw),'sha256':digest}
    runner.record()


def software_fence():
    item=runner.report.get('icd_manifest')
    if item is not None:
        original=pathlib.Path(item['path'])
        assert original==original.resolve() and not original.is_symlink(), 'configured ICD path changed'
        for path in (original,runner.evidence/item['file']):
            assert regular(path,65536).stat().st_size==item['bytes'] and runner.digest(path)==item['sha256'], 'configured or retained ICD bytes changed'


def capture_files(captures, expected):
    assert captures.is_dir() and not captures.is_symlink(), 'regular capture directory required'
    assert {path.name for path in captures.iterdir()} == set(expected), 'exact capture inventory'
    files = []
    for directory, folders, names in os.walk(captures, followlinks=False):
        for name in folders:
            assert not (pathlib.Path(directory) / name).is_symlink(), 'capture directory symlink'
        for name in names:
            path = regular(pathlib.Path(directory) / name)
            files.append(path)
            assert len(files) <= 256, 'bounded capture inventory'
    assert sum(path.stat().st_size for path in files) <= 512 * 1024**2, 'bounded capture bytes'
    return {path.relative_to(captures).as_posix(): {'bytes': path.stat().st_size, 'sha256': runner.digest(path)}
            for path in sorted(files)}


def difference(first, second, tolerance=0):
    assert len(first) == len(second) and len(first) % 4 == 0, 'equal RGBA geometry'
    return sum(any(abs(first[index + channel] - second[index + channel]) > tolerance
                   for channel in range(4)) for index in range(0, len(first), 4))


def fnv1a64(data):
    value = 0xcbf29ce484222325
    for byte in data:
        value = ((value ^ byte) * 0x100000001b3) & 0xffffffffffffffff
    return f'{value:016x}'


def check_reference(captures, item):
    assert set(item) == {'file', 'bytes', 'fnv1a64'}, 'exact capture reference fields'
    assert type(item['file']) is str and item['file'] == pathlib.Path(item['file']).name
    data = regular(captures / item['file']).read_bytes()
    assert type(item['bytes']) is int and item['bytes'] == len(data)
    assert item['fnv1a64'] == fnv1a64(data), 'retained capture bytes changed'

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



def f32(value):
    assert type(value) in {int, float} and math.isfinite(value), 'finite numeric vertex'
    result = struct.unpack('<f', struct.pack('<f', value))[0]
    assert math.isfinite(result), 'finite binary32 vertex'
    return result


def geometry_oracle(radius, height, segments):
    """Analytic trigonometric oracle: independent of production's ring table."""
    assert segments in (8, 16, 32, 64, 128)
    radius, half = f32(radius), f32(f32(height) * .5)
    ring = []
    for index in range(segments):
        angle = 2 * math.pi * index / segments
        x, z = math.cos(angle), math.sin(angle)
        # The four mathematical axes are exactly zero, not libm residuals.
        if index % (segments // 4) == 0:
            x, z = ((1., 0.), (0., 1.), (-1., 0.), (0., -1.))[index // (segments // 4)]
        ring.append((f32(x), f32(z)))
    side = []
    for index in range(segments + 1):
        x, z = ring[index % segments]
        for y, v in ((-half, 1.), (half, 0.)):
            side.append({'position': [f32(radius*x), y, f32(radius*z)],
                         'normal': [x, 0., z], 'uv': [index/segments, v]})
    side_indices = []
    for index in range(segments):
        bottom = 2*index
        side_indices.extend((bottom, bottom+1, bottom+2, bottom+1, bottom+3, bottom+2))
    result = [(side, side_indices)]
    for y, normal_y in ((half, 1.), (-half, -1.)):
        vertices = [{'position': [0., y, 0.], 'normal': [0., normal_y, 0.], 'uv': [.5, .5]}]
        indices = []
        for index, (x, z) in enumerate(ring):
            vertices.append({'position': [f32(radius*x), y, f32(radius*z)],
                             'normal': [0., normal_y, 0.],
                             'uv': [f32(.5 + f32(.5*x)), f32(.5 + f32(.5*z))]})
            current, following = index+1, (index+1)%segments+1
            indices.extend((0, following, current) if normal_y > 0 else (0, current, following))
        result.append((vertices, indices))
    return result


def validate_cylinder(model, recipe, recipe_hash, asset_id):
    assert set(recipe) == {'format', 'version', 'radius', 'height', 'segments'}
    assert recipe['format'] == 'orr_static_model_cylinder' and type(recipe['version']) is int and recipe['version'] == 1
    assert type(recipe['segments']) is int and recipe['segments'] in (8, 16, 32, 64, 128)
    for key in ('radius', 'height'):
        assert type(recipe[key]) in (int, float) and math.isfinite(recipe[key]) and .001 <= recipe[key] <= 1000000
    assert set(model) == {'format', 'version', 'asset_id', 'dependencies', 'primitives', 'materials', 'images'}
    assert model['format'] == 'orr_static_model' and type(model['version']) is int and model['version'] == 1
    assert model['asset_id'] == asset_id
    assert model['dependencies'] == [{'uri': '$source', 'sha256': recipe_hash}]
    assert model['images'] == [{'width': 1, 'height': 1, 'rgba8': [255]*4}]
    expected_material = {'base_color': [1.]*4, 'image': 0, 'linear_filter': False,
                         'wrap_s': 'clamp', 'wrap_t': 'clamp'}
    assert len(model['materials']) == 3 and all(item == expected_material for item in model['materials'])
    assert len(model['primitives']) == 3, 'three capped-cylinder primitives required'
    identity = [[1.,0.,0.,0.], [0.,1.,0.,0.], [0.,0.,1.,0.], [0.,0.,0.,1.]]
    edges = {}
    expected = geometry_oracle(recipe['radius'], recipe['height'], recipe['segments'])
    for slot, (primitive, (vertices, indices)) in enumerate(zip(model['primitives'], expected)):
        assert set(primitive) == {'id', 'vertices', 'indices', 'material', 'transform'}
        assert primitive['id'] == f'{asset_id}#node=0/mesh=0/primitive={slot}'
        assert type(primitive['material']) is int and primitive['material'] == slot
        assert primitive['transform'] == identity
        assert primitive['indices'] == indices and all(type(index) is int for index in primitive['indices'])
        assert len(primitive['vertices']) == len(vertices)
        for actual, target in zip(primitive['vertices'], vertices):
            assert set(actual) == {'position', 'normal', 'uv'}
            for field, extent in (('position', 3), ('normal', 3), ('uv', 2)):
                assert type(actual[field]) is list and len(actual[field]) == extent
                # Match analytic geometry within binary32 rounding, not an
                # implementation-specific quarter-table byte copy.
                for value, wanted in zip(actual[field], target[field]):
                    assert abs(f32(value)-wanted) <= max(abs(wanted)*2e-7, 1e-7), f'cylinder {field} analytic oracle'
        for offset in range(0, len(indices), 3):
            triangle = [primitive['vertices'][index]['position'] for index in indices[offset:offset+3]]
            a, b, c = triangle
            ab, ac = [b[i]-a[i] for i in range(3)], [c[i]-a[i] for i in range(3)]
            normal = [ab[1]*ac[2]-ab[2]*ac[1], ab[2]*ac[0]-ab[0]*ac[2], ab[0]*ac[1]-ab[1]*ac[0]]
            center = [sum(point[i] for point in triangle)/3 for i in range(3)]
            outward = [center[0], 0., center[2]] if slot == 0 else [0., 1. if slot == 1 else -1., 0.]
            assert sum(normal[i]*outward[i] for i in range(3)) > 0, 'outward nondegenerate triangle'
            for first, second in ((a,b), (b,c), (c,a)):
                first, second = tuple(f32(v) for v in first), tuple(f32(v) for v in second)
                assert first != second
                key = tuple(sorted((first, second)))
                edges.setdefault(key, []).append((first, second))
    assert all(len(pair) == 2 and pair[0] == pair[1][::-1] for pair in edges.values()), 'closed oriented welded surface'
    return model


def check_cylinder(recipe_path, model_path, asset_id):
    assert regular(recipe_path).stat().st_size <= 16*1024
    return validate_cylinder(strict_json(model_path), strict_json(recipe_path), runner.digest(recipe_path), asset_id)


GPU_SPECS = {'tall': (.55, 2., 32), 'wide': (1., .65, 16)}
GPU_KINDS = ('white', 'checker', 'imported', 'imported-white', 'original',
             'wrong-geometry', 'wrong-uv', 'wrong-winding', 'lit', 'imported-lit',
             'wrong-normal', 'missing-cap')
GPU_FAULTS = ('original', 'wrong-geometry', 'wrong-uv', 'wrong-winding', 'wrong-normal', 'missing-cap')
TEXELS = ((239,31,71,255),(17,211,89,255),(53,97,229,255),(179,137,43,255))


def ray_hit(screen_x, screen_y, spec, sign):
    """Clip one camera ray against a regular polygon prism's halfspaces.

    No triangle indices, captured vertices, production ring table, renderer
    matrices or Rust mesh/oracle routines participate in this calculation.
    """
    radius, height, segments = spec
    apothem = radius * math.cos(math.pi / segments)
    low, high, surface = -math.inf, math.inf, None
    for index in range(segments):
        angle = (index + .5) * 2 * math.pi / segments
        nx, nz = math.cos(angle), math.sin(angle)
        origin = nx*screen_x - nz*sign*.6*screen_y
        direction = nz*.8
        if abs(direction) < 1e-12:
            if origin > apothem: return None
            continue
        boundary = (apothem-origin)/direction
        if direction > 0:
            if boundary < high: high, surface = boundary, ('side', index)
        else:
            low = max(low,boundary)
    for normal in (1.,-1.):
        boundary = (height*.5-normal*.8*screen_y)/(normal*sign*.6)
        if normal*sign > 0:
            if boundary < high: high, surface = boundary, ('cap', 1 if normal > 0 else 2)
        else:
            low = max(low,boundary)
    if high < low or surface is None: return None
    y, z = .8*screen_y+sign*.6*high, -sign*.6*screen_y+.8*high
    if surface[0] == 'side':
        index = surface[1]
        a = 2*math.pi*index/segments; b=2*math.pi*(index+1)/segments
        start=(radius*math.cos(a),radius*math.sin(a)); delta=(radius*math.cos(b)-start[0],radius*math.sin(b)-start[1])
        chord=((screen_x-start[0])*delta[0]+(z-start[1])*delta[1])/(delta[0]**2+delta[1]**2)
        interior = .06 < chord < .94 and abs(y) < height*.46 and high-low > .03
        return 0, ((index+chord)/segments, .5-y/height), interior
    margin=min(apothem-math.cos((index+.5)*2*math.pi/segments)*screen_x-math.sin((index+.5)*2*math.pi/segments)*z
               for index in range(segments))
    return surface[1], (.5+screen_x/(2*radius),.5+z/(2*radius)), margin > radius*.025 and high-low > .03


@functools.lru_cache(maxsize=4)
def ray_samples(spec, sign):
    samples=[]
    radius,height,_=spec
    for y in range(256):
        for x in range(256):
            sx,sy=(x+.5-128)/64,(128-y-.5)/64
            hit=ray_hit(sx,sy,spec,sign)
            offset=(y*256+x)*4
            if hit:
                surface,uv,interior=hit
                if interior: samples.append((offset,surface,uv))
            elif abs(sx)>radius+.04 or abs(sy)>.4*height+.6*radius+.04 or all(
                    ray_hit(sx+dx,sy+dy,spec,sign) is None
                    for dx,dy in ((-.03,0),(.03,0),(0,-.03),(0,.03),(-.03,-.03),(-.03,.03),(.03,-.03),(.03,.03))):
                samples.append((offset,-1,(0.,0.)))
    return samples


def check_ray_pixels(pixels, spec, sign, kind):
    assert len(pixels) == 256*256*4
    counts=[0,0,0];background=0
    for offset,surface,uv in ray_samples(tuple(spec),sign):
        if surface < 0:
            color=(0,0,0,255);background+=1
        else:
            if kind=='checker' and any(abs(2*v-round(2*v)) < .04 for v in uv): continue
            if kind=='white':color=(255,255,255,255)
            elif kind=='lit':
                # Independently encode hemispheric linear radiance from known
                # side/cap normal Y; do not read captured normals or coverage.
                linear=(.5,.8,.2)[surface]
                channel=round((1.055*linear**(1/2.4)-.055)*255)
                color=(channel,channel,channel,255)
            else:
                assert kind=='checker'
                color=TEXELS[min(1,max(0,math.floor(uv[1]*2)))*2+min(1,max(0,math.floor(uv[0]*2)))]
            counts[surface]+=1
        assert all(abs(pixels[offset+c]-color[c])<=3 for c in range(4)), f'independent ray pixel mismatch at {offset//4}, surface {surface}, kind {kind}'
    assert counts[0]>1000 and counts[1 if sign>0 else 2]>500 and background>35000
    assert counts[2 if sign>0 else 1]==0
    return {'interior_per_surface':counts,'background':background,'mismatches':0}


def validate_installed(captures, project_name, package, assets):
    lock=strict_json(captures/project_name/'orr.packages.lock.json')
    entry=lock['packages'][package]
    assert re.fullmatch('[0-9a-f]{64}',entry['digest'])
    assert set(entry['files'])==set(assets)
    installed=captures/project_name/'.orr/packages/objects'/entry['digest']
    for name in assets:
        source=captures/'hidden-package-source'/name
        target=installed/name
        assert regular(source).read_bytes()==regular(target).read_bytes(), 'installed-only source bytes changed'
        assert runner.digest(target)==entry['files'][name]
    return entry


def check_gpu(captures):
    frame_names={f'{name}-{side}-{kind}' for name in GPU_SPECS for side in ('top','bottom') for kind in GPU_KINDS}
    expected={filename+suffix for filename in frame_names for suffix in ('.rgba','.ppm')}
    expected.update({'adapter.json','receipt.json','project','hidden-authoring','hidden-package-source'})
    expected.update(name+suffix for name in GPU_SPECS for suffix in ('.recipe.json','.orrmodel'))
    expected.update(f'cli-{name}.{stream}.log' for name in GPU_SPECS for stream in ('stdout','stderr'))
    inventory=capture_files(captures,expected)
    receipt=strict_json(captures/'receipt.json')
    assert set(receipt)=={'schema','status','test','mandatory_test_count','width','height','frame_count','adapter','assets','frames','coverage',
                          'control_pixel_differences','minimum_control_differences','color_epsilon','independent_triangle_uv_pixel_oracle',
                          'independent_hemisphere_normal_oracle','dual_renderers_byte_identical','diagnostic_preserves_installed_geometry_uv_indices','test_source_fnv1a64'}
    assert receipt['schema']==1 and receipt['status']=='passed' and receipt['test']==GPU_TEST
    assert type(receipt['mandatory_test_count']) is int and receipt['mandatory_test_count']==1
    assert (receipt['width'],receipt['height'],receipt['frame_count'])==(256,256,48)
    assert receipt['minimum_control_differences']==500 and receipt['color_epsilon']==3
    assert receipt['test_source_fnv1a64']==fnv1a64((source_root/'crates/orr_render/tests/static_model_cylinder.rs').read_bytes())
    adapter=receipt['adapter'];assert adapter==strict_json(captures/'adapter.json')
    assert set(adapter)=={'name','backend','device_type','software'} and adapter['backend']=='Vulkan' and adapter['device_type']=='Cpu' and adapter['software'] is True
    assert any(token in adapter['name'].lower() for token in ('llvmpipe','lavapipe'))
    for key in ('independent_triangle_uv_pixel_oracle','independent_hemisphere_normal_oracle','dual_renderers_byte_identical','diagnostic_preserves_installed_geometry_uv_indices'):
        assert receipt[key] is True
    assets=receipt['assets']
    assert set(assets)=={'variants','authoring_and_package_source_hidden','installed_cooked_bytes_exact','producer_fnv1a64'}
    assert assets['authoring_and_package_source_hidden'] is True and assets['installed_cooked_bytes_exact'] is True
    assert assets['producer_fnv1a64']==fnv1a64((runner.evidence/runner.report['producers']['cylinder-cli']['file']).read_bytes())
    assert len(assets['variants'])==2
    for variant,name in zip(assets['variants'],GPU_SPECS):
        assert set(variant)=={'asset_id','recipe','cooked','stdout','stderr'}
        assert variant['asset_id']==f'props/{name}.cylinder.json'
        for field,filename in [('recipe',name+'.recipe.json'),('cooked',name+'.orrmodel'),('stdout',f'cli-{name}.stdout.log'),('stderr',f'cli-{name}.stderr.log')]:
            assert variant[field]['file']==filename;check_reference(captures,variant[field])
        recipe=strict_json(captures/(name+'.recipe.json'))
        assert (recipe['radius'],recipe['height'],recipe['segments'])==GPU_SPECS[name]
        check_cylinder(captures/(name+'.recipe.json'),captures/(name+'.orrmodel'),variant['asset_id'])
        assert (captures/(name+'.recipe.json')).read_bytes()==(captures/'hidden-authoring'/(name+'.json')).read_bytes()
        assert (captures/(name+'.orrmodel')).read_bytes()==(captures/'hidden-authoring'/(name+'.model.json')).read_bytes()==(captures/'hidden-package-source'/(name+'.orrmodel')).read_bytes()
    validate_installed(captures,'project','static-cylinder-gpu',[name+'.orrmodel' for name in GPU_SPECS])
    assert set(receipt['frames'])==frame_names
    frames={}
    for name in sorted(frame_names):
        item=receipt['frames'][name];assert set(item)=={'raw','image'}
        for field,suffix in [('raw','.rgba'),('image','.ppm')]:
            assert item[field]['file']==name+suffix;check_reference(captures,item[field])
        raw=regular(captures/(name+'.rgba')).read_bytes();assert len(raw)==256*256*4
        ppm=regular(captures/(name+'.ppm')).read_bytes()
        assert ppm==b'P6\n256 256\n255\n'+bytes(value for i,value in enumerate(raw) if i%4!=3)
        frames[name]=raw
    assert set(receipt['coverage'])=={f'{name}-{side}-{kind}' for name in GPU_SPECS for side in ('top','bottom') for kind in ('white','checker','lit')}
    independent={}
    for name,spec in GPU_SPECS.items():
        for side,sign in [('top',1),('bottom',-1)]:
            prefix=f'{name}-{side}'
            for first,second in [('white','imported-white'),('checker','imported'),('lit','imported-lit')]:
                assert frames[prefix+'-'+first]==frames[prefix+'-'+second], 'both actual renderers must match'
            for kind in ('white','checker','lit'):
                key=prefix+'-'+kind
                cover=receipt['coverage'][key]
                assert set(cover)=={'interior_per_surface','background','mismatches'}
                assert cover['mismatches']==0 and cover['background']>35000
                assert len(cover['interior_per_surface'])==3 and all(type(x) is int for x in cover['interior_per_surface'])
                assert cover['interior_per_surface'][0]>1000 and cover['interior_per_surface'][1 if sign>0 else 2]>500
                assert cover['interior_per_surface'][2 if sign>0 else 1]==0
                independent[key]=check_ray_pixels(frames[key],spec,sign,kind)
    controls=receipt['control_pixel_differences']
    assert set(controls)=={f'{name}-{side}-{fault}' for name in GPU_SPECS for side in ('top','bottom') for fault in GPU_FAULTS}|{'distinct-proportions'}
    oracle_rejected=[]
    for name in GPU_SPECS:
        for side in ('top','bottom'):
            prefix=f'{name}-{side}'
            for fault in GPU_FAULTS:
                correct='white' if fault=='original' else ('lit' if fault=='wrong-normal' else 'checker')
                observed=difference(frames[prefix+'-'+correct],frames[prefix+'-'+fault],3)
                assert type(controls[prefix+'-'+fault]) is int and controls[prefix+'-'+fault]==observed and observed>500
                try:
                    check_ray_pixels(frames[prefix+'-'+fault],GPU_SPECS[name],1 if side=='top' else -1,correct)
                except AssertionError:
                    oracle_rejected.append(prefix+'-'+fault)
                else:
                    raise AssertionError('incorrect control did not falsify independent ray oracle: '+prefix+'-'+fault)
    assert controls['distinct-proportions']==difference(frames['tall-top-white'],frames['wide-top-white'],3)>500
    runner.report['gpu_capture_inventory']=inventory
    runner.report['gpu_receipt']=receipt
    assert len(oracle_rejected)==24
    runner.report['independent_polygon_ray_oracle_rejected_controls']=oracle_rejected
    runner.report['independent_polygon_ray_oracle']=independent
    runner.record()


UI_PNGS = ('01-verified','02-original-canvas','03-tall-canvas','03-tall','04-wide-canvas',
           '05-orbit-canvas','06-zoom-canvas','07-reset-canvas','08-assigned-undo-redo-saved',
           '09-saved-viewport','10-reopened-viewport','10-reopened','11-collected-viewport',
           '11-collected','12-restarted-viewport','12-restarted')
UI_TRUE_FIELDS = ('dithering','reset_exact','authoring_and_package_source_hidden','preview_read_only',
                  'key_use_undo_redo_save','fresh_reopen_binding_exact','fresh_reopen_viewport_pixels_exact',
                  'actual_play_take_control_keyboard_interact','restart_viewport_pixels_exact',
                  'presentation_free_frame_bytes_exact','presentation_free_checksums_exact','source_and_installed_files_preserved')
UI_FIELDS = {'schema','status','producer_fnv1a64','test_source_fnv1a64','test','mandatory_test_count',
             'assigned_guid','assigned_role','assigned_binding','installed_package_digest','variant_asset_id',
             'variant_source_hash','adapter','renderer_options','predictable_texture_filtering','original_changed_pixels',
             'proportions_changed_pixels','orbit_changed_pixels','zoom_changed_pixels','collect_changed_pixels',
             'normalized_viewport_size','key_placements_before_collect','key_placements_after_collect',
             'key_placements_after_restart','key_collected','restart_key_collected','recorded_ticks',
             'recorded_checksums_checked','authoritative_checksum'} | set(UI_TRUE_FIELDS)


def check_ui(captures):
    expected={name+'.png' for name in UI_PNGS}
    expected.update(f'cli-{name}.{stream}.log' for name in GPU_SPECS for stream in ('stdout','stderr'))
    expected.update({'receipt.json','room','hidden-package-source','hidden-authoring','actual-gui.orrp',
                     'collected-authoritative.frame','presentation-free.frame'})
    inventory=capture_files(captures,expected)
    receipt=strict_json(captures/'receipt.json')
    assert set(receipt)==UI_FIELDS, 'exact Room KEY acceptance receipt fields'
    assert type(receipt['schema']) is int and receipt['schema']==1 and receipt['status']=='passed'
    assert receipt['test']==UI_TEST and type(receipt['mandatory_test_count']) is int and receipt['mandatory_test_count']==1
    assert receipt['test_source_fnv1a64']==fnv1a64((source_root/'crates/orr_editor/tests/static_model_cylinder_package_preview_ui.rs').read_bytes())
    assert receipt['producer_fnv1a64']==fnv1a64((runner.evidence/runner.report['producers']['cylinder-cli']['file']).read_bytes())
    assert any(token in receipt['adapter'].lower() for token in ('llvmpipe','lavapipe'))
    assert receipt['renderer_options']=='default' and receipt['predictable_texture_filtering'] is False
    for key in UI_TRUE_FIELDS:assert receipt[key] is True,key
    assert receipt['assigned_role']=='KEY' and re.fullmatch('e_[0-9a-f]{8}',receipt['assigned_guid'])
    assert receipt['normalized_viewport_size']==[900,850]
    for key,value in [('key_placements_before_collect',2),('key_placements_after_collect',1),
                      ('key_placements_after_restart',2),('key_collected',1),('restart_key_collected',0)]:
        assert type(receipt[key]) is int and receipt[key]==value,key
    for key in ('recorded_ticks','recorded_checksums_checked'):
        assert type(receipt[key]) is int and 0<receipt[key]<=10000,key
    assert receipt['recorded_checksums_checked']<=receipt['recorded_ticks']+1
    assert re.fullmatch('[0-9a-f]{16}',receipt['authoritative_checksum'])
    frames={}
    for name in UI_PNGS:
        path=captures/(name+'.png');header=regular(path).read_bytes()[:33]
        assert len(header)==33 and header[12:16]==b'IHDR'
        width,height=struct.unpack_from('>II',header,16)
        assert 0<width<=2048 and 0<height<=2048
        if name.endswith('-canvas'):assert width<=1024 and height<=1024
        frames[name]=((width,height),png_pixels(path,width,height))
    for key,first,second in [('original_changed_pixels','02-original-canvas','03-tall-canvas'),
                             ('proportions_changed_pixels','03-tall-canvas','04-wide-canvas'),
                             ('orbit_changed_pixels','03-tall-canvas','05-orbit-canvas'),
                             ('zoom_changed_pixels','05-orbit-canvas','06-zoom-canvas'),
                             ('collect_changed_pixels','10-reopened-viewport','11-collected-viewport')]:
        assert frames[first][0]==frames[second][0]
        observed=difference(frames[first][1],frames[second][1])
        assert type(receipt[key]) is int and receipt[key]==observed and observed>1000,key
    assert frames['03-tall-canvas']==frames['07-reset-canvas']
    saved=frames['09-saved-viewport']
    assert saved==frames['10-reopened-viewport']==frames['12-restarted-viewport']
    for name in ('09-saved-viewport','10-reopened-viewport','11-collected-viewport','12-restarted-viewport'):
        dimensions,pixels=frames[name];assert dimensions==(900,850)
        assert sum(pixels[i:i+4]!=pixels[:4] for i in range(0,len(pixels),4))>1000, 'nonblank actual production viewport'
    entry=validate_installed(captures,'room','static-cylinder-preview',['original.orrmodel','tall.orrmodel','wide.orrmodel'])
    for name,spec in GPU_SPECS.items():
        recipe_path=captures/'hidden-authoring'/(name+'.json')
        model_path=captures/'hidden-package-source'/(name+'.orrmodel')
        recipe=strict_json(recipe_path)
        assert (recipe['radius'],recipe['height'],recipe['segments'])==spec
        check_cylinder(recipe_path,model_path,f'props/{name}.cylinder.json')
        assert regular(captures/'hidden-authoring'/(name+'.model.json')).read_bytes()==model_path.read_bytes()
    sidecar=strict_json(captures/'room/room.models.json')
    assert sidecar['version']==2 and sidecar['scene']=='room.scene.yaml' and sidecar['project']=='.'
    assert len(sidecar['bindings'])==2
    binding=sidecar['bindings'][receipt['assigned_guid']]
    assert set(binding)=={'kind','package','asset','package_digest','source_hash','transform'}
    assert binding==receipt['assigned_binding'] and binding['kind']=='static' and binding['package']=='static-cylinder-preview'
    assert binding['asset']=='tall.orrmodel'
    assert binding['package_digest']==receipt['installed_package_digest']==entry['digest']
    assert binding['source_hash']==receipt['variant_source_hash']==runner.digest(captures/'hidden-package-source/tall.orrmodel')
    assert receipt['variant_asset_id']=='props/tall.cylinder.json'
    assert binding['transform']=={'translation':[0.,0.,0.],'rotation':[0.,0.,0.,1.],'scale':[1.,1.,1.]}
    scene=regular(captures/'room/room.scene.yaml').read_text(encoding='utf-8')
    assert f"  {receipt['assigned_guid']}:\n    RoomEscapeV1::Actor: {{ kind: 2, ordinal: 0 }}\n" in scene, 'assigned saved entity is the actual Room KEY'
    project=strict_json(captures/'room/orr.project.json')
    assert project['entry']['game']=='room-escape-v1' and project['entry']['models']=='room.models.json'
    authoritative=regular(captures/'collected-authoritative.frame').read_bytes()
    assert len(authoritative)>32 and authoritative==regular(captures/'presentation-free.frame').read_bytes(), 'exact presentation-free authoritative frame parity'
    assert regular(captures/'actual-gui.orrp').stat().st_size>32
    runner.report['ui_capture_inventory']=inventory
    runner.report['ui_receipt']=receipt
    runner.record()


def cpu():
    model()
    flags = cargo_flags('orr_model_bindings')
    output = run('build-bindings-test', ['cargo', 'test', *flags, '--test',
                 'static_model_cylinder', '--no-run', '--message-format=json'])
    binary = pin(output, 'bindings-test', 'orr_model_bindings', 'static_model_cylinder', 'test', [])
    test('bindings', binary, [BINDINGS_TEST], ignored=True)
    run('strict-bindings', ['cargo', 'clippy', *flags, '--test',
                           'static_model_cylinder', '--', '-D', 'warnings'])
    runner.report['coverage']['bindings'] = 'PASSED'
    runner.record()


def gpu():
    configure_software()
    build_cli()
    captures = runner.evidence / 'captures'
    assert not captures.exists(), 'fresh captures required'
    os.environ['ORR_STATIC_MODEL_CYLINDER_CAPTURE_DIR'] = str(captures)
    flags = cargo_flags('orr_render')
    output = run('build-render-test', ['cargo','test',*flags,'--features','models,imported-scene',
                                     '--test','static_model_cylinder','--no-run','--message-format=json'])
    binary = pin(output,'render-test','orr_render','static_model_cylinder','test',['models','imported-scene'])
    run('strict-render',['cargo','clippy',*flags,'--features','models,imported-scene',
                        '--test','static_model_cylinder','--','-D','warnings'])
    test('render',binary,[GPU_TEST],ignored=True)
    check_gpu(captures)
    runner.report['coverage']['gpu'] = 'PASSED'
    runner.record()


def ui():
    configure_software()
    build_cli()
    captures = runner.evidence / 'captures'
    assert not captures.exists(), 'fresh captures required'
    os.environ['ORR_STATIC_MODEL_CYLINDER_PREVIEW_CAPTURE_DIR'] = str(captures)
    flags = cargo_flags('orr_editor')
    target = 'static_model_cylinder_package_preview_ui'
    output = run('build-editor-test', ['cargo', 'test', *flags, '--features', 'package-manager',
                  '--test', target, '--no-run', '--message-format=json'], seconds=2400)
    binary = pin(output, 'editor-test', 'orr_editor', target, 'test', ['models', 'package-manager', 'room-project'])
    run('strict-editor-test', ['cargo', 'clippy', *flags, '--features', 'package-manager',
                              '--test', target, '--', '-D', 'warnings'])
    test('editor', binary, [UI_TEST], ignored=True)
    check_ui(captures)
    runner.report['coverage']['ui'] = 'PASSED'
    runner.record()


def check_coverage():
    expected = {'cpu': {'model', 'bindings'}, 'gpu': {'gpu'}, 'ui': {'ui'}}[mode]
    assert runner.report['coverage'] == {name: 'PASSED' if name in expected else 'NOT RUN'
                                         for name in ('model', 'bindings', 'gpu', 'ui')}


def finish(error):
    try:
        assert frozen is not None, 'prebuild source snapshot missing'
        source_fence()
        runner.fence()
        software_fence()
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
        'usage: static_model_cylinder_gate.py cpu|gpu|ui')
    initialize(pathlib.Path(__file__).resolve().parents[1],
               pathlib.Path(os.environ['ORR_STATIC_CYLINDER_EVIDENCE']), sys.argv[1])
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
