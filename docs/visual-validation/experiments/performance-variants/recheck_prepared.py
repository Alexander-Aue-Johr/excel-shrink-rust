"""Recompute Default outputs without changing a live study or its prepared bundles.

The SQLite journal binds every result to the immutable binary and bundle hash.
Only a small bounded set of workers runs concurrently. Reused comparisons may be
considered later only for entries whose complete decompressed package is equal.
This script itself never imports results or changes scientific verdicts.
"""

import argparse
from concurrent.futures import ThreadPoolExecutor, wait, FIRST_COMPLETED
import hashlib
import json
from pathlib import Path
import shutil
import sqlite3
import uuid
import zipfile

from compare_variants import member_identities, sha256, shrink


def copy_member(archive, member, target, expected):
    with archive.open(member) as source, target.open("xb") as destination:
        shutil.copyfileobj(source, destination, 128 * 1024)
    if sha256(target) != expected:
        raise ValueError(f"Prepared member hash mismatch for {member}")


def recheck(bundle, binary, work):
    before = sha256(bundle)
    work.mkdir(parents=True, exist_ok=False)
    with zipfile.ZipFile(bundle) as archive:
        if len(archive.namelist()) != len(set(archive.namelist())):
            raise ValueError("Duplicate preparation bundle members")
        info = archive.getinfo("preparation.json")
        if info.file_size > 1024 * 1024:
            raise ValueError("Oversized preparation metadata")
        metadata = json.loads(archive.read(info))
        if metadata.get("version") != 2:
            raise ValueError("Unsupported preparation metadata version")
        group = hashlib.sha256(metadata["identity"].encode("utf-8")).hexdigest()[:20]
        if group != bundle.stem:
            raise ValueError("Preparation identity disagrees with its group filename")
        variants = [item for item in metadata["variants"] if item["profile"] == "default"]
        if len(variants) != 1:
            raise ValueError("Expected exactly one Default variant")
        variant = variants[0]
        copy_member(archive, metadata["converted_member"], work / "converted.xlsx",
                    metadata["converted_sha256"])
        copy_member(archive, variant["member"], work / "previous.xlsx", variant["sha256"])
        copy_member(archive, "original.xlsx", work / "baseline.xlsx", metadata["baseline_sha256"])
    seconds, candidate = shrink(binary, work / "converted.xlsx", work / "output")
    previous = member_identities(work / "previous.xlsx")
    changed = sorted(name for name in previous.keys() | candidate.keys()
                     if previous.get(name) != candidate.get(name))
    if sha256(bundle) != before:
        raise ValueError("Preparation bundle changed during inspection")
    return {
        "status": "verified", "group_id": bundle.stem, "bundle_sha256": before,
        "source_sha256": metadata["source_sha256"],
        "converted_sha256": metadata["converted_sha256"],
        "baseline_sha256": metadata["baseline_sha256"],
        "previous_producer": metadata["prepared_by"],
        "default_config_hash": variant["config_hash"],
        "previous_default_sha256": variant["sha256"],
        "new_default_sha256": sha256(work / "output" / "converted.xlsx"),
        "equal_package_members": not changed, "changed_members": changed,
        "member_count": len(candidate), "shrink_seconds": seconds, "work": str(work),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--prepared-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--group", action="append", default=[])
    args = parser.parse_args()
    if not 1 <= args.workers <= 16:
        parser.error("workers must be between 1 and 16")
    args.output.mkdir(parents=True, exist_ok=True)
    bundles = sorted(args.prepared_root.glob("*.zip"))
    if args.group:
        bundles = [path for path in bundles if path.stem in args.group]
        if {path.stem for path in bundles} != set(args.group):
            parser.error("A requested group has no preparation bundle")
    if not bundles:
        parser.error("No prepared bundles selected")
    identity = {"binary_sha256": sha256(args.binary),
                "prepared_root": str(args.prepared_root.resolve()),
                "policy": "complete-member-sha256-v1", "profile": "default"}
    database = sqlite3.connect(args.output / "recheck.sqlite3")
    database.execute("PRAGMA journal_mode=WAL")
    database.execute("CREATE TABLE IF NOT EXISTS metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
    database.execute("CREATE TABLE IF NOT EXISTS checks (id TEXT PRIMARY KEY, result TEXT NOT NULL)")
    saved = database.execute("SELECT value FROM metadata WHERE key='identity'").fetchone()
    if saved and json.loads(saved[0]) != identity:
        raise ValueError("The existing journal belongs to a different binary or source")
    database.execute("INSERT OR IGNORE INTO metadata VALUES ('identity', ?)", (json.dumps(identity),))
    database.commit()
    summary_path = args.output / "summary.json"
    summary_path.write_text(json.dumps({**identity, "status": "running", "selected": len(bundles)}),
                            encoding="utf-8")
    pending = []
    for bundle in bundles:
        old = database.execute("SELECT result FROM checks WHERE id=?", (bundle.stem,)).fetchone()
        if old:
            result = json.loads(old[0])
            if result["status"] == "verified" and result["bundle_sha256"] == sha256(bundle):
                continue
        pending.append(bundle)
    remaining = iter(pending)
    finished = len(bundles) - len(pending)
    with ThreadPoolExecutor(max_workers=args.workers) as pool:
        active = {}
        def admit():
            if shutil.disk_usage(args.output).free < 32 * 1024 ** 3:
                raise RuntimeError("Preserve at least 32 GiB free on the working drive")
            bundle = next(remaining, None)
            if bundle is None:
                return False
            work = args.output / "jobs" / bundle.stem / uuid.uuid4().hex
            active[pool.submit(recheck, bundle, args.binary, work)] = bundle
            return True
        while len(active) < args.workers and admit():
            pass
        while active:
            done, _ = wait(active, return_when=FIRST_COMPLETED)
            for future in done:
                bundle = active.pop(future)
                try:
                    result = future.result()
                except Exception as error:
                    result = {"status": "failed", "group_id": bundle.stem, "error": str(error)}
                database.execute("INSERT OR REPLACE INTO checks VALUES (?, ?)",
                                 (bundle.stem, json.dumps(result)))
                database.commit()
                finished += 1
                if finished % 100 == 0 or result["status"] == "failed":
                    print(f"Checked {finished}/{len(bundles)}; {bundle.stem} {result['status']}", flush=True)
                admit()
    selected = {bundle.stem for bundle in bundles}
    results = []
    for group, raw in database.execute("SELECT id, result FROM checks"):
        if group in selected:
            results.append(json.loads(raw))
    summary = {**identity, "status": "completed", "selected": len(bundles),
               "verified": sum(row["status"] == "verified" for row in results),
               "changed_groups": [row["group_id"] for row in results
                                  if row["status"] == "verified" and not row["equal_package_members"]],
               "failed_groups": [row["group_id"] for row in results if row["status"] == "failed"]}
    if summary["failed_groups"]:
        summary["status"] = "completed_with_failures"
    temporary = summary_path.with_suffix(".pending")
    temporary.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    temporary.replace(summary_path)
    database.close()
    print(json.dumps({key: value for key, value in summary.items() if not isinstance(value, list)}), flush=True)
    if summary["failed_groups"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
