"""Bounded Windows/Excel integration check; requires Python and openpyxl.

Without --run this only prepares a small local fixture. Run a sampled check,
then use --reference --compare-to <sampled-run-directory> on another free port.
Neither invocation reads or modifies the production corpus assignment.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import time
import urllib.error
import urllib.request
import zipfile


ROOT = Path(__file__).resolve().parents[3]
OUTPUT = ROOT / ".tmp/tall-sheet-smoke"
NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0)


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def write_json(path, value):
    temporary = path.with_suffix(".pending")
    temporary.write_text(json.dumps(value, indent=2), encoding="utf-8")
    temporary.replace(path)


def prepare_fixture():
    from openpyxl import Workbook
    from openpyxl.formatting.rule import CellIsRule
    from openpyxl.styles import PatternFill

    OUTPUT.mkdir(parents=True, exist_ok=True)
    archive = OUTPUT / "tall-and-small.zip"
    if archive.exists():
        return archive
    workbook = Workbook()
    sheet = workbook.active
    sheet.title = "Tall data"
    for row in range(1, 1601):
        sheet.append([row, row * 2, row * 3, 1, 1, 1, 1, 1])
    sheet["G900"] = "    "
    sheet.row_dimensions[1100].hidden = True
    sheet["H1100"] = "hidden unreferenced"
    sheet["D700"].fill = PatternFill(fill_type="solid", fgColor="FFF1B8")
    sheet.conditional_formatting.add(
        "B400:B405",
        CellIsRule(
            operator="greaterThan", formula=["0"],
            fill=PatternFill(fill_type="solid", fgColor="B8E9FF"),
        ),
    )
    small = workbook.create_sheet("Small summary")
    small["A1"] = "=SUM('Tall data'!A1:A1600)"
    small["A2"] = "Always captured in full"
    small["A3"] = "  "
    source = OUTPUT / "tall-and-small.xlsx"
    workbook.save(source)
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as output:
        output.write(source, source.name)
    return archive


def records(study):
    return [json.loads(path.read_bytes()) for path in (study / "records").glob("*.json")]


def difference_ranges(record):
    return sorted(
        (profile["profile"], sheet["name"], difference["range"])
        for profile in record["results"]
        for sheet in profile["sheets"]
        for difference in sheet.get("image_differences", [])
    )


def validate_captures(study, reference):
    result = records(study)
    assert len(result) == 1, "Expected exactly one completed fixture workbook"
    record = result[0]
    assert record.get("complete") and record["status"] in ("unchanged", "differences"), record
    state = json.loads(next((study / "control-room/groups").glob("*/state.json")).read_bytes())
    tall = next(sheet for sheet in state["sheets"] if sheet["name"] == "Tall data")
    small = next(sheet for sheet in state["sheets"] if sheet["name"] == "Small summary")
    assert small["tiles"] and all(tile["status"] in ("equal", "different") for tile in small["tiles"])
    assert small.get("capture_coverage", {}).get("omitted_tiles", 0) == 0
    coverage = tall["capture_coverage"]
    assert coverage["total_tiles"] > 5, coverage
    if reference:
        assert coverage["omitted_tiles"] == 0, coverage
    else:
        assert coverage["omitted_tiles"] > 0, coverage
        for omitted in coverage["omitted_ranges"]:
            start, end = omitted["core_rows"]
            assert not any(start <= row < end for row in (399, 404, 699)), omitted
    assert all(profile["checked_sheets"] == 2 for profile in record["results"])
    # Equal small sheets are deliberately pruned from report.sheets. Their
    # actual completed captures, rather than their report presence, prove coverage.
    return record, {
        "tall": coverage,
        "small": {"captured_tiles": len(small["tiles"]), "omitted_tiles": 0},
        "difference_ranges": difference_ranges(record),
        "capture_elapsed_ms": state.get("throughput", {}).get("capture_elapsed_ms"),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true", help="Explicitly enable native Excel")
    parser.add_argument("--reference", action="store_true", help="Use a threshold above every fixture sheet")
    parser.add_argument("--hard-reset-original", action="store_true", help="Restart only the hidden original visibly before capture")
    parser.add_argument("--hide-excel", action="store_true", help="Capture on private desktops without taking keyboard focus")
    parser.add_argument("--compare-to", type=Path, help="Previously completed sampled run directory")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/excel-shrink-cli.exe")
    parser.add_argument("--port", type=int, default=8799)
    parser.add_argument("--deadline", type=int, default=600)
    args = parser.parse_args()
    if not 1024 <= args.port <= 65535 or not 30 <= args.deadline <= 600:
        parser.error("Use a port from 1024 to 65535 and a deadline from 30 to 600 seconds")
    archive = prepare_fixture()
    if not args.run:
        print(json.dumps({"prepared": True, "excel_started": False, "archive": str(archive)}))
        return
    # Never send controls to a pre-existing listener on a shared test machine.
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", args.port))
    prefix = "reference-" if args.reference else "sampled-"
    directory = OUTPUT / (prefix + time.strftime("%Y%m%d-%H%M%S"))
    directory.mkdir()
    study, package = directory / "study", directory / "results.zip"
    binary = args.binary.resolve()
    threshold = 100000 if args.reference else 5
    command = [
        str(binary), "regression", "study", "--corpus", "Tall sheet smoke", str(archive),
        "--output-root", str(study), "--resident-capture", "--control-room",
        "--control-room-port", str(args.port), "--comparison-workers", "0",
        "--startup-timeout-seconds", "120", "--workbook-timeout-seconds", "180",
        "--machine-label", "isolated tall-sheet smoke", "--export-results", str(package),
    ]
    environment = os.environ.copy()
    environment.update(
        EXCEL_SHRINK_CAPTURE_TILE_MEGAPIXELS="1",
        EXCEL_SHRINK_CAPTURE_BATCH_TILES="4",
        EXCEL_SHRINK_TALL_SHEET_THRESHOLD=str(threshold),
        EXCEL_SHRINK_TALL_SHEET_EDGE_TILES="1",
        EXCEL_SHRINK_RESIDENT_VISIBLE="0" if args.hide_excel else "1",
    )
    receipt = {
        "status": "running", "command": command, "binary_sha256": digest(binary),
        "input_sha256": digest(archive), "threshold": threshold, "edge_tiles": 1,
        "deadline_seconds": args.deadline, "events": [],
    }
    started = time.monotonic()
    process = None

    def deadline():
        if time.monotonic() - started > args.deadline:
            raise TimeoutError("Absolute native smoke deadline reached")

    def event(kind, **values):
        entry = {"kind": kind, "seconds": round(time.monotonic() - started, 2), **values}
        receipt["events"].append(entry)
        write_json(directory / "receipt.json", receipt)
        print(json.dumps(entry), flush=True)

    def launch(arguments, label):
        with (directory / f"{label}.stdout.log").open("wb") as out, (directory / f"{label}.stderr.log").open("wb") as err:
            return subprocess.Popen(arguments, cwd=ROOT, stdout=out, stderr=err, env=environment, creationflags=NO_WINDOW)

    def request(path, body=None):
        req = urllib.request.Request(
            f"http://127.0.0.1:{args.port}{path}",
            data=None if body is None else json.dumps(body).encode(),
            headers={"Content-Type": "application/json"},
        )
        with urllib.request.urlopen(req, timeout=3) as response:
            return json.load(response)

    try:
        process = launch(command, "run")
        receipt["pid"] = process.pid
        while True:
            deadline()
            if process.poll() is not None:
                raise RuntimeError(f"Runner exited before control startup: {process.returncode}")
            try:
                state = request("/api/state")
                if state.get("runner_pid") == process.pid and state.get("running") and state["controls"]["target_groups"] == 0:
                    break
            except (urllib.error.URLError, OSError, KeyError):
                pass
            time.sleep(.25)
        request("/api/control", {
            "target_groups": 1, "capture_tile_megapixels": 1, "capture_batch_tiles": 4,
            "tall_sheet_threshold": threshold, "tall_sheet_edge_tiles": 1,
            "hide_excel": args.hide_excel or args.hard_reset_original, "paused": args.hard_reset_original,
        })
        event("started_isolated_group", hide_excel=args.hide_excel or args.hard_reset_original)
        if args.hard_reset_original:
            while True:
                deadline()
                groups = request('/api/workers')['groups']
                group = next((g for g in groups if len(g.get('clients', [])) >= 2
                    and any(c['client_id'] == 'original' and c.get('pid') for c in g['clients'])
                    and any(c['client_id'] != 'original' and c.get('heartbeat_count', 0) > 0 for c in g['clients'])), None)
                if group: break
                if process.poll() is not None: raise RuntimeError('Runner ended before reset')
                time.sleep(.5)
            before = {c['client_id']: c['pid'] for c in group['clients']}
            request('/api/control', {'hide_excel':False})
            reset = request(f"/api/groups/{group['id']}/restart", {'client_id':'original','hard_reset':True,'reason':'Isolated native reset check'})
            assert reset['restart_scope'] == 'client', reset
            while True:
                deadline()
                after = next((g for g in request('/api/workers')['groups'] if g['id'] == group['id']), None)
                original = next((c for c in (after or {}).get('clients', []) if c['client_id'] == 'original'), {})
                if original.get('pid') != before['original'] and original.get('heartbeat_count', 0) > 0: break
                if process.poll() is not None: raise RuntimeError('Runner ended during reset')
                time.sleep(.5)
            assert after['attempt_id'] == group['attempt_id']
            assert original['hide_excel'] is False
            assert all(c['pid'] == before[c['client_id']] for c in after['clients'] if c['client_id'] != 'original')
            event('hard_reset_visible_original_only', before=before, after={c['client_id']:c['pid'] for c in after['clients']})
            request('/api/control', {'paused':False})
        while process.poll() is None:
            deadline()
            time.sleep(1)
        assert process.returncode in (0, 1), process.returncode
        record, receipt["coverage"] = validate_captures(study, args.reference)
        event("capture_coverage_verified")
        destination = directory / "import-check"
        process = launch([
            str(binary), "regression", "study", "--corpus", "Tall sheet smoke", str(archive),
            "--output-root", str(destination), "--report-only", "--import-results", str(package),
        ], "import")
        while process.poll() is None:
            deadline()
            time.sleep(.25)
        assert process.returncode == 0, process.returncode
        with zipfile.ZipFile(package) as exported:
            entries = json.loads(exported.read("result-package.json"))["entries"]
            for entry in entries:
                source = Path("\\\\?\\" + str((study / entry["path"]).resolve()))
                restored = Path("\\\\?\\" + str((destination / entry["path"]).resolve()))
                assert digest(source) == digest(restored) == entry["sha256"], entry["path"]
        receipt["export_import"] = {"byte_identical": True, "verified_entries": len(entries)}
        if args.compare_to:
            previous = records(args.compare_to / "study")
            assert len(previous) == 1
            assert difference_ranges(record) == difference_ranges(previous[0])
            receipt["reference_comparison"] = {"same_difference_ranges": True}
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
                creationflags=NO_WINDOW, timeout=20,
            )
            process.wait(timeout=10)
        receipt["elapsed_seconds"] = round(time.monotonic() - started, 2)
        write_json(directory / "receipt.json", receipt)
        print(json.dumps({"status": receipt["status"], "receipt": str(directory / "receipt.json")}), flush=True)


if __name__ == "__main__":
    main()
