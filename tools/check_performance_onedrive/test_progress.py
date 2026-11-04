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

class PerformancePaneHostTests(unittest.IsolatedAsyncioTestCase):
    async def test_performance_controls_are_found_in_both_editor_and_resource_frames(self):
        """The pane can be embedded in Excel rather than resources.office.net."""
        for host in ("https://excel.cloud.microsoft/editor", "https://resources.office.net/pane"):
            frame = MagicMock()
            frame.url = host
            frame.get_by_role.return_value.count = AsyncMock(return_value=1)
            page = MagicMock()
            page.frames = [frame]
            command = MagicMock()
            command.wait_for = AsyncMock()
            command.is_enabled = AsyncMock(return_value=True)
            command.click = AsyncMock()
            editor = MagicMock()
            editor.locator.return_value.first = command
            with (patch.object(FLOW, "raise_if_workbook_blocked", new=AsyncMock()),
                  patch.object(FLOW.time, "monotonic", side_effect=[0, 0, 0, 121]),
                  patch.object(FLOW.asyncio, "sleep", new=AsyncMock())):
                self.assertIs(await FLOW.find_performance_pane(page, editor), frame)


    async def test_detached_frame_does_not_hide_a_live_performance_pane(self):
        from playwright.async_api import Error
        old, live = MagicMock(), MagicMock()
        old.get_by_role.return_value.count = AsyncMock(side_effect=Error("Frame was detached"))
        old.is_detached.return_value = True
        live.get_by_role.return_value.count = AsyncMock(return_value=1)
        page = MagicMock()
        page.frames = [old, live]
        editor = MagicMock()
        command = editor.locator.return_value.first
        command.wait_for = AsyncMock()
        command.is_enabled = AsyncMock(return_value=True)
        command.click = AsyncMock()
        with (patch.object(FLOW, "raise_if_workbook_blocked", new=AsyncMock()),
              patch.object(FLOW.time, "monotonic", side_effect=[0, 0, 0, 121])):
            self.assertIs(await FLOW.find_performance_pane(page, editor), live)
        old.is_detached.return_value = False
        with (patch.object(FLOW, "raise_if_workbook_blocked", new=AsyncMock()),
              patch.object(FLOW.time, "monotonic", side_effect=[0, 0, 0, 121])):
            with self.assertRaises(Error):
                await FLOW.find_performance_pane(page, editor)


class EditorReadinessTests(unittest.IsolatedAsyncioTestCase):
    async def test_duplicate_review_controls_use_the_first_visible_match(self):
        from playwright.async_api import Error
        frame, review, sign_in = MagicMock(), MagicMock(), MagicMock()
        review.is_visible = AsyncMock(side_effect=Error("strict mode violation"))
        review.first.is_visible = AsyncMock(return_value=True)
        sign_in.first.is_visible = AsyncMock(return_value=False)
        frame.locator.side_effect = [review, sign_in]
        page = MagicMock()
        page.frames = [frame]
        with (patch.object(FLOW, "workbook_open_error", new=AsyncMock(return_value=False)),
              patch.object(FLOW, "raise_if_workbook_blocked", new=AsyncMock())):
            self.assertIs(await FLOW.find_editor(page), frame)
        review.first.is_visible.assert_awaited_once()
        review.is_visible.assert_not_awaited()


    async def test_detached_editor_does_not_reject_a_ready_live_editor(self):
        from playwright.async_api import Error
        old, live = MagicMock(), MagicMock()
        old.is_detached.return_value = True
        for frame in (old, live):
            review, sign_in = MagicMock(), MagicMock()
            review.first.is_visible = AsyncMock(
                side_effect=Error("Frame was detached") if frame is old else None,
                return_value=True)
            sign_in.first.is_visible = AsyncMock(return_value=False)
            frame.locator.side_effect = [review, sign_in]
        page = MagicMock()
        page.frames = [old, live]
        with (patch.object(FLOW, "workbook_open_error", new=AsyncMock(return_value=False)),
              patch.object(FLOW, "raise_if_workbook_blocked", new=AsyncMock())):
            self.assertIs(await FLOW.find_editor(page), live)


class RenderingRecoveryTests(unittest.IsolatedAsyncioTestCase):
    async def test_rendering_failure_after_skip_moves_file_once(self):
        """A finished skip follows the common relocation path after folder recovery."""
        from argparse import Namespace
        with tempfile.TemporaryDirectory() as temporary:
            args = Namespace(folder_url="https://1drv.ms/f/test", output_root=Path(temporary),
                             profile_dir=None, headed=True, refresh_list=False, list_only=False,
                             only_file=None, retry_skipped=False, max_files=1)
            browser = (object(), object(), MagicMock())

            async def skip_then_render_error(*values):
                progress, name = values[5], values[7]
                attempt = progress.start_attempt(name)
                progress.skip(name, attempt, "workbook_repair_required")
                raise FLOW.OneDriveRenderingError("Folder rendering failed after skip")

            with (patch.object(FLOW, "restart_folder_browser", new=AsyncMock(return_value=browser)),
                  patch.object(FLOW, "discover_workbooks", new=AsyncMock(return_value=["sample.xlsx"])),
                  patch.object(FLOW, "next_workbook_with_recovery", new=AsyncMock(
                      return_value=(browser[2], ("sample.xlsx", None)))),
                  patch.object(FLOW, "process_selected_workbook", new=skip_then_render_error),
                  patch.object(FLOW, "archive_skipped_with_recovery", new=AsyncMock(
                      return_value=browser)) as archive,
                  patch.object(FLOW, "close_browser", new=AsyncMock()),
                  patch.object(FLOW, "ConsoleProgress"), patch("builtins.print")):
                self.assertEqual(await FLOW.run_folder_check_performance(args), 0)
            self.assertEqual(archive.await_count, 2)
            self.assertEqual(archive.await_args_list[0].args[-1], [])
            self.assertEqual(archive.await_args_list[1].args[-1], ["sample.xlsx"])

    async def test_rendering_failure_is_detected_before_folder_timeout(self):
        for text in ("Unbekannter Renderingfehler.", "Unknown rendering error."):
            page = MagicMock()
            page.url = "https://onedrive.live.com/my"
            page.locator.return_value.first.is_visible = AsyncMock(return_value=False)
            page.get_by_text.return_value.first.is_visible = AsyncMock(return_value=True)
            with self.assertRaises(FLOW.OneDriveRenderingError):
                await FLOW.wait_for_folder_response(page)
            self.assertRegex(text, page.get_by_text.call_args.args[0])

    async def test_rendering_restart_preserves_pending_workbook(self):
        from argparse import Namespace
        with tempfile.TemporaryDirectory() as temporary:
            args = Namespace(folder_url="https://1drv.ms/f/test", output_root=Path(temporary),
                             profile_dir=None, headed=True, refresh_list=False, list_only=False,
                             only_file=None, retry_skipped=False, max_files=1)
            browser = (object(), object(), MagicMock())
            calls = 0
            async def process(*values):
                nonlocal calls
                calls += 1
                progress, name = values[5], values[7]
                attempt = progress.start_attempt(name)
                if calls == 1:
                    progress.fail(name, attempt, "OneDrive rendering failed")
                    raise FLOW.OneDriveRenderingError("OneDrive rendering failed")
                progress.transition(name, JobState.CHECKING)
                progress.finish(name, attempt, "already_clean")
                return (*browser, False)
            with (patch.object(FLOW, "restart_folder_browser", new=AsyncMock(return_value=browser)) as restart,
                  patch.object(FLOW, "discover_workbooks", new=AsyncMock(return_value=["sample.xlsx"])),
                  patch.object(FLOW, "next_workbook_with_recovery", new=AsyncMock(
                      return_value=(browser[2], ("sample.xlsx", None)))),
                  patch.object(FLOW, "process_selected_workbook", new=process),
                  patch.object(FLOW, "close_browser", new=AsyncMock()),
                  patch.object(FLOW, "ConsoleProgress"), patch("builtins.print")):
                self.assertEqual(await FLOW.run_folder_check_performance(args), 0)
            self.assertEqual(restart.await_count, 2)
            with ProgressStore(Path(temporary) / FLOW.folder_id(args.folder_url),
                               FLOW.folder_id(args.folder_url)) as progress:
                self.assertEqual(progress.status("sample.xlsx"), JobState.COMPLETED.value)
                self.assertEqual(progress.skipped_count(), 0)

    async def test_detached_notice_frame_does_not_abort_live_frame_checks(self):
        from playwright.async_api import Error
        detached, live = MagicMock(), MagicMock()
        detached.get_by_role.return_value.filter.return_value.first.is_visible = AsyncMock(
            side_effect=Error("Frame was detached"))
        detached.is_detached.return_value = True
        live.get_by_role.return_value.filter.return_value.first.is_visible = AsyncMock(return_value=False)
        live.locator.return_value.get_by_text.return_value.first.is_visible = AsyncMock(return_value=False)
        page = MagicMock()
        page.frames = [detached, live]
        await FLOW.raise_if_workbook_blocked(page)
        live.get_by_role.assert_called()
        detached.is_detached.return_value = False
        with self.assertRaises(Error):
            await FLOW.raise_if_workbook_blocked(page)

    async def test_modified_sort_confirmation_uses_the_same_locator(self):
        page, header = MagicMock(), MagicMock()
        header.wait_for = AsyncMock()
        header.get_attribute = AsyncMock(return_value="none")
        header.locator.return_value.click = AsyncMock()
        menu = MagicMock()
        menu.click = AsyncMock()
        page.locator.side_effect = [header, menu]
        expectation = MagicMock()
        expectation.to_have_attribute = AsyncMock()
        with patch("playwright.async_api.expect", return_value=expectation) as expect:
            await FLOW.sort_oldest_first(page)
        expect.assert_called_once_with(header)
        expectation.to_have_attribute.assert_awaited_once_with("aria-sort", "ascending", timeout=30_000)
        page.wait_for_function.assert_not_called()


class ProgressStoreTests(unittest.TestCase):
    def test_interrupted_optimization_and_save_restart_through_transition_table(self):
        """Restarting either phase closes its attempt before returning to OPENING."""
        for phase in (JobState.OPTIMIZING, JobState.SAVING):
            with self.subTest(phase=phase), tempfile.TemporaryDirectory() as temporary:
                with ProgressStore(Path(temporary), "folder") as progress:
                    progress.discover(["interrupted.xlsx"])
                    previous = progress.start_attempt("interrupted.xlsx")
                    progress.transition("interrupted.xlsx", JobState.CHECKING)
                    progress.transition("interrupted.xlsx", JobState.OPTIMIZING)
                    if phase == JobState.SAVING:
                        progress.transition("interrupted.xlsx", JobState.SAVING)
                    self.assertEqual(FLOW.validated_state(phase, JobState.OPENING), "opening")
                    current = progress.start_attempt("interrupted.xlsx")
                    self.assertGreater(current, previous)
                    self.assertEqual(progress.status("interrupted.xlsx"), "opening")
                    old_attempt = progress.connection.execute(
                        "SELECT finished_at, error FROM attempts WHERE id = ?", (previous,)).fetchone()
                    self.assertIsNotNone(old_attempt[0])
                    self.assertEqual(old_attempt[1], "Interrupted before restart")

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

    def get_by_text(self, _name, exact: bool = False):
        page = self

        class FakeRow:
            @property
            def first(self):
                return self

            async def count(self):
                return int(page.position == page.target)

            async def is_visible(self):
                return False

        return FakeRow()


async def fake_scroll(page: FakePage, to_top: bool):
    if to_top:
        page.position = 0
        page.resets += 1
    else:
        page.position = min(page.position + 1, 5)
    return page.position, 5


class RowCursorTests(unittest.IsolatedAsyncioTestCase):
    async def test_blocked_workbooks_preserve_notice_and_skip_optimization(self) -> None:
        """Known browser restrictions finish once with their actual notice."""
        cases = [
            (FLOW.WorkbookRepairRequired, "workbook_repair_required"),
            (FLOW.WorkbookEditRestricted, "irm_browser_edit_unavailable"),
        ]
        for exception, outcome in cases:
            with self.subTest(outcome=outcome), tempfile.TemporaryDirectory() as temporary:
                with ProgressStore(Path(temporary), "folder") as progress:
                    progress.discover(["blocked.xlsx"])
                    folder = MagicMock()
                    folder.url = "https://example.test/folder"
                    folder.is_closed.return_value = True
                    notice = "The actual Excel notice"
                    with (patch.object(FLOW, "open_editor_with_retry", new_callable=AsyncMock,
                                       side_effect=exception(notice)),
                          patch.object(FLOW, "optimize", new_callable=AsyncMock) as optimize):
                        await FLOW.process_workbook(
                            None, folder, Path(temporary), progress, MagicMock(), "blocked.xlsx")
                    self.assertEqual(progress.status("blocked.xlsx"), JobState.SKIPPED.value)
                    row = progress.connection.execute(
                        "SELECT outcome, error, finished_at FROM files JOIN attempts "
                        "ON files.name = attempts.file_name").fetchone()
                    self.assertEqual(tuple(row[:2]), (outcome, notice))
                    self.assertIsNotNone(row[2])
                    phases = progress.connection.execute("SELECT phase FROM events").fetchall()
                    self.assertIn(outcome, [row[0] for row in phases])
                    optimize.assert_not_awaited()

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
