"""Copy prepared Original XLSX files into equal-count, size-ordered folders."""
import argparse
import csv
import hashlib
import json
from pathlib import Path, PurePosixPath
import time
import zipfile


def checksum(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def partition_counts(count, parts):
    if parts < 1:
        raise ValueError("The number of folders must be positive")
    quotient, remainder = divmod(count, parts)
    return [quotient + (index < remainder) for index in range(parts)]


def copy_originals(inventory, output, parts):
    with inventory.open(encoding="utf-8-sig", newline="") as stream:
        rows = sorted(csv.DictReader(stream), key=lambda r: (int(r["original_xlsx_bytes"]), r["case_id"]))
    if not rows or len({r["case_id"] for r in rows}) != len(rows):
        raise ValueError("Inventory must contain distinct workbook identities")
    counts = partition_counts(len(rows), parts)
    plan = {"inventory": str(inventory.resolve()), "inventory_sha256": checksum(inventory),
            "member": "original.xlsx", "workbooks": len(rows), "folder_counts": counts,
            "order": "Original XLSX bytes ascending, then case identity"}
    output.mkdir(parents=True, exist_ok=True)
    plan_path = output / "copy-plan.json"
    if plan_path.exists():
        if json.loads(plan_path.read_bytes()) != plan:
            raise ValueError("Existing destination has a different copy plan")
    elif any(output.iterdir()):
        raise ValueError("Destination must be empty or belong to this copy plan")
    plan_path.write_text(json.dumps(plan, indent=2) + "\n", encoding="utf8")
    started = time.monotonic()
    copied = reused = 0
    errors, folders = [], []
    rank = 0
    with (output / "manifest.csv").open("w", newline="", encoding="utf-8-sig") as manifest:
        writer = csv.DictWriter(manifest, fieldnames=[
            "rank", "folder", "file", "case_id", "source_member", "source_bundle", "bytes", "sha256"])
        writer.writeheader()
        for index, count in enumerate(counts, 1):
            folder = output / f"part-{index:02d}"
            folder.mkdir(exist_ok=True)
            sizes = []
            for row in rows[rank:rank + count]:
                rank += 1
                name = PurePosixPath(row["source_member"]).with_suffix(".xlsx").name
                target = folder / f"{rank:05d}_{name}"
                target.resolve().relative_to(output.resolve())
                pending = target.with_suffix(".xlsx.partial")
                try:
                    digest = hashlib.sha256()
                    with zipfile.ZipFile(row["bundle"]) as archive:
                        info = archive.getinfo("original.xlsx")
                        if info.file_size != int(row["original_xlsx_bytes"]):
                            raise ValueError("Source size differs from inventory")
                        with archive.open(info) as source, pending.open("wb") as destination:
                            for chunk in iter(lambda: source.read(1024 * 1024), b""):
                                destination.write(chunk)
                                digest.update(chunk)
                    sha = digest.hexdigest()
                    if pending.stat().st_size != info.file_size or checksum(pending) != sha:
                        raise ValueError("Copied file checksum differs")
                    if target.exists():
                        if checksum(target) != sha:
                            raise ValueError("Existing destination differs from source")
                        pending.unlink()
                        reused += 1
                    else:
                        pending.replace(target)
                        copied += 1
                    writer.writerow({"rank": rank, "folder": folder.name, "file": target.name,
                                     "case_id": row["case_id"], "source_member": row["source_member"],
                                     "source_bundle": row["bundle"], "bytes": info.file_size, "sha256": sha})
                    sizes.append(info.file_size)
                except (OSError, ValueError, KeyError, zipfile.BadZipFile) as error:
                    pending.unlink(missing_ok=True)
                    errors.append({"case_id": row["case_id"], "destination": str(target), "error": str(error)})
                if rank % 500 == 0:
                    manifest.flush()
                    print(json.dumps({"processed": rank, "total": len(rows), "copied": copied,
                                      "reused": reused, "errors": len(errors)}), flush=True)
            actual = len(list(folder.glob("*.xlsx")))
            folders.append({"folder": folder.name, "expected": count, "verified": len(sizes),
                            "actual_xlsx_files": actual, "bytes": sum(sizes),
                            "minimum_bytes": min(sizes, default=None), "maximum_bytes": max(sizes, default=None)})
    complete = not errors and all(f["verified"] == f["actual_xlsx_files"] == f["expected"] for f in folders)
    summary = {"state": "complete" if complete else "incomplete", "workbooks": len(rows),
               "copied": copied, "reused": reused, "elapsed_seconds": time.monotonic() - started,
               "folders": folders, "errors": errors}
    (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf8")
    lines = ["# Enron Original XLSX partitions", "",
             "Prepared Original XLSX files, sorted by byte size ascending and split into consecutive equal-count groups.",
             "Files retain their source names with a global sort number. The manifest records source bundles, names and SHA-256 checksums.",
             "", "| Folder | Verified files | Minimum bytes | Maximum bytes |", "| --- | ---: | ---: | ---: |"]
    lines += [f"| {f['folder']} | {f['verified']} | {f['minimum_bytes']} | {f['maximum_bytes']} |" for f in folders]
    (output / "README.md").write_text("\n".join(lines) + "\n", encoding="utf8")
    print(json.dumps(summary), flush=True)
    return summary


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--inventory", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--parts", type=int, default=6)
    args = parser.parse_args()
    result = copy_originals(args.inventory, args.output, args.parts)
    raise SystemExit(0 if result["state"] == "complete" else 1)
