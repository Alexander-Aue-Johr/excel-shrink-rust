"""Derive a compact, lossless presentation from a public DeltaSieve Atlas."""
import argparse
import base64
import gzip
import hashlib
import json
import os
import re
import shutil
from io import BytesIO
from pathlib import Path

from PIL import Image


WEBP_MAX_DIMENSION = 16_383
TILE_SIZE = 4_096
PAGES_BUDGET_BYTES = 1_000_000_000
REPORT_SCRIPT = re.compile(r'(<script defer src=")((?:\.\./)+)(report-assets/report-view\.js"></script>)')


def encode_webp(image):
    output = BytesIO()
    image.save(output, format="WEBP", lossless=True, exact=True, method=2)
    encoded = output.getvalue()
    with Image.open(BytesIO(encoded)) as decoded:
        if decoded.convert("RGBA").tobytes() != image.tobytes():
            raise ValueError("Lossless WebP did not preserve the source RGBA channels")
    return encoded


def encode_tiled_image(image):
    # SVG image documents cannot load external images. Embed each verified tile.
    elements = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{image.width}" '
        f'height="{image.height}" viewBox="0 0 {image.width} {image.height}">'
    ]
    for y in range(0, image.height, TILE_SIZE):
        for x in range(0, image.width, TILE_SIZE):
            bounds = (x, y, min(x + TILE_SIZE, image.width), min(y + TILE_SIZE, image.height))
            tile = image.crop(bounds)
            payload = base64.b64encode(encode_webp(tile)).decode("ascii")
            elements.append(
                f'<image x="{x}" y="{y}" width="{tile.width}" height="{tile.height}" '
                f'href="data:image/webp;base64,{payload}"/>'
            )
    elements.append("</svg>")
    return "\n".join(elements).encode("ascii")


def presentation_image(source, destination):
    with Image.open(BytesIO(source)) as original:
        image = original.convert("RGBA")
    if max(image.size) <= WEBP_MAX_DIMENSION:
        suffix, encoded = ".webp", encode_webp(image)
    else:
        suffix, encoded = ".svg", encode_tiled_image(image)
    if len(encoded) >= len(source):
        suffix, encoded = ".png", source
    digest = hashlib.sha256(encoded).hexdigest()
    target = destination / "images" / (digest + suffix)
    if not target.exists():
        target.write_bytes(encoded)
    return {
        "file": target.relative_to(destination).as_posix(),
        "width": image.width,
        "height": image.height,
        "source_bytes": len(source),
        "encoded_bytes": len(encoded),
    }


def replace_once(path, old, new):
    text = path.read_text(encoding="utf-8")
    if text.count(old) != 1:
        raise ValueError(f"Unexpected viewer template in {path.name}")
    path.write_text(text.replace(old, new), encoding="utf-8")


def connect_viewer(destination, paths, images):
    mapping = {path: images[digest]["file"] for path, digest in paths.items()}
    (destination / "presentation-image-map.js").write_text(
        "(() => {\n"
        "  const root = new URL('./', document.currentScript.src);\n"
        f"  const paths = new Map(Object.entries({json.dumps(mapping, separators=(',', ':'))}));\n"
        "  globalThis.atlasPresentationImage = (value, base) => {\n"
        "    const url = new URL(value, base);\n"
        "    if (url.origin !== root.origin || !url.pathname.startsWith(root.pathname)) return value;\n"
        "    const image = paths.get(url.pathname.slice(root.pathname.length));\n"
        "    return image ? new URL(image, root).href : value;\n"
        "  };\n"
        "})();\n",
        encoding="utf-8",
    )
    module = '<script type="module" src="./atlas-assets/app.js"></script>'
    replace_once(
        destination / "index.html", module,
        '<script src="./presentation-image-map.js"></script>\n' + module,
    )
    model = destination / "atlas-assets/model.js"
    replace_once(
        model, "  return url.href;",
        "  if (relative.startsWith('atlas-thumbnails/')) {\n"
        "    return globalThis.atlasPresentationImage?.(url.href, atlasRoot) ?? url.href;\n"
        "  }\n"
        "  return url.href;",
    )
    # Resolve the image at load time so comparison grouping keeps native image identities.
    replace_once(
        model, "      image.src = url;",
        "      image.src = globalThis.atlasPresentationImage?.(url, atlasRoot) ?? url;",
    )
    replace_once(
        destination / "report-assets/report-view.js", "    ? path : null;",
        "    ? (globalThis.atlasPresentationImage?.(path, document.baseURI) ?? path) : null;",
    )
    for page in (destination / "runs").rglob("index.html"):
        html = page.read_text(encoding="utf-8")
        updated, count = REPORT_SCRIPT.subn(
            lambda match: f'<script defer src="{match[2]}presentation-image-map.js"></script>\n  {match[0]}',
            html,
        )
        if count != 1:
            raise ValueError(f"Unexpected detail page template in {page}")
        page.write_text(updated, encoding="utf-8")


def export(source, destination):
    source = source.resolve(strict=True)
    destination = destination.resolve()
    if destination == source or source in destination.parents:
        raise ValueError("Choose a new destination outside the source Atlas")
    if os.name == "nt":
        # Nested comparison packages routinely exceed the Win32 path limit.
        source = extended_windows_path(source)
        destination = extended_windows_path(destination)
    if (source / ".incomplete").exists() or not (source / "public-provenance.json").is_file():
        raise ValueError("A completed public-atlas export is required")
    destination.mkdir()
    marker = destination / ".incomplete"
    marker.touch()
    (destination / "images").mkdir()
    paths = {}
    images = {}
    for path in sorted(source.rglob("*")):
        if path.is_symlink():
            raise ValueError(f"Atlas source contains a symbolic link: {path}")
        if not path.is_file():
            continue
        relative = path.relative_to(source)
        if path.suffix == ".png":
            data = path.read_bytes()
            digest = hashlib.sha256(data).hexdigest()
            if digest not in images:
                images[digest] = presentation_image(data, destination)
            paths[relative.as_posix()] = digest
        else:
            output = destination / relative
            output.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, output)
    connect_viewer(destination, paths, images)
    manifest = {"schema": 1, "source_images": images, "paths": paths}
    (destination / "image-derivation.json.gz").write_bytes(
        gzip.compress(json.dumps(manifest, separators=(",", ":")).encode(), mtime=0)
    )
    catalog_path = destination / "atlas-data/catalog.json.gz"
    catalog = json.loads(gzip.decompress(catalog_path.read_bytes()))
    size = sum(file.stat().st_size for file in destination.rglob("*") if file.is_file())
    catalog["storage"] = {
        "bytes_before_index": size,
        "github_pages_budget_bytes": PAGES_BUDGET_BYTES,
        "within_budget": size + 1024 * 1024 < PAGES_BUDGET_BYTES,
    }
    catalog_path.write_bytes(
        gzip.compress(json.dumps(catalog, separators=(",", ":")).encode(), mtime=0)
    )
    marker.unlink()
    return sum(file.stat().st_size for file in destination.rglob("*") if file.is_file())


def extended_windows_path(path):
    value = str(path)
    if value.startswith("\\\\?\\"):
        return path
    if value.startswith("\\\\"):
        return Path("\\\\?\\UNC\\" + value[2:])
    return Path("\\\\?\\" + value)


def file_sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def atlas_file(root, relative):
    if not isinstance(relative, str) or "\\" in relative:
        raise ValueError("Expected a relative Atlas path with forward slashes")
    parts = relative.split("/")
    if not relative or any(part in ("", ".", "..") or ":" in part for part in parts):
        raise ValueError("Atlas path escapes its directory")
    path = root.joinpath(*parts)
    if not path.resolve().is_relative_to(root.resolve()) or not path.is_file():
        raise ValueError(f"Missing or unconfined Atlas file: {relative}")
    return path


def verify(source, destination):
    source = source.resolve(strict=True)
    destination = destination.resolve(strict=True)
    if os.name == "nt":
        source = extended_windows_path(source)
        destination = extended_windows_path(destination)
    for root in (source, destination):
        if (root / ".incomplete").exists():
            raise ValueError("Cannot verify an incomplete Atlas")
    manifest_bytes = (destination / "image-derivation.json.gz").read_bytes()
    manifest = json.loads(gzip.decompress(manifest_bytes))
    if manifest.get("schema") != 1:
        raise ValueError("Unsupported image derivation schema")
    paths = manifest["paths"]
    images = manifest["source_images"]
    source_paths = {path.relative_to(source).as_posix() for path in source.rglob("*.png")}
    if source_paths != set(paths) or set(paths.values()) != set(images):
        raise ValueError("Source images and presentation mappings differ")
    checked_assets = set()
    for relative, digest in paths.items():
        original = atlas_file(source, relative)
        if file_sha256(original) != digest:
            raise ValueError(f"Source PNG hash differs: {relative}")
        info = images[digest]
        target = atlas_file(destination, info["file"])
        if (
            target.stat().st_size != info["encoded_bytes"]
            or original.stat().st_size != info["source_bytes"]
        ):
            raise ValueError(f"Recorded image size differs: {relative}")
        if target not in checked_assets:
            if (
                target.suffix not in (".png", ".webp", ".svg")
                or file_sha256(target) != target.stem
            ):
                raise ValueError(f"Presentation image hash differs: {info['file']}")
            checked_assets.add(target)
    catalogs = []
    for root in (source, destination):
        catalog_bytes = (root / "atlas-data/catalog.json.gz").read_bytes()
        catalog = json.loads(gzip.decompress(catalog_bytes))
        catalog.pop("storage", None)
        catalogs.append(catalog)
    if catalogs[0] != catalogs[1]:
        raise ValueError("Scientific Atlas catalog changed")
    details = {
        observation["detail"]
        for book in catalogs[0]["books"]
        for observation in book["observations"]
        if observation.get("detail")
    }
    for relative in sorted(details):
        original = atlas_file(source, relative)
        if file_sha256(original) != file_sha256(atlas_file(destination, relative)):
            raise ValueError(f"Scientific detail payload changed: {relative}")
        detail = json.loads(gzip.decompress(original.read_bytes()))
        report = detail["image_base"] + "report.json"
        if file_sha256(atlas_file(source, report)) != file_sha256(atlas_file(destination, report)):
            raise ValueError(f"Scientific report changed: {report}")
    total = sum(path.stat().st_size for path in destination.rglob("*") if path.is_file())
    return {
        "bytes": total,
        "within_one_gb": total < PAGES_BUDGET_BYTES,
        "exact_detail_payloads": len(details),
        "verified_source_png_paths": len(paths),
        "unique_presentation_assets": len(checked_assets),
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path, help="Completed public-atlas directory")
    parser.add_argument("destination", type=Path, help="New compact output directory")
    parser.add_argument(
        "--verify", action="store_true",
        help="Verify an existing compact export against its source",
    )
    args = parser.parse_args()
    if args.verify:
        print(json.dumps(verify(args.source, args.destination), indent=2))
    else:
        print(f"Compact Atlas: {export(args.source, args.destination):,} bytes")
