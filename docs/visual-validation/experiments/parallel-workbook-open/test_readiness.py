"""The experiment endpoint is verified Office.js readiness, not COM admission."""
import unittest

from run import verified_ready_at


class VerifiedReadinessTests(unittest.TestCase):
    def test_verified_workbook_does_not_wait_for_launcher_sample(self):
        client = {'startup_seconds': None, 'stage': 'waiting_for_startup_sample',
                  'monitor': {'events': [{'kind': 'addin_ready', 'at': '2026-11-01T00:00:00Z'}]}}
        self.assertEqual(verified_ready_at(client), '2026-11-01T00:00:00Z')

    def test_configuration_and_heartbeats_do_not_prove_workbook_readiness(self):
        client = {'heartbeat_count': 20, 'monitor': {'events': [
            {'kind': 'addin_configured', 'at': '2026-11-01T00:00:00Z'},
            {'kind': 'state_changed', 'state': 'verifying_workbook'}]}}
        self.assertIsNone(verified_ready_at(client))

    def test_missing_evidence_does_not_count_as_success(self):
        self.assertIsNone(verified_ready_at({}))
        self.assertIsNone(verified_ready_at({'monitor': None}))
        self.assertIsNone(verified_ready_at({'monitor': {'events': None}}))


if __name__ == '__main__':
    unittest.main()
