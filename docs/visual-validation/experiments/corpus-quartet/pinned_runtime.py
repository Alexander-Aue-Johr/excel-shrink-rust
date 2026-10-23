"""Load the Office capture runtime from main without sibling experiments."""

import io
from pathlib import Path
import posixpath
import re
import subprocess
import sys
import tarfile
import xml.etree.ElementTree as ET
from zipfile import ZipFile


RUNTIME_REF = "2fccb62c11dbbc97bcf9736dff07cb4a188dd237"


def windows_path(path):
    path = str(Path(path).resolve())
    return path if sys.platform == "win32" else subprocess.check_output(
        ["wslpath", "-w", path], text=True).strip()


def capture_plan(workbook, scopes):
    """Plan explicit visible worksheet selections, without trusting UsedRange."""
    with ZipFile(workbook) as archive:
        book = ET.fromstring(archive.read("xl/workbook.xml"))
        relations = ET.fromstring(archive.read("xl/_rels/workbook.xml.rels"))
        targets = {relation.get("Id"): posixpath.normpath(posixpath.join(
            "xl", relation.get("Target"))) for relation in relations}
        sheets = list(book.find("{*}sheets"))
        result = []
        for scope in scopes:
            name, address = scope.rsplit("!", 1)
            match = re.fullmatch(r"A1:([A-Z]+)([1-9][0-9]*)", address)
            if not match:
                raise ValueError("Scopes must start at A1, e.g. Sheet1!A1:AX1000")
            index = next(i for i, sheet in enumerate(sheets, 1) if sheet.get("name") == name)
            sheet = sheets[index - 1]
            if sheet.get("state", "visible") != "visible":
                raise ValueError("Cannot capture a hidden worksheet")
            relation = next(value for key, value in sheet.attrib.items() if key.endswith("}id"))
            target = targets[relation].lstrip("/")
            node = ET.fromstring(archive.read(target))
            column = 0
            for letter in match[1]:
                column = column * 26 + ord(letter) - 64
            bottom = int(match[2]) - 1
            if not 0 <= bottom < 1048576 or not 1 <= column <= 16384:
                raise ValueError("Scope exceeds Excel bounds")
            right = column - 1
            if any(item["index"] == index for item in result):
                raise ValueError("Use one scope per worksheet")
            result.append({
                "index": index, "name": name, "bottom_row": bottom,
                "base_right_column": right, "capture_right_column": min(16383, right + 32),
                "horizontal_overflow_required": right < 16383,
                "drawing_fallback_required": node.find("{*}drawing") is not None,
            })
    return {"mode": "paired", "workbook": "original.xlsx", "tile_megapixels": 32,
            "sheets": result}


def export_runtime(root, revision, output):
    data = subprocess.check_output(
        ["git", "archive", revision, "excel-shrink-cli/office-capture"], cwd=root)
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        archive.extractall(output, filter="data")
    runtime = output / "excel-shrink-cli/office-capture"
    processes = runtime / "CaptureProcesses.cs"
    source = processes.read_text(encoding="utf-8")
    anchor = "    public void Dispose() { job.Dispose(); }"
    if source.count(anchor) != 1:
        raise ValueError("Pinned capture process metrics anchor changed")
    source = source.replace(anchor, """
    public ulong PeakBytes
    {
        get
        {
            var info = new ExtendedLimits();
            if (!QueryInformationJobObject(job, 9, ref info,
                (uint)Marshal.SizeOf(typeof(ExtendedLimits)), IntPtr.Zero)) throw new Win32Exception();
            return info.PeakJobMemory.ToUInt64();
        }
    }
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool QueryInformationJobObject(SafeFileHandle job, int kind,
        ref ExtendedLimits info, uint size, IntPtr returned);
    [StructLayout(LayoutKind.Sequential)]
    struct Accounting
    {
        public long User, Kernel, PeriodUser, PeriodKernel;
        public uint Faults, Processes, Active, Terminated;
    }
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool QueryInformationJobObject(SafeFileHandle job, int kind,
        ref Accounting info, uint size, IntPtr returned);
    public double CpuSeconds
    {
        get
        {
            var info = new Accounting();
            if (!QueryInformationJobObject(job, 1, ref info,
                (uint)Marshal.SizeOf(typeof(Accounting)), IntPtr.Zero)) throw new Win32Exception();
            return (info.User + info.Kernel) / 10000000.0;
        }
    }
    public void Dispose() { job.Dispose(); }
""", 1)
    processes.write_text(source, encoding="utf-8")
    return runtime
