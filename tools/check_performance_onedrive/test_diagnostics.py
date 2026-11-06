"""Verify browser evidence survives partial failures and precedes recovery."""

import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import AsyncMock, MagicMock, patch

from .diagnostics import capture_failure
from . import runner as flow


class DiagnosticsTests(unittest.IsolatedAsyncioTestCase):
    async def test_failed_screenshot_and_detached_frame_keep_other_evidence(self):
        page = MagicMock()
        page.url = "https://example.test/workbook"
        page.screenshot = AsyncMock(side_effect=RuntimeError("Screenshot unavailable"))
        frame = MagicMock(url="https://example.test/editor")
        frame.content = AsyncMock(return_value="<html><body>Temporary error</body></html>")
        detached = MagicMock(url="https://example.test/detached")
        detached.content = AsyncMock(side_effect=RuntimeError("Frame detached"))
        page.frames = [frame, detached]
        page.context.new_cdp_session = AsyncMock(side_effect=RuntimeError("Disconnected"))
        with tempfile.TemporaryDirectory() as temporary:
            directory = await capture_failure(page, Path(temporary), "open", TimeoutError("Late"), "file.xlsx")
            metadata = json.loads((directory / "error.json").read_text(encoding="utf-8"))
            self.assertEqual(metadata["workbook"], "file.xlsx")
            self.assertIn("Temporary error", (directory / "frame-0.html").read_text())
            self.assertIn("Frame detached", metadata["frames"][1]["error"])
            self.assertTrue(metadata["capture_errors"])

    async def test_workbook_timeout_is_captured_before_returning_to_folder(self):
        events = []
        page = MagicMock(url="https://example.test/workbook")
        page.is_closed.return_value = False
        page.goto = AsyncMock(side_effect=lambda *args, **kwargs: events.append("navigate"))
        with tempfile.TemporaryDirectory() as temporary:
            with flow.ProgressStore(Path(temporary), "folder") as progress:
                progress.discover(["file.xlsx"])

                async def open_editor(*args):
                    page.url = "https://example.test/editor"
                    raise TimeoutError("Excel did not respond")

                with (patch.object(flow, "open_editor_with_retry", new=open_editor),
                      patch.object(flow, "capture_failure", new=AsyncMock(
                          side_effect=lambda *args: events.append("capture"))),
                      patch.object(flow, "wait_for_folder_rows", new=AsyncMock()),
                      patch.object(flow, "sort_oldest_first", new=AsyncMock())):
                    await flow.process_workbook(None, page, Path(temporary), progress,
                                                MagicMock(), "file.xlsx")
                self.assertEqual(events, ["capture", "navigate"])
                self.assertEqual(progress.status("file.xlsx"), "failed")

    async def test_real_browser_captures_screenshot_frames_and_shadow_dom(self):
        from playwright.async_api import async_playwright

        async with async_playwright() as playwright:
            browser = await playwright.chromium.launch(channel="msedge", headless=True)
            try:
                page = await browser.new_page()
                await page.set_content('<div id="host"></div><iframe srcdoc="<p>Frame notice</p>"></iframe>')
                await page.evaluate("""() => {
                    document.querySelector('#host').attachShadow({mode:'open'}).innerHTML =
                        '<p>Shadow notice</p>';
                }""")
                with tempfile.TemporaryDirectory() as temporary:
                    directory = await capture_failure(page, Path(temporary), "test", TimeoutError("Test"))
                    self.assertTrue((directory / "viewport.png").read_bytes().startswith(b"\x89PNG"))
                    self.assertIn("Frame notice", (directory / "frame-1.html").read_text())
                    self.assertIn("Shadow notice", (directory / "dom-snapshot.json").read_text())
                    metadata = json.loads((directory / "error.json").read_text())
                    self.assertEqual(metadata["capture_errors"], [])
            finally:
                await browser.close()
