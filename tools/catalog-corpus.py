"""Freeze workbook identities from an existing corpus ZIP without extracting files."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import zipfile


def sha256(stream):
    digest = hashlib.sha256()
    for chunk in iter(lambda: stream.read(1024 * 1024), b""):
        digest.update(chunk)
    return digest.hexdigest()


def catalog(archive_path, key, name, url, expected_sha256):
    before = archive_path.stat()
    with archive_path.open("rb") as stream:
        archive_hash = sha256(stream)
    if archive_hash != expected_sha256.lower():
        raise ValueError("Archive checksum differs from the declared corpus identity")
    members = []
    names = set()
    with zipfile.ZipFile(archive_path) as archive:
        for index, entry in enumerate(archive.infolist()):
            if entry.is_dir() or not entry.filename.lower().endswith((".xls", ".xlsx")):
                continue
            relative = PurePosixPath(entry.filename.replace("\\", "/"))
            if relative.is_absolute() or ".." in relative.parts or ":" in entry.filename:
                raise ValueError("Workbook name is not a safe relative path")
            if entry.filename in names:
                raise ValueError("Duplicate workbook names require an explicit occurrence policy")
            names.add(entry.filename)
            with archive.open(entry) as stream:
                digest = sha256(stream)
            members.append({"index": index, "name": entry.filename,
                            "bytes": entry.file_size, "crc32": entry.CRC, "sha256": digest})
    after = archive_path.stat()
    if (before.st_size, before.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
        raise ValueError("Archive changed during cataloging")
    if not members:
        raise ValueError("Archive contains no workbook members")
    return {"version": 1, "id": key, "name": name,
            "archive": {"url": url, "sha256": archive_hash, "bytes": before.st_size},
            "workbooks": members}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--id", required=True)
    parser.add_argument("--name", required=True)
    parser.add_argument("--url", required=True)
    parser.add_argument("--expected-sha256", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        raise FileExistsError("Choose a new manifest path")
    manifest = catalog(args.archive, args.id, args.name, args.url, args.expected_sha256)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x", encoding="utf-8") as output:
        json.dump(manifest, output, indent=2, ensure_ascii=False)
        output.write("\n")
    print(json.dumps({"id": args.id, "workbooks": len(manifest["workbooks"]),
                      "archive_sha256": manifest["archive"]["sha256"]}))


if __name__ == "__main__":
    main()
