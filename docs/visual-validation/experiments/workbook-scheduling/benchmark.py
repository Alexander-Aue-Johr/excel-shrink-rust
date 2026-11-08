"""Measure bounded synthetic scheduling workloads through the ordinary shrink CLI.

Outputs are compared by every decompressed ZIP member. Perfetto traces retain
resource handoffs. Concurrent validation makes timings exploratory, so record
the workload note and use an idle machine for controlled speedup claims.
"""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time
import zipfile


def package_members(path):
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)):
            raise ValueError(f"Duplicate ZIP members in {path}")
        result = {}
        for name in sorted(names):
            with archive.open(name) as stream:
                result[name] = hashlib.file_digest(stream, "sha256").hexdigest()
        return result


def workbook(path, sheet_count, row_count):
    """Write numeric content and empty cells with deterministic ZIP metadata."""
    namespace = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
    relations = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
    sheets = []
    links = []
    members = {}
    for number in range(1, sheet_count + 1):
        sheets.append(f'<sheet name="Data{number}" sheetId="{number}" r:id="rId{number}"/>')
        links.append(f'<Relationship Id="rId{number}" Type="{relations}/worksheet" '
                     f'Target="worksheets/sheet{number}.xml"/>')
        rows = []
        for row in range(1, row_count + 1):
            rows.append(f'<row r="{row}"><c r="A{row}"><v>{row}</v></c>'
                        + ''.join(f'<c r="{column}{row}"/>' for column in "BCDEFGH")
                        + '</row>')
        members[f"xl/worksheets/sheet{number}.xml"] = (
            f'<worksheet xmlns="{namespace}"><sheetData>' + ''.join(rows)
            + '</sheetData></worksheet>')
    members["xl/workbook.xml"] = (
        f'<workbook xmlns="{namespace}" xmlns:r="{relations}"><sheets>'
        + ''.join(sheets) + '</sheets></workbook>')
    members["xl/_rels/workbook.xml.rels"] = (
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        + ''.join(links) + '</Relationships>')
    members["_rels/.rels"] = (
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        f'<Relationship Id="rId1" Type="{relations}/officeDocument" Target="xl/workbook.xml"/>'
        '</Relationships>')
    overrides = ''.join(
        f'<Override PartName="/xl/worksheets/sheet{number}.xml" '
        'ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
        for number in range(1, sheet_count + 1))
    members["[Content_Types].xml"] = (
        '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
        '<Default Extension="xml" ContentType="application/xml"/>'
        '<Override PartName="/xl/workbook.xml" '
        'ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>'
        + overrides + '</Types>')
    with zipfile.ZipFile(path, "x") as archive:
        for name, text in sorted(members.items()):
            info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, text.encode("utf-8"))


def execute_trial(binary, source, work, threads, active):
    """Time one bounded CLI invocation and verify that all expected outputs exist."""
    work.mkdir()
    destination = work / "output"
    command = [str(binary), str(source), str(destination),
               "--rayon-threads", str(threads), "--max-active-workbooks", str(active),
               "--writer-workers", "1", "--trace-file", str(work / "trace.json")]
    with (work / "stdout.log").open("wb") as stdout, (work / "stderr.log").open("wb") as stderr:
        start = time.perf_counter()
        subprocess.run(command, check=True, timeout=120, stdout=stdout, stderr=stderr)
        seconds = time.perf_counter() - start
    outputs = {path.name: package_members(path) for path in destination.rglob("*.xlsx")}
    if set(outputs) != {path.name for path in source.glob("*.xlsx")}:
        raise ValueError(f"Missing or extra output workbooks in {work}")
    return seconds, outputs


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--large-rows", type=int, default=8192)
    parser.add_argument("--repetitions", type=int, default=2)
    parser.add_argument("--workload-note", required=True)
    args = parser.parse_args()
    if not 256 <= args.large_rows <= 1048576 or args.repetitions < 1:
        parser.error("Choose 256–1048576 rows and a positive repetition count")
    args.binary = args.binary.resolve(strict=True)
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=False)
    shapes = {
        "small-workbooks": [(1, 256)] * 16,
        "many-sheets": [(16, 256)],
        "large-sheets": [(2, args.large_rows)] * 2,
        "mixed": [(1, 256)] * 8 + [(2, args.large_rows)] * 2,
    }
    configurations = [(1, 1), (2, 1), (2, 4)]
    with args.binary.open("rb") as stream:
        binary_hash = hashlib.file_digest(stream, "sha256").hexdigest()
    report = {"status": "running", "binary_sha256": binary_hash,
              "workload_note": args.workload_note, "rows": [],
              "memory_measurement": "not measured", "inputs": {}}
    report_path = args.output / "results.json"
    for label, books in shapes.items():
        source = args.output / label / "input"
        source.mkdir(parents=True)
        for index, (sheets, rows) in enumerate(books):
            workbook(source / f"book-{index:03}.xlsx", sheets, rows)
        report["inputs"][label] = {path.name: package_members(path) for path in source.glob("*.xlsx")}
        reference = None
        for repetition in range(args.repetitions):
            order = configurations if repetition % 2 == 0 else list(reversed(configurations))
            for threads, active in order:
                work = args.output / label / f"trial-{repetition}-{threads}-{active}"
                seconds, outputs = execute_trial(args.binary, source, work, threads, active)
                if reference is None:
                    reference = outputs
                if outputs != reference:
                    raise ValueError(f"Scheduling changed output members in {work}")
                report["rows"].append({"workload": label, "repetition": repetition,
                                       "threads": threads, "active_workbooks": active,
                                       "seconds": seconds, "outputs_equal": True})
                report_path.write_text(json.dumps(report, indent=2), encoding="utf-8")
    report["status"] = "completed"
    report_path.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps({"status": report["status"], "trials": len(report["rows"])}))


if __name__ == "__main__":
    main()
