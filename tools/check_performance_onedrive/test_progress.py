"""Verify resume data and forward-only OneDrive row navigation."""

import json
import io
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import AsyncMock, MagicMock, patch


from tools.check_performance_onedrive import runner as FLOW
from tools.check_performance_onedrive.runner import JobState, ProgressStore

class ProgressStoreTests(unittest.TestCase):
    def test_run_level_error_exits_without_traceback_or_success(self) -> None:
        """A broken folder or browser stops cleanly instead of skipping the corpus."""
        with (patch.object(FLOW, "run_folder_check_performance", new=lambda _args: None),
              patch.object(FLOW.asyncio, "run", side_effect=RuntimeError("folder unavailable")),
              patch.object(FLOW.sys, "stderr", io.StringIO()) as output):
            self.assertEqual(FLOW.run_check_performance(None), 1)
        self.assertIn("folder unavailable", output.getvalue())

    def test_interrupted_checking_attempt_restarts(self) -> None:
        """A stopped browser run resumes without an invalid state transition."""
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            with ProgressStore(directory, "folder-one") as progress:
                progress.discover(["interrupted.xlsx", "done.xlsx"])
                first_attempt = progress.start_attempt("interrupted.xlsx")
                progress.transition("interrupted.xlsx", JobState.CHECKING)
                done_attempt = progress.start_attempt("done.xlsx")
                progress.transition("done.xlsx", JobState.CHECKING)
                progress.finish("done.xlsx", done_attempt, "already_clean")

            with ProgressStore(directory, "folder-one") as progress:
                next_attempt = progress.start_attempt("interrupted.xlsx")
                self.assertGreater(next_attempt, first_attempt)
                self.assertEqual(progress.status("interrupted.xlsx"), JobState.OPENING.value)
                previous = progress.connection.execute(
                    "SELECT finished_at, error FROM attempts WHERE id = ?", (first_attempt,)).fetchone()
                self.assertIsNotNone(previous[0])
                self.assertEqual(previous[1], "Interrupted before restart")
                with self.assertRaises(ValueError):
                    progress.start_attempt("done.xlsx")

    def test_unavailable_performance_is_recorded_as_skipped(self) -> None:
        """A disabled Excel feature must remain distinct from a successful check."""
        with tempfile.TemporaryDirectory() as temporary:
            with ProgressStore(Path(temporary), "folder-one") as progress:
                progress.discover(["unavailable.xlsx", "pending.xlsx"])
                attempt = progress.start_attempt("unavailable.xlsx")
                progress.transition("unavailable.xlsx", JobState.CHECKING)
                progress.skip("unavailable.xlsx", attempt, "performance_unavailable")
                self.assertEqual(progress.status("unavailable.xlsx"), JobState.SKIPPED.value)
                self.assertEqual(progress.unfinished_names(), {"pending.xlsx"})
                self.assertEqual(progress.unfinished_names(retry_skipped=True),
                                 {"unavailable.xlsx", "pending.xlsx"})
                self.assertEqual(progress.counts(), (1, 2))
                self.assertEqual(progress.skipped_count(), 1)
                progress.start_attempt("unavailable.xlsx")
                self.assertEqual(progress.status("unavailable.xlsx"), JobState.OPENING.value)

    def test_error_after_optimization_can_be_skipped_after_retry(self) -> None:
        """Two failed save attempts must retain their errors without claiming success."""
        with tempfile.TemporaryDirectory() as temporary:
            with ProgressStore(Path(temporary), "folder-one") as progress:
                progress.discover(["uncertain.xlsx"])
                attempt = progress.start_attempt("uncertain.xlsx")
                for state in (JobState.CHECKING, JobState.OPTIMIZING, JobState.SAVING):
                    progress.transition("uncertain.xlsx", state)
                progress.fail("uncertain.xlsx", attempt, "Save timed out")
                progress.skip_failed("uncertain.xlsx", "error_after_retry")
                self.assertEqual(progress.status("uncertain.xlsx"), JobState.SKIPPED.value)
                self.assertEqual(progress.outcome("uncertain.xlsx"), "error_after_retry")
                self.assertEqual(progress.connection.execute(
                    "SELECT error FROM attempts WHERE id = ?", (attempt,)).fetchone()[0],
                    "Save timed out")
                self.assertNotIn("uncertain.xlsx", progress.unfinished_names())

    def test_json_progress_migrates_and_failed_files_resume(self) -> None:
        """The SQLite upgrade must retain completed work and old timed events."""
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            legacy = {
                "folder_id": "folder-one",
                "created_at": "2026-11-03T12:00:00+00:00",
                "files": {
                    "done.xlsx": {
                        "status": "completed", "outcome": "optimized",
                        "attempts": [{
                            "started_at": "2026-11-03T12:01:00+00:00",
                            "finished_at": "2026-11-03T12:01:02+00:00",
                            "events": [{"phase": "cloud_saved", "at": "2026-11-03T12:01:02+00:00",
                                        "elapsed_seconds": 2.0, "since_previous_seconds": 2.0}],
                        }],
                    },
                    "interrupted.xlsx": {"status": "running", "attempts": []},
                },
            }
            (directory / "progress.json").write_text(json.dumps(legacy), encoding="utf-8")

            with ProgressStore(directory, "folder-one") as progress:
                self.assertEqual(progress.counts(), (1, 2))
                self.assertEqual(progress.status("interrupted.xlsx"), JobState.OPENING.value)
                attempt = progress.start_attempt("interrupted.xlsx")
                progress.transition("interrupted.xlsx", JobState.CHECKING)
                progress.fail("interrupted.xlsx", attempt, "temporary error")
                self.assertEqual(progress.status("interrupted.xlsx"), JobState.FAILED.value)
                self.assertEqual(progress.connection.execute(
                    "SELECT COUNT(*) FROM events").fetchone()[0], 1)

            with ProgressStore(directory, "folder-one") as progress:
                self.assertEqual(progress.counts(), (1, 2))
                self.assertEqual(progress.status("interrupted.xlsx"), JobState.FAILED.value)
                attempt = progress.start_attempt("interrupted.xlsx")
                progress.transition("interrupted.xlsx", JobState.CHECKING)
                progress.finish("interrupted.xlsx", attempt, "already_clean")
                self.assertEqual(progress.counts(), (2, 2))
            self.assertTrue((directory / "progress.json").exists())

    def test_terminal_summary_counts_success_and_keeps_failure_pending(self) -> None:
        """The live summary reflects durable completion, not merely attempted files."""
        class Terminal(io.StringIO):
            def isatty(self):
                return True

        with tempfile.TemporaryDirectory() as temporary:
            with ProgressStore(Path(temporary), "folder-one") as progress:
                progress.discover(["done.xlsx", "failed.xlsx"])
                output = Terminal()
                with patch.object(FLOW.sys, "stdout", output):
                    display = FLOW.ConsoleProgress(progress)
                    display.begin("done.xlsx", JobState.PENDING.value)
                    attempt = FLOW.AttemptClock(1, time.monotonic() - 5)
                    attempt.phase_elapsed = {"editor_ready": 2, "review_ready": 3,
                                             "performance_ready": 4, "optimization_finished": 5}
                    display.finish("done.xlsx", attempt, "already_clean", None)
                    display.begin("failed.xlsx", JobState.PENDING.value)
                    display.finish("failed.xlsx", attempt, "", "temporary error")
                    display.close()
                text = output.getvalue()
                self.assertIn("done.xlsx", text)
                self.assertIn("already_clean", text)
                self.assertIn("failed.xlsx", text)
                self.assertIn("Processed 1/2", text)
                self.assertIn("remaining 1", text)
                self.assertIn("ETA ~", text)


class FakePage:
    def __init__(self, position: int, target: int) -> None:
        self.position = position
        self.target = target
        self.resets = 0

    def get_by_text(self, _name, exact: bool):
        page = self

        class FakeRow:
            @property
            def first(self):
                return self

            async def count(self):
                return int(page.position == page.target)

        return FakeRow()


async def fake_scroll(page: FakePage, to_top: bool):
    if to_top:
        page.position = 0
        page.resets += 1
    else:
        page.position = min(page.position + 1, 5)
    return page.position, 5


class RowCursorTests(unittest.IsolatedAsyncioTestCase):
    async def test_dead_driver_does_not_mask_browser_restart(self) -> None:
        """BrowserContext.close may fail after the Playwright connection dies."""
        context = MagicMock()
        context.close = AsyncMock(side_effect=RuntimeError("Connection closed"))
        playwright = MagicMock()
        playwright.stop = AsyncMock()
        with patch.object(FLOW.sys, "stderr", io.StringIO()) as output:
            await FLOW.close_browser(playwright, context)
        playwright.stop.assert_awaited_once()
        self.assertIn("Connection closed", output.getvalue())

    async def test_folder_timeout_restarts_browser(self) -> None:
        """A folder timeout before file selection gets a new browser window."""
        playwright, context, page = object(), object(), object()
        with (patch.object(FLOW, "close_browser", new_callable=AsyncMock) as close,
              patch.object(FLOW, "start_folder_browser", new_callable=AsyncMock,
                           side_effect=[TimeoutError("folder stalled"),
                                        (playwright, context, page)]) as start,
              patch.object(FLOW.asyncio, "sleep", new_callable=AsyncMock) as sleep):
            result = await FLOW.restart_folder_browser(None, None, object(), Path("/tmp"))
        self.assertEqual(result, (playwright, context, page))
        self.assertEqual(start.await_count, 2)
        close.assert_awaited_once_with(None, None)
        sleep.assert_awaited_once_with(5)

    async def test_two_failed_attempts_skip_only_that_workbook(self) -> None:
        """A fresh browser is tried once before moving to the next workbook."""
        with tempfile.TemporaryDirectory() as temporary:
            with ProgressStore(Path(temporary), "folder-one") as progress:
                progress.discover(["broken.xlsx", "next.xlsx"])

                async def fail_attempt(*_args):
                    attempt = progress.start_attempt("broken.xlsx")
                    progress.fail("broken.xlsx", attempt, "Excel did not respond")

                playwright, context, page = object(), object(), object()
                with (patch.object(FLOW, "process_with_live_status", side_effect=fail_attempt) as process,
                      patch.object(FLOW, "restart_folder_browser", new_callable=AsyncMock,
                                   return_value=(playwright, context, page)) as restart):
                    _, _, _, failed = await FLOW.process_selected_workbook(
                        playwright, context, page, object(), Path(temporary), progress,
                        MagicMock(), "broken.xlsx", None)
                self.assertTrue(failed)
                self.assertEqual(process.await_count, 2)
                self.assertEqual(restart.await_count, 2)
                self.assertEqual(progress.outcome("broken.xlsx"), "error_after_retry")
                self.assertEqual(progress.status("broken.xlsx"), JobState.SKIPPED.value)
                self.assertIn("next.xlsx", progress.unfinished_names())

    async def test_second_attempt_can_complete(self) -> None:
        """A successful retry is recorded as completed, not skipped."""
        with tempfile.TemporaryDirectory() as temporary:
            with ProgressStore(Path(temporary), "folder-one") as progress:
                progress.discover(["retry.xlsx"])

                async def attempt_workbook(*_args):
                    attempt = progress.start_attempt("retry.xlsx")
                    if progress.connection.execute(
                            "SELECT COUNT(*) FROM attempts WHERE file_name = 'retry.xlsx'").fetchone()[0] == 1:
                        progress.fail("retry.xlsx", attempt, "Browser closed")
                    else:
                        progress.transition("retry.xlsx", JobState.CHECKING)
                        progress.finish("retry.xlsx", attempt, "already_clean")

                playwright, context, page = object(), object(), object()
                with (patch.object(FLOW, "process_with_live_status", side_effect=attempt_workbook),
                      patch.object(FLOW, "restart_folder_browser", new_callable=AsyncMock,
                                   return_value=(playwright, context, page)) as restart):
                    _, _, _, failed = await FLOW.process_selected_workbook(
                        playwright, context, page, object(), Path(temporary), progress,
                        MagicMock(), "retry.xlsx", None)
                self.assertFalse(failed)
                restart.assert_awaited_once()
                self.assertEqual(progress.status("retry.xlsx"), JobState.COMPLETED.value)

    async def test_folder_listing_failure_recovers_once(self) -> None:
        """A transient list error must not abort the remaining workbooks."""
        folder = object()
        recovered = object()
        with (patch.object(FLOW, "next_visible_unfinished", new_callable=AsyncMock,
                           side_effect=[RuntimeError("stale view"), ("next.xlsx", "row")]) as next_file,
              patch.object(FLOW, "recover_folder", new_callable=AsyncMock,
                           return_value=recovered) as recover):
            page, item = await FLOW.next_workbook_with_recovery(
                None, folder, "https://1drv.ms/f/example", True, {"next.xlsx"})
        self.assertIs(page, recovered)
        self.assertEqual(item, ("next.xlsx", "row"))
        self.assertEqual(next_file.await_count, 2)
        recover.assert_awaited_once_with(None, folder, "https://1drv.ms/f/example", True)

    async def test_next_file_continues_from_current_position(self) -> None:
        """The common ordered case must not scan from the folder top again."""
        page = FakePage(position=2, target=3)
        with patch.object(FLOW, "scroll_file_list", fake_scroll):
            await FLOW.find_file_row(page, "next.xlsx")
        self.assertEqual(page.resets, 0)

    async def test_reordered_file_wraps_once(self) -> None:
        """A file moved before the cursor remains discoverable after one wrap."""
        page = FakePage(position=4, target=1)
        with patch.object(FLOW, "scroll_file_list", fake_scroll):
            await FLOW.find_file_row(page, "earlier.xlsx")
        self.assertEqual(page.resets, 1)

    async def test_next_visible_unfinished_uses_current_rows(self) -> None:
        """A known pending file in view needs neither discovery nor a scroll."""
        class VisibleRows:
            async def all_text_contents(self):
                return ["done.xlsx", "next.xlsx"]

        class FolderPage:
            def locator(self, _selector):
                return VisibleRows()

            def get_by_text(self, name, exact):
                class Row:
                    first = name

                return Row()

        page = FolderPage()
        with patch.object(FLOW, "scroll_file_list", side_effect=AssertionError("unexpected scroll")):
            result = await FLOW.next_visible_unfinished(page, {"next.xlsx"})
        self.assertEqual(result, ("next.xlsx", "next.xlsx"))


if __name__ == "__main__":
    unittest.main()
