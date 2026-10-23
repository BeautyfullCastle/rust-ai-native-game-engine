import copy
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'tools'))
from ci_targeted_retry import Blocked, plan


class RetryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.head = 'a' * 40
        self.proof = b'synthetic DNS failure fixture, not hosted evidence\n'
        (self.root / 'failure.log').write_bytes(self.proof)
        def job(key, number, conclusion, needs):
            return dict(key=key, id=number, run_id=42, run_attempt=1, head_sha=self.head,
                        status='completed', conclusion=conclusion, needs=needs, uploads=[])
        self.doc = dict(schema=1, run=dict(id=42, run_attempt=1, head_sha=self.head, status='completed'),
                        inventory_complete=True, upload_inventory_complete=True,
                        required_jobs=['linux', 'macos', 'aggregate', 'publish_proof'],
                        jobs=[job('linux', 1, 'success', []), job('macos', 2, 'failure', []),
                              job('aggregate', 3, 'skipped', ['linux', 'macos']),
                              job('publish_proof', 4, 'skipped', ['aggregate'])],
                        transient_failures={'macos':'artifact_dns'}, failure_kinds={'macos':'transient'},
                        latest_job_ids={'linux':1,'macos':2,'aggregate':3,'publish_proof':4}, artifacts=[],
                        failure_evidence={'macos':dict(head_sha=self.head, run_id=42, job_id=2, run_attempt=1, path='failure.log',
                                                      sha256=hashlib.sha256(self.proof).hexdigest())})
    def go(self):
        return plan(self.doc, self.head, self.root)
    def blocked(self):
        with self.assertRaises(Blocked):
            self.go()
    def test_transitive_dependents_and_original_attempt(self):
        result = self.go()
        self.assertEqual(result['rerun_jobs'], ['aggregate', 'macos', 'publish_proof'])
        self.assertEqual(result['retained_successes'], ['linux'])
        self.assertEqual(result['observed_census'][0]['run_attempt'], 1)
        self.assertFalse(result['merge_ready'])
    def test_mixed_attempts_not_relabelled(self):
        self.doc['run']['run_attempt'] = 2
        for job in self.doc['jobs'][1:]:
            job['run_attempt'] = 2
        self.doc['failure_evidence']['macos']['run_attempt'] = 2
        result = self.go()
        self.assertEqual(result['proposed_attempt'], 3)
        self.assertEqual([j['run_attempt'] for j in result['observed_census']], [1, 2, 2, 2])
    def test_changed_head(self):
        self.doc['run']['head_sha'] = 'b' * 40
        self.blocked()
    def test_pending_run(self):
        self.doc['run']['status'] = 'in_progress'; self.blocked()
    def test_pending_job(self):
        self.doc['jobs'][0]['status'] = 'queued'; self.blocked()
    def test_unexplained_skipped(self):
        self.doc['jobs'][0]['conclusion'] = 'skipped'; self.blocked()
    def test_failed_dependency_requires_evidence(self):
        self.doc['jobs'][2]['conclusion'] = 'failure'
        self.blocked()
    def test_artifact_collision(self):
        self.doc['jobs'][1]['uploads'] = [{'name_template':'native', 'overwrite':False}]
        self.doc['artifacts'] = [dict(name='native', run_id=42, head_sha=self.head, id=123, producer_attempt=1, archive_path='failure.log', archive_sha256=hashlib.sha256(self.proof).hexdigest())]
        self.blocked()
    def test_unsuffixed_noncolliding_name_allowed(self):
        self.doc['jobs'][1]['uploads'] = [{'name_template':'native', 'overwrite':False}]
        self.assertEqual(self.go()['planned_artifact_names'], ['native'])
    def test_attempt_qualified_names(self):
        self.doc['jobs'][1]['uploads'] = [{'name_template':'native-{run_id}-{attempt}', 'overwrite':False}]
        self.assertEqual(self.go()['planned_artifact_names'], ['native-42-2'])
    def test_overwrite_forbidden(self):
        self.doc['jobs'][1]['uploads'] = [{'name_template':'native-{attempt}', 'overwrite':True}]
        self.blocked()
    def test_tampered_original_proof(self):
        (self.root/'failure.log').write_text('tampered'); self.blocked()
    def test_retry_budget(self):
        self.doc['run']['run_attempt'] = 3; self.blocked()
    def test_dependency_cycle(self):
        self.doc['jobs'][1]['needs'] = ['aggregate']; self.blocked()
    def test_missing_inventory(self):
        self.doc['required_jobs'].append('absent'); self.blocked()
    def test_wrong_job_binding(self):
        self.doc['jobs'][0]['head_sha'] = 'b' * 40; self.blocked()
    def test_independent_downstream_failure(self):
        self.doc['jobs'][2]['conclusion'] = 'failure'
        self.doc['failure_kinds']['aggregate'] = 'assertion'
        self.blocked()
    def test_stale_failed_job(self):
        self.doc['latest_job_ids']['macos'] = 99; self.blocked()
    def test_nonpositive_job_id(self):
        self.doc['jobs'][0]['id'] = 0; self.blocked()
    def test_bad_dependency_type(self):
        self.doc['jobs'][0]['needs'] = 'macos'; self.blocked()
    def test_duplicate_future_artifacts(self):
        for j in self.doc['jobs'][1:3]:
            j['uploads'] = [{'name_template':'same', 'overwrite':False}]
        self.blocked()
    def test_wrong_artifact_binding(self):
        self.doc['artifacts'] = [dict(name='native', run_id=43, head_sha=self.head)]
        self.blocked()
    def test_wrong_failure_proof_head(self):
        self.doc['failure_evidence']['macos']['head_sha'] = 'b'*40; self.blocked()
    def test_malformed_upload_template(self):
        self.doc['jobs'][1]['uploads'] = [{'name_template':'native-{', 'overwrite':False}]
        with self.assertRaises(ValueError): self.go()
    def test_real_macos_dns_snapshot_is_obsolete_and_incomplete(self):
        observed = json.loads((Path(__file__).parent/'fixtures/macos_dns_observation.json').read_text())
        job = observed['job']
        self.assertEqual(job['id'], 119227889203)
        self.assertEqual(job['conclusion'], 'failure')
        self.assertTrue(observed['other_jobs_running'])
        self.assertTrue(any(s['conclusion']=='skipped' for s in job['steps']))
        self.doc['run']['head_sha'] = job['head_sha']
        with self.assertRaisesRegex(Blocked, 'changed head'):
            plan(self.doc, 'db54a6b67d6830407209d4c8e89ed9e8799adb2e', self.root)


if __name__ == '__main__':
    unittest.main()
