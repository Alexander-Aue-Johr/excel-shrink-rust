"""Create small input pairs for native conditional-state verification."""

import argparse
from pathlib import Path
from shutil import copyfile

from openpyxl import Workbook


def create_pair(root: Path, name: str, *, formula: bool, identical: bool) -> None:
    """Vary a real cell while keeping an optional dependent formula intact."""
    inputs = root / name / "inputs"
    inputs.mkdir(parents=True, exist_ok=False)
    for label, value in (("original", 1), ("default", 2)):
        if identical and label == "default":
            copyfile(inputs / "original.xlsx", inputs / "default.xlsx")
            continue
        workbook = Workbook()
        sheet = workbook.active
        sheet.title = "Data"
        sheet["A1"] = "Input"
        sheet["B1"] = value
        if formula:
            sheet["A2"] = "Dependent"
            sheet["B2"] = "=B1*2"
        workbook.save(inputs / f"{label}.xlsx")


def main() -> None:
    """Keep all generated artifacts in an explicitly selected fresh directory."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    create_pair(args.output, "identical-static", formula=False, identical=True)
    create_pair(args.output, "changed-static", formula=False, identical=False)
    create_pair(args.output, "changed-formula", formula=True, identical=False)


if __name__ == "__main__":
    main()
