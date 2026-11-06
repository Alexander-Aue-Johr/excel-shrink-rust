"""Keep Excel's temporary service errors retryable rather than skipped."""

from pathlib import Path
import tempfile
import unittest
from unittest.mock import AsyncMock, MagicMock, patch

from tools.check_performance_onedrive import runner as flow


class ServiceErrorTests(unittest.IsolatedAsyncioTestCase):
    async def test_visible_service_notice_preserves_text(self):
        for text in (
            "Ihre Anforderung konnte leider nicht verarbeitet werden. Bitte wiederholen Sie diesen Vorgang in einigen Minuten.",
            "Sorry, we couldn't process your request. Please try again in a few minutes.",
        ):
            with self.subTest(text=text):
                page = MagicMock()
                frame = MagicMock()
                page.frames = [frame]
                notice = frame.get_by_text.return_value.first
                notice.is_visible = AsyncMock(return_value=True)
                notice.inner_text = AsyncMock(return_value=text)
                with self.assertRaises(flow.ExcelServiceUnavailable) as raised:
                    await flow.raise_if_workbook_blocked(page)
                self.assertEqual(str(raised.exception), text)
                pattern = frame.get_by_text.call_args.args[0]
                self.assertIsNotNone(pattern.search(text))

    async def test_service_failure_stays_retryable_and_bypasses_file_retry_budget(self):
        with tempfile.TemporaryDirectory() as temporary:
            with flow.ProgressStore(Path(temporary), "folder") as progress:
                progress.discover(["retry.xlsx"])
                page = MagicMock()
                page.url = "https://example.test/folder"
                page.is_closed.return_value = False
                notice = "Please try again in a few minutes."

                async def process(*args):
                    await flow.process_workbook(
                        None, page, Path(temporary), progress, MagicMock(), "retry.xlsx")

                with (patch.object(flow, "open_editor_with_retry", new=AsyncMock(
                        side_effect=flow.ExcelServiceUnavailable(notice))),
                      patch.object(flow, "process_with_live_status", new=process),
                      patch.object(progress, "skip_failed") as skip,
                      patch.object(flow, "restart_folder_browser", new=AsyncMock()) as restart):
                    with self.assertRaises(flow.ExcelServiceUnavailable):
                        await flow.process_selected_workbook(
                            None, None, page, None, Path(temporary), progress,
                            MagicMock(), "retry.xlsx", None)
                self.assertEqual(progress.status("retry.xlsx"), "failed")
                self.assertEqual(progress.unfinished_names(), {"retry.xlsx"})
                self.assertEqual(progress.connection.execute(
                    "SELECT error FROM attempts").fetchone()[0], notice)
                skip.assert_not_called()
                restart.assert_not_awaited()
