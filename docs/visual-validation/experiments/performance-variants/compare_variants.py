"""Compare two immutable shrink binaries on hash-identified inputs.

Each repetition alternates execution order. Output identity is checked using
the decompressed members, with duplicate member names rejected. Timings include
process startup and output writing. Concurrent workload must be recorded by the
caller and prevents treating these observations as controlled speedup evidence.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import sys
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from process_metrics import run_measured


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def member_identities(path):
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)):
            raise ValueError(f"Duplicate ZIP members in {path}")
        identities = {}
        for name in sorted(names):
            with archive.open(name) as stream:
                identities[name] = hashlib.file_digest(stream, "sha256").hexdigest()
        return identities


def shrink(binary, source, destination):
    destination.mkdir(parents=True)
    command = [str(binary), str(source), str(destination),
               "--rayon-threads", "1", "--max-active-workbooks", "1",
               "--writer-workers", "1"]
    measurement = run_measured(command, destination, timeout=300)
    outputs = list(destination.rglob("*.xlsx"))
    if len(outputs) != 1:
        raise ValueError(f"Expected one output in {destination}, got {len(outputs)}")
    return measurement, member_identities(outputs[0])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True, type=Path)
    parser.add_argument("--candidate", required=True, type=Path)
    parser.add_argument("--inputs", required=True, type=Path)
    parser.add_argument("--input-root", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--repetitions", type=int, default=2)
    parser.add_argument("--workload-note", required=True)
    args = parser.parse_args()
    if os.name == "nt":
        for field in ("baseline", "candidate", "inputs", "input_root", "output"):
            path = str(getattr(args, field).resolve())
            if not path.startswith("\\\\?\\"):
                setattr(args, field, Path("\\\\?\\" + path))
    if args.repetitions < 1:
        parser.error("repetitions must be positive")
    args.output.mkdir(parents=True, exist_ok=False)
    inputs = json.loads(args.inputs.read_text(encoding="utf-8"))["workbooks"]
    report = {
        "status": "running", "repetitions": args.repetitions,
        "workload_note": args.workload_note,
        "baseline_sha256": sha256(args.baseline),
        "candidate_sha256": sha256(args.candidate),
        "threads": 1, "rows": [],
    }
    report_path = args.output / "comparison.json"
    try:
        for item in inputs:
            name = item["name"]
            if Path(name).name != name:
                raise ValueError("Input names must be filenames")
            source = args.input_root / name
            if sha256(source) != item["input_sha256"]:
                raise ValueError(f"Input identity mismatch for {name}")
            row = {"name": name, "input_sha256": item["input_sha256"], "observations": []}
            report["rows"].append(row)
            for repetition in range(args.repetitions):
                order = ("baseline", "candidate") if repetition % 2 == 0 else ("candidate", "baseline")
                results = {}
                for kind in order:
                    results[kind] = shrink(getattr(args, kind), source,
                                           args.output / source.stem / str(repetition) / kind)
                if results["baseline"][1] != results["candidate"][1]:
                    raise ValueError(f"Decompressed output mismatch for {name}")
                row["observations"].append({
                    "order": order, "baseline_seconds": results["baseline"][0]["seconds"],
                    "candidate_seconds": results["candidate"][0]["seconds"],
                    "baseline_resources": results["baseline"][0]["resources"],
                    "candidate_resources": results["candidate"][0]["resources"],
                    "identical_members": len(results["baseline"][1]),
                })
                report_path.write_text(json.dumps(report, indent=2), encoding="utf-8")
            for kind in ("baseline", "candidate"):
                row[f"{kind}_median_seconds"] = statistics.median(
                    observation[f"{kind}_seconds"] for observation in row["observations"])
            print(f"Verified {name}", flush=True)
        report["status"] = "output_identity_verified"
    except Exception as error:
        report["status"] = "failed"
        report["error"] = str(error)
        raise
    finally:
        report_path.write_text(json.dumps(report, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
