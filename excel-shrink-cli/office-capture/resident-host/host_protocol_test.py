"""Resident HTTP protocol checks; build the Release host before running."""
import base64
import hashlib
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

    def test_image_first_upload_needs_no_client_hash_but_receipt_is_bound_to_saved_bytes(self):
        self.connect()
        png = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLttAAAAABJRU5ErkJggg==")
        tile = {"sheet_index": 1,"tile_index": 0,"width": 1,"height": 1,"sha256": "0"*64}
        self.command({"id":"tile","kind":"capture_images","capture_id":"plan","tiles":[tile]})
        receipt = self.request("/image/one/tile/1/0",png,{"Content-Type":"image/png"})
        self.assertEqual(receipt["sha256"],hashlib.sha256(png).hexdigest())
        self.assertEqual(Path(receipt["image_path"]).read_bytes(),png)
        saved_receipt = json.loads((self.session / "receipts/one/tile/s0001-t00000.json").read_text())
        self.assertEqual(saved_receipt, receipt)
        self.assertEqual(receipt["png_bytes"], len(png))
        self.assertGreater(receipt["received_unix_ms"], 0)
        self.assertFalse((self.session / "responses/one/tile.json").exists(), "Tile publication must precede batch completion")
        with self.assertRaises(urllib.error.HTTPError):
            self.request("/result/one/tile",{"ok":True,"images":[{**tile,**receipt,"sha256":"f"*64}]})
        self.request("/result/one/tile",{"ok":True,"images":[{**tile,**receipt}]})

    def test_only_an_actual_heartbeat_updates_the_heartbeat_timestamp(self):
        self.connect()
        self.assertIsNone(self.request("/health")["clients"]["one"]["last_heartbeat_at"])
        self.request("/job?client=one")
        self.assertIsNone(self.request("/health")["clients"]["one"]["last_heartbeat_at"])
        self.request("/heartbeat?client=one")
        before = self.request("/health")["clients"]["one"]
        self.request("/job?client=one")
        after = self.request("/health")["clients"]["one"]
        self.assertIsNotNone(before["last_heartbeat_at"])
        self.assertEqual(after["last_heartbeat_at"], before["last_heartbeat_at"])
        self.assertEqual(after["heartbeat_count"], 1)

    def test_heartbeats_and_job_polls_do_not_invent_progress_or_reset_state_age(self):
        self.connect()
        self.command({"id": "capture", "kind": "capture", "sheets": [{"sheet_name": "Observed", "sheet_index": 2}]})
        before = self.request("/health")["clients"]["one"]
        self.assertEqual(before["monitor"]["state"], "capturing")
        self.assertEqual(before["monitor"]["active_job"]["locations"][0]["sheet_name"], "Observed")
        self.request("/heartbeat?client=one")
        after = self.request("/health")["clients"]["one"]
        self.assertEqual(after["monitor"], before["monitor"])
        self.assertGreater(after["heartbeat_count"], before["heartbeat_count"])
        with self.assertRaises(urllib.error.HTTPError):
            self.request("/job?client=one")
        self.assertEqual(self.request("/health")["clients"]["one"]["monitor"], before["monitor"])
        self.request("/result/one/capture", {"ok": True, "tiles": [], "heights": []})
        complete = self.request("/health")["clients"]["one"]["monitor"]
        self.assertEqual(complete["last_success"]["kind"], "job_completed")
        self.assertEqual(complete["completed_commands"], 1)
        for _ in range(3):
            self.request("/job?client=one")
            self.request("/heartbeat?client=one")
        self.assertEqual(self.request("/health")["clients"]["one"]["monitor"], complete)

    def test_each_image_is_real_progress_and_history_is_bounded(self):
        self.connect()
        png = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLttAAAAABJRU5ErkJggg==")
        tiles = [{"sheet_index": 2, "sheet_name": "Observed", "tile_index": n, "range": f"A{n+1}", "width": 1, "height": 1} for n in range(30)]
        self.command({"id": "batch", "kind": "capture_images", "tiles": tiles})
        receipts = [self.request(f"/image/one/batch/2/{n}", png, {"Content-Type": "image/png"}) for n in range(30)]
        monitor = self.request("/health")["clients"]["one"]["monitor"]
        self.assertEqual(monitor["uploaded_images"], 30)
        self.assertEqual(monitor["completed_commands"], 0)
        self.assertEqual(monitor["last_success"]["kind"], "image_received")
        self.assertEqual(monitor["last_success"]["details"]["range"], "A30")
        self.assertEqual(len(monitor["events"]), 24)
        self.assertEqual(len(monitor["active_job"]["locations"]), 8)
        self.request("/result/one/batch", {"ok": True, "images": receipts})

    def test_image_batches_publish_each_receipt_and_reject_partial_completion(self):
        self.connect()
        png = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLttAAAAABJRU5ErkJggg==")
        tiles = [{"sheet_index":1,"tile_index":i,"width":1,"height":1,"x":0,"y":i,"capture_pixels":1} for i in range(2)]
        self.command({"id":"batch","kind":"capture_images","capture_id":"plan","tiles":tiles})
        first = self.request("/image/one/batch/1/0",png,{"Content-Type":"image/png"})
        self.assertEqual(first["y"],0)
        self.assertFalse((self.session / "receipts/one/batch/s0001-t00001.json").exists())
        with self.assertRaises(urllib.error.HTTPError):
            self.request("/result/one/batch",{"ok":True,"images":[first]})
        second = self.request("/image/one/batch/1/1",png,{"Content-Type":"image/png"})
        self.assertEqual(second["y"],1)
        self.assertFalse((self.session / "responses/one/batch.json").exists())
        self.request("/result/one/batch",{"ok":True,"images":[first,second]})
        self.assertEqual(len(json.loads((self.session / "responses/one/batch.json").read_text())["images"]),2)

    def test_live_jobs_wait_for_all_primary_clients_and_pause_only_at_job_boundaries(self):
        self.process.terminate()
        self.process.wait(timeout=10)
        self.write("clients/two.json",{"client_id":"two","workbook":"two.xlsx","sheets":[]})
        self.write("control/control.json",{"paused":False})
        environment = {**os.environ,"EXCEL_SHRINK_CONTROL_ROOM_DIR":str(self.session / "control")}
        self.process = subprocess.Popen(["dotnet",str(self.root / "bin/Release/net8.0/ResidentCaptureHost.dll"),str(self.session / "host.json")],
                                        env=environment,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,
                                        creationflags=getattr(subprocess,"CREATE_NO_WINDOW",0))
        for _ in range(100):
            try:
                self.request("/health")
                break
            except urllib.error.URLError:
                time.sleep(.05)
        self.connect()
        self.write("commands/one/plan.json",{"id":"plan","kind":"plan","sheets":[]})
        self.assertTrue(self.request("/job?client=one")["waiting_for_peers"])
        self.request("/config?client=two")
        self.write("control/control.json",{"paused":True})
        self.assertTrue(self.request("/job?client=one")["paused"])
        (self.session / "control/control.json").write_text("{broken",encoding="utf-8")
        self.assertTrue(self.request("/job?client=one")["paused"])
        self.write("control/control.json",{"paused":False})
        self.assertEqual(self.next_job("one")["kind"],"plan")
        self.write("control/control.json",{"paused":True})
        # An already dispatched job is allowed to report completion while paused.
        self.request("/result/one/plan",{"ok":True,"tiles":[],"heights":[]})
        self.assertTrue(self.request("/job?client=one")["paused"])


if __name__ == "__main__":
    unittest.main()
