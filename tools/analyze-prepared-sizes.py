"""Compare prepared Default sizes using ZIP directories, without reading XML."""
import argparse
import collections
import csv
import hashlib
import io
import json
from pathlib import Path
import re
import struct
import time
import zipfile


SHEET_PART = re.compile(r"^xl/(worksheets|chartsheets|dialogsheets|macrosheets)/[^/]+\.xml$")
THRESHOLDS = (50, 80, 90)


class StoredMember(io.RawIOBase):
    """A seekable view of one stored ZIP member, sharing the outer file handle."""
    def __init__(self, archive, info):
        super().__init__()
        if info.compress_type != zipfile.ZIP_STORED or info.flag_bits & 1:
            raise ValueError("Metadata-only access requires an unencrypted stored XLSX member")
        self.file = archive.fp
        self.file.seek(info.header_offset)
        header = self.file.read(30)
        if len(header) != 30 or header[:4] != b"PK\x03\x04":
            raise ValueError("Invalid outer ZIP local header")
        name_bytes, extra_bytes = struct.unpack_from("<HH", header, 26)
        self.start = info.header_offset + 30 + name_bytes + extra_bytes
        self.length = info.file_size
        self.position = 0
        if self.start + self.length > archive.start_dir:
            raise ValueError("Stored XLSX overlaps the outer ZIP directory")

    def readable(self):
        return True

    def seekable(self):
        return True

    def tell(self):
        return self.position

    def seek(self, offset, whence=0):
        if whence not in (0, 1, 2):
            raise ValueError("Invalid seek origin")
        position = (0, self.position, self.length)[whence] + offset
        if position < 0:
            raise OSError("Seek before stored XLSX")
        self.position = position
        return position

    def read(self, size=-1):
        remaining = max(0, self.length - self.position)
        size = remaining if size < 0 else min(size, remaining)
        self.file.seek(self.start + self.position)
        data = self.file.read(size)
        self.position += len(data)
        return data


def inspect_xlsx(archive, name):
    info = archive.getinfo(name)
    with StoredMember(archive, info) as member, zipfile.ZipFile(member) as workbook:
        sheets = sheet_directory(workbook)
    return info.file_size, sheets


def sheet_directory(workbook):
    sheets = {}
    for part in workbook.infolist():
        if SHEET_PART.fullmatch(part.filename):
            if part.filename in sheets:
                raise ValueError("Duplicate sheet part " + part.filename)
            sheets[part.filename] = (part.file_size, part.compress_size)
    return sheets


def reduction(before, after):
    return 100 * (before - after) / before if before and after is not None else None


def exceeds(before, after, threshold):
    return before > 0 and 100 * (before - after) > threshold * before


def bucket(before, after):
    if not before:
        return "No original reference"
    if after > before:
        return "Growth"
    if after == before:
        return "Unchanged"
    lower = 0
    for upper in (500, 800, 900, 950, 990, 999, 1000):
        if 1000 * (before - after) <= upper * before:
            return f"({lower / 10:g}, {upper / 10:g}] %"
        lower = upper
    raise ValueError("Negative ZIP member size")


def source_catalog(path, corpus_id):
    with zipfile.ZipFile(path) as source:
        return {hashlib.sha256(f"{corpus_id}:{index}:{part.filename}".encode()).hexdigest()[:20]: part.filename
                for index, part in enumerate(source.infolist())}


def write_csv(path, rows):
    if not rows:
        path.write_text("", encoding="utf8")
        return
    with path.open("w", newline="", encoding="utf-8-sig") as output:
        writer = csv.DictWriter(output, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)


def analyze(prepared, output, labels):
    output.mkdir(parents=True, exist_ok=False)
    started = time.monotonic()
    workbooks, sheets, errors = [], [], []
    distributions = {name: collections.Counter() for name in ("xlsx", "sheet_xml", "sheet_compressed")}
    candidates = {str(t): {"xlsx": set(), "sheet_xml": set(), "sheet_compressed": set()} for t in THRESHOLDS}
    for index, bundle in enumerate(prepared.glob("*.zip"), 1):
        case_id = bundle.stem
        if case_id not in labels:
            errors.append({"case_id": case_id, "error": "Bundle is absent from the requested source catalog"})
            continue
        try:
            with zipfile.ZipFile(bundle) as archive:
                before, old = inspect_xlsx(archive, "original.xlsx")
                after, new = inspect_xlsx(archive, "default.xlsx")
            local_sheets = []
            for part in sorted(old.keys() | new.keys()):
                old_xml, old_compressed = old.get(part, (0, 0))
                new_xml, new_compressed = new.get(part, (0, 0))
                local_sheets.append({
                    "case_id": case_id, "source_member": labels[case_id], "bundle": str(bundle),
                    "sheet_part": part,
                    "part_state": "matched" if part in old and part in new else "removed" if part in old else "added",
                    "original_xml_bytes": old_xml, "default_xml_bytes": new_xml,
                    "saved_xml_bytes": old_xml - new_xml,
                    "xml_reduction_percent": reduction(old_xml, new_xml),
                    "remaining_xml_percent": 100 * new_xml / old_xml if old_xml else None,
                    "original_compressed_bytes": old_compressed, "default_compressed_bytes": new_compressed,
                    "saved_compressed_bytes": old_compressed - new_compressed,
                    "compressed_reduction_percent": reduction(old_compressed, new_compressed)})
            workbook = {"case_id": case_id, "source_member": labels[case_id], "bundle": str(bundle),
                        "original_xlsx_bytes": before, "default_xlsx_bytes": after,
                        "saved_xlsx_bytes": before - after, "xlsx_reduction_percent": reduction(before, after),
                        "original_sheet_parts": len(old), "default_sheet_parts": len(new),
                        "max_sheet_xml_reduction_percent": max(
                            (s["xml_reduction_percent"] for s in local_sheets if s["xml_reduction_percent"] is not None),
                            default=None)}
            workbooks.append(workbook)
            sheets.extend(local_sheets)
            distributions["xlsx"][bucket(before, after)] += 1
            for threshold in THRESHOLDS:
                if exceeds(before, after, threshold):
                    candidates[str(threshold)]["xlsx"].add(case_id)
            for sheet in local_sheets:
                for kind, before_key, after_key in (
                        ("sheet_xml", "original_xml_bytes", "default_xml_bytes"),
                        ("sheet_compressed", "original_compressed_bytes", "default_compressed_bytes")):
                    distributions[kind][bucket(sheet[before_key], sheet[after_key])] += 1
                    for threshold in THRESHOLDS:
                        if exceeds(sheet[before_key], sheet[after_key], threshold):
                            candidates[str(threshold)][kind].add(case_id)
        except (OSError, ValueError, zipfile.BadZipFile, KeyError, struct.error) as error:
            errors.append({"case_id": case_id, "bundle": str(bundle), "error": str(error)})
        if index % 500 == 0:
            print(json.dumps({"scanned": index, "workbooks": len(workbooks), "errors": len(errors)}), flush=True)
    workbooks.sort(key=lambda r: (r["xlsx_reduction_percent"] or 0, r["saved_xlsx_bytes"]), reverse=True)
    sheets.sort(key=lambda r: (r["xml_reduction_percent"] or 0, r["saved_xml_bytes"]), reverse=True)
    write_csv(output / "workbooks.csv", workbooks)
    write_csv(output / "sheets.csv", sheets)
    thresholds = []
    for threshold in THRESHOLDS:
        groups = candidates[str(threshold)]
        groups["xlsx_or_sheet_xml"] = groups["xlsx"] | groups["sheet_xml"]
        thresholds.append({"threshold_percent_strictly_greater_than": threshold,
                           **{kind: len(ids) for kind, ids in groups.items()}})
    summary = {"profile": "default", "workbooks": len(workbooks), "sheet_parts": len(sheets),
               "errors": len(errors), "elapsed_seconds": time.monotonic() - started,
               "method": "ZIP directory metadata only; no XLSX or XML payload extraction",
               "sheet_identity": "Package part paths; worksheet display names were not read",
               "default_generation": "Existing prepared variants; producer provenance remains in the study records",
               "thresholds": thresholds, "distributions": distributions,
               "original_xlsx_bytes": sum(r["original_xlsx_bytes"] for r in workbooks),
               "default_xlsx_bytes": sum(r["default_xlsx_bytes"] for r in workbooks)}
    (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf8")
    (output / "candidates.json").write_text(json.dumps({
        threshold: {kind: sorted(ids) for kind, ids in groups.items()}
        for threshold, groups in candidates.items()}, indent=2) + "\n", encoding="utf8")
    (output / "errors.json").write_text(json.dumps(errors, indent=2) + "\n", encoding="utf8")
    lines = ["# Enron Default size distribution", "", summary["method"] + ".",
             "Existing prepared Default variants are the comparison baseline. Sheet identifiers are package part paths.",
             "", f"{len(workbooks):,} workbooks, {len(sheets):,} sheet parts, {len(errors):,} inventory errors.", "",
             "| Reduction | Workbooks | Workbooks with a qualifying sheet XML | Union |",
             "| --- | ---: | ---: | ---: |"]
    for row in thresholds:
        lines.append(f"| > {row['threshold_percent_strictly_greater_than']}% | {row['xlsx']:,} | {row['sheet_xml']:,} | {row['xlsx_or_sheet_xml']:,} |")
    lines += ["", "## Distribution", "", "| Reduction | XLSX files | Sheet XML parts | Compressed sheet parts |",
              "| --- | ---: | ---: | ---: |"]
    for label in ("Growth", "Unchanged", "(0, 50] %", "(50, 80] %", "(80, 90] %", "(90, 95] %", "(95, 99] %", "(99, 99.9] %", "(99.9, 100] %", "No original reference"):
        lines.append(f"| {label} | {distributions['xlsx'][label]:,} | {distributions['sheet_xml'][label]:,} | {distributions['sheet_compressed'][label]:,} |")
    lines += ["", "The CSV files retain exact byte counts, source members and bundle paths. Candidates target size-reduction extremes."]
    (output / "README.md").write_text("\n".join(lines) + "\n", encoding="utf8")
    print(json.dumps(summary), flush=True)


def compare_controls(run, output, default_root=None, record_files=None):
    """Summarize completed cases, retaining ordinary-save controls separately."""
    output.mkdir(parents=True, exist_ok=True)
    workbooks, sheets, errors = [], [], []
    for record_file in record_files if record_files is not None else (run / "results").glob("*.json"):
        result = json.loads(record_file.read_bytes())
        if result["status"] != "prepared":
            continue
        case_id = result["id"]
        attempt = Path(result["attempt"])
        paths = {
            "original": attempt / "inputs/original.xlsx",
            "default": (default_root / case_id / "original.xlsx" if default_root else
                        attempt / "inputs/default/original.xlsx"),
            "saved_original": attempt / "artifacts/original.saved.xlsx",
            "cp_before": attempt / "artifacts/check-performance.before-optimization.xlsx",
            "cp_after": attempt / "artifacts/check-performance.optimized.saved.xlsx"}
        try:
            inventories = {}
            for name, path in paths.items():
                with zipfile.ZipFile(path) as archive:
                    inventories[name] = sheet_directory(archive)
            row = {"case_id": case_id, "source_member": result["source_member"]}
            if default_root:
                row["historical_default_xlsx_bytes"] = (attempt / "inputs/default/original.xlsx").stat().st_size
            row.update({name + "_xlsx_bytes": path.stat().st_size for name, path in paths.items()})
            row["default_reduction_percent"] = reduction(row["original_xlsx_bytes"], row["default_xlsx_bytes"])
            row["cp_roundtrip_reduction_percent"] = reduction(row["original_xlsx_bytes"], row["cp_after_xlsx_bytes"])
            row["cp_optimization_reduction_percent"] = reduction(row["cp_before_xlsx_bytes"], row["cp_after_xlsx_bytes"])
            difference = row["default_xlsx_bytes"] - row["cp_after_xlsx_bytes"]
            row["smaller_result"] = "check_performance" if difference > 0 else "default" if difference < 0 else "equal"
            row["default_path"] = str(paths["default"])
            row["check_performance_path"] = str(paths["cp_after"])
            local_sheets = []
            for part in sorted(set().union(*(set(parts) for parts in inventories.values()))):
                sheet = {"case_id": case_id, "source_member": result["source_member"], "sheet_part": part}
                for name, parts in inventories.items():
                    xml, compressed = parts.get(part, (None, None))
                    sheet[name + "_xml_bytes"] = xml
                    sheet[name + "_compressed_bytes"] = compressed
                for kind in ("xml", "compressed"):
                    default = sheet["default_" + kind + "_bytes"]
                    cp = sheet["cp_after_" + kind + "_bytes"]
                    sheet["default_" + kind + "_reduction_percent"] = reduction(sheet["original_" + kind + "_bytes"], default)
                    sheet["cp_roundtrip_" + kind + "_reduction_percent"] = reduction(sheet["original_" + kind + "_bytes"], cp)
                    sheet["cp_optimization_" + kind + "_reduction_percent"] = reduction(sheet["cp_before_" + kind + "_bytes"], cp)
                    sheet["smaller_" + kind + "_result"] = (
                        "missing_part" if default is None or cp is None else
                        "check_performance" if cp < default else "default" if default < cp else "equal")
                local_sheets.append(sheet)
            workbooks.append(row)
            sheets.extend(local_sheets)
        except (OSError, ValueError, zipfile.BadZipFile) as error:
            errors.append({"case_id": case_id, "error": str(error)})
    workbooks.sort(key=lambda r: r["default_reduction_percent"] or 0, reverse=True)
    write_csv(output / "workbooks.csv", workbooks)
    write_csv(output / "sheets.csv", sheets)
    summary = {"completed_workbooks": len(workbooks), "sheet_parts": len(sheets), "errors": errors,
               "workbook_results": dict(collections.Counter(r["smaller_result"] for r in workbooks)),
               "sheet_xml_results": dict(collections.Counter(r["smaller_xml_result"] for r in sheets)),
               "sheet_compressed_results": dict(collections.Counter(r["smaller_compressed_result"] for r in sheets)),
               "method": "ZIP directory sizes; raw Default, saved Original and Check Performance before/after retained separately",
               "visual_validation": "pending"}
    (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf8")
    print(json.dumps(summary), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--prepared", type=Path)
    mode.add_argument("--comparison-run", type=Path)
    parser.add_argument("--source", type=Path)
    parser.add_argument("--corpus-id")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--wait-for-run", type=Path)
    args = parser.parse_args()
    if args.comparison_run:
        compare_controls(args.comparison_run, args.output)
        raise SystemExit(0)
    if not args.source or not args.corpus_id:
        parser.error("Prepared inventory requires --source and --corpus-id")
    if args.wait_for_run:
        while json.loads((args.wait_for_run / "status.json").read_bytes())["state"] in ("running", "waiting_for_resources"):
            time.sleep(5)
    analyze(args.prepared, args.output, source_catalog(args.source, args.corpus_id))
