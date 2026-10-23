"""A process-free check of the common start line and completion-driven slots."""
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
spec = importlib.util.spec_from_file_location('quartet_runner', ROOT / 'run.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class SchedulerTests(unittest.TestCase):
    def test_signal_is_not_visible_until_the_writer_has_closed_it(self):
        with tempfile.TemporaryDirectory() as temporary:
            signal = Path(temporary) / 'phase-3.launch'
            partial = threading.Event()
            finish = threading.Event()
            closed = threading.Event()
            errors = []
            replace = Path.replace

            def slow_write(path, content, **kwargs):
                with path.open('w', **kwargs) as stream:
                    stream.write(content[:1])
                    stream.flush()
                    partial.set()
                    if not finish.wait(2):
                        raise TimeoutError('Test writer was not released')
                    stream.write(content[1:])
                closed.set()
                return len(content)

            def checked_replace(path, destination):
                if not closed.is_set():
                    raise AssertionError('Published a signal while its writer was open')
                return replace(path, destination)

            def publish():
                try:
                    runner.publish_signal(signal, 'go\n')
                except Exception as error:
                    errors.append(error)

            with patch.object(Path, 'write_text', slow_write), patch.object(
                    Path, 'replace', checked_replace):
                writer = threading.Thread(target=publish)
                writer.start()
                try:
                    self.assertTrue(partial.wait(2))
                    self.assertFalse(signal.exists())
                    self.assertTrue(signal.with_name(signal.name + '.pending').exists())
                finally:
                    finish.set()
                    writer.join(2)
            self.assertFalse(writer.is_alive())
            self.assertEqual(errors, [])
            self.assertTrue(closed.is_set())
            self.assertEqual(signal.read_text(encoding='utf-8'), 'go\n')
            self.assertFalse(signal.with_name(signal.name + '.pending').exists())

    def test_all_sixteen_ready_before_clock_and_slot_limit(self):
        for limit in (1, 2, 4):
            with self.subTest(active_groups=limit), tempfile.TemporaryDirectory() as temporary:
                output = Path(temporary)
                entries = [{'name': f'book{i}.xlsx', **{
                    side: {'bytes': 1} for side in ('original', *runner.PROFILES)}}
                    for i in range(4)]
                plans = {entry['name']: {'capture': {}} for entry in entries}
                events = []
                lock = threading.Lock()

                def worker(entry, _capture, _runtime, _launcher, trial, _args):
                    group = trial / Path(entry['name']).stem
                    group.mkdir()
                    group.joinpath('stage-ready.signal').write_text('ready')
                    while not (trial / 'launch.signal').exists():
                        time.sleep(.001)
                    for phase in range(4):
                        while not group.joinpath(f'phase-{phase}.launch').exists():
                            time.sleep(.001)
                        group.joinpath(f'phase-{phase}.ready').write_text('ready')
                        if phase < 3:
                            while not (trial / f'phase-{phase}.signal').exists():
                                time.sleep(.001)
                    time.sleep(.02)  # slow workbook identity/geometry preflight
                    with lock:
                        events.append(('ready', entry['name'], time.monotonic()))
                    group.joinpath('ready.signal').write_text('ready')
                    while not group.joinpath('start.signal').exists():
                        time.sleep(.001)
                    with lock:
                        events.append(('start', entry['name'], time.monotonic()))
                    time.sleep(.025)
                    with lock:
                        events.append(('done', entry['name'], time.monotonic()))
                    runner.publish_signal(group / 'capture-result.json', json.dumps({'status': 'complete'}))
                    while not group.joinpath('release.signal').exists():
                        time.sleep(.001)
                    return {'status': 'complete', 'workbook': entry['name']}

                args = type('Args', (), {'ram_reserve_mib': 1024,
                    'startup_timeout': 1, 'capture_timeout': 2})()
                with patch.object(runner, 'run_one', worker), patch.object(
                        runner, 'available_mib', lambda: 32768):
                    row = runner.run_trial(entries, plans, output, output, output,
                                           0, limit, args)
                self.assertEqual(row['status'], 'complete', row.get('error'))
                starts = [event[2] for event in events if event[0] == 'start']
                ready = [event[2] for event in events if event[0] == 'ready']
                self.assertEqual(len(starts), 4)
                self.assertLess(max(ready), min(starts))
                active = 0
                maximum = 0
                for event, _name, _time in sorted(events, key=lambda item: item[2]):
                    if event == 'start':
                        active += 1
                        maximum = max(maximum, active)
                    elif event == 'done':
                        active -= 1
                self.assertEqual(maximum, limit)
                self.assertEqual(active, 0)
                self.assertGreater(row['measured_seconds'], 0)
                self.assertLess(row['measured_seconds'], 1)


if __name__ == '__main__':
    unittest.main()
