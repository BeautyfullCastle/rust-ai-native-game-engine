#!/usr/bin/env python3
"""Issue259: one Linux x86_64 source-bound CPU/render/editor acceptance batch.

Reuse existing bounded process logging, immutable producer pinning, complete
source inventory, and stderr-interleaved libtest parsing. Never retries a failure.
"""
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import sys
import traceback

import animation_pose_bake_gate as source
import static_model_extract_gate as runner
from tilemap_authoring_gate import SUMMARY, validate_cases

CPU = [
    'independent_f64_clamps_quaternions_globals_palettes_and_weighted_vertices',
    'preserves_original_content_and_deterministic_direct_input_provenance',
    'rejects_strict_recipe_invalid_transitions_targets_and_names_as_group',
    'rejects_shift_collapse_duration_overflow_and_combined_budgets_but_allows_single_key',
    'actual_cli_writes_deterministically_without_overwrite_or_symlink_following',
]
CLI = CPU[-1]
PURE_CPU = CPU[:-1]
RENDER = 'actual_cli_installed_sequence_has_independent_visible_transition'
EDITOR = 'actual_cli_room_yard_widgets_save_reopen_play_and_authority_parity'
REQUIRED = [
    'crates/orr_model/examples/animation_clip_sequence.rs',
    'crates/orr_model/examples/animation_clip_sequence/authoring.rs',
    'crates/orr_model/tests/animation_clip_sequence.rs',
    'crates/orr_model/tests/common/animation_clip_sequence_oracle.rs',
    'crates/orr_render/tests/animation_clip_sequence.rs',
    'crates/orr_editor/tests/animation_clip_sequence_consumer.rs',
    'docs/animation-clip-sequence.md',
    'tools/animation_clip_sequence_gate.py',
    'tools/test_animation_clip_sequence_gate.py',
    'tools/static_model_extract_gate.py',
    'tools/animation_pose_bake_gate.py',
    'tools/tilemap_authoring_gate.py',
    '.github/workflows/animation-pose-bake.yml',
]


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def read(path):
    require(path.is_file() and not path.is_symlink(), f'Missing regular evidence: {path}')
    require(path.stat().st_size <= 64 * 1024**2, f'Oversized evidence: {path}')
    return path.read_bytes()


def document(path):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, 'Duplicate receipt key')
            result[key] = value
        return result
    return json.loads(read(path), object_pairs_hook=unique)


def fields(value, expected):
    require(all(type(value.get(k)) is type(v) and value[k] == v for k, v in expected.items()),
            'Incomplete or inconsistent receipt')


def adapter(value):
    fields(value, {'backend': 'Vulkan', 'device_type': 'Cpu'})
    require(isinstance(value.get('name'), str) and 'llvmpipe' in value['name'].lower(),
            'Hosted llvmpipe required; local SwiftShader is separate evidence')


def rgba(root, name, size=None):
    raw = read(root / (name + '.rgba'))
    ppm = read(root / (name + '.ppm'))
    match = re.match(rb'P6\n([1-9][0-9]*) ([1-9][0-9]*)\n255\n', ppm)
    require(match is not None, 'Missing PPM dimensions')
    dims = tuple(map(int, match.groups()))
    require(max(dims) <= 8192 and (size is None or dims == size), 'Wrong capture dimensions')
    require(len(raw) == dims[0] * dims[1] * 4, 'Incomplete RGBA capture')
    require(ppm[match.end():] == b''.join(raw[i:i+3] for i in range(0, len(raw), 4)),
            'PPM/raw capture mismatch')
    return raw


def cli_receipt(root, cli):
    value = document(root / 'started.json')
    fields(value, {'status': 'partial', 'exit': 0, 'cli': cli})
    require(b'published ' in read(root / 'cli.stdout'), 'Actual CLI publication missing')
    require(read(root / 'cli.stderr') == b'', 'Unexpected CLI stderr')
    require(not (root / 'inputs').exists() and (root / 'inputs-hidden').is_dir(), 'Inputs not hidden')


def receipts(render, editor, cli):
    cli_receipt(render, cli)
    cli_receipt(editor, cli)
    r = document(render / 'acceptance.json')
    fields(r, {'status': 'passed', 'actual_cli': True, 'inputs_hidden': True})
    adapter(r.get('adapter', {}))
    require(r['adapter'] == document(render / 'adapter.json'), 'Render adapter receipts disagree')
    require([s.get('time') for s in r.get('samples', [])] == [1.0, 2.25, 2.5, 2.75, 4.0], 'Incomplete render sample census')
    for sample in r['samples']:
        coverage = sample.get('coverage')
        require(isinstance(coverage, list) and len(coverage) == 2 and
                all(type(n) is int for n in coverage) and coverage[0] > 100 and coverage[1] > 20000,
                'Vacuous render coverage')
    require(r.get('controls') == document(render / 'control-plan.json'), 'Control receipts disagree')
    for key in ('wrong_order_confident_texels', 'wrong_transition_confident_texels'):
        require(type(r['controls'].get(key)) is int and r['controls'][key] > 50, 'Vacuous discriminator')
    for i in range(5):
        rgba(render, f'sample-{i}', (192, 192))
    first = rgba(render, 'sample-1', (192, 192))
    for label in ('wrong-order', 'wrong-transition-duration'):
        raw = rgba(render, label, (192, 192))
        require(sum(any(abs(a-b) > 3 for a, b in zip(first[i:i+4], raw[i:i+4]))
                    for i in range(0, len(raw), 4)) > 50, 'Control image is not discriminating')
        require(b'published ' in read(render / (label + '-cli.stdout')) and
                read(render / (label + '-cli.stderr')) == b'', 'Control CLI publication missing')
    require(rgba(render, 'empty', (192, 192)) == bytes([0, 0, 0, 255]) * 192 * 192, 'Empty image not restored')
    for name in ('room', 'yard'):
        root = editor / name
        adapter(document(root / 'adapter.json'))
        e = document(root / 'acceptance.json')
        fields(e, {'status': 'passed', 'actual_EditorApp_widgets_and_GPU': True,
                   'catalog': True, 'save_reopen': True, 'instances': 2,
                   'authority_byte_parity': True, 'inputs_hidden': True})
        partial = document(root / 'partial.json')
        fields(partial, {'status': 'partial', 'tick': 135, 'bindings': 2, 'cooked_provenance_entries': 2})
        require(type(partial.get('checksum')) is int and partial['checksum'] == partial.get('presentation_off_checksum'), 'Authority checksum mismatch')
        require(re.fullmatch('[0-9a-f]{64}', partial.get('binding_source_hash', '')) is not None, 'Missing output hash')
        require(partial['binding_source_hash'] == hashlib.sha256(read(editor / 'inputs-hidden/action.animated.json')).hexdigest(), 'Binding/output hash mismatch')
        require(len(read(root / 'checkpoint.frame')) > 0, 'Missing authoritative Frame')
        before = rgba(root, 'transition-quarter')
        after = rgba(root, 'transition-three-quarter')
        require(len(before) == len(after), 'Viewport dimensions changed')
        changed = sum(before[i:i+4] != after[i:i+4] for i in range(0, len(before), 4))
        require(type(e.get('visible_changed_pixels')) is int and e['visible_changed_pixels'] == changed and changed > 20, 'Visible movement receipt mismatch')
        require(rgba(root, 'seek-quarter') == before, 'Replayed viewport differs')
        for label in ('transition-quarter', 'transition-three-quarter', 'seek-quarter'):
            ui = read(root / (label + '-ui.rgba'))
            require(len(ui) > 0 and len(ui) % 4 == 0, 'Missing composed UI capture')


def run(label, command, expected=0):
    source.source_fence()
    result = runner.run(label, command, expected=expected, seconds=2400)
    source.source_fence()
    return result


def build(label, package, target, kind, features, expanded):
    command = ['cargo', 'build' if kind == 'example' else 'test', '--locked', '--release',
               '--no-default-features', '-p', package]
    if features:
        command += ['--features', features]
    command += ['--example' if kind == 'example' else '--test', target, '--message-format=json']
    if kind == 'test':
        command += ['--no-run']
    return runner.pin(run('build-' + label, command), label, package, target, kind, expanded)


def cases(label, binary, names, ignored=False, all_names=None, ignored_names=None, skip=None):
    all_names = names if all_names is None else all_names
    ignored_names = (names if ignored else []) if ignored_names is None else ignored_names
    for kind, extra, expected in [('all', [], all_names), ('ignored', ['--ignored'], ignored_names)]:
        runner.inventory(run(label + '-' + kind, [binary, *extra, '--list']), expected)
    flags = ['--exact', names[0], '--ignored'] if ignored else []
    if skip is not None:
        flags += ['--skip', skip]
    output = run(label, [binary, *flags, '--test-threads=1', '--show-output', '--color=never'])
    validate_cases(output, names)
    require(int(SUMMARY.findall(output.replace('\r\n', '\n'))[0][4]) == len(all_names) - len(names),
            'Unexpected filtered test count')
    runner.report['coverage'][label] = len(names)
    runner.record()


def main():
    require(__debug__ and not sys.flags.optimize, 'Nonoptimized Python required')
    require(sys.platform == 'linux' and platform.machine() == 'x86_64', 'Linux x86_64 only')
    require(os.environ.get('PYTHONDONTWRITEBYTECODE') == '1', 'Inherited no-bytecode required')
    root = Path(__file__).resolve().parents[1]
    source.source_root = root
    source.frozen = None
    destination = Path(os.environ['ORR_ANIMATION_SEQUENCE_EVIDENCE'])
    require(not destination.resolve().is_relative_to(root), 'Evidence must be outside source')
    runner.initialize(root, destination, 'gpu')
    runner.report['coverage'] = {}
    error = None
    try:
        require(run('clean-before', ['git', 'status', '--porcelain=v1', '--untracked-files=all']).strip() == '', 'Clean committed source required')
        for key, rev in [('head', 'HEAD'), ('tree', 'HEAD^{tree}')]:
            runner.report[key] = run(key, ['git', 'rev-parse', rev]).strip()
        require(os.environ.get('GITHUB_SHA', runner.report['head']) == runner.report['head'], 'Checkout mismatch')
        tracked = run('tracked', ['git', 'ls-files', '-z']).split('\0')
        require(tracked.pop() == '', 'Invalid tracked inventory')
        source.check_source_inventory(tracked, 'before')
        source.frozen = source.snapshot()
        require(set(REQUIRED) <= set(source.frozen), 'Missing feature sources')
        runner.report['source_sha256'] = source.frozen
        runner.record()
        run('gate-self-tests', [sys.executable, '-B', str(root / 'tools/test_animation_clip_sequence_gate.py')])
        off = build('disabled-cli', 'orr_model', 'animation_clip_sequence', 'example', '', [])
        require('requires --features animation,import' in run('disabled-refusal', [off, '--help'], expected=2), 'Disabled CLI did not explicitly refuse')
        cli = build('enabled-cli', 'orr_model', 'animation_clip_sequence', 'example', 'animation,import', ['animation', 'import'])
        os.environ['ORR_ANIMATION_SEQUENCE_CLI'] = cli
        model = build('model-test', 'orr_model', 'animation_clip_sequence', 'test', 'animation,import', ['animation', 'import'])
        render = build('render-test', 'orr_render', 'animation_clip_sequence', 'test', 'animation,imported-scene', ['animation', 'imported-scene', 'models'])
        editor = build('editor-test', 'orr_editor', 'animation_clip_sequence_consumer', 'test', 'room-animated-models,package-manager', ['animated-models', 'models', 'package-manager', 'room-animated-models', 'room-project'])
        cases('pure-cpu', model, PURE_CPU, all_names=CPU, ignored_names=[CLI], skip=CLI)
        cases('actual-cli', model, [CLI], ignored=True, all_names=CPU, ignored_names=[CLI])
        runner.software()
        os.environ.update(ORR_SEQUENCE_REQUIRE_SOFTWARE='1',
                          ORR_SEQUENCE_RENDER_EVIDENCE=str(destination / 'render'),
                          ORR_SEQUENCE_EDITOR_EVIDENCE=str(destination / 'editor'))
        cases('renderer', render, [RENDER], ignored=True)
        cases('editor', editor, [EDITOR], ignored=True)
        receipts(destination / 'render', destination / 'editor', cli)
        require(runner.report['coverage'] == {'pure-cpu': 4, 'actual-cli': 1, 'renderer': 1, 'editor': 1}, 'Incomplete coverage')
    except BaseException as failure:
        error = failure
    finally:
        try:
            source.source_fence()
            require(source.frozen is not None, 'Missing prebuild snapshot')
            runner.fence()
            require(run('clean-after', ['git', 'status', '--porcelain=v1', '--untracked-files=all']).strip() == '', 'Source changed')
            for key, rev in [('head', 'HEAD'), ('tree', 'HEAD^{tree}')]:
                require(run(key + '-after', ['git', 'rev-parse', rev]).strip() == runner.report[key], 'Revision changed')
            runner.report['source_unchanged'] = True
        except BaseException as failure:
            runner.report['final_fence_failure'] = repr(failure)
            error = error or failure
        # Keep partial captures/logs even when any test or final receipt fails.
        runner.report['capture_sha256'] = {str(p.relative_to(destination)): runner.digest(p)
            for folder in ('render', 'editor') if (destination / folder).is_dir()
            for p in (destination / folder).rglob('*') if p.is_file() and not p.is_symlink()}
        runner.report['status'] = 'failed' if error else 'passed'
        if error:
            runner.report['failure'] = ''.join(traceback.format_exception(error))
        runner.record()
    if error:
        raise SystemExit(error.code if isinstance(error, runner.GateFailure) else 1)


if __name__ == '__main__':
    main()
