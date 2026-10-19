#!/usr/bin/env python3
"""Mandatory source-bound assembly acceptance, standard library only.

Hosted CPU and GPU modes build retained producers in fresh targets. The reviewed
static prop runner supplies bounded output, process-tree cleanup and producer
identity; its source and independent fixtures join every source fence.
"""
import hashlib
import json
import os
import pathlib
import re
import sys
import traceback

if not __debug__ or sys.flags.optimize or os.environ.get('PYTHONOPTIMIZE') not in (None, '', '0'):
    raise SystemExit("static assembly acceptance requires nonoptimized Python")

import static_model_extract_gate as runner

FEATURE_PATHS = (
 'crates/orr_model/examples/static_model_assemble.rs',
 'crates/orr_model/examples/static_model_assemble/authoring.rs',
 'crates/orr_model/tests/static_model_assemble.rs',
 'crates/orr_render/tests/static_model_assemble.rs',
 'docs/static-model-assemble.md',
 'tools/static_model_assemble_gate.py',
 'tools/test_static_model_assemble_gate.py',
 '.github/workflows/static-model-assemble.yml',
)
INFRA_PATHS = ('Cargo.toml','Cargo.lock','tools/static_model_extract_gate.py',
 'tools/test_static_model_extract_gate.py','tools/static_model_assemble_gpu_check.py',
 'tools/test_static_model_assemble_gpu_check.py')
GPU_TEST = 'static_assembly_installed_models_match_independent_oracles'
FIXTURE_COUNT = 8
WORKFLOW_JOB = 'acceptance'
WORKFLOW_ROWS = (('cpu','ubuntu-22.04'),('cpu','windows-latest'),('gpu','ubuntu-22.04'))
WORKFLOW_EVIDENCE = '${{ runner.temp }}/orr-static-assemble-${{ matrix.mode }}-${{ github.run_id }}-${{ github.run_attempt }}'
WORKFLOW_ARTIFACT = 'static-model-assemble-${{ matrix.mode }}-${{ matrix.os }}'

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
    assert len(FEATURE_PATHS) == len(set(FEATURE_PATHS)) == 8
    required = set(FEATURE_PATHS + INFRA_PATHS)
    for package in ('orr_model', 'orr_render'):
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
    assert phase in {'cpu', 'gpu'}
    source_root = pathlib.Path(root).resolve()
    frozen = None
    mode = phase
    destination = pathlib.Path(destination)
    assert not destination.is_relative_to(source_root), 'evidence must be outside source'
    runner.initialize(source_root, destination, phase)
    runner.report['mode'] = phase
    runner.report['coverage'] = {name: 'NOT RUN' for name in ('model', 'gpu')}
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
        ('test_static_model_assemble_gate.py', f'STATIC_ASSEMBLE_GATE_FIXTURES: {FIXTURE_COUNT} passed; 0 failed; 0 skipped'),
        ('test_static_model_assemble_gpu_check.py', 'STATIC_ASSEMBLE_GPU_CHECKER_FIXTURES: 8 passed; 0 failed; 0 skipped'),
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
                              '--example', 'static_model_assemble', '--message-format=json'])
    binary = pin(output, 'assemble-cli', 'orr_model', 'static_model_assemble', 'example', ['import'])
    os.environ['ORR_STATIC_MODEL_ASSEMBLE_BIN'] = binary
    return binary


def model_tests(platform):
    assert platform in {'linux', 'darwin', 'win32'}, 'unsupported model test platform'
    names = ['aggregate_material_geometry_decoded_and_input_budgets_are_independent', 'positional_and_duplicate_part_records_are_rejected', 'actual_cli_publishes_validated_output_and_rejects_second_write', 'first_use_tables_repeated_sources_colliding_ids_and_bit_preservation', 'placement_left_multiplies_noncommuting_source_and_mirrors', 'strict_recipe_parts_aliases_types_and_byte_limits_reject_before_resolution', 'singular_nonaffine_nonfinite_and_composition_overflow_reject', 'full_source_admission_includes_unused_resources_and_aggregate_limits', 'thirty_two_repeated_parts_resolve_once_and_roundtrip_deterministically', 'cli_publishes_once_without_overwrite_and_preserves_all_inputs', 'cli_help_and_three_unique_pairs_are_bounded', 'cli_bad_arguments_fail_without_output_or_staging', 'cli_portable_output_names_and_package_store_rejection', 'cli_missing_nonregular_and_oversized_inputs_leave_no_output', 'cli_resolver_is_anchored_to_recipe_parent', 'cli_parent_traversal_preserves_raw_spelling_and_never_publishes']
    if platform == 'win32':
        names.append('cli_windows_nonlocal_prefixes_reject')
    else:
        names.extend(['cli_symlink_inputs_ancestors_and_destinations_reject',
                      'cli_non_utf8_option_names_and_logical_ids_reject',
                      'cli_unix_socket_inputs_reject_without_opening_or_publishing'])
        if platform == 'linux':
            names.append('cli_non_utf8_filesystem_inputs_work')
    assert len(names) == {'linux': 20, 'darwin': 19, 'win32': 17}[platform]
    assert len(names) == len(set(names))
    return names


def model():
    flags = cargo_flags()
    output = run('build-feature-off', ['cargo', 'build', *flags, '--example',
                                      'static_model_assemble', '--message-format=json'])
    disabled = pin(output, 'feature-off-cli', 'orr_model', 'static_model_assemble', 'example', [])
    assert 'requires --features import' in run('feature-off-exit2', [disabled, '--help'], expected=2)
    cli = build_cli()
    assert 'Usage: static_model_assemble' in run('actual-help', [cli, '--help'])
    output = run('build-model-test', ['cargo', 'test', *flags, '--features', 'import',
                                    '--test', 'static_model_assemble', '--no-run', '--message-format=json'])
    binary = pin(output, 'model-test', 'orr_model', 'static_model_assemble', 'test', ['import'])
    test('model', binary, model_tests(sys.platform))
    # Target only model library and the new source-bound fixture. Other workspace
    # acceptance (including existing socket fixtures) has its own required gates.
    run('model-library-regressions', ['cargo', 'test', *flags, '--features', 'import', '--lib'])
    run('strict-model-import', ['cargo', 'clippy', *flags, '--features', 'import',
                               '--lib', '--example', 'static_model_assemble', '--test',
                               'static_model_assemble', '--', '-D', 'warnings'])
    run('strict-model-default', ['cargo', 'clippy', *flags, '--lib', '--example',
                                'static_model_assemble', '--', '-D', 'warnings'])
    runner.report['coverage']['model'] = 'PASSED'
    runner.record()


def cpu():
    model()


def gpu():
    from static_model_assemble_gpu_check import check_gpu
    runner.software()
    build_cli()
    captures = runner.evidence / 'captures'
    assert not captures.exists()
    os.environ['ORR_STATIC_MODEL_ASSEMBLE_CAPTURE_DIR'] = str(captures)
    flags = cargo_flags('orr_render')
    output = run('build-render-test', ['cargo','test',*flags,'--features','models,imported-scene',
                                     '--test','static_model_assemble','--no-run','--message-format=json'])
    binary = pin(output,'render-test','orr_render','static_model_assemble','test',['models','imported-scene'])
    run('strict-render',['cargo','clippy',*flags,'--features','models,imported-scene',
                        '--test','static_model_assemble','--','-D','warnings'])
    test('render',binary,[GPU_TEST],ignored=True)
    runner.report['captures'] = check_gpu(captures)
    runner.report['coverage']['gpu'] = 'PASSED'


def check_coverage():
    expected = 'model' if mode == 'cpu' else 'gpu'
    assert runner.report['coverage'] == {name: 'PASSED' if name == expected else 'NOT RUN' for name in ('model','gpu')}


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
    assert len(sys.argv) == 2 and sys.argv[1] in {'cpu', 'gpu'}, (
        'usage: static_model_assemble_gate.py cpu|gpu')
    initialize(pathlib.Path(__file__).resolve().parents[1],
               pathlib.Path(os.environ['ORR_STATIC_ASSEMBLE_EVIDENCE']), sys.argv[1])
    error = None
    try:
        begin()
        {'cpu': cpu, 'gpu': gpu}[mode]()
    except BaseException as failure:
        error = failure
    finally:
        finish(error)


if __name__ == '__main__':
    main()
