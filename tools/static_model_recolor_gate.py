#!/usr/bin/env python3
"""Mandatory current-source color-variant acceptance; standard library only.

Hosted cpu/gpu/ui each compile independent, retained producers in a fresh
target. Separate local model-only acceptance cannot certify bindings, GPU
pixels, or source-hidden editor preview. The reviewed static
prop runner supplies bounded output, process-tree cleanup and producer identity;
its source and independent fixtures are part of every prebuild source fence.
"""
import hashlib
import json
import os
import pathlib
import re
import sys
import traceback
import struct
import zlib

import static_model_extract_gate as runner

FEATURE_PATHS = (
    'crates/orr_model/examples/static_model_recolor.rs',
    'crates/orr_model/examples/static_model_recolor/authoring.rs',
    'crates/orr_model/tests/static_model_recolor.rs',
    'crates/orr_model_bindings/tests/static_model_recolor.rs',
    'crates/orr_render/tests/static_model_recolor.rs',
    'crates/orr_editor/tests/static_model_recolor_package_preview_ui.rs',
    'docs/static-model-recolor.md',
    'tools/static_model_recolor_gate.py',
    'tools/test_static_model_recolor_gate.py',
    '.github/workflows/static-model-recolor.yml',
)
INFRA_PATHS = ('Cargo.toml', 'Cargo.lock', 'tools/static_model_extract_gate.py',
               'tools/test_static_model_extract_gate.py')
BINDINGS_TEST = 'installed_static_color_variants_keep_distinct_bindings_and_source_geometry'
GPU_TEST = 'static_color_variants_match_independent_gpu_oracle'
UI_TEST = 'installed_recolor_variants_preview_read_only_and_reopen_identity'
FIXTURE_COUNT = 15
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
        ('test_static_model_recolor_gate.py', f'STATIC_RECOLOR_GATE_FIXTURES: {FIXTURE_COUNT} passed; 0 failed; 0 skipped'),
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
                              '--example', 'static_model_recolor', '--message-format=json'])
    binary = pin(output, 'recolor-cli', 'orr_model', 'static_model_recolor', 'example', ['import'])
    os.environ['ORR_STATIC_MODEL_RECOLOR_BIN'] = binary
    return binary


def model_tests(platform):
    assert platform in {"linux", "darwin", "win32"}
    names = 'exact_recipe_path_and_model_table_budgets strict_recipe_types_duplicates_shapes_and_f64_bounds portable_paths_and_identity_reject_before_resolution full_source_admission_used_slot_and_bounded_resolution replacement_preserves_every_unowned_float_bit_including_signed_zero provenance_exact_bytes_distinct_ids_same_rgb_and_nested_variants rgb_endpoints_and_rounding_are_admitted_after_f64_validation cli_publishes_once_without_overwrite_and_preserves_all_inputs cli_help_and_three_unique_pairs_are_bounded cli_bad_arguments_fail_without_output_or_staging cli_portable_output_names_and_package_store_rejection cli_missing_nonregular_and_oversized_inputs_leave_no_output cli_resolver_is_anchored_to_recipe_parent cli_parent_traversal_preserves_raw_spelling_and_never_publishes'.split()
    if platform == "win32":
        names.append("cli_windows_nonlocal_prefixes_reject")
    else:
        names.extend(["cli_symlink_inputs_ancestors_and_destinations_reject", "cli_non_utf8_option_names_and_logical_ids_reject"])
        if platform == "linux":
            names.append("cli_non_utf8_filesystem_inputs_work")
    return names


def model():
    flags = cargo_flags()
    output = run('build-feature-off', ['cargo', 'build', *flags, '--example',
                                      'static_model_recolor', '--message-format=json'])
    disabled = pin(output, 'feature-off-cli', 'orr_model', 'static_model_recolor', 'example', [])
    assert 'requires --features import' in run('feature-off-exit2', [disabled, '--help'], expected=2)
    cli = build_cli()
    assert 'Usage: static_model_recolor' in run('actual-help', [cli, '--help'])
    output = run('build-model-test', ['cargo', 'test', *flags, '--features', 'import',
                                    '--test', 'static_model_recolor', '--no-run', '--message-format=json'])
    binary = pin(output, 'model-test', 'orr_model', 'static_model_recolor', 'test', ['import'])
    test('model', binary, model_tests(sys.platform))
    # Full original model regression suites remain mandatory, including Unix
    # socket refusal fixtures; restricted local execution is not a waiver.
    run('model-full-regressions', ['cargo', 'test', *flags, '--features', 'import', '--tests', '--', '--test-threads=1'])
    run('strict-model-import', ['cargo', 'clippy', *flags, '--features', 'import',
                               '--lib', '--example', 'static_model_recolor', '--test',
                               'static_model_recolor', '--', '-D', 'warnings'])
    run('strict-model-default', ['cargo', 'clippy', *flags, '--lib', '--example',
                                'static_model_recolor', '--', '-D', 'warnings'])
    runner.report['coverage']['model'] = 'PASSED'
    runner.record()


def cpu():
    model()
    flags = cargo_flags('orr_model_bindings')
    output = run('build-bindings-test', ['cargo', 'test', *flags, '--test',
                                       'static_model_recolor', '--no-run', '--message-format=json'])
    binary = pin(output, 'bindings-test', 'orr_model_bindings', 'static_model_recolor', 'test', [])
    test('bindings', binary, [BINDINGS_TEST], ignored=True)
    run('strict-bindings', ['cargo', 'clippy', *flags, '--test', 'static_model_recolor', '--', '-D', 'warnings'])
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


def check_gpu(captures):
    """Recompute RGB/coverage from fixture-space constants, never cooked geometry."""
    import math
    import struct
    controls = ('unchanged_factor', 'old_factor_multiplied', 'srgb_input', 'wrong_material',
                'only_first_user', 'all_material_tint', 'swapped_channels', 'wrong_uv',
                'wrong_transform', 'offscreen')
    frame_files = {'source': 'source', 'rose': 'rose', 'teal': 'teal',
                   'variants_together': 'variants-together', 'variants_reversed': 'variants-reversed'}
    frame_files.update({name: name for name in controls})
    names = {'source.glb', 'source.model.json', 'adapter.json', 'receipt.json'}
    names.update(color + suffix for color in ('rose', 'teal') for suffix in ('.recolor.json', '.model.json'))
    names.update(name + suffix for name in frame_files.values() for suffix in ('.rgba', '.ppm'))
    inventory = capture_files(captures, names)
    receipt = json.loads((captures / 'receipt.json').read_text())
    adapter = json.loads((captures / 'adapter.json').read_text())
    assert receipt['schema'] == 1 and receipt['status'] == 'passed'
    assert receipt['width'] == receipt['height'] == 256
    assert receipt['adapter'] == adapter and adapter['software'] is True and adapter['backend'] == 'Vulkan'
    for name in ('shared_texture_materials', 'shared_material_users_checked', 'source_images_byte_identical',
                 'source_factor_alpha_bits_preserved_cpu', 'distinct_recipe_namespaces',
                 'reversed_batch_order_byte_identical'):
        assert receipt[name] is True
    assert receipt['tolerances'] == {'color_bytes': 3, 'minimum_control_differences': 500,
                                      'minimum_unaffected_differences': 100}
    assert set(receipt['frames']) == set(frame_files)
    pixels = {}
    for key, name in frame_files.items():
        frame = receipt['frames'][key]
        assert set(frame) == {'raw', 'image'}
        for field, suffix in (('raw', '.rgba'), ('image', '.ppm')):
            assert frame[field]['file'] == name + suffix
            capture_reference(captures, frame[field])
        data = pixels[key] = (captures / (name + '.rgba')).read_bytes()
        assert len(data) == 256 * 256 * 4
        assert data[3::4] == b'\xff' * (256 * 256)
        assert (captures / (name + '.ppm')).read_bytes() == b'P6\n256 256\n255\n' + bytes(v for i, v in enumerate(data) if i % 4 != 3)
    assert pixels['variants_together'] == pixels['variants_reversed']
    def diff(a, b):
        return sum(any(abs(a[i+c]-b[i+c]) > 3 for c in range(4)) for i in range(0, len(a), 4))
    assert set(receipt['control_pixel_differences']) == set(controls)
    for name in controls:
        count = diff(pixels[name], pixels['rose'])
        assert receipt['control_pixel_differences'][name] == count
        assert count > (100 if name == 'all_material_tint' else 500)
    assert diff(pixels['rose'], pixels['teal']) > 500
    positions = ((-1., -1.), (1., -1.), (1., .4), (.4, 1.), (-1., 1.), (1.1, -.75), (1.8, -.75), (1.4, .4))
    triangles = ((0,1,2), (0,2,3), (0,3,4), (5,6,7))
    texels = ((225,83,41,0), (47,198,112,63), (108,56,237,149), (173,219,91,231))
    factors = ((0.,.4,.8), (.65,.22,.48), (.18,.72,.34))
    def color(texel, factor):
        out = []
        for channel, multiplier in zip(texel, factor):
            c = channel / 255
            linear = (c / 12.92 if c <= .04045 else ((c + .055) / 1.055)**2.4) * multiplier * .6
            encoded = 12.92 * linear if linear <= .0031308 else 1.055 * linear**(1/2.4) - .055
            out.append(int(math.floor(encoded * 255 + .5)))
        return bytes(out + [255])
    colors = [[color(texel, factor) for texel in texels] for factor in factors]
    marker = color(texels[0], (.5,.9,.3))
    def weights(p, ids):
        a,b,c = [positions[i] for i in ids]
        d = (b[1]-c[1])*(a[0]-c[0]) + (c[0]-b[0])*(a[1]-c[1])
        u = ((b[1]-c[1])*(p[0]-c[0]) + (c[0]-b[0])*(p[1]-c[1])) / d
        v = ((c[1]-a[1])*(p[0]-c[0]) + (a[0]-c[0])*(p[1]-c[1])) / d
        return u,v,1-u-v
    marker_offsets = []
    configurations = {'source': ((0,0),), 'rose': ((0,1),), 'teal': ((0,2),),
                      'variants_together': ((-2,1),(2,2))}
    assert set(receipt['coverage']) == set(configurations)
    for name, placements in configurations.items():
        selected = [[0,0] for _ in placements]; markers = [[0,0] for _ in placements]
        checker = [0]*4; clear = 0
        for y in range(256):
            for x in range(256):
                world = ((x+.5-128)/32, (128-y-.5)/32)
                expected = None; near = False
                for model, (cx, mode) in enumerate(placements):
                    for node, (sign,cy) in enumerate(((-1,1.25),(1,-1.25))):
                        local = ((world[0]-cx)/(.8*sign), (world[1]-cy)/.65)
                        for triangle, ids in enumerate(triangles):
                            bary = weights(local, ids)
                            near |= min(bary) >= -.08
                            if min(bary) <= .08: continue
                            if triangle == 3:
                                markers[model][node] += 1
                                candidate = marker
                                if name == 'source': marker_offsets.append((y*256+x)*4)
                            else:
                                texel = (local[0]+1,1-local[1])
                                if any(not .02 <= v-math.trunc(v) <= .98 for v in texel): continue
                                slot = math.floor(texel[1])*2+math.floor(texel[0])
                                checker[slot] += 1; selected[model][node] += 1
                                candidate = colors[mode][slot]
                            assert expected is None
                            expected = candidate
                if expected is None and not near: expected = b'\0\0\0\xff'; clear += 1
                if expected is not None:
                    offset = (y*256+x)*4
                    assert all(abs(a-b) <= 3 for a,b in zip(pixels[name][offset:offset+4], expected)), (name,x,y)
        measured = {'selected_pixels_per_model_node': selected, 'unaffected_pixels_per_model_node': markers,
                    'source_checker_pixels': checker, 'clear_checked': clear}
        assert receipt['coverage'][name] == measured
        assert clear > 40000 and all(v > 500 for pair in selected for v in pair)
        assert all(v > 40 for pair in markers for v in pair) and all(v > 100 for v in checker)
    assert len(marker_offsets) == receipt['unaffected_material_pixels_checked'] > 100
    for offset in marker_offsets:
        assert pixels['source'][offset:offset+4] == pixels['rose'][offset:offset+4] == pixels['teal'][offset:offset+4]
    assets = receipt['assets']
    assert assets['source_hidden_after_install'] is True and assets['cooked_reload_before_render'] is True
    assert assets['installed_files'] == 2
    for field, name in (('original_glb','source.glb'), ('source','source.model.json')):
        assert assets[field]['file'] == name; capture_reference(captures, assets[field])
    source = json.loads((captures / 'source.model.json').read_text())
    assert source['images'][0] == {'width':2,'height':2,'rgba8':[v for t in texels for v in t]}
    assert source['materials'][0]['image'] == source['materials'][1]['image'] == 0
    assert len(source['materials']) == len(source['images']) == 3
    assert [p['material'] for p in source['primitives']] == [0,1,0,1]
    assert len(assets['variants']) == 2
    for entry, name, factor in zip(assets['variants'], ('rose','teal'), factors[1:]):
        asset_id = f'variants/{name}.recolor.json'
        assert entry['asset_id'] == asset_id
        for field, suffix in (('recipe','.recolor.json'),('cooked','.model.json')):
            assert entry[field]['file'] == name+suffix; capture_reference(captures,entry[field])
        recipe = json.loads((captures / (name+'.recolor.json')).read_text())
        cooked = json.loads((captures / (name+'.model.json')).read_text())
        assert recipe == {'format':'orr_static_material_color','version':1,'source':'inputs/prop.model.json','material':0,'linear_rgb':list(factor)}
        assert cooked['dependencies'] == [
            {'uri':'$source','sha256':runner.digest(captures/(name+'.recolor.json'))},
            {'uri':recipe['source'],'sha256':runner.digest(captures/'source.model.json')}]
        assert set(cooked) == set(source) and cooked['asset_id'] == asset_id
        for field in set(source)-{'asset_id','dependencies','primitives','materials'}: assert cooked[field] == source[field]
        assert len(cooked['primitives']) == len(source['primitives'])
        for before, after in zip(source['primitives'], cooked['primitives']):
            assert after == dict(before, id=asset_id+before['id'][len(source['asset_id']):])
        assert cooked['materials'][1:] == source['materials'][1:]
        before,after = source['materials'][0],cooked['materials'][0]
        assert {k:v for k,v in before.items() if k != 'base_color'} == {k:v for k,v in after.items() if k != 'base_color'}
        assert struct.pack('<f',after['base_color'][3]) == struct.pack('<f',before['base_color'][3])
        assert all(struct.pack('<f',a) == struct.pack('<f',b) for a,b in zip(after['base_color'][:3],factor))
    runner.report['captures'] = inventory


def gpu():
    runner.software()
    build_cli()
    captures = runner.evidence / 'captures'
    assert not captures.exists()
    os.environ['ORR_STATIC_MODEL_RECOLOR_CAPTURE_DIR'] = str(captures)
    flags = cargo_flags('orr_render')
    output = run('build-render-test', ['cargo', 'test', *flags, '--features', 'models,imported-scene',
                                     '--test', 'static_model_recolor', '--no-run', '--message-format=json'])
    binary = pin(output, 'render-test', 'orr_render', 'static_model_recolor', 'test', ['models', 'imported-scene'])
    run('strict-render', ['cargo', 'clippy', *flags, '--features', 'models,imported-scene',
                         '--test', 'static_model_recolor', '--', '-D', 'warnings'])
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


def check_ui(captures):
    assert captures.is_dir() and not captures.is_symlink()
    receipt = json.loads((captures / 'receipt.json').read_text())
    assert receipt['test'] == UI_TEST
    assert any(word in receipt['adapter'].lower() for word in ('llvmpipe', 'lavapipe'))
    for key in ('different_asset_pixels', 'red_blue_variant_pixels', 'red_blue_channel_reversal_pixels', 'orbit_changed_pixels', 'zoom_changed_pixels'):
        assert type(receipt[key]) is int and receipt[key] > 500, key
    for key in ('authoring_and_package_source_hidden', 'geometry_uv_images_alpha_unselected_materials_preserved', 'removed_cached_asset_refused', 'default_transform_and_full_binding_identity_reopened', 'reset_exact', 'preview_without_selection', 'preview_preserves_project_source_files_frame_history_bindings_and_editor_camera', 'corrupt_cached_asset_refused', 'use_model_save_reopen_preserved'):
        assert receipt[key] is True, key
    assert len(receipt['real_cli_logs']) == 4
    for name in receipt['real_cli_logs']:
        path = captures / name
        assert path.name == name and path.is_file() and not path.is_symlink()
    frames = sorted(captures.glob('*.png'))
    assert len(frames) >= 10
    for frame in frames:
        assert frame.read_bytes().startswith(b'\x89PNG\r\n\x1a\n')
    def decode(name):
        path = captures / name
        header = path.read_bytes()[:33]
        assert len(header) == 33 and header[12:16] == b'IHDR'
        width, height = struct.unpack_from('>II', header, 16)
        assert 0 < width <= 1024 and 0 < height <= 1024
        return (width, height), png_pixels(path, width, height)
    frames = {name: decode(name + '-canvas.png') for name in
              ('02-original', '03-blue', '03-red', '04-orbit', '05-zoom', '06-reset')}
    assert len({value[0] for value in frames.values()}) == 1
    def different(a, b):
        a, b = frames[a][1], frames[b][1]
        return sum(a[i:i+4] != b[i:i+4] for i in range(0, len(a), 4))
    for key, a, b in [('different_asset_pixels', '02-original', '03-blue'),
                      ('red_blue_variant_pixels', '03-red', '03-blue'),
                      ('orbit_changed_pixels', '03-blue', '04-orbit'),
                      ('zoom_changed_pixels', '04-orbit', '05-zoom')]:
        observed = different(a, b)
        assert observed == receipt[key] and observed > 1000, key
    red, blue = frames['03-red'][1], frames['03-blue'][1]
    reversal = sum(red[i] > min(255, blue[i] + 20) and blue[i+2] > min(255, red[i+2] + 20)
                   for i in range(0, len(red), 4))
    assert reversal == receipt['red_blue_channel_reversal_pixels'] and reversal > 500
    assert frames['03-blue'] == frames['06-reset'], 'reset exact independent pixels'
    assert not (captures / 'authoring').exists() and not (captures / 'owned-package-copy').exists()
    hidden = captures / 'hidden-package-source'
    authoring = captures / 'hidden-authoring'
    original = json.loads((hidden / 'original.orrmodel').read_text())
    for name in ('blue', 'red'):
        recipe_bytes = (authoring / (name + '.json')).read_bytes()
        recipe = json.loads(recipe_bytes)
        variant = json.loads((hidden / (name + '.orrmodel')).read_text())
        expected = json.loads(json.dumps(original))
        identity = 'variants/' + name + '.recolor.json'
        expected['asset_id'] = identity
        for primitive in expected['primitives']:
            primitive['id'] = identity + '#node=' + primitive['id'].split('#node=', 1)[1]
        assert [struct.pack('<f', v) for v in variant['materials'][0]['base_color'][:3]] == [struct.pack('<f', v) for v in recipe['linear_rgb']]
        expected['materials'][0]['base_color'][:3] = variant['materials'][0]['base_color'][:3]
        expected['dependencies'] = [
            {'uri': '$source', 'sha256': hashlib.sha256(recipe_bytes).hexdigest()},
            {'uri': recipe['source'], 'sha256': runner.digest(authoring / recipe['source'])}]
        assert variant == expected, 'UI cooked asset identity and preservation'
    sidecar = json.loads((captures / 'room' / 'room.models.json').read_text())
    assert any(value['asset'] == 'blue.orrmodel' for value in sidecar['bindings'].values())
    frames = sorted(captures.glob('*.png'))
    runner.report['ui_capture_sha256'] = {path.name: runner.digest(path) for path in frames}
    runner.report['ui_receipt'] = receipt
    runner.record()


def ui():
    runner.software()
    build_cli()
    captures = runner.evidence / 'captures'
    os.environ['ORR_STATIC_MODEL_RECOLOR_PREVIEW_CAPTURE_DIR'] = str(captures)
    flags = cargo_flags('orr_editor')
    target = 'static_model_recolor_package_preview_ui'
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
        'usage: static_model_recolor_gate.py cpu|gpu|ui')
    initialize(pathlib.Path(__file__).resolve().parents[1],
               pathlib.Path(os.environ['ORR_STATIC_RECOLOR_EVIDENCE']), sys.argv[1])
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
