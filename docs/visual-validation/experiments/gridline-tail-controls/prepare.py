"""Create the single-fill mutation used by the Mikrozensus tail control.

This byte-level fixture mutation deliberately preserves all other ZIP members.
It refuses a different source identity, ambiguous replacements and existing output.
"""

import argparse
import copy
import hashlib
import json
from pathlib import Path
from zipfile import ZipFile


SOURCE_SHA256 = "3947034b0125e388e9c4fe08db667ac3d4395383dae29cd419ff87e15b572566"


def identity(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def prepare(source, output):
    if identity(source) != SOURCE_SHA256:
        raise ValueError("Unexpected Mikrozensus control source")
    with ZipFile(source) as original:
        names = original.namelist()
        if len(names) != len(set(names)):
            raise ValueError("Duplicate ZIP members")
        styles = original.read("xl/styles.xml")
        white = b'<fgColor indexed="9"/>'
        if styles.count(white) != 1:
            raise ValueError("Expected exactly one indexed-white foreground")
        changed = styles.replace(white, b'<fgColor rgb="FF00FFFF"/>', 1)
        with ZipFile(output, "x") as variant:
            for entry in original.infolist():
                data = changed if entry.filename == "xl/styles.xml" else original.read(entry)
                variant.writestr(copy.copy(entry), data)
    with ZipFile(source) as original, ZipFile(output) as variant:
        if variant.testzip() is not None:
            raise ValueError("Mutation ZIP failed its CRC check")
        changed_parts = [name for name in names if original.read(name) != variant.read(name)]
        if changed_parts != ["xl/styles.xml"]:
            raise ValueError("Mutation changed unexpected members")
    if identity(source) != SOURCE_SHA256:
        raise ValueError("Control source changed during preparation")
    return {
        "source_sha256": SOURCE_SHA256,
        "variant_sha256": identity(output),
        "changed_parts": changed_parts,
        "mutation": "indexed white to opaque cyan",
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    print(json.dumps(prepare(arguments.source, arguments.output), indent=2))
