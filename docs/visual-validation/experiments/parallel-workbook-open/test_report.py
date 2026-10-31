"""CPU evidence must not turn serial activity or missing samples into overlap."""
import unittest

from report import concurrent_cpu_seconds


def observation(second, cpus, ready=(False, False), pids=(100, 200)):
    return {'at': f'2026-11-01T00:00:{second:02d}+00:00',
            'clients': [{'pid': pid, 'cpu_seconds': cpu, 'ready': done}
                        for pid, cpu, done in zip(pids, cpus, ready)]}


class CpuOverlapTests(unittest.TestCase):
    def test_simultaneous_pending_progress_counts_elapsed_interval(self):
        samples = [observation(0, (0, 0)), observation(1, (0.5, 0.5))]
        self.assertEqual(concurrent_cpu_seconds(samples), 1)

    def test_serial_cpu_progress_does_not_count_as_parallel(self):
        samples = [observation(0, (0, 0)), observation(1, (1, 0)), observation(2, (1, 1))]
        self.assertEqual(concurrent_cpu_seconds(samples), 0)

    def test_background_cpu_after_readiness_is_excluded(self):
        samples = [observation(0, (0, 0), (True, False)),
                   observation(1, (0.5, 0.5), (True, False))]
        self.assertEqual(concurrent_cpu_seconds(samples), 0)

    def test_missing_process_sample_is_not_evidence_of_overlap(self):
        samples = [observation(0, (None, 0)), observation(1, (0.5, 0.5))]
        self.assertEqual(concurrent_cpu_seconds(samples), 0)

    def test_replaced_process_is_not_evidence_of_overlap(self):
        samples = [observation(0, (0, 0)), observation(1, (0.5, 0.5), pids=(300, 200))]
        self.assertEqual(concurrent_cpu_seconds(samples), 0)


if __name__ == '__main__':
    unittest.main()
