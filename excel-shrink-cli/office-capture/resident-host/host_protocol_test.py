"""Resident HTTP protocol checks; build the Release host before running."""
import base64
import hashlib
import json
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import unittest
import urllib.error
import urllib.request


class HostProtocol(unittest.TestCase):
    def setUp(self):
        self.root = Path(__file__).resolve().parent
        self.temp = tempfile.TemporaryDirectory(prefix="resident-host-", dir=self.root / "obj")
        self.session = Path(self.temp.name)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        settings = {"session": str(self.session), "web": str(self.root.parent / "web"), "port": port}
        config = self.session / "host.json"
        config.write_text(json.dumps(settings), encoding="utf-8")
        self.process = subprocess.Popen(["dotnet", str(self.root / "bin/Release/net8.0/ResidentCaptureHost.dll"), str(config)],
                                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        self.url = f"http://localhost:{port}"
        self.token = None
        for _ in range(100):
            try:
                self.request("/health")
                break
            except urllib.error.URLError:
                time.sleep(.05)
        else:
            self.fail("Resident host did not start")
        self.write("clients/one.json", {"client_id": "one", "workbook": "test.xlsx", "sheets": []})

    def tearDown(self):
        self.process.terminate()
        self.process.wait(timeout=10)
        self.temp.cleanup()

    def write(self, relative, data):
        path = self.session / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(data), encoding="utf-8")

    def request(self, path, body=None, extra=None):
        headers = {} if self.token is None else {"X-Client-Token": self.token}
        headers.update(extra or {})
        if isinstance(body, dict):
            body = json.dumps(body).encode()
            headers["Content-Type"] = "application/json"
        with urllib.request.urlopen(urllib.request.Request(self.url + path, data=body, headers=headers), timeout=5) as result:
            return json.load(result)

    def connect(self):
        self.token = self.request("/config?client=one")["client_token"]

    def command(self, data):
        self.write(f"commands/one/{data['id']}.json", data)
        return self.next_job("one")

    def next_job(self, client):
        for _ in range(80):
            result = self.request(f"/job?client={client}")
            if not result.get("idle"):
                return result
            time.sleep(.05)
        self.fail("Resident client did not leave its startup quiet period")

    def test_pre_capture_reload_retires_old_token_and_job_results_are_atomic(self):
        self.connect()
        first = self.token
        self.connect()
        self.assertNotEqual(first, self.token)
        with self.assertRaises(urllib.error.HTTPError):
            self.request("/job?client=one", extra={"X-Client-Token": first})
        self.assertTrue(self.request("/job?client=one")["idle"])
        command = self.command({"id": "0001", "kind": "capture", "sheets": []})
        self.assertEqual(command["id"], "0001")
        with self.assertRaises(urllib.error.HTTPError):
            self.request("/job?client=one")
        result = {"ok": True, "heights": [], "tiles": []}
        self.request("/result/one/0001", result)
        self.assertEqual(json.loads((self.session / "responses/one/0001.json").read_text()), result)
        self.assertFalse(list(self.session.rglob("*.pending")))
        self.assertTrue(self.request("/job?client=one")["idle"])

    def test_restart_after_job_is_explicit_failure(self):
        self.connect()
        self.command({"id": "capture", "kind": "capture", "sheets": []})
        with self.assertRaises(urllib.error.HTTPError) as raised:
            self.request("/config?client=one")
        self.assertEqual(raised.exception.code, 409)
        result = json.loads((self.session / "responses/one/capture.json").read_text())
        self.assertFalse(result["ok"])
        self.assertEqual(result["error"]["code"], "TASKPANE_RESTARTED")
        self.assertEqual(self.request("/health")["errors"][0]["client_id"], "one")

    def test_new_client_can_start_while_another_capture_is_in_progress(self):
        self.connect()
        self.command({"id": "capture", "kind": "capture", "sheets": []})
        self.write("clients/two.json", {"client_id": "two", "workbook": "second.xlsx", "sheets": []})
        self.write("commands/two/capture.json", {"id": "capture", "kind": "capture", "sheets": []})
        self.token = self.request("/config?client=two")["client_token"]
        self.assertTrue(self.request("/job?client=two")["settling"])
        self.assertEqual(self.next_job("two")["id"], "capture")
        health = self.request("/health")
        self.assertEqual(health["seen"], ["one", "two"])
        self.assertEqual(health["clients"]["one"]["active_command"], "capture")
        self.assertEqual(health["clients"]["two"]["active_command"], "capture")

    def test_only_requested_matching_png_can_be_saved(self):
        self.connect()
        png = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLttAAAAABJRU5ErkJggg==")
        sha = hashlib.sha256(png).hexdigest()
        tile = {"sheet_index": 1, "tile_index": 0, "sha256": sha, "width": 1, "height": 1}
        self.command({"id": "images", "kind": "images", "capture_id": "capture", "tiles": [tile]})
        with self.assertRaises(urllib.error.HTTPError):
            self.request("/image/one/images/1/1", png, {"X-Sha256": sha})
        with self.assertRaises(urllib.error.HTTPError):
            self.request("/image/one/images/1/0", png + b"bad", {"X-Sha256": sha})
        receipt = self.request("/image/one/images/1/0", png, {"X-Sha256": sha})
        self.assertEqual(Path(receipt["image_path"]).read_bytes(), png)
        with self.assertRaises(urllib.error.HTTPError):
            self.request("/image/one/images/1/0", png, {"X-Sha256": sha})
        self.request("/result/one/images", {"ok": True, "images": [{**tile, "image_path": receipt["image_path"]}]})


if __name__ == "__main__":
    unittest.main()
