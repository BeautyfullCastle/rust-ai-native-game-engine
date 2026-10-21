#!/usr/bin/env python3
"""Fail-closed identity fixtures. No Cargo and no native-acceptance claim."""
from unittest import mock
import unittest
import sample_ci_identity as identity

class Tests(unittest.TestCase):
    def setUp(self):
        self.current = dict(head='head', tree='tree', lock_sha256='lock', rustc='rustc',
                            GITHUB_SHA='head', GITHUB_RUN_ID='run', GITHUB_RUN_ATTEMPT='1',
                            RUNNER_OS='Linux', RUNNER_ARCH='X64', PR_HEAD_SHA='pr',
                            GITHUB_EVENT_NAME='pull_request', GITHUB_REF='refs/pull/277/merge')
        self.binding = dict(schema=1, status='started', identity=self.current.copy())
        self.receipt = dict(status='passed', mode='cpu', ui_profile=True, head='head', tree='tree',
                            source_unchanged=True, source_authority_unchanged=True,
                            source_sha256={'Cargo.lock': {'sha256': 'lock'}},
                            commands=[{'state': 'passed'}], coverage={'required': 'passed'},
                            producers={'harness': {'sha256': 'bytes', 'bytes': 123,
                              'producer': {'fresh': False, 'reason': 'compiler-artifact',
                                           'features': ['project'], 'profile': {'test': True}}}})
    def check(self):
        return identity.verify(self.binding, self.current, self.receipt)
    def test_pull_request_context(self):
        self.assertEqual(self.actual_context('pull_request', 'pr')['PR_HEAD_SHA'], 'pr')
    def test_dispatch_context(self):
        observed = self.actual_context('workflow_dispatch', '')
        self.assertIsNone(observed['PR_HEAD_SHA'])
        self.assertEqual(observed['GITHUB_REF'], 'refs/heads/development')
    def test_missing_pr_head_context_rejects(self):
        with self.assertRaises(AssertionError): self.actual_context('pull_request', '')
    def test_dispatch_fabricated_pr_head_rejects(self):
        with self.assertRaises(AssertionError): self.actual_context('workflow_dispatch', 'pr')
    def actual_context(self, event, pr):
        env=dict(self.current, GITHUB_EVENT_NAME=event, GITHUB_REF='refs/heads/development', PR_HEAD_SHA=pr)
        values=['head', 'tree', 'rustc', '']
        with mock.patch.dict(identity.os.environ, env, clear=True), \
             mock.patch.object(identity.subprocess, 'check_output', side_effect=values), \
             mock.patch.object(identity, 'digest', return_value='lock'):
            return identity.context()
    def test_current_complete_receipt(self):
        self.assertEqual(set(self.check()), {'harness'})
    def test_each_identity_dimension_rejects_stale_receipt(self):
        for key in self.current:
            with self.subTest(key=key):
                original = self.current[key]; self.current[key] = 'different'
                with self.assertRaises(AssertionError): self.check()
                self.current[key] = original
    def test_missing_receipt_fields_reject(self):
        for key in list(self.receipt):
            with self.subTest(key=key):
                original = self.receipt.pop(key)
                with self.assertRaises((AssertionError, KeyError)): self.check()
                self.receipt[key] = original
    def test_failed_cancelled_skipped_started_reject(self):
        for status in ('failed', 'cancelled', 'skipped', 'started', None):
            self.receipt['status'] = status
            with self.assertRaises(AssertionError): self.check()
    def test_changed_source_or_lock_reject(self):
        for field in ('source_unchanged', 'source_authority_unchanged'):
            self.receipt[field] = False
            with self.assertRaises(AssertionError): self.check()
            self.receipt[field] = True
        self.receipt['source_sha256']['Cargo.lock']['sha256'] = 'changed'
        with self.assertRaises(AssertionError): self.check()
    def test_missing_or_skipped_commands_reject(self):
        for commands in ([], [{'state': 'skipped'}], [{'state': 'failed'}]):
            self.receipt['commands'] = commands
            with self.assertRaises(AssertionError): self.check()
    def test_cached_or_missing_producer_reject(self):
        self.receipt['producers']['harness']['producer']['fresh'] = True
        with self.assertRaises(AssertionError): self.check()
        self.receipt['producers'] = {}
        with self.assertRaises(AssertionError): self.check()
    def test_different_checkout_receipt_reject(self):
        for field in ('head', 'tree'):
            original = self.receipt[field]; self.receipt[field] = 'other'
            with self.assertRaises(AssertionError): self.check()
            self.receipt[field] = original
    def test_reverification_of_finished_identity_rejects(self):
        self.binding['status'] = 'passed'
        with self.assertRaises(AssertionError): self.check()

if __name__ == '__main__':
    result=unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Tests))
    assert result.wasSuccessful() and result.testsRun == 13 and not result.skipped
