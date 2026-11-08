"""Freeze a reference sample using ZIP entry metadata before inspecting outcomes."""

import argparse
import gzip
import hashlib
import json
from pathlib import Path
import zipfile


SEED = "deltasieve-icde2027-reference-v1"


def encode(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode("utf-8")


def select(entries, corpus_id, seed, per_quartile):
    """Use size-ranked quartiles and hash priorities independent of result status."""
    ordered = sorted(entries, key=lambda entry: (entry["bytes"], entry["member_name"], entry["archive_index"]))
    chosen = []
    strata = []
    for quartile in range(4):
        start = len(ordered) * quartile // 4
        end = len(ordered) * (quartile + 1) // 4
        members = ordered[start:end]
        ranked = []
        for member in members:
            identity = f'{corpus_id}:{member["archive_index"]}:{member["member_name"]}'
            priority = hashlib.sha256(f"{seed}\0{identity}".encode("utf-8")).hexdigest()
            ranked.append({**member, "identity": identity, "quartile": quartile + 1,
                           "group_id": hashlib.sha256(identity.encode("utf-8")).hexdigest()[:20],
                           "selection_priority": priority})
        selected = sorted(ranked, key=lambda member: member["selection_priority"])[:per_quartile]
        chosen.extend(selected)
        strata.append({"quartile": quartile + 1, "population": len(members), "selected": len(selected),
                       "smallest_bytes": members[0]["bytes"] if members else None,
                       "largest_bytes": members[-1]["bytes"] if members else None})
    return ordered, chosen, strata


def freeze(archive_path, corpus_id, output, seed=SEED, per_quartile=3):
    """Publish the complete frame and selection last, without extracting workbooks."""
    if len(corpus_id) != 64 or any(char not in "0123456789abcdef" for char in corpus_id):
        raise ValueError("Corpus identity must be a lowercase SHA-256")
    if per_quartile < 1:
        raise ValueError("Per-quartile sample count must be positive")
    before = archive_path.stat()
    with zipfile.ZipFile(archive_path) as archive:
        entries = [{"archive_index": index, "member_name": entry.filename,
                    "bytes": entry.file_size, "crc32": entry.CRC}
                   for index, entry in enumerate(archive.infolist())
                   if not entry.is_dir() and Path(entry.filename).suffix.lower() in (".xls", ".xlsx")]
    if not entries:
        raise ValueError("Archive has no direct XLS/XLSX members")
    frame, chosen, strata = select(entries, corpus_id, seed, per_quartile)
    with archive_path.open("rb") as source:
        archive_sha256 = hashlib.file_digest(source, "sha256").hexdigest()
    after = archive_path.stat()
    if (before.st_size, before.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
        raise ValueError("Archive changed while its sample was frozen")
    output.mkdir(parents=True, exist_ok=False)
    frame_bytes = b"".join(encode(member) + b"\n" for member in frame)
    with (output / "frame.jsonl.gz").open("xb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
            compressed.write(frame_bytes)
    manifest = {
        "version": 1, "status": "selection_frozen", "corpus_id": corpus_id,
        "archive_name": archive_path.name, "archive_sha256": archive_sha256,
        "frame": "frame.jsonl.gz", "frame_uncompressed_sha256": hashlib.sha256(frame_bytes).hexdigest(),
        "seed": seed, "per_quartile": per_quartile, "population": len(frame),
        "selection_inputs": "Archive member index, name, uncompressed bytes and fixed seed only",
        "strata": strata, "selected": chosen,
        "limits": [
            "The population contains direct XLS/XLSX archive occurrences, including duplicates and invalid inputs.",
            "Conversion, reference timeout and unsupported-area failures remain in the selected denominator.",
            "Prior development exposure is assessed separately without replacing selected members."
        ],
    }
    (output / "selection.json").write_bytes(encode(manifest))
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--corpus-id", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--seed", default=SEED)
    parser.add_argument("--per-quartile", type=int, default=3)
    args = parser.parse_args()
    result = freeze(args.archive, args.corpus_id, args.output, args.seed, args.per_quartile)
    print(json.dumps({"status": result["status"], "population": result["population"],
                      "selected": len(result["selected"]), "archive_sha256": result["archive_sha256"]}))


if __name__ == "__main__":
    main()
