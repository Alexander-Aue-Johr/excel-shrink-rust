"""Recover owned browser leftovers while preserving unrelated profiles."""

import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, MagicMock, Mock, patch

from tools.check_performance_onedrive import browser_profile as profiles
from tools.check_performance_onedrive import runner as flow


@unittest.skipIf(profiles.psutil is None, "psutil is required for process ownership tests")
class BrowserProfileTests(unittest.IsolatedAsyncioTestCase):
    def test_profile_match_requires_a_full_user_data_directory_argument(self):
        with tempfile.TemporaryDirectory() as temporary:
            profile = Path(temporary) / "profile"
            process = MagicMock()
            process.name.return_value = "msedge.exe"
            process.cmdline.return_value = ["msedge.exe", f"--user-data-dir={profile}-other"]
            self.assertFalse(profiles.uses_profile(process, profile))
            process.cmdline.return_value = ["msedge.exe", "--user-data-dir", str(profile)]
            self.assertTrue(profiles.uses_profile(process, profile))

    async def test_temporary_session_lock_is_retried_before_launch(self):
        with tempfile.TemporaryDirectory() as temporary:
            discard = Mock(side_effect=[PermissionError("WinError 32"), None])
            with (patch.object(profiles.psutil, "process_iter", return_value=[]),
                  patch.object(profiles.asyncio, "sleep", new=AsyncMock()) as sleep):
                await profiles.prepare_profile(Path(temporary), discard)
            self.assertEqual(discard.call_count, 2)
            sleep.assert_awaited_once_with(0.25)

    async def test_persistent_session_lock_has_a_short_bounded_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            with (patch.object(profiles.psutil, "process_iter", return_value=[]),
                  patch.object(profiles.time, "monotonic", side_effect=[0, 6])):
                with self.assertRaisesRegex(profiles.BrowserProfileBusy, "remain locked"):
                    await profiles.prepare_profile(Path(temporary), Mock(side_effect=PermissionError("locked")))

    async def test_unrecorded_profile_browser_is_not_terminated(self):
        with tempfile.TemporaryDirectory() as temporary:
            profile = Path(temporary)
            process = MagicMock()
            process.name.return_value = "msedge.exe"
            process.cmdline.return_value = ["msedge.exe", f"--user-data-dir={profile}"]
            with patch.object(profiles.psutil, "process_iter", return_value=[process]):
                with self.assertRaisesRegex(profiles.BrowserProfileBusy, "unrecorded browser"):
                    await profiles.prepare_profile(profile, Mock())
            process.terminate.assert_not_called()
            process.kill.assert_not_called()

    async def test_reused_browser_pid_is_not_terminated(self):
        with tempfile.TemporaryDirectory() as temporary:
            profile = Path(temporary)
            owner = {"profile": str(profile), "runner_pid": 333, "runner_created": 1,
                     "browser_pid": 444, "browser_created": 1}
            profiles.owner_path(profile).write_text(json.dumps(owner), encoding="utf-8")
            process = MagicMock(pid=444)
            process.create_time.return_value = 2

            def lookup(pid):
                if pid == 333:
                    raise profiles.psutil.NoSuchProcess(pid)
                return process

            with patch.object(profiles.psutil, "Process", side_effect=lookup):
                await profiles.release_owned_browser(profile)
            process.terminate.assert_not_called()
            process.kill.assert_not_called()
            self.assertFalse(profiles.owner_path(profile).exists())

    async def test_recorded_browser_of_another_active_runner_is_preserved(self):
        with tempfile.TemporaryDirectory() as temporary:
            profile = Path(temporary)
            owner = {"profile": str(profile), "runner_pid": 333, "runner_created": 1,
                     "browser_pid": 444, "browser_created": 2}
            profiles.owner_path(profile).write_text(json.dumps(owner), encoding="utf-8")
            other_runner = MagicMock(pid=333)
            other_runner.create_time.return_value = 1
            with patch.object(profiles.psutil, "Process", return_value=other_runner):
                with self.assertRaisesRegex(profiles.BrowserProfileBusy, "another active runner"):
                    await profiles.release_owned_browser(profile)
            other_runner.terminate.assert_not_called()
            self.assertTrue(profiles.owner_path(profile).exists())

    async def test_busy_profile_does_not_enter_nine_browser_restart_attempts(self):
        with (patch.object(flow, "close_browser", new=AsyncMock()),
              patch.object(flow, "start_folder_browser", new=AsyncMock(
                  side_effect=profiles.BrowserProfileBusy("another active runner"))) as start,
              patch.object(flow.asyncio, "sleep", new=AsyncMock()) as sleep):
            with self.assertRaises(profiles.BrowserProfileBusy):
                await flow.restart_folder_browser(None, None, object(), Path("profile"))
        start.assert_awaited_once()
        sleep.assert_not_awaited()

    async def test_owned_browser_is_released_even_after_driver_cleanup_fails(self):
        with tempfile.TemporaryDirectory() as temporary:
            profile = Path(temporary)
            args = SimpleNamespace(folder_url="https://example.test", browser_channel="chromium", headed=False)
            driver = context = None
            with (patch.object(flow, "open_folder", new=AsyncMock(side_effect=lambda page, *_: page)),
                  patch.object(flow, "sort_oldest_first", new=AsyncMock())):
                try:
                    driver, context, _ = await flow.start_folder_browser(args, profile)
                    owner = json.loads(profiles.owner_path(profile).read_text(encoding="utf-8"))
                    with (patch.object(context, "close", new=AsyncMock(side_effect=RuntimeError("disconnected"))),
                          patch.object(driver, "stop", new=AsyncMock(side_effect=RuntimeError("dead driver")))):
                        await flow.close_browser(driver, context)
                    self.assertIsNone(profiles.same_process(owner["browser_pid"], owner["browser_created"]))
                    self.assertFalse(profiles.owner_path(profile).exists())
                    await driver.stop()
                    driver, context, _ = await flow.start_folder_browser(args, profile)
                    self.assertTrue(profiles.owner_path(profile).exists())
                finally:
                    await flow.close_browser(driver, context)


if __name__ == "__main__":
    unittest.main()
