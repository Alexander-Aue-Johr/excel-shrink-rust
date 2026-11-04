"""Keep skipped-file relocation resumable without changing analysis outcomes."""

from pathlib import Path
import html
import tempfile
import unittest
from unittest.mock import AsyncMock, MagicMock, patch

from tools.check_performance_onedrive import runner as FLOW


class SkippedMoveTests(unittest.IsolatedAsyncioTestCase):
    async def test_destination_lookup_failure_preserves_parent_navigation(self):
        page = MagicMock()
        page.url = "https://onedrive.live.com/my?folder=parent"
        page.goto = AsyncMock()
        with (patch.object(FLOW, "find_file_row", new=AsyncMock(
                side_effect=[None, RuntimeError("Missing from skipped")])) as find,
              patch.object(FLOW, "scroll_file_list", new=AsyncMock()),
              patch.object(FLOW, "wait_for_folder_rows", new=AsyncMock())):
            page.locator = MagicMock()
            page.locator.return_value.get_by_text.return_value.dblclick = AsyncMock()
            with self.assertRaisesRegex(RuntimeError, "Missing from skipped"):
                await FLOW.move_skipped_workbook(page, "excluded.xlsx")
        self.assertEqual(find.await_count, 2)
        page.goto.assert_awaited_once_with(page.url, wait_until="domcontentloaded", timeout=120_000)

    async def test_offscreen_source_with_same_named_destination_stays_pending(self):
        """An older destination cannot stand in for a source outside the viewport."""
        page = MagicMock()
        page.get_by_text.return_value.first.count = AsyncMock(return_value=0)
        source = MagicMock()
        source.click = AsyncMock()
        page.locator.return_value.last.click = AsyncMock()
        picker = page.frame_locator.return_value
        picker.get_by_role.return_value.dblclick = AsyncMock()
        picker.get_by_role.return_value.click = AsyncMock(side_effect=RuntimeError("Name conflict"))
        with tempfile.TemporaryDirectory() as temporary:
            with FLOW.ProgressStore(Path(temporary), "folder") as progress:
                progress.discover(["excluded.xlsx"])
                attempt = progress.start_attempt("excluded.xlsx")
                progress.skip("excluded.xlsx", attempt, "performance_unavailable")
                with patch.object(FLOW, "find_file_row", new=AsyncMock(return_value=source)) as find:
                    with self.assertRaisesRegex(RuntimeError, "Name conflict"):
                        await FLOW.archive_skipped_workbooks(page, progress, ["excluded.xlsx"])
                find.assert_awaited_once_with(page, "excluded.xlsx", missing_ok=True)
                source.click.assert_awaited_once_with(button="right", timeout=10_000)
                self.assertEqual(progress.pending_skipped_moves(), ["excluded.xlsx"])

    async def test_recovery_requires_source_absence_and_destination_presence(self):
        """A completed move can resume after the process missed the confirmation."""
        page = MagicMock()
        page.url = "https://onedrive.live.com/my?folder=parent"
        page.goto = AsyncMock()
        page.locator.return_value.get_by_text.return_value.dblclick = AsyncMock()
        with (patch.object(FLOW, "find_file_row", new=AsyncMock(side_effect=[None, object()])) as find,
              patch.object(FLOW, "scroll_file_list", new=AsyncMock()),
              patch.object(FLOW, "wait_for_folder_rows", new=AsyncMock())):
            await FLOW.move_skipped_workbook(page, "excluded.xlsx")
        self.assertEqual(find.await_count, 2)
        page.frame_locator.assert_not_called()
        page.goto.assert_awaited_once_with(page.url, wait_until="domcontentloaded", timeout=120_000)

    async def test_failed_move_remains_pending_after_reopen(self):
        """A browser failure must preserve both the skip reason and relocation work."""
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            with FLOW.ProgressStore(directory, "folder") as progress:
                progress.discover(["excluded.xlsx", "completed.xlsx"])
                attempt = progress.start_attempt("excluded.xlsx")
                progress.skip("excluded.xlsx", attempt, "workbook_repair_required", "Repair needed")
                with patch.object(FLOW, "move_skipped_workbook", new=AsyncMock(side_effect=RuntimeError("offline"))):
                    with self.assertRaises(RuntimeError):
                        await FLOW.archive_skipped_workbooks(None, progress, ["excluded.xlsx"])
                self.assertEqual(progress.pending_skipped_moves(), ["excluded.xlsx"])
            with FLOW.ProgressStore(directory, "folder") as progress:
                with patch.object(FLOW, "move_skipped_workbook", new=AsyncMock()) as move:
                    await FLOW.archive_skipped_workbooks(None, progress, progress.pending_skipped_moves())
                move.assert_awaited_once_with(None, "excluded.xlsx")
                self.assertEqual(progress.pending_skipped_moves(), [])
                self.assertEqual(progress.outcome("excluded.xlsx"), "workbook_repair_required")
                self.assertEqual(progress.status("completed.xlsx"), "pending")

    async def test_refresh_recognizes_manually_restored_workbook(self):
        """An explicit refresh lets a restored file use the normal retry path."""
        with tempfile.TemporaryDirectory() as temporary:
            with FLOW.ProgressStore(Path(temporary), "folder") as progress:
                progress.discover(["excluded.xlsx"])
                attempt = progress.start_attempt("excluded.xlsx")
                progress.skip("excluded.xlsx", attempt, "performance_unavailable")
                progress.confirm_skipped_move("excluded.xlsx")
                progress.discover(["excluded.xlsx"])
                self.assertEqual(progress.pending_skipped_moves(), ["excluded.xlsx"])
                progress.start_attempt("excluded.xlsx")
                self.assertEqual(progress.pending_skipped_moves(), [])

    async def test_completed_files_cannot_be_marked_as_moved(self):
        with tempfile.TemporaryDirectory() as temporary:
            with FLOW.ProgressStore(Path(temporary), "folder") as progress:
                progress.discover(["done.xlsx"])
                attempt = progress.start_attempt("done.xlsx")
                progress.transition("done.xlsx", FLOW.JobState.CHECKING)
                progress.finish("done.xlsx", attempt, "already_clean")
                with self.assertRaises(ValueError):
                    progress.confirm_skipped_move("done.xlsx")


class SkippedMoveBrowserTests(unittest.IsolatedAsyncioTestCase):
    async def test_move_uses_picker_frame_and_never_opens_workbook(self):
        """The real move picker is an iframe and filename clicks open Excel."""
        from playwright.async_api import async_playwright
        async with async_playwright() as playwright:
            browser = await playwright.chromium.launch(headless=True)
            try:
                page = await browser.new_page()
                picker = """
                    <button role="link" id="skipped">skipped</button>
                    <button id="move" hidden>Hierher verschieben</button>
                    <script>
                        document.querySelector('#skipped').ondblclick = () => {
                            document.querySelector('#move').hidden = false;
                        };
                        document.querySelector('#move').onclick = () => {
                            parent.document.querySelector('#file').remove();
                            parent.document.querySelector('#notice').textContent =
                                '1 Element nach skipped verschoben';
                            parent.document.querySelector('iframe').remove();
                        };
                    </script>
                """
                await page.set_content(f"""
                  <span id="file">excluded.xlsx</span>
                  <button id="command" hidden data-automationid="moveCommand">Verschieben nach</button>
                  <iframe hidden title="Verschieben nach" srcdoc="{html.escape(picker, quote=True)}"></iframe>
                  <div id="notice" role="alert"></div>
                  <script>
                      const file = document.querySelector('#file');
                      file.onclick = () => {{ window.workbookOpened = true; }};
                      file.oncontextmenu = (event) => {{
                          event.preventDefault();
                          document.querySelector('#command').hidden = false;
                      }};
                      document.querySelector('#command').onclick = () => {{
                          document.querySelector('iframe').hidden = false;
                      }};
                  </script>
                """)
                await FLOW.move_skipped_workbook(page, "excluded.xlsx")
                self.assertFalse(await page.evaluate("Boolean(window.workbookOpened)"))
                self.assertEqual(await page.get_by_text("excluded.xlsx", exact=True).count(), 0)
                self.assertIn("nach skipped verschoben", await page.get_by_role("alert").inner_text())
            finally:
                await browser.close()


if __name__ == "__main__":
    unittest.main()
