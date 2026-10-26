"""Opt-in Windows/Excel A/B check for bounded capture pipelining.

Requires openpyxl and Pillow. Without --run, only prepares a fixed local fixture.
With --run, uses the SAME binary and input in baseline/pipeline/pipeline/baseline
order, in fresh study directories. The baseline sets EXCEL_SHRINK_CAPTURE_PIPELINE=0.
Fresh Excel instances are used throughout; OS/Office caches are not cleared.
This is a local paired experiment, not a statistically conclusive performance test.
Use --pause-check --compare-to <completed-sample> for one separate, untimed check.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import socket
import statistics
import subprocess
import time
import urllib.error
import urllib.request
import zipfile


ROOT = Path(__file__).resolve().parents[3]
OUTPUT = ROOT / ".tmp/capture-pipeline"
NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0)
PROFILES = ("original", "default", "maximum-cleaning", "too-aggressive-cleaning")
SETTINGS = {
    "target_groups": 1,
    "capture_tile_megapixels": 1,
    "capture_batch_tiles": 4,
    "tall_sheet_threshold": 100000,
    "tall_sheet_edge_tiles": 1,
    "equal_preview_width": 512,
    "hide_excel": True,
    "paused": False,
}


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def write_json(path, value):
    pending = path.with_suffix(".pending")
    pending.write_text(json.dumps(value, indent=2), encoding="utf-8")
    pending.replace(path)


def read_json(path):
    return json.loads(path.read_bytes())


def prepare_fixture():
    from openpyxl import Workbook
    from openpyxl.chart import BarChart, Reference
    from openpyxl.formatting.rule import CellIsRule
    from openpyxl.styles import PatternFill

    OUTPUT.mkdir(parents=True, exist_ok=True)
    archive = OUTPUT / "pipeline-fixture-v2.zip"
    if archive.exists():
        return archive
    workbook = Workbook()
    data = workbook.active
    data.title = "Capture bands"
    for row in range(1, 641):
        data.append([row, row * 2, row * 3, 1, 1, 1, 1, 1])
    # The whitespace blocks text overflow in the original/default. Maximum
    # cleaning removes it; the matched control must explain this difference.
    for col in range(1, 9):
        data.cell(120, col).value = None
    data["A120"] = "Whitespace blocks this deliberately long label from overflowing"
    data["B120"] = "    "
    data["D250"].fill = PatternFill(fill_type="solid", fgColor="FFF1B8")
    data.conditional_formatting.add(
        "B400:B405", CellIsRule(
            operator="greaterThan", formula=["0"],
            fill=PatternFill(fill_type="solid", fgColor="B8E9FF"),
        ),
    )
    # Only the chart references these hidden values. The deliberately aggressive
    # profile ignores chart references, giving a non-whitespace visual difference.
    data.column_dimensions["J"].hidden = True
    for row in range(1, 9):
        data.row_dimensions[row].height = 15
    for row in range(1, 7):
        data.cell(row, 10, row * 7)
    data["J8"] = 999  # Unreferenced: even Default changes this worksheet.
    summary = workbook.create_sheet("Hidden data chart")
    summary["A1"] = "Chart values come from the hidden J column"
    chart = BarChart()
    chart.add_data(Reference(data, min_col=10, min_row=1, max_row=6))
    chart.visible_cells_only = False
    chart.title = "Hidden values remain visible here"
    summary.add_chart(chart, "A3")
    summary["P40"] = "End of chart capture area"
    source = OUTPUT / "pipeline-fixture-v2.xlsx"
    workbook.save(source)

    # openpyxl writes inline strings. Use the shared-string representation that
    # whitespace cleanup intentionally targets, without rewriting worksheet XML.
    shared = []

    def shared_cell(match):
        index = len(shared)
        shared.append(match[2])
        attrs = match[1].replace(b' t="inlineStr"', b' t="s"')
        return b"<c" + attrs + b"><v>" + str(index).encode() + b"</v></c>"

    with zipfile.ZipFile(source) as original:
        entries = {name: original.read(name) for name in original.namelist()}
    for name in entries:
        if name.startswith("xl/worksheets/") and name.endswith(".xml"):
            entries[name] = re.sub(rb"<c([^>]* t=\"inlineStr\"[^>]*)><is>(.*?)</is></c>", shared_cell, entries[name])
    entries["xl/sharedStrings.xml"] = (
        b'<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
        + b"".join(b"<si>" + text + b"</si>" for text in shared) + b"</sst>"
    )
    entries["xl/_rels/workbook.xml.rels"] = entries["xl/_rels/workbook.xml.rels"].replace(
        b"</Relationships>",
        b'<Relationship Id="pipelineSharedStrings" Target="sharedStrings.xml" '
        b'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings"/></Relationships>',
    )
    entries["[Content_Types].xml"] = entries["[Content_Types].xml"].replace(
        b"</Types>", b'<Override PartName="/xl/sharedStrings.xml" '
        b'ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/></Types>',
    )
    with zipfile.ZipFile(source, "w", compression=zipfile.ZIP_DEFLATED) as output:
        for name, contents in entries.items():
            output.writestr(name, contents)
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as output:
        output.write(source, source.name)
    return archive


def pixel_digest(path):
    from PIL import Image

    with Image.open(path) as image:
        rgba = image.convert("RGBA")
        content = rgba.width.to_bytes(4, "little") + rgba.height.to_bytes(4, "little") + rgba.tobytes()
        return hashlib.sha256(content).hexdigest()


def result_signature(record, study):
    def sheets(values):
        result = []
        for sheet in values:
            item = {key: sheet.get(key) for key in ("index", "name", "status", "row_heights")}
            differences = []
            for difference in sheet.get("image_differences", []):
                evidence = dict(difference)
                for key in ("original_image", "shrunk_image", "diff_image"):
                    path = Path(evidence[key])
                    evidence[key] = pixel_digest(path if path.is_absolute() else study / path)
                differences.append(evidence)
            item["image_differences"] = sorted(differences, key=lambda value: (value["range"], value["region"]))
            result.append(item)
        return sorted(result, key=lambda value: value["index"])

    profiles = []
    for profile in record["results"]:
        normalized = {key: profile.get(key) for key in (
            "profile", "status", "checks_complete", "checked_sheets", "unchanged_sheets",
            "stored_value_differences", "recalculated_value_differences", "value_differences", "failures",
        )}
        normalized["sheets"] = sheets(profile.get("sheets", []))
        profiles.append(normalized)
    diagnosis = record.get("whitespace_diagnosis", {})
    return {
        "status": record["status"], "results": profiles,
        "whitespace": {
            "status": diagnosis.get("status"),
            "profiles": [{
                "profile": profile["profile"], "sheets": profile["sheets"],
                "control_sheets": sheets(profile["control"].get("sheets", [])),
            } for profile in diagnosis.get("profiles", [])],
        },
    }


def collect_result(study):
    records = [read_json(path) for path in (study / "records").glob("*.json")]
    assert len(records) == 1, "Expected one completed fixture workbook"
    record = records[0]
    assert record.get("complete") and record["status"] == "differences", (
        f"Expected completed visual differences, got status={record.get('status')}, "
        f"complete={record.get('complete')}, error={record.get('error')}; see study/records"
    )
    assert record.get("whitespace_diagnosis", {}).get("status") == "complete", "Whitespace diagnosis did not finish"
    profiles = {profile["profile"]: profile for profile in record["results"]}
    for name, sheet_name in (
        ("maximum-cleaning", "Capture bands"),
        ("too-aggressive-cleaning", "Hidden data chart"),
    ):
        assert any(
            sheet["name"] == sheet_name and sheet.get("image_differences")
            for sheet in profiles[name].get("sheets", [])
        ), f"Expected visual difference missing: {name}, {sheet_name}"
    diagnoses = {profile["profile"]: profile for profile in record["whitespace_diagnosis"]["profiles"]}
    assert any(
        sheet["index"] == 1 and sheet["category"] == "whitespace_explained"
        for sheet in diagnoses["maximum-cleaning"]["sheets"]
    ), "Matched control did not explain the whitespace overflow"
    assert any(
        sheet["index"] == 2 and sheet["category"] == "other"
        for sheet in diagnoses["too-aggressive-cleaning"]["sheets"]
    ), "Hidden-chart regression must remain a non-whitespace finding"
    state = read_json(next((study / "control-room/groups").glob("*/state.json")))
    coverage = []
    for sheet in state["sheets"]:
        assert sheet.get("capture_coverage", {}).get("omitted_tiles", 0) == 0
        for tile in sheet["tiles"]:
            assert tile["status"] in ("equal", "different"), tile
            assert set(tile["images"]) == set(PROFILES), tile
            coverage.append({
                "sheet": sheet["name"],
                **{key: tile[key] for key in ("range", "core_rows", "width", "height", "status")},
                "pixels": {profile: image["pixel_class"] for profile, image in tile["images"].items()},
                "changed_pixels": {profile: image["changed_pixels"] for profile, image in tile["images"].items()},
            })
    assert len(coverage) >= 24, "Fixture must cover enough tiles for several capture batches"
    resident = record["resident_capture"]
    physical = len({resident["capture_client_aliases"][name] for name in PROFILES})
    assert physical == 4, f"Fixture no longer exercises four physical primary clients: {physical}"
    return {
        "signature": {
            "coverage": coverage, "report": result_signature(record, study),
            "capture_counts": {key: state["throughput"][key] for key in (
                "captured_pixels", "core_pixels", "uploaded_images",
            )},
        },
        "metrics": {
            "physical_primary_clients": physical,
            "primary_pids": {name: resident["clients"][name]["pid"] for name in PROFILES},
            "tiles": len(coverage),
            "launch_to_ready_seconds": {name: client.get("startup_seconds") for name, client in resident["clients"].items()},
            "peak_job_bytes": resident.get("peak_job_bytes"),
            "comparing_elapsed_seconds": record.get("comparing_elapsed_seconds"),
            "throughput": state.get("throughput"),
            "pipeline_timings": state.get("pipeline_timings"),
            "capture_pipeline": state.get("capture_pipeline"),
        },
    }


def cleanup_owned_host(study):
    # The Observatory intentionally survives its runner. This test's copy lives
    # inside its fresh study directory; never kill another study's host.
    environment = os.environ.copy()
    environment["PIPELINE_SMOKE_HOST_ROOT"] = str(study / "control-room/host") + os.sep
    environment["PIPELINE_SMOKE_HOST_SETTINGS"] = str(study / "control-room/host-settings.json")
    script = r"""
$ErrorActionPreference = 'Stop'
Get-CimInstance Win32_Process -Filter "Name = 'excel-shrink-cli.exe'" |
    Where-Object {
        $_.ExecutablePath -and $_.CommandLine -and
        $_.ExecutablePath.Replace('\\?\', '').StartsWith($env:PIPELINE_SMOKE_HOST_ROOT, [StringComparison]::OrdinalIgnoreCase) -and
        $_.CommandLine.Contains($env:PIPELINE_SMOKE_HOST_SETTINGS)
    } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction Stop }
"""
    subprocess.run(
        ["powershell.exe", "-NoProfile", "-Command", script], env=environment,
        check=True, creationflags=NO_WINDOW, timeout=30,
    )


def pause_and_resume(state_path, request, process, deadline):
    before = read_json(state_path)
    metrics_path = state_path.parent / "launcher.json"
    clients = read_json(metrics_path)["clients"]
    pids = {name: clients[name]["pid"] for name in PROFILES}
    request("/api/control", {"paused": True})
    requested = time.monotonic()
    stable_since = requested
    previous_count = before["throughput"]["uploaded_images"]
    while True:
        deadline()
        assert process.poll() is None, "Runner ended during the pause check"
        state = read_json(state_path)
        count = state.get("throughput", {}).get("uploaded_images", 0)
        if count != previous_count:
            previous_count = count
            stable_since = time.monotonic()
        clients = read_json(metrics_path)["clients"]
        idle = all(
            not clients[name].get("active_command") and not clients[name].get("monitor", {}).get("active_job")
            for name in PROFILES
        )
        if idle and time.monotonic() - stable_since >= 2:
            break
        time.sleep(.25)
    assert not state.get("primary_complete"), "Pause arrived after all primary captures finished"
    assert state["attempt_id"] == before["attempt_id"], "Pausing restarted the group"
    assert {name: clients[name]["pid"] for name in PROFILES} == pids, "Pausing replaced an Excel process"
    request("/api/control", {"paused": False})
    result = {
        "duration_seconds": time.monotonic() - requested,
        "images_before_pause": before["throughput"]["uploaded_images"],
        "images_after_drain": count, "stable_seconds": time.monotonic() - stable_since,
        "attempt_id": before["attempt_id"], "primary_pids": pids,
    }
    print(json.dumps({"pause_resumed": result}), flush=True)
    return result


def run_sample(args, archive, directory, enabled):
    directory.mkdir()
    study = directory / "study"
    command = [
        str(args.binary), "regression", "study", "--corpus", "Capture pipeline smoke", str(archive),
        "--output-root", str(study), "--resident-capture", "--control-room",
        "--control-room-port", str(args.port), "--comparison-workers", "0",
        "--startup-timeout-seconds", "120", "--workbook-timeout-seconds", "180",
        "--machine-label", "isolated capture pipeline smoke", "--hide-excel",
    ]
    environment = os.environ.copy()
    environment.update(
        EXCEL_SHRINK_CAPTURE_PIPELINE="1" if enabled else "0",
        EXCEL_SHRINK_CAPTURE_TILE_MEGAPIXELS="1", EXCEL_SHRINK_CAPTURE_BATCH_TILES="4",
        EXCEL_SHRINK_TALL_SHEET_THRESHOLD="100000", EXCEL_SHRINK_TALL_SHEET_EDGE_TILES="1",
        EXCEL_SHRINK_RESIDENT_VISIBLE="0",
    )
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", args.port))
    receipt = {
        "status": "running", "pipeline": enabled, "command": command, "settings": SETTINGS,
        "binary_sha256": digest(args.binary), "input_sha256": digest(archive),
    }
    write_json(directory / "receipt.json", receipt)
    started = time.monotonic()
    process = None

    def deadline():
        if time.monotonic() - started > args.deadline:
            raise TimeoutError("Absolute per-sample deadline reached")

    def request(path, body=None):
        req = urllib.request.Request(
            f"http://127.0.0.1:{args.port}{path}",
            data=None if body is None else json.dumps(body).encode(),
            headers={"Content-Type": "application/json"},
        )
        with urllib.request.urlopen(req, timeout=3) as response:
            return json.load(response)

    try:
        with (directory / "stdout.log").open("wb") as out, (directory / "stderr.log").open("wb") as err:
            process = subprocess.Popen(command, cwd=ROOT, stdout=out, stderr=err, env=environment, creationflags=NO_WINDOW)
        receipt["pid"] = process.pid
        while True:
            deadline()
            if process.poll() is not None:
                raise RuntimeError(f"Runner exited before controls were ready: {process.returncode}")
            try:
                state = request("/api/state")
                if state.get("runner_pid") == process.pid and state.get("running"):
                    assert state["controls"]["target_groups"] == 0
                    break
            except (urllib.error.URLError, OSError, KeyError):
                pass
            time.sleep(.25)
        request("/api/control", SETTINGS)
        receipt["control_start_seconds"] = time.monotonic() - started
        while process.poll() is None:
            deadline()
            if args.pause_check and "pause" not in receipt:
                paths = list((study / "control-room/groups").glob("*/state.json"))
                if paths:
                    state = read_json(paths[0])
                    if not state.get("primary_complete") and state.get("throughput", {}).get("uploaded_images", 0) > 0:
                        receipt["pause"] = pause_and_resume(paths[0], request, process, deadline)
            time.sleep(.25)
        receipt["runner_wall_seconds"] = time.monotonic() - started
        assert process.returncode in (0, 1), process.returncode
        # Exit 1 also denotes the intentional visual differences in this fixture.
        receipt.update(collect_result(study))
        if args.pause_check:
            assert "pause" in receipt, "Did not pause during primary captures"
            assert receipt["metrics"]["throughput"]["uploaded_images"] > receipt["pause"]["images_after_drain"]
            assert receipt["metrics"]["primary_pids"] == receipt["pause"]["primary_pids"], "Resuming replaced an Excel process"
            pipeline = receipt["metrics"]["capture_pipeline"]
            peaks = pipeline["processing"]["clients"]
            assert set(PROFILES).issubset(peaks), "Primary processing peaks were lost during control captures"
            assert any(name.startswith("control-") for name in peaks), "Missing on-demand control processing peaks"
            assert all(peak["tiles"] <= pipeline["max_pending_tiles_per_client"] for peak in peaks.values())
            assert all(peak["pixels"] <= pipeline["max_pending_pixels_per_client"] for peak in peaks.values())
        receipt["status"] = "passed"
    except Exception as error:
        receipt["status"] = "failed"
        receipt["error"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        if process is not None and process.poll() is None:
            subprocess.run(
                ["taskkill", "/F", "/T", "/PID", str(process.pid)],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                creationflags=NO_WINDOW, timeout=20, check=True,
            )
            process.wait(timeout=10)
        write_json(directory / "receipt.json", receipt)
        cleanup_owned_host(study)
        print(json.dumps({"sample": directory.name, "status": receipt["status"], "metrics": receipt.get("metrics")}), flush=True)
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true", help="Explicitly enable four native Excel study runs")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/excel-shrink-cli.exe")
    parser.add_argument("--port", type=int, default=8827)
    parser.add_argument("--deadline", type=int, default=240, help="Maximum seconds per sample")
    parser.add_argument("--pause-check", action="store_true", help="Run one untimed pipeline pause/resume check instead of ABBA")
    parser.add_argument("--compare-to", type=Path, help="Completed sample directory whose signature the pause check must preserve")
    args = parser.parse_args()
    if not 1024 <= args.port <= 65535 or not 30 <= args.deadline <= 600:
        parser.error("Use a port from 1024 to 65535 and a deadline from 30 to 600 seconds")
    if args.pause_check != (args.compare_to is not None):
        parser.error("Use --pause-check and --compare-to together")
    archive = prepare_fixture()
    if not args.run:
        print(json.dumps({"prepared": True, "excel_started": False, "archive": str(archive)}))
        return
    args.binary = args.binary.resolve(strict=True)
    directory = OUTPUT / (("pause-" if args.pause_check else "abba-") + time.strftime("%Y%m%d-%H%M%S"))
    directory.mkdir()
    modes = (True,) if args.pause_check else (False, True, True, False)
    result = {
        "status": "running", "binary_sha256": digest(args.binary), "input_sha256": digest(archive),
        "order": ["pipeline" if enabled else "baseline" for enabled in modes], "samples": [],
        "untimed_pause_check": args.pause_check,
        "cache_policy": "Fresh Excel instances; OS/Office caches retained. ABBA balances sample order, not all sources of noise.",
    }
    try:
        reference = None
        if args.compare_to:
            previous = read_json(args.compare_to / "receipt.json")
            assert previous["input_sha256"] == result["input_sha256"], "Reference used a different input fixture"
            reference = previous["signature"]
        for index, enabled in enumerate(modes, start=1):
            label = f"{index}-{'pipeline' if enabled else 'baseline'}"
            sample = run_sample(args, archive, directory / label, enabled)
            result["samples"].append({"directory": label, "pipeline": enabled, "metrics": sample["metrics"], "runner_wall_seconds": sample["runner_wall_seconds"]})
            if reference is None:
                reference = sample["signature"]
            assert sample["signature"] == reference, f"Coverage, pixels or findings changed in {label}; inspect both receipts"
            assert digest(args.binary) == result["binary_sha256"], "Binary changed during the benchmark"
            write_json(directory / "receipt.json", result)
        if args.pause_check:
            result.update(status="passed", identical_coverage_pixels_and_findings=True)
            return
        medians = {}
        for enabled, label in ((False, "baseline"), (True, "pipeline")):
            samples = [item for item in result["samples"] if item["pipeline"] == enabled]
            medians[label] = {
                "capture_ms": statistics.median(item["metrics"]["throughput"]["capture_elapsed_ms"] for item in samples),
                "comparison_seconds": statistics.median(item["metrics"]["comparing_elapsed_seconds"] for item in samples),
                "runner_wall_seconds": statistics.median(item["runner_wall_seconds"] for item in samples),
            }
        result.update(status="passed", identical_coverage_pixels_and_findings=True, medians=medians)
    except Exception as error:
        result.update(status="failed", error=f"{type(error).__name__}: {error}")
        raise
    finally:
        write_json(directory / "receipt.json", result)
        print(json.dumps({"status": result["status"], "receipt": str(directory / "receipt.json"), "medians": result.get("medians")}), flush=True)


if __name__ == "__main__":
    main()
