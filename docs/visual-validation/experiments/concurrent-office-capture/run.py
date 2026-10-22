"""Prepare one A/B workload, then measure independent Excel pairs on one hidden desktop."""
import argparse
from collections import defaultdict
import csv
import hashlib
import html
import io
import json
import math
from pathlib import Path
import posixpath
import random
import re
import shutil
import statistics
import subprocess
import sys
import tarfile
import time
import xml.etree.ElementTree as ET
from zipfile import ZipFile

from patch_runtime import prepare_web, process_metrics
from controls import changed_workbook

HERE = Path(__file__).resolve().parent
RUNTIME_REF = "6a0ef2615f922665c4a4f7b4c24e58204f53d6ea"
MAXIMUM_FLAGS = [
    "--remove-textless-default-font-representatives",
    "--remove-textless-style-representatives",
    "--remove-textless-column-style-representatives",
    "--remove-unreferenced-whitespace-cells",
]


def windows_path(path):
    path = str(Path(path).resolve())
    return path if sys.platform == "win32" else subprocess.check_output(
        ["wslpath", "-w", path], text=True).strip()


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def capture_plan(workbook, scopes):
    """Explicit worksheet selections, never inferred from a bloated UsedRange."""
    with ZipFile(workbook) as archive:
        book = ET.fromstring(archive.read("xl/workbook.xml"))
        relations = ET.fromstring(archive.read("xl/_rels/workbook.xml.rels"))
        targets = {r.get("Id"): posixpath.normpath(posixpath.join("xl", r.get("Target")))
                   for r in relations}
        sheets = list(book.find("{*}sheets"))
        result = []
        for scope in scopes:
            name, address = scope.rsplit("!", 1)
            match = re.fullmatch(r"A1:([A-Z]+)([1-9][0-9]*)", address)
            if not match:
                raise ValueError("Scopes must start at A1, e.g. Sheet1!A1:AX1000")
            index = next(i for i, sheet in enumerate(sheets, 1) if sheet.get("name") == name)
            sheet = sheets[index - 1]
            if sheet.get("state", "visible") != "visible":
                raise ValueError("Cannot capture a hidden worksheet")
            relation = next(v for k, v in sheet.attrib.items() if k.endswith("}id"))
            target = targets[relation].lstrip("/")
            node = ET.fromstring(archive.read(target))
            column = 0
            for letter in match[1]:
                column = column * 26 + ord(letter) - 64
            bottom = int(match[2]) - 1
            if not 0 <= bottom < 1048576 or not 1 <= column <= 16384:
                raise ValueError("Scope exceeds Excel bounds")
            right = column - 1
            if any(item["index"] == index for item in result):
                raise ValueError("Use one scope per worksheet")
            result.append({
                "index": index, "name": name, "bottom_row": bottom,
                "base_right_column": right, "capture_right_column": min(16383, right + 32),
                "horizontal_overflow_required": right < 16383,
                "drawing_fallback_required": node.find("{*}drawing") is not None,
            })
    return {"mode": "paired", "workbook": "original.xlsx", "tile_megapixels": 32,
            "sheets": result}


def export_runtime(root, revision, output):
    data = subprocess.check_output(
        ["git", "archive", revision, "excel-shrink-cli/office-capture"], cwd=root)
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        archive.extractall(output, filter="data")
    runtime = output / "excel-shrink-cli/office-capture"
    prepare_web(runtime, runtime / "web", HERE)
    path = runtime / "CaptureProcesses.cs"
    path.write_text(process_metrics(path.read_text()), encoding="utf-8")
    return runtime


def signature(result):
    return [[(d["sheet_index"], d["tile_index"], d["range"], d["difference"]["bounds"])
             for d in pair["differences"]] for pair in result["capture"]["pairs"]]


def publish(output, rows):
    (output / "results.json").write_text(json.dumps(rows, indent=2) + "\n")
    measurements = []
    for row in rows:
        item = {key: row.get(key) for key in [
            "trial", "repeat", "pairs", "transport", "batch_size", "status", "setup_seconds",
            "capture_seconds", "total_seconds", "comparisons_per_minute", "peak_job_bytes",
            "capture_cpu_seconds", "logical_processors", "error"]}
        capture = row.get("capture", {})
        metrics = [client.get("experiment_metrics", {}) for client in capture.get("clients", {}).values()]
        for key in ["office_ms", "capture_ms", "hash_ms", "offer_wait_ms", "upload_wait_ms", "image_bytes", "images"]:
            item[key + "_sum"] = sum(m.get(key, 0) for m in metrics)
        item["effective_max_batch"] = max((m.get("max_batch", 0) for m in metrics), default=0)
        measurements.append(item)
    if measurements:
        with (output / "measurements.csv").open("w", newline="", encoding="utf-8-sig") as stream:
            writer = csv.DictWriter(stream, fieldnames=list(measurements[0]))
            writer.writeheader()
            writer.writerows(measurements)
    groups = defaultdict(list)
    for row in rows:
        if row["status"] == "complete" and row.get("repeat", 0) > 0:
            groups[(row["transport"], row["batch_size"], row["pairs"])].append(row)
    table = []
    for (transport, batch, pairs), values in sorted(groups.items()):
        table.append("<tr><td>" + transport + "</td><td>" + str(batch) + "</td><td>" +
                     str(pairs) + "</td><td>" + str(len(values)) + "</td><td>" +
                     format(statistics.median(v["capture_seconds"] for v in values), ".2f") +
                     "</td><td>" + format(statistics.median(v["comparisons_per_minute"] for v in values), ".2f") +
                     "</td><td>" + format(max(v.get("peak_job_bytes", 0) for v in values) / 2**20, ".0f") +
                     "</td></tr>")
    failures = [r for r in rows if r["status"] != "complete"]
    details = "".join("<li>" + html.escape(str(r.get("trial", "")) + ": " +
                       str(r.get("error", r["status"]))) + "</li>" for r in failures)
    (output / "report.html").write_text(
        '<!doctype html><meta charset="utf-8"><title>Concurrent Excel pairs</title>'
        '<style>body{font:16px system-ui;max-width:1100px;margin:40px auto;color:#153047}'
        'table{border-collapse:collapse;width:100%}td,th{padding:12px;border-bottom:1px solid #ccd}</style>'
        '<h1>Concurrent Excel image comparisons</h1>'
        '<p>One pair means two separate Excel processes. Capture timing starts after all add-ins are ready. '
        'Pilot observations (repeat 0) are excluded from the medians. '
        'Ranges and raw observations are in manifest.json, results.json and measurements.csv.</p>'
        '<table><tr><th>Transport</th><th>Batch</th><th>Pairs</th><th>Complete repeats</th>'
        '<th>Median capture s</th><th>Pairs/min</th><th>Peak job MiB</th></tr>' +
        "".join(table) + '</table><h2>Failed observations</h2><ul>' + details + '</ul>',
        encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, required=True)
    parser.add_argument("--workbook", type=Path, required=True)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--scope", action="append", required=True,
                        help="Explicit selected scope, e.g. ' KHV_2022!A1:FP200'")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--pairs", default="1,2,3,4,5,6,7,8")
    parser.add_argument("--batches", default="1")
    parser.add_argument("--transports", default="images,hash")
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--seed", type=int, default=73021)
    parser.add_argument("--runtime-ref", default=RUNTIME_REF)
    parser.add_argument("--memory-mib", type=int, default=0)
    parser.add_argument("--max-batch-pixels", type=int, default=64_000_000)
    parser.add_argument("--startup-timeout", type=int, default=120)
    parser.add_argument("--capture-timeout", type=int, default=600)
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument("--control", choices=["maximum", "equal", "changed"], default="maximum")
    args = parser.parse_args()
    pairs = [int(n) for n in args.pairs.split(",")]
    batches = [int(n) for n in args.batches.split(",")]
    modes = args.transports.split(",")
    if not pairs or any(n not in range(1, 9) for n in pairs) or any(n not in (1, 2, 4) for n in batches):
        parser.error("Pairs must be 1..8 and batches 1,2,4")
    if any(mode not in ("images", "hash") for mode in modes) or args.repeats < 0:
        parser.error("Invalid transport or repeat count")
    if not 32_000_000 <= args.max_batch_pixels <= 128_000_000:
        parser.error("Batch pixel budget must be between 32 MP and 128 MP")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if (output / "manifest.json").exists():
        parser.error("Choose a new output directory; earlier observations are preserved")
    source, cli, root = args.workbook.resolve(), args.cli.resolve(), args.repo_root.resolve()
    prepared = output / "prepared"
    prepared.mkdir()
    workbook = prepared / "original.xlsx"
    shutil.copyfile(source, workbook)
    capture = capture_plan(workbook, args.scope)
    shrunk = prepared / "maximum.xlsx"
    cleaned = prepared / "cleaned"
    with (output / "shrink.log").open("wb") as log:
        subprocess.run([str(cli), str(workbook), str(cleaned), *MAXIMUM_FLAGS],
                       check=True, stdout=log, stderr=subprocess.STDOUT)
    outputs = list(cleaned.rglob("*.xlsx"))
    if len(outputs) != 1:
        raise RuntimeError("Expected exactly one cleaned workbook")
    outputs[0].rename(shrunk)
    runtime = export_runtime(root, args.runtime_ref, prepared)
    comparison = shrunk
    if args.control == "equal":
        comparison = workbook
    elif args.control == "changed":
        comparison = prepared / "changed.xlsx"
        changed_workbook(workbook, comparison, capture["sheets"][0]["index"])
    experiment = prepared / "experiment"
    (experiment / "host").mkdir(parents=True)
    for name in ["run.ps1", "Memory.cs"]:
        shutil.copyfile(HERE / name, experiment / name)
    for path in (HERE / "host").iterdir():
        if path.suffix in (".cs", ".csproj"):
            shutil.copyfile(path, experiment / "host" / path.name)
    manifest = {"runtime_commit": subprocess.check_output(
        ["git", "rev-parse", args.runtime_ref], cwd=root, text=True).strip(),
        "source": str(source), "source_sha256": digest(workbook), "shrunk_sha256": digest(shrunk),
        "cli_sha256": digest(cli), "maximum_flags": MAXIMUM_FLAGS, "scope": args.scope,
        "capture": capture, "control": args.control, "comparison_sha256": digest(comparison), "seed": args.seed,
        "experiment_sources": {str(p.relative_to(HERE)): digest(p) for p in HERE.rglob("*")
            if p.is_file() and p.suffix in (".py", ".ps1", ".cs", ".csproj", ".js")},
        "timing": "capture includes planning, native rendering, overflow, transport and pixel comparison; excludes startup and shrink"}
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    if args.prepare_only:
        print("Prepared: " + str(output), flush=True)
        return
    rows = []
    expected = None
    # First establish startup at one and two pairs. Only then randomize larger waves.
    waves = [(0, n, 1, mode) for n in pairs if n <= 2 for mode in modes]
    rng = random.Random(args.seed)
    for repeat in range(1, args.repeats + 1):
        wave = [(repeat, n, batch, mode) for n in pairs for batch in batches for mode in modes]
        rng.shuffle(wave)
        waves.extend(wave)
    failed_startups = set()
    for trial, (repeat, count, batch, mode) in enumerate(waves):
        name = f"{trial:03d}-r{repeat}-p{count}-b{batch}-{mode}"
        directory = output / name
        directory.mkdir()
        if count > 2 and mode in failed_startups:
            rows.append({"trial": name, "status": "not-run", "pairs": count, "batch_size": batch,
                         "transport": mode, "error": "Two-pair pilot failed; scaling gate closed"})
            publish(output, rows)
            continue
        request = {"pairs": count, "transport": mode, "batch_size": batch,
                   "max_batch_pixels": args.max_batch_pixels, "capture": capture,
                   "original": windows_path(workbook),
                   "shrunk": windows_path(comparison)}
        request_path = directory / "request.json"
        request_path.write_text(json.dumps(request, ensure_ascii=False, indent=2), encoding="utf-8")
        observed = [row["peak_job_bytes"] / row["pairs"] / 2**20 for row in rows
                    if row["status"] == "complete" and row.get("peak_job_bytes")]
        expected_peak = math.ceil(max(observed) * count * 1.15 + 128) if observed else 0
        command = ["powershell.exe", "-STA", "-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass",
                   "-File", windows_path(experiment / "run.ps1"), "-RequestPath", windows_path(request_path),
                   "-RuntimeRoot", windows_path(runtime), "-OutputDir", windows_path(directory),
                   "-StartupTimeoutSeconds", str(args.startup_timeout),
                   "-CaptureTimeoutSeconds", str(args.capture_timeout), "-MemoryMiB", str(args.memory_mib),
                   "-ExpectedPeakMiB", str(expected_peak)]
        print("Trial " + name, flush=True)
        with (directory / "launcher.log").open("wb") as log:
            completed = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT,
                                       timeout=count * 2 * (args.startup_timeout + 30) + args.capture_timeout + 300)
        result_path = directory / "result.json"
        if result_path.exists():
            row = json.loads(result_path.read_text(encoding="utf-8-sig"))
        else:
            row = {"status": "failed", "error": "Launcher did not publish a result"}
        row.update(trial=name, repeat=repeat, pairs=count, batch_size=batch, transport=mode,
                   exit_code=completed.returncode)
        if row["status"] == "complete":
            actual = signature(row)
            if not all(pair == actual[0] for pair in actual):
                row.update(status="invalid", error="Workers found different visual results")
            elif expected is not None and actual[0] != expected:
                row.update(status="invalid", error="Visual findings differ from the one-pair reference")
            else:
                expected = actual[0]
                if (args.control == "equal" and expected) or (args.control == "changed" and not expected):
                    row.update(status="invalid", error="Known equal/changed control produced the wrong result")
        if repeat == 0 and row["status"] != "complete":
            failed_startups.add(mode)
        rows.append(row)
        (directory / "observation.json").write_text(json.dumps(row, indent=2) + "\n")
        try:
            publish(output, rows)
        except OSError as error:
            print("Report refresh failed; per-trial observations retained: " + str(error), flush=True)
        print(name + ": " + row["status"], flush=True)
    print("Report: " + str(output / "report.html"), flush=True)
    if any(row["status"] != "complete" for row in rows):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
