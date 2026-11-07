"""Compare scalar and batched production geometry reads in one isolated Excel worker."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import time
import zipfile
import xml.etree.ElementTree as ET

REPO = Path(__file__).resolve().parents[4]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--output", required=True, type=Path)
parser.add_argument("--binary", required=True, type=Path)
parser.add_argument("--captures", action="store_true", help="Compare capture packages of 1, 2, 3, 4 and 8 images")
args = parser.parse_args()
OUTPUT = args.output.resolve()
OUTPUT.mkdir(parents=True, exist_ok=False)
BINARY = args.binary.resolve()
helper_path = REPO / "docs/visual-validation/experiments/officejs-million-row-heights/run.py"
spec = importlib.util.spec_from_file_location("height_probe", helper_path)
helper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helper)
parts = helper.fixture_parts(OUTPUT)
# Include uniform, custom and hidden rows in the fixture's first worksheet.
ns = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
rows = []
for row in range(1, 2049):
    attributes = ' hidden="1"' if 257 <= row <= 384 else ''
    if row == 17:
        attributes = ' ht="30" customHeight="1"'
    rows.append(f'<row r="{row}"{attributes}><c r="A{row}"><v>{row}</v></c></row>')
parts["xl/worksheets/sheet1.xml"] = (
    f'<worksheet xmlns="{ns}"><dimension ref="A1:P2048"/>'
    '<sheetFormatPr defaultRowHeight="15"/><sheetData>' + ''.join(rows) +
    '</sheetData></worksheet>').encode()
workbook = OUTPUT / "input.xlsx"
with zipfile.ZipFile(workbook, "w", zipfile.ZIP_DEFLATED) as archive:
    for name, content in parts.items():
        archive.writestr(name, content)
sheet = ET.fromstring(parts["xl/workbook.xml"]).find(f"{{{ns}}}sheets")[0].attrib["name"]
session = OUTPUT / "session"
control = OUTPUT / "control"
helper.publish(control / "control.json", {"paused": False, "hide_excel": True})
helper.publish(session / "capacity.json", {"reserved_clients": 1})
helper.publish(session / "clients/one.json", {
    "client_id": "one", "source_path": str(workbook), "workbook": "one.xlsx",
    "sheets": [{"index": 1, "name": sheet, "bottom_row": 2047,
                "base_right_column": 15, "right_column": 15,
                "horizontal_overflow_required": False, "drawing_fallback_required": False}]})
environment = dict(os.environ, EXCEL_SHRINK_CONTROL_ROOM_DIR=str(control),
    EXCEL_SHRINK_CONTROL_ROOM_GROUP_ID="geometry-probe",
    EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID=OUTPUT.name,
    EXCEL_SHRINK_SQLITE_METADATA="0")
sequence = 0

def command(request):
    """Wait for the actual production worker's result, rejecting diagnostic failures."""
    global sequence
    sequence += 1
    identity = f"geometry-{sequence}"
    helper.publish(session / f"commands/one/{identity}.json", {
        "id": identity, "kind": "diagnosis", "operation": "targeted", "request": request})
    result = helper.wait_until(
        lambda: helper.read_json(session / f"responses/one/{identity}.json"), process, 30)
    if result.get("diagnostic_error") or result.get("error") or result.get("ok") is False:
        raise RuntimeError(json.dumps(result))
    return result

with (OUTPUT / "stdout.log").open("wb") as stdout, (OUTPUT / "stderr.log").open("wb") as stderr:
    process = subprocess.Popen([
        "powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
        str(REPO / "excel-shrink-cli/office-capture/resident.ps1"), "-SessionPath", str(session),
        "-HostExecutable", str(BINARY), "-StartupTimeoutSeconds", "120",
        "-CaptureTimeoutSeconds", "180", "-HideExcel"],
        stdout=stdout, stderr=stderr, env=environment, creationflags=subprocess.CREATE_NO_WINDOW)
results = []
try:
    helper.wait_until(lambda: True if (helper.read_json(session / "metrics.json") or {}).get(
        "primary_clients_ready") else None, process, 120)
    ranges = [f"A{row}:P{row + 31}" for row in range(1, 2049, 32)]
    command({"kind": "measure_batch", "sheet": sheet, "ranges": ranges})
    reference = None
    for mode in ("scalar", "batch", "batch", "scalar"):
        started = time.monotonic()
        if mode == "batch":
            observed = command({"kind": "measure_batch", "sheet": sheet,
                                "ranges": ranges})["ranges"]
            geometries = [{key: value[key] for key in ("top", "left", "width", "height")}
                          for value in observed]
            assert [value["range"] for value in observed] == ranges
        else:
            geometries = [command({"kind": "measure", "sheet": sheet, "range": address})
                          for address in ranges]
            geometries = [{key: value[key] for key in ("top", "left", "width", "height")}
                          for value in geometries]
        if reference is None:
            reference = geometries
        assert geometries == reference
        assert sum(item["height"] == 0 for item in geometries) == 4
        sample = {"mode": mode, "ranges": len(ranges), "wall_seconds": time.monotonic() - started}
        results.append(sample)
        print(json.dumps(sample), flush=True)
    if args.captures:
        from PIL import Image
        reference_images = None
        for batch_size in (1, 2, 3, 4, 8):
            started = time.monotonic()
            observations = []
            selected = ranges[:12]
            for offset in range(0, len(selected), batch_size):
                addresses = selected[offset:offset + batch_size]
                response = command({"kind": "capture_many", "sheet": sheet,
                    "ranges": [{"range": address} for address in addresses]})
                assert [item["range"] for item in response["images"]] == addresses
                for item in response["images"]:
                    if item.get("non_rendering"):
                        observations.append({"range": item["range"], "zero_area": True})
                    else:
                        with Image.open(item["image_path"]) as image:
                            observations.append({"range": item["range"], "size": image.size,
                                "rgba_sha256": hashlib.sha256(image.convert("RGBA").tobytes()).hexdigest()})
            if reference_images is None:
                reference_images = observations
            assert observations == reference_images
            sample = {"mode": "capture", "batch_size": batch_size,
                "ranges": len(selected), "wall_seconds": time.monotonic() - started,
                "all_pixels_and_zero_area_results_equal": True}
            results.append(sample)
            print(json.dumps(sample), flush=True)

finally:
    helper.close_client(session, process)
    helper.publish(OUTPUT / "result.json", {
        "binary_sha256": hashlib.sha256(BINARY.read_bytes()).hexdigest(),
        "results": results, "final_metrics": helper.read_json(session / "metrics.json")})
print(str(OUTPUT), flush=True)
