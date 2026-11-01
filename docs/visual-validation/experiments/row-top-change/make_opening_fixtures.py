"""Add a deliberately stale formula cache to an Excel-generated workbook."""

import argparse
import re
import zipfile
from pathlib import Path


FORMULA_ROW = b'<row r="2"><c r="A2"><v>2</v></c><c r="B2"><f>A2*2</f><v>-999</v></c></row>'


def with_formula(sheet_xml: bytes) -> bytes:
    """Insert a formula whose saved result disagrees with its input."""
    marker = b'</row><row r="500000"'
    if sheet_xml.count(marker) != 1:
        raise ValueError("Unexpected row order in the Excel-generated fixture")
    return sheet_xml.replace(marker, b'</row>' + FORMULA_ROW + b'<row r="500000"')


def with_calculation_mode(workbook_xml: bytes, mode: str, full_on_load: bool) -> bytes:
    """Keep the stale cache eligible for an explicit opening-mode test."""
    replacement = (
        f'<calcPr calcId="191029" calcMode="{mode}" '
        f'fullCalcOnLoad="{int(full_on_load)}" forceFullCalc="0"/>'
    ).encode("ascii")
    updated, count = re.subn(rb'<calcPr\b[^>]*/>', replacement, workbook_xml)
    if count != 1:
        raise ValueError("Expected one workbook calculation setting")
    return updated


def write_fixture(source: Path, destination: Path, mode: str, full_on_load: bool) -> None:
    """Copy every XLSX part, changing only formula and calculation XML."""
    with zipfile.ZipFile(source) as original, zipfile.ZipFile(destination, "w") as output:
        for part in original.infolist():
            data = original.read(part.filename)
            if part.filename == "xl/worksheets/sheet1.xml":
                data = with_formula(data)
            elif part.filename == "xl/workbook.xml":
                data = with_calculation_mode(data, mode, full_on_load)
            output.writestr(part, data)


def main() -> None:
    """Create manual and automatic calculation fixtures for the save probe."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("output_directory", type=Path)
    args = parser.parse_args()
    args.output_directory.mkdir(parents=True, exist_ok=True)
    for name, mode, full_on_load in (
        ("manual", "manual", False),
        ("auto", "auto", False),
        ("auto-full-on-load", "auto", True),
    ):
        write_fixture(
            args.source,
            args.output_directory / f"opening-{name}.xlsx",
            mode,
            full_on_load,
        )


if __name__ == "__main__":
    main()
