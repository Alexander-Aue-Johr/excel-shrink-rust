"""Recover Microsoft Graph chart data after recursive corpus extraction.

Requires Windows, desktop PowerPoint, olefile, pywin32 and psutil. Originals and
the extraction ledger are immutable inputs. The output is a separate derived
inventory, not a replacement for the corpus's original workbook inventory.
"""

import argparse
from contextlib import contextmanager
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import time
import zipfile
import xml.etree.ElementTree as ET


GRAPH_CLSID = "00020803-0000-0000-C000-000000000046"
REVISION = 1
P = "http://schemas.openxmlformats.org/presentationml/2006/main"
A = "http://schemas.openxmlformats.org/drawingml/2006/main"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
REL = "http://schemas.openxmlformats.org/package/2006/relationships"
CT = "http://schemas.openxmlformats.org/package/2006/content-types"


class WorkerFailed(RuntimeError):
    """A native worker exited or exceeded its deadline."""


class GraphJobBlocked(RuntimeError):
    """A failed object's receipt is durable and the next object can proceed."""


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def write_json(path, value):
    temporary = path.with_suffix(".pending")
    temporary.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    for attempt in range(50):
        try:
            temporary.replace(path)
            return
        except PermissionError:
            if attempt == 49:
                raise
            time.sleep(0.1)


def is_graph(data):
    import io
    import olefile

    if not data.startswith(bytes.fromhex("d0cf11e0a1b11ae1")):
        return False
    with olefile.OleFileIO(io.BytesIO(data)) as storage:
        return storage.root.clsid.upper() == GRAPH_CLSID


def create_carrier(template, target, payload):
    """Embed exactly one untouched Graph object in a generated blank slide."""
    with zipfile.ZipFile(template) as archive:
        entries = {name: archive.read(name) for name in archive.namelist()}
    slide = ET.fromstring(entries["ppt/slides/slide1.xml"])
    tree = slide.find(f"{{{P}}}cSld/{{{P}}}spTree")
    if tree is None:
        raise ValueError("Blank template has no shape tree")
    frame = ET.fromstring(f'''<p:graphicFrame xmlns:p="{P}" xmlns:a="{A}" xmlns:r="{R}">
      <p:nvGraphicFramePr><p:cNvPr id="2" name="Recovered Graph"/>
      <p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
      <p:xfrm><a:off x="0" y="0"/><a:ext cx="6096000" cy="4572000"/></p:xfrm>
      <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/presentationml/2006/ole">
      <p:oleObj name="Chart" r:id="rIdGraph" progId="MSGraph.Chart.8"
        imgW="6096000" imgH="4572000"><p:embed/>
        <p:pic><p:nvPicPr><p:cNvPr id="3" name="Graph preview"/>
        <p:cNvPicPr/><p:nvPr/></p:nvPicPr>
        <p:blipFill><a:blip r:embed="rIdPreview"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>
        <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="6096000" cy="4572000"/></a:xfrm>
        <a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>
      </p:oleObj>
      </a:graphicData></a:graphic></p:graphicFrame>''')
    tree.append(frame)
    entries["ppt/slides/slide1.xml"] = ET.tostring(slide, encoding="utf-8", xml_declaration=True)
    rel_path = "ppt/slides/_rels/slide1.xml.rels"
    relations = ET.fromstring(entries[rel_path])
    ET.SubElement(relations, f"{{{REL}}}Relationship", {
        "Id": "rIdGraph", "Type": f"{R}/oleObject", "Target": "../embeddings/graph.bin",
    })
    ET.SubElement(relations, f"{{{REL}}}Relationship", {
        "Id": "rIdPreview", "Type": f"{R}/image", "Target": "../media/preview.png",
    })
    entries[rel_path] = ET.tostring(relations, encoding="utf-8", xml_declaration=True)
    content_types = ET.fromstring(entries["[Content_Types].xml"])
    ET.SubElement(content_types, f"{{{CT}}}Override", {
        "PartName": "/ppt/embeddings/graph.bin",
        "ContentType": "application/vnd.openxmlformats-officedocument.oleObject",
    })
    ET.SubElement(content_types, f"{{{CT}}}Override", {
        "PartName": "/ppt/media/preview.png", "ContentType": "image/png",
    })
    entries["[Content_Types].xml"] = ET.tostring(content_types, encoding="utf-8", xml_declaration=True)
    entries["ppt/embeddings/graph.bin"] = payload
    import base64
    entries["ppt/media/preview.png"] = base64.b64decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a0XcAAAAASUVORK5CYII=")
    with zipfile.ZipFile(target, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, data in entries.items():
            archive.writestr(name, data)


def extracted_workbook(presentation):
    """Reject ambiguous outputs instead of assigning another chart's data."""
    import io

    with zipfile.ZipFile(presentation) as archive:
        names = [name for name in archive.namelist()
                 if name.startswith("ppt/embeddings/") and name.lower().endswith(".xlsx")]
        if len(names) != 1:
            raise ValueError(f"Expected one generated workbook, found {len(names)}")
        data = archive.read(names[0])
    with zipfile.ZipFile(io.BytesIO(data)) as workbook:
        if "xl/workbook.xml" not in workbook.namelist():
            raise ValueError("Generated object is not an XLSX workbook")
        if workbook.testzip() is not None:
            raise ValueError("Generated XLSX failed CRC validation")
    return data


def powerpoint_processes():
    import psutil

    return [process for process in psutil.process_iter(["name", "create_time"])
            if (process.info["name"] or "").lower() == "powerpnt.exe"]


def com_call(operation):
    """Office briefly rejects calls while its chart converter is committing."""
    import pythoncom
    import pywintypes

    deadline = time.monotonic() + 10
    while True:
        try:
            return operation()
        except pywintypes.com_error as error:
            if error.hresult not in (-2147418111, -2147417846) or time.monotonic() >= deadline:
                raise
            pythoncom.PumpWaitingMessages()
            time.sleep(0.1)


def worker(root):
    import pythoncom
    import psutil
    import win32com.client
    import win32con
    import win32gui
    import win32process
    import pywintypes

    # PowerPoint can reuse an existing application even with DispatchEx. Refuse
    # to share it, so a timeout can never close a user's presentation.
    if powerpoint_processes():
        raise RuntimeError("Close PowerPoint before starting Graph recovery")
    pythoncom.CoInitialize()
    app = win32com.client.DispatchEx("PowerPoint.Application")
    processes = powerpoint_processes()
    if len(processes) != 1:
        raise RuntimeError("Cannot identify the isolated PowerPoint process")
    process = processes[0]
    pid = process.pid
    write_json(root / "owner.json", {"pid": pid, "created": process.create_time()})
    app.AutomationSecurity = 3
    app.DisplayAlerts = 2
    template = root / "blank.pptx"
    presentation = None
    dialog_log = (root / "dialog.log").open("w")
    monitor = subprocess.Popen([
        "powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
        str(Path(__file__).with_name("graph-conversion-dialog.ps1")),
        "-PowerPointPid", str(pid), "-Session", str(root),
    ], stdout=dialog_log, stderr=subprocess.STDOUT)
    try:
        presentation = app.Presentations.Add(0)
        presentation.Slides.Add(1, 12)
        presentation.SaveAs(str(template), 24)
        presentation.Close()
        presentation = None
        write_json(root / "ready.json", {"powerpoint_version": app.Version})
        while not (root / "stop").exists():
            request_path = root / "request.json"
            if not request_path.exists():
                time.sleep(0.1)
                continue
            if app.Presentations.Count:
                write_json(root / "relinquished.json", {"reason": "Another presentation was opened"})
                raise RuntimeError("PowerPoint isolation lost; leaving the other presentation open")
            request = json.loads(request_path.read_text())
            request_path.unlink()
            directory = Path(request["directory"])
            response = {"success": False}
            stage = "create carrier"
            try:
                create_carrier(template, directory / "carrier.pptx", (directory / "source.bin").read_bytes())
                stage = "open carrier"
                presentation = com_call(lambda: app.Presentations.Open(str(directory / "carrier.pptx"), 0, 0, -1))
                stage = "activate Graph"
                shape = presentation.Slides(1).Shapes(1)
                shape.Select()
                (root / "unsupported-chart").unlink(missing_ok=True)
                (root / "convert-active").touch()
                window = presentation.Windows(1)
                window.Activate()
                windows = []
                win32gui.EnumWindows(lambda handle, _: windows.append(handle)
                                     if win32process.GetWindowThreadProcessId(handle)[1] == pid
                                     and win32gui.IsWindowVisible(handle) else None, None)
                main_windows = [handle for handle in windows
                                if "carrier.pptx" in win32gui.GetWindowText(handle)]
                if len(main_windows) != 1:
                    raise RuntimeError("Cannot identify the carrier window")
                win32gui.SetForegroundWindow(main_windows[0])
                for pane in window.Panes:
                    if pane.ViewType == 1:
                        pane.Activate()
                point = (int(window.PointsToScreenPixelsX(shape.Left + shape.Width / 2)),
                         int(window.PointsToScreenPixelsY(shape.Top + shape.Height / 2)))
                handle = win32gui.WindowFromPoint(point)
                if win32process.GetWindowThreadProcessId(handle)[1] != pid:
                    raise RuntimeError("Graph window is obscured; no input sent")
                x, y = win32gui.ScreenToClient(handle, point)
                position = (y << 16) | (x & 0xFFFF)
                win32gui.PostMessage(handle, win32con.WM_LBUTTONDOWN, win32con.MK_LBUTTON, position)
                win32gui.PostMessage(handle, win32con.WM_LBUTTONUP, 0, position)
                win32gui.PostMessage(handle, win32con.WM_LBUTTONDBLCLK, win32con.MK_LBUTTON, position)
                win32gui.PostMessage(handle, win32con.WM_LBUTTONUP, 0, position)
                deadline = time.monotonic() + 15
                while time.monotonic() < deadline:
                    if (root / "unsupported-chart").exists():
                        raise ValueError("PowerPoint cannot convert this chart format")
                    if monitor.poll() is not None:
                        raise RuntimeError("Graph conversion dialog monitor exited")
                    try:
                        if presentation.Slides(1).Shapes(1).HasChart:
                            break
                    except pywintypes.com_error as error:
                        if error.hresult not in (-2147418111, -2147417846):
                            raise
                    pythoncom.PumpWaitingMessages()
                    time.sleep(0.1)
                (root / "convert-active").unlink(missing_ok=True)
                if not presentation.Slides(1).Shapes(1).HasChart:
                    raise RuntimeError("PowerPoint did not create a modern chart")
                stage = "save converted chart"
                com_call(lambda: presentation.SaveAs(str(directory / "converted.pptx"), 24))
                payload = extracted_workbook(directory / "converted.pptx")
                (directory / "data.xlsx.pending").write_bytes(payload)
                (directory / "data.xlsx.pending").replace(directory / "data.xlsx")
                response = {"success": True, "xlsx_sha256": sha256(payload), "bytes": len(payload)}
            except Exception as error:
                response["error"] = f"{stage}: {error}"
                print(response, flush=True)
            finally:
                (root / "convert-active").unlink(missing_ok=True)
                if presentation is not None:
                    com_call(lambda: setattr(presentation, "Saved", -1))
                    com_call(lambda: presentation.Close())
                    presentation = None
            write_json(directory / "response.json", response)
    finally:
        (root / "stop").touch()
        if presentation is not None:
            presentation.Close()
        if not app.Presentations.Count:
            app.Quit()
        else:
            write_json(root / "relinquished.json", {"reason": "Another presentation remains open"})
        try:
            monitor.wait(timeout=5)
        except subprocess.TimeoutExpired:
            monitor.kill()
            monitor.wait()
        dialog_log.close()
        pythoncom.CoUninitialize()


def check_resources(root, reserve_gib):
    import psutil
    import ctypes
    from ctypes import wintypes

    if (root / "stop.request").exists():
        raise RuntimeError("Graph recovery stop requested")

    class MemoryStatus(ctypes.Structure):
        _fields_ = [("length", wintypes.DWORD), ("load", wintypes.DWORD)] + [
            (name, ctypes.c_ulonglong) for name in (
                "total_physical", "available_physical", "total_pagefile", "available_pagefile",
                "total_virtual", "available_virtual", "available_extended")]

    status = MemoryStatus()
    status.length = ctypes.sizeof(status)
    if not ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)):
        raise OSError("Windows memory query failed")
    reserve = reserve_gib * 1024 ** 3
    if min(status.available_physical, status.available_pagefile) < reserve:
        raise RuntimeError("RAM or commit reserve reached; recovery stopped")
    if psutil.disk_usage(str(root)).free < 8 * 1024 ** 3:
        raise RuntimeError("Output drive has less than 8 GiB free")


def wait_for(path, process, root, timeout, reserve_gib):
    deadline = time.monotonic() + timeout
    while True:
        check_resources(root, reserve_gib)
        if path.exists():
            try:
                return json.loads(path.read_text())
            except PermissionError:
                # A scanner can briefly hold the atomically published response.
                # Keep the same deadline and memory guard while waiting for it.
                pass
        if process.poll() is not None:
            raise WorkerFailed(f"Graph worker exited with {process.returncode}")
        if time.monotonic() >= deadline:
            raise WorkerFailed(f"Graph recovery exceeded {timeout} seconds")
        time.sleep(0.2)


def stop_worker(process, session):
    import psutil

    (session / "stop").touch()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
    owner_path = session / "owner.json"
    if owner_path.exists() and not (session / "relinquished.json").exists():
        owner = json.loads(owner_path.read_text())
        try:
            powerpoint = psutil.Process(owner["pid"])
            if (powerpoint.create_time() == owner["created"]
                    and powerpoint.name().lower() == "powerpnt.exe"):
                powerpoint.terminate()
        except psutil.NoSuchProcess:
            pass


def recover(args):
    import msvcrt

    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=True)
    # An OS-owned lock is released after crashes, unlike a stale lock filename.
    with (root / "recovery.lock").open("a+b") as lock:
        lock.seek(0)
        if not lock.read(1):
            lock.write(b"0")
            lock.flush()
        lock.seek(0)
        msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
        run_locked(args, root)


@contextmanager
def candidate_payloads(args):
    if args.candidates:
        with zipfile.ZipFile(args.candidates) as archive:
            yield ((entry.filename, archive.read(entry)) for entry in archive.infolist()
                   if not entry.is_dir())
        return
    workspace = args.extraction_workspace.resolve()
    with (workspace / "reports/candidates.jsonl").open(encoding="utf-8") as manifest:
        def records():
            for line in manifest:
                row = json.loads(line)
                path = (workspace / row["path"]).resolve()
                if not path.is_relative_to(workspace):
                    raise ValueError("Candidate path leaves extraction workspace")
                data = path.read_bytes()
                if sha256(data) != row["sha256"] or len(data) != row["bytes"]:
                    raise ValueError(f"Candidate checksum or size changed: {path}")
                yield row["path"], data
        yield records()


def run_locked(args, root):
    import uuid

    check_resources(root, args.reserve_gib)
    session = root / ("session-" + uuid.uuid4().hex)
    session.mkdir()
    process = None
    count = 0
    scanned = 0
    graphs = 0
    source = str((args.candidates or args.extraction_workspace).resolve())
    write_json(session / "recipe.json", {
        "revision": REVISION, "python": sys.version,
        "files": {path.name: sha256(path.read_bytes()) for path in (
            Path(__file__), Path(__file__).with_name("graph-conversion-dialog.ps1"))},
    })
    with candidate_payloads(args) as candidates, (session / "worker.log").open("w") as log:
        try:
            for member, payload in candidates:
                scanned += 1
                check_resources(root, args.reserve_gib)
                if not is_graph(payload):
                    continue
                graphs += 1
                identity = sha256(payload)
                directory = root / identity
                directory.mkdir(exist_ok=True)
                receipt_path = directory / "receipt.json"
                if receipt_path.exists():
                    receipt = json.loads(receipt_path.read_text())
                    if receipt.get("success"):
                        if sha256((directory / "data.xlsx").read_bytes()) != receipt["xlsx_sha256"]:
                            raise ValueError(f"Recovered XLSX checksum changed: {identity}")
                    elif not args.retry_failed:
                        continue
                    else:
                        receipt = None
                    if receipt is not None:
                        continue
                if args.limit and count >= args.limit:
                    break
                count += 1
                if receipt_path.exists():
                    history = directory / "attempts"
                    history.mkdir(exist_ok=True)
                    (history / (session.name + ".json")).write_bytes(receipt_path.read_bytes())
                if process is None:
                    process = subprocess.Popen([sys.executable, str(Path(__file__).resolve()),
                                                "--worker", str(session)], stdout=log, stderr=subprocess.STDOUT)
                    wait_for(session / "ready.json", process, root, 60, args.reserve_gib)
                (directory / "source.bin").write_bytes(payload)
                (directory / "response.json").unlink(missing_ok=True)
                write_json(session / "request.json", {"directory": str(directory)})
                started = time.monotonic()
                try:
                    response = wait_for(directory / "response.json", process, root, args.timeout, args.reserve_gib)
                except WorkerFailed as error:
                    write_json(directory / "receipt.json", {
                        "success": False, "source_sha256": identity, "source_member": member,
                        "error": str(error), "session": session.name,
                    })
                    raise GraphJobBlocked(str(error)) from error
                receipt = dict(response, revision=REVISION, source_sha256=identity,
                               source_member=member, source_archive=source,
                               producer="PowerPoint Graph conversion", derived_data_only=True,
                               environment=json.loads((session / "ready.json").read_text()),
                               session=session.name,
                               elapsed_seconds=round(time.monotonic() - started, 3),
                               output=str(directory / "data.xlsx"))
                write_json(receipt_path, receipt)
                write_json(root / "status.json", {
                    "state": "running", "attempts_this_session": count,
                    "latest_source_sha256": identity, "latest_success": response["success"],
                })
                print(json.dumps(receipt), flush=True)
                # Keep sources and receipts. Carrier files are reproducible and
                # cannot be used to claim visual equivalence to the old chart.
                for name in ("carrier.pptx", "converted.pptx"):
                    for attempt in range(10):
                        try:
                            (directory / name).unlink(missing_ok=True)
                            break
                        except PermissionError:
                            if attempt == 9:
                                print(f"Retained locked temporary file: {directory / name}", flush=True)
                            time.sleep(0.1)
        finally:
            if process is not None:
                stop_worker(process, session)
            all_receipts = []
            failures = []
            for path in sorted(root.glob("*/receipt.json")):
                receipt = json.loads(path.read_text())
                if receipt.get("success"):
                    output = path.parent / "data.xlsx"
                    if sha256(output.read_bytes()) != receipt["xlsx_sha256"]:
                        raise ValueError(f"Recovered XLSX checksum changed: {output}")
                    all_receipts.append(receipt)
                else:
                    failures.append(receipt)
            write_json(root / "derived-workbooks.json", {
                "version": REVISION, "kind": "microsoft-graph-derived-data",
                "source_archive": source, "workbooks": all_receipts, "failures": failures,
            })
            write_json(root / "status.json", {
                "state": "stopped" if sys.exc_info()[0] else "finished",
                "successful_total": len(all_receipts), "attempts_this_session": count,
                "failed_total": len(failures), "scanned_candidates": scanned, "graph_objects_seen": graphs,
                "limit": args.limit,
            })


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sources = parser.add_mutually_exclusive_group()
    sources.add_argument("--candidates", type=Path, help="ZIP emitted by the recursive extraction/import")
    sources.add_argument("--extraction-workspace", type=Path,
                         help="Workspace containing reports/candidates.jsonl")
    parser.add_argument("--pipeline", type=Path,
                        help="Run this preserved prepare_enron.py directory before Graph recovery")
    parser.add_argument("--source-url", action="append")
    parser.add_argument("--seven-zip", type=Path)
    parser.add_argument("--output", type=Path, help="Separate resumable Graph recovery directory")
    parser.add_argument("--limit", type=int, default=0, help="Maximum new attempts, 0 for all")
    parser.add_argument("--timeout", type=int, default=90)
    parser.add_argument("--reserve-gib", type=float, default=8)
    parser.add_argument("--retry-failed", action="store_true")
    parser.add_argument("--worker", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.worker:
        worker(args.worker)
    else:
        if (args.candidates is None and args.extraction_workspace is None) or args.output is None:
            parser.error("An input source and --output are required")
        if args.limit < 0 or args.timeout <= 0 or args.reserve_gib <= 0:
            parser.error("Invalid limit, timeout or memory reserve")
        if args.pipeline:
            if not args.extraction_workspace or not args.source_url or not args.seven_zip:
                parser.error("--pipeline requires --extraction-workspace, --source-url and --seven-zip")
            sys.path.insert(0, str(args.pipeline.resolve()))
            from prepare_enron import Preparation, prepare_corpus
            request = Preparation(tuple(args.source_url), args.extraction_workspace, args.seven_zip)
            prepare_corpus(request)
        restarts_without_progress = 0
        previous_successes = -1
        while True:
            try:
                recover(args)
                break
            except GraphJobBlocked as error:
                if args.limit:
                    raise
                status = json.loads((args.output / "status.json").read_text())
                successes = status["successful_total"]
                restarts_without_progress = restarts_without_progress + 1 if successes == previous_successes else 0
                if restarts_without_progress >= 3:
                    raise RuntimeError("Three worker restarts made no progress; inspect the session logs") from error
                previous_successes = successes
                # Retry is explicit for old failures. Newly blocked objects have
                # already received a receipt and must not trap the batch in a loop.
                args.retry_failed = False
                print(f"Worker restarted after blocked object: {error}", flush=True)


if __name__ == "__main__":
    main()
