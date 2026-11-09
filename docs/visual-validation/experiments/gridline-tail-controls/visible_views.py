"""Enable gridlines in both sides of an existing bounded background fixture.

Only literal showGridLines="0" attributes in worksheet parts are changed.
The new output directory retains input/output identities and changed-part counts.
"""

import argparse
import copy
import json
from pathlib import Path
from zipfile import ZipFile

from prepare import identity


def make_visible(source, output):
    before = identity(source)
    changes = {}
    with ZipFile(source) as original:
        names = original.namelist()
        if len(names) != len(set(names)):
            raise ValueError("Duplicate fixture members")
        with ZipFile(output, "x") as variant:
            for entry in original.infolist():
                data = original.read(entry)
                if entry.filename.startswith("xl/worksheets/") and entry.filename.endswith(".xml"):
                    count = data.count(b'showGridLines="0"')
                    if count:
                        changes[entry.filename] = count
                        data = data.replace(b'showGridLines="0"', b'showGridLines="1"')
                variant.writestr(copy.copy(entry), data)
    if not changes:
        raise ValueError("Fixture has no explicitly hidden gridlines")
    with ZipFile(source) as original, ZipFile(output) as variant:
        if variant.testzip() is not None:
            raise ValueError("Visible-view fixture failed its CRC check")
        actual = {name for name in names if original.read(name) != variant.read(name)}
        if actual != set(changes):
            raise ValueError("Unexpected changed fixture members")
    if identity(source) != before:
        raise ValueError("Fixture source changed during preparation")
    return {"source_sha256": before, "output_sha256": identity(output), "changes": changes}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--original", type=Path, required=True)
    parser.add_argument("--variant", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    arguments.output.mkdir(parents=True, exist_ok=False)
    receipt = {
        name: make_visible(source, arguments.output / f"{name}.xlsx")
        for name, source in [("original", arguments.original), ("variant", arguments.variant)]
    }
    (arguments.output / "preparation.json").write_text(
        json.dumps(receipt, indent=2) + "\n", encoding="utf-8"
    )
