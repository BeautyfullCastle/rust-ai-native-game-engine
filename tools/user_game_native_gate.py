#!/usr/bin/env python3
"""Hosted acceptance of a bootstrapped, independently committed user Cargo project.

This orchestrates the original mandatory native scenarios, never substitutes a
mock, source-only check or missing-capability skip for native acceptance.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib
import time

sys.dont_write_bytecode = True
import bootstrap_user_game as bootstrap
import static_game_native_gate as editor
import static_game_standalone_gate as standalone

SCHEMA = 'orr.user-game-native/1'
CONTEXT_SCHEMA = 'orr.native-execution-context/1'
PACKAGE = 'orr_independent_acceptance'
STARTER = Path('examples/static-game-starter')
FIXTURES = ('delivery', 'switchboard', 'editor_delivery', 'editor_switchboard')
OVERLAY = ('tests/native_window.rs', 'tests/standalone_x11.rs',
           'tests/standalone_editor_trace.rs', 'tests/support/standalone_x11.rs',
           'tests/support/acceptance_context.rs') + tuple('tests/fixtures/'+name+'.scene.yaml' for name in FIXTURES)
TARGETS = {'orr_static_starter': ('src/lib.rs', 'lib'),
           'native_window': ('tests/native_window.rs', 'test'),
           'standalone_x11': ('tests/standalone_x11.rs', 'test'),
           'standalone_editor_trace': ('tests/standalone_editor_trace.rs', 'test'),
           'delivery-player': ('src/bin/delivery-player.rs', 'bin'),
           'switchboard-player': ('src/bin/switchboard-player.rs', 'bin'),
           'bundle': ('src/bin/bundle.rs', 'bin')}
require = editor.require


def separate_roots(*roots):
    roots = [Path(p) for p in roots]
    for root in roots:
        require(root.is_absolute() and root == root.resolve(), 'noncanonical root')
        for parent in (root, *root.parents):
            require(not parent.is_symlink(), 'linked root ancestor')
    for index, root in enumerate(roots):
        for other in roots[index+1:]:
            require(root != other and root not in other.parents and other not in root.parents,
                    'overlapping source/output roots')


def build_environment(env, target):
    # A clean source fence is not enough if ambient Cargo configuration can inject
    # another compiler, flags, loader or alternate target. Fail, do not silently
    # discard a user's configuration and pretend it was the reviewed build.
    for key in env:
        forbidden = key in ('RUSTC', 'RUSTDOC', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER',
                            'RUSTFLAGS', 'RUSTDOCFLAGS', 'CARGO_ENCODED_RUSTFLAGS',
                            'CARGO_ENCODED_RUSTDOCFLAGS', 'CARGO_TARGET_DIR',
                            'LD_PRELOAD', 'LD_LIBRARY_PATH', 'DYLD_INSERT_LIBRARIES')
        forbidden |= key.startswith(('CARGO_BUILD_', 'CARGO_TARGET_', 'CARGO_PROFILE_',
                                     'CARGO_ALIAS_', 'CARGO_SOURCE_', 'CARGO_REGISTRIES_'))
        require(not forbidden, 'ambient compiler configuration '+key)
    result = dict(env, CARGO_TARGET_DIR=str(target), PYTHONDONTWRITEBYTECODE='1')
    cargo_home = Path(env.get('CARGO_HOME', str(Path.home()/'.cargo'))).resolve()
    for name in ('config', 'config.toml'):
        path = cargo_home/name
        require(not path.exists() and not path.is_symlink(), 'ambient Cargo home configuration')
    return result


def prepare_own_lock(project, execute):
    """Resolve this fresh workspace, then cache its exact lock before going offline.

    An engine's locked fetch may legally cache yanked crates. It is not a
    registry closure for a new workspace: offline resolution cannot choose an
    uncached compatible replacement. Never seed from the engine lock or bypass
    Cargo's yank/MSRV checks. The original 180-second preparation cap is shared
    by resolution and fetch; all subsequent consumers remain locked/offline.
    """
    lock = project/'Cargo.lock'
    require(not lock.exists() and not lock.is_symlink(), 'own lock must start absent')
    deadline = time.monotonic() + 180
    def run(args, name):
        remaining = deadline - time.monotonic()
        require(remaining > 0, 'own lock preparation timeout')
        execute(args, name, remaining)
    run(['cargo', 'generate-lockfile'], 'own-lock')
    require(lock.is_file() and not lock.is_symlink(), 'own lock missing/linked')
    lock_hash = editor.digest(lock)
    run(['cargo', 'fetch', '--locked'], 'own-lock-fetch')
    require(lock.is_file() and not lock.is_symlink(), 'own lock missing/linked after fetch')
    require(editor.digest(lock) == lock_hash, 'own lock changed during fetch')
    return lock_hash


def overlay(project, gate):
    before = {p.relative_to(project).as_posix(): editor.digest(p) for p in project.rglob('*') if p.is_file()}
    records = []
    for relative in OVERLAY:
        source, destination = gate/STARTER/relative, project/relative
        require(source.is_file() and not source.is_symlink(), 'missing/linked acceptance overlay '+relative)
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(source.read_bytes())
        destination.chmod(0o644)
        records.append({'path': relative, 'before_sha256': before.get(relative),
                        'sha256': editor.digest(destination), 'gate_source_sha256': editor.digest(source), 'mode': '100644'})
    manifest = project/'Cargo.toml'
    original = manifest.read_text()
    feature = 'standalone-native-acceptance = ["standalone", "bundle", "dep:x11rb", "dep:png"]\n'
    require('standalone-native-acceptance' not in original, 'unexpected preexisting native overlay')
    changed = original.replace('[features]\n', '[features]\n'+feature, 1)
    changed += '\n[[test]]\nname = "standalone_x11"\npath = "tests/standalone_x11.rs"\nrequired-features = ["standalone-native-acceptance"]\n'
    parsed = tomllib.loads(changed)
    require(parsed['package']['name'] == PACKAGE and parsed['lib']['name'] == 'orr_static_starter', 'overlay package identity')
    manifest.write_text(changed)
    allowed = set(OVERLAY)|{'Cargo.toml'}
    for relative, digest in before.items():
        if relative not in allowed:
            require(editor.digest(project/relative) == digest, 'overlay changed production source '+relative)
    receipt = {'schema': 1, 'purpose': 'acceptance-only; production src unchanged',
               'gate_source_files': records, 'manifest_before_sha256': before['Cargo.toml'],
               'manifest_after_sha256': editor.digest(manifest)}
    (project/'acceptance-overlay.json').write_text(json.dumps(receipt, indent=2)+'\n')
    return receipt


def own_commit(project, env):
    # Local fixture identity only. No remote, credentials, hooks or signing.
    git_env = {k:v for k,v in env.items() if not k.startswith('GIT_')}
    git_env.update(GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')
    def git(*args):
        return subprocess.check_output(['git', '-c', 'core.hooksPath=/dev/null',
            '-c', 'commit.gpgSign=false', '-c', 'user.name=Native acceptance fixture',
            '-c', 'user.email=native-acceptance@invalid', *args], cwd=project, env=git_env, text=True).strip()
    require(not (project/'.git').exists(), 'generated project already has Git identity')
    git('init', '--quiet')
    git('add', '--all')
    git('commit', '--quiet', '-m', 'Commit generated acceptance project and own resolved lock')
    require(not git('remote'), 'generated project must have no remote')
    return git('rev-parse', 'HEAD')


def package_context(metadata, project, source_sha, gate, gate_sha, fence, context_path, hidden):
    packages = [p for p in metadata['packages'] if p['name'] == PACKAGE]
    require(len(packages) == 1, 'exact generated package required')
    package = packages[0]
    require(package['manifest_path'] == str(project/'Cargo.toml'), 'wrong package manifest')
    require(metadata['workspace_root'] == str(project), 'generated project must own its workspace')
    require(metadata['workspace_members'] == [package['id']], 'unexpected generated workspace member')
    sources = {}
    for name, (relative, kind) in TARGETS.items():
        targets = [t for t in package['targets'] if t['name'] == name and t['kind'] == [kind]]
        require(len(targets) == 1 and targets[0]['src_path'] == str(project/relative), 'wrong generated target '+name)
        sources[name] = project/relative
    return editor.ExecutionContext(project, PACKAGE, package['id'], sources, source_sha,
                                   gate, gate_sha, fence, context_path, hidden, metadata['target_directory'])


def run(gate, engine, project, target, artifacts):
    gate, engine, project, target, artifacts = map(Path, (gate, engine, project, target, artifacts))
    separate_roots(gate, engine, project, target, artifacts)
    require(' ' in str(project), 'generated project path must exercise spaces')
    require(not project.exists() and not target.exists() and not artifacts.exists(), 'fresh project, target and evidence paths required')
    artifacts.mkdir(parents=True)
    receipt = {'schema': SCHEMA, 'status': 'started', 'native_acceptance': False}
    def save():
        (artifacts/'gate.json').write_text(json.dumps(receipt, indent=2)+'\n')
    save()
    try:
        env = build_environment(os.environ, target)
        gate_sha = env.get('GITHUB_SHA', '')
        require(re.fullmatch('[0-9a-f]{40}', gate_sha) and all(env.get(k,'').isdecimal() for k in ('GITHUB_RUN_ID','GITHUB_RUN_ATTEMPT')), 'hosted gate/run identity required')
        gate_source = editor.clean_checkout(gate_sha, gate, complete=True)
        for root in (engine, project, target):
            editor.reject_ancestor_config(root)
        inventory = editor.json_file(gate/'tools/user_game_bootstrap_source.json')
        require(not (engine/'target').exists() and not (engine/'target').is_symlink(), 'pinned engine must have no unrecorded target tree')
        bootstrap.validate_engine(engine, inventory)
        receipt.update(hosted_gate_commit=gate_sha, engine_commit=bootstrap.PINNED_COMMIT,
                       engine_tree=bootstrap.PINNED_TREE)
        receipt['bootstrap'] = bootstrap.bootstrap(engine, project, PACKAGE, inventory)
        receipt['overlay'] = overlay(project, gate)
        save()
        execute = lambda args,name,timeout: standalone.command(args,artifacts,name,env,timeout,cwd=project)
        lock_hash = prepare_own_lock(project, execute)
        user_sha = own_commit(project, env)
        require(user_sha not in (gate_sha, bootstrap.PINNED_COMMIT), 'user/engine/gate commit conflation')
        user_source = editor.clean_checkout(user_sha, project, complete=True)
        hidden = [str(p) for p in (project, engine, gate, target)]
        context_value = {'schema': CONTEXT_SCHEMA, 'mode': 'generated_user_project',
                         'user_root': str(project), 'engine_root': str(engine), 'gate_root': str(gate),
                         'user_source_commit': user_sha, 'hosted_gate_commit': gate_sha,
                         'engine_commit': bootstrap.PINNED_COMMIT, 'hidden_source_roots': hidden,
                         'acceptance_scenes': {name:str(project/'tests/fixtures'/(name+'.scene.yaml')) for name in FIXTURES}}
        context_path = artifacts/'execution-context.json'
        context_path.write_text(json.dumps(context_value, sort_keys=True, indent=2)+'\n')
        context_hash = editor.digest(context_path)
        def fence():
            require(editor.digest(context_path) == context_hash, 'execution context changed')
            require(editor.clean_checkout(gate_sha, gate, complete=True) == gate_source, 'gate source changed')
            require(not (engine/'target').exists() and not (engine/'target').is_symlink(), 'engine target appeared')
            bootstrap.validate_engine(engine, inventory)
            require(editor.digest(project/'Cargo.lock') == lock_hash, 'own lock changed')
            current = editor.clean_checkout(user_sha, project, complete=True)
            require(current == user_source, 'user source changed')
            return current
        metadata = json.loads(execute(['cargo','metadata','--offline','--locked','--no-deps','--format-version=1'], 'metadata', 60))
        require(metadata['target_directory'] == str(target), 'unexpected Cargo target directory')
        context = package_context(metadata, project, user_sha, gate, gate_sha, fence, context_path, hidden)
        receipt.update(user_source_commit=user_sha, user_source_tree=user_source['tree'],
                       own_lock_sha256=lock_hash, package_id=context.package_id,
                       execution_context_sha256=context_hash)
        for name,value in [('gate-source',gate_source),('user-source',user_source)]:
            path=artifacts/(name+'.json');path.write_text(json.dumps(value,indent=2)+'\n')
            receipt[name+'_sha256']=editor.digest(path)
        save()
        # Both drivers preserve all original execution and independent validation.
        # Pass explicit environment to avoid a process-global chdir or SHA override.
        # A fresh generated target has no warm engine artifacts. Share, rather
        # than increase, the three existing 900s compiler phase allocations.
        compile_budget = editor.CompileBudget()
        editor.run(artifacts/'editor', context, env, compile_budget=compile_budget)
        standalone.run(artifacts/'standalone', context, env, compile_budget=compile_budget)
        fence()
        receipt.update(status='passed', native_acceptance=True,
                       editor_gate_sha256=editor.digest(artifacts/'editor/gate.json'),
                       standalone_gate_sha256=editor.digest(artifacts/'standalone/gate.json'))
    except Exception as error:
        receipt.update(status='failed', error=str(error))
        raise
    finally:
        save()


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('gate','engine','project','target','artifacts'):
        parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args()
    try:
        run(args.gate,args.engine,args.project,args.target,args.artifacts)
    except Exception as error:
        print('generated user native gate FAILED: '+str(error),file=sys.stderr)
        sys.exit(1)
