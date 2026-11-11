"""Export a completed distribution recording using the shared Observatory UI."""

import argparse
from collections import Counter
import gzip
import hashlib
import json
from pathlib import Path
import re
import shutil


def redact_local_paths(value):
    """Replace standalone Windows paths while retaining domain names and numbers."""
    if isinstance(value, dict):
        return {key: redact_local_paths(item) for key, item in value.items()}
    if isinstance(value, list):
        return [redact_local_paths(item) for item in value]
    if isinstance(value, str) and re.match(r"^(?:[A-Za-z]:[\\/]|\\\\)", value):
        return value.replace("\\", "/").rstrip("/").rsplit("/", 1)[-1]
    return value


def export(run, destination, *, redact_paths=False):
    metadata = json.loads((run / "distribution-run.json").read_text(encoding="utf-8"))
    events = run / metadata["replay"]
    previous_time = -1
    count = 0
    outcomes = Counter()
    last = None
    with gzip.open(events, "rt", encoding="utf-8") as stream:
        for line in stream:
            last = json.loads(line)
            if last["sequence"] != count or last["elapsed_micros"] < previous_time:
                raise ValueError("Recording sequence or clock is inconsistent")
            previous_time = last["elapsed_micros"]
            if last.get("operation") != "sheet-shrink":
                raise ValueError("Only SheetShrink work recordings are supported")
            outcomes[last["event_type"]] += 1
            count += 1
    if last is None or last["event_type"] not in ("run.completed", "run.failed"):
        raise ValueError("A terminal recording is required")
    summary = metadata["summary"]
    if (
        outcomes["job.started"] != summary["total"]
        or outcomes["job.completed"] != summary["processed"] + summary["preserved"]
        or outcomes["job.failed"] != summary["skipped"] + summary["failed"]
    ):
        raise ValueError("Recording outcomes disagree with the preparation summary")
    source = {
        "run_id": hashlib.sha256(events.read_bytes()).hexdigest(),
        "operation": "sheet-shrink",
        "status": metadata["ui_terminal_status"],
        "error": None,
        "elapsed_ms": last["elapsed_millis"],
        "next": count,
        "reset": False,
        "jobs": [],
        "tasks": [],
        "events": [],
        "history_limit": 256,
        "defaults": {
            **metadata["config"],
            "inputs": [metadata["corpus"]],
            "output": None,
            "source_manifest": None,
            "reuse_from": None,
        },
        "counts": {
            "total": summary["total"],
            "prepared": 0,
            "failed": 0,
            "reused": 0,
            "before_bytes": 0,
            "after_bytes": 0,
        },
    }
    assets = Path(__file__).resolve().parents[1] / "crates/preparation-web/assets"
    destination.mkdir(parents=True, exist_ok=False)
    for name in ("app.js", "app.css"):
        shutil.copyfile(assets / name, destination / name)
    exported_events = destination / "events.jsonl.gz"
    if redact_paths:
        with gzip.open(events, "rt", encoding="utf-8") as stream, exported_events.open("wb") as output:
            with gzip.GzipFile(filename="", fileobj=output, mode="wb", mtime=0) as compressed:
                for line in stream:
                    projected = redact_local_paths(json.loads(line))
                    compressed.write((json.dumps(projected, separators=(",", ":")) + "\n").encode("utf-8"))
    else:
        shutil.copyfile(events, exported_events)
    manifest = {
        "schema": 1,
        "events_file": "events.jsonl.gz",
        "event_count": count,
        "events_bytes": exported_events.stat().st_size,
        "events_sha256": hashlib.sha256(exported_events.read_bytes()).hexdigest(),
        "source_events_sha256": source["run_id"],
        "path_projection": "standalone_windows_paths_to_basename" if redact_paths else "unchanged",
        "source": source,
        "producer_revision": metadata["code_revision"],
        "producer_sha256": metadata["binary_sha256"],
        "publication_status": "review_required",
        "limitations": [
            "Full event history is loaded into browser memory.",
            "Recorded names, errors and sheet metadata require publication review.",
        ],
    }
    (destination / "recording.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    html = (assets / "index.html").read_text(encoding="utf-8")
    html = html.replace('__TOKEN__', '').replace('__APPLICATION__', 'SheetShrink replay')
    html = html.replace('href="/app.css"', 'href="./app.css"').replace('src="/app.js"', 'src="./app.js"')
    html = html.replace('<title>', '<meta name="observatory-replay" content="./recording.json">\n    <title>')
    (destination / "index.html").write_text(html, encoding="utf-8")
    (destination / ".nojekyll").touch()
    inventory = [
        {
            "file": path.name,
            "bytes": path.stat().st_size,
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        }
        for path in sorted(destination.iterdir())
        if path.is_file()
    ]
    (destination / "inventory.json").write_text(json.dumps(inventory, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"events": count, "destination": str(destination), "publication_status": "review_required"}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--redact-local-paths", action="store_true",
                        help="Replace standalone Windows paths with basenames; publication review remains required.")
    args = parser.parse_args()
    export(args.run, args.destination, redact_paths=args.redact_local_paths)
