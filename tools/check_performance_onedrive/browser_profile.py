"""Release only a recorded runner browser before touching its session files."""

import asyncio
import json
import os
from pathlib import Path
import time

try:
    import psutil
except ImportError:
    psutil = None


class BrowserProfileBusy(RuntimeError):
    """The profile cannot safely be acquired by this runner."""


def owner_path(profile: Path) -> Path:
    return profile / ".runner-browser.json"


def uses_profile(process, profile: Path) -> bool:
    """Compare a full user-data-dir argument, not a path substring."""
    browser_names = {"msedge", "msedge.exe", "chrome", "chrome.exe", "chromium", "chromium.exe",
                     "chrome-headless-shell", "chrome-headless-shell.exe", "headless_shell", "headless_shell.exe"}
    if process.name().lower() not in browser_names:
        return False
    arguments = process.cmdline()
    for index, argument in enumerate(arguments):
        if argument.startswith("--user-data-dir="):
            value = argument.split("=", 1)[1]
        elif argument == "--user-data-dir" and index + 1 < len(arguments):
            value = arguments[index + 1]
        else:
            continue
        return Path(value.strip('"')).resolve() == profile.resolve()
    return False


def same_process(pid: int, created: float):
    try:
        process = psutil.Process(pid)
        return process if process.create_time() == created else None
    except psutil.NoSuchProcess:
        return None


def owned_processes(profile: Path) -> list:
    """Verify the recorded browser, then snapshot its actual descendants."""
    if psutil is None or not owner_path(profile).exists():
        return []
    try:
        owner = json.loads(owner_path(profile).read_text(encoding="utf-8"))
        if Path(owner["profile"]).resolve() != profile.resolve():
            raise ValueError("Profile path differs from the ownership record")
        runner = same_process(owner["runner_pid"], owner["runner_created"])
        if runner is not None and runner.pid != os.getpid():
            raise BrowserProfileBusy("This browser profile belongs to another active runner")
        browser = same_process(owner["browser_pid"], owner["browser_created"])
        if browser is None:
            return []
        if not uses_profile(browser, profile):
            raise ValueError("The recorded process does not use this browser profile")
        processes = browser.children(recursive=True) + [browser]
        return [(process, process.create_time()) for process in processes]
    except psutil.NoSuchProcess:
        return []
    except (KeyError, ValueError, psutil.AccessDenied) as error:
        raise BrowserProfileBusy(f"Cannot verify browser ownership: {error}") from error


async def release_owned_browser(profile: Path, processes=None) -> None:
    """Bounded exit waits and termination apply only to verified process identities."""
    if psutil is None:
        return
    try:
        record = owner_path(profile).read_bytes()
    except FileNotFoundError:
        record = None
    if processes is None:
        processes = owned_processes(profile)
    alive = [process for process, created in processes if same_process(process.pid, created) is not None]
    if alive:
        _, remaining = await asyncio.to_thread(psutil.wait_procs, alive, timeout=3)
        for action in ("terminate", "kill"):
            for process in remaining:
                created = next(created for candidate, created in processes if candidate.pid == process.pid)
                if same_process(process.pid, created) is not None:
                    try:
                        getattr(process, action)()
                    except psutil.NoSuchProcess:
                        pass
            _, remaining = await asyncio.to_thread(psutil.wait_procs, remaining, timeout=3)
            if not remaining:
                break
        if remaining:
            raise BrowserProfileBusy("The recorded browser did not release its processes")
    if record is not None:
        try:
            if owner_path(profile).read_bytes() == record:
                owner_path(profile).unlink(missing_ok=True)
        except FileNotFoundError:
            pass


async def record_browser(context, profile: Path) -> None:
    """Obtain the browser PID from our own CDP connection and checkpoint it."""
    if psutil is None:
        return
    session = await context.browser.new_browser_cdp_session()
    try:
        info = await session.send("SystemInfo.getProcessInfo")
    finally:
        await session.detach()
    pid = next(int(item["id"]) for item in info["processInfo"] if item["type"] == "browser")
    browser = psutil.Process(pid)
    if not uses_profile(browser, profile):
        raise BrowserProfileBusy("The launched browser does not match the requested profile")
    runner = psutil.Process(os.getpid())
    owner = {"profile": str(profile.resolve()), "runner_pid": runner.pid,
             "runner_created": runner.create_time(), "browser_pid": pid,
             "browser_created": browser.create_time()}
    temporary = owner_path(profile).with_suffix(".tmp")
    temporary.write_text(json.dumps(owner), encoding="utf-8")
    temporary.replace(owner_path(profile))


async def prepare_profile(profile: Path, discard_tabs) -> None:
    await release_owned_browser(profile)
    if psutil is not None:
        for process in psutil.process_iter():
            try:
                if uses_profile(process, profile):
                    raise BrowserProfileBusy("An unrecorded browser still uses this profile; close its window first")
            except (psutil.NoSuchProcess, psutil.AccessDenied):
                continue
    deadline = time.monotonic() + 5
    while True:
        try:
            discard_tabs(profile)
            return
        except FileNotFoundError:
            return
        except PermissionError as error:
            if time.monotonic() >= deadline:
                raise BrowserProfileBusy(f"Browser session files remain locked: {error}") from error
            await asyncio.sleep(0.25)
