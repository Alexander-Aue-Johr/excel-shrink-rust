"""Prepare an isolated memchr 2.8.1 architecture-dispatch experiment."""

import argparse
import hashlib
import json
import shutil
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--memchr-source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    manifest = arguments.memchr_source / "Cargo.toml"
    original_manifest = manifest.read_text(encoding="utf-8")
    if 'version = "2.8.1"' not in original_manifest or 'name = "memchr"' not in original_manifest:
        raise ValueError("Expected the normalized memchr 2.8.1 crate source")
    if arguments.output.exists():
        raise ValueError("Choose a new output directory")
    shutil.copytree(arguments.memchr_source, arguments.output)
    changed_conditions = {}
    for path in arguments.output.rglob("*.rs"):
        source = path.read_text(encoding="utf-8")
        count = source.count('target_arch = "x86_64"')
        if count:
            source = source.replace(
                'target_arch = "x86_64"',
                'all(target_arch = "x86_64", not(feature = "force-portable"))',
            )
            path.write_text(source, encoding="utf-8", newline="\n")
            changed_conditions[path.relative_to(arguments.output).as_posix()] = count
    expected = {"src/lib.rs": 1, "src/memchr.rs": 14, "src/vector.rs": 2,
                "src/arch/mod.rs": 1, "src/memmem/searcher.rs": 13}
    if changed_conditions != expected:
        raise ValueError("Architecture conditions differ from the audited dependency")
    patched_manifest = original_manifest.replace(
        "[features]\n", "[features]\nforce-portable = []\n", 1
    )
    if patched_manifest == original_manifest:
        raise ValueError("Cannot locate the dependency feature table")
    (arguments.output / "Cargo.toml").write_text(patched_manifest, encoding="utf-8", newline="\n")
    receipt = {
        "upstream": "memchr 2.8.1",
        "original_manifest_sha256": hashlib.sha256(manifest.read_bytes()).hexdigest(),
        "changed_conditions": changed_conditions,
        "patched_source_sha256": {
            path.relative_to(arguments.output).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in sorted(arguments.output.rglob("*.rs"))
        },
    }
    (arguments.output / "override-receipt.json").write_text(
        json.dumps(receipt, indent=2) + "\n", encoding="utf-8"
    )


if __name__ == "__main__":
    main()
