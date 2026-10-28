"""Opt-in native Excel check for a capture larger than 32 megapixels.

This runs one private resident client with a synthetic workbook. The right edge,
bottom edge, and bottom-right cell have distinct fills. The native capture
records a PNG; a second invocation with --verify-receipt checks its actual edge
pixels from WSL using Pillow. No study runner or corpus output is touched.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
import zipfile


ROOT = Path(__file__).resolve().parents[3]
OUTPUT = ROOT / ".tmp/large-capture-smoke"
DEFAULT_SHEET_ROWS = 250
SHEET_COLUMNS = 120
NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0)


def publish(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    pending = path.with_suffix(".pending")
    pending.write_text(json.dumps(value, indent=2), encoding="utf-8")
    os.replace(pending, path)


def read(path):
    try:
        return json.loads(path.read_text(encoding="utf-8-sig"))
    except (OSError, ValueError):
        return None


def prepare_fixture(directory, sheet_rows):
    directory.mkdir(parents=True, exist_ok=True)
    source = directory / "edge-sentinels.xlsx"
    main_namespace = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
    relationship_namespace = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"

    def column_name(index):
        name = ""
        while index:
            index, remainder = divmod(index - 1, 26)
            name = chr(65 + remainder) + name
        return name

    last_column = column_name(SHEET_COLUMNS)
    rows = []
    for row in range(1, sheet_rows + 1):
        cells = []
        if row == 1:
            cells.append('<c r="A1"><v>1</v></c>')
        if row == sheet_rows:
            cells.extend(f'<c r="{column_name(column)}{row}" s="2"/>'
                         for column in range(1, SHEET_COLUMNS))
            cells.append(f'<c r="{last_column}{row}" s="3"><v>2</v></c>')
        else:
            cells.append(f'<c r="{last_column}{row}" s="1"/>')
        rows.append(f'<row r="{row}">{"".join(cells)}</row>')

    sheet = (
        f'<worksheet xmlns="{main_namespace}">'
        f'<dimension ref="A1:{last_column}{sheet_rows}"/>'
        '<sheetFormatPr defaultRowHeight="15"/>'
        f'<cols><col min="1" max="{SHEET_COLUMNS}" width="11" customWidth="1"/></cols>'
        f'<sheetData>{"".join(rows)}</sheetData>'
        '</worksheet>'
    )
    fills = [
        '<fill><patternFill patternType="none"/></fill>',
        '<fill><patternFill patternType="gray125"/></fill>',
        *(
            '<fill><patternFill patternType="solid">'
            f'<fgColor rgb="{color}"/><bgColor indexed="64"/>'
            '</patternFill></fill>'
            for color in ("FFFF0000", "FF0000FF", "FFFF00FF")
        ),
    ]
    styles = (
        f'<styleSheet xmlns="{main_namespace}">'
        '<fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>'
        f'<fills count="5">{"".join(fills)}</fills>'
        '<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>'
        '<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>'
        '<cellXfs count="4">'
        + ''.join(f'<xf numFmtId="0" fontId="0" fillId="{fill}" borderId="0" xfId="0" applyFill="1"/>'
                  for fill in (0, 2, 3, 4))
        + '</cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/>'
        '</cellStyles></styleSheet>'
    )
    parts = {
        '[Content_Types].xml': (
            '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
            '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
            '<Default Extension="xml" ContentType="application/xml"/>'
            '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>'
            '<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
            '<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>'
            '</Types>'
        ),
        '_rels/.rels': (
            '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>'
            '</Relationships>'
        ),
        'xl/workbook.xml': (
            f'<workbook xmlns="{main_namespace}" xmlns:r="{relationship_namespace}">'
            '<sheets><sheet name="Large capture" sheetId="1" r:id="rId1"/></sheets></workbook>'
        ),
        'xl/_rels/workbook.xml.rels': (
            '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>'
            '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>'
            '</Relationships>'
        ),
        'xl/worksheets/sheet1.xml': sheet,
        'xl/styles.xml': styles,
    }
    with zipfile.ZipFile(source, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, content in parts.items():
            archive.writestr(name, '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' + content)
    return source


def await_response(session, command_id, process, deadline):
    path = session / f"responses/one/{command_id}.json"
    while time.monotonic() < deadline:
        result = read(path)
        if result is not None:
            if result.get("ok") is not True:
                raise RuntimeError(f"Resident command {command_id} failed: {result}")
            return result
        if process.poll() is not None:
            raise RuntimeError(f"Resident exited before {command_id}: {process.returncode}")
        time.sleep(0.25)
    raise TimeoutError(f"No resident response for {command_id}")


def count_color(image, box, color):
    def matches(pixel):
        red, green, blue = pixel
        if color == "red":
            return red >= 220 and green <= 80 and blue <= 80
        if color == "blue":
            return blue >= 220 and red <= 80 and green <= 80
        return red >= 220 and blue >= 220 and green <= 80

    pixels = image.crop(box).convert("RGB").tobytes()
    return sum(matches(pixels[offset:offset + 3]) for offset in range(0, len(pixels), 3))


def check_image(path, expected):
    from PIL import Image

    with Image.open(path) as image:
        image.load()  # Decode the entire PNG; header dimensions alone are insufficient.
        width, height = image.size
        assert (width, height) == expected, (image.size, expected)
        assert 32_000_000 < width * height <= 64_000_000, image.size
        # Stay inside the edge cells, beyond a possible one-pixel range border.
        counts = {
            "right_red": count_color(image, (width - 120, height // 2, width - 2, height // 2 + 40), "red"),
            "bottom_blue": count_color(image, (width // 2, height - 35, width // 2 + 120, height - 2), "blue"),
            "corner_magenta": count_color(image, (width - 120, height - 35, width - 2, height - 2), "magenta"),
        }
        assert all(count >= 20 for count in counts.values()), counts
        edge_pixels = {
            "right": image.getpixel((width - 2, height // 2))[:3],
            "bottom": image.getpixel((width // 2, height - 2))[:3],
            "corner": image.getpixel((width - 2, height - 2))[:3],
        }
        assert edge_pixels == {
            "right": (255, 0, 0),
            "bottom": (0, 0, 255),
            "corner": (255, 0, 255),
        }, edge_pixels
        return {"width": width, "height": height, "pixels": width * height,
                "sentinels": counts, "edge_pixels": edge_pixels}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true", help="Launch an isolated native Excel client")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/excel-shrink-cli.exe")
    parser.add_argument("--deadline", type=int, default=900)
    parser.add_argument("--rows", type=int, default=DEFAULT_SHEET_ROWS,
                        help="Fixture height; 346 rows was near 64 MP on the tested Excel host")
    parser.add_argument("--verify-receipt", type=Path,
                        help="On WSL, inspect the native PNG and all three edge sentinels")
    args = parser.parse_args()
    if args.verify_receipt:
        receipt = read(args.verify_receipt)
        if not receipt or receipt.get("status") != "captured_unverified":
            parser.error("Expected a successful native capture receipt")
        capture = receipt["capture"]
        png_path = capture["png"]
        if os.name != "nt" and png_path.startswith("\\\\?\\"):
            png_path = png_path[4:]
        if os.name != "nt" and len(png_path) > 2 and png_path[1:3] == ":\\":
            png_path = "/mnt/" + png_path[0].lower() + png_path[2:].replace("\\", "/")
        verification = check_image(Path(png_path), (capture["width"], capture["height"]))
        print(json.dumps({"status": "passed", "verification": verification}))
        return 0
    if args.deadline < 120:
        parser.error("--deadline must be at least 120 seconds")
    if not 2 <= args.rows <= 1000:
        parser.error("--rows must be between 2 and 1000")
    directory = OUTPUT / f"run-{time.time_ns()}"
    source = prepare_fixture(directory, args.rows)
    if not args.run:
        print(json.dumps({"prepared": str(source), "excel_started": False}))
        return 0
    if os.name != "nt":
        parser.error("Native Excel requires Windows Python")
    binary = args.binary.resolve()
    if not binary.is_file():
        parser.error(f"Windows capture host not found: {binary}")

    session = directory / "session"
    control = directory / "control"
    publish(control / "control.json", {"paused": False, "hide_excel": True})
    publish(session / "clients/one.json", {
        "client_id": "one",
        "source_path": str(source),
        # resident.ps1 stages each client under its client ID before Excel opens it.
        "workbook": "one.xlsx",
        "sheets": [{
            "index": 1,
            "name": "Large capture",
            "bottom_row": args.rows - 1,
            "base_right_column": SHEET_COLUMNS - 1,
            "right_column": SHEET_COLUMNS - 1,
            "horizontal_overflow_required": False,
            "drawing_fallback_required": False,
        }],
    })
    deadline = time.monotonic() + args.deadline
    receipt = {"status": "running", "source": str(source), "binary": str(binary)}
    process = None
    try:
        with (directory / "resident.stdout.log").open("wb") as stdout, (
            directory / "resident.stderr.log"
        ).open("wb") as stderr:
            environment = {**os.environ,
                           "EXCEL_SHRINK_CONTROL_ROOM_DIR": str(control),
                           "EXCEL_SHRINK_CONTROL_ROOM_GROUP_ID": "large-capture-smoke",
                           "EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID": "large-capture-smoke"}
            process = subprocess.Popen([
                "powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass",
                "-File", str(ROOT / "excel-shrink-cli/office-capture/resident.ps1"),
                "-SessionPath", str(session), "-HostExecutable", str(binary),
                "-StartupTimeoutSeconds", "120", "-CaptureTimeoutSeconds", "600", "-HideExcel",
            ], stdout=stdout, stderr=stderr, env=environment, creationflags=NO_WINDOW)
            receipt["launcher_pid"] = process.pid
            while time.monotonic() < deadline:
                metrics = read(session / "metrics.json") or {}
                if metrics.get("primary_clients_ready"):
                    break
                if process.poll() is not None:
                    raise RuntimeError(f"Resident exited during startup: {metrics}")
                time.sleep(0.25)
            else:
                raise TimeoutError("Resident client did not become ready")

            publish(session / "commands/one/plan.json", {
                "id": "plan", "kind": "plan", "tile_megapixels": 64,
                "sheets": [{
                    "index": 1,
                    "name": "Large capture",
                    "bottom_row": args.rows - 1,
                    "base_right_column": SHEET_COLUMNS - 1,
                    "right_column": SHEET_COLUMNS - 1,
                    "horizontal_overflow_required": False,
                    "drawing_fallback_required": False,
                }],
            })
            plan = await_response(session, "plan", process, deadline)
            tiles = plan.get("tiles", [])
            assert len(tiles) == 1, f"Expected one >32 MP tile, got {tiles}"
            tile = tiles[0]
            assert tile["core_rows"] == [0, args.rows], tile
            assert tile["capture_pixels"] > 32_000_000, tile

            publish(session / "commands/one/image.json", {
                "id": "image", "kind": "capture_images", "capture_id": "plan", "tiles": tiles,
            })
            result = await_response(session, "image", process, deadline)
            images = result.get("images", [])
            assert len(images) == 1, images
            image = images[0]
            path = Path(image["image_path"])
            assert path.is_file(), path
            assert hashlib.sha256(path.read_bytes()).hexdigest() == image["sha256"]
            assert 32_000_000 < image["width"] * image["height"] <= 64_000_000, image
            receipt["capture"] = {
                "width": image["width"], "height": image["height"],
                "pixels": image["width"] * image["height"], "range": image["range"],
                "png": str(path),
            }
            receipt["status"] = "captured_unverified"
    except Exception as error:
        receipt["status"] = "failed"
        receipt["error"] = f"{type(error).__name__}: {error}"
    finally:
        if process is not None and process.poll() is None:
            publish(session / "finish.json", {"complete": True})
            try:
                process.wait(timeout=90)
            except subprocess.TimeoutExpired:
                subprocess.run(["taskkill", "/F", "/T", "/PID", str(process.pid)],
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                               creationflags=NO_WINDOW, timeout=20)
                process.wait(timeout=20)
        receipt["launcher_exit_code"] = process.returncode if process is not None else None
        if receipt["status"] == "captured_unverified" and process.returncode != 0:
            receipt["status"] = "failed"
            receipt["error"] = f"Resident launcher exited with {process.returncode}"
        publish(directory / "receipt.json", receipt)
        print(json.dumps({"status": receipt["status"], "receipt": str(directory / "receipt.json")}))
    return 0 if receipt["status"] == "captured_unverified" else 1


if __name__ == "__main__":
    raise SystemExit(main())
