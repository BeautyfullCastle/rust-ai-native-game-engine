#!/usr/bin/env python3
"""Issues258/261/263 bounded, exact-inventory CPU + hosted llvmpipe acceptance.

Run CPU then GPU on the same checked-out head/target directory. No retries,
no hidden refreshes, no success substitution after an original failure.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import runpy
import struct
import sys
import zlib

FEATURES = 'tilemap,project-create'
CONSUMER = ['project_tilemap::tests::' + n for n in ('cache_reuse_budget_and_failure_are_transactional', 'cache_hit_still_checks_lock_tamper_and_installer_authority')]
PACKAGE = ['tests::' + n for n in ('tilemap_declaration_and_all_lock_writers_fail_closed', 'tilemap_added_inside_checked_callback_cannot_publish')]
CPU_COUNT = 35
CORE = ['tests::' + n for n in (
    'strict_json_and_resource_bounds', 'structural_bounds_and_finite_world_arithmetic',
    'region_validation_and_failure_preserving_edits', 'asymmetric_literal_world_and_uv_oracle')]
ADMISSION = [
    'tile_only_and_shared_actor_atlas_admit_once_with_row_major_draws',
    'empty_cells_and_region_zero_are_distinct_and_budget_is_combined',
    'malformed_refs_regions_and_bounded_documents_fail_full_admission',
    'missing_capability_invalid_png_and_tamper_never_become_usage',
    'no_tilemap_usage_is_none_and_legacy_admission_cannot_drop_tiles',
    'explicit_null_and_symlink_sidecars_are_rejected']
RECORDING = 'recording_batch_proves_pass_order_atomic_admission_and_clear_restoration'
COMPOSITOR = 'gpu_asymmetric_three_by_two_blank_shape_actor_and_wrong_region'
RUNTIME = 'cli_saved_map_four_captures_playhost_frame_parity'
EDITOR = 'app::tilemap_tests::actual_cli_editor_palette_paint_save_reopen_play'
FILL_EDITOR = 'app::tilemap_fill_tests::actual_cli_fill_save_reopen_play_literal_cells'
RESIZE = ['tilemap_panel::resize::tests::' + n for n in (
    'asymmetric_growth_trim_and_mixed_axes_preserve_coordinates_and_metadata_bits',
    'every_cropped_edge_rejects_even_region_zero_without_mutation',
    'bounds_maximum_noop_and_full_atlas_admission_are_atomic')]
RESIZE += ['tilemap_panel::resize_history_tests::' + n for n in (
    'installed_resize_dispatch_grouped_history_noop_and_crop_are_atomic',
    'installed_resize_rejects_invalid_guard_atlas_and_budget_without_commit')]
FILL = ['tilemap_panel::fill::tests::' + n for n in (
    'literal_components_distinguish_empty_zero_diagonals_and_row_edges',
    'maximum_component_fill_and_erase_are_bounded_and_immutable',
    'noops_and_invalid_bounds_or_regions_preserve_source')]
FILL += ['tilemap_panel::fill_history_tests::' + n for n in (
    'installed_fill_dispatch_is_atomic_for_budget_error_and_noop',
    'grouped_fill_undo_redo_noop_and_budget_failure_preserve_history')]
CREATOR = 'project_create::tilemap::tests::tilemap_creator_is_real_bounded_and_atomic'
PANEL = ['tilemap_panel::tests::' + n for n in (
    'paint_erase_noop_undo_and_redo_saved_bytes', 'history_is_bounded_and_budget_is_atomic')]
PANEL += ['tilemap_panel::tests::installed::' + n for n in (
    'real_template_shared_cache_paint_save_reopen_and_stale_save',
    'stale_manifest_and_lock_preserve_saved_file_and_history',
    'replaced_root_cannot_receive_save',
    'installed_png_and_sprite_document_tamper_refuse_save_with_unchanged_lock',
    'changed_actor_sidecar_refuses_save_and_epoch_pin_is_not_implicitly_refreshed',
    'scene_read_limit_matches_four_mebibyte_admission', 'sidecar_symlink_save_is_refused')]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


SUMMARY = re.compile(r"^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out; finished in .+$", re.M)
CASE = re.compile(r"^test (\S+) \.\.\. (ok|FAILED|ignored)(?: .*)?$", re.M)


def validate_inventory(output, names):
    actual = re.findall(r'^(\S+): test$', output.replace('\r\n', '\n'), re.M)
    if sorted(actual) != sorted(names) or len(actual) != len(set(actual)):
        raise RuntimeError('Unexpected test or ignored inventory')


def validate_cases(output, expected):
    output = output.replace('\r\n', '\n')
    summaries = SUMMARY.findall(output)
    # Native libraries can write directly to stderr even with --show-output,
    # interrupting libtest's "test NAME ... ok" line. Keep the named start and
    # require its unique terminal status, never infer success from a diagnostic.
    preamble = re.split(r'^(?:successes:|failures:|test result:)', output, maxsplit=1, flags=re.M)[0]
    rows, pending, split = [], None, False
    for line in preamble.splitlines():
        start = re.fullmatch(r'test (\S+) \.\.\. (.*)', line)
        if start:
            if pending is not None:
                raise RuntimeError('Missing terminal status before next named test')
            pending, tail = start.groups()
            inline = re.fullmatch(r'(ok|FAILED|ignored)(?:,.*)?', tail)
            if inline:
                rows.append((pending, inline.group(1)))
                pending = None
            else:
                split = True
        elif re.fullmatch(r'ok|FAILED|ignored(?:,.*)?', line):
            if pending is None:
                raise RuntimeError('Unowned or duplicate terminal test status')
            rows.append((pending, line.split(',', 1)[0]))
            pending = None
    if (not expected or pending is not None or len(summaries) != 1 or output.count('test result:') != 1
        or tuple(map(int, summaries[0][:4])) != (len(expected), 0, 0, 0)
        or re.findall(r'^running (\d+) tests?$', output, re.M) != [str(len(expected))]
        or sorted(rows) != sorted((name, 'ok') for name in expected)
        or re.search(r'(?im)^.*\bskip(?:ping)?\b', output)):
        raise RuntimeError('Incomplete/empty/skipped/nonpassing acceptance')
    if split:
        # The final --show-output success-name block is independent of the
        # interrupted progress lines. Pin it too; arbitrary warning text or a
        # lone "ok" is never sufficient to certify a named test.
        blocks = re.findall(r'^successes:\n((?:[ \t]+[^\n]+\n)+)', output, re.M)
        if len(blocks) != 1 or sorted(blocks[0].split()) != sorted(expected):
            raise RuntimeError('Interrupted test line lacks exact success-name corroboration')


def capture(path, size):
    """Read bounded RGBA8 PNG, checking CRC, complete structure and raw pixels."""
    data = path.read_bytes()
    if len(data) > 32 * 1024 * 1024 or data[:8] != b'\x89PNG\r\n\x1a\n':
        raise RuntimeError('Invalid PNG signature/size')
    offset, payload, kinds = 8, bytearray(), []
    while offset < len(data):
        if offset + 12 > len(data):
            raise RuntimeError('Truncated PNG chunk')
        n = struct.unpack('>I', data[offset:offset + 4])[0]
        kind, body = data[offset + 4:offset + 8], data[offset + 8:offset + 8 + n]
        if offset + 12 + n > len(data) or zlib.crc32(kind + body) & 0xffffffff != struct.unpack('>I', data[offset + 8 + n:offset + 12 + n])[0]:
            raise RuntimeError('Invalid PNG CRC/length')
        kinds.append(kind)
        if kind == b'IHDR':
            if len(kinds) != 1 or body != struct.pack('>IIBBBBB', *size, 8, 6, 0, 0, 0):
                raise RuntimeError('Unexpected PNG dimensions/format')
        elif kind == b'IDAT':
            payload.extend(body)
        elif kind == b'IEND':
            if body or offset + n + 12 != len(data):
                raise RuntimeError('Invalid PNG end')
        elif kind[:1].isupper():
            raise RuntimeError('Unexpected critical PNG chunk')
        offset += n + 12
    if not kinds or kinds[0] != b'IHDR' or kinds[-1] != b'IEND' or not payload:
        raise RuntimeError('Incomplete PNG')
    stride = size[0] * 4
    expected = (stride + 1) * size[1]
    decoder = zlib.decompressobj()
    filtered = decoder.decompress(payload, expected + 1)
    if len(filtered) != expected or not decoder.eof or decoder.unused_data or decoder.unconsumed_tail:
        raise RuntimeError('Incorrect bounded PNG pixel payload')
    pixels, previous = bytearray(), bytearray(stride)
    for y in range(size[1]):
        row = bytearray(filtered[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        method = filtered[y * (stride + 1)]
        if method > 4:
            raise RuntimeError('Unsupported PNG row filter')
        for x in range(stride):
            a, b, c = row[x - 4] if x >= 4 else 0, previous[x], previous[x - 4] if x >= 4 else 0
            if method == 1: predictor = a
            elif method == 2: predictor = b
            elif method == 3: predictor = (a + b) // 2
            elif method == 4:
                q = a + b - c
                distances = (abs(q - a), abs(q - b), abs(q - c))
                predictor = (a, b, c)[distances.index(min(distances))]
            else: predictor = 0
            row[x] = (row[x] + predictor) & 255
        pixels.extend(row)
        previous = row
    return sha(path), bytes(pixels)


def validate_receipts(runtime, editor, evidence, creator_hash, source_hash, load_capture=capture):
    def read(path):
        return json.loads(path.read_text())
    r, rp = read(runtime / 'receipt.json'), read(runtime / 'partial-receipt.json')
    e, ep = read(editor / 'editor-receipt.json'), read(editor / 'partial-editor-receipt.json')
    runtime_names = ['saved.png', 'fresh-reopen.png', 'actor-moving.png', 'tile-only-wrong-region.png']
    editor_names = ['dirty-paint.png', 'saved.png', 'fresh-reopen.png', 'play.png']
    for receipt in (rp, ep):
        if 'llvmpipe' not in receipt.get('adapter', '').lower():
            raise RuntimeError('Hosted llvmpipe receipt required; SwiftShader is not equivalent')
    if (r.get('completed') is not True or any(r.get(k) is not False for k in ('native_os_input', 'export', 'windows'))
        or [c.get('file') for c in r.get('captures', [])] != runtime_names
        or rp.get('captures') != r['captures'] or rp.get('creator_sha256') != creator_hash
        or rp.get('source_sha256') != source_hash):
        raise RuntimeError('Incomplete runtime acceptance or mismatched source identity')
    if (e.get('completed') is not True or e.get('captures') != editor_names
        or e.get('native_os_input') is not False or e.get('fresh_host_reopen') is not True
        or ep.get('last_capture') != 'play.png' or ep.get('size') != [1800, 2000]):
        raise RuntimeError('Incomplete full-editor acceptance')
    files, runtime_pixels = {}, []
    for directory, names, size in [(runtime, runtime_names, (192, 128)), (editor, editor_names, (1800, 2000))]:
        for index, name in enumerate(names):
            path = directory / name
            digest, pixels = load_capture(path, size)
            files[str(path.relative_to(evidence))] = digest
            if directory == runtime:
                runtime_pixels.append(pixels)
                if hashlib.sha256(pixels).hexdigest() != r['captures'][index].get('sha256'):
                    raise RuntimeError('Runtime capture raw-pixel hash mismatch')
            elif name == 'play.png':
                fingerprint = 0xcbf29ce484222325
                for byte in pixels:
                    fingerprint = ((fingerprint ^ byte) * 0x100000001b3) & 0xffffffffffffffff
                if ep.get('rgba_fnv1a64') != f'{fingerprint:016x}':
                    raise RuntimeError('Editor capture raw-pixel fingerprint mismatch')
    if runtime_pixels[0] != runtime_pixels[1]:
        raise RuntimeError('Fresh runtime reopen capture differs')
    return files



def source_fence(root):
    """Scoped acceptance sources, not a claim of whole-workspace identity."""
    paths = {root / name for name in ('Cargo.toml', 'Cargo.lock',
        'tools/tilemap_authoring_gate.py', 'tools/test_tilemap_authoring_gate.py',
        'crates/orr_editor/src/app/tilemap_fill_tests.rs',
        'crates/orr_editor/src/app/tilemap_resize_tests.rs',
        'crates/orr_editor/src/tilemap_panel/resize.rs',
        'crates/orr_editor/src/tilemap_panel/fill.rs')}
    # Includes original acceptance test dependencies, candidate validation,
    # producer templates, renderer/Frame dependencies and relevant manifests.
    for name in ('orr_editor', 'orr_tilemap', 'orr_sample', 'orr_package',
                 'orr_sprite', 'orr_sim', 'orr_session', 'orr_render', 'orr_rhi',
                 'orr_view', 'orr_bridge', 'orr_ecs', 'orr_fp'):
        directory = root / 'crates' / name
        if not directory.is_dir():
            raise RuntimeError('Missing scoped source directory: ' + name)
        paths.add(directory / 'Cargo.toml')
        for child in ('src', 'tests'):
            paths.update(p for p in (directory / child).rglob('*') if p.is_file())
    paths.update(p for p in (root / 'assets/sprite_demo').rglob('*') if p.is_file())
    return {str(p.relative_to(root)): sha(p) for p in sorted(paths)}


def fnv1a64(data):
    """Rust's explicitly labeled lightweight fingerprint; never called SHA256."""
    value = 0xcbf29ce484222325
    for byte in data:
        value = ((value ^ byte) * 0x100000001b3) & 0xffffffffffffffff
    return f'{value:016x}'


def evidence_bytes(path):
    if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= 64 * 1024 * 1024:
        raise RuntimeError('Missing or invalid regular Fill evidence: ' + str(path))
    return path.read_bytes()


def evidence_json(path):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise RuntimeError('Duplicate Fill receipt key')
            result[key] = value
        return result
    return json.loads(evidence_bytes(path), object_pairs_hook=unique)


FILL_PHASES = ['before', 'outside-origin', 'filled-held-drag', 'undo', 'redo',
               'saved', 'fresh-reopen', 'play', 'runtime-saved', 'runtime-wrong-region']
FILL_BEFORE = [20] * 42
for _index in (8, 9, 16, 17, 23, 25, 41):
    FILL_BEFORE[_index] = None
FILL_CELLS = FILL_BEFORE.copy()
for _index in (8, 9, 16, 17, 23):
    FILL_CELLS[_index] = 11


def validate_fill(directory, evidence, identities, load_capture=capture, expected_adapter="llvmpipe"):
    """Audit retained artifacts independently; Rust certifies Frame decode/oracle."""
    def require(condition, message):
        if not condition:
            raise RuntimeError('Fill ' + message)
    receipt = evidence_json(directory / 'receipt.json')
    partial = evidence_json(directory / 'partial-receipt.json')
    require(receipt.get('completed') is True and all(receipt.get(k) is False
        for k in ('native_os_input', 'export', 'windows')), 'incomplete or overclaimed scope')
    require(receipt.get('literal_cells') == FILL_CELLS and
        receipt.get('oracle_pixels_per_runtime_capture') == 10752, 'literal cell/oracle contract')
    require(type(receipt.get('wrong_region_visible_pixels')) is int and
        8 <= receipt['wrong_region_visible_pixels'] <= 256, 'wrong-region verdict')
    for field, data in identities.items():
        require(receipt.get(field) == fnv1a64(data), 'source/creator fingerprint mismatch: ' + field)
    rows = receipt.get('captures')
    require(isinstance(rows, list) and [r.get('name') for r in rows] == FILL_PHASES,
            'capture census/order mismatch')
    # The last partial intentionally records the last oracle as pending before
    # assertions. It must never substitute for the final completed receipt.
    expected_partial = json.loads(json.dumps(rows))
    expected_partial[-1]['pixel_verdict_pending'] = True
    expected_partial[-1].pop('oracle_pixels_checked', None)
    require(partial == {'completed': False, 'captures': expected_partial, 'native_os_input': False},
            'partial capture history mismatch')
    expected_files = {'receipt.json', 'partial-receipt.json', 'saved.tilemap.json',
                      'wrong-region.tilemap.json', 'creator.log'}
    frames, pixels, checksums, files = {}, {}, {}, {}
    for index, row in enumerate(rows):
        name = row['name']
        suffixes = ['.rgba', '.png', '.frame'] + (['-app.png'] if index < 8 else [])
        require(row.get('files') == [name + suffix for suffix in suffixes], 'file manifest mismatch')
        expected_files.update(row['files'])
        size = row.get('size')
        require(isinstance(size, list) and len(size) == 2 and
            all(type(n) is int and 1 <= n <= 4096 for n in size), 'invalid capture dimensions')
        rgba = evidence_bytes(directory / (name + '.rgba'))
        require(len(rgba) == size[0] * size[1] * 4 and fnv1a64(rgba) == row.get('rgba_fnv1a64'),
                'raw RGBA fingerprint/size mismatch')
        require(load_capture(directory / (name + '.png'), tuple(size))[1] == rgba,
                'PNG/raw RGBA mismatch')
        frame = evidence_bytes(directory / (name + '.frame'))
        require(fnv1a64(frame) == row.get('frame_fnv1a64'), 'Frame fingerprint mismatch')
        require(type(row.get('tick')) is int and row['tick'] == (0 if index < 7 else 1) and
            type(row.get('checksum')) is int and 0 <= row['checksum'] < 2**64, 'Frame identity mismatch')
        if index < 8:
            require(row.get('scope') == 'actual App viewport; authoring grid, no pixel verdict',
                    'App capture scope mismatch')
            require(expected_adapter.lower() in row.get('adapter', '').lower() and row.get('backend') == 'Vulkan'
                and row.get('device_type') == 'Cpu', 'hosted llvmpipe required; local SwiftShader is not equivalent')
            require(row.get('frame_source') == 'watch.subscribe source=view matched to predicted tick/checksum',
                    'actual authoritative Frame source missing')
            require(row.get('cells') == (FILL_BEFORE if index in (0, 1, 3) else FILL_CELLS),
                    'actual App literal cells mismatch')
            load_capture(directory / (name + '-app.png'), (1800, 2000))
        else:
            require(row.get('scope') == 'actual saved PreparedRuntime PlayHost production compositor',
                    'runtime capture scope mismatch')
            require(size == [224, 224] and row.get('pixel_verdict_pending') is False and
                row.get('oracle_pixels_checked') == 10752, 'runtime pixel oracle incomplete')
        frames[name], pixels[name], checksums[name] = frame, rgba, row['checksum']
    for names in (FILL_PHASES[:7], FILL_PHASES[7:]):
        require(all(frames[n] == frames[names[0]] and checksums[n] == checksums[names[0]] for n in names),
                'actual Frame byte/checksum parity mismatch')
    good, wrong = pixels['runtime-saved'], pixels['runtime-wrong-region']
    changed = sum(any(abs(good[i+c] - wrong[i+c]) > 3 for c in range(4))
                  for y in range(64, 80) for x in range(72, 88) for i in [(y * 224 + x) * 4])
    require(changed == receipt['wrong_region_visible_pixels'], 'retained wrong-region pixel verdict mismatch')
    saved = evidence_json(directory / 'saved.tilemap.json')
    wrong_map = evidence_json(directory / 'wrong-region.tilemap.json')
    expected_wrong = FILL_CELLS.copy(); expected_wrong[8] = 10
    for doc, cells in ((saved, FILL_CELLS), (wrong_map, expected_wrong)):
        require(doc.get('cells') == cells and doc.get('width') == 7 and doc.get('height') == 6
            and doc.get('origin') == [-56., 144.], 'retained literal map mismatch')
    require({p.name for p in directory.iterdir()} == expected_files, 'retained artifact census mismatch')
    for name in sorted(expected_files):
        path = directory / name
        # CLI stdout may legitimately be empty; it is still retained and hashed.
        if name == 'creator.log':
            require(path.is_file() and not path.is_symlink() and path.stat().st_size <= 64 * 1024**2,
                    'missing creator log')
            data = path.read_bytes()
        else:
            data = evidence_bytes(path)
        files[str(path.relative_to(evidence))] = hashlib.sha256(data).hexdigest()
    return files


RESIZE_EDITOR = 'app::tilemap_resize_tests::actual_cli_resize_save_reopen_play_literal_cells'
RESIZE_BEFORE = [10, None, 20, 11, 20, None]
RESIZE_GROWN = [10, None, 20, None, None, 11, 20, None, None, None, None, None, None, None, None]
RESIZE_PAINTED = [*RESIZE_GROWN[:-1], 11]
RESIZE_CELLS = [10, None, 20, None, 11, 20, None, None]
RESIZE_WRONG = [10, None, 20, 11, 20, None, None, None]
RESIZE_APP_PHASES = ['before', 'grown', 'resize-undo', 'noop-preserves-redo',
    'resize-redo', 'paint-new-cell', 'paint-undo', 'paint-redo',
    'occupied-crop-rejected', 'erased-crop-cell', 'empty-trim', 'saved',
    'fresh-reopen', 'play']
RESIZE_PHASES = RESIZE_APP_PHASES + ['runtime-saved', 'runtime-wrong-stride']
RESIZE_IDENTITIES = {'creator_fnv1a64', 'test_source_fnv1a64', 'panel_source_fnv1a64',
    'panel_resize_source_fnv1a64', 'core_source_fnv1a64', 'source_rgba_fnv1a64'}
RESIZE_FRAME_SCOPE = ('authoring unchanged; actual App Play and saved runtime tick 1; '
    'wrong-stride and presentation-free authoritative Frame bytes equal')


def validate_resize(directory, evidence, identities, load_capture=capture, expected_adapter='llvmpipe'):
    """Artifact integrity audit only; Rust owns decoded Frame/pixel/literal assertions.

    A nondefault adapter is an explicit local audit, never the hosted main route.
    """
    def require(condition, message):
        if not condition:
            raise RuntimeError('Resize ' + message)
    def keys(value, expected, message):
        require(type(value) is dict and set(value) == set(expected), message + ' keys')
    def cells(value, expected):
        return (type(value) is list and len(value) == len(expected) and
            all((a is None if b is None else type(a) is int and a == b)
                for a, b in zip(value, expected)))
    require(type(expected_adapter) is str and bool(expected_adapter.strip()), 'empty adapter identity')
    require(set(identities) == RESIZE_IDENTITIES, 'source identity census')
    receipt = evidence_json(directory / 'receipt.json')
    partial = evidence_json(directory / 'partial-receipt.json')
    keys(receipt, RESIZE_IDENTITIES | {'completed', 'captures', 'literal_before', 'literal_grown',
        'literal_painted', 'literal_cells', 'literal_wrong_stride', 'oracle_pixels_per_runtime_capture',
        'wrong_stride_visible_pixels', 'frame_parity', 'presentation_free_frame_fnv1a64',
        'fixture_file_bindings', 'native_os_input', 'export', 'windows'}, 'final receipt')
    require(receipt['completed'] is True and all(receipt[k] is False
        for k in ('native_os_input', 'export', 'windows')), 'incomplete or overclaimed scope')
    for field, expected in [('literal_before', RESIZE_BEFORE), ('literal_grown', RESIZE_GROWN),
            ('literal_painted', RESIZE_PAINTED), ('literal_cells', RESIZE_CELLS),
            ('literal_wrong_stride', RESIZE_WRONG)]:
        require(cells(receipt[field], expected), 'literal cell contract: ' + field)
    require(type(receipt['oracle_pixels_per_runtime_capture']) is int and
        receipt['oracle_pixels_per_runtime_capture'] == 3840, 'oracle count')
    require(type(receipt['wrong_stride_visible_pixels']) is int and
        8 <= receipt['wrong_stride_visible_pixels'] <= 256, 'wrong-stride verdict')
    require(receipt['frame_parity'] == RESIZE_FRAME_SCOPE, 'Frame scope')
    for field, data in identities.items():
        require(receipt[field] == fnv1a64(data), 'source/creator fingerprint mismatch: ' + field)
    bindings = receipt['fixture_file_bindings']
    require(type(bindings) is list and bool(bindings), 'fixture provenance missing')
    paths = []
    for binding in bindings:
        keys(binding, {'path', 'bytes', 'fnv1a64'}, 'fixture binding')
        name = binding['path']
        require(type(name) is str and name and '\\' not in name and
            not Path(name).is_absolute() and all(p not in ('', '.', '..') for p in name.split('/')),
            'fixture provenance path')
        require(type(binding['bytes']) is int and 0 <= binding['bytes'] <= 64 * 1024**2 and
            type(binding['fnv1a64']) is str and re.fullmatch('[0-9a-f]{16}', binding['fnv1a64']),
            'fixture provenance fingerprint')
        paths.append(name)
    require(paths == sorted(set(paths)) and 'arena.tilemap.json' in paths and 'orr.project.json' in paths,
            'fixture provenance census/order')
    rows = receipt['captures']
    require(type(rows) is list and all(type(r) is dict for r in rows) and
        [r.get('name') for r in rows] == RESIZE_PHASES, 'capture census/order')
    expected_partial = json.loads(json.dumps(rows))
    expected_partial[-1]['pixel_verdict_pending'] = True
    expected_partial[-1].pop('oracle_pixels_checked', None)
    require(json.dumps(partial, sort_keys=True) == json.dumps(
        {'completed': False, 'captures': expected_partial, 'native_os_input': False}, sort_keys=True),
            'partial capture history')
    expected_files = {'receipt.json', 'partial-receipt.json', 'creator.log',
        'saved.tilemap.json', 'wrong-stride.tilemap.json', 'presentation-free.frame'}
    frames, pixels, checksums = {}, {}, {}
    common = {'name', 'files', 'scope', 'size', 'rgba_fnv1a64', 'frame_fnv1a64', 'tick', 'checksum'}
    for row in rows:
        name = row['name']
        app = name in RESIZE_APP_PHASES
        keys(row, common | ({'frame_source', 'adapter', 'backend', 'device_type', 'cells', 'dimensions'}
            if app else {'pixel_verdict_pending', 'oracle_pixels_checked'}), 'capture ' + name)
        suffixes = ['.rgba', '.png', '.frame'] + (['-app.png'] if app else [])
        require(row['files'] == [name + suffix for suffix in suffixes], 'file manifest')
        expected_files.update(row['files'])
        size = row['size']
        require(type(size) is list and len(size) == 2 and
            all(type(n) is int and 1 <= n <= 4096 for n in size), 'capture dimensions')
        rgba = evidence_bytes(directory / (name + '.rgba'))
        require(len(rgba) == size[0] * size[1] * 4 and fnv1a64(rgba) == row['rgba_fnv1a64'],
                'raw RGBA fingerprint/size')
        evidence_bytes(directory / (name + '.png'))
        require(load_capture(directory / (name + '.png'), tuple(size))[1] == rgba, 'PNG/raw RGBA mismatch')
        frame = evidence_bytes(directory / (name + '.frame'))
        require(fnv1a64(frame) == row['frame_fnv1a64'], 'Frame fingerprint')
        tick = 1 if name in ('play', 'runtime-saved', 'runtime-wrong-stride') else 0
        require(type(row['tick']) is int and row['tick'] == tick and
            type(row['checksum']) is int and 0 <= row['checksum'] < 2**64, 'Frame identity')
        if app:
            require(row['scope'] == 'actual App viewport; authoring grid, no pixel verdict', 'App scope')
            require(type(row['adapter']) is str and expected_adapter.lower() in row['adapter'].lower()
                and row['backend'] == 'Vulkan' and row['device_type'] == 'Cpu',
                'hosted llvmpipe required; local SwiftShader is not equivalent')
            require(row['frame_source'] == 'watch.subscribe source=view matched to predicted tick/checksum',
                'actual authoritative Frame source')
            if name in ('before', 'resize-undo', 'noop-preserves-redo'):
                literal, dims = RESIZE_BEFORE, [3, 2]
            elif name in ('paint-new-cell', 'paint-redo', 'occupied-crop-rejected'):
                literal, dims = RESIZE_PAINTED, [5, 3]
            elif name in ('empty-trim', 'saved', 'fresh-reopen', 'play'):
                literal, dims = RESIZE_CELLS, [4, 2]
            else:
                literal, dims = RESIZE_GROWN, [5, 3]
            require(cells(row['cells'], literal) and cells(row['dimensions'], dims), 'App literal grid/dimensions')
            evidence_bytes(directory / (name + '-app.png'))
            load_capture(directory / (name + '-app.png'), (1800, 2000))
        else:
            require(row['scope'] == 'actual saved PreparedRuntime PlayHost production compositor', 'runtime scope')
            require(size == [224, 224] and row['pixel_verdict_pending'] is False and
                type(row['oracle_pixels_checked']) is int and row['oracle_pixels_checked'] == 3840,
                'runtime pixel oracle incomplete')
        frames[name], pixels[name], checksums[name] = frame, rgba, row['checksum']
    for group in (RESIZE_APP_PHASES[:-1], RESIZE_PHASES[-3:]):
        require(all(frames[n] == frames[group[0]] and checksums[n] == checksums[group[0]] for n in group),
                'actual Frame byte/checksum parity mismatch')
    bare = evidence_bytes(directory / 'presentation-free.frame')
    require(fnv1a64(bare) == receipt['presentation_free_frame_fnv1a64'] and bare == frames['play'],
            'presentation-free Frame parity')
    good, wrong = pixels['runtime-saved'], pixels['runtime-wrong-stride']
    changed = sum(any(abs(good[i+c] - wrong[i+c]) > 3 for c in range(4))
        for y in range(48, 64) for x in range(104, 120) for i in [(y * 224 + x) * 4])
    require(changed == receipt['wrong_stride_visible_pixels'], 'retained wrong-stride pixel verdict')
    for filename, literal in [('saved.tilemap.json', RESIZE_CELLS), ('wrong-stride.tilemap.json', RESIZE_WRONG)]:
        doc = evidence_json(directory / filename)
        keys(doc, {'version', 'sprite', 'width', 'height', 'tile_size', 'origin', 'units_per_pixel', 'cells'},
             'retained map')
        require(type(doc['version']) is int and doc['version'] == 1 and
            type(doc['width']) is int and doc['width'] == 4 and
            type(doc['height']) is int and doc['height'] == 2 and cells(doc['cells'], literal) and
            cells(doc['tile_size'], [16, 16]) and doc['origin'] == [-56., 144.] and
            type(doc['units_per_pixel']) in (int, float) and doc['units_per_pixel'] == 1.0 and
            doc['sprite'] == {'package': 'sample-sprites', 'document': 'sprites.json'}, 'retained literal map/metadata')
    require({p.name for p in directory.iterdir()} == expected_files, 'retained artifact census')
    files = {}
    for name in sorted(expected_files):
        path = directory / name
        if name == 'creator.log':
            require(path.is_file() and not path.is_symlink() and path.stat().st_size <= 64 * 1024**2,
                    'missing creator log')
            data = path.read_bytes()
        else:
            data = evidence_bytes(path)
        files[str(path.relative_to(evidence))] = hashlib.sha256(data).hexdigest()
    return files


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('cpu', 'gpu'))
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != 'linux':
        raise RuntimeError('This gate certifies Linux only; existing Windows gates remain unchanged')
    root = Path.cwd().resolve()
    evidence = args.evidence
    if not evidence.is_absolute():
        raise RuntimeError('Evidence root must be absolute')
    if args.mode == 'cpu':
        evidence.mkdir(parents=True, exist_ok=False)
    phase = evidence / args.mode
    phase.mkdir(exist_ok=False)
    helper = runpy.run_path(str(root / 'tools/check-authoring-controls.py'))

    def run(label, command):
        return helper['run_logged'](command, phase / (label + '.log')).replace('\r\n', '\n')

    def inventory(label, binary, prefix, expected, ignored):
        for kind, names, extra in [('all', expected, []), ('ignored', ignored, ['--ignored'])]:
            output = run(label + '-' + kind, [binary, prefix, '--list', '--format=pretty', '--color=never', *extra])
            validate_inventory(output, names)

    def cases(label, binary, selector, expected, ignored=False, exact=False):
        output = run(label, [binary, selector, '--format=pretty', '--color=never', '--show-output',
                             '--test-threads=1', *(['--ignored'] if ignored else []),
                             *(['--exact'] if exact else [])])
        validate_cases(output, expected)
        return output

    manifest = evidence / 'executables.json'
    sources = evidence / 'sources.json'
    initial_sources = source_fence(root)
    if args.mode == 'cpu':
        sources.write_text(json.dumps(initial_sources, indent=2))
    elif evidence_json(sources) != initial_sources:
        raise RuntimeError('Scoped acceptance sources changed since CPU build')

    def fenced_cases(*args, **kwargs):
        if source_fence(root) != initial_sources:
            raise RuntimeError('Scoped acceptance sources changed before test execution')
        binary = Path(args[1])
        built = evidence_json(manifest)
        matching = [v['sha256'] for v in built.values() if v['path'] == str(binary)]
        if matching != [sha(binary)]:
            raise RuntimeError('Test executable changed before execution')
        result = cases(*args, **kwargs)
        if matching != [sha(binary)]:
            raise RuntimeError('Test executable changed during execution')
        if source_fence(root) != initial_sources:
            raise RuntimeError('Scoped acceptance sources changed during test execution')
        return result

    if args.mode == 'cpu':
        bins = {}
        for package, targets in [
            ('orr_tilemap', ['--lib']),
            ('orr_package', ['--lib']),
            ('orr_sample', ['--lib', '--test', 'project_tilemap', '--test', 'project_tilemap_compositor', '--test', 'project_tilemap_runtime']),
            ('orr_editor', ['--lib']),
        ]:
            command = ['cargo', 'test', '--locked', '--release', '-p', package, '--no-default-features',
                       *([] if package in ('orr_tilemap', 'orr_package') else ['--features', FEATURES]),
                       *targets, '--no-run', '--message-format=json']
            output = run('build-' + package, command)
            for line in output.splitlines():
                if not line.startswith('{'):
                    continue
                item = json.loads(line)
                if item.get('reason') == 'compiler-artifact' and item.get('profile', {}).get('test') and item.get('executable'):
                    name = item['target']['name']
                    if name in bins:
                        raise RuntimeError('Duplicate test executable: ' + name)
                    bins[name] = str(Path(item['executable']).resolve())
        expected_targets = {'orr_tilemap', 'orr_package', 'orr_sample', 'orr_editor', 'project_tilemap', 'project_tilemap_compositor', 'project_tilemap_runtime'}
        if set(bins) != expected_targets:
            raise RuntimeError('Unexpected executable build inventory')
        run('build-creator', ['cargo', 'build', '--locked', '--release', '-p', 'orr_sample', '--no-default-features', '--features', FEATURES, '--bin', 'orr_new_arena'])
        target = Path(os.environ.get('CARGO_TARGET_DIR', root / 'target')).resolve()
        bins['creator'] = str(target / 'release/orr_new_arena')
        manifest.write_text(json.dumps({k: {'path': v, 'sha256': sha(Path(v))} for k, v in bins.items()}, indent=2))
        inventory('consumer', bins['orr_sample'], 'project_tilemap::tests::', CONSUMER, [])
        inventory('package', bins['orr_package'], 'tests::tilemap_', PACKAGE, [])
        inventory('core', bins['orr_tilemap'], '', CORE, [])
        inventory('admission', bins['project_tilemap'], '', ADMISSION, [])
        inventory('compositor', bins['project_tilemap_compositor'], '', [RECORDING, COMPOSITOR], [])
        inventory('runtime', bins['project_tilemap_runtime'], '', [RUNTIME], [RUNTIME])
        inventory('panel', bins['orr_editor'], 'tilemap_panel::tests::', PANEL, [])
        inventory('editor', bins['orr_editor'], 'app::tilemap_tests::', [EDITOR], [EDITOR])
        inventory('fill', bins['orr_editor'], 'tilemap_panel::fill', FILL, [])
        inventory('resize', bins['orr_editor'], 'tilemap_panel::resize', RESIZE, [])
        inventory('resize-editor', bins['orr_editor'], 'app::tilemap_resize_tests::', [RESIZE_EDITOR], [RESIZE_EDITOR])
        inventory('fill-editor', bins['orr_editor'], 'app::tilemap_fill_tests::', [FILL_EDITOR], [FILL_EDITOR])
        inventory('creator', bins['orr_sample'], 'project_create::tilemap::tests::', [CREATOR], [])
        for label, target_name, selector, expected, exact in [
            ('consumer', 'orr_sample', 'project_tilemap::tests::', CONSUMER, False),
            ('package', 'orr_package', 'tests::tilemap_', PACKAGE, False),
            ('core', 'orr_tilemap', '', CORE, False),
            ('admission', 'project_tilemap', '', ADMISSION, False),
            ('recording', 'project_tilemap_compositor', RECORDING, [RECORDING], True),
            ('panel', 'orr_editor', 'tilemap_panel::tests::', PANEL, False),
            ('fill', 'orr_editor', 'tilemap_panel::fill', FILL, False),
            ('resize', 'orr_editor', 'tilemap_panel::resize', RESIZE, False),
            ('creator', 'orr_sample', CREATOR, [CREATOR], True),
        ]:
            fenced_cases(label, bins[target_name], selector, expected, exact=exact)
        run('strict-libs', ['cargo', 'clippy', '--locked', '-p', 'orr_tilemap', '-p', 'orr_sample', '-p', 'orr_editor', '--no-default-features', '--features', 'orr_sample/tilemap,orr_sample/project-create,orr_editor/tilemap,orr_editor/project-create', '--lib', '--', '-D', 'warnings'])
        if source_fence(root) != initial_sources:
            raise RuntimeError('Scoped acceptance sources changed during CPU gate')
        (evidence / 'cpu-complete.json').write_text(json.dumps({'passed': CPU_COUNT, 'executables_sha256': sha(manifest), 'sources_sha256': sha(sources)}))
        return

    complete = json.loads((evidence / 'cpu-complete.json').read_text())
    if complete != {'passed': CPU_COUNT, 'executables_sha256': sha(manifest), 'sources_sha256': sha(sources)}:
        raise RuntimeError('CPU gate did not finish with the exact executable manifest')
    records = json.loads(manifest.read_text())
    bins = {}
    for name, record in records.items():
        path = Path(record['path'])
        if not path.is_absolute() or sha(path) != record['sha256']:
            raise RuntimeError('Missing or changed current-build executable: ' + name)
        bins[name] = str(path)
    icd = Path('/usr/share/vulkan/icd.d/lvp_icd.x86_64.json')
    if not icd.is_file():
        raise RuntimeError('Hosted Mesa lavapipe ICD is required')
    os.environ.update(VK_ICD_FILENAMES=str(icd), WGPU_BACKEND='vulkan', ORR_REQUIRE_GPU='1', ORR_NEW_ARENA_BIN=bins['creator'])
    runtime, editor, fill, resize = phase / 'runtime', phase / 'editor', phase / 'fill', phase / 'resize'
    runtime.mkdir(); editor.mkdir(); fill.mkdir(); resize.mkdir()
    os.environ.update(ORR_TILEMAP_RUNTIME_EVIDENCE=str(runtime), ORR_TILEMAP_EDITOR_EVIDENCE=str(editor), ORR_TILEMAP_FILL_EVIDENCE=str(fill), ORR_TILEMAP_RESIZE_EVIDENCE=str(resize))
    output = fenced_cases('compositor', bins['project_tilemap_compositor'], COMPOSITOR, [COMPOSITOR], exact=True)
    adapter_lines = re.findall(r'^tile background GPU: (.+)$', output, re.M)
    if len(adapter_lines) != 1 or 'llvmpipe' not in adapter_lines[0].lower():
        raise RuntimeError('Compositor did not record hosted llvmpipe')
    fenced_cases('runtime', bins['project_tilemap_runtime'], RUNTIME, [RUNTIME], ignored=True, exact=True)
    fenced_cases('editor', bins['orr_editor'], EDITOR, [EDITOR], ignored=True, exact=True)

    fenced_cases('fill-editor', bins['orr_editor'], FILL_EDITOR, [FILL_EDITOR], ignored=True, exact=True)

    fenced_cases('resize-editor', bins['orr_editor'], RESIZE_EDITOR, [RESIZE_EDITOR], ignored=True, exact=True)

    files = validate_receipts(runtime, editor, evidence, records['creator']['sha256'],
                              sha(root / 'assets/sprite_demo/lantern_keeper.rgba'))
    identities = {
        'creator_fnv1a64': Path(bins['creator']).read_bytes(),
        'test_source_fnv1a64': (root / 'crates/orr_editor/src/app/tilemap_fill_tests.rs').read_bytes(),
        'panel_source_fnv1a64': (root / 'crates/orr_editor/src/tilemap_panel.rs').read_bytes(),
        'panel_fill_source_fnv1a64': (root / 'crates/orr_editor/src/tilemap_panel/fill.rs').read_bytes(),
        'core_source_fnv1a64': (root / 'crates/orr_tilemap/src/lib.rs').read_bytes(),
        'source_rgba_fnv1a64': (root / 'assets/sprite_demo/lantern_keeper.rgba').read_bytes(),
    }
    files.update(validate_fill(fill, evidence, identities))
    resize_identities = {k: v for k, v in identities.items() if k != 'panel_fill_source_fnv1a64'}
    resize_identities['test_source_fnv1a64'] = (root / 'crates/orr_editor/src/app/tilemap_resize_tests.rs').read_bytes()
    resize_identities['panel_resize_source_fnv1a64'] = (root / 'crates/orr_editor/src/tilemap_panel/resize.rs').read_bytes()
    files.update(validate_resize(resize, evidence, resize_identities))
    (phase / 'verified.json').write_text(json.dumps({'gpu_tests_passed': 5, 'capture_sha256': files,
        'scope': 'Linux hosted llvmpipe; no native OS input, exported player, or Windows GPU claim'}, indent=2))


if __name__ == '__main__':
    main()
