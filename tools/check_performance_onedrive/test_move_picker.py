"""Exercise the actual move-picker column and generated sort-command selectors."""

import json
import unittest
from unittest.mock import AsyncMock, MagicMock, patch

from .move_picker import open_destination, scan_destination


class MovePickerTests(unittest.IsolatedAsyncioTestCase):
    async def test_scroll_and_snapshot_use_same_container_with_overflowing_inner_rows(self):
        from playwright.async_api import async_playwright
        from .runner import scroll_file_list
        from .visible_batches import snapshot

        async with async_playwright() as playwright:
            browser = await playwright.chromium.launch(channel="msedge", headless=True)
            try:
                page = await browser.new_page()
                await page.set_content('''
                    <div id="list" style="height:150px;overflow-y:auto">
                      <div style="height:900px">
                        <div style="height:20px;overflow:visible">
                          <div style="height:80px" data-automationid="field-LinkFilename">folder</div>
                        </div>
                      </div>
                    </div>
                ''')
                before = await snapshot(page)
                position, _ = await scroll_file_list(page, to_top=False)
                after = await snapshot(page)
                self.assertGreater(position, 0)
                self.assertEqual(after["scroll_top"], position)
                self.assertNotEqual(before["scroll_top"], after["scroll_top"])
            finally:
                await browser.close()

    async def test_fractional_scroll_end_does_not_loop_indefinitely(self):
        picker, pane, target = MagicMock(), MagicMock(), MagicMock()
        picker.locator.return_value.first = pane
        pane.wait_for = AsyncMock()
        pane.evaluate = AsyncMock(return_value=(100.5, 101))
        target.count = AsyncMock(return_value=0)
        with patch("tools.check_performance_onedrive.move_picker.asyncio.sleep", new=AsyncMock()):
            self.assertFalse(await scan_destination(picker, target))
        self.assertEqual(pane.evaluate.await_count, 9)

    async def test_existing_destination_keeps_current_sort(self):
        picker = MagicMock()
        picker.get_by_role.return_value.dblclick = AsyncMock()
        await open_destination(picker, "optimized")
        picker.locator.assert_not_called()

    async def test_missing_destination_remains_an_error_after_both_directions(self):
        from playwright.async_api import TimeoutError

        picker = MagicMock()
        picker.get_by_role.return_value.dblclick = AsyncMock(side_effect=TimeoutError("Missing folder"))
        with (patch("tools.check_performance_onedrive.move_picker.sort_modified", new=AsyncMock()) as sort,
              patch("tools.check_performance_onedrive.move_picker.scan_destination", new=AsyncMock(return_value=False))):
            with self.assertRaisesRegex(RuntimeError, "not found"):
                await open_destination(picker, "skipped")
        self.assertEqual([call.args[1] for call in sort.await_args_list], ["descending", "ascending"])
        self.assertEqual(picker.get_by_role.return_value.dblclick.await_count, 3)

    async def test_real_iframe_reveals_both_destination_names_using_modified_sort(self):
        from playwright.async_api import async_playwright
        import html

        async with async_playwright() as playwright:
            browser = await playwright.chromium.launch(channel="msedge", headless=True)
            try:
                page = await browser.new_page()
                for destination, reveal_on in (("optimized", "ascending"), ("skipped", "descending"),
                                               ("optimized", "virtualized")):
                    with self.subTest(destination=destination):
                        content = '''
                          <div role="columnheader" data-item-key="dateModifiedColumn_794" aria-sort="none">
                            <button id="modified">Modified</button>
                          </div>
                          <button data-automationid="sortCommand" id="sort">Sort</button>
                          <div class="ms-Spinner" hidden>Loading</div>
                          <div id="menu" hidden>
                            <button role="menuitemcheckbox" data-automationid="sortdateModifiedColumn_794" aria-checked="false" id="field">Modified</button>
                            <button role="menuitemcheckbox" data-automationid="sortAsc" id="asc">Oldest</button>
                            <button role="menuitemcheckbox" data-automationid="sortDesc" id="desc">Newest</button>
                          </div>
                          <div class="od-FilePicker-scrollPane" style="height:100px;overflow:auto">
                            <div style="height:500px"></div>
                            <a role="link" id="destination" hidden>DESTINATION</a>
                          </div>
                          <script>
                            const revealOn = REVEAL_ON;
                            document.querySelector('#sort').onclick = () => {
                              document.querySelector('#menu').hidden = false;
                            };
                            document.querySelector('#field').onclick = event => {
                              event.target.setAttribute('aria-checked', 'true');
                              document.querySelector('#menu').hidden = true;
                            };
                            for (const [id, direction] of [['asc','ascending'],['desc','descending']]) {
                              document.querySelector('#'+id).onclick = () => {
                                document.querySelector('[role=columnheader]').setAttribute('aria-sort', direction);
                                document.querySelector('#menu').hidden = true;
                                document.querySelector('#destination').hidden = true;
                                document.querySelector('.ms-Spinner').hidden = false;
                                setTimeout(() => {
                                  document.querySelector('.ms-Spinner').hidden = true;
                                  document.querySelector('#destination').hidden = direction !== revealOn;
                                }, 100);
                              };
                            }
                            document.querySelector('#destination').ondblclick = () => {
                              window.openedDestination = document.querySelector('#destination').textContent;
                            };
                            document.querySelector('.od-FilePicker-scrollPane').onscroll = event => {
                              if (revealOn === 'virtualized' && event.target.scrollTop > 0) {
                                document.querySelector('#destination').hidden = false;
                              }
                            };
                          </script>
                        '''.replace('DESTINATION', destination).replace('REVEAL_ON', json.dumps(reveal_on))
                        await page.set_content('<iframe title="Move to" srcdoc="' + html.escape(content, quote=True) + '"></iframe>')
                        picker = page.frame_locator('iframe[title="Move to"]')
                        await open_destination(picker, destination)
                        self.assertEqual(await page.frames[1].evaluate('window.openedDestination'), destination)
            finally:
                await browser.close()
