"""Exercise the native diagnostic host and migration without opening Excel."""
import argparse
import contextlib
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid


def request(base, payload, token="fixture-token"):
    message = urllib.request.Request(
        base + "/launcher/diagnostics",
        data=json.dumps(payload).encode(),
        headers={"Content-Type": "application/json", "X-Launcher-Token": token},
    )
    with urllib.request.urlopen(message, timeout=10) as response:
        return json.load(response)


def start_host(binary, settings, base):
    environment = dict(os.environ, EXCEL_SHRINK_SQLITE_METADATA="1")
    environment.pop("EXCEL_SHRINK_CONTROL_ROOM_DIR", None)
    environment.pop("EXCEL_SHRINK_CONTROL_ROOM_GROUP_ID", None)
    process = subprocess.Popen(
        [str(binary), "capture-host", "resident", str(settings)],
        env=environment,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0,
    )
    try:
        for _ in range(100):
            if process.poll() is not None:
                raise RuntimeError("Diagnostic host exited before readiness")
            try:
                with urllib.request.urlopen(base + "/health", timeout=1) as response:
                    assert json.load(response)["ok"]
                return process
            except (OSError, urllib.error.URLError):
                time.sleep(0.1)
        raise TimeoutError("Diagnostic host readiness timed out")
    except BaseException:
        stop_host(process)
        raise


def stop_host(process):
    if process.poll() is None:
        process.terminate()
    process.wait(timeout=10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    with tempfile.TemporaryDirectory(prefix="sqlite-diagnostics-") as temporary:
        root = Path(temporary)
        session = root / ("session-ä-" + "x" * 110)
        if os.name == "nt":
            session = Path("\\\\?\\" + str(session))
        (session / "clients").mkdir(parents=True)
        legacy = b'{"event":"legacy"}\n'
        (session / "launcher-events.jsonl").write_bytes(legacy)
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            port = probe.getsockname()[1]
        base = f"http://127.0.0.1:{port}"
        settings = root / "host.json"
        settings.write_text(json.dumps({
            "session": str(session),
            "web": str(Path(__file__).resolve().parents[1] / "web"),
            "port": port,
            "launcher_token": "fixture-token",
        }))
        events = [{"id": str(uuid.uuid4()), "event": "progress", "index": n} for n in range(100)]
        payload = {"events": events, "metrics": {"status": "running", "sequence": 100}}
        process = start_host(binary, settings, base)
        try:
            try:
                request(base, payload, token="wrong-token")
                raise AssertionError("Unauthenticated diagnostic write was accepted")
            except urllib.error.HTTPError as error:
                assert error.code in (400, 401, 403)
            assert request(base, payload)["accepted_events"] == 100
            assert request(base, payload)["accepted_events"] == 100
            if os.name == "nt":
                client = subprocess.run([
                    "powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                    str(Path(__file__).with_name("sqlite-diagnostics-client.tests.ps1")),
                    "-Session", str(session), "-BaseUrl", base,
                ], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, creationflags=subprocess.CREATE_NO_WINDOW)
                if client.returncode:
                    raise RuntimeError(client.stdout.decode("utf-8", errors="replace"))
        finally:
            stop_host(process)
        # Termination skips normal teardown. WAL recovery must retain the acked batch.
        process = start_host(binary, settings, base)
        try:
            assert request(base, payload)["ok"]
            assert request(base, {"events": [], "export": True})["ok"]
            assert request(base, {"events": [], "export": True})["ok"]
        finally:
            stop_host(process)
        database = session / "telemetry.sqlite"
        with contextlib.closing(sqlite3.connect(database)) as connection:
            assert connection.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
            expected = 103 if os.name == "nt" else 100
            assert connection.execute("SELECT count(*) FROM events").fetchone()[0] == expected
        history = (session / "launcher-events.jsonl").read_bytes()
        assert history.startswith(legacy)
        assert len(history.splitlines()) == expected + 1
        snapshot = subprocess.check_output([
            str(binary), "metadata-store", "read", str(session / "metrics.json")
        ])
        assert json.loads(snapshot)["sequence"] == 100
        source = root / "legacy-source"
        source.mkdir()
        original = b'{ "unknown": true, "status": "differences" }\n'
        (source / "record.json").write_bytes(original)
        migrated = root / "import.sqlite"
        subprocess.run([str(binary), "metadata-store", "import", str(source), str(migrated)], check=True)
        subprocess.run([str(binary), "metadata-store", "verify", str(source), str(migrated)], check=True)
        exported = root / "export"
        subprocess.run([str(binary), "metadata-store", "export", str(migrated), str(exported)], check=True)
        assert (exported / "record.json").read_bytes() == original
        assert (source / "record.json").read_bytes() == original
        print("SQLite diagnostics, crash recovery, event deduplication and byte-exact migration passed.")


if __name__ == "__main__":
    main()
