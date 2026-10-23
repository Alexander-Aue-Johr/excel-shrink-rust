#!/usr/bin/env python3
r"""Compare complete study wall time, accepting only equal, complete evidence.

Example (explicit invocation starts native Excel through the two CLIs):
  python compare_study_runtime.py --baseline-cli C:/baseline/target/release/excel-shrink-cli.exe \
    --candidate-cli C:/candidate/target/release/excel-shrink-cli.exe \
    --corpus Sample C:/data/sample.zip --output C:/results/study-ab --repeats 2

Each invocation has a fresh study directory and validation cache. The clock spans
shrink preparation, Excel opens, validation, report writing and process cleanup.
AB/BA ordering reduces order bias; OS and Office caches are not flushed. This is
not a cold-start benchmark. Different coverage, diagnoses or decoded evidence
invalidate a pair. An unchanged stronger profile may intentionally leave weaker
profiles untested, but that coverage must match too. No existing study is resumed.
Requires Pillow for decoded RGBA hashes; no native run happens on import or tests.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import statistics
import subprocess
import sys
import time
import zipfile

PROFILES = ("too-aggressive-cleaning", "maximum-cleaning", "default")
PROFILE_FIELDS = ("profile", "config_hash", "status", "checks_complete", "checked_sheets",
                  "unchanged_sheets", "requested_sheets", "value_differences",
                  "stored_value_differences", "recalculated_value_differences", "image_comparison")
GOOD_STATUSES = {"visually unchanged", "visual differences", "cell value differences"}


def digest_file(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def canonical(value) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False)


def save_json(path: Path, value) -> None:
    pending = path.with_suffix(path.suffix + ".pending")
    pending.write_text(json.dumps(value, indent=2, ensure_ascii=False, allow_nan=False) + "\n", encoding="utf-8")
    os.replace(pending, path)


def pixel_fingerprint(path: Path) -> dict:
    from PIL import Image
    with Image.open(path) as image:
        if image.format != "PNG":
            raise ValueError(f"evidence is not a PNG: {path}")
        rgba = image.convert("RGBA")
        return {"width":rgba.width, "height":rgba.height,
                "rgba_sha256":hashlib.sha256(rgba.tobytes()).hexdigest()}


def evidence(value, output: Path, images: dict):
    """Retain geometry and pixels, normalize artifact names and set-like ordering."""
    if isinstance(value, list):
        return [evidence(item, output, images) for item in value]
    if not isinstance(value, dict):
        return value
    result = {}
    for key, item in value.items():
        if key in ("provenance", "producer", "elapsed_seconds") or key.endswith("_seconds"):
            continue
        if key.endswith("_image") and item is not None:
            path = (output / item).resolve()
            if not path.is_relative_to(output.resolve()):
                raise ValueError(f"image escapes study output: {item}")
            if path not in images:
                images[path] = pixel_fingerprint(path)
            result[key] = images[path]
        else:
            normalized = evidence(item, output, images)
            result[key] = sorted(normalized, key=canonical) if key in ("sheets", "row_heights", "image_differences") and isinstance(normalized, list) else normalized
    return result


def profile_signature(profile: dict, output: Path, images: dict) -> dict:
    result = {key:profile.get(key) for key in PROFILE_FIELDS}
    result["sheets"] = sorted((evidence(sheet, output, images) for sheet in profile.get("sheets", [])), key=canonical)
    return result


def record_signature(record: dict, output: Path) -> dict:
    images = {}
    diagnosis = record.get("whitespace_diagnosis") or {"status":"not_investigated"}
    controls = []
    for control in diagnosis.get("profiles", []):
        controls.append({
            "profile":control.get("profile"), "base_config_hash":control.get("base_config_hash"),
            "control_config_hash":control.get("control_config_hash"),
            "sheets":sorted((evidence(s, output, images) for s in control.get("sheets", [])), key=canonical),
            "control":profile_signature(control.get("control", {}), output, images),
        })
    return {
        "identity":record["identity"], "source_sha256":record.get("source_sha256"),
        "baseline_sha256":record.get("baseline_sha256"), "baseline_repaired":record.get("baseline_repaired"),
        "status":record["status"], "profiles":sorted((profile_signature(p, output, images) for p in record["results"]), key=canonical),
        "whitespace_diagnosis":{"status":diagnosis.get("status"), "profiles":sorted(controls, key=canonical)},
    }


def validate_record(record: dict) -> None:
    identity = record.get("identity", "<missing identity>")
    if record.get("complete") is not True or record.get("status") not in ("unchanged", "differences") or record.get("error"):
        raise ValueError(f"incomplete or technically failed workbook: {identity}")
    results = record.get("results")
    if not isinstance(results, list) or len(results) != 3 or {p.get("profile") for p in results} != set(PROFILES):
        raise ValueError(f"missing or duplicate profile results: {identity}")
    if any(not isinstance(p.get("config_hash"), str) or not p["config_hash"] for p in results):
        raise ValueError(f"missing profile settings: {identity}")
    by_name = {p["profile"]:p for p in results}
    followup_needed = True
    for name in PROFILES:
        profile = by_name[name]
        if profile.get("status") == "not compared":
            if followup_needed or profile.get("checks_complete") is not False or profile.get("sheets"):
                raise ValueError(f"missing required comparison: {identity}/{name}")
            continue
        if profile.get("checks_complete") is not True or profile.get("status") not in GOOD_STATUSES:
            raise ValueError(f"incomplete profile comparison: {identity}/{name}")
        if any(not isinstance(profile.get(key), int) or profile[key] < 0 for key in ("value_differences", "stored_value_differences", "recalculated_value_differences")):
            raise ValueError(f"missing value comparison counts: {identity}/{name}")
        sheets = profile.get("sheets")
        if not isinstance(sheets, list) or not isinstance(profile.get("checked_sheets"), int) or profile["checked_sheets"] < max(1,len(sheets)):
            raise ValueError(f"missing sheet coverage: {identity}/{name}")
        indices = [s.get("index") for s in sheets]
        if any(not isinstance(i, int) or i < 1 for i in indices) or len(indices) != len(set(indices)):
            raise ValueError(f"invalid sheet coverage: {identity}/{name}")
        if any(s.get("status") not in ("same", "changed") for s in sheets):
            raise ValueError(f"technically failed sheet: {identity}/{name}")
        for sheet in sheets:
            changes = sheet.get("row_heights", []) + sheet.get("image_differences", [])
            if sheet["status"] == "changed" and not changes:
                raise ValueError(f"changed sheet lacks evidence: {identity}/{name}")
            if any(not all(isinstance(change.get(key), str) and change[key] for key in ("original_image", "shrunk_image")) for change in changes):
                raise ValueError(f"difference lacks both PNG images: {identity}/{name}")
        followup_needed = any(s.get("status") == "changed" for s in sheets)
    changed = any(p.get("status") in ("visual differences", "cell value differences") for p in results)
    if changed != (record["status"] == "differences"):
        raise ValueError(f"workbook/profile status mismatch: {identity}")
    diagnosis = record.get("whitespace_diagnosis") or {"status":"not_investigated"}
    if diagnosis.get("status") not in ("not_investigated", "complete"):
        raise ValueError(f"incomplete whitespace diagnosis: {identity}")
    if diagnosis["status"] == "complete":
        expected = {p["profile"] for p in results if p["profile"] != "default" and any(s.get("image_differences") and not s.get("row_heights") for s in p.get("sheets", []))}
        actual = [p.get("profile") for p in diagnosis.get("profiles", [])]
        if len(actual) != len(set(actual)) or set(actual) != expected:
            raise ValueError(f"missing whitespace diagnosis coverage: {identity}")
    for profile in diagnosis.get("profiles", []):
        if profile.get("control", {}).get("checks_complete") is not True:
            raise ValueError(f"incomplete whitespace control: {identity}")
        if any(s.get("checks_complete") is not True or s.get("category") not in ("unresolved", "height_unchecked", "whitespace_explained", "other") for s in profile.get("sheets", [])):
            raise ValueError(f"incomplete whitespace classification: {identity}")


def expected_members(corpora: list, limit: int | None) -> dict:
    members = {}
    for name, path in corpora:
        corpus_id = hashlib.sha256(str(path).encode()).hexdigest()
        selected = 0
        with zipfile.ZipFile(path) as archive:
            for index, entry in enumerate(archive.infolist()):
                if entry.is_dir() or not entry.filename.lower().endswith((".xlsx", ".xls")):
                    continue
                if limit is not None and selected >= limit:
                    break
                with archive.open(entry) as stream:
                    sha256 = hashlib.file_digest(stream, "sha256").hexdigest()
                identity = f"{corpus_id}:{index}:{entry.filename}"
                members[identity] = {"corpus":name, "archive_index":index, "member":entry.filename,
                                     "bytes":entry.file_size, "source_sha256":sha256}
                selected += 1
        if not selected:
            raise ValueError(f"no selected XLSX/XLS members in {path}")
    return members


def validate_output(output: Path, expected: dict, exit_code: int) -> tuple[list, list]:
    records = [json.loads(path.read_text(encoding="utf-8-sig")) for path in sorted((output / "records").glob("*.json"))]
    identities = [record.get("identity") for record in records]
    if len(identities) != len(set(identities)) or set(identities) != set(expected):
        raise ValueError(f"workbook coverage differs: expected {len(expected)}, got {len(identities)}; missing={sorted(set(expected)-set(identities))[:3]}")
    if exit_code not in (0, 1) or (exit_code == 1 and not any(r.get("status") == "differences" for r in records)):
        raise ValueError(f"unexpected CLI exit code {exit_code}")
    for record in records:
        validate_record(record)
        if record.get("source_sha256") != expected[record["identity"]]["source_sha256"]:
            raise ValueError(f"source bytes changed: {record['identity']}")
    signature = sorted((record_signature(record, output) for record in records), key=lambda r:r["identity"])
    return records, signature


def cache_observations(run: Path) -> dict:
    # These are observations, not proof of zero cache hits when logging is absent.
    hits, reused = [], 0
    logs = [run / "console.log", *run.glob("**/*.log"), *run.glob("**/*.jsonl")]
    for path in sorted(set(logs)):
        if not path.is_file():
            continue
        with path.open(encoding="utf-8-sig", errors="replace") as stream:
            for line in stream:
                if re.search(r"cache[ _-]*hit", line, re.IGNORECASE):
                    hits.append({"log":str(path.relative_to(run)), "line":line.strip()[:500]})
                match = re.search(r"reused (\d+) prepared workbooks", line)
                if match:
                    reused += int(match.group(1))
    return {"reported_hit_lines":len(hits), "hit_examples":hits[:30], "reused_preparations":reused,
            "interpretation":"Observed log events only; absence does not establish zero internal hits. Fresh application caches do not flush OS/Office caches."}


def run_one(cli: Path, label: str, repeat: int, corpora: list, output: Path, expected: dict, args) -> dict:
    run = output / f"repeat-{repeat:02d}-{label}"
    run.mkdir()
    study = run / "study"
    command = [str(cli), "regression", "study", "--output-root", str(study)]
    for name, path in corpora:
        command.extend(["--corpus", name, str(path)])
    if args.limit:
        command.extend(["--limit", str(args.limit)])
    command.extend(["--workbook-timeout-seconds", str(args.workbook_timeout_seconds),
                    "--startup-timeout-seconds", str(args.startup_timeout_seconds)])
    environment = os.environ.copy()
    environment["EXCEL_SHRINK_REGRESSION_CACHE_DIR"] = str(run / "cache")
    result = {"label":label, "repeat":repeat, "command":command, "study":str(study),
              "cli_sha256":digest_file(cli), "status":"running", "cache_root":environment["EXCEL_SHRINK_REGRESSION_CACHE_DIR"]}
    save_json(run / "measurement.json", result)
    print(f"[{repeat}/{args.repeats}] {label}: {run}", flush=True)
    started = time.perf_counter()
    try:
        if result["cli_sha256"] != getattr(args, "cli_hashes", {}).get(label, result["cli_sha256"]):
            raise ValueError("CLI executable changed between benchmark runs")
        with (run / "console.log").open("wb") as console:
            completed = subprocess.run(command, cwd=run, env=environment, stdout=console, stderr=subprocess.STDOUT, check=False)
        result["wall_seconds"] = time.perf_counter() - started
        result["exit_code"] = completed.returncode
        records, signature = validate_output(study, expected, completed.returncode)
        if result["cli_sha256"] != digest_file(cli):
            raise ValueError("CLI executable changed during benchmark")
        result["workbook_statuses"] = [{"identity":r["identity"],"status":r["status"],
            "profiles":[{"profile":p["profile"],"status":p["status"],"checks_complete":p["checks_complete"]} for p in r["results"]],
            "whitespace_diagnosis_status":(r.get("whitespace_diagnosis") or {}).get("status", "not_investigated")} for r in records]
        save_json(run / "semantic-signature.json", signature)
        result["semantic_sha256"] = hashlib.sha256(canonical(signature).encode()).hexdigest()
        result["status"] = "complete"
    except Exception as error:
        result.setdefault("wall_seconds", time.perf_counter() - started)
        result["status"], result["error"] = "invalid", str(error)
    result["cache_observations"] = cache_observations(run)
    save_json(run / "measurement.json", result)
    return result


def pair_result(repeat: int, runs: list, reference: str | None) -> dict:
    pair = {"repeat":repeat, "status":"invalid"}
    if len(runs) != 2 or any(r["status"] != "complete" for r in runs):
        pair["error"] = "Both studies must complete without technical failures."
        return pair
    by_label = {r["label"]:r for r in runs}
    before, after = by_label["baseline"], by_label["candidate"]
    if before["semantic_sha256"] != after["semantic_sha256"]:
        pair["error"] = "Semantic result/evidence mismatch; inspect the two semantic-signature.json files."
    elif reference and before["semantic_sha256"] != reference:
        pair["error"] = "Semantic result/evidence changed between repeats."
    else:
        pair.update(status="complete", baseline_seconds=before["wall_seconds"], candidate_seconds=after["wall_seconds"],
                    speedup=before["wall_seconds"] / after["wall_seconds"], semantic_sha256=before["semantic_sha256"])
    return pair


def summary(pairs: list) -> dict:
    complete = [p for p in pairs if p["status"] == "complete"]
    return {"complete_pairs":len(complete), "invalid_pairs":len(pairs)-len(complete),
            "median_speedup":statistics.median(p["speedup"] for p in complete) if complete else None,
            "median_baseline_seconds":statistics.median(p["baseline_seconds"] for p in complete) if complete else None,
            "median_candidate_seconds":statistics.median(p["candidate_seconds"] for p in complete) if complete else None}


def positive(value: str) -> int:
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("must be positive")
    return number


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--baseline-cli", required=True, type=Path)
    parser.add_argument("--candidate-cli", required=True, type=Path)
    parser.add_argument("--corpus", required=True, action="append", nargs=2, metavar=("NAME", "ZIP"))
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--repeats", type=positive, default=2)
    parser.add_argument("--limit", type=positive, help="Limit new workbooks per corpus, in archive order")
    parser.add_argument("--workbook-timeout-seconds", type=positive, default=1800)
    parser.add_argument("--startup-timeout-seconds", type=positive, default=120)
    parser.add_argument("--plan-only", action="store_true", help="Write the expected coverage without executing either CLI")
    args = parser.parse_args(argv)
    try:
        from PIL import Image  # Fail before launching Excel if pixel decoding is unavailable.
        clis = {label:path.resolve(strict=True) for label,path in (("baseline",args.baseline_cli),("candidate",args.candidate_cli))}
        corpora = [(name,Path(path).resolve(strict=True)) for name,path in args.corpus]
        if len({name for name,_ in corpora}) != len(corpora) or len({path for _,path in corpora}) != len(corpora):
            raise ValueError("corpus names and archive paths must be unique")
        expected = expected_members(corpora, args.limit)
        output = args.output.resolve()
        output.mkdir(parents=True, exist_ok=False)
        args.cli_hashes = {label:digest_file(path) for label,path in clis.items()}
        report = {"schema_version":1, "measurement":"full study wall time; fresh study and application caches; OS/Office caches retained",
                  "started_unix_seconds":time.time(), "cli":{label:{"path":str(path),"sha256":args.cli_hashes[label]} for label,path in clis.items()},
                  "expected_members":expected, "repeats":args.repeats, "limit":args.limit,
                  "memory_limits":{key:os.environ.get(key) for key in ("EXCEL_SHRINK_WORKER_MEMORY_MIB", "EXCEL_SHRINK_EXCEL_MEMORY_MIB")},
                  "runs":[], "pairs":[], "summary":summary([]), "status":"planned" if args.plan_only else "running"}
        save_json(output / "results.json", report)
        if args.plan_only:
            print(f"Plan only: {len(expected)} workbooks; {output / 'results.json'}")
            return 0
        reference = None
        for repeat in range(1,args.repeats + 1):
            order = ("baseline","candidate") if repeat % 2 else ("candidate","baseline")
            runs = []
            for label in order:
                row = run_one(clis[label],label,repeat,corpora,output,expected,args)
                runs.append(row)
                report["runs"].append(row)
                save_json(output / "results.json", report)
                if row["status"] != "complete":
                    break
            pair = pair_result(repeat,runs,reference)
            report["pairs"].append(pair)
            report["summary"] = summary(report["pairs"])
            if pair["status"] != "complete":
                report["status"] = "invalid"
                save_json(output / "results.json", report)
                print(f"Stopped: {pair['error']}", file=sys.stderr)
                return 1
            reference = pair["semantic_sha256"]
            save_json(output / "results.json", report)
        report["status"] = "complete"
        save_json(output / "results.json", report)
        print(json.dumps(report["summary"], indent=2))
        return 0
    except (OSError, ValueError, ImportError, zipfile.BadZipFile) as error:
        parser.exit(2, f"{error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
