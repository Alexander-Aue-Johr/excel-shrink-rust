"""Capture browser evidence before recovery changes or closes the failing page."""

import asyncio
from datetime import datetime, timezone
import json
from pathlib import Path
import sys
import uuid


async def capture_failure(page, root: Path, operation: str, error: Exception,
                          workbook: str | None = None) -> Path | None:
    """Keep partial evidence when a page, frame, or browser is unresponsive."""
    directory = None
    metadata = {
        "operation": operation, "workbook": workbook,
        "error": f"{type(error).__name__}: {error}", "frames": [], "capture_errors": [],
    }
    try:
        stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
        directory = root / "diagnostics" / f"{stamp}-{uuid.uuid4().hex[:8]}"
        directory.mkdir(parents=True, exist_ok=False)
        metadata["url"] = str(page.url)
        async with asyncio.timeout(20):
            try:
                await page.screenshot(path=str(directory / "viewport.png"), timeout=5_000)
            except Exception as capture_error:
                metadata["capture_errors"].append(f"screenshot: {capture_error}")
            for index, frame in enumerate(page.frames):
                entry = {"index": index, "url": str(frame.url), "file": f"frame-{index}.html"}
                metadata["frames"].append(entry)
                try:
                    content = await asyncio.wait_for(frame.content(), timeout=3)
                    (directory / entry["file"]).write_text(content, encoding="utf-8")
                except Exception as capture_error:
                    entry["error"] = str(capture_error)
            session = None
            try:
                session = await page.context.new_cdp_session(page)
                snapshot = await asyncio.wait_for(session.send(
                    "DOMSnapshot.captureSnapshot", {"computedStyles": [], "includeDOMRects": True}
                ), timeout=5)
                (directory / "dom-snapshot.json").write_text(
                    json.dumps(snapshot, ensure_ascii=False), encoding="utf-8")
            except Exception as capture_error:
                metadata["capture_errors"].append(f"DOM snapshot: {capture_error}")
            finally:
                if session is not None:
                    await asyncio.wait_for(session.detach(), timeout=1)
    except Exception as capture_error:
        metadata["capture_errors"].append(str(capture_error))
    finally:
        if directory is not None:
            try:
                (directory / "error.json").write_text(
                    json.dumps(metadata, ensure_ascii=False, indent=2), encoding="utf-8")
                print(f"Browser diagnostics: {directory}", file=sys.stderr, flush=True)
            except Exception as capture_error:
                print(f"Browser diagnostics could not be written: {capture_error}",
                      file=sys.stderr, flush=True)
    return directory
