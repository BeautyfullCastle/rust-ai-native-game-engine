#!/usr/bin/env python3
"""Fail-closed receipt primitives for a reviewed Linux-local gate adapter.

This module does not launch Cargo or import historical receipts. The shell gate
remains authoritative until a separately reviewed execution adapter uses these
primitives. Environment values are never serialized; all unknown keys matter.
"""
import hashlib
import json
import os
from pathlib import Path
import stat

POLICY_VERSION = 2
# Shell bookkeeping already excluded by the historical runner. No proxy,
# platform-injected ID, or unknown environment variable is exempted here.
ENV_METADATA = frozenset({"_", "SHLVL", "OLDPWD"})
LIMITATIONS = "Linux baseline only; not Windows/macOS/Android/browser/GPU/ignored UI certification"


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def file_stamp(path):
    path = Path(path)
    info = path.lstat()
    if not path.is_file():
        raise ValueError("receipt dependency must be a regular file or file symlink")
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(block)
    return {"sha256": hasher.hexdigest(), "mode": stat.S_IMODE(info.st_mode),
            "link": os.readlink(path) if path.is_symlink() else None,
            "resolved": str(path.resolve())}


def environment_identity(environment):
    # Aggregate only: never persist per-key hashes of credentials (dictionary leak).
    relevant = {k: v for k, v in environment.items() if k not in ENV_METADATA}
    return {"names": sorted(relevant), "fingerprint": digest(relevant)}


def phase_key(phase, *, source, context, environment, producers):
    """source/context are verified current snapshots, not caller claims of PASS.

    Caller must pin/recheck source files, cwd, toolchain payload, native libraries,
    Cargo/Python/compiler configs and runtime inputs before AND after execution.
    No crate-level dependency closure is claimed: any source change invalidates.
    Producers identify upstream completed receipts/artifacts, not just phase names.
    """
    return digest({"policy": POLICY_VERSION, "phase": phase, "source": source,
                   "context": context, "environment": environment_identity(environment),
                   "producers": producers})


def terminal_receipt(phase, key, exit_code, log, artifacts=(), *, interrupted=False):
    """Build terminal evidence only after the adapter rechecks all inputs.

    Interrupted or signal-like exits are never advisory successes. This API does
    not infer completion from a log that happens to contain passing test lines.
    """
    unknown = interrupted or exit_code < 0 or exit_code >= 128
    status = ("UNKNOWN" if unknown else "PASS" if exit_code == 0 else
              "ADVISORY" if phase["advisory"] else "FAIL")
    paths = [Path(log), *(Path(p) for p in artifacts)]
    return {"policy": POLICY_VERSION, "phase": phase, "key": key,
            "status": status, "exit_code": exit_code,
            "log": str(Path(log)), "artifacts": [str(p) for p in paths[1:]],
            "files": {str(p): file_stamp(p) for p in paths}}


def reusable(receipt, phase, key):
    try:
        code = receipt["exit_code"]
        allowed = ((receipt["status"] == "PASS" and code == 0) or
                   (phase["advisory"] and receipt["status"] == "ADVISORY" and 0 < code < 128))
        if not allowed or receipt["policy"] != POLICY_VERSION or receipt["phase"] != phase or receipt["key"] != key:
            return False
        expected = {receipt["log"], *receipt["artifacts"]}
        if set(receipt["files"]) != expected:
            return False
        return all(file_stamp(p) == receipt["files"][p] for p in expected)
    except (KeyError, TypeError, ValueError, OSError):
        return False


def composed_coverage(phases, evidence):
    """Reject omitted/duplicate phases; validate every current phase key again."""
    if not phases or len({p["name"] for p in phases}) != len(phases) or len(evidence) != len(phases):
        return None
    for phase, (receipt, current_key) in zip(phases, evidence):
        if not reusable(receipt, phase, current_key):
            return None
    return {"status": "COMPOSED_COVERAGE_PASS", "original_uninterrupted_exit_zero": False,
            "receipts": [r for r, _ in evidence], "limitations": LIMITATIONS}


# Exact legacy commands, including assembler extraction and nonzero test count.
# These are definitions for review/testing, NOT an executable replacement gate.
PHASES = [{'name': 'rustc', 'command': 'rustc --version --verbose', 'advisory': False},
 {'name': 'cargo', 'command': 'cargo --version', 'advisory': False},
 {'name': 'python', 'command': 'python3 --version', 'advisory': False},
 {'name': 'format', 'command': 'cargo fmt --all -- --check', 'advisory': True},
 {'name': 'sim-float', 'command': 'python3 -B tools/check_sim_float_types.py', 'advisory': False},
 {'name': 'float-fixtures',
  'command': 'python3 -B tools/test_sim_float_guard.py',
  'advisory': False},
 {'name': 'workspace-build',
  'command': 'cargo build --locked --workspace --release',
  'advisory': False},
 {'name': 'assemble-build',
  'command': 'cargo build --locked --release -p orr_model --no-default-features --features import '
             '\\\n'
             '  --example static_model_assemble --message-format=json > "$assemble_artifacts"\n',
  'advisory': False},
 {'name': 'assemble-extract',
  'command': 'ORR_STATIC_MODEL_ASSEMBLE_BIN=$(python3 - "$assemble_artifacts" <<\'PYARTIFACT\'\n'
             'import json\n'
             'import sys\n'
             'paths = [record["executable"] for line in open(sys.argv[1])\n'
             '         if (record := json.loads(line)).get("reason") == "compiler-artifact"\n'
             '         and record.get("target", {}).get("name") == "static_model_assemble"\n'
             '         and record.get("executable")]\n'
             'assert len(paths) == 1, paths\n'
             'print(paths[0])\n'
             'PYARTIFACT\n'
             ')\n'
             'export ORR_STATIC_MODEL_ASSEMBLE_BIN\n'
             '\n'
             'printf "%s\\n" "$ORR_STATIC_MODEL_ASSEMBLE_BIN" > "$assembler_path"\n',
  'advisory': False},
 {'name': 'workspace-tests',
  'command': 'cargo test --locked --workspace --release',
  'advisory': False},
 {'name': 'workspace-clippy',
  'command': 'cargo clippy --locked --workspace --all-targets',
  'advisory': False},
 {'name': 'actions-build',
  'command': 'cargo build --locked --release -p orr_static_starter --no-default-features '
             '--features standalone,static-game-actions --bins',
  'advisory': False},
 {'name': 'x11-compile-only',
  'command': 'cargo test --locked --release -p orr_static_starter --no-default-features --features '
             'standalone-native-acceptance --test standalone_x11 --no-run',
  'advisory': False},
 {'name': 'input-tests',
  'command': 'cargo test --locked --release -p orr_input',
  'advisory': False},
 {'name': 'multi-action-tests',
  'command': 'cargo test --locked --release -p orr_static_starter --no-default-features --features '
             'standalone,static-game-actions --test multi_action_contract 2>&1 | tee '
             '"$action_contract_log"\n',
  'advisory': False},
 {'name': 'multi-action-count',
  'command': 'python3 - "$action_contract_log" <<\'PYCOUNT\'\n'
             'import re\n'
             'import sys\n'
             'from pathlib import Path\n'
             'counts = re.findall(r"test result: ok\\. (\\d+) passed; 0 failed;", '
             'Path(sys.argv[1]).read_text())\n'
             'if len(counts) != 1 or int(counts[0]) <= 0:\n'
             '    raise SystemExit("multi_action_contract must execute and pass tests")\n'
             'print(f"Verified multi_action_contract: {counts[0]} tests passed")\n'
             'PYCOUNT\n',
  'advisory': False},
 {'name': 'editor-tests',
  'command': 'cargo test --locked --release -p orr_editor --features static-game-actions --lib',
  'advisory': False},
 {'name': 'static_game_actions_observation_pacing',
  'command': 'python3 -B tools/test_static_game_actions_observation_pacing.py',
  'advisory': False},
 {'name': 'static_game_standalone_focus_sync',
  'command': 'python3 -B tools/test_static_game_standalone_focus_sync.py',
  'advisory': False},
 {'name': 'static_game_standalone_gate',
  'command': 'python3 -B tools/test_static_game_standalone_gate.py',
  'advisory': False},
 {'name': 'static_game_native_ui_settle_gate',
  'command': 'python3 -B tools/test_static_game_native_ui_settle_gate.py',
  'advisory': False},
 {'name': 'user_game_action_preview_gate',
  'command': 'python3 -B tools/test_user_game_action_preview_gate.py',
  'advisory': False},
 {'name': 'user_game_own_lock_gate',
  'command': 'python3 -B tools/test_user_game_own_lock_gate.py',
  'advisory': False}]

# Audited source-only scripts: stdlib imports, fixed repository reads, no process
# or network operations. Never extend this set from caller-supplied configuration.
LOCAL_PHASES = frozenset({
    'static_game_actions_observation_pacing',
    'static_game_standalone_focus_sync',
    'static_game_native_ui_settle_gate',
})
AUDITED_SCRIPT_SHA256 = {'static_game_actions_observation_pacing': 'a07614893d53577ddc457d32240b2136500e37fbff55900da9249cd48ef5364f', 'static_game_standalone_focus_sync': '49d62d9985f4fed5883eb6fa4ec70b496acb40642461ad44588731e4ab2f3906', 'static_game_native_ui_settle_gate': 'bee63bf76ed1fa17aea8a73f24f8f9c2f76a263f442590aae1fc4384a18bb0f9'}
LOCAL_SOURCE_DEPENDENCIES = {
    'static_game_actions_observation_pacing': (
        'examples/static-game-starter/src/standalone.rs',
        'examples/static-game-starter/tests/standalone_x11.rs',
        'examples/static-game-starter/tests/multi_action_standalone_x11.rs',
    ),
    'static_game_standalone_focus_sync': ('examples/static-game-starter/tests/standalone_x11.rs',),
    'static_game_native_ui_settle_gate': ('examples/static-game-starter/tests/native_window.rs',),
}


def local_source_snapshot(name, root):
    files = {}
    for relative in LOCAL_SOURCE_DEPENDENCIES[name]:
        path = root / relative
        if path.is_symlink() or path.resolve() != path or root not in path.resolve().parents:
            raise ValueError('fixed source dependency must not resolve through symlinks')
        files[relative] = file_stamp(path)
    return files


# Observed rotating transport variables. Relevant to Cargo and arbitrary tests;
# irrelevant ONLY to the isolated, audited source-only scripts above.
LOCAL_TRANSPORT_METADATA = frozenset({
    'CODEX_TOOL_CALL_ID', 'ALL_PROXY', 'BUNDLE_HTTPS_PROXY', 'BUNDLE_HTTP_PROXY',
    'DOCKER_HTTPS_PROXY', 'DOCKER_HTTP_PROXY', 'FTP_PROXY', 'HTTPS_PROXY',
    'HTTP_PROXY', 'NPM_CONFIG_HTTPS_PROXY', 'NPM_CONFIG_HTTP_PROXY',
    'NPM_CONFIG_PROXY', 'PIP_PROXY', 'WSS_PROXY', 'WS_PROXY', 'YARN_HTTPS_PROXY',
    'YARN_HTTP_PROXY', 'all_proxy', 'ftp_proxy', 'http_proxy', 'https_proxy',
    'npm_config_http_proxy', 'npm_config_https_proxy', 'npm_config_proxy',
    'ws_proxy', 'wss_proxy',
})


def source_snapshot(root):
    import subprocess
    names = subprocess.check_output(['git', 'ls-files', '-z'], cwd=root).split(b'\0')
    return {os.fsdecode(name): file_stamp(root / os.fsdecode(name)) for name in names if name}


def python_snapshot():
    import platform
    import sys
    import sysconfig
    import unittest  # load the complete audited scripts' import set before pinning native mappings
    del unittest
    native = {}
    for line in Path('/proc/self/maps').read_text().splitlines():
        fields = line.split(maxsplit=5)
        if len(fields) == 6 and fields[5].startswith('/'):
            mapped = fields[5]
            if mapped.endswith(' (deleted)'):
                raise RuntimeError('mapped runtime payload was deleted')
            if mapped not in native:
                native[mapped] = file_stamp(mapped)
    lib = Path(sysconfig.get_path('stdlib'))
    imports = {}
    for entry in sys.path:
        path = Path(entry)
        if path.suffix == '.zip':
            imports[str(path)] = file_stamp(path) if path.exists() else {'absent': True}
        elif path.resolve() not in {lib.resolve(), (lib / 'lib-dynload').resolve()}:
            raise RuntimeError('unsupported isolated Python import path')
    files = {}
    for directory, dirs, names in os.walk(lib):
        dirs[:] = sorted(d for d in dirs if d not in {'site-packages', 'dist-packages'})
        if any((Path(directory) / d).is_symlink() for d in dirs):
            raise RuntimeError('symlinked standard-library package directories are unsupported')
        for name in sorted(names):
            if name.endswith(('.py', '.pyc', '.so')):
                path = Path(directory) / name
                files[str(path)] = file_stamp(path)
    return {'executable': file_stamp(sys.executable), 'stdlib': files,
            'version': sys.version, 'platform': platform.platform(), 'sys_path': sys.path,
            'loaded_native_payload': native, 'import_archives': imports}


def write_json(path, value):
    temporary = path.with_suffix(path.suffix + '.tmp')
    with temporary.open('w') as stream:
        json.dump(value, stream, sort_keys=True)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)
    directory = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def execute_local(name, receipt_dir, root):
    """Executable partial adapter: no Cargo, no environment/network modification."""
    import fcntl
    import subprocess
    import sys
    if not sys.flags.isolated or not sys.flags.no_site or not sys.flags.dont_write_bytecode or sys.flags.optimize:
        raise RuntimeError('invoke with python3 -I -S -B; optimization forbidden')
    import uuid  # load any native UUID dependency before the runtime snapshot
    if name not in LOCAL_PHASES:
        raise ValueError('phase has not been audited for local-only reuse')
    phase = next(p for p in PHASES if p['name'] == name)
    root, receipt_dir = Path(root).resolve(), Path(receipt_dir).resolve()
    if root == receipt_dir or root in receipt_dir.parents:
        raise ValueError('receipt directory must be outside the source checkout')
    receipt_dir.mkdir(parents=True, exist_ok=True)
    with (receipt_dir / 'local-policy.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        script = root / 'tools' / ('test_' + name + '.py')
        if script.is_symlink() or script.resolve() != script or root not in script.resolve().parents:
            raise ValueError('audited script must remain inside its source checkout, not a symlink')
        if file_stamp(script)['sha256'] != AUDITED_SCRIPT_SHA256[name]:
            raise ValueError('source-policy script changed; independent audit and policy update required')
        command = [sys.executable, '-I', '-S', '-B', str(script)]
        environment = dict(os.environ)  # passed unchanged; proxy routing untouched
        key_environment = {k: v for k, v in environment.items() if k not in LOCAL_TRANSPORT_METADATA}
        source, runtime = source_snapshot(root), python_snapshot()
        # Explicit script/adapter binding also covers a not-yet-tracked development file.
        context = {'runtime': runtime, 'cwd': str(root), 'actual_command': command,
                   'script': file_stamp(script), 'adapter': file_stamp(__file__),
                   'fixed_sources': local_source_snapshot(name, root)}
        key = phase_key(phase, source=source, context=context,
                        environment=key_environment, producers={})
        # Explicit latest pointer, not wall-clock sorting: clock rollback must not
        # resurrect an older PASS after a newer interrupted attempt.
        latest = receipt_dir / (name + '.latest.json')
        if latest.exists():
            try:
                selected = json.loads(latest.read_text())['start']
                if Path(selected).name != selected or not selected.startswith(name + '-') or not selected.endswith('.start.json'):
                    raise ValueError('invalid latest attempt pointer')
                start = json.loads((receipt_dir / selected).read_text())
                terminal = receipt_dir / (selected.removesuffix('.start.json') + '.end.json')
                old = json.loads(terminal.read_text())
                bound = (start['status'] == 'RUNNING' and start['phase'] == phase
                         and start['key'] == key and start['log'] == old['log'])
            except (OSError, ValueError, KeyError, TypeError):
                old, bound = None, False
            if bound and old is not None and reusable(old, phase, key):
                print('REUSED local source-policy phase: ' + name)
                return 0
        prefix = receipt_dir / (name + '-' + uuid.uuid4().hex)
        log = prefix.with_suffix('.log')
        start_path = prefix.with_suffix('.start.json')
        write_json(start_path,
                   {'status': 'RUNNING', 'phase': phase, 'key': key,
                    'environment_names': sorted(environment), 'log': str(log)})
        write_json(latest, {'start': start_path.name})
        with log.open('wb') as output:
            result = subprocess.run(command, cwd=root, env=environment,
                                    stdout=output, stderr=subprocess.STDOUT)
            output.flush()
            os.fsync(output.fileno())
        stable = (source_snapshot(root) == source and python_snapshot() == runtime
                  and local_source_snapshot(name, root) == context['fixed_sources']
                  and file_stamp(script) == context['script']
                  and file_stamp(__file__) == context['adapter'])
        receipt = terminal_receipt(phase, key, result.returncode, log, interrupted=not stable)
        write_json(prefix.with_suffix('.end.json'), receipt)
        print(log.read_text(errors='replace'), end='')
        return result.returncode if stable and result.returncode >= 0 else 1


if __name__ == '__main__':
    import argparse
    import sys
    parser = argparse.ArgumentParser(description='Audited local source-policy phase reuse only')
    parser.add_argument('--phase', required=True, choices=sorted(LOCAL_PHASES))
    parser.add_argument('--receipt-dir', required=True)
    arguments = parser.parse_args()
    sys.exit(execute_local(arguments.phase, arguments.receipt_dir, Path(__file__).resolve().parents[1]))
