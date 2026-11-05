"""Drain visible OneDrive rows by alternating checking and bulk relocation."""

import asyncio
from enum import StrEnum
from pathlib import Path
import re
import sys

from . import runner as flow


class Phase(StrEnum):
    MOVE = "move visible results"
    CHECK = "check visible workbooks"


class ViewportChanged(RuntimeError):
    """A captured row left the visible list before it could be used."""


async def snapshot(page) -> dict:
    """Read only fully visible XLSX rows within the actual list viewport."""
    return await page.evaluate("""() => {
      const cells = [...document.querySelectorAll('[data-automationid="field-LinkFilename"]')];
      let list = cells[0]?.parentElement;
      while (list && !/^(auto|scroll|overlay)$/.test(getComputedStyle(list).overflowY)) {
        list = list.parentElement;
      }
      list ||= document.scrollingElement;
      const box = list.getBoundingClientRect();
      const header = document.querySelector('[data-automationid="row-header"]');
      const top = Math.max(0, box.top, header?.getBoundingClientRect().bottom || 0);
      const bottom = Math.min(innerHeight, box.bottom);
      const files = cells.map(cell => {
        const row = cell.closest('[data-selection-index]');
        const name = cell.textContent.trim();
        const rect = cell.getBoundingClientRect();
        const icon = row?.querySelector('[data-automationid="field-DocIcon"] img');
        const isXlsx = icon && (icon.getAttribute('alt') === '.xlsx' || icon.getAttribute('aria-label') === '.xlsx');
        return {name, y: rect.top, visible: isXlsx && name.toLowerCase().endsWith('.xlsx') &&
          rect.width > 0 && rect.height > 0 && rect.top >= top && rect.bottom <= bottom};
      }).filter(file => file.visible).sort((a,b) => a.y - b.y).map(file => file.name);
      return {files: [...new Set(files)], scroll_top: list.scrollTop,
        more_below: list.scrollTop + list.clientHeight < list.scrollHeight - 2};
    }""")


async def visible_row(page, name: str):
    if name not in (await snapshot(page))["files"]:
        raise ViewportChanged(f"Workbook left the visible group: {name}")
    return page.get_by_text(name, exact=True).first


def selection_row(page, name: str):
    cell = page.locator('[data-automationid="field-LinkFilename"]').filter(
        has_text=re.compile(rf"^{re.escape(name)}$"))
    return cell.locator('xpath=ancestor::*[@data-selection-index][1]')


async def selected_names(page) -> list[str]:
    names = await page.locator(
        '[data-selection-index]:not([data-selection-index="-1"])[aria-selected="true"] '
        '[data-automationid="field-LinkFilename"]'
    ).all_text_contents()
    return [name.strip() for name in names]


async def move_visible_batch(page, progress: flow.ProgressStore, names: list[str], destination: str):
    """Select exact visible rows, use the toolbar and durably confirm the batch."""
    from playwright.async_api import expect

    initial_view = await snapshot(page)
    current = set(initial_view["files"])
    selected = await selected_names(page)
    if not set(names).issubset(current) or not set(selected).issubset(current):
        raise ViewportChanged("The visible selection changed before relocation")
    for name in selected:
        await selection_row(page, name).locator('[data-automationid^="row-selection-"]').click(timeout=5_000)
    for name in names:
        row = selection_row(page, name)
        # OneDrive overlays its transparent checkbox input with this selection cell.
        await row.locator('[data-automationid^="row-selection-"]').click(timeout=5_000)
        await expect(row).to_have_attribute("aria-selected", "true", timeout=5_000)
    chosen = await selected_names(page)
    if len(chosen) != len(names) or set(chosen) != set(names):
        raise RuntimeError("OneDrive selected files outside the intended move batch")
    if (await snapshot(page))["scroll_top"] != initial_view["scroll_top"]:
        raise ViewportChanged("Selecting the group moved the list viewport")
    batch_id = progress.start_move_batch(names, destination)
    try:
        await page.locator('[data-automationid="moveCommand"]:visible').first.click(timeout=10_000)
        picker = page.frame_locator('iframe[title="Move to"], iframe[title="Verschieben nach"]')
        await picker.get_by_role("link", name=destination, exact=True).dblclick(timeout=30_000)
        await picker.get_by_role("heading", name=destination, exact=True).wait_for(
            state="visible", timeout=30_000)
        if set(await selected_names(page)) != set(names):
            raise RuntimeError("Move selection changed while opening the destination")
        await picker.get_by_role("button", name=re.compile(r"^(Move here|Hierher verschieben)$", re.I)).click(
            timeout=15_000)
        await page.wait_for_function("""({names, destination}) => {
          const notices = [...document.querySelectorAll('[role="alert"], [role="status"]')];
          const confirmed = notices.some(e => {
            const text = e.textContent.toLowerCase();
            return text.includes(destination) && /\\bmoved\\b|\\bverschoben\\b/.test(text) &&
              new RegExp('\\\\b' + names.length + '\\\\b').test(text);
          });
          const remaining = [...document.querySelectorAll('[data-automationid="field-LinkFilename"]')]
            .some(e => names.includes(e.textContent.trim()));
          return confirmed && !remaining;
        }""", arg={"names": names, "destination": destination}, timeout=60_000)
        progress.finish_move_batch(batch_id)
        print(f"Moved {len(names)} visible workbooks to {destination}.", flush=True)
    except Exception as error:
        progress.fail_move_batch(batch_id, f"{type(error).__name__}: {error}")
        raise


async def move_with_recovery(browser, args, profile: Path, progress, names, destination):
    """Retry only original files still visible after a browser recovery."""
    playwright, context, page = browser
    for attempt in (1, 2):
        current = set((await snapshot(page))["files"])
        batch = [name for name in names if name in current]
        if not batch:
            return playwright, context, page
        try:
            await move_visible_batch(page, progress, batch, destination)
            return playwright, context, page
        except ViewportChanged:
            page = await flow.recover_folder(context, page, args.folder_url, args.headed)
            return playwright, context, page
        except Exception as error:
            if attempt == 2:
                raise RuntimeError(f"Visible move to {destination} failed: {error}") from error
            print(f"Visible move interrupted ({error}); recovering browser.", file=sys.stderr, flush=True)
            playwright, context, page = await flow.restart_folder_browser(
                playwright, context, args, profile)


async def read_view_with_recovery(browser, args, profile):
    for attempt in (1, 2):
        try:
            await flow.wait_for_folder_rows(browser[2])
            await flow.sort_oldest_first(browser[2])
            # The destination folder is always present in this mode. Header-only
            # markup can otherwise be mistaken for an empty list during reload.
            await browser[2].locator('[data-selection-index]:not([data-selection-index="-1"])').first.wait_for(
                state="visible", timeout=30_000)
            return browser, await snapshot(browser[2])
        except Exception:
            if attempt == 2:
                raise
            browser = await flow.restart_folder_browser(browser[0], browser[1], args, profile)


async def run_visible_batches(args) -> int:
    identifier = flow.folder_id(args.folder_url)
    profile = args.profile_dir.resolve() if args.profile_dir else args.output_root.resolve() / "browser-profile"
    browser = await flow.restart_folder_browser(None, None, args, profile)
    phase = Phase.MOVE
    attempted = set()
    failed = False
    view_changes = 0
    quiet_views = 0
    last_advance = None
    cohort = []
    try:
        with flow.ProgressStore(args.output_root.resolve() / identifier, identifier) as progress:
            if args.refresh_list:
                await flow.load_workbook_inventory(browser[2], progress, args)
            display = flow.ConsoleProgress(progress)
            try:
                while True:
                    browser, view = await read_view_with_recovery(browser, args, profile)
                    playwright, context, page = browser
                    names = view["files"]
                    progress.discover(names)
                    if phase == Phase.MOVE:
                        completed = [name for name in names if progress.status(name) == "completed"]
                        skipped = [name for name in names if progress.status(name) == "skipped" and
                                   (not args.retry_skipped or name in attempted)]
                        if completed or skipped:
                            destination, batch = ("optimized", completed) if completed else ("skipped", skipped)
                            browser = await move_with_recovery(browser, args, profile, progress, batch, destination)
                            quiet_views = 0
                            last_advance = None
                            continue
                        if len(attempted) >= args.max_files:
                            break
                        cohort = [name for name in names if name not in attempted and
                                  progress.status(name) != "completed" and
                                  (args.retry_skipped or progress.status(name) != "skipped")]
                        if cohort:
                            phase = Phase.CHECK
                            quiet_views = 0
                            last_advance = None
                            print(f"Checking visible group of {len(cohort)} workbooks.", flush=True)
                            continue
                        # Allow OneDrive's delayed replacement rows to settle before
                        # deciding the folder is drained or advancing past blocked rows.
                        quiet_views += 1
                        if quiet_views < (20 if not names else 4):
                            await asyncio.sleep(0.5)
                            continue
                        if view["more_below"]:
                            position = (tuple(names), view["scroll_top"])
                            if position == last_advance:
                                raise RuntimeError("OneDrive did not advance the list viewport")
                            last_advance = position
                            print("No eligible rows in the settled viewport; advancing one viewport.", flush=True)
                            await flow.scroll_file_list(page, to_top=False)
                            quiet_views = 0
                            continue
                        break
                    for name in cohort:
                        if len(attempted) >= args.max_files:
                            break
                        try:
                            playwright, context, page, file_failed = await flow.process_selected_workbook(
                                playwright, context, page, args, profile, progress, display,
                                name, None, row_resolver=visible_row)
                            browser = playwright, context, page
                            attempted.add(name)
                            failed = failed or file_failed
                            view_changes = 0
                        except ViewportChanged:
                            view_changes += 1
                            if view_changes > 3:
                                raise RuntimeError("The visible workbook group keeps changing before opening")
                            break
                        except Exception as error:
                            view_changes += 1
                            if view_changes > 3:
                                raise
                            print(f"Visible check interrupted ({error}); recovering browser.",
                                  file=sys.stderr, flush=True)
                            browser = await flow.restart_folder_browser(
                                playwright, context, args, profile)
                            break
                    phase = Phase.MOVE
            finally:
                display.close()
            print(f"Visible run finished; checked {len(attempted)} workbooks in this invocation.", flush=True)
            return 1 if failed else 0
    finally:
        await flow.close_browser(browser[0], browser[1])
