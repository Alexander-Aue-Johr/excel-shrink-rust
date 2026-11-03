"""Compare serial, staged parallel, and hybrid optimization of shared OneDrive copies."""

import argparse
import asyncio
from dataclasses import dataclass
import json
from pathlib import Path
import re
import shutil
import sys
import time


FOLDERS = {
    "sequential": "Optimized Kopieren4",
    "parallel": "Optimized Kopieren3",
    "hybrid": "Optimized Kopieren2",
}
OPTIMIZE = re.compile(r"^(Optimize all|Alle optimieren)$", re.I)
ALREADY_CLEAN = re.compile(r"Keine Optimierung erforderlich|No optimization required", re.I)
OPTIMIZED = re.compile(r"Arbeitsmappe optimiert|Workbook optimized", re.I)
CHECK_AGAIN = re.compile(r"^(Check again|Erneut prüfen)$", re.I)
ERROR = re.compile(r"Something went wrong|Etwas ist schiefgelaufen", re.I)


@dataclass
class WorkbookSession:
    """Keep browser objects beside a JSON-safe per-file observation."""

    name: str
    page: object
    record: dict
    editor: object = None
    pane: object = None


def write_result(directory: Path, result: dict) -> None:
    """Persist phase results before a later workbook can block the run."""
    path = directory / "onedrive-staged-result.json"
    path.write_text(json.dumps(result, indent=2, ensure_ascii=True) + "\n", encoding="utf-8")


def mark(session: WorkbookSession, phase: str) -> None:
    """Record comparable wall times for all browser tabs."""
    at = time.time()
    session.record["times"][phase] = at
    print(json.dumps({"file": session.name, "phase": phase, "at": at}), flush=True)


async def open_folder(page, folder_url: str, folder_name: str) -> list[str]:
    """Use the public share to discover the eight workbooks in a copied folder."""
    await page.goto(folder_url, wait_until="domcontentloaded", timeout=120_000)
    folder = page.get_by_text(folder_name, exact=True)
    await folder.wait_for(state="visible", timeout=60_000)
    await folder.dblclick()
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        names = [name for name in await page.locator(
            '[data-automationid="field-LinkFilename"]').all_text_contents()
                 if name.lower().endswith(".xlsx")]
        if len(names) == 8:
            return sorted(names)
        await asyncio.sleep(0.5)
    raise RuntimeError(f"Expected eight XLSX files in {folder_name}")


async def open_workbook(context, folder_page, name: str) -> WorkbookSession:
    """Open a cloud workbook from its OneDrive row in a new browser tab."""
    record = {"status": "running", "times": {}}
    session = WorkbookSession(name, None, record)
    mark(session, "open_started")
    await folder_page.locator(".ms-Overlay:visible").wait_for(state="hidden", timeout=60_000)
    async with context.expect_page(timeout=60_000) as event:
        await folder_page.get_by_text(name, exact=True).dblclick()
    session.page = await event.value
    mark(session, "tab_created")
    return session


async def find_signed_in_editor(page):
    """Wait for Excel's editor and promote an anonymous shared view to the consented account."""
    deadline = time.monotonic() + 120
    while time.monotonic() < deadline:
        for frame in page.frames:
            try:
                if not await frame.locator("#Review:visible").count():
                    continue
                account = frame.locator(
                    'button[aria-label*="Konto-Manager"], button[aria-label*="Account manager"]')
                if await account.count():
                    return frame
                sign_in = frame.locator("#O365_MainLink_SigninLink:visible")
                if await sign_in.count():
                    await sign_in.click()
                    await asyncio.sleep(1)
                    break
            except Exception:
                continue
        await asyncio.sleep(0.5)
    raise RuntimeError("Excel did not expose an authenticated editor")


async def prepare_workbook(session: WorkbookSession) -> None:
    """Wait until the workbook is interactive before any synchronized ribbon action."""
    session.editor = await find_signed_in_editor(session.page)
    session.record["editor_header"] = (await session.editor.locator("body").inner_text())[:180]
    mark(session, "editor_ready")


async def select_review(session: WorkbookSession) -> None:
    """Wait for the Review tab's selected state, including ignored-click retries."""
    from batch import open_review_ribbon

    await open_review_ribbon(session.editor)
    mark(session, "review_ready")


async def open_performance(session: WorkbookSession) -> None:
    """Open the Performance task pane and wait for an actionable or clean result."""
    command = session.editor.locator("#ShowOptimizeWorkbookTaskpane:visible").first
    try:
        await command.wait_for(state="visible", timeout=30_000)
    except Exception:
        session.record["review_selected"] = await session.editor.locator(
            "#Review:visible").first.get_attribute("aria-selected")
        session.record["performance_dom_count"] = await session.editor.locator(
            "#ShowOptimizeWorkbookTaskpane").count()
        session.record["review_body"] = (await session.editor.locator("body").inner_text())[:650]
        raise
    await command.click()
    deadline = time.monotonic() + 120
    while time.monotonic() < deadline:
        for frame in session.page.frames:
            if "resources.office.net" not in frame.url:
                continue
            if (await frame.get_by_role("button", name=OPTIMIZE).count()
                    or await frame.get_by_text(ALREADY_CLEAN).count()
                    or await frame.get_by_role("button", name=CHECK_AGAIN).count()):
                session.pane = frame
                mark(session, "performance_ready")
                return
        await asyncio.sleep(0.5)
    raise RuntimeError("Performance pane did not become ready")


async def wait_for_optimization(session: WorkbookSession) -> str:
    """Distinguish success, already-clean, recoverable error, and timeout."""
    deadline = time.monotonic() + 180
    checked_again = False
    while time.monotonic() < deadline:
        pane = session.pane
        if await pane.get_by_text(OPTIMIZED).count():
            return "optimized"
        if await pane.get_by_text(ALREADY_CLEAN).count():
            return "already_clean_after_check_again" if checked_again else "already_clean"
        check_again = pane.get_by_role("button", name=CHECK_AGAIN)
        if await check_again.count():
            if checked_again:
                return "error_after_check_again"
            await check_again.click()
            checked_again = True
            mark(session, "check_again_clicked")
        elif await pane.get_by_text(ERROR).count() and checked_again:
            return "error_after_check_again"
        await asyncio.sleep(0.5)
    session.record["pane_at_timeout"] = (await session.pane.locator("body").inner_text())[-2000:]
    return "timed_out"


async def optimize_workbook(session: WorkbookSession) -> None:
    """Click Optimize all once and record its explicit outcome without downloading."""
    if await session.pane.get_by_text(ALREADY_CLEAN).count():
        session.record["optimization"] = "already_clean_before_click"
        mark(session, "optimization_finished")
        return
    mark(session, "optimization_started")
    await session.pane.get_by_role("button", name=OPTIMIZE).click(timeout=120_000)
    session.record["optimization"] = await wait_for_optimization(session)
    mark(session, "optimization_finished")
    if session.record["optimization"] in {"timed_out", "error_after_check_again"}:
        session.record["status"] = "failed"
        return
    if session.record["optimization"] in {"optimized", "already_clean_after_check_again"}:
        await wait_for_cloud_save(session)


async def wait_for_cloud_save(session: WorkbookSession) -> None:
    """Keep the tab open until Excel confirms that its cloud save completed."""
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        labels = await session.editor.locator("button[aria-label]").evaluate_all("""buttons =>
          buttons.map(button => button.getAttribute('aria-label'))
            .filter(label => /Zuletzt gespeichert|Saved|Gespeichert|Wird gespeichert/i.test(label || ''))""")
        session.record["save_indicator"] = labels[:2]
        if any(re.search(r"Gespeichert|Saved", label, re.I) and
               not re.search(r"Wird gespeichert|Saving", label, re.I) for label in labels):
            session.record["save_confirmed"] = True
            mark(session, "cloud_saved")
            return
        await asyncio.sleep(0.5)
    session.record["save_confirmed"] = False
    session.record["status"] = "save_unconfirmed"


async def apply_stage(sessions: list[WorkbookSession], action, concurrent: bool) -> None:
    """Apply one stage to all healthy sessions before the next stage begins."""
    async def attempt(session: WorkbookSession) -> None:
        if session.record["status"] != "running":
            return
        try:
            await action(session)
        except Exception as error:
            session.record["status"] = "failed"
            session.record["error"] = str(error)[:500]
            mark(session, "failed")

    if concurrent:
        await asyncio.gather(*(attempt(session) for session in sessions))
    else:
        for session in sessions:
            await attempt(session)


async def run_sequential(context, folder_page, names: list[str], mode: dict,
                         directory: Path, result: dict) -> None:
    """Complete each workbook before opening the next workbook."""
    for name in names:
        session = await open_workbook(context, folder_page, name)
        mode["jobs"][name] = session.record
        try:
            for action in (prepare_workbook, select_review, open_performance, optimize_workbook):
                await apply_stage([session], action, concurrent=False)
            if session.record["status"] == "running":
                session.record["status"] = "completed"
        finally:
            await session.page.close()
            write_result(directory, result)


async def run_staged(context, folder_page, names: list[str], mode: dict,
                     optimize_concurrently: bool, directory: Path, result: dict) -> None:
    """Load all files, then cross stage barriers before the optimization policy."""
    sessions = []
    for name in names:
        session = await open_workbook(context, folder_page, name)
        mode["jobs"][name] = session.record
        sessions.append(session)
    try:
        await apply_stage(sessions, prepare_workbook, concurrent=True)
        mode["times"]["all_editors_ready"] = time.time()
        write_result(directory, result)
        await asyncio.sleep(5)
        await apply_stage(sessions, select_review, concurrent=True)
        mode["times"]["all_review_ready"] = time.time()
        write_result(directory, result)
        await asyncio.sleep(3)
        await apply_stage(sessions, open_performance, concurrent=True)
        mode["times"]["all_performance_ready"] = time.time()
        write_result(directory, result)
        await asyncio.sleep(3)
        await apply_stage(sessions, optimize_workbook, concurrent=optimize_concurrently)
        mode["times"]["all_optimization_finished"] = time.time()
        for session in sessions:
            if session.record["status"] == "running":
                session.record["status"] = "completed"
        write_result(directory, result)
    finally:
        await asyncio.gather(*(session.page.close() for session in sessions))


async def run_experiment(directory: Path, folder_url: str, mode_name: str, limit: int,
                         folder_override: str | None, only_file: str | None) -> None:
    """Measure one strategy against one copied folder in a fresh browser process."""
    from playwright.async_api import async_playwright

    result = {"browser": {}, "modes": {}}
    result_path = directory / "onedrive-staged-result.json"
    if result_path.exists():
        backup = directory / f"onedrive-staged-before-{int(time.time())}.json"
        shutil.copyfile(result_path, backup)
    result["browser"]["started"] = time.time()
    async with async_playwright() as playwright:
        context = await playwright.chromium.launch_persistent_context(
            directory / "edge-profile", channel="msedge", headless=True,
            chromium_sandbox=True, viewport={"width": 1920, "height": 1080})
        try:
            result["browser"]["ready"] = time.time()
            folder_page = context.pages[0] if context.pages else await context.new_page()
            folder_name = folder_override or FOLDERS[mode_name]
            names = await open_folder(folder_page, folder_url, folder_name)
            if only_file:
                names = [name for name in names if name == only_file]
                if not names:
                    raise RuntimeError(f"{only_file} is absent from {folder_name}")
            names = names[:limit]
            mode = {"folder": folder_name, "files": names, "times": {"started": time.time()}, "jobs": {}}
            result["modes"][mode_name] = mode
            write_result(directory, result)
            if mode_name == "sequential":
                await run_sequential(context, folder_page, names, mode, directory, result)
            else:
                await run_staged(context, folder_page, names, mode,
                                 optimize_concurrently=mode_name == "parallel",
                                 directory=directory, result=result)
            mode["times"]["finished"] = time.time()
            write_result(directory, result)
        finally:
            await context.close()
            result["browser"]["closed"] = time.time()
            write_result(directory, result)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--mode", choices=list(FOLDERS), required=True)
    parser.add_argument("--limit", type=int, default=8)
    parser.add_argument("--folder", help="Use one specified copied folder for a single mode")
    parser.add_argument("--only-file", help="Diagnose one workbook without touching its neighbors")
    args = parser.parse_args()
    directory = args.output.resolve()
    folder_url = (directory / "onedrive-share-url.txt").read_text(encoding="utf-8").strip()
    sys.path.insert(0, str(directory / "python-packages"))
    asyncio.run(run_experiment(directory, folder_url, args.mode, args.limit,
                               args.folder, args.only_file))


if __name__ == "__main__":
    main()
