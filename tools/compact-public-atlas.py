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


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path, help="Completed public-atlas directory")
    parser.add_argument("destination", type=Path, help="New compact output directory")
    args = parser.parse_args()
    print(f"Compact Atlas: {export(args.source, args.destination):,} bytes")
