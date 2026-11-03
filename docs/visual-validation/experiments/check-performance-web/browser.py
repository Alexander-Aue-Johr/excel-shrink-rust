"""Drive the synthetic Check Performance probe in Excel for the web."""

import hashlib
import hmac
import json
from pathlib import Path
import re
import secrets
import time
from urllib.parse import urlsplit
from uuid import uuid4

from playwright.sync_api import TimeoutError as PlaywrightTimeout
from playwright.sync_api import sync_playwright


HOME = "https://excel.cloud.microsoft/"
def account_fingerprint(secret: str, address: str) -> str:
    """Bind a remembered choice to one email without storing that email in JSON."""
    return hmac.new(bytes.fromhex(secret), address.lower().encode(), hashlib.sha256).hexdigest()


def signed_in_accounts(page) -> list[dict]:
    """Read only account tiles explicitly marked signed in on Microsoft's login page."""
    return page.evaluate("""() => {
      const accounts = new Map();
      const email = /[A-Z0-9._%+-]+@[A-Z0-9.-]+\\.[A-Z]{2,}/i;
      for (const element of document.querySelectorAll('button, [role="button"], div')) {
        const label = element.textContent.trim();
        if (label.length > 400 || !/Angemeldet|Signed in/i.test(label)) continue;
        const address = label.match(email)?.[0];
        if (address && (!accounts.has(address) || label.length < accounts.get(address).length)) {
          accounts.set(address, label);
        }
      }
      return [...accounts].map(([address, label]) => ({address, label}));
    }""")


def ask_account_choice(page, accounts: list[dict]) -> int:
    """Show a small browser dialog for selecting a signed-in account."""
    page.evaluate("""labels => {
      window.__excelShrinkAccountChoice = null;
      const cover = document.createElement('div');
      cover.style.cssText = 'position:fixed;inset:0;z-index:2147483647;background:#0009;display:grid;place-items:center;font:16px sans-serif';
      const panel = document.createElement('section');
      panel.style.cssText = 'background:white;color:#222;padding:24px;border-radius:12px;max-width:540px;box-shadow:0 12px 40px #0006';
      const heading = document.createElement('h2');
      heading.textContent = 'May I use one of these signed-in accounts for this and future experiment logins?';
      panel.append(heading);
      labels.forEach((label, index) => {
        const button = document.createElement('button');
        button.textContent = label;
        button.style.cssText = 'display:block;width:100%;margin:10px 0;padding:12px;text-align:left';
        button.onclick = () => { window.__excelShrinkAccountChoice = index; cover.remove(); };
        panel.append(button);
      });
      const manual = document.createElement('button');
      manual.textContent = 'No, I will sign in myself';
      manual.onclick = () => { window.__excelShrinkAccountChoice = -1; cover.remove(); };
      panel.append(manual);
      cover.append(panel);
      document.body.append(cover);
    }""", [account["label"] for account in accounts])
    page.wait_for_function("window.__excelShrinkAccountChoice !== null", timeout=600_000)
    return page.evaluate("window.__excelShrinkAccountChoice")


def choose_login_account(page, directory: Path, headless: bool = False) -> None:
    """Reuse a consented tile or obtain consent; leave other sign-in steps to the user."""
    deadline = time.monotonic() + 30
    accounts = []
    while time.monotonic() < deadline:
        if urlsplit(page.url).hostname == "excel.cloud.microsoft":
            return
        accounts = signed_in_accounts(page)
        if accounts:
            break
        page.wait_for_timeout(500)
    if not accounts:
        return

    consent_path = directory / "account-consent.json"
    consent = json.loads(consent_path.read_text()) if consent_path.exists() else None
    remembered = next((index for index, account in enumerate(accounts)
                       if consent and hmac.compare_digest(
                           account_fingerprint(consent["secret"], account["address"]),
                           consent["fingerprint"])), None)
    if remembered is None and headless:
        raise RuntimeError("No consented signed-in account matches; run visibly to choose one")
    choice = remembered if remembered is not None else ask_account_choice(page, accounts)
    if choice < 0:
        consent_path.unlink(missing_ok=True)
        return
    if remembered is None:
        secret = secrets.token_hex(32)
        consent_path.write_text(json.dumps({
            "secret": secret,
            "fingerprint": account_fingerprint(secret, accounts[choice]["address"]),
        }))
    page.get_by_text(accounts[choice]["address"], exact=True).first.click(timeout=15_000)


def submit_upload(page, fixture: Path, retry: bool) -> bool:
    """Set the file through Playwright's chooser so no Windows Open dialog appears."""
    if retry:
        upload = page.get_by_role("button", name="Datei hochladen")
        upload.wait_for(state="visible", timeout=60_000)
    else:
        page.goto(HOME, wait_until="domcontentloaded")
        upload = page.locator('[data-testid="0301"]')
        upload.wait_for(state="visible", timeout=120_000)
    try:
        with page.expect_file_chooser(timeout=30_000) as chooser:
            upload.click(timeout=30_000)
        chooser.value.set_files(fixture)
        return True
    except PlaywrightTimeout:
        if urlsplit(page.url).hostname in {"login.microsoftonline.com", "login.live.com"}:
            return False
        raise


def upload_fixture(page, fixture: Path, directory: Path, headless: bool = False) -> None:
    """Upload once, then repeat from the recovery prompt if sign-in interrupted it."""
    for attempt in range(3):
        submitted = submit_upload(page, fixture, retry=attempt > 0)
        if submitted:
            try:
                page.wait_for_url(re.compile(r"/open/"), timeout=30_000)
                return
            except PlaywrightTimeout:
                pass
        if urlsplit(page.url).hostname in {"login.microsoftonline.com", "login.live.com"}:
            choose_login_account(page, directory, headless=headless)
            print("Complete any remaining Microsoft sign-in steps in Edge.", flush=True)
            page.wait_for_url(re.compile(r"^https://excel\.cloud\.microsoft/"), timeout=600_000)
        if "/open/" in urlsplit(page.url).path:
            return
    raise RuntimeError("Upload did not open the workbook after three attempts")


def excel_editor(page):
    """Find the loaded Excel ribbon in any frame of the opened workbook tab."""
    deadline = time.monotonic() + 600
    while time.monotonic() < deadline:
        for frame in page.frames:
            try:
                if (frame.locator("#Review:visible").count()
                        and "Wird geladen" not in frame.locator("body").inner_text()[:250]):
                    return frame
            except Exception:
                continue
        page.wait_for_timeout(500)
    raise RuntimeError("The opened workbook has no Review ribbon in any frame")


def optimize_workbook(page, editor) -> str:
    """Optimize when offered, or report Excel's explicit already-clean message."""
    review = editor.locator("#Review:visible").first
    deadline = time.monotonic() + 120
    while time.monotonic() < deadline and review.get_attribute("aria-selected") != "true":
        review.click(timeout=120_000)
        page.wait_for_timeout(500)
    if review.get_attribute("aria-selected") != "true":
        raise RuntimeError("Review tab never became selected")
    editor.locator("#ShowOptimizeWorkbookTaskpane:visible").first.click(timeout=30_000)
    deadline = time.monotonic() + 60
    pane = None
    while time.monotonic() < deadline:
        pane = next((frame for frame in page.frames
                     if "resources.office.net" in frame.url
                     and (frame.get_by_role("button", name=re.compile(r"^(Optimize all|Alle optimieren)$", re.I)).count()
                          or frame.get_by_text(re.compile(r"Keine Optimierung erforderlich|No optimization required", re.I)).count())), None)
        if pane:
            break
        page.wait_for_timeout(500)
    if pane is None:
        raise RuntimeError("The Performance pane showed neither Optimize all nor an already-clean result")
    if pane.get_by_text(re.compile(r"Keine Optimierung erforderlich|No optimization required", re.I)).count():
        return "already_clean"
    pane.get_by_role("button", name=re.compile(r"^(Optimize all|Alle optimieren)$", re.I)).click(timeout=120_000)
    pane.get_by_text(re.compile(r"Workbook optimized|Arbeitsmappe optimiert", re.I)).first.wait_for(
        state="visible", timeout=180_000)
    return "optimized"


def download_copy(page, editor, directory: Path) -> Path:
    """Save the actual browser download before its temporary context closes."""
    editor.locator("#FileMenuFlyoutLauncher").click(timeout=30_000)
    editor.locator('[data-unique-id="FileMenuCreateACopySection"]').click(timeout=30_000)
    with page.expect_download(timeout=180_000) as download_event:
        editor.locator('[data-unique-id="DownloadACopy"]').click(timeout=30_000)
    destination = directory / "downloads" / f"optimized-{uuid4().hex}.xlsx"
    destination.parent.mkdir(parents=True, exist_ok=True)
    download_event.value.save_as(destination)
    return destination


def run_experiment(directory: Path, fixture: Path, headless: bool = False) -> None:
    """Open a private Edge profile, optimize the fixture, and retain a bounded result."""
    result = {"status": "running", "started_at": time.time()}
    result_path = directory / "browser-result.json"
    try:
        with sync_playwright() as playwright:
            context = playwright.chromium.launch_persistent_context(
                directory / "edge-profile", channel="msedge", headless=headless,
                accept_downloads=True, chromium_sandbox=True,
                viewport={"width": 1920, "height": 1080},
            )
            try:
                page = context.pages[0] if context.pages else context.new_page()
                result["phase"] = "upload"
                upload_fixture(page, fixture, directory, headless=headless)
                result["phase"] = "find_editor"
                editor = excel_editor(page)
                result["phase"] = "optimize"
                result["optimization"] = optimize_workbook(page, editor)
                result["phase"] = "download"
                download = download_copy(page, editor, directory)
                result.update(status="downloaded", download=str(download))
            finally:
                context.close()
    except Exception as error:
        result.update(status="failed", error=str(error)[:1000])
        raise
    finally:
        result["finished_at"] = time.time()
        result_path.write_text(json.dumps(result, indent=2) + "\n")
