"""Resident HTTP contract checks for the Rust CLI or a legacy host DLL."""
import argparse
import base64
import hashlib
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
import uuid


HERE = Path(__file__).resolve().parent
parser = argparse.ArgumentParser(add_help=False)
parser.add_argument("--host", type=Path, default=HERE.parents[2] / "target/release/excel-shrink-cli.exe")
OPTIONS, REMAINING = parser.parse_known_args()


def host_command(settings):
    host = OPTIONS.host.resolve()
    if host.suffix.lower() == ".dll":
        return ["dotnet", str(host), str(settings)]
    return [str(host), "capture-host", "resident", str(settings)]


class HostProtocol(unittest.TestCase):
    def setUp(self):
        self.root = Path(__file__).resolve().parent
        (self.root / "obj").mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix="resident-host-", dir=self.root / "obj")
        self.addCleanup(self.temp.cleanup)
        self.session = Path(self.temp.name)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        settings = {"session": str(self.session), "web": str(self.root.parent / "web"), "port": port, "launcher_token":"test-launcher"}
        config = self.session / "host.json"
        config.write_text(json.dumps(settings), encoding="utf-8")
        self.log = (self.session / "server.log").open("wb")
        self.addCleanup(self.log.close)
        self.process = subprocess.Popen(host_command(config),
                                        stdout=self.log, stderr=self.log,
                                        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        self.addCleanup(self.stop_host)
        self.url = f"http://localhost:{port}"
        self.token = None
        for _ in range(100):
            try:
                self.request("/health")
                break
            except urllib.error.URLError:
                if self.process.poll() is not None:
                    self.fail("Resident host exited: " + (self.session / "server.log").read_text())
                time.sleep(.05)
        else:
            self.fail("Resident host did not start")
        self.write("clients/one.json", {"client_id": "one", "workbook": "test.xlsx", "sheets": []})

    def stop_host(self):
        if self.process.poll() is None:
            self.process.terminate()
        self.process.wait(timeout=10)

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

    def config_path(self, client='one', document=None, run=None):
        return f'/config?client={client}&document={document or uuid.uuid4().hex}&run={run or uuid.uuid4().hex}&request={uuid.uuid4().hex}'

    def configure(self, client='one', ready=True):
        self.last_config_path = self.config_path(client)
        config = self.request(self.last_config_path)
        if ready:
            self.mark_ready(client, config)
        return config

    def mark_ready(self, client, config):
        observed = self.request(f'/ready?client={client}', {'workbook': config['workbook']}, {'X-Client-Token':config['client_token']})
        self.request(f'/launcher/startup-sampled/{client}', {'ready_at':observed['ready_at']}, {'X-Launcher-Token':'test-launcher'})

    def connect(self):
        self.token = self.configure()["client_token"]

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

    def test_hard_reset_retires_only_target_before_jobs_and_protects_new_session(self):
        self.connect()
        old = self.token
        with self.assertRaises(urllib.error.HTTPError) as error:
            self.request('/launcher/reset/one', {'stage':'prepare'})
        self.assertEqual(error.exception.code, 403)
        self.write('clients/peer.json', {'client_id':'peer','workbook':'peer.xlsx','sheets':[]})
        peer = self.configure('peer')['client_token']
        header = {'X-Launcher-Token':'test-launcher'}
        self.assertTrue(self.request('/launcher/reset/one', {'stage':'prepare'}, header)['ok'])
        health = self.request('/health')
        self.assertNotIn('one', health['seen'])
        self.assertIn('peer', health['seen'])
        self.assertTrue(self.request('/job?client=peer', extra={'X-Client-Token':peer})['resetting_client'])
        with self.assertRaises(urllib.error.HTTPError): self.request(self.config_path())
        self.request('/launcher/reset/one', {'stage':'ready'}, header)
        self.connect()
        self.assertNotEqual(self.token, old)
        with self.assertRaises(urllib.error.HTTPError): self.request('/heartbeat?client=one', extra={'X-Client-Token':old})
        self.assertTrue(self.request('/heartbeat?client=peer', extra={'X-Client-Token':peer})['ok'])

    def test_hard_reset_after_any_peer_job_requires_fresh_group_evidence(self):
        self.connect()
        self.command({'id':'0001','kind':'plan','sheets':[]})
        self.write('clients/peer.json', {'client_id':'peer','workbook':'peer.xlsx','sheets':[]})
        self.configure('peer')
        with self.assertRaises(urllib.error.HTTPError) as error:
            self.request('/launcher/reset/peer', {'stage':'prepare'}, {'X-Launcher-Token':'test-launcher'})
        self.assertEqual(error.exception.code, 409)
        self.assertEqual(self.request('/health')['clients']['one']['active_command'], '0001')

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
            self.request(self.config_path())
        self.assertEqual(raised.exception.code, 409)
        result = json.loads((self.session / "responses/one/capture.json").read_text())
        self.assertFalse(result["ok"])
        self.assertEqual(result["error"]["code"], "TASKPANE_CONTEXT_CHANGED")
        self.assertEqual(self.request("/health")["errors"][0]["client_id"], "one")

    def test_workbook_ready_and_exact_launcher_sample_both_precede_dispatch(self):
        config = self.configure(ready=False)
        self.token = config['client_token']
        self.write('commands/one/plan.json', {'id':'plan', 'kind':'plan', 'sheets':[]})
        self.assertTrue(self.request('/job?client=one')['waiting_for_startup_sample'])
        with self.assertRaises(urllib.error.HTTPError):
            self.request('/ready?client=one', {'workbook':'wrong.xlsx'})
        observed = self.request('/ready?client=one', {'workbook':'test.xlsx'})
        self.assertTrue(self.request('/job?client=one')['waiting_for_startup_sample'])
        with self.assertRaises(urllib.error.HTTPError):
            self.request('/launcher/startup-sampled/one', {'ready_at':observed['ready_at']})
        with self.assertRaises(urllib.error.HTTPError):
            self.request('/launcher/startup-sampled/one', {'ready_at':'old-observation'}, {'X-Launcher-Token':'test-launcher'})
        self.request('/launcher/startup-sampled/one', {'ready_at':observed['ready_at']}, {'X-Launcher-Token':'test-launcher'})
        self.assertEqual(self.next_job('one')['id'], 'plan')

    def test_same_runtime_configuration_retry_does_not_fail_an_active_job(self):
        self.connect()
        path = self.last_config_path
        self.command({'id':'plan', 'kind':'plan', 'sheets':[]})
        self.assertEqual(self.request(path)['client_token'], self.token)
        # A bare status probe must not be interpreted as a replacement pane.
        with self.assertRaises(urllib.error.HTTPError): self.request('/config?client=one')
        health = self.request('/health')['clients']['one']
        self.assertIsNone(health['error'])
        self.assertEqual(health['active_command'], 'plan')
        self.assertEqual(health['monitor']['last_event']['details']['outcome'], 'repeated_request')

    def test_replacement_before_work_requires_a_new_ready_sample(self):
        self.connect()
        old_ready = self.request('/health')['clients']['one']['ready_at']
        self.token = self.configure(ready=False)['client_token']
        self.assertTrue(self.request('/job?client=one')['waiting_for_startup_sample'])
        with self.assertRaises(urllib.error.HTTPError):
            self.request('/launcher/startup-sampled/one', {'ready_at':old_ready}, {'X-Launcher-Token':'test-launcher'})

    def test_new_client_can_start_while_another_capture_is_in_progress(self):
        self.connect()
        self.command({"id": "capture", "kind": "capture", "sheets": []})
        self.write("clients/two.json", {"client_id": "two", "workbook": "second.xlsx", "sheets": []})
        self.write("commands/two/capture.json", {"id": "capture", "kind": "capture", "sheets": []})
        self.token = self.configure('two')["client_token"]
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

    def test_slow_image_upload_does_not_block_a_peer_startup_or_heartbeat(self):
        self.connect()
        png = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLttAAAAABJRU5ErkJggg==")
        tile = {"sheet_index": 1, "tile_index": 0, "width": 1, "height": 1}
        self.command({"id": "upload", "kind": "capture_images", "tiles": [tile]})
        port = json.loads((self.session / "host.json").read_text())["port"]
        connection = http.client.HTTPConnection("localhost", port, timeout=5)
        self.addCleanup(connection.close)
        connection.putrequest("POST", "/image/one/upload/1/0")
        connection.putheader("Content-Type", "image/png")
        connection.putheader("Content-Length", str(len(png)))
        connection.putheader("X-Client-Token", self.token)
        connection.endheaders()
        connection.send(png[:24])

        self.write("clients/peer.json", {"client_id": "peer", "workbook": "peer.xlsx", "sheets": []})
        peer = self.configure("peer")
        self.assertTrue(self.request("/heartbeat?client=peer", extra={"X-Client-Token": peer["client_token"]})["ok"])
        connection.send(png[24:])
        response = connection.getresponse()
        self.assertEqual(response.status, 200)
        receipt = json.loads(response.read())
        self.request("/result/one/upload", {"ok": True, "images": [receipt]})

    def test_retired_image_session_and_oversized_diagnostics_fail_before_reading_bodies(self):
        self.connect()
        retired_token = self.token
        self.connect()
        port = json.loads((self.session / "host.json").read_text())["port"]
        for path, token, length in [
            ("/image/one/old/1/0", retired_token, 1024 * 1024),
            ("/heartbeat?client=one", self.token, 8193),
        ]:
            with self.subTest(path=path):
                connection = http.client.HTTPConnection("localhost", port, timeout=3)
                try:
                    connection.putrequest("POST", path)
                    connection.putheader("Content-Length", str(length))
                    connection.putheader("X-Client-Token", token)
                    connection.endheaders()
                    # Deliberately send no body. Rejection must not wait for it.
                    self.assertEqual(connection.getresponse().status, 400)
                finally:
                    connection.close()

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

    def test_pane_resource_samples_are_bounded_and_do_not_invent_progress(self):
        self.connect()
        before = self.request('/health')['clients']['one']['monitor']
        for count in range(30):
            self.request('/heartbeat?client=one', {'retained_capture_plans':count,
                'pending_png_characters':-1, 'untrusted':'ignored'})
        after = self.request('/health')['clients']['one']['monitor']
        self.assertEqual(after['events'], before['events'])
        self.assertEqual(after['last_progress_at'], before['last_progress_at'])
        self.assertEqual(after['pane_resources']['retained_capture_plans'], 29)
        self.assertNotIn('untrusted', after['pane_resources'])
        self.assertNotIn('pending_png_characters', after['pane_resources'])

    def test_heartbeats_and_job_polls_do_not_invent_progress_or_reset_state_age(self):
        self.connect()
        self.command({"id": "capture", "kind": "capture", "sheets": [{"sheet_name": "Observed", "sheet_index": 2}]})
        before = self.request("/health")["clients"]["one"]
        self.assertEqual(before["monitor"]["state"], "capturing")
        transitions = [event for event in before["monitor"]["events"] if event["kind"] == "state_changed"]
        self.assertEqual(transitions[-1]["state"], "capturing")
        self.assertIsNone(transitions[-1]["wait_reason"])
        startup = next(event for event in transitions if event["state"] == "waiting_for_startup_sample")
        self.assertEqual(startup["wait_reason"], "launcher")
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
        self.process = subprocess.Popen(host_command(self.session / "host.json"),
                                        env=environment,stdout=self.log,stderr=self.log,
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
        self.configure('two')
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
    unittest.main(argv=[__file__, *REMAINING])
