"""Exercise viewport-first cleanup and OneDrive's actual selection controls."""

from argparse import Namespace
import html
from pathlib import Path
import tempfile
import unittest
from unittest.mock import AsyncMock, MagicMock, patch

from tools.check_performance_onedrive import runner as flow
from tools.check_performance_onedrive import visible_batches as visible


def complete(progress, name):
    attempt = progress.start_attempt(name)
    progress.transition(name, flow.JobState.CHECKING)
    progress.finish(name, attempt, "already_clean")


class VisibleRunTests(unittest.IsolatedAsyncioTestCase):
    async def test_entry_drains_results_then_checks_cohort_then_bulk_moves_without_scanning(self):
        with tempfile.TemporaryDirectory() as temporary:
            args = Namespace(folder_url="https://1drv.ms/f/test", output_root=Path(temporary),
                             profile_dir=None, headed=True, refresh_list=False, list_only=False,
                             only_file=None, retry_skipped=True, move_optimized=True, max_files=10)
            identifier = flow.folder_id(args.folder_url)
            directory = args.output_root / identifier
            with flow.ProgressStore(directory, identifier) as progress:
                progress.discover(["done.xlsx", "retry.xlsx", "pending.xlsx", "archived.xlsx"])
                complete(progress, "done.xlsx")
                complete(progress, "archived.xlsx")
                attempt = progress.start_attempt("retry.xlsx")
                progress.skip("retry.xlsx", attempt, "error_after_retry")
            browser = (None, None, MagicMock())
            browser[2].locator.return_value.first.wait_for = AsyncMock()
            files = ["done.xlsx", "retry.xlsx", "pending.xlsx"]
            events = []

            async def view(_page):
                return {"files": list(files), "more_below": False, "scroll_top": 0}

            async def move(values, _args, _profile, progress, names, destination):
                events.append(("move", destination, list(names)))
                batch = progress.start_move_batch(names, destination)
                progress.finish_move_batch(batch)
                for name in names:
                    files.remove(name)
                return values

            async def process(*values, row_resolver=None):
                name = values[7]
                progress = values[5]
                await row_resolver(values[2], name)
                events.append(("check", name))
                if name == "retry.xlsx":
                    complete(progress, name)
                else:
                    attempt = progress.start_attempt(name)
                    progress.skip(name, attempt, "performance_unavailable")
                return (*browser, False)

            with (patch.object(flow, "restart_folder_browser", new=AsyncMock(return_value=browser)),
                  patch.object(flow, "wait_for_folder_rows", new=AsyncMock()),
                  patch.object(flow, "sort_oldest_first", new=AsyncMock()) as sort,
                  patch.object(flow, "discover_workbooks", side_effect=AssertionError("full discovery")),
                  patch.object(flow, "find_file_row", side_effect=AssertionError("full search")),
                  patch.object(flow, "scroll_file_list", side_effect=AssertionError("scrolling")),
                  patch.object(flow, "process_selected_workbook", new=process),
                  patch.object(flow, "close_browser", new=AsyncMock()),
                  patch.object(flow, "ConsoleProgress"),
                  patch.object(visible, "snapshot", new=view),
                  patch.object(visible, "move_with_recovery", new=move),
                  patch.object(visible.asyncio, "sleep", new=AsyncMock())):
                self.assertEqual(await flow.run_folder_check_performance(args), 0)
            self.assertEqual(events, [
                ("move", "optimized", ["done.xlsx"]),
                ("check", "retry.xlsx"), ("check", "pending.xlsx"),
                ("move", "optimized", ["retry.xlsx"]),
                ("move", "skipped", ["pending.xlsx"]),
            ])
            self.assertGreater(sort.await_count, 1)
            with flow.ProgressStore(directory, identifier) as progress:
                self.assertEqual(progress.status("archived.xlsx"), "completed")
                self.assertEqual(progress.connection.execute(
                    "SELECT COUNT(*) FROM attempts WHERE file_name = 'retry.xlsx'"
                ).fetchone()[0], 2)
                self.assertEqual(progress.connection.execute(
                    "SELECT COUNT(*) FROM move_batches WHERE confirmed_at IS NOT NULL"
                ).fetchone()[0], 3)

    async def test_retry_resolves_only_visible_row_after_browser_restart(self):
        with tempfile.TemporaryDirectory() as temporary:
            with flow.ProgressStore(Path(temporary), "folder") as progress:
                progress.discover(["retry.xlsx"])
                browser = (None, None, object())

                async def process(*_values):
                    attempt = progress.start_attempt("retry.xlsx")
                    count = progress.connection.execute("SELECT COUNT(*) FROM attempts").fetchone()[0]
                    if count == 1:
                        progress.fail("retry.xlsx", attempt, "old browser disconnected")
                    else:
                        progress.transition("retry.xlsx", flow.JobState.CHECKING)
                        progress.finish("retry.xlsx", attempt, "already_clean")

                resolve = AsyncMock(return_value="visible-row")
                with (patch.object(flow, "process_with_live_status", new=process),
                      patch.object(flow, "restart_folder_browser", new=AsyncMock(return_value=browser)),
                      patch.object(flow, "find_file_row", side_effect=AssertionError("full search"))):
                    result = await flow.process_selected_workbook(
                        *browser, object(), Path(temporary), progress, MagicMock(), "retry.xlsx", None,
                        row_resolver=resolve)
                self.assertFalse(result[-1])
                self.assertEqual(resolve.await_count, 2)

    async def test_repeated_selection_changes_are_bounded_and_reported(self):
        browser = (None, None, MagicMock())
        args = Namespace(folder_url="https://1drv.ms/f/test", headed=True)
        with (patch.object(visible, "snapshot", new=AsyncMock(return_value={"files": ["first.xlsx"]})),
              patch.object(visible, "move_visible_batch", new=AsyncMock(
                  side_effect=visible.ViewportChanged("source rows changed"))) as move,
              patch.object(flow, "recover_folder", new=AsyncMock(return_value=browser[2])) as recover):
            with self.assertRaisesRegex(RuntimeError, "selection remained unstable"):
                await visible.move_with_recovery(browser, args, Path("profile"), None,
                                                 ["first.xlsx"], "optimized")
        self.assertEqual(move.await_count, 2)
        recover.assert_awaited_once()

    async def test_max_files_flushes_the_checked_result_before_stopping(self):
        with tempfile.TemporaryDirectory() as temporary:
            args = Namespace(folder_url="https://1drv.ms/f/test", output_root=Path(temporary),
                             profile_dir=None, headed=True, refresh_list=False, list_only=False,
                             only_file=None, retry_skipped=True, move_optimized=True, max_files=1)
            browser = (None, None, MagicMock())
            browser[2].locator.return_value.first.wait_for = AsyncMock()
            files = ["first.xlsx", "next.xlsx"]
            moved = []
            calls = 0

            async def view(_page):
                return {"files": list(files), "more_below": False, "scroll_top": 0}

            async def process(*values, **_kwargs):
                nonlocal calls
                calls += 1
                if calls == 1:
                    raise RuntimeError("Browser disconnected before opening")
                complete(values[5], values[7])
                return (*browser, False)

            async def move(values, _args, _profile, progress, names, destination):
                moved.extend(names)
                batch = progress.start_move_batch(names, destination)
                progress.finish_move_batch(batch)
                for name in names:
                    files.remove(name)
                return values

            with (patch.object(flow, "restart_folder_browser", new=AsyncMock(return_value=browser)),
                  patch.object(flow, "wait_for_folder_rows", new=AsyncMock()),
                  patch.object(flow, "sort_oldest_first", new=AsyncMock()),
                  patch.object(flow, "process_selected_workbook", new=process),
                  patch.object(flow, "close_browser", new=AsyncMock()),
                  patch.object(flow, "ConsoleProgress"),
                  patch.object(visible, "snapshot", new=view),
                  patch.object(visible, "move_with_recovery", new=move)):
                self.assertEqual(await flow.run_folder_check_performance(args), 0)
            self.assertEqual(moved, ["first.xlsx"])
            self.assertEqual(files, ["next.xlsx"])
            self.assertEqual(calls, 2)

    def test_batch_checkpoint_survives_reopen_without_false_move_confirmation(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            with flow.ProgressStore(directory, "folder") as progress:
                progress.discover(["first.xlsx", "second.xlsx", "pending.xlsx"])
                complete(progress, "first.xlsx")
                complete(progress, "second.xlsx")
                with self.assertRaises(ValueError):
                    progress.start_move_batch(["pending.xlsx"], "optimized")
                batch = progress.start_move_batch(["first.xlsx", "second.xlsx"], "optimized")
                progress.fail_move_batch(batch, "Cloud confirmation was interrupted")
            with flow.ProgressStore(directory, "folder") as progress:
                self.assertEqual(progress.pending_optimized_moves(), {"first.xlsx", "second.xlsx"})
                progress.finish_move_batch(batch)
                self.assertEqual(progress.pending_optimized_moves(), set())
                self.assertEqual(progress.status("first.xlsx"), "completed")


class VisibleBrowserTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        from playwright.async_api import async_playwright
        self.driver = await async_playwright().start()
        self.browser = await self.driver.chromium.launch(headless=True)
        self.page = await self.browser.new_page(viewport={"width": 1000, "height": 600})

    async def asyncTearDown(self):
        await self.browser.close()
        await self.driver.stop()

    async def set_folder(self, partial=False):
        picker = '''
          <a role="link" id="destination">optimized</a>
          <button id="confirm" hidden>Move here</button>
          <script>
            document.querySelector('#destination').ondblclick = () => {
              const heading=document.createElement('h2'); heading.textContent='optimized';
              document.body.append(heading); document.querySelector('#confirm').hidden=false;
            };
            document.querySelector('#confirm').onclick = () => parent.finishMove();
          </script>
        '''
        rows = ''.join(f'''
          <div data-selection-index="{index}" aria-selected="false" class="row">
            <div data-automationid="row-selection-{index}" class="selection"
                 onclick="toggle(this.parentElement)">
              <input type="checkbox" style="opacity:0;pointer-events:none">
            </div>
            <div data-automationid="field-DocIcon"><img alt=".xlsx"></div>
            <div data-automationid="field-LinkFilename">{name}</div>
          </div>''' for index, name in enumerate(["first.xlsx", "second.xlsx", "unprocessed.xlsx", "offscreen.xlsx"]))
        await self.page.set_content(f'''
          <style>
            .list {{height:150px;overflow-y:auto;}} .row {{height:48px;display:flex;}}
            .selection {{width:40px;}} [data-automationid="field-LinkFilename"] {{width:600px;}}
          </style>
          <button data-automationid="moveCommand" hidden onclick="document.querySelector('iframe').hidden=false">Move to</button>
          <div class="list">{rows}</div>
          <iframe hidden title="Move to" srcdoc="{html.escape(picker,quote=True)}"></iframe>
          <div role="status" id="notice"></div>
          <script>
            window.contextMenus=0; document.oncontextmenu=()=>window.contextMenus++;
            function toggle(row) {{
              row.setAttribute('aria-selected',row.getAttribute('aria-selected')==='true'?'false':'true');
              document.querySelector('button').hidden=false;
            }}
            window.finishMove=()=>{{
              const selected=[...document.querySelectorAll('[data-selection-index][aria-selected="true"]')];
              selected.slice(0,{1 if partial else 100}).forEach(row=>row.remove());
              document.querySelector('#notice').textContent='Moved 2 items to optimized';
              document.querySelector('iframe').remove();
            }};
          </script>
        ''')

    async def test_snapshot_excludes_rendered_rows_below_viewport(self):
        await self.set_folder()
        view = await visible.snapshot(self.page)
        self.assertEqual(view["files"], ["first.xlsx", "second.xlsx", "unprocessed.xlsx"])
        self.assertTrue(view["more_below"])

    async def test_toolbar_batch_moves_exact_selection_and_reveals_new_rows(self):
        await self.set_folder()
        with tempfile.TemporaryDirectory() as temporary:
            with flow.ProgressStore(Path(temporary), "folder") as progress:
                progress.discover(["first.xlsx", "second.xlsx", "unprocessed.xlsx"])
                complete(progress, "first.xlsx")
                complete(progress, "second.xlsx")
                await visible.move_visible_batch(self.page, progress,
                                                 ["first.xlsx", "second.xlsx"], "optimized")
                self.assertEqual(progress.pending_optimized_moves(), set())
                self.assertEqual(progress.status("unprocessed.xlsx"), "pending")
                self.assertEqual(await self.page.evaluate("window.contextMenus"), 0)
                self.assertEqual((await visible.snapshot(self.page))["files"],
                                 ["unprocessed.xlsx", "offscreen.xlsx"])
                self.assertEqual(await self.page.locator('.list').evaluate('e=>e.scrollTop'), 0)

    async def test_partial_move_does_not_confirm_entire_batch(self):
        await self.set_folder(partial=True)
        original_wait = self.page.wait_for_function

        async def short_wait(script, **kwargs):
            return await original_wait(script, **{**kwargs, "timeout": 300})

        with tempfile.TemporaryDirectory() as temporary:
            with flow.ProgressStore(Path(temporary), "folder") as progress:
                progress.discover(["first.xlsx", "second.xlsx"])
                complete(progress, "first.xlsx")
                complete(progress, "second.xlsx")
                with patch.object(self.page, "wait_for_function", new=short_wait):
                    with self.assertRaises(Exception):
                        await visible.move_visible_batch(self.page, progress,
                                                         ["first.xlsx", "second.xlsx"], "optimized")
                self.assertEqual(progress.pending_optimized_moves(), {"first.xlsx", "second.xlsx"})
                confirmed, error = progress.connection.execute(
                    "SELECT confirmed_at,error FROM move_batches").fetchone()
                self.assertIsNone(confirmed)
                self.assertTrue(error)

    async def test_automatic_scroll_during_selection_still_moves_exact_batch(self):
        await self.set_folder()
        await self.page.evaluate('''() => {
          const original = window.toggle;
          window.toggle = row => {
            original(row);
            document.querySelector('.list').scrollTop += 1;
          };
          document.querySelector('[data-automationid="moveCommand"]').addEventListener('click', () => {
            window.scrollBeforeMove = document.querySelector('.list').scrollTop;
          });
        }''')
        with tempfile.TemporaryDirectory() as temporary:
            with flow.ProgressStore(Path(temporary), "folder") as progress:
                progress.discover(["first.xlsx", "second.xlsx"])
                complete(progress, "first.xlsx")
                complete(progress, "second.xlsx")
                await visible.move_visible_batch(self.page, progress,
                                                 ["first.xlsx", "second.xlsx"], "optimized")
                self.assertGreater(await self.page.evaluate('window.scrollBeforeMove'), 0)
                self.assertEqual(progress.pending_optimized_moves(), set())

    async def test_move_notice_is_retained_until_delayed_source_updates(self):
        await self.set_folder()
        await self.page.evaluate('''() => {
          window.finishMove=()=>{
            const selected=[...document.querySelectorAll('[data-selection-index][aria-selected="true"]')];
            document.querySelector('#notice').textContent='Moved 2 items to optimized';
            setTimeout(()=>document.querySelector('#notice').textContent='',30);
            setTimeout(()=>{
              selected.forEach(row=>row.remove());
              document.querySelector('iframe').remove();
            },200);
          };
        }''')
        original_wait = self.page.wait_for_function

        async def bounded_wait(script, **kwargs):
            return await original_wait(script, **{**kwargs, "timeout": 2000})

        with tempfile.TemporaryDirectory() as temporary:
            with flow.ProgressStore(Path(temporary), "folder") as progress:
                progress.discover(["first.xlsx", "second.xlsx"])
                complete(progress, "first.xlsx")
                complete(progress, "second.xlsx")
                with patch.object(self.page, "wait_for_function", new=bounded_wait):
                    await visible.move_visible_batch(self.page, progress,
                                                     ["first.xlsx", "second.xlsx"], "optimized")
                self.assertEqual(progress.pending_optimized_moves(), set())
                self.assertEqual(await self.page.locator('#notice').inner_text(), '')

    async def test_empty_onedrive_toast_confirms_only_after_progress(self):
        await self.set_folder()
        await self.page.evaluate('''() => {
          window.finishMove=()=>{
            const notice=document.querySelector('#notice');
            notice.dataset.automationid='msFluentToast';
            notice.innerHTML='<i data-icon-name="SyncStatusSolid"></i>';
            setTimeout(()=>{
              notice.querySelector('i').dataset.iconName='CompletedSolid';
              document.querySelectorAll('[data-selection-index][aria-selected="true"]').forEach(row=>row.remove());
              document.querySelector('iframe').remove();
            },50);
          };
        }''')
        with tempfile.TemporaryDirectory() as temporary:
            with flow.ProgressStore(Path(temporary), "folder") as progress:
                progress.discover(["first.xlsx", "second.xlsx"])
                complete(progress, "first.xlsx")
                complete(progress, "second.xlsx")
                await visible.move_visible_batch(self.page, progress,
                                                 ["first.xlsx", "second.xlsx"], "optimized")
                self.assertEqual(progress.pending_optimized_moves(), set())

    async def test_old_move_notice_cannot_confirm_a_new_batch(self):
        await self.set_folder()
        await self.page.evaluate('''() => {
          document.querySelector('#notice').textContent='Moved 2 items to optimized';
          document.querySelector('#notice').dataset.automationid='msFluentToast';
          document.querySelector('#notice').insertAdjacentHTML('beforeend','<i data-icon-name="CompletedSolid"></i>');
          window.finishMove=()=>{
            document.querySelectorAll('[data-selection-index][aria-selected="true"]').forEach(row=>row.remove());
            document.querySelector('iframe').remove();
          };
        }''')
        original_wait = self.page.wait_for_function

        async def bounded_wait(script, **kwargs):
            return await original_wait(script, **{**kwargs, "timeout": 300})

        with tempfile.TemporaryDirectory() as temporary:
            with flow.ProgressStore(Path(temporary), "folder") as progress:
                progress.discover(["first.xlsx", "second.xlsx"])
                complete(progress, "first.xlsx")
                complete(progress, "second.xlsx")
                with patch.object(self.page, "wait_for_function", new=bounded_wait):
                    with self.assertRaises(Exception):
                        await visible.move_visible_batch(self.page, progress,
                                                         ["first.xlsx", "second.xlsx"], "optimized")
                self.assertEqual(progress.pending_optimized_moves(), {"first.xlsx", "second.xlsx"})

    async def test_changed_file_selection_still_aborts_after_automatic_scroll(self):
        await self.set_folder()
        await self.page.evaluate('''() => {
          const original = window.toggle;
          window.toggle = row => {
            original(row);
            document.querySelector('.list').scrollTop += 1;
            if (row.dataset.selectionIndex === '1') {
              document.querySelector('[data-selection-index="2"]').setAttribute('aria-selected','true');
            }
          };
        }''')
        with tempfile.TemporaryDirectory() as temporary:
            with flow.ProgressStore(Path(temporary), "folder") as progress:
                progress.discover(["first.xlsx", "second.xlsx", "unprocessed.xlsx"])
                complete(progress, "first.xlsx")
                complete(progress, "second.xlsx")
                with self.assertRaisesRegex(RuntimeError, "outside the intended move batch"):
                    await visible.move_visible_batch(self.page, progress,
                                                     ["first.xlsx", "second.xlsx"], "optimized")
                self.assertEqual(progress.connection.execute("SELECT COUNT(*) FROM move_batches").fetchone()[0], 0)

    async def test_preselected_folder_is_not_included_in_a_file_batch(self):
        await self.set_folder()
        await self.page.locator('.list').evaluate('''list => list.insertAdjacentHTML("afterbegin", `
          <div data-selection-index="folder" aria-selected="true">
            <div data-automationid="field-LinkFilename">skipped</div>
          </div>` )''')
        with tempfile.TemporaryDirectory() as temporary:
            with flow.ProgressStore(Path(temporary), "folder") as progress:
                progress.discover(["first.xlsx", "second.xlsx"])
                complete(progress, "first.xlsx")
                complete(progress, "second.xlsx")
                with self.assertRaises(visible.ViewportChanged):
                    await visible.move_visible_batch(self.page, progress,
                                                     ["first.xlsx", "second.xlsx"], "optimized")
                self.assertEqual(progress.connection.execute("SELECT COUNT(*) FROM move_batches").fetchone()[0], 0)


if __name__ == "__main__":
    unittest.main()
