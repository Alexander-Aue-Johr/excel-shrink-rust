#!/usr/bin/env python3
"""Run Excel's Check Performance on a shared OneDrive folder, one file at a time.

Completed files are recorded atomically. Re-running the command skips them and
reopens interrupted files to ask Excel whether optimization is still needed.
"""

import argparse
import asyncio
from dataclasses import dataclass, field
from datetime import datetime, timezone
from enum import StrEnum
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import statistics
import sys
import time
from urllib.parse import urlsplit

class JobState(StrEnum):
    """Durable stages of one workbook's Check Performance attempt."""

    PENDING = "pending"
    OPENING = "opening"
    CHECKING = "checking"
    OPTIMIZING = "optimizing"
    SAVING = "saving"
    COMPLETED = "completed"
    SKIPPED = "skipped"
    FAILED = "failed"


ALLOWED_TRANSITIONS = {
    JobState.PENDING: {JobState.OPENING},
    JobState.OPENING: {JobState.CHECKING, JobState.SKIPPED, JobState.FAILED},
    JobState.CHECKING: {JobState.OPTIMIZING, JobState.SAVING, JobState.COMPLETED,
                        JobState.SKIPPED, JobState.FAILED},
    JobState.OPTIMIZING: {JobState.SAVING, JobState.SKIPPED, JobState.FAILED},
    JobState.SAVING: {JobState.COMPLETED, JobState.SKIPPED, JobState.FAILED},
    JobState.FAILED: {JobState.OPENING, JobState.SKIPPED},
    JobState.COMPLETED: set(),
    JobState.SKIPPED: set(),
}

RESTARTABLE_STATES = {
    JobState.PENDING, JobState.OPENING, JobState.CHECKING,
    JobState.OPTIMIZING, JobState.SAVING, JobState.FAILED, JobState.SKIPPED,
}


def validated_state(previous: str, next_state: JobState) -> str:
    """Reject invalid lifecycle changes, including retries of completed files."""
    current = JobState.OPENING if previous == "running" else JobState(previous)
    if next_state != current and next_state not in ALLOWED_TRANSITIONS[current]:
        raise ValueError(f"Invalid Check Performance transition: {current} -> {next_state}")
    return next_state.value


def utc_now() -> str:
    """Timestamp progress records independently of the browser's locale."""
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


class ProgressStore:
    """Own one folder's file states, attempts, and timed events."""

    def __init__(self, directory: Path, folder_id: str) -> None:
        directory.mkdir(parents=True, exist_ok=True)
        self.path = directory / "progress.sqlite3"
        self.connection = sqlite3.connect(self.path, timeout=30)
        self.connection.execute("PRAGMA journal_mode=WAL")
        self.connection.execute("PRAGMA synchronous=FULL")
        self._initialize(folder_id, directory / "progress.json")

    def __enter__(self):
        return self

    def __exit__(self, _type, _value, _traceback) -> None:
        self.connection.close()

    def _initialize(self, folder_id: str, legacy_path: Path) -> None:
        """Create the schema and import prior JSON progress once."""
        with self.connection:
            self.connection.executescript("""
                CREATE TABLE IF NOT EXISTS metadata (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS files (
                    name TEXT PRIMARY KEY,
                    status TEXT NOT NULL,
                    outcome TEXT,
                    discovered_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS attempts (
                    id INTEGER PRIMARY KEY,
                    file_name TEXT NOT NULL REFERENCES files(name),
                    started_at TEXT NOT NULL,
                    finished_at TEXT,
                    error TEXT
                );
                CREATE TABLE IF NOT EXISTS events (
                    id INTEGER PRIMARY KEY,
                    attempt_id INTEGER NOT NULL REFERENCES attempts(id),
                    phase TEXT NOT NULL,
                    at TEXT NOT NULL,
                    elapsed_seconds REAL NOT NULL,
                    since_previous_seconds REAL NOT NULL,
                    memory_json TEXT
                );
            """)
            existing = self.connection.execute(
                "SELECT value FROM metadata WHERE key = 'folder_id'").fetchone()
            if existing and existing[0] != folder_id:
                raise ValueError("Progress database belongs to a different OneDrive folder")
            if existing is None:
                if legacy_path.exists():
                    self._import_legacy_json(legacy_path, folder_id)
                else:
                    self.connection.execute(
                        "INSERT INTO metadata (key, value) VALUES ('folder_id', ?)", (folder_id,))

    def _import_legacy_json(self, path: Path, folder_id: str) -> None:
        """Preserve every completed file and observed phase from the JSON runner."""
        legacy = json.loads(path.read_text(encoding="utf-8"))
        if legacy.get("folder_id") != folder_id:
            raise ValueError("Legacy progress belongs to a different OneDrive folder")
        self.connection.execute(
            "INSERT INTO metadata (key, value) VALUES ('folder_id', ?)", (folder_id,))
        for name, entry in legacy["files"].items():
            status = JobState.OPENING.value if entry["status"] == "running" else entry["status"]
            self.connection.execute(
                "INSERT INTO files (name, status, outcome, discovered_at) VALUES (?, ?, ?, ?)",
                (name, status, entry.get("outcome"), legacy.get("discovered_at", legacy["created_at"])))
            for attempt in entry.get("attempts", []):
                cursor = self.connection.execute(
                    "INSERT INTO attempts (file_name, started_at, finished_at, error) VALUES (?, ?, ?, ?)",
                    (name, attempt["started_at"], attempt.get("finished_at"), attempt.get("error")))
                for event in attempt.get("events", []):
                    self._insert_event(cursor.lastrowid, event)

    def _insert_event(self, attempt_id: int, event: dict) -> None:
        """Write one event inside the caller's transaction."""
        memory = event.get("memory")
        self.connection.execute("""
            INSERT INTO events (attempt_id, phase, at, elapsed_seconds,
                                since_previous_seconds, memory_json)
            VALUES (?, ?, ?, ?, ?, ?)
        """, (attempt_id, event["phase"], event["at"], event["elapsed_seconds"],
              event["since_previous_seconds"], json.dumps(memory) if memory is not None else None))

    def discover(self, names: list[str]) -> list[str]:
        """Add newly uploaded files while preserving the known processing order."""
        with self.connection:
            self.connection.executemany(
                "INSERT OR IGNORE INTO files (name, status, discovered_at) VALUES (?, ?, ?)",
                ((name, JobState.PENDING.value, utc_now()) for name in names))
        return [row[0] for row in self.connection.execute("SELECT name FROM files ORDER BY rowid")]

    def status(self, name: str) -> str:
        """Read one file's durable state."""
        row = self.connection.execute("SELECT status FROM files WHERE name = ?", (name,)).fetchone()
        if row is None:
            raise KeyError(name)
        return row[0]

    def outcome(self, name: str) -> str | None:
        """Read the reason recorded for a completed or skipped workbook."""
        row = self.connection.execute("SELECT outcome FROM files WHERE name = ?", (name,)).fetchone()
        if row is None:
            raise KeyError(name)
        return row[0]

    def names(self) -> list[str]:
        """Return the files already discovered for this OneDrive folder."""
        return [row[0] for row in self.connection.execute("SELECT name FROM files ORDER BY rowid")]

    def unfinished_names(self, retry_skipped: bool = False) -> set[str]:
        """Select incomplete work and optionally retry previously unavailable files."""
        rows = self.connection.execute(
            "SELECT name, status FROM files WHERE status != ?", (JobState.COMPLETED.value,))
        return {name for name, status in rows
                if retry_skipped or status != JobState.SKIPPED.value}

    def transition(self, name: str, next_state: JobState) -> None:
        """Persist a validated state change for one workbook."""
        with self.connection:
            status = validated_state(self.status(name), next_state)
            self.connection.execute("UPDATE files SET status = ? WHERE name = ?", (status, name))

    def start_attempt(self, name: str) -> int:
        """Restart unfinished work after a crash without retrying completed files."""
        with self.connection:
            current = JobState(self.status(name))
            if current not in RESTARTABLE_STATES:
                raise ValueError(f"Cannot restart Check Performance file in state {current}")
            self.connection.execute("""
                UPDATE attempts SET finished_at = ?, error = ?
                WHERE file_name = ? AND finished_at IS NULL
            """, (utc_now(), "Interrupted before restart", name))
            self.connection.execute(
                "UPDATE files SET status = ? WHERE name = ?", (JobState.OPENING.value, name))
            cursor = self.connection.execute(
                "INSERT INTO attempts (file_name, started_at) VALUES (?, ?)", (name, utc_now()))
        return cursor.lastrowid

    def record_event(self, attempt_id: int, event: dict) -> None:
        """Commit one phase without rewriting other workbooks' history."""
        with self.connection:
            self._insert_event(attempt_id, event)

    def finish(self, name: str, attempt_id: int, outcome: str) -> None:
        """Commit an outcome and completion together."""
        with self.connection:
            status = validated_state(self.status(name), JobState.COMPLETED)
            self.connection.execute(
                "UPDATE files SET status = ?, outcome = ? WHERE name = ?", (status, outcome, name))
            self.connection.execute(
                "UPDATE attempts SET finished_at = ? WHERE id = ?", (utc_now(), attempt_id))

    def skip(self, name: str, attempt_id: int, reason: str,
             error: str | None = None) -> None:
        """Record an unverified workbook and retain the failure detail, if any."""
        with self.connection:
            status = validated_state(self.status(name), JobState.SKIPPED)
            self.connection.execute(
                "UPDATE files SET status = ?, outcome = ? WHERE name = ?", (status, reason, name))
            self.connection.execute(
                "UPDATE attempts SET finished_at = ?, error = ? WHERE id = ?",
                (utc_now(), error[:500] if error else None, attempt_id))

    def fail(self, name: str, attempt_id: int, error: str) -> None:
        """Keep an unsuccessful file retryable on the next invocation."""
        with self.connection:
            status = validated_state(self.status(name), JobState.FAILED)
            self.connection.execute("UPDATE files SET status = ? WHERE name = ?", (status, name))
            self.connection.execute(
                "UPDATE attempts SET finished_at = ?, error = ? WHERE id = ?",
                (utc_now(), error[:500], attempt_id))

    def skip_failed(self, name: str, reason: str) -> None:
        """Stop retrying one failed workbook while preserving its attempt errors."""
        with self.connection:
            status = validated_state(self.status(name), JobState.SKIPPED)
            self.connection.execute(
                "UPDATE files SET status = ?, outcome = ? WHERE name = ?",
                (status, reason, name))

    def counts(self) -> tuple[int, int]:
        """Return processed and known file counts for a concise status line."""
        processed = self.connection.execute(
            "SELECT COUNT(*) FROM files WHERE status IN (?, ?)",
            (JobState.COMPLETED.value, JobState.SKIPPED.value)).fetchone()[0]
        total = self.connection.execute("SELECT COUNT(*) FROM files").fetchone()[0]
        return processed, total

    def skipped_count(self) -> int:
        """Keep unavailable files visible in the run summary."""
        return self.connection.execute(
            "SELECT COUNT(*) FROM files WHERE status = ?", (JobState.SKIPPED.value,)).fetchone()[0]


OPTIMIZE = re.compile(r"^(Optimize all|Alle optimieren)$", re.I)
ALREADY_CLEAN = re.compile(
    r"Keine Optimierung (?:erforderlich|nötig)|No optimization (?:required|needed)", re.I)
CHECK_AGAIN = re.compile(r"^(Check again|Erneut prüfen)$", re.I)
INVALID_OBJECTS = re.compile(
    r"many\s+invalid\s+objects|viele[nr]?\s+ungültige[nr]?\s+Objekte", re.I)
REMOVE_AND_EDIT = re.compile(r"^(Remove and Edit|Entfernen und bearbeiten)$", re.I)


def folder_id(url: str) -> str:
    """Identify a share without persisting its editable URL or query token."""
    parsed = urlsplit(url)
    if parsed.scheme != "https" or parsed.hostname not in {"1drv.ms", "onedrive.live.com"}:
        raise ValueError("Expected an HTTPS OneDrive folder link")
    return hashlib.sha256(f"{parsed.netloc}{parsed.path}".encode()).hexdigest()[:20]


@dataclass
class AttemptClock:
    """Keep elapsed-time calculations local to one live browser attempt."""

    id: int
    started: float
    previous_elapsed: float = 0
    phase_elapsed: dict[str, float] = field(default_factory=dict)
    display: "ConsoleProgress | None" = None


class ConsoleProgress:
    """Show completed-file timings and one replaceable live status line on a TTY."""

    PHASES = ("editor_ready", "review_ready", "performance_ready",
              "optimization_finished", "cloud_saved")

    def __init__(self, progress: ProgressStore) -> None:
        self.interactive = sys.stdout.isatty()
        self.processed, self.total = progress.counts()
        self.skipped = progress.skipped_count()
        self.durations = [row[0] for row in progress.connection.execute("""
            SELECT MAX(events.elapsed_seconds)
            FROM attempts
            JOIN files ON files.name = attempts.file_name
            JOIN events ON events.attempt_id = attempts.id
            WHERE files.status = 'completed' AND attempts.error IS NULL
              AND attempts.finished_at IS NOT NULL
            GROUP BY attempts.id
            ORDER BY attempts.id DESC
            LIMIT 20
        """).fetchall() if row[0] is not None]
        self.name = ""
        self.stage = "waiting"
        self.started = time.monotonic()
        self.name_width = max(16, min(40, shutil.get_terminal_size().columns - 55))
        if self.interactive:
            print(f"{'File':<{self.name_width}}      O      R      P      X      S   Total  Result")
            print("O=open  R=Review  P=Performance  X=Optimize  S=cloud save; times in seconds")

    def begin(self, name: str, previous_status: str) -> None:
        if previous_status == JobState.SKIPPED.value:
            self.processed -= 1
            self.skipped -= 1
        self.name = name
        self.stage = "opening"
        self.started = time.monotonic()
        self.refresh()

    def phase(self, name: str, phase: str, elapsed: float) -> None:
        self.stage = phase.replace("_", " ")
        if self.interactive:
            self.refresh()
        else:
            print(f"{name}: {phase} ({elapsed:.1f} s)", flush=True)

    def finish(self, name: str, attempt: AttemptClock, outcome: str, error: str | None) -> None:
        if not self.interactive:
            if error:
                print(f"{name}: {error}", file=sys.stderr, flush=True)
            return
        self._clear_live_line()
        elapsed = attempt.phase_elapsed
        boundaries = [elapsed.get(phase) for phase in self.PHASES]
        times = []
        previous = 0.0
        for boundary in boundaries:
            times.append(f"{boundary - previous:6.1f}" if boundary is not None else "     -")
            if boundary is not None:
                previous = boundary
        total = time.monotonic() - attempt.started
        result = "failed" if error else outcome
        label = name if len(name) <= self.name_width else name[:self.name_width - 1] + "…"
        print(f"{label:<{self.name_width}} {' '.join(times)} {total:7.1f}  {result}")
        if error:
            print(f"  {error}")
        else:
            self.processed += 1
            if outcome == "performance_unavailable":
                self.skipped += 1
            else:
                self.durations = ([total] + self.durations)[:20]
        self.name = ""
        self.refresh()

    def refresh(self) -> None:
        if not self.interactive:
            return
        self._clear_live_line()
        remaining = self.total - self.processed
        estimate = f"~{format_duration(remaining * statistics.median(self.durations))}" if self.durations else "unknown"
        current = f" | {self.name}: {self.stage} ({format_duration(time.monotonic() - self.started)})" if self.name else ""
        line = (f"Processed {self.processed}/{self.total} | skipped {self.skipped} | "
                f"remaining {remaining} | ETA {estimate}{current}")
        print(line[:shutil.get_terminal_size().columns - 1], end="", flush=True)

    def close(self) -> None:
        if self.interactive:
            self._clear_live_line()

    def _clear_live_line(self) -> None:
        print("\r\033[2K", end="", flush=True)


def format_duration(seconds: float) -> str:
    """Render a compact duration for the live progress line."""
    minutes, seconds = divmod(round(seconds), 60)
    hours, minutes = divmod(minutes, 60)
    return f"{hours}h {minutes}m" if hours else f"{minutes}m {seconds}s"


def profile_memory(profile: Path) -> int | None:
    """Sum Edge processes using this isolated profile when psutil is installed."""
    try:
        import psutil
    except ImportError:
        return None
    profile_path = str(profile.resolve()).lower()
    total = 0
    found = False
    for process in psutil.process_iter(["cmdline", "memory_info"]):
        try:
            command = " ".join(process.info["cmdline"] or []).lower()
            browser_names = {"msedge.exe", "chrome.exe", "chromium.exe", "msedge", "chrome", "chromium"}
            if profile_path in command and process.name().lower() in browser_names:
                total += process.info["memory_info"].rss
                found = True
        except (psutil.AccessDenied, psutil.NoSuchProcess):
            continue
    return total if found else None


async def sample_tab_memory(page, profile: Path, editor=None) -> dict:
    """Read the Excel frame's JS heap and the whole profile's physical working set."""
    sample = {
        "js_heap_bytes": None,
        "js_heap_scope": "excel_frame" if editor is not None else "page",
        "browser_profile_working_set_bytes": profile_memory(profile),
    }
    try:
        session = await page.context.new_cdp_session(editor or page)
        await session.send("Performance.enable")
        metrics = await session.send("Performance.getMetrics")
        sample["js_heap_bytes"] = next(
            (int(metric["value"]) for metric in metrics["metrics"]
             if metric["name"] == "JSHeapUsedSize"), None)
        await session.detach()
    except Exception as error:
        print(f"Folder listing failed ({type(error).__name__}: {error}); "
              "reloading the shared folder once.", file=sys.stderr)
    return sample


def record_phase(progress: ProgressStore, attempt: AttemptClock, name: str,
                 phase: str, sample: dict | None = None) -> None:
    """Persist one timed event without rewriting other files' progress."""
    elapsed = round(time.monotonic() - attempt.started, 3)
    event = {
        "phase": phase,
        "at": utc_now(),
        "elapsed_seconds": elapsed,
        "since_previous_seconds": round(elapsed - attempt.previous_elapsed, 3),
    }
    if sample is not None:
        event["memory"] = sample
    progress.record_event(attempt.id, event)
    attempt.previous_elapsed = elapsed
    attempt.phase_elapsed[phase] = elapsed
    if attempt.display is not None:
        attempt.display.phase(name, phase, elapsed)


async def scroll_file_list(page, to_top: bool) -> tuple[int, int]:
    """Move OneDrive's virtualized list without assuming a fixed folder size."""
    return await page.evaluate("""toTop => {
      const item = document.querySelector('[data-automationid="field-LinkFilename"]');
      let element = item?.parentElement;
      while (element && element.scrollHeight <= element.clientHeight + 10) {
        element = element.parentElement;
      }
      if (!element) element = document.scrollingElement;
      element.scrollTop = toTop ? 0 : element.scrollTop + Math.max(400, element.clientHeight * 0.8);
      return [element.scrollTop, element.scrollHeight - element.clientHeight];
    }""", to_top)


async def visible_sign_in(page):
    """Find an explicit sign-in control in OneDrive's or Excel's top bar."""
    controls = (
        page.locator("#O365_MainLink_SigninLink:visible").first,
        page.get_by_role("button", name=re.compile(r"^(Sign in|Anmelden)$", re.I)).first,
        page.get_by_role("link", name=re.compile(r"^(Sign in|Anmelden)$", re.I)).first,
    )
    for control in controls:
        if await control.is_visible():
            return control
    return None


def is_login_page(page) -> bool:
    """Recognize Microsoft's account flow independently of the browser tab."""
    return urlsplit(page.url).hostname in {"login.microsoftonline.com", "login.live.com"}


async def wait_for_signed_in_folder(page, url: str, headed: bool):
    """Wait for Microsoft to return to the shared folder in the same tab."""
    if not headed:
        raise RuntimeError("Microsoft sign-in required; restart with --headed")
    print("Complete the Microsoft sign-in in the browser window.", flush=True)
    deadline = time.monotonic() + 300
    opened_folder = False
    while time.monotonic() < deadline:
        if not is_login_page(page):
            files = page.locator('[data-automationid="field-LinkFilename"]').first
            if await files.is_visible() and await visible_sign_in(page) is None:
                return page
            if (not opened_folder and urlsplit(page.url).hostname in {
                    "onedrive.live.com", "excel.cloud.microsoft", "onedrive.com"}
                    and await visible_sign_in(page) is None):
                await page.goto(url, wait_until="domcontentloaded", timeout=120_000)
                opened_folder = True
        await asyncio.sleep(0.5)
    raise TimeoutError("Microsoft sign-in did not return to the OneDrive folder")


async def open_folder(page, url: str, headed: bool):
    """Load the folder and trigger sign-in when its top bar requests it."""
    await page.goto(url, wait_until="domcontentloaded", timeout=120_000)
    await wait_for_folder_response(page)
    if is_login_page(page):
        page = await wait_for_signed_in_folder(page, url, headed)
    sign_in = await visible_sign_in(page)
    if sign_in is not None:
        if not headed:
            raise RuntimeError("Microsoft sign-in required; restart with --headed")
        href = await sign_in.get_attribute("href")
        if href:
            await page.goto(href, wait_until="domcontentloaded", timeout=120_000)
        else:
            await sign_in.click(force=True, timeout=10_000)
        page = await wait_for_signed_in_folder(page, url, headed)
    await wait_for_folder_rows(page)
    return page


class OneDriveThrottled(Exception):
    """OneDrive redirected the current tab to its request-throttling page."""


class OneDriveRenderingError(OneDriveThrottled):
    """OneDrive failed to render its folder. Workbook progress remains valid."""


async def raise_if_folder_render_failed(page) -> None:
    notice = page.get_by_text(re.compile(
        r"Unbekannter Renderingfehler|Unknown rendering error", re.I)).first
    if await notice.is_visible():
        raise OneDriveRenderingError("OneDrive folder rendering failed. Restart the browser")


async def wait_for_folder_response(page) -> None:
    """Follow OneDrive redirects while waiting for files, sign-in, or throttling."""
    from playwright.async_api import Error as PlaywrightError

    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        if is_login_page(page):
            return
        if page.url.lower().split("#", 1)[0].endswith("/throttle.htm"):
            raise OneDriveThrottled("OneDrive throttled requests; restart the run later")
        try:
            if await page.locator('[data-automationid="field-LinkFilename"]').first.is_visible():
                return
            await raise_if_folder_render_failed(page)
            if await visible_sign_in(page) is not None:
                return
        except PlaywrightError:
            if page.is_closed():
                raise
            # OneDrive replaces its first document after the share-link redirect.
        await asyncio.sleep(0.25)
    parts = urlsplit(page.url)
    try:
        title = await asyncio.wait_for(page.title(), timeout=2)
        body = await asyncio.wait_for(page.locator("body").inner_text(), timeout=2)
    except Exception:
        title, body = "unavailable", "unavailable"
    excerpt = re.sub(r"[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}",
                     "[redacted email]", body[:250], flags=re.I)
    raise TimeoutError(
        f"OneDrive folder did not show files or sign-in: "
        f"{parts.hostname}{parts.path}; title={title!r}; text={excerpt!r}")


async def wait_for_folder_rows(page) -> None:
    """Require a usable listing after login or after returning from a workbook."""
    await wait_for_folder_response(page)
    if await visible_sign_in(page) is not None:
        raise RuntimeError("Microsoft sign-in required for the OneDrive folder")
    await page.locator('[data-automationid="field-LinkFilename"]').first.wait_for(
        state="visible", timeout=60_000)


def discard_restored_tabs(profile: Path) -> None:
    """Remove Chrome's tab history while keeping cookies and other sign-in data."""
    sessions = profile / "Default" / "Sessions"
    if not sessions.is_dir():
        return
    for entry in sessions.iterdir():
        if entry.is_file() and entry.name.startswith(("Session_", "Tabs_")):
            entry.unlink()


SINGLE_TAB_NAVIGATION = """(() => {
  window.open = function (url) {
    if (url) window.top.location.href = url;
    return window;
  };
  document.addEventListener('click', event => {
    if (!(event.target instanceof Element)) return;
    const link = event.target.closest('a[target]');
    if (!link || !link.href || !['_blank', 'undefined'].includes(link.target)) return;
    event.preventDefault();
    window.top.location.href = link.href;
  }, true);
})()"""


async def sort_oldest_first(page) -> None:
    """Keep optimized files moving below older work in OneDrive's folder view."""
    header = page.locator('[role="columnheader"][data-automationid="field-Modified"]')
    await header.wait_for(state="visible", timeout=30_000)
    if await header.get_attribute("aria-sort") == "ascending":
        return
    await header.locator('[role="button"]').click()
    await page.locator('[role="menuitemcheckbox"][data-automationid="sortAsc"]').click(
        timeout=10_000)
    from playwright.async_api import expect
    await expect(header).to_have_attribute("aria-sort", "ascending", timeout=30_000)


async def discover_workbooks(page) -> list[str]:
    """Collect XLSX rows, including rows rendered only after scrolling."""
    names = []
    seen = set()
    unchanged_at_bottom = 0
    for _ in range(5_000):
        visible = await page.locator('[data-automationid="field-LinkFilename"]').all_text_contents()
        old_count = len(seen)
        for name in visible:
            if name.lower().endswith(".xlsx") and name not in seen:
                names.append(name)
                seen.add(name)
        position, bottom = await scroll_file_list(page, to_top=False)
        unchanged_at_bottom = unchanged_at_bottom + 1 if position >= bottom and len(seen) == old_count else 0
        # OneDrive can pause at a temporary bottom while it fetches more rows.
        if unchanged_at_bottom >= 20:
            break
        await asyncio.sleep(0.5)
    else:
        raise RuntimeError("OneDrive file listing did not settle after scrolling")
    await scroll_file_list(page, to_top=True)
    return names


async def next_visible_unfinished(page, unfinished: set[str]):
    """Use the current viewport before scrolling to older unprocessed rows."""
    for pass_number in range(2):
        unchanged_at_bottom = 0
        for _ in range(5_000):
            visible = await page.locator(
                '[role="gridcell"][data-automationid="field-LinkFilename"]').all_text_contents()
            if not visible:
                await raise_if_folder_render_failed(page)
            for cell_text in visible:
                name = cell_text.strip()
                if name in unfinished:
                    return name, page.get_by_text(name, exact=True).first
            position, bottom = await scroll_file_list(page, to_top=False)
            unchanged_at_bottom = unchanged_at_bottom + 1 if position >= bottom else 0
            if unchanged_at_bottom >= 20:
                break
            await asyncio.sleep(0.5 if unchanged_at_bottom else 0.05)
        else:
            raise RuntimeError("OneDrive file listing did not settle while finding pending work")
        if pass_number == 0:
            await scroll_file_list(page, to_top=True)
    return None


async def find_file_row(page, name: str):
    """Continue from the prior row; wrap once if OneDrive reordered the list."""
    for pass_number in range(2):
        bottom_observations = 0
        for _ in range(10_000):
            row = page.get_by_text(name, exact=True).first
            if await row.count():
                return row
            await raise_if_folder_render_failed(page)
            position, bottom = await scroll_file_list(page, to_top=False)
            bottom_observations = bottom_observations + 1 if position >= bottom else 0
            if bottom_observations >= 8:
                break
            await asyncio.sleep(0.25 if bottom_observations else 0.05)
        if pass_number == 0:
            await scroll_file_list(page, to_top=True)
    raise RuntimeError(f"Workbook is no longer visible in the folder: {name}")


async def open_workbook_in_current_tab(context, folder_page, name: str, row=None):
    """Open one OneDrive file in the existing browser tab."""
    if row is None:
        row = await find_file_row(folder_page, name)
    await row.dblclick(force=True, timeout=10_000)
    await folder_page.wait_for_url(re.compile(r"/(?:Doc\.aspx|Throttle\.htm)", re.I),
                                   wait_until="domcontentloaded", timeout=60_000)
    if "/throttle.htm" in folder_page.url.lower():
        raise OneDriveThrottled("OneDrive throttled requests; restart the run later")
    return folder_page


class WorkbookCannotOpen(Exception):
    """Excel explicitly rejected a workbook before Check Performance could run."""


class WorkbookEditRestricted(Exception):
    """Excel for the web cannot edit this rights-managed workbook."""


class WorkbookRepairRequired(Exception):
    """Excel offers only a repaired Reading View or asks to repair the file."""


async def raise_if_workbook_blocked(page) -> None:
    """Preserve the visible Excel notice before any editing or optimization."""
    from playwright.async_api import Error as PlaywrightError

    for frame in page.frames:
        try:
            dialogs = frame.get_by_role("dialog")
            irm = dialogs.filter(has_text=re.compile(
                r"Information Rights Management\s*(?:\(|,)\s*IRM\)", re.I)).first
            if await irm.is_visible():
                raise WorkbookEditRestricted(await irm.inner_text())
            bar = frame.locator("#BusinessBar")
            repaired = bar.get_by_text(re.compile(
                r"^(WORKBOOK REPAIRED|ARBEITSMAPPE REPARIERT)$", re.I)).first
            if await repaired.is_visible():
                raise WorkbookRepairRequired(await bar.inner_text())
            repair_dialog = dialogs.filter(has_text=re.compile(
                r"Repair this workbook\?|Diese Arbeitsmappe reparieren\?", re.I)).first
            if await repair_dialog.is_visible():
                raise WorkbookRepairRequired(await repair_dialog.inner_text())
        except PlaywrightError:
            if not frame.is_detached():
                raise


async def workbook_open_error(page) -> bool:
    """Find Excel's fatal open dialog, including one layered over a visible ribbon."""
    from playwright.async_api import Error as PlaywrightError

    for frame in page.frames:
        try:
            message = frame.get_by_text("The workbook cannot be opened.", exact=True)
            if await message.is_visible():
                return True
        except PlaywrightError:
            if not frame.is_detached():
                raise
    return False


async def find_editor(page):
    """Find Excel's Review tab and handle an explicit sign-in prompt if present."""
    from playwright.async_api import Error as PlaywrightError

    ready = asyncio.get_running_loop().create_future()
    tasks = set()
    seen = set()

    async def inspect_frame(frame) -> None:
        review = frame.locator("#Review:visible").first
        sign_in = frame.locator("#O365_MainLink_SigninLink:visible").first
        try:
            deadline = time.monotonic() + 120
            while True:
                if await workbook_open_error(page):
                    raise WorkbookCannotOpen("Excel says the workbook cannot be opened")
                await raise_if_workbook_blocked(page)
                if await sign_in.is_visible():
                    await sign_in.click(force=True, timeout=5_000)
                    await sign_in.wait_for(state="hidden", timeout=30_000)
                    continue
                if await review.is_visible():
                    if not ready.done():
                        ready.set_result(frame)
                    return
                if time.monotonic() >= deadline:
                    raise TimeoutError("Excel did not expose the Review tab within 120 seconds")
                await asyncio.sleep(0.25)
        except Exception as error:
            if isinstance(error, PlaywrightError) and frame.is_detached():
                return
            if not ready.done():
                ready.set_exception(error)
            return

    def watch(frame) -> None:
        identity = (id(frame), frame.url)
        if identity in seen:
            return
        seen.add(identity)
        task = asyncio.create_task(inspect_frame(frame))
        tasks.add(task)
        task.add_done_callback(tasks.discard)

    page.on("frameattached", watch)
    page.on("framenavigated", watch)
    for frame in page.frames:
        watch(frame)
    try:
        return await asyncio.wait_for(ready, timeout=120)
    except TimeoutError as error:
        raise RuntimeError("Excel did not expose the Review tab within 120 seconds") from error
    finally:
        page.remove_listener("frameattached", watch)
        page.remove_listener("framenavigated", watch)
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)


async def select_review(page, editor) -> None:
    """Retry Review only when its selected attribute fails to change."""
    from playwright.async_api import TimeoutError as PlaywrightTimeout

    review = editor.locator("#Review:visible").first
    selected = editor.locator('#Review[aria-selected="true"]:visible').first
    deadline = time.monotonic() + 120
    while time.monotonic() < deadline:
        if await workbook_open_error(page):
            raise WorkbookCannotOpen("Excel says the workbook cannot be opened")
        await raise_if_workbook_blocked(page)
        if await selected.count():
            return
        try:
            await review.click(force=True, timeout=3_000)
            await selected.wait_for(state="visible", timeout=2_000)
            return
        except PlaywrightTimeout:
            continue
    raise RuntimeError("Review tab never became selected")


async def remove_invalid_objects_for_editing(editor) -> bool:
    """Accept Excel's explicit invalid-object repair and expose it to the run log."""
    bar = editor.locator("#BusinessBar")
    button = bar.get_by_role("button", name=REMOVE_AND_EDIT)
    if not await button.count():
        return False
    message = await bar.inner_text()
    if not INVALID_OBJECTS.search(message):
        raise RuntimeError("Remove and Edit appeared for an unrecognized read-only warning")
    await button.click(force=True, timeout=10_000)
    await button.wait_for(state="hidden", timeout=60_000)
    return True


async def find_performance_pane(page, editor):
    """Open Performance, or return None when Excel keeps its ribbon group disabled."""
    from playwright.async_api import Error as PlaywrightError, TimeoutError as PlaywrightTimeout
    await raise_if_workbook_blocked(page)
    command = editor.locator("#ShowOptimizeWorkbookTaskpane:visible").first
    await command.wait_for(state="visible", timeout=30_000)
    enabled_deadline = time.monotonic() + 10
    while not await command.is_enabled():
        await raise_if_workbook_blocked(page)
        if time.monotonic() >= enabled_deadline:
            return None
        await asyncio.sleep(0.5)
    try:
        await command.click(timeout=15_000)
    except PlaywrightTimeout:
        # A notice can arrive while Playwright waits for the ribbon to accept input.
        await raise_if_workbook_blocked(page)
        raise
    deadline = time.monotonic() + 120
    while time.monotonic() < deadline:
        await raise_if_workbook_blocked(page)
        for frame in page.frames:
            # Excel may host Performance in its editor frame or an Office
            # resource frame. Match the pane's visible controls in either host.
            try:
                if (await frame.get_by_role("button", name=OPTIMIZE).count()
                        or await frame.get_by_text(ALREADY_CLEAN).count()
                        or await frame.get_by_role("button", name=CHECK_AGAIN).count()):
                    return frame
            except PlaywrightError:
                if not frame.is_detached():
                    raise
        await asyncio.sleep(0.5)
    diagnostics = []
    for frame in page.frames:
        try:
            labels = await frame.get_by_role("button").all_text_contents()
            relevant = [label.strip() for label in labels
                        if re.search(r"optimi|performance|pr\u00fcf|check", label, re.I)]
            diagnostics.append({"host": urlsplit(frame.url).hostname, "controls": relevant[:12]})
        except Exception as error:
            diagnostics.append({"host": urlsplit(frame.url).hostname, "error": type(error).__name__})
    raise RuntimeError("Performance pane did not become ready; " + json.dumps(diagnostics))


async def optimize(page, pane, progress: ProgressStore,
                   attempt: AttemptClock, name: str) -> str:
    """Dispatch Optimize all and observe Excel's acceptance and result events."""
    check_again = pane.get_by_role("button", name=CHECK_AGAIN)
    if await check_again.count():
        await check_again.click()
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            if await pane.get_by_text(ALREADY_CLEAN).count():
                return "already_clean_after_check_again"
            if await pane.get_by_role("button", name=OPTIMIZE).count():
                break
            await asyncio.sleep(0.5)
    if await pane.get_by_text(ALREADY_CLEAN).count():
        return "already_clean"

    async def on_pane_event(phase: str) -> None:
        if phase == "optimization_accepted":
            progress.transition(name, JobState.OPTIMIZING)
        record_phase(progress, attempt, name, phase)

    bridge = getattr(page, "_check_performance_event_bridge", None)
    if bridge is None:
        bridge = {"handler": None}

        async def dispatch(_source, phase: str) -> None:
            handler = bridge["handler"]
            if handler is not None:
                await handler(phase)

        await page.expose_binding("__checkPerformanceEvent", dispatch)
        page._check_performance_event_bridge = bridge
    bridge["handler"] = on_pane_event
    script = Path(__file__).with_name("pane.js").read_text(encoding="utf-8")
    try:
        outcome = await pane.evaluate(script)
    finally:
        bridge["handler"] = None
    if outcome not in {"optimized", "already_clean", "already_clean_after_check_again"}:
        raise RuntimeError(f"Excel did not complete Check Performance: {outcome}")
    return outcome


async def wait_for_cloud_save(editor) -> None:
    """Do not close a changed workbook while Excel still says it is saving."""
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        labels = await editor.locator("button[aria-label]").evaluate_all("""buttons =>
          buttons.map(button => button.getAttribute('aria-label') || '')
            .filter(label => /Gespeichert|Saved|Wird gespeichert|Saving/i.test(label))""")
        if any(re.search(r"Gespeichert|Saved", label, re.I) and
               not re.search(r"Wird gespeichert|Saving", label, re.I) for label in labels):
            return
        await asyncio.sleep(0.5)
    raise TimeoutError("Excel did not confirm its cloud save within 60 seconds")


async def open_editor_with_retry(context, folder_page, name: str,
                                 progress: ProgressStore, attempt: AttemptClock, row=None):
    """Retry one explicit Excel open rejection in the same browser tab."""
    folder_url = folder_page.url
    for opening in range(2):
        page = await open_workbook_in_current_tab(context, folder_page, name, row)
        record_phase(progress, attempt, name, "workbook_opened")
        try:
            editor = await find_editor(page)
            if await workbook_open_error(page):
                raise WorkbookCannotOpen("Excel says the workbook cannot be opened")
            progress.transition(name, JobState.CHECKING)
            record_phase(progress, attempt, name, "editor_ready")
            await select_review(page, editor)
            return page, editor
        except WorkbookCannotOpen:
            record_phase(progress, attempt, name,
                         "workbook_open_rejected" if opening else "workbook_open_retrying")
            if opening == 0:
                await page.goto(folder_url, wait_until="domcontentloaded", timeout=120_000)
                await page.locator('[data-automationid="field-LinkFilename"]').first.wait_for(
                    state="visible", timeout=60_000)
                await sort_oldest_first(page)
                row = None
    raise WorkbookCannotOpen("Excel rejected the workbook on both opening attempts")


async def process_workbook(context, folder_page, profile: Path,
                           progress: ProgressStore, display: ConsoleProgress, name: str,
                           row=None) -> None:
    """Open, inspect, optimize, and confirm one workbook before opening the next."""
    previous_status = progress.status(name)
    folder_url = folder_page.url
    attempt = AttemptClock(progress.start_attempt(name), time.monotonic(), display=display)
    display.begin(name, previous_status)
    page = None
    outcome = ""
    message = None
    throttled = False
    try:
        page, editor = await open_editor_with_retry(
            context, folder_page, name, progress, attempt, row)
        record_phase(progress, attempt, name, "review_ready")
        await raise_if_workbook_blocked(page)
        repaired = await remove_invalid_objects_for_editing(editor)
        if repaired:
            record_phase(progress, attempt, name, "invalid_objects_removed")
        pane = await find_performance_pane(page, editor)
        if pane is None:
            if repaired:
                await wait_for_cloud_save(editor)
                record_phase(progress, attempt, name, "cloud_saved_after_remove_and_edit")
            record_phase(progress, attempt, name, "performance_unavailable")
            outcome = "performance_unavailable"
            progress.skip(name, attempt.id, outcome)
            return
        record_phase(progress, attempt, name, "performance_ready")
        record_phase(progress, attempt, name, "optimization_button_ready")
        outcome = await optimize(page, pane, progress, attempt, name)
        if repaired:
            outcome += "_after_remove_and_edit"
        record_phase(progress, attempt, name, "optimization_finished")
        if outcome != "already_clean" or repaired:
            progress.transition(name, JobState.SAVING)
            record_phase(progress, attempt, name, "saving_started")
            await wait_for_cloud_save(editor)
            record_phase(progress, attempt, name, "cloud_saved")
        record_phase(progress, attempt, name, "memory_sampled", await sample_tab_memory(page, profile, editor))
        progress.finish(name, attempt.id, outcome)
    except WorkbookCannotOpen as error:
        outcome = "workbook_cannot_open"
        message = str(error)
        progress.skip(name, attempt.id, outcome)
    except WorkbookEditRestricted as error:
        outcome = "irm_browser_edit_unavailable"
        message = str(error)
        record_phase(progress, attempt, name, outcome)
        progress.skip(name, attempt.id, outcome, message)
    except WorkbookRepairRequired as error:
        outcome = "workbook_repair_required"
        message = str(error)
        record_phase(progress, attempt, name, outcome)
        progress.skip(name, attempt.id, outcome, message)
    except OneDriveThrottled as error:
        message = str(error)
        throttled = True
        progress.fail(name, attempt.id, message)
        raise
    except Exception as error:
        message = f"{type(error).__name__}: {error}"[:500]
        progress.fail(name, attempt.id, message)
    finally:
        try:
            if not throttled and not folder_page.is_closed() and folder_page.url != folder_url:
                await folder_page.goto(folder_url, wait_until="domcontentloaded", timeout=120_000)
                await wait_for_folder_rows(folder_page)
                await sort_oldest_first(folder_page)
        finally:
            display.finish(name, attempt, outcome, message)


async def process_with_live_status(context, folder_page, profile: Path,
                                   progress: ProgressStore, display: ConsoleProgress,
                                   name: str, row=None) -> None:
    """Keep the elapsed-time line current while Excel is busy with a workbook."""
    async def refresh_each_second() -> None:
        while True:
            await asyncio.sleep(1)
            display.refresh()

    ticker = asyncio.create_task(refresh_each_second()) if display.interactive else None
    try:
        await process_workbook(context, folder_page, profile, progress, display, name, row)
    finally:
        if ticker is not None:
            ticker.cancel()
            await asyncio.gather(ticker, return_exceptions=True)


async def recover_folder(context, folder_page, folder_url: str, headed: bool):
    """Restore the shared folder in one tab after a per-workbook navigation error."""
    if folder_page.is_closed():
        folder_page = await context.new_page()
        await folder_page.add_init_script(SINGLE_TAB_NAVIGATION)
    folder_page = await open_folder(folder_page, folder_url, headed)
    await sort_oldest_first(folder_page)
    return folder_page


async def close_browser(playwright, context) -> None:
    """Close browser and driver without hiding the workbook's actual error."""
    if context is not None:
        try:
            await context.close()
        except Exception as error:
            print(f"Browser cleanup failed: {type(error).__name__}: {error}", file=sys.stderr)
    if playwright is not None:
        try:
            await playwright.stop()
        except Exception as error:
            print(f"Playwright cleanup failed: {type(error).__name__}: {error}", file=sys.stderr)


async def start_folder_browser(args, profile: Path):
    """Open a fresh Playwright driver and browser with the retained sign-in profile."""
    from playwright.async_api import async_playwright

    discard_restored_tabs(profile)
    playwright = await async_playwright().start()
    context = None
    try:
        context = await playwright.chromium.launch_persistent_context(
            profile, channel=args.browser_channel, headless=not args.headed,
            chromium_sandbox=True, viewport={"width": 1920, "height": 1080})
        folder_page = context.pages[0] if context.pages else await context.new_page()
        for extra_page in context.pages:
            if extra_page is not folder_page:
                await extra_page.close()
        await folder_page.add_init_script(SINGLE_TAB_NAVIGATION)
        folder_page = await open_folder(folder_page, args.folder_url, args.headed)
        await sort_oldest_first(folder_page)
        return playwright, context, folder_page
    except BaseException:
        await close_browser(playwright, context)
        raise


async def restart_folder_browser(playwright, context, args, profile: Path):
    """Retry a broken browser with bounded backoff, preserving SQLite progress."""
    await close_browser(playwright, context)
    delays = (0, 5, 15, 30, 60, 120, 300, 300, 300)
    for attempt, delay in enumerate(delays, 1):
        if delay:
            print(f"Browser unavailable; retrying in {delay} s ({attempt}/{len(delays)}).",
                  file=sys.stderr, flush=True)
            await asyncio.sleep(delay)
        try:
            return await start_folder_browser(args, profile)
        except RuntimeError as error:
            if "sign-in required" in str(error):
                raise
            print(f"Browser restart {attempt} failed: {error}", file=sys.stderr, flush=True)
        except Exception as error:
            print(f"Browser restart {attempt} failed: {type(error).__name__}: {error}",
                  file=sys.stderr, flush=True)
    raise RuntimeError("Could not restore the OneDrive browser after nine attempts")


async def next_workbook_with_recovery(context, folder_page, folder_url: str,
                                      headed: bool, unfinished: set[str]):
    """Retry a broken folder view once before treating it as a run-level failure."""
    try:
        next_file = await next_visible_unfinished(folder_page, unfinished)
        if next_file is not None:
            return folder_page, next_file
    except OneDriveRenderingError:
        raise
    except Exception as error:
        print(f"Folder listing failed ({type(error).__name__}: {error}); "
              "reloading the shared folder once.", file=sys.stderr, flush=True)
    folder_page = await recover_folder(context, folder_page, folder_url, headed)
    return folder_page, await next_visible_unfinished(folder_page, unfinished)


async def process_selected_workbook(playwright, context, folder_page, args, profile: Path,
                                    progress: ProgressStore, display: ConsoleProgress,
                                    name: str, row):
    """Retry one unexpected failure in a fresh browser, then skip only that file."""
    for attempt_number in (1, 2):
        navigation_failed = False
        try:
            await process_with_live_status(
                context, folder_page, profile, progress, display, name, row)
        except OneDriveRenderingError:
            raise
        except Exception as error:
            print(f"{name}: browser or folder navigation failed: "
                  f"{type(error).__name__}: {error}", file=sys.stderr, flush=True)
            if progress.status(name) not in {
                    JobState.COMPLETED.value, JobState.SKIPPED.value, JobState.FAILED.value}:
                raise RuntimeError(f"Cannot safely retry {name}: its attempt state is uncertain") from error
            navigation_failed = True

        state = progress.status(name)
        if state in {JobState.COMPLETED.value, JobState.SKIPPED.value}:
            if navigation_failed:
                playwright, context, folder_page = await restart_folder_browser(
                    playwright, context, args, profile)
            return playwright, context, folder_page, False
        if state != JobState.FAILED.value:
            raise RuntimeError(f"Unexpected workbook state after attempt: {name}: {state}")
        if attempt_number == 2:
            progress.skip_failed(name, "error_after_retry")
            print(f"{name}: two attempts failed; skipped. The attempt errors remain in SQLite.",
                  file=sys.stderr, flush=True)
            playwright, context, folder_page = await restart_folder_browser(
                playwright, context, args, profile)
            return playwright, context, folder_page, True
        print(f"{name}: retrying in a fresh browser window.", file=sys.stderr, flush=True)
        playwright, context, folder_page = await restart_folder_browser(
            playwright, context, args, profile)
        row = None
    raise AssertionError("Both workbook attempts should have returned")


async def run_folder_check_performance(args) -> int:
    """Resume known files from the oldest visible row and record each outcome."""
    identifier = folder_id(args.folder_url)
    directory = args.output_root.resolve() / identifier
    profile = args.profile_dir.resolve() if args.profile_dir else args.output_root.resolve() / "browser-profile"
    playwright, context, folder_page = await restart_folder_browser(None, None, args, profile)
    try:
        with ProgressStore(directory, identifier) as progress:
            if not progress.names() or args.refresh_list or args.list_only:
                discovered = await discover_workbooks(folder_page)
                names = progress.discover(discovered)
                print(f"Found {len(discovered)} XLSX files; {len(names)} known across runs. "
                      f"Progress: {progress.path}", flush=True)
            else:
                names = progress.names()
                print(f"Reusing {len(names)} known XLSX files. Progress: {progress.path}", flush=True)
            if args.list_only:
                for name in names:
                    print(name)
                return 0
            if args.only_file and args.only_file not in names:
                raise ValueError(f"Workbook is absent from the folder: {args.only_file}")
            unfinished = progress.unfinished_names(args.retry_skipped)
            if args.only_file:
                unfinished.intersection_update({args.only_file})
            attempted = 0
            failed = False
            display = ConsoleProgress(progress)
            try:
                while unfinished and attempted < args.max_files:
                    if args.only_file:
                        name, row = args.only_file, None
                    else:
                        try:
                            folder_page, next_file = await next_workbook_with_recovery(
                                context, folder_page, args.folder_url, args.headed, unfinished)
                        except Exception as error:
                            print(f"Folder view failed ({type(error).__name__}: {error}); "
                                  "restarting the browser.", file=sys.stderr, flush=True)
                            playwright, context, folder_page = await restart_folder_browser(
                                playwright, context, args, profile)
                            next_file = await next_visible_unfinished(folder_page, unfinished)
                        if next_file is None:
                            print(f"{len(unfinished)} known files were not found in the folder view.",
                                  file=sys.stderr)
                            failed = True
                            break
                        name, row = next_file
                    try:
                        playwright, context, folder_page, file_failed = await process_selected_workbook(
                            playwright, context, folder_page, args, profile,
                            progress, display, name, row)
                    except OneDriveRenderingError as error:
                        print(f"{error} Preserving progress and restarting.", file=sys.stderr, flush=True)
                        if progress.status(name) in {JobState.COMPLETED.value, JobState.SKIPPED.value}:
                            attempted += 1
                            unfinished.discard(name)
                        playwright, context, folder_page = await restart_folder_browser(
                            playwright, context, args, profile)
                        continue
                    attempted += 1
                    unfinished.remove(name)
                    failed = failed or file_failed
            finally:
                display.close()
            processed, total = progress.counts()
            print(f"Processed: {processed}/{total}; skipped: {progress.skipped_count()}; "
                  f"remaining: {total - processed}", flush=True)
            return 1 if failed or (unfinished and attempted < args.max_files) else 0
    finally:
        await close_browser(playwright, context)


def parse_arguments() -> argparse.Namespace:
    """Parse and validate the OneDrive runner's command-line options."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("folder_url", help="Editable OneDrive folder share link")
    parser.add_argument("--output-root", type=Path, default=Path(".tmp/onedrive-check-performance"))
    parser.add_argument("--profile-dir", type=Path, help="Persistent signed-in browser profile")
    parser.add_argument("--browser-channel", choices=["msedge", "chrome", "chromium"], default="msedge")
    parser.add_argument("--headed", action="store_true", help="Show the browser for the initial sign-in")
    parser.add_argument("--list-only", action="store_true", help="Discover files without optimizing")
    parser.add_argument("--refresh-list", action="store_true", help="Scan the full folder for new files")
    parser.add_argument("--retry-skipped", action="store_true", help="Retry files previously marked unavailable")
    parser.add_argument("--only-file", help="Process or check one named workbook")
    parser.add_argument("--max-files", type=int, default=sys.maxsize, help="Process at most N files this run")
    args = parser.parse_args()
    if args.max_files < 1:
        parser.error("--max-files must be positive")
    return args


def include_local_windows_packages() -> None:
    """Use the earlier web experiment's local Playwright installation on Windows."""
    package_dir = Path(__file__).resolve().parents[2] / ".tmp/check-performance-web/python-packages"
    if os.name == "nt" and package_dir.exists():
        sys.path.insert(0, str(package_dir))


def run_check_performance(args: argparse.Namespace) -> int:
    """Run the browser workflow and report run-level failures without false success."""
    try:
        return asyncio.run(run_folder_check_performance(args))
    except ImportError as error:
        if error.name == "playwright":
            print("Install Playwright first: python -m pip install playwright", file=sys.stderr)
        else:
            print(f"Check Performance stopped: ImportError: {error}", file=sys.stderr)
        return 1
    except Exception as error:
        print(f"Check Performance stopped: {type(error).__name__}: {error}", file=sys.stderr)
        return 1


def main() -> int:
    args = parse_arguments()
    include_local_windows_packages()
    return run_check_performance(args)
