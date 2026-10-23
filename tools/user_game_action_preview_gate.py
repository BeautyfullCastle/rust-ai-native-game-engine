#!/usr/bin/env python3
"""Hosted CPU-only checks for an explicitly pinned generated action preview.

No window, hardware input, GPU, source-hidden delivery or native acceptance claim.
The legacy generated-user native gate remains separate and mandatory.
"""
import argparse
import json
import os
from pathlib import Path
import re
import sys

sys.dont_write_bytecode = True
import bootstrap_user_game as bootstrap
import static_game_native_gate as source
import static_game_standalone_gate as command_gate
import static_game_actions_native_gate as artifacts_gate
import user_game_native_gate as shared

SCHEMA = 'orr.generated-action-preview-cpu/1'
PROFILE = 'multi-action-preview'
PACKAGE = 'orr_generated_action_preview'
ACTION_TESTS = {
    'frozen_domain_lifecycle_and_replay_are_exact',
    'nonneutral_and_oversized_profiles_are_rejected_before_play',
    'invalid_typed_domain_and_mismatched_json_are_rejected',
}
LEGACY_TESTS = {
    'both_games_start_paused_and_require_a_fresh_neutral_boundary',
    'both_games_cancel_opposing_keys_and_clear_releases',
    'both_games_focus_loss_pauses_without_a_hidden_tick_and_resumes_neutral',
    'both_games_escape_pauses_without_a_hidden_tick_and_resumes_neutral',
    'both_games_all_eighteen_held_values_round_trip_through_the_typed_trace_decoder',
    'both_games_record_actual_consumed_inputs_and_replay_every_tick_past_two_keyframes',
    'both_games_reject_wrong_replay_identity_and_unbounded_or_noncanonical_envelopes',
    'both_games_reject_inputs_outside_the_finite_held_contract',
    'both_games_verify_future_keyframes_against_independent_admitted_simulation',
    'options_reject_duplicate_relative_and_conflicting_output_queries',
}
MODES = (
    ('feature-off', 'standalone', 'standalone_contract', LEGACY_TESTS),
    ('actions', 'standalone,static-game-actions', 'multi_action_contract', ACTION_TESTS),
    ('editor-actions', 'editor,standalone,static-game-actions', 'multi_action_contract',
     ACTION_TESTS | {'editor_companion_matches_frozen_typed_admission'}),
)
TARGETS = {
    'orr_static_starter': ('src/lib.rs', 'lib'),
    'standalone_contract': ('tests/standalone_contract.rs', 'test'),
    'multi_action_contract': ('tests/multi_action_contract.rs', 'test'),
    'player': ('src/bin/player.rs', 'bin'),
    'editor': ('src/bin/editor.rs', 'bin'),
    'delivery-player': ('src/bin/delivery-player.rs', 'bin'),
    'switchboard-player': ('src/bin/switchboard-player.rs', 'bin'),
}
require = source.require


def metadata_context(metadata, project, engine, target, user_sha, gate, gate_sha, features):
    packages = [p for p in metadata['packages'] if p['name'] == PACKAGE]
    require(len(packages) == 1, 'one generated package required')
    package = packages[0]
    require(package['manifest_path'] == str(project/'Cargo.toml'), 'wrong generated manifest')
    require(metadata['workspace_root'] == str(project), 'wrong generated workspace')
    require(metadata['workspace_members'] == [package['id']], 'extra generated workspace member')
    require(metadata['target_directory'] == str(target), 'wrong dedicated target directory')
    nodes = metadata['resolve']['nodes']
    require(len({n['id'] for n in nodes}) == len(nodes), 'duplicate resolution node')
    root = [n for n in nodes if n['id'] == package['id']]
    require(len(root) == 1 and metadata['resolve']['root'] == package['id'], 'wrong resolution root')
    for feature in ('editor', 'static-game-actions', 'standalone'):
        require((feature in root[0]['features']) == (feature in features.split(',')), 'wrong selected feature '+feature)
    require('default' not in root[0]['features'], 'default features must be explicitly disabled')
    resolved = {n['id'] for n in nodes}
    editor_packages = [p for p in metadata['packages'] if p['name'] == 'orr_editor' and p['id'] in resolved]
    require(bool(editor_packages) == ('editor' in features.split(',')), 'editor dependency feature isolation')
    for dependency in metadata['packages']:
        if dependency['id'] in resolved and dependency.get('source') is None and dependency['id'] != package['id']:
            require(engine in Path(dependency['manifest_path']).parents, 'local dependency outside pinned engine')
    targets = {}
    for name, (relative, kind) in TARGETS.items():
        matches = [t for t in package['targets'] if t['name'] == name and t['kind'] == [kind]]
        require(len(matches) == 1 and matches[0]['src_path'] == str(project/relative), 'wrong generated target '+name)
        targets[name] = project/relative
    return source.ExecutionContext(project, PACKAGE, package['id'], targets, user_sha, gate, gate_sha, target_root=target)


def mode_library(output, context, features):
    """Bind compiler producers to this exact feature mode and release library."""
    expected = set(features.split(','))
    library = None
    for line in output.splitlines():
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if value.get('reason') != 'compiler-artifact' or value.get('package_id') != context.package_id:
            continue
        selected = value.get('features', [])
        require(len(selected) == len(expected) and set(selected) == expected,
                'compiler producer features differ from selected mode')
        if value.get('target', {}).get('kind') == ['lib']:
            profile = value.get('profile', {})
            require(profile.get('opt_level') == '3' and profile.get('test') is False,
                    'generated library must be release non-test')
            library = {'features': sorted(selected), 'profile': profile,
                       'source_sha256': source.digest(Path(value['target']['src_path']))}
    result = context.library_artifact(output)
    require(library is not None, 'generated library mode evidence missing')
    result.update(library)
    return result


def test_result(output, expected):
    command_gate.test_summary(output, len(expected), 0)
    completed = re.findall(r'(?m)^test (\S+) \.\.\. ok$', output)
    require(len(completed) == len(expected) and set(completed) == set(expected), 'exact portable tests must execute')


def validate_creation_receipt(receipt, engine):
    require(receipt['schema'] == 1 and receipt['template_profile'] == PROFILE, 'wrong preview creation profile')
    require(receipt['engine_commit'] == bootstrap.ACTION_PREVIEW_COMMIT
            and receipt['engine_tree'] == bootstrap.ACTION_PREVIEW_TREE, 'wrong preview engine identity')
    require(receipt['engine_path'] == str(engine) and receipt['project_name'] == PACKAGE, 'wrong preview project identity')
    require(receipt['stability'] == 'preview' and receipt['hosted_native_acceptance'] == 'unverified'
            and receipt['action_feature_default'] is False, 'preview must not certify acceptance/default enablement')


def run(gate, engine, project, target, artifacts):
    gate, engine, project, target, artifacts = map(Path, (gate, engine, project, target, artifacts))
    shared.separate_roots(gate, engine, project, target, artifacts)
    require(' ' in str(project), 'generated project path must exercise spaces')
    require(not project.exists() and not target.exists() and not artifacts.exists(), 'fresh output paths required')
    artifacts.mkdir(parents=True)
    receipt = {'schema': SCHEMA, 'status': 'started', 'native_acceptance': False,
               'scope': 'generated project portable CPU contracts only', 'modes': {}}
    def save():
        (artifacts/'gate.json').write_text(json.dumps(receipt, indent=2)+'\n')
    save()
    try:
        env = shared.build_environment(os.environ, target)
        gate_sha = env.get('GITHUB_SHA', '')
        require(re.fullmatch('[0-9a-f]{40}', gate_sha)
                and all(env.get(k, '').isdecimal() for k in ('GITHUB_RUN_ID', 'GITHUB_RUN_ATTEMPT')),
                'hosted source/run identity required')
        require(not any(env.get(k) for k in ('ORR_STATIC_STANDALONE_BUILD_SHA', 'ORR_STATIC_NATIVE_BUILD_SHA',
                                           'ORR_ACCEPTANCE_CONTEXT', 'ORR_ACCEPTANCE_CONTEXT_SHA256')),
                'unexpected inherited build/context identity')
        gate_source = source.clean_checkout(gate_sha, gate, complete=True)
        for root in (engine, project, target):
            source.reject_ancestor_config(root)
        inventory = source.json_file(gate/'tools/user_game_action_preview_source.json')
        require(not (engine/'target').exists() and not (engine/'target').is_symlink(), 'engine target tree must be absent')
        bootstrap.validate_engine(engine, inventory, profile=PROFILE)
        receipt.update(hosted_gate_commit=gate_sha, engine_commit=bootstrap.ACTION_PREVIEW_COMMIT,
                       engine_tree=bootstrap.ACTION_PREVIEW_TREE, run_id=env['GITHUB_RUN_ID'],
                       run_attempt=env['GITHUB_RUN_ATTEMPT'])
        receipt['bootstrap'] = bootstrap.bootstrap(engine, project, PACKAGE, inventory, profile=PROFILE)
        creation = source.json_file(project/'provenance.json')
        validate_creation_receipt(creation, engine)
        for path, digest in creation['files_sha256'].items():
            require(source.digest(project/path) == digest, 'generated creation file differs '+path)
        receipt['creation_sha256'] = source.digest(project/'provenance.json')
        execute = lambda args, name, seconds: command_gate.command(args, artifacts, name, env, seconds, cwd=project)
        execute(['cargo', 'generate-lockfile', '--offline'], 'own-lock', 180)
        require((project/'Cargo.lock').is_file() and not (project/'Cargo.lock').is_symlink(), 'own lock missing/linked')
        lock_hash = source.digest(project/'Cargo.lock')
        user_sha = shared.own_commit(project, env)
        require(user_sha not in (gate_sha, bootstrap.ACTION_PREVIEW_COMMIT), 'user/gate/engine source conflation')
        user_source = source.clean_checkout(user_sha, project, complete=True)
        env.update(ORR_STATIC_STANDALONE_BUILD_SHA=user_sha, ORR_STATIC_NATIVE_BUILD_SHA=user_sha)
        receipt.update(user_source_commit=user_sha, user_source_tree=user_source['tree'], own_lock_sha256=lock_hash)
        for name, value in [('gate-source', gate_source), ('user-source', user_source)]:
            path = artifacts/(name+'.json')
            path.write_text(json.dumps(value, indent=2)+'\n')
            receipt[name+'_sha256'] = source.digest(path)
        def fence():
            require(source.clean_checkout(gate_sha, gate, complete=True) == gate_source, 'gate source changed')
            require(not (engine/'target').exists() and not (engine/'target').is_symlink(), 'engine target appeared')
            bootstrap.validate_engine(engine, inventory, profile=PROFILE)
            require(source.clean_checkout(user_sha, project, complete=True) == user_source, 'user source changed')
            require(source.digest(project/'Cargo.lock') == lock_hash, 'user lock changed')
        fence()
        save()
        for mode, features, test, names in MODES:
            fence()
            metadata = json.loads(execute(['cargo', 'metadata', '--offline', '--locked', '--format-version=1',
                '--no-default-features', '--features', features], mode+'-metadata', 90))
            context = metadata_context(metadata, project, engine, target, user_sha, gate, gate_sha, features)
            output = execute(['cargo', 'test', '--offline', '--locked', '--release', '-j1', '-p', PACKAGE,
                '--no-default-features', '--features', features, '--test', test, '--no-run',
                '--message-format=json-render-diagnostics'], mode+'-compile', 1200)
            expected = {test: 'test', 'player': 'bin', 'delivery-player': 'bin', 'switchboard-player': 'bin'}
            if 'editor' in features.split(','):
                expected['editor'] = 'bin'
            producers = artifacts_gate.producers(output, context, expected)
            entry = {'features': features, 'package_id': context.package_id,
                     'library_producer': mode_library(output, context, features), 'producers': producers}
            receipt['modes'][mode] = entry
            save()
            executable = producers[test]['path']
            listing = execute([executable, '--list'], mode+'-inventory', 15)
            artifacts_gate.test_inventory(listing, names)
            result = execute([executable, '--test-threads=1', '--nocapture'], mode+'-tests', 60)
            test_result(result, names)
            for producer in producers.values():
                require(source.digest(Path(producer['path'])) == producer['sha256'], 'compiled producer changed')
            for file in entry['library_producer']['files']:
                require(source.digest(Path(file['path'])) == file['sha256'], 'compiled library changed')
            fence()
            entry.update(status='passed', tests=len(names), inventory_sha256=source.digest(artifacts/(mode+'-inventory.log')),
                         tests_sha256=source.digest(artifacts/(mode+'-tests.log')))
            save()
        receipt.update(status='passed', portable_cpu_acceptance=True, native_acceptance=False)
    except Exception as error:
        receipt.update(status='failed', error=str(error))
        raise
    finally:
        save()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('gate', 'engine', 'project', 'target', 'artifacts'):
        parser.add_argument('--'+name, type=Path, required=True)
    args = parser.parse_args()
    try:
        run(args.gate, args.engine, args.project, args.target, args.artifacts)
    except Exception as error:
        print('generated action preview CPU gate FAILED: '+str(error), file=sys.stderr)
        sys.exit(1)
