#!/usr/bin/env python3
"""Fresh source-bound assemble CLI for unchanged native workspace tests.

Preparation builds only the required CLI in an exclusive target. It does not
redirect or narrow the subsequent workspace Cargo command or its cache.
"""
import argparse
import json
import os
import pathlib
import sys
import signal
import subprocess
import time
import shutil

import static_model_extract_gate as runner


def environment_lines(binary):
    # GITHUB_ENV's single-line syntax must never admit another setting/command.
    value = str(pathlib.Path(binary).resolve())
    assert value and not any(c in value for c in '\r\n\0'), 'unsafe environment path'
    return 'ORR_STATIC_MODEL_ASSEMBLE_BIN=' + value + '\nPYTHONDONTWRITEBYTECODE=1\n'


def run(label, command, seconds=1200):
    if sys.platform != 'darwin':
        return runner.run(label, command, seconds=seconds)
    # macOS has no Linux /proc/subreaper. A private supervisor stays alive until
    # we kill its own process group, so its PID cannot be reused before cleanup.
    runner.fence()
    log = runner.evidence / (label + '.log')
    status = runner.evidence / (label + '.exit.json')
    assert not status.exists(), 'exclusive command status'
    entry = {'label': label, 'argv': command, 'state': 'running', 'log': log.name}
    runner.report['commands'].append(entry)
    runner.record()
    supervisor = ("import json,subprocess,sys; "
        "result=subprocess.run(json.loads(sys.argv[1]),stdin=subprocess.DEVNULL); "
        "f=open(sys.argv[2],'x'); json.dump(result.returncode,f); f.close(); "
        "sys.stdin.read()")
    process = None
    start = time.monotonic()
    try:
        with log.open('xb') as stream:
            process = subprocess.Popen([sys.executable, '-B', '-c', supervisor,
                json.dumps(command), str(status)], cwd=runner.repo, env=os.environ.copy(),
                stdin=subprocess.PIPE, stdout=stream, stderr=subprocess.STDOUT,
                start_new_session=True)
            while not status.exists():
                assert time.monotonic() - start < seconds, 'command deadline exceeded'
                assert log.stat().st_size <= runner.LOG_CAP, 'command output cap exceeded'
                assert shutil.disk_usage(runner.evidence).free >= runner.DISK_FLOOR, 'disk floor'
                # Do not poll/reap the supervisor: even an unexpected exit
                # must retain its PID until bounded group cleanup.
                time.sleep(.02)
            # The child writes only one integer, but file appearance precedes
            # its close. Wait for a complete JSON value within the same deadline.
            while True:
                try:
                    exit_code = json.loads(status.read_text())
                    break
                except json.JSONDecodeError:
                    assert time.monotonic() - start < seconds, 'status deadline exceeded'
                    time.sleep(.01)
            assert type(exit_code) is int and exit_code == 0, 'command failed: ' + str(exit_code)
            assert log.stat().st_size <= runner.LOG_CAP, 'command output cap exceeded'
            entry['state'] = 'passed'
    except BaseException as error:
        entry.update(state='failed', error=repr(error))
        raise
    finally:
        if process is not None:
            # The supervisor is deliberately not waited/reaped before killpg.
            if process.returncode is None:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass  # Already empty group; still reap our owned leader.
            process.wait(timeout=10)
            process.stdin.close()
        entry.update(seconds=time.monotonic()-start,
                     log_sha256=runner.digest(log) if log.exists() else None)
        runner.record()
    runner.fence()
    return log.read_text(encoding='utf-8')


def source_identity(prefix):
    assert not run(prefix + '-clean', ['git', 'status', '--porcelain=v1', '--untracked-files=all']).strip()
    head = run(prefix + '-head', ['git', 'rev-parse', 'HEAD']).strip()
    tree = run(prefix + '-tree', ['git', 'rev-parse', 'HEAD^{tree}']).strip()
    paths = run(prefix + '-paths', ['git', 'ls-files', '-z']).split('\0')
    assert paths.pop() == '' and paths and len(paths) == len(set(paths))
    assert len(head) == len(tree) == 40
    return head, tree, runner.source_snapshot(paths)


def prepare(root, evidence, environment_file):
    os.environ['PYTHONDONTWRITEBYTECODE'] = '1'
    os.environ['CARGO_BUILD_JOBS'] = '2'
    runner.initialize(root, pathlib.Path(evidence).resolve(), 'cpu')
    try:
        head, tree, sources = source_identity('prepare-before')
        assert os.environ.get('GITHUB_SHA', head) == head, 'checkout SHA mismatch'
        runner.report.update(head=head, tree=tree, source_sha256=sources, checkout=str(runner.repo))
        output = run('build-native-assemble', ['cargo', 'build', '--locked', '--release',
            '-p', 'orr_model', '--no-default-features', '--features', 'import',
            '--example', 'static_model_assemble', '--message-format=json'])
        binary = runner.pin(output, 'native-assemble', 'orr_model',
                            'static_model_assemble', 'example', ['import'])
        assert source_identity('prepare-after') == (head, tree, sources), 'source changed'
        runner.fence()
        lines = environment_lines(binary)
        runner.report.update(status='passed', purpose='native assemble prerequisite only',
                             workspace_tests_passed=False, source_unchanged=True)
        runner.record()
    except BaseException as error:
        runner.report.update(status='failed', failure=repr(error))
        runner.record()
        raise
    with pathlib.Path(environment_file).open('a', encoding='utf-8', newline='\n') as stream:
        stream.write(lines)


def verify(root, evidence):
    runner.repo = pathlib.Path(root).resolve()
    runner.evidence = pathlib.Path(evidence).resolve()
    runner.report = json.loads((runner.evidence / 'gate-receipt.json').read_text())
    assert runner.report['status'] == 'passed', 'prerequisite did not complete'
    assert runner.report['checkout'] == str(runner.repo), 'checkout identity changed'
    try:
        runner.fence()
        assert source_identity('verify-after-workspace') == (runner.report['head'],
            runner.report['tree'], runner.report['source_sha256']), 'source changed'
        runner.report['post_workspace_identity_verified'] = True
    except BaseException as error:
        runner.report.update(status='failed', verification_failure=repr(error))
        raise
    finally:
        runner.record()
    # This postcondition checks identity, not workspace-test success.


def main():
    assert __debug__ and not sys.flags.optimize, 'nonoptimized Python required'
    parser = argparse.ArgumentParser()
    parser.add_argument('mode', choices=['prepare', 'verify'])
    parser.add_argument('--evidence', required=True)
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parents[1]
    if args.mode == 'prepare':
        prepare(root, pathlib.Path(args.evidence), os.environ['GITHUB_ENV'])
    else:
        verify(root, pathlib.Path(args.evidence))


if __name__ == '__main__':
    main()
