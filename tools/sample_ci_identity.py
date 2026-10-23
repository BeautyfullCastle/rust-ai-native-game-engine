#!/usr/bin/env python3
"""Bind the independent Arena CPU lane, never reuse or certify cached binaries."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def context():
    def output(*argv):
        return subprocess.check_output(argv, text=True).strip()
    required = ('GITHUB_SHA', 'GITHUB_RUN_ID', 'GITHUB_RUN_ATTEMPT', 'RUNNER_OS', 'RUNNER_ARCH', 'GITHUB_EVENT_NAME', 'GITHUB_REF')
    binding = {key: os.environ[key] for key in required}
    assert all(binding.values()), 'missing execution identity'
    event = binding['GITHUB_EVENT_NAME']
    assert event in ('pull_request', 'workflow_dispatch'), 'unsupported event'
    pr_head = os.environ.get('PR_HEAD_SHA') or None
    if event == 'pull_request':
        assert pr_head, 'missing PR head'
    else:
        assert pr_head is None, 'dispatch must not claim a PR head'
    binding['PR_HEAD_SHA'] = pr_head
    binding.update(head=output('git', 'rev-parse', 'HEAD'),
                   tree=output('git', 'rev-parse', 'HEAD^{tree}'),
                   lock_sha256=digest('Cargo.lock'), rustc=output('rustc', '-vV'))
    assert binding['head'] == binding['GITHUB_SHA'], 'checkout is not workflow SHA'
    assert not output('git', 'status', '--porcelain=v1', '--untracked-files=all'), 'changed source'
    return binding


def verify(binding, current, receipt):
    assert binding['schema'] == 1 and binding['status'] == 'started'
    assert binding['identity'] == current, 'stale identity or changed source/toolchain/attempt'
    assert receipt['status'] == 'passed' and receipt['mode'] == 'cpu'
    assert receipt['ui_profile'] is True
    assert receipt['head'] == current['head'] and receipt['tree'] == current['tree']
    assert receipt['source_unchanged'] is True and receipt['source_authority_unchanged'] is True
    assert receipt['source_sha256']['Cargo.lock']['sha256'] == current['lock_sha256']
    assert receipt['commands'] and all(c['state'] == 'passed' for c in receipt['commands'])
    assert receipt['producers'] and receipt['coverage']
    # The mandatory gate verifies exact requested features, profile, paths and
    # byte hashes itself. Retain its complete compiler-JSON provenance unchanged.
    producers = {}
    for name, item in receipt['producers'].items():
        producer = item['producer']
        assert producer['fresh'] is False and producer['reason'] == 'compiler-artifact'
        assert isinstance(producer['features'], list) and isinstance(producer['profile'], dict)
        assert producer['profile'] and item['sha256'] and item['bytes'] > 0
        producers[name] = {key: item[key] for key in ('sha256', 'bytes', 'producer')}
    return producers


def main():
    mode, destination, *rest = sys.argv[1:]
    path = Path(destination)
    assert path.is_absolute() and not path.is_relative_to(Path.cwd()), 'identity must be external'
    current = context()
    if mode == 'begin':
        assert not rest and not path.exists(), 'immutable identity already exists'
        path.write_text(json.dumps({'schema': 1, 'status': 'started', 'identity': current}, indent=2), encoding='utf-8')
    else:
        assert mode == 'verify' and len(rest) == 1
        binding = json.loads(path.read_text(encoding='utf-8'))
        receipt_path = Path(rest[0])
        receipt = json.loads(receipt_path.read_text(encoding='utf-8'))
        producers = verify(binding, current, receipt)
        binding.update(status='passed', gate_receipt_sha256=digest(receipt_path), producers=producers)
        path.write_text(json.dumps(binding, indent=2), encoding='utf-8')


if __name__ == '__main__':
    main()
