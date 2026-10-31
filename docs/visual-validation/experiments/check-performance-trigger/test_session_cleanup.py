"""Check resource ownership when experiment startup fails, without opening Excel."""
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch


spec = importlib.util.spec_from_file_location('performance_experiment', Path(__file__).with_name('run.py'))
experiment = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = experiment
spec.loader.exec_module(experiment)


class SessionCleanupTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='check-performance-test-')
        self.addCleanup(self.directory.cleanup)
        directory = Path(self.directory.name)
        self.helper = Mock()
        self.fixture = experiment.Experiment(
            directory, directory / 'worker', directory / 'session', self.helper, 12345, {})
        self.launcher = Mock()

    def assert_launcher_closed(self, spawn):
        self.helper.publish.assert_called_once_with(
            self.fixture.session / 'finish.json', {'complete': True})
        self.launcher.wait.assert_called_once_with(timeout=30)
        self.assertTrue(spawn.call_args_list[0].kwargs['stdout'].closed)
        self.assertTrue(spawn.call_args_list[0].kwargs['stderr'].closed)

    def test_addin_startup_failure_closes_launcher_and_logs(self):
        self.helper.wait_until.side_effect = RuntimeError('Add-in startup failed')
        with patch.object(experiment.subprocess, 'Popen', return_value=self.launcher) as spawn:
            with self.assertRaisesRegex(RuntimeError, 'Add-in startup failed'):
                with experiment.open_excel(self.fixture, Path('runner.exe')):
                    self.fail('An unready Excel session must not be yielded')
        self.assert_launcher_closed(spawn)

    def test_debugger_startup_failure_closes_both_processes_and_logs(self):
        debugger = Mock()
        self.helper.wait_until.side_effect = [None, RuntimeError('Debugger startup failed')]
        pages = [{'url': 'http://localhost/taskpane.html', 'webSocketDebuggerUrl': 'ws://localhost/test'}]
        with patch.object(experiment.subprocess, 'Popen', side_effect=[self.launcher, debugger]) as spawn:
            with patch.object(experiment, 'targets', return_value=pages):
                with self.assertRaisesRegex(RuntimeError, 'Debugger startup failed'):
                    with experiment.open_excel(self.fixture, Path('runner.exe')) as session:
                        experiment.attach_runtime_debugger(self.fixture, session)
        self.assert_launcher_closed(spawn)
        debugger.terminate.assert_called_once_with()
        debugger.wait.assert_called_once_with(timeout=10)


if __name__ == '__main__':
    unittest.main()
