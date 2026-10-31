"""Populate one empty Excel row and remove its unique large-font cell."""

import argparse
import re
import zipfile
from pathlib import Path


SHEET_XML = "xl/worksheets/sheet1.xml"
SENTINEL_PATTERN = rb'<c r="XFD500000" s="\d+"/>'


def column_name(number: int) -> str:
    """Convert a one-based column index to its Excel address letters."""
    name = ""
    while number:
        number, digit = divmod(number - 1, 26)
        name = chr(65 + digit) + name
    return name


def make_sheet_variants(sheet_xml: bytes) -> tuple[bytes, bytes]:
    """Return sheets that differ only by the empty large-font cell at XFD500000.

    The input must contain exactly one such cell. Both outputs retain the
    original row-height attribute so Excel must resolve the effective height.
    """
    matches = re.findall(SENTINEL_PATTERN, sheet_xml)
    if len(matches) != 1:
        raise ValueError("Expected exactly one empty XFD500000 font cell")
    sentinel = matches[0]
    ordinary_cells = b"".join(
        f'<c r="{column_name(column)}500000"/>'.encode("ascii")
        for column in range(1, 16384)
    )
    baseline = sheet_xml.replace(sentinel, ordinary_cells + sentinel)
    changed = sheet_xml.replace(sentinel, ordinary_cells)
    if len(re.findall(rb'<c\b[^>]*r="[A-Z]+500000"', baseline)) != 16384:
        raise ValueError("Baseline does not contain every Excel column")
    return baseline, changed


def write_workbook(source: Path, destination: Path, sheet_xml: bytes) -> None:
    """Copy an XLSX while replacing only its first worksheet XML part."""
    with zipfile.ZipFile(source) as original, zipfile.ZipFile(destination, "w") as output:
        for part in original.infolist():
            data = sheet_xml if part.filename == SHEET_XML else original.read(part.filename)
            output.writestr(part, data)


def main() -> None:
    """Read the Excel fixture, build both sheet variants, and write the XLSX files."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("changed", type=Path)
    args = parser.parse_args()
    with zipfile.ZipFile(args.source) as source:
        baseline_xml, changed_xml = make_sheet_variants(source.read(SHEET_XML))
    write_workbook(args.source, args.baseline, baseline_xml)
    write_workbook(args.source, args.changed, changed_xml)


if __name__ == "__main__":
    main()
