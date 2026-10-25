"""Opt-in native startup check using copied resident client specifications.

Each --cases child directory contains client JSON files with source_path,
workbook, client_id and sheets, as in a resident session's clients directory.
Use durable private copies of the input files. No corpus results are changed.
"""
import argparse
import ctypes
import json
import os
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[3]


def read(path):
    try:
        return json.loads(path.read_text(encoding="utf-8-sig"))
    except (OSError, ValueError):
        return {}


def publish(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    pending = path.with_suffix(".pending")
    pending.write_text(json.dumps(value), encoding="utf-8")
    os.replace(pending, path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true")
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--groups", type=int, choices=(2, 4), default=4)
    parser.add_argument("--reset-visible", action="store_true", help="Explicitly show one restarted pane before any capture jobs.")
    args = parser.parse_args()
    if not args.run or os.name != "nt":
        parser.error("Pass --run on Windows with Excel installed.")
    cases = sorted(path for path in args.cases.resolve().iterdir() if path.is_dir())
    if not cases or any(not list(path.glob("*.json")) for path in cases):
        parser.error("Each case directory must contain resident client JSON files.")
    root = ROOT / ".tmp/concurrent-startup-check" / str(time.time_ns())
    root.mkdir(parents=True)
    control = root / "control"
    publish(control / "control.json", {"paused": False, "hide_excel": True})
    sessions, logs, foreground = [], [], set()
    receipt = {"groups": args.groups, "reset_visible": args.reset_visible}
    user32 = ctypes.windll.user32
    user32.GetForegroundWindow.restype = ctypes.c_void_p
    user32.GetWindowThreadProcessId.argtypes = (ctypes.c_void_p, ctypes.POINTER(ctypes.c_ulong))

    def wait_until(description, check, timeout=90):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            pid = ctypes.c_ulong()
            user32.GetWindowThreadProcessId(user32.GetForegroundWindow(), ctypes.byref(pid))
            foreground.add(pid.value)
            if check():
                return
            if any(process.poll() is not None for _, process, _ in sessions):
                raise RuntimeError(f"Launcher exited during {description}")
            time.sleep(.25)
        raise TimeoutError(description)

    try:
        # Deliberately launch every group before waiting for the first one.
        # Sequential group startup masked missing add-ins in the old smoke test.
        for index in range(args.groups):
            name = f"group-{index}"
            session = root / name
            specs = [read(path) for path in cases[index % len(cases)].glob("*.json")]
            for spec in specs:
                publish(session / "clients" / (spec["client_id"] + ".json"), spec)
            output = (root / f"{name}.stdout.log").open("w")
            error = (root / f"{name}.stderr.log").open("w")
            logs.extend((output, error))
            environment = {**os.environ, "EXCEL_SHRINK_CONTROL_ROOM_DIR": str(control),
                           "EXCEL_SHRINK_CONTROL_ROOM_GROUP_ID": name,
                           "EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID": f"test-{name}"}
            process = subprocess.Popen([
                "powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                str(ROOT / "excel-shrink-cli/office-capture/resident.ps1"),
                "-SessionPath", str(session), "-StartupTimeoutSeconds", "60", "-HideExcel"],
                env=environment, stdout=output, stderr=error, creationflags=0x08000000)
            sessions.append((session, process, specs))
        wait_until("primary add-in readiness", lambda: all(read(session / "metrics.json").get("primary_clients_ready") for session, _, _ in sessions))
        print("All concurrent groups are ready.", flush=True)
        if args.reset_visible:
            session, _, specs = sessions[0]
            client_id = specs[0]["client_id"]
            current = read(session / "metrics.json")["clients"][client_id]
            path = control / "groups" / session.name / "client-resets/visible.json"
            publish(path, dict(id="visible", status="pending", client_id=client_id,
                              group_id=session.name, run_id="test", attempt_id=f"test-{session.name}",
                              expected_pid=current["pid"], expected_started_unix_ms=current["process_started_unix_ms"],
                              hide_excel=False, reason="Check private-to-interactive desktop reset"))

            def reset_complete():
                result = read(path)
                if result.get("status") in ("failed", "rejected", "group_restart"):
                    raise RuntimeError(result)
                return result.get("status") == "complete"

            wait_until("visible reset", reset_complete)
            receipt["visible_reset"] = read(path)
            print("Visible reset is ready; hidden peers stayed alive.", flush=True)
        for session, _, specs in sessions:
            for spec in specs:
                publish(session / "commands" / spec["client_id"] / "0001.json",
                        dict(id="0001", kind="capture", sheets=spec["sheets"][:1], tile_megapixels=4))

        def captures_complete():
            responses = [read(session / "responses" / spec["client_id"] / "0001.json")
                         for session, _, specs in sessions for spec in specs]
            failures = [response for response in responses if response.get("ok") is False]
            if failures:
                raise RuntimeError(failures)
            return all(response.get("ok") is True for response in responses)

        wait_until("first-sheet captures", captures_complete)
        receipt["passed"] = True
    except Exception as error:
        receipt["error"] = repr(error)
        print(receipt["error"], flush=True)
    finally:
        receipt["before_cleanup"] = {session.name: read(session / "metrics.json") for session, _, _ in sessions}
        hidden_pids = {client.get("pid") for metrics in receipt["before_cleanup"].values()
                       for client in metrics.get("clients", {}).values() if client.get("hide_excel")}
        receipt["foreground_pids"] = sorted(foreground)
        receipt["hidden_excel_foreground_pids"] = sorted(hidden_pids & foreground)
        if receipt["hidden_excel_foreground_pids"]:
            receipt["passed"] = False
            receipt["error"] = "A hidden Excel process took foreground focus."
        for session, _, _ in sessions:
            publish(session / "finish.json", {"complete": True})
        for _, process, _ in sessions:
            try:
                process.wait(timeout=30)
            except subprocess.TimeoutExpired:
                process.terminate()  # Only this test's launcher and its owned Job.
                process.wait(timeout=10)
        for log in logs:
            log.close()
        publish(root / "receipt.json", receipt)
        print("Receipt:", root / "receipt.json", flush=True)
    return 0 if receipt.get("passed") else 1


if __name__ == "__main__":
    raise SystemExit(main())
