"""Freeze the Enron v2 source ZIP catalog from a locally retained Archive.org response."""

import argparse
import hashlib
import json
import re
from pathlib import Path


ITEM = "edrm.enron.email.data.set.v2.xml"
ZIP_NAME = re.compile(r"edrm-enron-v2_[A-Za-z0-9_-]+_xml(?:_[0-9]+of[0-9]+)?\.zip")


def build_profile(metadata_bytes, retrieved_at):
    metadata = json.loads(metadata_bytes.decode("utf-8-sig"))
    description = metadata["metadata"]
    if description["identifier"] != ITEM:
        raise ValueError("Unexpected Internet Archive item")
    archives = []
    names = set()
    for file in metadata["files"]:
        name = file["name"]
        if not name.lower().endswith(".zip"):
            continue
        if not ZIP_NAME.fullmatch(name) or name.casefold() in names:
            raise ValueError(f"Unexpected or duplicate source ZIP name: {name}")
        names.add(name.casefold())
        size = int(file["size"])
        sha1 = file["sha1"].lower()
        md5 = file["md5"].lower()
        if size <= 0 or not re.fullmatch(r"[0-9a-f]{40}", sha1):
            raise ValueError(f"Invalid source ZIP identity: {name}")
        if not re.fullmatch(r"[0-9a-f]{32}", md5):
            raise ValueError(f"Invalid source ZIP MD5: {name}")
        archives.append({
            "name": name,
            "url": f"https://archive.org/download/{ITEM}/{name}",
            "bytes": size,
            "sha1": sha1,
            "md5": md5,
        })
    if not archives:
        raise ValueError("No source ZIP files in metadata")
    archives.sort(key=lambda archive: archive["name"])
    return {
        "version": 2,
        "id": "enron-edrm-v2",
        "name": "Enron EDRM v2 XML",
        "source": {
            "identifier": ITEM,
            "url": f"https://archive.org/details/{ITEM}",
            "metadata_url": f"https://archive.org/metadata/{ITEM}",
            "title": description["title"],
            "creator": description["creator"],
            "retrieved_at": retrieved_at,
            "metadata_document_sha256": hashlib.sha256(metadata_bytes).hexdigest(),
        },
        "archives": archives,
        "extraction": {
            "kind": "recursive-edrm-attachments",
            "status": "pending-recipe",
        },
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--metadata", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--retrieved-at", required=True)
    args = parser.parse_args()
    profile = build_profile(args.metadata.read_bytes(), args.retrieved_at)
    args.output.write_text(json.dumps(profile, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"{len(profile['archives'])} archives, {sum(item['bytes'] for item in profile['archives'])} bytes")


if __name__ == "__main__":
    main()
