#!/usr/bin/env python3
"""Remove local generated Excel Shrink state without touching research inputs/evidence."""
from __future__ import annotations

import argparse
import shutil
from pathlib import Path

CACHE_PATHS = (".tmp", "regression/tmp")
FRESH_EXTRA_PATHS = ("target", "logs")


def remove_path(root: Path, relative: str, dry_run: bool) -> None:
    path = root / relative
    root_resolved = root.resolve()
    parent = path.parent.resolve()
    try:
        parent.relative_to(root_resolved)
    except ValueError as exc:
        raise RuntimeError(f"refusing to remove path outside repository: {path}") from exc

    if not path.exists() and not path.is_symlink():
        print(f"skip   {path} (absent)")
        return

    print(f"remove {path}")
    if dry_run:
        return

    if path.is_symlink() or path.is_file():
        path.unlink()
    else:
        shutil.rmtree(path)


def main() -> int:
    parser = argparse.ArgumentParser()
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument(
        "--cache-only",
        action="store_true",
        help="remove .tmp and the legacy regression/tmp directory",
    )
    mode.add_argument(
        "--fresh",
        action="store_true",
        help="also remove target and logs, approximating a fresh local checkout",
    )
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    root = Path(__file__).resolve().parents[1]
    paths = list(CACHE_PATHS)
    if args.fresh:
        paths.extend(FRESH_EXTRA_PATHS)

    for relative in paths:
        remove_path(root, relative, args.dry_run)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
