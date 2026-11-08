"""Run explicitly licensed external comparators on frozen synthetic input pairs."""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from process_metrics import run_measured


def command_for(tool, executable, original, candidate, report):
    """Use the vendors' documented whole-workbook report commands without GUI scrolling."""
    if tool == "xlcompare":
        return [str(executable), str(original), str(candidate), f"-r:{report}"]
    return [str(executable), f"/inbook1:{original}", f"/inbook2:{candidate}",
            f"/report:{report}", "/compareformulae:true", "/colordifferences:false",
            "/show:false", "/whitespacehandling:1"]


def successful_exit(tool, code):
    """DiffEngineX returns difference counts as positive exit codes."""
    if tool != "diffenginex":
        return code == 0
    signed = (code + 2**31) % 2**32 - 2**31
    return signed >= 0 or signed in (-4, -101, -102)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def report_preview(path):
    """Keep the complete vendor report while bounding its chat-friendly text projection."""
    from openpyxl import load_workbook
    workbook = load_workbook(path, read_only=True, data_only=False)
    try:
        sheets = []
        for sheet in workbook.worksheets:
            rows = []
            for row in sheet.iter_rows(max_row=40, max_col=12):
                cells = {cell.coordinate: str(cell.value)[:256]
                         for cell in row if cell.value is not None}
                if cells:
                    rows.append(cells)
            sheets.append({"name": sheet.title, "first_40_rows": rows})
        return sheets
    finally:
        workbook.close()


def run(args):
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    fixtures = json.loads(args.fixtures.read_text(encoding="utf-8-sig"))
    pairs = [pair for pair in fixtures["pairs"] if not args.case or pair["id"] == args.case]
    if not pairs:
        raise ValueError("The selected fixture case does not exist")
    receipt = {"tool": args.tool, "executable_sha256": digest(args.executable),
               "resource_scope": "direct comparator process; excludes separate Excel processes",
               "deadline_seconds": args.timeout, "trials": []}
    for pair in pairs:
        directory = output / pair["id"]
        directory.mkdir()
        inputs = []
        for lane in ("original", "candidate"):
            source = Path(pair[lane])
            expected = pair["sha256"][source.name]
            if digest(source) != expected:
                raise ValueError(f"Frozen {lane} input changed for {pair['id']}")
            copied = directory / f"{lane}.xlsx"
            shutil.copyfile(source, copied)
            if digest(copied) != expected:
                raise ValueError("Input copy identity mismatch")
            inputs.append(copied)
        report = directory / "report.xlsx"
        command = command_for(args.tool, args.executable.resolve(), *inputs, report)
        trial = {"id": pair["id"], "command": command, "expected_changes": pair["expected_changes"],
                 "input_dimensions": pair.get("dimensions"), "input_bytes": inputs[0].stat().st_size,
                 "candidate_bytes": inputs[1].stat().st_size}
        try:
            metrics = run_measured(command, directory, args.timeout)
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
            metrics = json.loads((directory / "process-metrics.json").read_text(encoding="utf-8"))
        trial["process"] = metrics
        trial["completed_with_report"] = (
            not metrics["timed_out"] and successful_exit(args.tool, metrics["exit_code"]) and report.is_file())
        trial["input_copies_unchanged"] = all(
            digest(path) == pair["sha256"][path.name] for path in inputs)
        if report.is_file():
            trial["report_bytes"] = report.stat().st_size
            trial["report_sha256"] = digest(report)
            try:
                trial["report_preview"] = report_preview(report)
            except Exception as error:
                trial["report_preview_error"] = str(error)
                trial["completed_with_report"] = False
        receipt["trials"].append(trial)
        (output / "pilot.json").write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
        print(json.dumps({"tool": args.tool, "case": pair["id"], "seconds": metrics["seconds"],
                          "exit_code": metrics["exit_code"], "report": trial["completed_with_report"]}), flush=True)
        if not trial["completed_with_report"] or not trial["input_copies_unchanged"]:
            raise RuntimeError("Pilot stopped; inspect retained process/ownership and input evidence before another trial")
    return receipt


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tool", choices=("xlcompare", "diffenginex"), required=True)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--fixtures", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--case")
    parser.add_argument("--timeout", type=float, default=45)
    arguments = parser.parse_args()
    if arguments.timeout <= 0:
        parser.error("The external deadline must be positive")
    run(arguments)
