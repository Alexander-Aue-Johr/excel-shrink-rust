"""CPU evidence must not turn serial activity or missing samples into overlap."""
import unittest

from report import batch_seconds, concurrent_cpu_seconds, loading_bars


def observation(second, cpus, ready=(False, False), pids=(100, 200)):
    return {'at': f'2026-11-01T00:00:{second:02d}+00:00',
            'clients': [{'pid': pid, 'cpu_seconds': cpu, 'ready': done}
                        for pid, cpu, done in zip(pids, cpus, ready)]}


class CpuOverlapTests(unittest.TestCase):
    def test_four_pending_processes_must_all_make_progress(self):
        ready = (False,) * 4
        pids = (100, 200, 300, 400)
        first = observation(0, (0, 0, 0, 0), ready, pids)
        simultaneous = observation(1, (0.5, 0.5, 0.5, 0.5), ready, pids)
        only_three = observation(1, (0.5, 0.5, 0.5, 0), ready, pids)
        self.assertEqual(concurrent_cpu_seconds([first, simultaneous]), 1)
        self.assertEqual(concurrent_cpu_seconds([first, only_three]), 0)

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


class SavedReportTests(unittest.TestCase):
    def test_old_pair_durations_remain_readable(self):
        self.assertEqual(batch_seconds({'pair_seconds': 12}), 12)
        self.assertEqual(batch_seconds({'batch_seconds': 24}), 24)

    def test_timeline_reserves_space_for_all_four_workbooks(self):
        clients = [{'pid': index, 'requested_at': '2026-11-01T00:00:00Z',
                    'ready_at': '2026-11-01T00:00:10Z'} for index in range(4)]
        svg = loading_bars({'arrangement': 'same_desktop', 'clients': clients}, 10)
        self.assertIn('Excel 4', svg)
        self.assertIn('viewBox="0 0 820 163"', svg)


if __name__ == '__main__':
    unittest.main()
