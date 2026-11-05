"""Continue folder cleanup while preserving observations of missing workbooks."""

from argparse import Namespace
from pathlib import Path
import tempfile
import unittest
from unittest.mock import AsyncMock, MagicMock, patch

from tools.check_performance_onedrive import runner as FLOW


class MissingFileTests(unittest.IsolatedAsyncioTestCase):
    def test_missing_history_survives_restart_and_resolves_on_rediscovery(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            with FLOW.ProgressStore(directory, "folder") as progress:
                progress.discover(["pending.xlsx", "done.xlsx", "skipped.xlsx"])
                attempt = progress.start_attempt("done.xlsx")
                progress.transition("done.xlsx", FLOW.JobState.CHECKING)
                progress.finish("done.xlsx", attempt, "already_clean")
                attempt = progress.start_attempt("skipped.xlsx")
                progress.skip("skipped.xlsx", attempt, "performance_unavailable")
                for name in progress.names():
                    progress.record_missing_file(name, "move", "Absent after folder search")
            with FLOW.ProgressStore(directory, "folder") as progress:
                self.assertEqual(progress.unfinished_names(True), set())
                self.assertEqual(progress.pending_skipped_moves(), [])
                self.assertEqual(progress.pending_optimized_moves(), set())
                self.assertEqual(progress.outcome("done.xlsx"), "already_clean")
                self.assertEqual(progress.outcome("skipped.xlsx"), "performance_unavailable")
                progress.discover(["pending.xlsx"])
                self.assertEqual(progress.unfinished_names(True), {"pending.xlsx"})
                rows = progress.connection.execute(
                    "SELECT file_name, observed_at, operation, detail, resolved_at FROM missing_files"
                ).fetchall()
                self.assertEqual(len(rows), 3)
                self.assertTrue(all(row[1] and row[2] == "move" and row[3] for row in rows))
                self.assertEqual([row[0] for row in rows if row[4]], ["pending.xlsx"])

    async def test_missing_optimized_file_does_not_block_next_move(self):
        with tempfile.TemporaryDirectory() as temporary:
            with FLOW.ProgressStore(Path(temporary), "folder") as progress:
                names = ["absent.xlsx", "present.xlsx"]
                progress.discover(names)
                for name in names:
                    attempt = progress.start_attempt(name)
                    progress.transition(name, FLOW.JobState.CHECKING)
                    progress.finish(name, attempt, "already_clean")

                async def select_next(_page, pending):
                    return sorted(pending)[0], None

                with (patch.object(FLOW, "next_visible_unfinished", new=select_next),
                      patch.object(FLOW, "move_workbook_to_folder", new=AsyncMock(
                          side_effect=[FLOW.WorkbookNotFound("Missing from source and destination"), None])) as move,
                      patch.object(FLOW, "restart_folder_browser", new=AsyncMock()) as restart):
                    await FLOW.archive_optimized_with_recovery(
                        None, None, None, None, Path(temporary), progress, names)
                self.assertEqual(move.await_count, 2)
                restart.assert_not_awaited()
                self.assertEqual(progress.pending_optimized_moves(), set())
                self.assertEqual(progress.status("absent.xlsx"), "completed")
                self.assertEqual(progress.connection.execute(
                    "SELECT file_name FROM optimized_moves").fetchall(), [("present.xlsx",)])
                self.assertEqual(progress.connection.execute(
                    "SELECT file_name, operation FROM missing_files").fetchall(),
                    [("absent.xlsx", "move_to_optimized")])

    async def test_missing_skipped_file_does_not_block_next_move(self):
        with tempfile.TemporaryDirectory() as temporary:
            with FLOW.ProgressStore(Path(temporary), "folder") as progress:
                names = ["absent.xlsx", "present.xlsx"]
                progress.discover(names)
                for name in names:
                    attempt = progress.start_attempt(name)
                    progress.skip(name, attempt, "performance_unavailable")
                with (patch.object(FLOW, "move_skipped_workbook", new=AsyncMock(
                          side_effect=[FLOW.WorkbookNotFound("Missing"), None])) as move,
                      patch.object(FLOW, "restart_folder_browser", new=AsyncMock()) as restart):
                    await FLOW.archive_skipped_with_recovery(
                        None, None, None, None, Path(temporary), progress, names)
                self.assertEqual(move.await_count, 2)
                restart.assert_not_awaited()
                self.assertEqual(progress.pending_skipped_moves(), [])
                self.assertEqual(progress.moved_skipped_names(), {"present.xlsx"})
                self.assertEqual(progress.status("absent.xlsx"), "skipped")

    async def test_missing_open_closes_attempt_without_retrying_or_skipping(self):
        with tempfile.TemporaryDirectory() as temporary:
            with FLOW.ProgressStore(Path(temporary), "folder") as progress:
                progress.discover(["absent.xlsx", "next.xlsx"])
                page = MagicMock()
                page.url = "https://example.test/folder"
                page.is_closed.return_value = True
                with (patch.object(FLOW, "open_editor_with_retry", new=AsyncMock(
                          side_effect=FLOW.WorkbookNotFound("Missing from readable source"))) as open_editor,
                      patch.object(FLOW, "restart_folder_browser", new=AsyncMock()) as restart):
                    result = await FLOW.process_selected_workbook(
                        None, None, page, None, Path(temporary), progress,
                        MagicMock(), "absent.xlsx", None)
                self.assertFalse(result[-1])
                open_editor.assert_awaited_once()
                restart.assert_not_awaited()
                self.assertEqual(progress.unfinished_names(True), {"next.xlsx"})
                self.assertEqual(progress.status("absent.xlsx"), "failed")
                self.assertIsNotNone(progress.connection.execute(
                    "SELECT finished_at FROM attempts").fetchone()[0])

    async def test_unreadable_folder_is_not_reported_as_a_missing_file(self):
        page = MagicMock()
        page.get_by_text.return_value.first.count = AsyncMock(return_value=0)
        with (patch.object(FLOW, "scroll_file_list", new=AsyncMock(return_value=(0, 0))),
              patch.object(FLOW, "raise_if_folder_render_failed", new=AsyncMock()),
              patch.object(FLOW.asyncio, "sleep", new=AsyncMock()),
              patch.object(FLOW, "wait_for_folder_rows", new=AsyncMock(
                  side_effect=RuntimeError("Microsoft sign-in required")))):
            with self.assertRaisesRegex(RuntimeError, "Microsoft sign-in required"):
                await FLOW.find_file_row(page, "absent.xlsx")

    async def test_readable_folder_reports_missing_after_complete_search(self):
        page = MagicMock()
        page.get_by_text.return_value.first.count = AsyncMock(return_value=0)
        with (patch.object(FLOW, "scroll_file_list", new=AsyncMock(return_value=(0, 0))),
              patch.object(FLOW, "raise_if_folder_render_failed", new=AsyncMock()),
              patch.object(FLOW.asyncio, "sleep", new=AsyncMock()),
              patch.object(FLOW, "wait_for_folder_rows", new=AsyncMock()) as readable):
            with self.assertRaises(FLOW.WorkbookNotFound):
                await FLOW.find_file_row(page, "absent.xlsx")
        readable.assert_awaited_once()

    async def test_missing_move_restores_parent_before_reporting_absence(self):
        page = MagicMock()
        page.url = "https://onedrive.live.com/my?folder=parent"
        page.goto = AsyncMock()
        page.wait_for_url = AsyncMock()
        page.locator.return_value.get_by_text.return_value.dblclick = AsyncMock()
        with (patch.object(FLOW, "find_file_row", new=AsyncMock(
                  side_effect=[None, MagicMock(dblclick=AsyncMock()), None])),
              patch.object(FLOW, "scroll_file_list", new=AsyncMock()),
              patch.object(FLOW, "wait_for_folder_rows", new=AsyncMock())):
            with self.assertRaises(FLOW.WorkbookNotFound):
                await FLOW.move_workbook_to_folder(page, "absent.xlsx", "optimized")
        page.goto.assert_awaited_once_with(page.url, wait_until="domcontentloaded", timeout=120_000)

    async def test_refresh_confirms_missing_selection_and_continues_present_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            args = Namespace(folder_url="https://1drv.ms/f/test", output_root=Path(temporary),
                             profile_dir=None, headed=True, refresh_list=False, list_only=False,
                             only_file=None, retry_skipped=False, max_files=10)
            identifier = FLOW.folder_id(args.folder_url)
            directory = Path(temporary) / identifier
            with FLOW.ProgressStore(directory, identifier) as progress:
                progress.discover(["absent.xlsx", "present.xlsx"])
            browser = (None, None, MagicMock())

            async def complete(*values):
                progress, name = values[5], values[7]
                self.assertEqual(name, "present.xlsx")
                attempt = progress.start_attempt(name)
                progress.transition(name, FLOW.JobState.CHECKING)
                progress.finish(name, attempt, "already_clean")
                return (*browser, False)

            with (patch.object(FLOW, "restart_folder_browser", new=AsyncMock(return_value=browser)),
                  patch.object(FLOW, "next_workbook_with_recovery", new=AsyncMock(
                      return_value=(browser[2], None))),
                  patch.object(FLOW, "recover_folder", new=AsyncMock(return_value=browser[2])),
                  patch.object(FLOW, "discover_workbooks", new=AsyncMock(return_value=["present.xlsx"])),
                  patch.object(FLOW, "next_visible_unfinished", new=AsyncMock(
                      return_value=("present.xlsx", None))),
                  patch.object(FLOW, "process_selected_workbook", new=complete),
                  patch.object(FLOW, "archive_skipped_with_recovery", new=AsyncMock(return_value=browser)),
                  patch.object(FLOW, "close_browser", new=AsyncMock()),
                  patch.object(FLOW, "ConsoleProgress")):
                self.assertEqual(await FLOW.run_folder_check_performance(args), 0)
            with FLOW.ProgressStore(directory, identifier) as progress:
                self.assertEqual(progress.unfinished_names(True), set())
                self.assertEqual(progress.status("absent.xlsx"), "pending")
                self.assertEqual(progress.status("present.xlsx"), "completed")
                self.assertEqual(progress.connection.execute(
                    "SELECT operation FROM missing_files").fetchone()[0], "select_workbook")


if __name__ == "__main__":
    unittest.main()
