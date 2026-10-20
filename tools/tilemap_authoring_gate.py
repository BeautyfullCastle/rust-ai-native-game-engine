#!/usr/bin/env python3
"""Issue258 bounded, exact-inventory CPU + hosted llvmpipe acceptance.

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
CPU_COUNT = 25
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
        inventory('creator', bins['orr_sample'], 'project_create::tilemap::tests::', [CREATOR], [])
        for label, target_name, selector, expected, exact in [
            ('consumer', 'orr_sample', 'project_tilemap::tests::', CONSUMER, False),
            ('package', 'orr_package', 'tests::tilemap_', PACKAGE, False),
            ('core', 'orr_tilemap', '', CORE, False),
            ('admission', 'project_tilemap', '', ADMISSION, False),
            ('recording', 'project_tilemap_compositor', RECORDING, [RECORDING], True),
            ('panel', 'orr_editor', 'tilemap_panel::tests::', PANEL, False),
            ('creator', 'orr_sample', CREATOR, [CREATOR], True),
        ]:
            cases(label, bins[target_name], selector, expected, exact=exact)
        run('strict-libs', ['cargo', 'clippy', '--locked', '-p', 'orr_tilemap', '-p', 'orr_sample', '-p', 'orr_editor', '--no-default-features', '--features', 'orr_sample/tilemap,orr_sample/project-create,orr_editor/tilemap,orr_editor/project-create', '--lib', '--', '-D', 'warnings'])
        (evidence / 'cpu-complete.json').write_text(json.dumps({'passed': CPU_COUNT, 'executables_sha256': sha(manifest)}))
        return

    complete = json.loads((evidence / 'cpu-complete.json').read_text())
    if complete != {'passed': CPU_COUNT, 'executables_sha256': sha(manifest)}:
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
    runtime, editor = phase / 'runtime', phase / 'editor'
    runtime.mkdir(); editor.mkdir()
    os.environ.update(ORR_TILEMAP_RUNTIME_EVIDENCE=str(runtime), ORR_TILEMAP_EDITOR_EVIDENCE=str(editor))
    output = cases('compositor', bins['project_tilemap_compositor'], COMPOSITOR, [COMPOSITOR], exact=True)
    adapter_lines = re.findall(r'^tile background GPU: (.+)$', output, re.M)
    if len(adapter_lines) != 1 or 'llvmpipe' not in adapter_lines[0].lower():
        raise RuntimeError('Compositor did not record hosted llvmpipe')
    cases('runtime', bins['project_tilemap_runtime'], RUNTIME, [RUNTIME], ignored=True, exact=True)
    cases('editor', bins['orr_editor'], EDITOR, [EDITOR], ignored=True, exact=True)

    files = validate_receipts(runtime, editor, evidence, records['creator']['sha256'],
                              sha(root / 'assets/sprite_demo/lantern_keeper.rgba'))
    (phase / 'verified.json').write_text(json.dumps({'gpu_tests_passed': 3, 'capture_sha256': files,
        'scope': 'Linux hosted llvmpipe; no native OS input, exported player, or Windows GPU claim'}, indent=2))


if __name__ == '__main__':
    main()
