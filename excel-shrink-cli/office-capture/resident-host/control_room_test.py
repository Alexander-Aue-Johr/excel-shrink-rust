"""Loopback control API checks; pass --host path/to/ResidentCaptureHost.dll."""
import argparse
import concurrent.futures
import ctypes
import http.client
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import unittest
import urllib.error
import urllib.request


HERE = Path(__file__).resolve().parent
parser = argparse.ArgumentParser(add_help=False)
parser.add_argument("--host", type=Path, default=HERE / "bin/Debug/net8.0/ResidentCaptureHost.dll")
OPTIONS, REMAINING = parser.parse_known_args()


class ControlRoomProtocol(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="control-room-", dir=HERE / "obj")
        self.directory = Path(self.temp.name)
        self.data = self.directory / "data"
        self.web = self.directory / "web"
        self.web.mkdir()
        (self.web / "index.html").write_text("<title>Control room test</title>", encoding="utf-8")
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            self.port = sock.getsockname()[1]
        settings = self.directory / "settings.json"
        settings.write_text(json.dumps({
            "root": str(self.data), "web_root": str(self.web), "port": self.port,
            "host_version": "control-room-test-build",
            "initial_controls": {"target_groups": 2, "open_timeout_seconds": 900},
        }), encoding="utf-8")
        self.log = (self.directory / "server.log").open("wb")
        self.process = subprocess.Popen(
            ["dotnet", str(OPTIONS.host.resolve()), "--control-room", str(settings)],
            stdout=self.log, stderr=self.log,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
        self.addCleanup(self.cleanup_process)
        self.url = f"http://127.0.0.1:{self.port}"
        for _ in range(100):
            try:
                self.request("/health")
                break
            except urllib.error.URLError:
                if self.process.poll() is not None:
                    self.fail("Control server exited: " + (self.directory / "server.log").read_text())
                time.sleep(.05)
        else:
            self.fail("Control server did not start")

    def cleanup_process(self):
        if self.process.poll() is None:
            self.process.terminate()
        self.process.wait(timeout=10)
        self.log.close()
        self.temp.cleanup()

    def request(self, path, body=None, headers=None, raw=False):
        headers = dict(headers or {})
        if isinstance(body, dict):
            body = json.dumps(body).encode()
            headers["Content-Type"] = "application/json"
        with urllib.request.urlopen(urllib.request.Request(self.url + path, data=body, headers=headers), timeout=5) as response:
            data = response.read()
            return data if raw else json.loads(data)

    def write(self, path, data):
        target = self.data / path
        target.parent.mkdir(parents=True, exist_ok=True)
        temporary = target.with_suffix(".pending")
        temporary.write_text(json.dumps(data), encoding="utf-8")
        temporary.replace(target)

    def assert_status(self, code, path, body=None, headers=None):
        with self.assertRaises(urllib.error.HTTPError) as raised:
            self.request(path, body, headers)
        self.assertEqual(raised.exception.code, code)

    def test_live_summary_and_detail_preserve_completed_history(self):
        self.assertEqual(self.request("/health")["host_version"], "control-room-test-build")
        self.write("queue.json", {"run_id": "test-run", "machine": "test-machine", "groups": [
            {"id": "current", "workbook": "current.xlsx", "corpus": "Data.gov", "status": "opening"},
            {"id": "finished", "workbook": "finished.xlsx", "status": "complete"},
        ]})
        self.write("groups/current/state.json", {"id": "current", "status": "capturing", "sheets": [
            {"name": "Sheet1", "tiles": [{"id": "s0t0", "address": "A1:F20", "status": "equal"}]},
        ]})
        state = self.request("/api/state")
        self.assertEqual(state["run_id"], "test-run")
        self.assertEqual(len(state["groups"]), 2)
        self.assertTrue(state["server"]["live"])
        self.assertEqual(state["controls"]["target_groups"], 2)
        self.assertEqual(state["controls"]["open_timeout_seconds"], 900)
        detail = self.request("/api/groups/current")
        self.assertEqual(detail["workbook"], "current.xlsx")
        self.assertEqual(detail["status"], "capturing")
        self.assertEqual(detail["sheets"][0]["tiles"][0]["address"], "A1:F20")
        self.assertTrue(detail["available"])
        self.assertFalse(self.request("/api/groups/finished")["available"])
        self.write("groups/finished/state.json", {"id": "finished", "status": "capturing"})
        finished = self.request("/api/groups/finished")
        self.assertEqual(finished["status"], "complete")
        self.assertEqual(finished["capture_status"], "capturing")
        self.assert_status(404, "/api/groups/unknown")

    def test_controls_have_no_eight_group_cap_and_persist_atomically(self):
        result = self.request("/api/control", {"target_groups": 128, "paused": True, "equal_preview_width": 0})
        self.assertEqual(result["controls"]["target_groups"], 128)
        self.assertTrue(result["controls"]["paused"])
        stored = json.loads((self.data / "control.json").read_text())
        self.assertEqual(stored, result["controls"])
        self.assertFalse(list(self.data.glob("*.pending")))
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            results = list(pool.map(lambda patch: self.request("/api/control", patch), [
                {"job_timeout_seconds": 1200}, {"open_timeout_seconds": 1800},
            ]))
        self.assertTrue(all(item["ok"] for item in results))
        stored = self.request("/api/state")["controls"]
        self.assertEqual(stored["job_timeout_seconds"], 1200)
        self.assertEqual(stored["open_timeout_seconds"], 1800)
        self.assertEqual(stored["target_groups"], 128)

    def test_invalid_controls_cannot_partially_change_state(self):
        before = (self.data / "control.json").read_bytes()
        for patch in [
            {"target_groups": 0}, {"target_groups": 1.5}, {"paused": "true"},
            {"job_timeout_seconds": -1}, {"equal_preview_width": 513}, {"typo": 2},
            {"target_groups": 24, "equal_preview_width": None},
        ]:
            with self.subTest(patch=patch):
                self.assert_status(400, "/api/control", patch)
                self.assertEqual((self.data / "control.json").read_bytes(), before)
        self.assert_status(415, "/api/control", b"target_groups=24", {"Content-Type": "application/x-www-form-urlencoded"})
        self.assert_status(400, "/api/control", b"{", {"Content-Type": "application/json"})

    def test_local_host_and_origin_protect_control_mutations(self):
        self.assert_status(403, "/api/control", {"paused": True}, {"Origin": "https://example.com"})
        self.assert_status(403, "/api/control", {"paused": True}, {"Origin": "null"})
        self.assert_status(403, "/api/control", {"paused": True}, {"Sec-Fetch-Site": "cross-site"})
        self.assert_status(403, "/api/state", headers={"Host": f"example.com:{self.port}"})
        self.assertTrue(self.request("/api/control", {"paused": True}, {"Origin": self.url})["ok"])

    def test_only_confined_image_assets_are_served(self):
        image = self.data / "groups/one/images/tile.png"
        image.parent.mkdir(parents=True)
        image.write_bytes(b"test-image-bytes")
        self.assertEqual(self.request("/data/groups/one/images/tile.png", raw=True), b"test-image-bytes")
        self.assert_status(404, "/data/control.json")
        self.assert_status(400, "/data/groups%5Cone%5Cimages%5Ctile.png")
        self.assert_status(400, "/data/..%20/outside.png")
        # Send an unnormalized path so the server, not the client, checks traversal.
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=5)
        try:
            connection.request("GET", "/data/%2e%2e/web/index.html")
            response = connection.getresponse()
            self.assertIn(response.status, (400, 404))
            response.read()
        finally:
            connection.close()
        self.assertIn(b"Control room test", self.request("/", raw=True))

    def test_incomplete_snapshot_is_explicit_and_does_not_break_controls(self):
        (self.data / "queue.json").write_text('{"groups":', encoding="utf-8")
        state = self.request("/api/state")
        self.assertEqual(state["groups"], [])
        self.assertEqual(state["diagnostics"][0]["file"], "queue.json")
        self.assertTrue(self.request("/api/control", {"paused": True})["ok"])
        self.write("queue.json", {"groups": [{"id": "one", "workbook": "one.xlsx"}]})
        self.write("groups/one/state.json", {"id": "different"})
        self.assert_status(400, "/api/groups/one")

    @unittest.skipUnless(os.name == "nt", "Native Windows process creation identity")
    def test_stale_runner_pid_cannot_present_saved_progress_as_live(self):
        from ctypes import wintypes
        kernel = ctypes.windll.kernel32
        kernel.GetCurrentProcess.restype = wintypes.HANDLE
        kernel.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
        stamps = [wintypes.FILETIME() for _ in range(4)]
        self.assertTrue(kernel.GetProcessTimes(kernel.GetCurrentProcess(), *(ctypes.byref(stamp) for stamp in stamps)))
        ticks = (stamps[0].dwHighDateTime << 32) | stamps[0].dwLowDateTime
        started = (ticks - 116444736000000000) // 10000
        queue = {"runner_pid": os.getpid(), "runner_started_unix_ms": started,
                 "running": True, "phase": "comparing", "groups": [{"id": "one", "active": True, "status": "active"}]}
        self.write("queue.json", queue)
        state = self.request("/api/state")
        self.assertTrue(state["runner_alive"])
        self.assertTrue(state["running"])
        queue["runner_started_unix_ms"] = started - 1000
        self.write("queue.json", queue)
        stale = self.request("/api/state")
        self.assertFalse(stale["runner_alive"])
        self.assertFalse(stale["running"])
        self.assertEqual(stale["phase"], "interrupted")
        self.assertEqual(stale["groups"][0]["status"], "interrupted")
        self.assertEqual(self.request("/api/groups/one")["status"], "interrupted")
        self.assertTrue(json.loads((self.data / "queue.json").read_text())["running"], "Read-only owner check modified durable state")
        del queue["runner_started_unix_ms"]
        self.write("queue.json", queue)
        unknown = self.request("/api/state")
        self.assertIsNone(unknown["runner_alive"])
        self.assertEqual(unknown["phase"], "owner_unverified")


if __name__ == "__main__":
    unittest.main(argv=[__file__, *REMAINING])
