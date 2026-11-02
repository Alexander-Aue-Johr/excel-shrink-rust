"""Create deliberately different workbook states for native capture tests, not shrink benchmarks."""

import argparse
from pathlib import Path

from openpyxl import Workbook
from openpyxl.formatting.rule import FormulaRule
from openpyxl.styles import PatternFill


def create_workbook(destination: Path, variant: str, visibility_transition: bool) -> None:
    """Exercise visible splitting, hidden observations and an optional visibility mismatch."""
    workbook = Workbook()
    visible = workbook.active
    visible.title = "Visible"
    visible["A1"] = "original" if variant == "original" else "changed"
    visible["P2000"] = "Bottom marker"
    visible.conditional_formatting.add(
        "A1:P2000",
        FormulaRule(formula=["NOW()>0"], fill=PatternFill(patternType="solid", fgColor="FFFFFF")),
    )
    hidden = workbook.create_sheet("Hidden rows")
    hidden["A1"] = "hidden original" if variant == "original" else "hidden changed"
    hidden["P10000"] = "Hidden bottom marker"
    for row in range(1, 10001):
        hidden.row_dimensions[row].hidden = True
    hidden.conditional_formatting.add(
        "A1:P10000",
        FormulaRule(formula=["NOW()>0"], fill=PatternFill(patternType="solid", fgColor="FFFFFF")),
    )
    if visibility_transition and variant == "default":
        hidden.row_dimensions[1].hidden = False
    workbook.save(destination)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--visibility-transition", action="store_true")
    arguments = parser.parse_args()
    arguments.output.mkdir(parents=True, exist_ok=True)
    for name in ("original", "default"):
        create_workbook(arguments.output / f"{name}.xlsx", name, arguments.visibility_transition)
