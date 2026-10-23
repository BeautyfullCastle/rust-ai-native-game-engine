#!/usr/bin/env python3
"""Offline planning only. Inputs must be freshly collected and independently reviewed."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import sys


class Blocked(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise Blocked(message)


def plan(document, expected_head, evidence_root):
    """Validate normalized, complete one-workflow snapshot; return a non-executable plan."""
    require(document.get('schema') == 1, 'unsupported schema')
    run = document['run']
    require(re.fullmatch('[0-9a-f]{40}', expected_head) is not None, 'invalid expected head')
    require(run['head_sha'] == expected_head, 'changed head: old results cannot validate new head')
    require(run['status'] == 'completed', 'run is pending or running')
    attempt = run['run_attempt']
    require(type(attempt) is int and 1 <= attempt < 3, 'retry budget exhausted or invalid attempt')
    require(type(run['id']) is int and run['id'] > 0, 'invalid run ID')
    require(document.get('inventory_complete') is True, 'incomplete job/artifact inventory')
    require(document.get('upload_inventory_complete') is True, 'upload inventory not reviewed')
    jobs = document['jobs']
    require(bool(jobs), 'empty jobs')
    keys = [j['key'] for j in jobs]
    require(len(keys) == len(set(keys)), 'duplicate logical job key')
    require(set(keys) == set(document['required_jobs']), 'required inventory mismatch')
    require(len(document['required_jobs']) == len(keys), 'duplicate required key')
    require(len({j['id'] for j in jobs}) == len(jobs), 'duplicate job ID')
    by_key = {j['key']: j for j in jobs}
    require(set(document['latest_job_ids']) == set(keys), 'latest observation inventory mismatch')
    for j in jobs:
        require(type(j['id']) is int and j['id'] > 0, 'invalid job ID')
        require(document['latest_job_ids'][j['key']] == j['id'], 'stale job observation')
        require(type(j['needs']) is list and all(type(k) is str for k in j['needs']), 'invalid dependency list')
        require(j['head_sha'] == expected_head and j['run_id'] == run['id'], 'job binding mismatch')
        require(type(j['run_attempt']) is int and 1 <= j['run_attempt'] <= attempt, 'invalid job attempt')
        require(j['status'] == 'completed', 'job is pending or running')
        require(j['conclusion'] in ('success', 'failure', 'skipped'), 'unsupported job conclusion')
        require(set(j['needs']) <= set(keys), 'unknown dependency')
    # Detect cycles, including cycles not reachable from a failure.
    visited, active = set(), set()
    def visit(key):
        require(key not in active, 'cyclic dependency graph')
        if key in visited:
            return
        active.add(key)
        for dependency in by_key[key]['needs']:
            visit(dependency)
        active.remove(key)
        visited.add(key)
    for key in keys:
        visit(key)
    failed = {j['key'] for j in jobs if j['conclusion'] == 'failure'}
    require(bool(failed), 'no failed jobs to rerun')
    selected = set(failed)
    while True:
        expanded = selected | {j['key'] for j in jobs if set(j['needs']) & selected}
        if expanded == selected:
            break
        selected = expanded
    for j in jobs:
        require(j['conclusion'] != 'skipped' or (j['key'] in selected and bool(set(j['needs']) & selected)),
                'unexplained skipped job')
    require(set(document['failure_kinds']) == failed, 'every failure needs classification')
    for key in failed:
        kind = document['failure_kinds'][key]
        require(kind == 'transient' or (kind == 'dependency_aggregate' and by_key[key].get('aggregate_only') is True
                and bool(set(by_key[key]['needs']) & selected)), 'independent downstream failure is not retryable')
    roots = [k for k in failed if not (set(by_key[k]['needs']) & selected)]
    require(bool(roots), 'missing failure root')
    require(set(document['transient_failures']) == {k for k in failed if document['failure_kinds'][k] == 'transient'}, 'unclassified or nontransient root failure')
    require(all(document['failure_kinds'][k] == 'transient' for k in roots), 'root failure is not transient')
    for root in document['transient_failures']:
        require(document['transient_failures'][root] in ('artifact_dns', 'runner_service_unavailable'),
                'retry requires an explicitly classified transient infrastructure failure')
    # Each original failed job needs retained, locally hash-verified evidence.
    root_path = Path(evidence_root).resolve()
    proofs = document['failure_evidence']
    for key in failed:
        proof = proofs[key]
        j = by_key[key]
        require(proof['head_sha'] == expected_head and proof['run_id'] == run['id'], 'failure evidence head/run mismatch')
        require(proof['job_id'] == j['id'] and proof['run_attempt'] == j['run_attempt'], 'failure evidence binding mismatch')
        path = (root_path / proof['path']).resolve()
        require(path.is_relative_to(root_path), 'evidence path escapes archive')
        require(hashlib.sha256(path.read_bytes()).hexdigest() == proof['sha256'], 'failure evidence digest mismatch')
    existing = document['artifacts']
    require(all(a['run_id'] == run['id'] and a['head_sha'] == expected_head for a in existing), 'artifact binding mismatch')
    for artifact in existing:
        require(type(artifact['id']) is int and artifact['id'] > 0, 'invalid artifact ID')
        require(type(artifact['producer_attempt']) is int and 1 <= artifact['producer_attempt'] <= attempt,
                'unknown artifact producer attempt')
        archived = (root_path / artifact['archive_path']).resolve()
        require(archived.is_relative_to(root_path), 'artifact archive escapes evidence root')
        require(hashlib.sha256(archived.read_bytes()).hexdigest() == artifact['archive_sha256'],
                'artifact archive digest mismatch')
    names = {a['name'] for a in existing}
    future_names = set()
    for key in sorted(selected):
        for upload in by_key[key]['uploads']:
            require(upload['overwrite'] is False, 'artifact overwrite forbidden')
            template = upload['name_template']
            name = template.format(attempt=attempt + 1, run_id=run['id'])
            require(name not in names and name not in future_names, 'artifact name collision')
            future_names.add(name)
    census = [{field: j[field] for field in ('key', 'id', 'run_attempt', 'conclusion', 'head_sha')}
              for j in jobs]
    return {'schema': 1, 'decision': 'PLAN_ONLY', 'execution_authorized_by_tool': False,
            'head_sha': expected_head, 'run_id': run['id'], 'source_attempt': attempt,
            'proposed_attempt': attempt + 1, 'failed_job_ids': [by_key[k]['id'] for k in sorted(failed)],
            'rerun_jobs': sorted(selected), 'retained_successes': sorted(set(keys) - selected),
            'observed_census': census, 'failure_evidence': proofs, 'preserved_artifacts': existing,
            'planned_artifact_names': sorted(future_names), 'merge_ready': False,
            'post_retry_requirement': 'Fetch complete latest jobs plus attempt-specific history; verify every required logical job success on this exact head. Preserve each actual job attempt. This plan proves no retry or CI success.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('snapshot', type=Path)
    parser.add_argument('--expected-head', required=True)
    parser.add_argument('--evidence-root', type=Path, required=True)
    args = parser.parse_args()
    try:
        document = json.loads(args.snapshot.read_text(encoding='utf-8'))
        result = plan(document, args.expected_head, args.evidence_root)
    except (Blocked, KeyError, TypeError, ValueError, OSError, AttributeError, IndexError) as exc:
        print(json.dumps({'decision': 'BLOCKED', 'reason': str(exc), 'merge_ready': False}))
        return 2
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == '__main__':
    sys.exit(main())
