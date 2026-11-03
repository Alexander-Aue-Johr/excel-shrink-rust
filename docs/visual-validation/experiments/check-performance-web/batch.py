"""Measure eight Excel for the web optimizations sequentially and in eight tabs."""

import argparse
import asyncio
import json
from pathlib import Path
import re
import shutil
import sys
import time
from urllib.parse import urlsplit


HOME = "https://excel.cloud.microsoft/"
OPTIMIZE = re.compile(r"^(Optimize all|Alle optimieren)$", re.I)
ALREADY_CLEAN = re.compile(r"Keine Optimierung erforderlich|No optimization required", re.I)
OPTIMIZED = re.compile(r"Arbeitsmappe optimiert|Workbook optimized", re.I)


def write_result(path: Path, result: dict) -> None:
    """Keep completed job observations even if a later browser operation fails."""
    path.write_text(json.dumps(result, indent=2, ensure_ascii=True) + "\n", encoding="utf-8")


def mark(job: dict, phase: str) -> None:
    """Record wall time so intervals from separate tabs can be compared."""
    job["times"][phase] = time.time()
    print(json.dumps({"file": job["file"], "phase": phase, "at": job["times"][phase]}), flush=True)


async def choose_consented_account(page, directory: Path) -> None:
    """Use only the signed-in account already approved for this experiment."""
    await page.get_by_text(re.compile(r"Angemeldet|Signed in", re.I)).first.wait_for(
        state="visible", timeout=30_000)
    consent = json.loads((directory / "account-consent.json").read_text())
    address = await page.evaluate("""() => {
      const email = /[A-Z0-9._%+-]+@[A-Z0-9.-]+\\.[A-Z]{2,}/i;
      const tile = [...document.querySelectorAll('button, [role="button"], div')]
        .filter(element => /Angemeldet|Signed in/i.test(element.textContent || '')
          && email.test(element.textContent || ''))
        .sort((a, b) => a.textContent.length - b.textContent.length)[0];
      return tile?.textContent.match(email)?.[0] || null;
    }""")
    if not address:
        raise RuntimeError("No signed-in account tile appeared in headless mode")
    import hashlib
    import hmac
    digest = hmac.new(bytes.fromhex(consent["secret"]), address.lower().encode(), hashlib.sha256).hexdigest()
    if not hmac.compare_digest(digest, consent["fingerprint"]):
        raise RuntimeError("Signed-in account differs from the consented account")
    await page.get_by_text(address, exact=True).first.click()
    await page.wait_for_url(re.compile(r"^https://excel\.cloud\.microsoft/"), timeout=180_000)


async def upload_workbook(page, source: Path, directory: Path, job: dict) -> None:
    """Retry the file chooser after any per-tab Microsoft sign-in redirect."""
    from playwright.async_api import TimeoutError as PlaywrightTimeout

    for attempt in range(3):
        if attempt == 0:
            await page.goto(HOME, wait_until="domcontentloaded")
            upload = page.locator('[data-testid="0301"]')
        else:
            upload = page.get_by_role("button", name="Datei hochladen")
        await upload.wait_for(state="visible", timeout=120_000)
        mark(job, "home_ready" if attempt == 0 else "retry_ready")
        try:
            async with page.expect_file_chooser(timeout=8_000) as chooser:
                await upload.click()
            file_chooser = await chooser.value
            await file_chooser.set_files(source)
            mark(job, "upload_sent")
            await page.wait_for_url(re.compile(r"/open/"), timeout=600_000)
            mark(job, "workbook_opened")
            return
        except PlaywrightTimeout:
            if urlsplit(page.url).hostname not in {"login.microsoftonline.com", "login.live.com"}:
                raise
        mark(job, "login_required")
        await choose_consented_account(page, directory)
        mark(job, "login_completed")
    raise RuntimeError("Upload did not open a workbook after three attempts")


async def find_editor(page):
    """Wait for the ribbon and for Excel's loading screen to disappear."""
    deadline = time.monotonic() + 600
    while time.monotonic() < deadline:
        for frame in page.frames:
            try:
                if await frame.locator("#Review:visible").count() and "Wird geladen" not in (
                    await frame.locator("body").inner_text())[:250]:
                    return frame
            except Exception:
                continue
        await asyncio.sleep(0.5)
    raise RuntimeError("Excel editor did not load within ten minutes")


async def find_performance_pane(page):
    """Recognize both the actionable and already-clean Performance states."""
    deadline = time.monotonic() + 120
    while time.monotonic() < deadline:
        for frame in page.frames:
            if "resources.office.net" not in frame.url:
                continue
            if await frame.get_by_role("button", name=OPTIMIZE).count() or await frame.get_by_text(ALREADY_CLEAN).count():
                return frame
        await asyncio.sleep(0.5)
    raise RuntimeError("Performance pane did not become ready")


async def open_review_ribbon(editor) -> None:
    """Retry clicks ignored while Excel is still finishing workbook startup."""
    review = editor.locator("#Review:visible").first
    deadline = time.monotonic() + 120
    while time.monotonic() < deadline:
        if await review.get_attribute("aria-selected") == "true":
            return
        await review.click(timeout=120_000)
        for _ in range(10):
            if await review.get_attribute("aria-selected") == "true":
                return
            await asyncio.sleep(0.5)
    raise RuntimeError("Review tab never became selected")


async def download_copy(page, editor, destination: Path) -> None:
    """Save the downloaded artifact after the cloud operation has finished."""
    await editor.locator("#FileMenuFlyoutLauncher:visible").first.click()
    await editor.locator('[data-unique-id="FileMenuCreateACopySection"]:visible').first.click()
    async with page.expect_download(timeout=600_000) as event:
        await editor.locator('[data-unique-id="DownloadACopy"]:visible').first.click()
    destination.parent.mkdir(parents=True, exist_ok=True)
    download = await event.value
    await download.save_as(destination)


async def run_job(context, source: Path, mode: str, directory: Path, result: dict) -> None:
    """Run one workbook through upload, optimization, and download with phase times."""
    from playwright.async_api import TimeoutError as PlaywrightTimeout

    job = {"file": source.name, "bytes": source.stat().st_size, "times": {}, "status": "running"}
    result[mode][source.name] = job
    page = None
    editor = None
    try:
        mark(job, "tab_start")
        page = await context.new_page()
        mark(job, "tab_ready")
        await upload_workbook(page, source, directory, job)
        editor = await find_editor(page)
        mark(job, "editor_ready")
        await open_review_ribbon(editor)
        mark(job, "review_opened")
        performance = editor.locator("#ShowOptimizeWorkbookTaskpane:visible").first
        try:
            await performance.wait_for(state="visible", timeout=60_000)
        except PlaywrightTimeout:
            job["status"] = "skipped"
            job["reason"] = "Performance command not visible in the headless ribbon"
            job["performance_dom_count"] = await editor.locator("#ShowOptimizeWorkbookTaskpane").count()
            job["review_tab_count"] = await editor.locator("#Review:visible").count()
            job["editor_diagnostic"] = (await editor.locator("body").inner_text())[-1200:]
            job["button_labels"] = await editor.locator("button").evaluate_all("""elements =>
              elements.filter(element => element.getBoundingClientRect().width > 0)
                .map(element => ({text: element.textContent.trim().slice(0, 60),
                  title: element.getAttribute('title'), label: element.getAttribute('aria-label')}))
                .filter(item => item.text || item.title || item.label)
                .slice(0, 100)""")
            screenshot = directory / "diagnostics" / f"{source.stem}.png"
            screenshot.parent.mkdir(parents=True, exist_ok=True)
            await page.screenshot(path=str(screenshot))
            mark(job, "skipped")
            return
        await performance.click()
        pane = await find_performance_pane(page)
        mark(job, "performance_ready")
        if await pane.get_by_text(ALREADY_CLEAN).count():
            job["optimization"] = "already_clean"
        else:
            mark(job, "optimization_started")
            await pane.get_by_role("button", name=OPTIMIZE).click()
            await pane.get_by_text(OPTIMIZED).first.wait_for(state="visible", timeout=1_200_000)
            mark(job, "optimization_finished")
            job["optimization"] = "optimized"
        mark(job, "download_started")
        destination = directory / "batch-downloads" / result["run_id"] / mode / source.name
        await download_copy(page, editor, destination)
        mark(job, "download_finished")
        job["download"] = str(destination)
        job["download_bytes"] = destination.stat().st_size
        job["status"] = "completed"
    except Exception as error:
        job["status"] = "failed"
        job["error"] = str(error)[:500]
        if editor is not None:
            job["editor_diagnostic"] = (await editor.locator("body").inner_text())[-1200:]
        mark(job, "failed")
    finally:
        if page is not None:
            await page.close()
        write_result(directory / "batch-result.json", result)


async def run_batches(directory: Path, sources: list[Path], sequential_only: bool) -> None:
    """Use one browser for both modes and keep job start/end times comparable."""
    from playwright.async_api import async_playwright

    run_id = time.strftime("%Y%m%d-%H%M%S")
    result = {"run_id": run_id, "browser": {}, "sequential": {}, "parallel": {}}
    result_path = directory / "batch-result.json"
    if result_path.exists():
        shutil.copyfile(result_path, directory / f"batch-result-before-{run_id}.json")
    result["browser"]["start"] = time.time()
    async with async_playwright() as playwright:
        context = await playwright.chromium.launch_persistent_context(
            directory / "edge-profile", channel="msedge", headless=True,
            accept_downloads=True, chromium_sandbox=True,
            viewport={"width": 1920, "height": 1080},
        )
        try:
            result["browser"]["ready"] = time.time()
            for source in sources:
                await run_job(context, source, "sequential", directory, result)
            result["browser"]["sequential_finished"] = time.time()
            write_result(result_path, result)
            eligible = [source for source in sources
                        if result["sequential"][source.name]["status"] == "completed"]
            if not sequential_only:
                await asyncio.gather(*(run_job(context, source, "parallel", directory, result) for source in eligible))
                result["browser"]["parallel_finished"] = time.time()
        finally:
            await context.close()
            result["browser"]["closed"] = time.time()
            write_result(result_path, result)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--limit", type=int, default=8)
    parser.add_argument("--only", type=str)
    parser.add_argument("--sequential-only", action="store_true")
    args = parser.parse_args()
    directory = args.output.resolve()
    sources = sorted((directory / "destatis-input").glob("*.xlsx"), key=lambda p: p.stat().st_size)
    if len(sources) != 8:
        raise RuntimeError(f"Expected eight local Destatis XLSX files, found {len(sources)}")
    if args.only:
        sources = [source for source in sources if source.name == args.only]
        if not sources:
            raise RuntimeError(f"No Destatis file named {args.only}")
    sys.path.insert(0, str(directory / "python-packages"))
    asyncio.run(run_batches(directory, sources[:args.limit], args.sequential_only))


if __name__ == "__main__":
    main()
