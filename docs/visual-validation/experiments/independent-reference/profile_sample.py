"""Freeze reference selections from a versioned corpus inventory without downloading it."""

import argparse
import gzip
import hashlib
import json
from pathlib import Path

from sample import encode, select


def freeze(profile_path, output, seed):
    payload = profile_path.read_bytes()
    profile = json.loads(payload)
    entries = [
        {"archive_index": item["index"], "member_name": item["name"],
         "bytes": item["bytes"], "sha256": item["sha256"]}
        for item in profile["workbooks"]
    ]
    frame, selected, strata = select(entries, profile["archive"]["sha256"], seed, 1)
    output.mkdir(parents=True, exist_ok=False)
    selection = {
        "schema": 1, "population": len(frame),
        "source_profile_sha256": hashlib.sha256(payload).hexdigest(),
        "archive_sha256": profile["archive"]["sha256"], "seed": seed,
        "strata": strata, "selected": selected, "prior_exposure": "unclassified",
        "status": "selection frozen before joining optimization results",
    }
    (output / "selection.json").write_text(
        json.dumps(selection, indent=2), encoding="utf-8",
    )
    with gzip.open(output / "frame.jsonl.gz", "wb") as stream:
        stream.write(b"".join(encode(item) + b"\n" for item in frame))
    return selection


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--seed", required=True)
    arguments = parser.parse_args()
    print(json.dumps(freeze(arguments.profile, arguments.output, arguments.seed)))
