"""Place verified presentation images on a second static site without changing reports."""

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
from urllib.parse import urljoin, urlparse


MAP_PREFIX = "  const paths = new Map(Object.entries("
MAP_SUFFIX = "));"


def absolute_path(value, strict=False):
    path = Path(value).resolve(strict=strict)
    text = str(path)
    if os.name != "nt" or text.startswith("\\\\?\\"):
        return path
    if text.startswith("\\\\"):
        return Path("\\\\?\\UNC\\" + text[2:])
    return Path("\\\\?\\" + text)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def relative_path(value):
    path = PurePosixPath(value)
    if path.is_absolute() or not path.parts or any(part in ("..", ".") for part in path.parts):
        raise ValueError(f"Invalid relative publication path: {value}")
    if "\\" in value or ":" in value:
        raise ValueError(f"Invalid relative publication path: {value}")
    return path


def read_mapping(source):
    script = (source / "presentation-image-map.js").read_text(encoding="utf-8")
    lines = script.splitlines()
    matches = [index for index, line in enumerate(lines) if line.startswith(MAP_PREFIX)]
    if len(matches) != 1:
        raise ValueError("Expected exactly one generated presentation-image mapping")
    index = matches[0]
    line = lines[index]
    if not line.endswith(MAP_SUFFIX):
        raise ValueError("Unexpected presentation-image mapping terminator")
    mapping = json.loads(line[len(MAP_PREFIX):-len(MAP_SUFFIX)])
    for original, presentation in mapping.items():
        relative_path(original)
        path = relative_path(presentation)
        if path.parts[0] != "images":
            raise ValueError("Presentation assets must be inside images/")
    return lines, index, mapping


def new_output(path):
    path = absolute_path(path)
    path.mkdir(parents=True, exist_ok=False)
    return path


def partition(configuration):
    base_url = configuration["shared_base_url"]
    parsed = urlparse(base_url)
    site_url = urlparse(configuration["site_base_url"])
    if (parsed.scheme != "https" or not parsed.netloc or not base_url.endswith("/")
            or parsed.query or parsed.fragment or parsed.username or parsed.password):
        raise ValueError("The shared image base must be an absolute HTTPS directory URL")
    if (site_url.scheme, site_url.netloc) != (parsed.scheme, parsed.netloc):
        raise ValueError("Both sites must share an origin for pixel inspection")
    budget = configuration["shared_budget_bytes"]
    if isinstance(budget, bool) or not isinstance(budget, int) or budget <= 0:
        raise ValueError("The shared image budget must be a positive integer")

    atlases = []
    assets = {}
    destinations = set()
    for descriptor in configuration["atlases"]:
        source = absolute_path(descriptor["source"], strict=True)
        destination = relative_path(descriptor["destination"])
        if destination in destinations:
            raise ValueError("Duplicate Atlas destination")
        if any(destination in other.parents or other in destination.parents for other in destinations):
            raise ValueError("Atlas destinations must not contain each other")
        destinations.add(destination)
        if (source / ".incomplete").exists():
            raise ValueError("Cannot partition an incomplete Atlas")
        lines, index, mapping = read_mapping(source)
        for relative in set(mapping.values()):
            path = source / relative
            if path.is_symlink() or not path.is_file():
                raise ValueError(f"Missing or symbolic presentation asset: {relative}")
            sha256 = digest(path)
            if path.stem != sha256:
                raise ValueError(f"Presentation image checksum differs: {relative}")
            asset = assets.setdefault(relative, {
                "source": path,
                "bytes": path.stat().st_size,
                "sha256": sha256,
                "references": 0,
            })
            if asset["sha256"] != sha256:
                raise ValueError("Presentation asset identity collision")
            asset["references"] += 1
        atlases.append((source, destination, lines, index, mapping))

    selected = set()
    shared_bytes = 0
    # Sharing a duplicate removes a copy from each Atlas. Larger assets follow.
    for relative, asset in sorted(assets.items(), key=lambda item: (
        -item[1]["references"], -item[1]["bytes"], item[0]
    )):
        if shared_bytes + asset["bytes"] <= budget:
            selected.add(relative)
            shared_bytes += asset["bytes"]

    outputs = [absolute_path(configuration[key]) for key in ("site_output", "shared_output")]
    for index, output in enumerate(outputs):
        for other in outputs[:index] + [atlas[0] for atlas in atlases]:
            if output == other or output in other.parents or other in output.parents:
                raise ValueError("Sources and output directories must be disjoint")
    site = new_output(outputs[0])
    shared = new_output(outputs[1])
    marker = site / ".incomplete"
    marker.touch()
    (shared / ".incomplete").touch()
    for relative in sorted(selected):
        target = shared / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(assets[relative]["source"], target)
        if digest(target) != assets[relative]["sha256"]:
            raise ValueError("Shared asset copy checksum differs")

    for source, destination, lines, index, mapping in atlases:
        target = site / destination
        for path in source.rglob("*"):
            if path.is_symlink():
                raise ValueError("Symbolic links are not publication inputs")
            if not path.is_file():
                continue
            relative = path.relative_to(source).as_posix()
            if relative in selected:
                continue
            output = target / relative
            output.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, output)
        relocated = {
            original: urljoin(base_url, relative) if relative in selected else relative
            for original, relative in mapping.items()
        }
        lines[index] = MAP_PREFIX + json.dumps(relocated, separators=(",", ":")) + MAP_SUFFIX
        (target / "presentation-image-map.js").write_text("\n".join(lines) + "\n", encoding="utf-8")
        for path in source.rglob("*"):
            if not path.is_file():
                continue
            relative = path.relative_to(source).as_posix()
            if relative == "presentation-image-map.js":
                continue
            actual = shared / relative if relative in selected else target / relative
            if digest(actual) != digest(path):
                raise ValueError(f"Publication copy differs: {destination}/{relative}")

    receipt = {
        "schema": 1,
        "shared_base_url": base_url,
        "shared_image_bytes": shared_bytes,
        "shared_images": {
            relative: {"sha256": assets[relative]["sha256"], "bytes": assets[relative]["bytes"]}
            for relative in sorted(selected)
        },
        "atlases": [str(destination) for _, destination, _, _, _ in atlases],
        "verification": "All input files except the generated URL map remain byte-identical across the two outputs.",
    }
    for root in (site, shared):
        (root / "publication-shards.json").write_text(json.dumps(receipt, indent=2), encoding="utf-8")
        (root / ".nojekyll").touch()
    receipt["site_bytes"] = sum(path.stat().st_size for path in site.rglob("*") if path.is_file())
    receipt["shared_site_bytes"] = sum(path.stat().st_size for path in shared.rglob("*") if path.is_file())
    if max(receipt["site_bytes"], receipt["shared_site_bytes"]) >= 1_000_000_000:
        raise ValueError("Partitioned publication still exceeds a one-GB site budget")
    for root in (site, shared):
        (root / ".incomplete").unlink()
    return receipt


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("configuration", type=Path)
    arguments = parser.parse_args()
    settings = json.loads(arguments.configuration.read_text(encoding="utf-8"))
    result = partition(settings)
    print(json.dumps({key: value for key, value in result.items() if key != "shared_images"}, indent=2))
