"""Check the pane observer without opening or changing a cloud workbook."""

import asyncio
from pathlib import Path
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, MagicMock, patch

from playwright.async_api import async_playwright


from tools.check_performance_onedrive import runner as FLOW

PANE_SCRIPT = Path(__file__).with_name("pane.js").read_text(encoding="utf-8")


class PaneObserverTests(unittest.IsolatedAsyncioTestCase):
    async def test_folder_selection_accepts_filename_cells_without_gridcell_role(self):
        await self.page.set_content('''
            <span data-automationid="field-LinkFilename">Name</span>
            <span data-automationid="field-LinkFilename">optimized</span>
            <span data-automationid="field-LinkFilename">pending.xlsx</span>
        ''')
        with patch.object(FLOW, "scroll_file_list", side_effect=AssertionError("unexpected scroll")):
            name, row = await FLOW.next_visible_unfinished(self.page, {"pending.xlsx"})
        self.assertEqual(name, "pending.xlsx")
        self.assertEqual(await row.inner_text(), name)

    async def test_temporary_repair_banner_skips_without_editing(self) -> None:
        """The repaired Reading View is excluded before any editing action."""
        for title, message in [
            ("WORKBOOK REPAIRED", "We temporarily repaired this workbook so that you can open it in Reading View."),
            ("ARBEITSMAPPE REPARIERT", "Wir haben diese Arbeitsmappe vorübergehend repariert, sodass Sie sie in der Leseansicht öffnen können."),
        ]:
            await self.page.set_content(f"""
                <button id="Review">Review</button>
                <div id="BusinessBar"><span>{title}</span><span>{message}</span>
                  <button onclick="window.edited=true">Edit Workbook</button>
                </div>
            """)
            with self.assertRaises(FLOW.WorkbookRepairRequired) as error:
                await FLOW.find_editor(self.page)
            self.assertIn(message, str(error.exception))
            self.assertFalse(await self.page.evaluate("Boolean(window.edited)"))

    async def test_german_irm_dialog_skips_before_review_is_ready(self) -> None:
        """German IRM wording must be recognized over Excel's loading skeleton."""
        await self.page.set_content("""
            <div role="dialog"><h2>In Desktop-App öffnen?</h2>
              <p>Excel kann dieses Dokument leider nicht zum Bearbeiten in einem Browser öffnen,
              weil es durch die Verwaltung von Informationsrechten
              (Information Rights Management, IRM) geschützt ist.</p>
              <button onclick="window.desktopOpened=true">In Desktop-App öffnen</button>
            </div>
        """)
        with self.assertRaises(FLOW.WorkbookEditRestricted) as error:
            await asyncio.wait_for(FLOW.find_editor(self.page), timeout=2)
        self.assertIn("Informationsrechten", str(error.exception))
        self.assertFalse(await self.page.evaluate("Boolean(window.desktopOpened)"))

    async def test_repair_confirmation_is_logged_without_accepting(self) -> None:
        """An already-visible repair dialog is excluded like the repair banner."""
        await self.page.set_content("""
            <div role="dialog"><h2>Diese Arbeitsmappe reparieren?</h2>
              <p>Diese Arbeitsmappe ist beschädigt.</p>
              <button onclick="window.repaired=true">Reparieren</button>
            </div>
        """)
        with self.assertRaises(FLOW.WorkbookRepairRequired):
            await FLOW.find_editor(self.page)
        self.assertFalse(await self.page.evaluate("Boolean(window.repaired)"))

    async def test_repair_arriving_while_performance_is_disabled_is_excluded(self) -> None:
        """A late notice ends readiness waiting with the specific skip reason."""
        await self.page.set_content("""
            <button id="ShowOptimizeWorkbookTaskpane" disabled>Performance</button>
            <script>setTimeout(() => {
              const bar = document.createElement('div');
              bar.id = 'BusinessBar';
              bar.innerHTML = '<span>WORKBOOK REPAIRED</span><span>Reading View</span>';
              document.body.append(bar);
            }, 100);</script>
        """)
        with self.assertRaises(FLOW.WorkbookRepairRequired):
            await asyncio.wait_for(FLOW.find_performance_pane(self.page, self.page), timeout=2)

    async def test_browser_restart_replaces_playwright_driver(self) -> None:
        """A dead driver needs a new connection, not just another Chromium tab."""
        with tempfile.TemporaryDirectory() as temporary:
            args = SimpleNamespace(browser_channel="chromium", headed=False,
                                   folder_url="https://example.test/folder")
            with (patch.object(FLOW, "open_folder", side_effect=lambda page, *_: page),
                  patch.object(FLOW, "sort_newest_first", new_callable=AsyncMock)):
                first_driver, first_context, _ = await FLOW.restart_folder_browser(
                    None, None, args, Path(temporary))
                second_driver, second_context, _ = await FLOW.restart_folder_browser(
                    first_driver, first_context, args, Path(temporary))
                try:
                    self.assertIsNot(first_driver, second_driver)
                    self.assertIsNot(first_context, second_context)
                finally:
                    await FLOW.close_browser(second_driver, second_context)

    async def test_irm_dialog_blocks_visible_review_tab(self) -> None:
        """A visible ribbon behind Excel's IRM dialog must not start optimization."""
        await self.page.set_content("<iframe></iframe>")
        frame = self.page.frames[1]
        await frame.set_content("""
            <button id="Review">Review</button>
            <div role="dialog">
              <h2>Open in Desktop App?</h2>
              <p>Sorry, Excel can't open this document for editing in a browser
                 because it is protected by Information Rights Management (IRM).
                 To edit this document, please open it in the desktop version of Excel.</p>
              <button>Open in Desktop App</button>
              <button>Cancel</button>
            </div>
        """)
        with self.assertRaises(FLOW.WorkbookEditRestricted):
            await FLOW.find_editor(self.page)
        with self.assertRaises(FLOW.WorkbookEditRestricted):
            await FLOW.select_review(self.page, frame)

    async def test_invalid_objects_banner_enables_editing(self) -> None:
        """The exact BusinessBar warning can be repaired before Performance opens."""
        await self.page.set_content("""
            <div id="BusinessBar">
              <span>Read-only</span>
              <span>The workbook is opened in viewing mode because of presence
                    of many invalid objects.</span>
              <button aria-label="Remove and Edit" onclick="
                document.querySelector('#BusinessBar').remove();
                document.querySelector('#ShowOptimizeWorkbookTaskpane').disabled = false;
              ">Remove and Edit</button>
            </div>
            <button id="ShowOptimizeWorkbookTaskpane" disabled>Performance</button>
        """)
        self.assertTrue(await FLOW.remove_invalid_objects_for_editing(self.page))
        self.assertTrue(await self.page.locator("#ShowOptimizeWorkbookTaskpane").is_enabled())

    async def test_other_read_only_warning_is_not_repaired(self) -> None:
        """A different read-only cause must not trigger object removal."""
        await self.page.set_content("""
            <div id="BusinessBar">
              <span>Read-only because another user is editing.</span>
              <button aria-label="Remove and Edit">Remove and Edit</button>
            </div>
        """)
        with self.assertRaisesRegex(RuntimeError, "unrecognized read-only warning"):
            await FLOW.remove_invalid_objects_for_editing(self.page)

    async def test_disabled_performance_is_unavailable(self) -> None:
        """Excel can show the ribbon command while disabling its whole group."""
        class DisabledCommand:
            first = None

            async def wait_for(self, **_kwargs):
                return None

            async def is_enabled(self):
                return False

        command = DisabledCommand()
        command.first = command

        class Editor:
            def locator(self, _selector):
                return command

        with patch.object(FLOW, "time") as clock:
            clock.monotonic.side_effect = [0, 11]
            self.assertIsNone(await FLOW.find_performance_pane(self.page, Editor()))

    async def asyncSetUp(self) -> None:
        self.playwright = await async_playwright().start()
        self.browser = await self.playwright.chromium.launch(headless=True)
        self.page = await self.browser.new_page()
        self.events = []
        await self.page.expose_binding(
            "__checkPerformanceEvent", lambda _source, phase: self.events.append(phase))

    async def asyncTearDown(self) -> None:
        await self.browser.close()
        await self.playwright.stop()

    async def test_iframe_reports_accepted_click_and_completion(self) -> None:
        """An Excel-like pane must report acceptance only after its button changes."""
        await self.page.set_content("<iframe></iframe>")
        pane = self.page.frames[1]
        await pane.set_content("""
            <button id="optimize">Optimize all</button><div id="result"></div>
            <script>
              document.querySelector("#optimize").onclick = () => {
                document.querySelector("#optimize").disabled = true;
                document.querySelector("#optimize").textContent = "Optimizing";
                setTimeout(() => {
                  document.querySelector("#result").textContent = "Workbook optimized";
                }, 20);
              };
            </script>
        """)
        self.assertEqual(await pane.evaluate(PANE_SCRIPT), "optimized")
        self.assertEqual(self.events, ["optimization_click_dispatched", "optimization_accepted"])

    async def test_two_workbooks_reuse_one_page_event_binding(self) -> None:
        """A second workbook in the same tab must not register the binding again."""
        page = await self.browser.new_page()
        progress = MagicMock()
        workbook_html = """
            <button onclick="
              this.disabled = true;
              this.textContent = 'Optimizing';
              document.querySelector('#result').textContent = 'Workbook optimized';
            ">Optimize all</button>
            <div id="result"></div>
        """
        try:
            with patch.object(FLOW, "record_phase") as record:
                for name in ("first.xlsx", "second.xlsx"):
                    await page.set_content(workbook_html)
                    self.assertEqual(
                        await FLOW.optimize(page, page, progress, MagicMock(), name),
                        "optimized")
                self.assertEqual(record.call_count, 4)
                self.assertEqual(
                    [call.args[2] for call in record.call_args_list],
                    ["first.xlsx", "first.xlsx", "second.xlsx", "second.xlsx"])
        finally:
            await page.close()

    async def test_no_optimization_needed_is_already_clean(self) -> None:
        """Excel's current English no-work message must complete without a click."""
        await self.page.set_content(
            "<div>No optimization needed</div><p>Nothing to optimize in this workbook.</p>")
        self.assertRegex("No optimization needed", FLOW.ALREADY_CLEAN)
        self.assertEqual(await self.page.evaluate(PANE_SCRIPT), "already_clean")
        self.assertEqual(self.events, [])

    async def test_ignored_click_is_not_reported_as_accepted(self) -> None:
        """A visible button whose handler ignores clicks must fail promptly."""
        await self.page.set_content("<button>Alle optimieren</button>")
        short_timeout_script = PANE_SCRIPT.replace("20_000", "100")
        self.assertEqual(await self.page.evaluate(short_timeout_script), "click_not_acknowledged")
        self.assertEqual(self.events, ["optimization_click_dispatched"])

    async def test_check_again_can_confirm_completion(self) -> None:
        """A transient error may resolve to Excel's no-work message."""
        await self.page.set_content("""
            <button id="optimize">Alle optimieren</button><div id="result"></div>
            <script>
              const button = document.querySelector("#optimize");
              button.onclick = () => {
                button.textContent = "Check again";
                button.onclick = () => {
                  button.remove();
                  document.querySelector("#result").textContent =
                    "Keine Optimierung erforderlich";
                };
              };
            </script>
        """)
        self.assertEqual(await self.page.evaluate(PANE_SCRIPT), "already_clean_after_check_again")
        self.assertIn("check_again_clicked", self.events)

    async def test_editor_readiness_ignores_unrelated_dom_activity(self) -> None:
        """A late Excel iframe becomes ready without a global quiet-period debounce."""
        waiting = asyncio.create_task(FLOW.find_editor(self.page))
        await asyncio.sleep(0)
        await self.page.set_content("<iframe></iframe>")
        frame = self.page.frames[1]
        await frame.set_content("<div id='noise'></div>")
        await frame.evaluate("""() => {
          const noise = document.querySelector('#noise');
          const timer = setInterval(() => noise.textContent += '.', 5);
          setTimeout(() => {
            const review = document.createElement('button');
            review.id = 'Review';
            document.body.appendChild(review);
            const account = document.createElement('button');
            account.setAttribute('aria-label', 'Konto-Manager');
            document.body.appendChild(account);
            setTimeout(() => clearInterval(timer), 1000);
          }, 50);
        }""")
        started = time.monotonic()
        self.assertIs(await asyncio.wait_for(waiting, timeout=5), frame)
        self.assertLess(time.monotonic() - started, 0.5)

    async def test_review_is_ready_without_account_manager_button(self) -> None:
        """Excel can show an editable Review tab with no matching account label."""
        await self.page.set_content("<iframe></iframe>")
        frame = self.page.frames[1]
        await frame.set_content('<button id="Review">Review</button>')
        self.assertIs(await asyncio.wait_for(FLOW.find_editor(self.page), timeout=2), frame)

    async def test_review_waits_for_selected_attribute(self) -> None:
        """The ribbon click counts only after Excel marks Review selected."""
        await self.page.set_content("""
            <button id="Review" aria-selected="false">Review</button>
            <script>
              document.querySelector('#Review').onclick = () => setTimeout(() => {
                document.querySelector('#Review').setAttribute('aria-selected', 'true');
              }, 50);
            </script>
        """)
        await FLOW.select_review(self.page, self.page.main_frame)
        self.assertEqual(await self.page.locator('#Review').get_attribute('aria-selected'), 'true')

    async def test_open_error_over_visible_ribbon_stops_review(self) -> None:
        """A ribbon behind Excel's fatal modal must not count as an open workbook."""
        await self.page.set_content("""
            <button id="Review">Review</button>
            <div role="dialog">
              <h2>Couldn't Open the Workbook</h2>
              <p>The workbook cannot be opened.</p>
              <button>OK</button>
            </div>
        """)
        with self.assertRaises(FLOW.WorkbookCannotOpen):
            await FLOW.find_editor(self.page)
        with self.assertRaises(FLOW.WorkbookCannotOpen):
            await FLOW.select_review(self.page, self.page.main_frame)

    async def test_open_error_retries_once_then_reports_skip_cause(self) -> None:
        """Two explicit Excel rejections reuse the same tab without claiming success."""
        page = MagicMock()
        page.url = "https://example.test/folder"
        page.goto = AsyncMock()
        page.locator.return_value.first.wait_for = AsyncMock()
        progress = AsyncMock()
        with (patch.object(FLOW, "open_workbook_in_current_tab", new_callable=AsyncMock,
                           return_value=page) as open_tab,
              patch.object(FLOW, "find_editor", side_effect=FLOW.WorkbookCannotOpen()),
              patch.object(FLOW, "sort_newest_first", new_callable=AsyncMock),
              patch.object(FLOW, "record_phase")):
            with self.assertRaisesRegex(FLOW.WorkbookCannotOpen, "both opening attempts"):
                await FLOW.open_editor_with_retry(None, page, "broken.xlsx", progress, None)
        self.assertEqual(open_tab.await_count, 2)
        page.goto.assert_awaited_once_with(
            page.url, wait_until="domcontentloaded", timeout=120_000)

    async def test_editor_waits_through_sign_in_control(self) -> None:
        """An anonymous Excel frame can become an authenticated editor."""
        await self.page.set_content("<iframe></iframe>")
        frame = self.page.frames[1]
        await frame.set_content("""
            <button id="Review">Review</button>
            <button id="O365_MainLink_SigninLink">Sign in</button>
            <script>
              document.querySelector('#O365_MainLink_SigninLink').onclick = () => {
                document.querySelector('#O365_MainLink_SigninLink').remove();
                const account = document.createElement('button');
                account.setAttribute('aria-label', 'Konto-Manager');
                document.body.appendChild(account);
              };
            </script>
        """)
        self.assertIs(await FLOW.find_editor(self.page), frame)

    async def test_editor_starts_sign_in_before_review_appears(self) -> None:
        """A workbook that first exposes Sign in must not stall waiting for Review."""
        await self.page.set_content("<iframe></iframe>")
        frame = self.page.frames[1]
        await frame.set_content("""
            <button id="O365_MainLink_SigninLink" onclick="
              this.remove();
              document.body.insertAdjacentHTML('beforeend',
                '<button id=Review>Review</button>');
            ">Sign in</button>
        """)
        self.assertIs(await asyncio.wait_for(FLOW.find_editor(self.page), timeout=2), frame)
        self.assertFalse(await frame.locator("#O365_MainLink_SigninLink").count())

    async def test_folder_sign_in_requires_visible_browser(self) -> None:
        """A headless login redirect must fail clearly before searching for files."""
        page = AsyncMock()
        page.url = "https://login.microsoftonline.com/common/oauth2/authorize"
        with self.assertRaisesRegex(RuntimeError, "restart with --headed"):
            await FLOW.open_folder(page, "https://1drv.ms/f/example", headed=False)
        page.locator.assert_not_called()

    async def test_folder_wait_follows_document_redirect(self) -> None:
        """OneDrive can replace the share-link document before showing its rows."""
        async def route(request):
            if request.request.url.endswith("/share"):
                body = "<script>setTimeout(() => location.href = '/folder', 30)</script>"
            else:
                body = '<div data-automationid="field-LinkFilename">book.xlsx</div>'
            await request.fulfill(status=200, content_type="text/html", body=body)

        await self.page.route("https://example.test/**", route)
        await self.page.goto("https://example.test/share")
        await asyncio.wait_for(FLOW.wait_for_folder_response(self.page), timeout=3)
        self.assertTrue(self.page.url.endswith("/folder"))

    async def test_visible_folder_sign_in_waits_for_return(self) -> None:
        """A visible login redirect must resume the same folder workflow."""
        page = AsyncMock()
        page.url = "https://login.microsoftonline.com/common/oauth2/authorize"
        page.locator = MagicMock()
        page.locator.return_value.first.wait_for = AsyncMock()
        with (patch.object(FLOW, "wait_for_signed_in_folder", new_callable=AsyncMock,
                           return_value=page) as wait,
              patch.object(FLOW, "visible_sign_in", new_callable=AsyncMock, return_value=None)):
            await FLOW.open_folder(page, "https://1drv.ms/f/example", headed=True)
        wait.assert_awaited_once_with(page, "https://1drv.ms/f/example", True)
        page.locator.return_value.first.wait_for.assert_awaited_once()

    async def test_public_folder_sign_in_navigates_in_same_tab(self) -> None:
        """Visible files do not imply edit access when the top bar says Sign in."""
        page = AsyncMock()
        page.url = "https://1drv.ms/f/example"
        page.locator = MagicMock()
        page.locator.return_value.first.wait_for = AsyncMock()
        page.locator.return_value.first.is_visible = AsyncMock(return_value=True)
        sign_in = AsyncMock()
        sign_in.get_attribute.return_value = "https://login.microsoftonline.com/authorize"
        with (patch.object(FLOW, "visible_sign_in", new_callable=AsyncMock,
                           side_effect=[sign_in, None]),
              patch.object(FLOW, "wait_for_signed_in_folder", new_callable=AsyncMock,
                           return_value=page) as wait):
            await FLOW.open_folder(page, page.url, headed=True)
        sign_in.click.assert_not_awaited()
        page.goto.assert_any_await(
            "https://login.microsoftonline.com/authorize",
            wait_until="domcontentloaded", timeout=120_000)
        wait.assert_awaited_once_with(page, page.url, True)

    async def test_workbook_opens_without_creating_a_second_tab(self) -> None:
        """OneDrive's window.open navigation stays in the current page."""
        async def route(request):
            body = ('<button id="file" ondblclick="window.open(\'/Doc.aspx?file=x.xlsx\')">'
                    'x.xlsx</button>' if request.request.url.endswith('/folder') else
                    '<button id="Review">Review</button>')
            await request.fulfill(status=200, content_type="text/html", body=body)

        await self.page.route("https://example.test/**", route)
        await self.page.add_init_script(FLOW.SINGLE_TAB_NAVIGATION)
        await self.page.goto("https://example.test/folder")
        result = await FLOW.open_workbook_in_current_tab(
            None, self.page, "x.xlsx", self.page.locator("#file"))
        self.assertIs(result, self.page)
        self.assertEqual(len(self.page.context.pages), 1)

    async def test_workbook_open_reports_onedrive_throttling(self) -> None:
        """A throttle redirect must not look like a slow Excel workbook."""
        async def route(request):
            if request.request.url.endswith("/folder"):
                await request.fulfill(
                    status=200, content_type="text/html",
                    body="<button id='file' ondblclick=\"location.href='/Throttle.htm'\">x.xlsx</button>")
            else:
                await request.fulfill(status=200, content_type="text/html", body="Throttled")

        await self.page.route("https://example.test/**", route)
        await self.page.goto("https://example.test/folder")
        with self.assertRaises(FLOW.OneDriveThrottled):
            await FLOW.open_workbook_in_current_tab(
                None, self.page, "x.xlsx", self.page.locator("#file"))

    async def test_folder_wait_reports_onedrive_throttling(self) -> None:
        """Returning from a workbook must not wait for rows on a throttle page."""
        await self.page.route("https://example.test/Throttle.htm", lambda route:
                              route.fulfill(status=200, content_type="text/html", body="Throttled"))
        await self.page.goto("https://example.test/Throttle.htm")
        with self.assertRaises(FLOW.OneDriveThrottled):
            await FLOW.wait_for_folder_rows(self.page)

    async def test_irm_workbook_is_recorded_as_skipped(self) -> None:
        """The desktop-only dialog must not count as a successful optimization."""
        progress = MagicMock()
        progress.status.return_value = FLOW.JobState.PENDING.value
        progress.start_attempt.return_value = 7
        display = MagicMock()
        with patch.object(FLOW, "open_editor_with_retry", new_callable=AsyncMock,
                          side_effect=FLOW.WorkbookEditRestricted("IRM")):
            await FLOW.process_workbook(
                None, self.page, Path("/tmp"), progress, display, "restricted.xlsx")
        progress.skip.assert_called_once_with(
            "restricted.xlsx", 7, "irm_browser_edit_unavailable", "IRM")
        progress.finish.assert_not_called()
        display.finish.assert_called_once()

    async def test_unexpected_workbook_error_remains_retryable(self) -> None:
        """A file-specific exception must be recorded before the fresh-browser retry."""
        with tempfile.TemporaryDirectory() as temporary:
            with FLOW.ProgressStore(Path(temporary), "folder-one") as progress:
                progress.discover(["broken.xlsx", "next.xlsx"])
                display = MagicMock()
                with patch.object(FLOW, "open_editor_with_retry", new_callable=AsyncMock,
                                  side_effect=RuntimeError("Excel did not respond")):
                    await FLOW.process_workbook(
                        None, self.page, Path(temporary), progress, display, "broken.xlsx")
                self.assertEqual(progress.status("broken.xlsx"), FLOW.JobState.FAILED.value)
                self.assertIsNone(progress.outcome("broken.xlsx"))
                self.assertIn("next.xlsx", progress.unfinished_names())
                self.assertIn("Excel did not respond", progress.connection.execute(
                    "SELECT error FROM attempts WHERE file_name = 'broken.xlsx'").fetchone()[0])

    async def test_folder_recovery_uses_shared_link_after_navigation_failure(self) -> None:
        """The next workbook can use the same tab after one folder reload fails."""
        with (patch.object(FLOW, "open_folder", new_callable=AsyncMock,
                           return_value=self.page) as open_folder,
              patch.object(FLOW, "sort_newest_first", new_callable=AsyncMock) as sort):
            recovered = await FLOW.recover_folder(
                self.page.context, self.page, "https://1drv.ms/f/example", True)
        self.assertIs(recovered, self.page)
        open_folder.assert_awaited_once_with(
            self.page, "https://1drv.ms/f/example", True)
        sort.assert_awaited_once_with(self.page)

    async def test_target_blank_login_link_stays_in_one_tab(self) -> None:
        """An account link with a new-tab target navigates the current window."""
        async def route(request):
            body = ('<a id="login" href="/login" target="_blank">Sign in</a>'
                    if request.request.url.endswith('/folder') else '<p>Sign-in page</p>')
            await request.fulfill(status=200, content_type="text/html", body=body)

        await self.page.route("https://example.test/**", route)
        await self.page.add_init_script(FLOW.SINGLE_TAB_NAVIGATION)
        await self.page.goto("https://example.test/folder")
        await self.page.locator("#login").click(force=True)
        await self.page.wait_for_url("https://example.test/login")
        self.assertEqual(len(self.page.context.pages), 1)

    async def test_fresh_start_keeps_cookies_but_discards_restored_tabs(self) -> None:
        """Restarting the runner must not reopen last run's browser tabs."""
        with tempfile.TemporaryDirectory() as directory:
            profile = Path(directory)
            sessions = profile / "Default" / "Sessions"
            sessions.mkdir(parents=True)
            (sessions / "Tabs_123").write_bytes(b"old tabs")
            (sessions / "Session_123").write_bytes(b"old session")
            cookies = profile / "Default" / "Cookies"
            cookies.write_bytes(b"auth data")
            FLOW.discard_restored_tabs(profile)
            self.assertEqual(list(sessions.iterdir()), [])
            self.assertEqual(cookies.read_bytes(), b"auth data")

    async def test_top_bar_sign_in_is_detected_without_special_id(self) -> None:
        """OneDrive's localized top-bar label is enough to request authentication."""
        await self.page.set_content('<header><button>Sign in</button></header>')
        control = await FLOW.visible_sign_in(self.page)
        self.assertIsNotNone(control)
        self.assertEqual(await control.inner_text(), "Sign in")

    async def test_folder_without_rows_reports_sign_in_prompt_immediately(self) -> None:
        """A public folder sign-in control must not wait 60 seconds for file rows."""
        await self.page.set_content('<header><button>Sign in</button></header>')
        await asyncio.wait_for(FLOW.wait_for_folder_response(self.page), timeout=2)
        with self.assertRaisesRegex(RuntimeError, "sign-in required"):
            await asyncio.wait_for(FLOW.wait_for_folder_rows(self.page), timeout=2)



class JobStateTests(unittest.TestCase):
    def test_only_valid_transitions_are_accepted(self) -> None:
        """An accepted click changes checking to optimizing, then saving."""
        from tools.check_performance_onedrive.runner import JobState, validated_state

        status = "pending"
        for state in (JobState.OPENING, JobState.CHECKING,
                      JobState.OPTIMIZING, JobState.SAVING, JobState.COMPLETED):
            status = validated_state(status, state)
        self.assertEqual(status, "completed")
        with self.assertRaises(ValueError):
            validated_state(status, JobState.OPENING)

    def test_old_running_status_can_resume(self) -> None:
        """Existing progress files with `running` can start a fresh attempt."""
        from tools.check_performance_onedrive.runner import JobState, validated_state

        self.assertEqual(validated_state("running", JobState.OPENING), "opening")


if __name__ == "__main__":
    unittest.main()
