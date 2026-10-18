#!/usr/bin/env python3
"""Rank completed visual phases, incomplete phases, and timestamped log gaps.

A log gap is an interval without messages, not measured CPU time. Nested phase
wall times overlap and must not be added together. Uses Python 3.10+ stdlib only.
"""
from __future__ import annotations
import argparse
import datetime as dt
import glob
import heapq
import json
import re
import sys
from pathlib import Path
from typing import Any

ANSI = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")
STAMP = re.compile(r"\b(\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?)\b")

class Ranking:
    def __init__(self, count: int, minimum_ms: float) -> None:
        self.count = count
        self.minimum = minimum_ms
        self.serial = 0
        self.rows: list[tuple[float, int, dict[str, Any]]] = []
        self.total_qualifying = 0

    def add(self, milliseconds: float, row: dict[str, Any]) -> None:
        if milliseconds < self.minimum:
            return
        self.total_qualifying += 1
        self.serial += 1
        row["duration_ms"] = milliseconds
        item = (milliseconds, self.serial, row)
        if len(self.rows) < self.count:
            heapq.heappush(self.rows, item)
        elif milliseconds > self.rows[0][0]:
            heapq.heapreplace(self.rows, item)

    def result(self) -> list[dict[str, Any]]:
        return [row for _, _, row in sorted(self.rows, reverse=True)]


def analyze(paths: list[Path], top: int = 30, min_seconds: float = 5.0) -> dict[str, Any]:
    completed = Ranking(top, min_seconds * 1000)
    steps = Ranking(top, min_seconds * 1000)
    gaps = Ranking(top, min_seconds * 1000)
    incomplete = Ranking(top, min_seconds * 1000)
    for path in paths:
        previous: tuple[dt.datetime, int, str] | None = None
        active: dict[tuple[Any, Any], dict[str, Any]] = {}
        with path.open("r", encoding="utf-8-sig", errors="replace") as source:
            for number, raw in enumerate(source, 1):
                text = ANSI.sub("", raw).strip()
                if not text:
                    continue
                record: dict[str, Any] | None = None
                if text.startswith("{"):
                    try:
                        candidate = json.loads(text)
                        if isinstance(candidate, dict) and "phase_id" in candidate and "event" in candidate:
                            record = candidate
                    except json.JSONDecodeError:
                        pass  # A running process can leave an incomplete final JSON line.
                if record is not None:
                    key = (record.get("pid"), record["phase_id"])
                    event = record["event"]
                    row = {**record, "file": str(path), "line": number}
                    if event == "end":
                        active.pop(key, None)
                        completed.add(float(record.get("elapsed_ms", 0)), row)
                        # Include the last step, which has no subsequent progress event.
                        steps.add(float(record.get("detail_elapsed_ms", 0)), {**row, "event": "last_step"})
                    elif event == "progress":
                        steps.add(float(record.get("detail_elapsed_ms", record.get("elapsed_ms", 0))), row)
                    elif event in {"start", "heartbeat"}:
                        active[key] = row
                    continue
                found = STAMP.search(text)
                if not found:
                    continue
                try:
                    instant = dt.datetime.fromisoformat(found[1].replace("Z", "+00:00"))
                except ValueError:
                    continue
                if previous is not None:
                    earlier, earlier_line, earlier_text = previous
                    try:
                        duration = (instant - earlier).total_seconds() * 1000
                    except TypeError:
                        duration = -1  # Never guess a timezone when a file mixes formats.
                    if duration >= 0:
                        gaps.add(duration, {"file": str(path), "before_line": earlier_line,
                            "after_line": number, "before": earlier_text, "after": text,
                            "kind": "silence_between_messages_not_cpu_time"})
                previous = (instant, number, text)
        for row in active.values():
            incomplete.add(float(row.get("elapsed_ms", 0)), row)
    return {"notice": "Wall times overlap. Do not sum nested phases. Log gaps are not CPU timings.",
        "files": [str(path) for path in paths], "completed_phases": completed.result(),
        "completed_steps": steps.result(), "incomplete_phases_last_observed": incomplete.result(),
        "log_gaps": gaps.result(), "counts_above_threshold": {
            "completed_phases": completed.total_qualifying, "completed_steps": steps.total_qualifying,
            "log_gaps": gaps.total_qualifying}}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("logs", nargs="+", help="Log/JSONL files or glob patterns (quote patterns in PowerShell).")
    parser.add_argument("--top", type=int, default=30)
    parser.add_argument("--min-seconds", type=float, default=5.0)
    parser.add_argument("--json", type=Path, help="Write a NEW JSON report; existing files are never overwritten.")
    args = parser.parse_args(argv)
    if args.top < 1 or args.min_seconds < 0:
        parser.error("--top must be positive and --min-seconds nonnegative")
    paths: list[Path] = []
    for value in args.logs:
        matches = [Path(value)] if Path(value).is_file() else [Path(p) for p in sorted(glob.glob(value))]
        if not matches:
            parser.error(f"No files matched {value!r}")
        paths.extend(p.resolve() for p in matches if p.is_file())
    paths = list(dict.fromkeys(paths))
    try:
        report = analyze(paths, args.top, args.min_seconds)
        print(report["notice"])
        for heading, key in [("Completed phases", "completed_phases"), ("Completed steps", "completed_steps"),
                             ("Incomplete phases (last observation)", "incomplete_phases_last_observed"),
                             ("Gaps in timestamped text logs", "log_gaps")]:
            print(f"\n{heading}")
            if not report[key]:
                print("  None above threshold.")
            for row in report[key]:
                label = row.get("phase", "log gap")
                detail = str(row.get("detail", row.get("before", "")))
                print(f"  {row['duration_ms']/1000:10.3f}s  {label}: {detail[:300]}")
                print(f"                {row['file']}:{row.get('line', row.get('before_line'))}")
                if "after" in row:
                    print(f"                next: {row['after'][:300]}")
        if args.json:
            # Never overwrite a source log, workbook, or previous report.
            with args.json.open("x", encoding="utf-8", newline="\n") as target:
                json.dump(report, target, indent=2, ensure_ascii=False)
                target.write("\n")
            print(f"\nReport: {args.json.resolve()}")
        return 0
    except (OSError, ValueError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1

if __name__ == "__main__":
    raise SystemExit(main())
