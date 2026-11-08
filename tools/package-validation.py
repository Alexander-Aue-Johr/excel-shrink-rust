"""Package supplied Windows binaries and tracked runtime assets into a new directory."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

RUNTIME_ASSETS = {
    Path("office-capture/run-targeted-comparison.ps1"),
    Path("office-capture/targeted-comparison-host.ps1"),
    Path("office-capture/control-room/sheet-scope.html"),
    Path("src/regression/archive_processes.ps1"),
    Path("src/regression/ArchiveProcesses.cs"),
}


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def runtime_members(tracked):
    """Keep runtime sources while excluding tests, local data and generated dependencies."""
    members = {}
    for name in tracked:
        path = Path(name)
        parts = path.parts
        if parts[:2] == ("excel-shrink-cli", "office-capture"):
            relative = Path(*parts[2:])
            if any(part in {"tests", "node_modules", "bin", "obj"} for part in relative.parts):
                continue
            if relative.name.endswith((".test.js", ".test.cjs")):
                continue
            if relative.suffix.lower() not in {".ps1", ".psm1", ".cs", ".js", ".html", ".css", ".xml", ".png", ".svg", ".ico", ".json"}:
                continue
            members[Path("office-capture") / relative] = path
        elif parts[:3] == ("excel-shrink-cli", "src", "regression") and len(parts) == 4:
            if path.suffix.lower() in {".ps1", ".cs"}:
                members[Path("src/regression") / path.name] = path
    return members


def verify(root):
    """Verify a closed package and every listed member before trusting its contents."""
    if (root / ".incomplete").exists():
        raise ValueError("Runtime package publication is incomplete")
    manifest = json.loads((root / "release-manifest.json").read_text(encoding="utf-8"))
    if manifest.get("version") != 1 or manifest.get("kind") != "deltasieve-runtime":
        raise ValueError("Unsupported runtime package manifest")
    expected = manifest["files"]
    required = {path.as_posix() for path in RUNTIME_ASSETS} | {"deltasieve.exe", "excel-shrink-cli.exe"}
    if not required.issubset(expected):
        raise ValueError("Runtime package is missing required members")
    for name, identity in expected.items():
        relative = Path(name)
        if relative.is_absolute() or ".." in relative.parts or ":" in name or "\\" in name:
            raise ValueError("Unsafe package member")
        path = root / relative
        if path.is_symlink() or not path.resolve().is_relative_to(root.resolve()):
            raise ValueError("Package member escapes its root")
        if path.stat().st_size != identity["bytes"] or digest(path) != identity["sha256"]:
            raise ValueError(f"Runtime package member changed: {name}")
    actual = {path.relative_to(root).as_posix() for path in root.rglob("*") if path.is_file()}
    if actual != set(expected) | {"release-manifest.json"}:
        raise ValueError("Runtime package contains unexpected or missing files")
    return manifest


def package(repository, binary_directory, output, tracked, checkout):
    """Publish a new package with its manifest last, preserving incomplete failures for inspection."""
    members = runtime_members(tracked)
    if not RUNTIME_ASSETS.issubset(members):
        raise ValueError("Required tracked runtime assets are missing")
    sources = {target: repository / source for target, source in members.items()}
    for name in ("deltasieve.exe", "excel-shrink-cli.exe"):
        sources[Path(name)] = binary_directory / name
    for target, source in sources.items():
        if source.is_symlink() or not source.is_file():
            raise ValueError(f"Missing or linked runtime input: {target}")
        if target in members and not source.resolve().is_relative_to(repository.resolve()):
            raise ValueError("Tracked runtime input escapes the repository")
    output.mkdir()
    marker = output / ".incomplete"
    marker.write_text("Runtime package publication in progress\n", encoding="utf-8")
    files = {}
    for target, source in sorted(sources.items()):
        before = digest(source)
        destination = output / target
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
        if digest(destination) != before or digest(source) != before:
            raise ValueError(f"Runtime input changed while copying: {target}")
        files[target.as_posix()] = {"sha256": before, "bytes": destination.stat().st_size}
    manifest = {
        "version": 1,
        "kind": "deltasieve-runtime",
        "asset_checkout": checkout,
        "binary_source_attribution": "Supplied executables are identified by bytes; their source commit is not inferred from the asset checkout.",
        "files": files,
    }
    (output / "release-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    marker.unlink()
    return verify(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--binary-directory", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--verify", type=Path)
    args = parser.parse_args()
    if args.verify:
        manifest = verify(args.verify)
    else:
        if args.binary_directory is None or args.output is None:
            parser.error("Packaging requires --binary-directory and --output")
        tracked = subprocess.check_output(["git", "ls-files", "-z"], cwd=args.repository).decode("utf-8").split("\0")
        checkout = {
            "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=args.repository, text=True).strip(),
            "tracked_dirty": bool(subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=args.repository)),
        }
        manifest = package(args.repository, args.binary_directory, args.output, tracked, checkout)
    print(json.dumps({"verified": True, "files": len(manifest["files"])}))


if __name__ == "__main__":
    main()
