"""Summarize per-file phase durations and actual overlap from a batch result."""

import argparse
import json
from pathlib import Path


PHASES = (
    ("tab_start", "tab_ready", "tab_open_seconds"),
    ("tab_ready", "home_ready", "home_seconds"),
    ("upload_sent", "workbook_opened", "upload_to_open_seconds"),
    ("workbook_opened", "review_opened", "editor_to_review_seconds"),
    ("review_opened", "performance_ready", "performance_panel_seconds"),
    ("optimization_started", "optimization_finished", "optimization_seconds"),
    ("download_started", "download_finished", "download_seconds"),
)


def job_durations(job: dict) -> dict:
    """Calculate durations only for phase pairs actually observed."""
    times = job["times"]
    durations = {
        label: round(times[end] - times[start], 3)
        for start, end, label in PHASES
        if start in times and end in times
    }
    if "tab_start" in times and "download_finished" in times:
        durations["total_seconds"] = round(times["download_finished"] - times["tab_start"], 3)
    return durations


def max_overlap(jobs: dict, start: str, end: str) -> int:
    """Count the largest number of simultaneously active measured intervals."""
    events = []
    for job in jobs.values():
        times = job["times"]
        if start in times and end in times:
            events.extend(((times[start], 1), (times[end], -1)))
    active = maximum = 0
    for _, delta in sorted(events):
        active += delta
        maximum = max(maximum, active)
    return maximum


def summarize(result: dict) -> dict:
    """Compare batch makespans without treating failed or skipped files as wins."""
    sequential = result["sequential"]
    parallel = result["parallel"]
    summary = {
        "browser_start_seconds": round(result["browser"]["ready"] - result["browser"]["start"], 3),
        "sequential": {name: {"status": job["status"], **job_durations(job)}
                       for name, job in sequential.items()},
        "parallel": {name: {"status": job["status"], **job_durations(job)}
                     for name, job in parallel.items()},
        "max_simultaneous_upload_to_open": max_overlap(parallel, "upload_sent", "workbook_opened"),
        "max_simultaneous_optimization": max_overlap(parallel, "optimization_started", "optimization_finished"),
    }
    complete_in_both = [name for name in sequential
                        if sequential[name]["status"] == "completed"
                        and parallel.get(name, {}).get("status") == "completed"]
    summary["complete_in_both"] = complete_in_both
    if complete_in_both:
        parallel_first = min(parallel[name]["times"]["tab_start"] for name in complete_in_both)
        parallel_last = max(parallel[name]["times"]["download_finished"] for name in complete_in_both)
        sequential_seconds = sum(job_durations(sequential[name])["total_seconds"]
                                 for name in complete_in_both)
        parallel_seconds = parallel_last - parallel_first
        summary["comparable_sequential_sum_seconds"] = round(sequential_seconds, 3)
        summary["comparable_parallel_wall_seconds"] = round(parallel_seconds, 3)
        summary["comparable_speedup"] = round(sequential_seconds / parallel_seconds, 3)
        summary["longest_sequential_file_seconds"] = max(
            job_durations(sequential[name])["total_seconds"] for name in complete_in_both)
    all_sequential = list(sequential.values())
    if all(job["status"] == "completed" for job in all_sequential):
        summary["all_eight_sequential_wall_seconds"] = round(
            max(job["times"]["download_finished"] for job in all_sequential)
            - min(job["times"]["tab_start"] for job in all_sequential), 3)
    return summary


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("result", type=Path)
    args = parser.parse_args()
    report = summarize(json.loads(args.result.read_text(encoding="utf-8")))
    destination = args.result.with_name("batch-summary.json")
    destination.write_text(json.dumps(report, indent=2, ensure_ascii=True) + "\n", encoding="utf-8")
    print(json.dumps({key: value for key, value in report.items()
                      if key not in {"sequential", "parallel"}}, indent=2))


if __name__ == "__main__":
    main()
