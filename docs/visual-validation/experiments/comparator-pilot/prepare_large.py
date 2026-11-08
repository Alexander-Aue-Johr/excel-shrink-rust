"""Prepare hash-verified large Destatis pairs without consuming comparator trials."""

import argparse
import json
from pathlib import Path
import shutil
import zipfile

from run_pilot import digest, run_measured


NAMES = (
    "statistischer-bericht-rechnungsergebnis-kernhaushalt-gemeinden-2140331217005.xlsx",
    "verkehrsunfaelle-zeitreihen-xlsx-5462403.xlsx",
)


def prepare(args):
    args.output.mkdir(parents=True, exist_ok=False)
    verified = json.loads(args.verified_inputs.read_text(encoding="utf-8"))
    identities = {item["name"]: item["input_sha256"] for item in verified["workbooks"]}
    pairs = []
    with zipfile.ZipFile(args.archive) as archive:
        for name in NAMES:
            directory = args.output / Path(name).stem
            directory.mkdir()
            original = directory / "original.xlsx"
            with archive.open(name) as source, original.open("xb") as output:
                shutil.copyfileobj(source, output, length=1024 * 1024)
            if digest(original) != identities[name]:
                raise ValueError(f"Input identity mismatch for {name}")
            with zipfile.ZipFile(original) as workbook:
                sheets = [member for member in workbook.infolist()
                          if member.filename.startswith("xl/worksheets/") and member.filename.endswith(".xml")]
                dimensions = {"sheets": len(sheets),
                              "sheet_xml_bytes": sum(member.file_size for member in sheets),
                              "largest_sheet_xml_bytes": max(member.file_size for member in sheets)}
            cleanup = directory / "cleanup"
            cleanup.mkdir()
            metrics = run_measured([str(args.shrinker.resolve()), str(original.resolve()),
                                    str(cleanup.resolve()), "--rayon-threads", "1"], cleanup, 300)
            candidate = directory / "candidate.xlsx"
            shutil.copyfile(cleanup / "original.xlsx", candidate)
            pair = {"id": Path(name).stem, "source_name": name,
                    "original": str(original.resolve()), "candidate": str(candidate.resolve()),
                    "sha256": {path.name: digest(path) for path in (original, candidate)},
                    "input_bytes": original.stat().st_size, "output_bytes": candidate.stat().st_size,
                    "dimensions": dimensions, "cleanup_process": metrics,
                    "expected_changes": ["SheetShrink Default cleanup; visual differences require independent validation"]}
            pairs.append(pair)
            manifest = {"version": 1, "synthetic": False, "corpus": "Destatis",
                        "shrinker_sha256": digest(args.shrinker), "pairs": pairs}
            (args.output / "fixtures.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
            print(json.dumps({"name": name, "input_bytes": pair["input_bytes"],
                              "output_bytes": pair["output_bytes"], **dimensions}), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--verified-inputs", type=Path, required=True)
    parser.add_argument("--shrinker", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    prepare(parser.parse_args())
