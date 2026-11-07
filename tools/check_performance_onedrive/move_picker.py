"""Reveal child destination folders in OneDrive's independently sorted move picker."""

import asyncio
import sys

from .diagnostics import capture_failure


async def scan_destination(picker, target) -> bool:
    """Search the picker's virtualized list, including folders outside its first page."""
    pane = picker.locator('.od-FilePicker-scrollPane').first
    await pane.wait_for(state="visible", timeout=10_000)
    await pane.evaluate("element => { element.scrollTop = 0; }")
    settled_bottom = 0
    previous_position = None
    for index in range(10_000):
        if await target.count() and await target.is_visible():
            await target.dblclick(timeout=5_000)
            return True
        position, bottom = await pane.evaluate("""element => {
          element.scrollTop += Math.max(300, element.clientHeight * 0.8);
          return [element.scrollTop, element.scrollHeight - element.clientHeight];
        }""")
        at_end = position >= bottom - 2 or position == previous_position
        settled_bottom = settled_bottom + 1 if at_end else 0
        previous_position = position
        if index % 25 == 0:
            print(f"Move picker search: scroll {position}/{bottom}.", file=sys.stderr, flush=True)
        if settled_bottom >= 8:
            return False
        await asyncio.sleep(0.25 if settled_bottom else 0.1)
    raise RuntimeError("Move picker list did not settle while locating destination")


async def sort_modified(picker, direction: str) -> None:
    from playwright.async_api import expect

    menu = picker.locator('[data-automationid="sortCommand"]')
    await menu.click(timeout=15_000)
    field = picker.locator('[role="menuitemcheckbox"][data-automationid^="sortdateModifiedColumn_"]')
    if await field.get_attribute("aria-checked") != "true":
        await field.click(timeout=10_000)
        await menu.click(timeout=10_000)
    command = "sortDesc" if direction == "descending" else "sortAsc"
    await picker.locator(f'[role="menuitemcheckbox"][data-automationid="{command}"]').click(timeout=10_000)
    modified = picker.locator('[role="columnheader"][data-item-key^="dateModifiedColumn"]')
    await expect(modified).to_have_attribute("aria-sort", direction, timeout=15_000)
    # The sort arrow updates before OneDrive has fetched the replacement rows.
    # An empty scroll pane during that fetch is not the end of the listing.
    await picker.locator('.ms-Spinner').first.wait_for(state="hidden", timeout=60_000)


async def open_destination(picker, destination: str, *, page=None, diagnostic_root=None) -> None:
    from playwright.async_api import TimeoutError as PlaywrightTimeout

    target = picker.get_by_role("link", name=destination, exact=True)
    try:
        await target.dblclick(timeout=2_000)
        return
    except PlaywrightTimeout:
        pass

    for direction in ("descending", "ascending"):
        print(f"Move picker: revealing {destination} by modified date {direction}.",
              file=sys.stderr, flush=True)
        await sort_modified(picker, direction)
        try:
            await target.dblclick(timeout=5_000)
            return
        except PlaywrightTimeout as error:
            if page is not None and diagnostic_root is not None:
                await capture_failure(page, diagnostic_root,
                                      f"move-picker-{destination}-{direction}", error)
            # Flip both directions before traversing thousands of source files.
            # Folders can appear only once the opposite order has been loaded.
            if direction == "ascending" and await scan_destination(picker, target):
                return
    raise RuntimeError(f"Destination folder {destination!r} was not found after sorting and scanning the move picker")
