"""Keep successful outcomes durable while moving their OneDrive copies."""

from argparse import Namespace
from pathlib import Path
import tempfile
import unittest
from unittest.mock import AsyncMock, MagicMock, patch

from tools.check_performance_onedrive import runner as FLOW


def complete(progress, name):
    attempt = progress.start_attempt(name)
    progress.transition(name, FLOW.JobState.CHECKING)
    progress.finish(name, attempt, "already_clean")


class OptimizedMoveTests(unittest.IsolatedAsyncioTestCase):
    async def test_browser_move_selects_optimized_and_waits_for_source_removal(self):
        from playwright.async_api import async_playwright
        import html

        async with async_playwright() as playwright:
            browser = await playwright.chromium.launch(headless=True)
            try:
                page = await browser.new_page()
                picker = '''
                    <button role="link">skipped</button>
                    <button role="link" id="optimized">optimized</button>
                    <button id="move" hidden>Move here</button>
                    <script>
                      document.querySelector('#optimized').ondblclick = () => {
                        document.querySelector('#move').hidden = false;
                      };
                      document.querySelector('#move').onclick = () => {
                        parent.document.querySelector('#file').remove();
                        parent.document.querySelector('#notice').textContent = 'Moved to optimized';
                        parent.document.querySelector('iframe').remove();
                      };
                    </script>
                '''
                await page.set_content(f'''
                    <span id="file" data-automationid="field-LinkFilename">done.xlsx</span>
                    <button id="command" hidden data-automationid="moveCommand">Move to</button>
                    <iframe hidden title="Move to" srcdoc="{html.escape(picker, quote=True)}"></iframe>
                    <div id="notice" role="alert"></div>
                    <script>
                      document.querySelector('#file').oncontextmenu = event => {{
                        event.preventDefault();
                        document.querySelector('#command').hidden = false;
                      }};
                      document.querySelector('#command').onclick = () => {{
                        document.querySelector('iframe').hidden = false;
                      }};
                    </script>
                ''')
                await FLOW.move_workbook_to_folder(page, "done.xlsx", "optimized")
                self.assertEqual(await page.get_by_text("done.xlsx", exact=True).count(), 0)
                self.assertEqual(await page.get_by_role("alert").inner_text(), "Moved to optimized")
            finally:
                await browser.close()

    def test_move_confirmation_survives_restart_and_manual_restore(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            with FLOW.ProgressStore(directory, "folder") as progress:
                progress.discover(["done.xlsx", "pending.xlsx"])
                complete(progress, "done.xlsx")
                with self.assertRaises(ValueError):
                    progress.confirm_optimized_move("pending.xlsx")
                progress.confirm_optimized_move("done.xlsx")
            with FLOW.ProgressStore(directory, "folder") as progress:
                self.assertEqual(progress.pending_optimized_moves(), set())
                progress.discover(["done.xlsx"])
                self.assertEqual(progress.pending_optimized_moves(), {"done.xlsx"})
                self.assertEqual(progress.outcome("done.xlsx"), "already_clean")

    async def test_retry_preserves_confirmed_moves_and_successful_outcomes(self):
        with tempfile.TemporaryDirectory() as temporary:
            with FLOW.ProgressStore(Path(temporary), "folder") as progress:
                progress.discover(["first.xlsx", "second.xlsx", "pending.xlsx"])
                complete(progress, "first.xlsx")
                complete(progress, "second.xlsx")
                browser = (object(), object(), object())

                async def next_file(_page, pending):
                    return sorted(pending)[0], None

                with (patch.object(FLOW, "next_visible_unfinished", new=next_file),
                      patch.object(FLOW, "move_workbook_to_folder", new=AsyncMock(
                          side_effect=[None, RuntimeError("connection lost"), None])) as move,
                      patch.object(FLOW, "restart_folder_browser", new=AsyncMock(
                          return_value=browser)) as restart):
                    await FLOW.archive_optimized_with_recovery(
                        *browser, object(), Path(temporary), progress,
                        ["first.xlsx", "second.xlsx", "pending.xlsx"])
                self.assertEqual([call.args[1] for call in move.await_args_list],
                                 ["first.xlsx", "second.xlsx", "second.xlsx"])
                self.assertTrue(all(call.args[2] == "optimized" for call in move.await_args_list))
                restart.assert_awaited_once()
                self.assertEqual(progress.pending_optimized_moves(), set())
                self.assertEqual(progress.status("second.xlsx"), "completed")
                self.assertEqual(progress.outcome("second.xlsx"), "already_clean")

    async def test_run_moves_prior_and_new_completions_only_from_input_folder(self):
        with tempfile.TemporaryDirectory() as temporary:
            args = Namespace(folder_url="https://1drv.ms/f/test", output_root=Path(temporary),
                             profile_dir=None, headed=True, refresh_list=False, list_only=False,
                             only_file=None, retry_skipped=True, move_optimized=True, max_files=10)
            identifier = FLOW.folder_id(args.folder_url)
            directory = Path(temporary) / identifier
            with FLOW.ProgressStore(directory, identifier) as progress:
                progress.discover(["done.xlsx", "archived.xlsx", "new.xlsx"])
                complete(progress, "done.xlsx")
                complete(progress, "archived.xlsx")
            browser = (object(), object(), MagicMock())
            moved = []

            async def archive(*values):
                progress, names = values[5], values[6]
                moved.extend(names)
                for name in names:
                    progress.confirm_optimized_move(name)
                return browser

            async def process(*values):
                self.assertEqual(values[7], "new.xlsx")
                complete(values[5], values[7])
                return (*browser, False)

            with (patch.object(FLOW, "restart_folder_browser", new=AsyncMock(return_value=browser)),
                  patch.object(FLOW, "discover_workbooks", new=AsyncMock(
                      return_value=["done.xlsx", "new.xlsx"])),
                  patch.object(FLOW, "next_workbook_with_recovery", new=AsyncMock(
                      return_value=(browser[2], ("new.xlsx", None)))),
                  patch.object(FLOW, "process_selected_workbook", new=process),
                  patch.object(FLOW, "archive_optimized_with_recovery", new=archive),
                  patch.object(FLOW, "close_browser", new=AsyncMock()),
                  patch.object(FLOW, "ConsoleProgress")):
                self.assertEqual(await FLOW.run_folder_check_performance(args), 0)
            self.assertEqual(moved, ["done.xlsx", "new.xlsx"])
            with FLOW.ProgressStore(directory, identifier) as progress:
                self.assertEqual(progress.status("archived.xlsx"), "completed")
                self.assertEqual(progress.pending_optimized_moves(), {"archived.xlsx"})

    def test_cli_enables_completed_moves(self):
        with patch.object(FLOW.sys, "argv", ["runner", "https://1drv.ms/f/test", "--move-optimized"]):
            self.assertTrue(FLOW.parse_arguments().move_optimized)


if __name__ == "__main__":
    unittest.main()
