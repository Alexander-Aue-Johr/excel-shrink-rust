"""Exercise the classic host through HTTP and its actual Windows named pipe.

Pass --host path/to/excel-shrink-cli.exe; the legacy OfficeCaptureHost.dll can
also be used as a migration oracle. No Excel or installed .NET is needed for
the Rust host.
"""

import argparse
import base64
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.request
import uuid
import zlib


PARSER = argparse.ArgumentParser(add_help=False)
PARSER.add_argument("--host", type=Path, required=True)
OPTIONS, REMAINING = PARSER.parse_known_args()


def png():
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    header = struct.pack(">IIBBBBB", 1, 1, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(
        b"IDAT", zlib.compress(b"\x00\xff\x00\x00\xff")
    ) + chunk(b"IEND", b"")


class ClassicHostProtocol(unittest.TestCase):
    def setUp(self):
        test_root = Path(__file__).resolve().parent / "obj"
        test_root.mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix="classic-host-", dir=test_root)
        self.root = Path(self.temp.name)
        self.addCleanup(self.temp.cleanup)
        self.output = self.root / "output"
        self.run_id = uuid.uuid4().hex
        self.pipe_name = "excel-shrink-host-test-" + uuid.uuid4().hex
        self.process = None

    def start(self, mode="capture", sheets=(1,), telemetry=False):
        web = self.root / "web"
        web.mkdir()
        (web / "index.html").write_text("<title>Capture host test</title>")
        config = self.root / "config.json"
        options = {"mode": mode, "sheets": [{"index": index, "name": f"S{index}"} for index in sheets]}
        if mode == "paired":
            options.update(original={"sheets": options["sheets"]}, shrunk={"sheets": options["sheets"]})
        config.write_text(json.dumps(options), encoding="utf-8")
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        self.url = f"http://127.0.0.1:{port}"
        host = str(OPTIONS.host.resolve())
        command = ["dotnet", host] if OPTIONS.host.suffix == ".dll" else [host, "capture-host", "classic"]
        command += [
            "--config", str(config), "--output", str(self.output),
            "--published-output", str(self.output), "--web", str(web),
            "--run-id", self.run_id, "--port", str(port),
        ]
        if mode == "paired":
            command += ["--stream-pipe", self.pipe_name]
        if telemetry:
            self.telemetry = self.root / "observatory" / "classic.json"
            self.telemetry.parent.mkdir()
            command += ["--telemetry-path", str(self.telemetry), "--attempt-id", "attempt-1",
                        "--comparison-profile", "too-aggressive-cleaning"]
        self.log = (self.root / "host.log").open("wb")
        self.process = subprocess.Popen(
            command, stdout=self.log, stderr=self.log,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
        self.addCleanup(self.stop)
        for _ in range(100):
            try:
                self.request("/health")
                return
            except urllib.error.URLError:
                if self.process.poll() is not None:
                    self.fail("Host exited: " + (self.root / "host.log").read_text())
                time.sleep(.05)
        self.fail("Host did not start")

    def stop(self):
        if self.process.poll() is None:
            self.process.terminate()
        self.process.wait(timeout=10)
        self.log.close()

    @contextmanager
    def workers(self, count):
        executor = ThreadPoolExecutor(max_workers=count)
        try:
            yield executor
        except BaseException:
            # A failed assertion must release a consumer blocked on the pipe,
            # otherwise waiting for its thread would hide the actual failure.
            if self.process.poll() is None:
                self.process.terminate()
                self.process.wait(timeout=10)
            raise
        finally:
            executor.shutdown(wait=True, cancel_futures=True)

    def request(self, route, data=None):
        headers = {}
        if data is not None:
            headers["Content-Type"] = "application/json"
            data = json.dumps(data).encode()
        with urllib.request.urlopen(
            urllib.request.Request(self.url + route, data=data, headers=headers), timeout=10
        ) as response:
            return json.load(response)

    def data(self, side="original", sheet=1, tile=0):
        return {"run_id": self.run_id, "side": side, "sheet_index": sheet,
                "sheet_name": f"S{sheet}", "tile_index": tile, "range": "A1:B1"}

    def assert_http(self, status, route, data):
        with self.assertRaises(urllib.error.HTTPError) as failure:
            self.request(route, data)
        self.assertEqual(failure.exception.code, status)

    def test_images_validate_identity_png_and_do_not_overwrite(self):
        self.start()
        image = self.data()
        image["base64"] = base64.b64encode(png()).decode()
        receipt = self.request("/image", image)
        self.assertEqual((receipt["width"], receipt["height"]), (1, 1))
        self.assertEqual(Path(receipt["path"]).read_bytes(), png())
        # The legacy host let CreateNew's IOException become HTTP 500. Both
        # implementations must preserve the first image and reject the retry.
        duplicate_status = 500 if OPTIONS.host.suffix == ".dll" else 400
        self.assert_http(duplicate_status, "/image", image)
        self.assert_http(409, "/image", {**image, "run_id": "retired"})
        self.assert_http(400, "/image", {**image, "tile_index": 1, "sheet_name": "wrong"})
        self.assert_http(400, "/image", {**image, "tile_index": 1, "range": "../../A1"})
        corrupt = bytearray(png())
        corrupt[29] ^= 1
        self.assert_http(400, "/image", {**image, "tile_index": 1, "base64": base64.b64encode(corrupt).decode()})
        self.assertEqual(len(list((self.output / "images").glob("*.png"))), 1)

    def test_result_is_published_once_and_requires_expected_array(self):
        self.start(mode="heights")
        self.assert_http(400, "/result", {"run_id": self.run_id, "captures": []})
        result = {"run_id": self.run_id, "heights": []}
        self.assertTrue(self.request("/result", result)["ok"])
        self.assertEqual(json.loads((self.output / "result.json").read_text()), result)
        self.assert_http(409, "/result", result)

    @unittest.skipIf(OPTIONS.host.suffix == ".dll", "Rust classic telemetry")
    def test_classic_observations_survive_worker_cleanup_and_separate_heartbeat_from_progress(self):
        self.start(telemetry=True)
        phase = {"run_id": self.run_id, "side": "shrunk", "state": "capturing",
                 "kind": "image_received", "progress": True,
                 "details": {"sheet_name": "S1", "range": "A1:B1"}}
        self.request("/telemetry", phase)
        before = json.loads(self.telemetry.read_text())
        self.request("/telemetry", {"run_id": self.run_id, "side": "shrunk", "heartbeat": True})
        after = json.loads(self.telemetry.read_text())
        self.assertEqual(after["attempt_id"], "attempt-1")
        client = after["clients"]["too-aggressive-cleaning"]
        self.assertEqual(client["heartbeat_count"], 1)
        self.assertEqual(client["monitor"]["uploaded_images"], 1)
        self.assertEqual(client["monitor"], before["clients"]["too-aggressive-cleaning"]["monitor"])
        self.request("/telemetry", {**phase, "state": "closed", "kind": "classic_phase", "progress": False})
        self.request("/telemetry", {"run_id": self.run_id, "side": "shrunk", "heartbeat": True})
        self.assertEqual(json.loads(self.telemetry.read_text())["clients"]["too-aggressive-cleaning"]["stage"], "closed")
        self.assert_http(400, "/telemetry", {**phase, "run_id": "retired"})
        self.assert_http(413, "/telemetry", {**phase, "details": {"unbounded": "x" * 9000}})
        # The durable observation is outside the disposable capture output.
        self.output.mkdir(exist_ok=True)
        self.output.rmdir()
        self.assertTrue(self.telemetry.is_file())

    @unittest.skipUnless(os.name == "nt" and OPTIONS.host.suffix != ".dll", "Rust paired telemetry")
    def test_snapshot_write_failure_does_not_abort_paired_geometry(self):
        self.start(mode="paired", telemetry=True)
        self.telemetry.unlink()
        self.telemetry.mkdir()  # A directory cannot be replaced by a JSON file.
        data = [{**self.data(side), "tile_rows": [[0, 1]], "end_column": 4, "max_end_column": 8}
                for side in ("original", "shrunk")]
        with self.workers(2) as executor:
            geometry = self.exchange(executor, "/pair/geometry", data)
        self.assertEqual(geometry, [{"tile_rows": [[0, 1]], "capture_right_column": 4}] * 2)
        self.assertFalse(self.request("/health")["telemetry_available"])
        receipt = self.request("/telemetry", {"run_id": self.run_id, "side": "original", "heartbeat": True})
        self.assertTrue(receipt["ok"])
        self.assertFalse(receipt["telemetry_saved"])

    def open_pipe(self):
        for _ in range(100):
            try:
                return open("\\\\.\\pipe\\" + self.pipe_name, "r+b", buffering=0)
            except OSError:
                time.sleep(.05)
        self.fail("Named pipe did not accept the consumer")

    @staticmethod
    def read_exact(stream, length):
        chunks = bytearray()
        while len(chunks) < length:
            part = stream.read(length - len(chunks))
            if not part:
                raise EOFError("Comparison pipe ended early")
            chunks.extend(part)
        return bytes(chunks)

    def read_frame(self, stream):
        header = self.read_exact(stream, 16)
        length, metadata, original, shrunk = struct.unpack("<iiii", header)
        self.assertEqual(length, 12 + metadata + original + shrunk)
        return (json.loads(self.read_exact(stream, metadata)),
                self.read_exact(stream, original), self.read_exact(stream, shrunk))

    def exchange(self, executor, route, payloads):
        futures = [executor.submit(self.request, route, payload) for payload in payloads]
        return [future.result(timeout=15) for future in futures]

    @unittest.skipUnless(os.name == "nt", "Windows named-pipe protocol")
    def test_three_sheets_keep_height_image_order_and_ack_backpressure(self):
        sheets = (1, 3, 5)
        self.start(mode="paired", sheets=sheets)
        for side in ("original", "shrunk", "original"):
            self.assertEqual(self.request(f"/config?side={side}")["side"], side)
        received_image = threading.Event()
        acknowledge_image = threading.Event()

        def consume():
            with self.open_pipe() as pipe:
                for sheet in sheets:
                    metadata, left, right = self.read_frame(pipe)
                    self.assertEqual(metadata, {"kind": "heights", "sheet_index": sheet})
                    self.assertEqual(json.loads(left)["side"], "original")
                    self.assertEqual(json.loads(right)["side"], "shrunk")
                    response = b'{"rows":[]}'
                    pipe.write(b"\x03" + struct.pack("<i", len(response)) + response)
                    metadata, left, right = self.read_frame(pipe)
                    self.assertEqual((metadata["sheet_index"], metadata["tile_index"]), (sheet, 0))
                    self.assertEqual(metadata["capture_right_column"], 30)
                    self.assertEqual((left, right), (png(), png()))
                    if sheet == 1:
                        received_image.set()
                        if not acknowledge_image.wait(timeout=10):
                            raise TimeoutError("Test did not acknowledge the image")
                    pipe.write(b"\x01")
                self.assertEqual(self.read_exact(pipe, 4), b"\0" * 4)

        with self.workers(5) as executor:
            consumer = executor.submit(consume)
            for sheet in sheets:
                sides = ("shrunk", "original") if sheet == 3 else ("original", "shrunk")
                data = [self.data(side, sheet) for side in sides]
                heights = self.exchange(executor, "/pair/heights", [
                    {**item, "heights": {"sheet_index": sheet, "side": item["side"], "runs": []}}
                    for item in data
                ])
                self.assertEqual(heights, [{"rows": []}, {"rows": []}])
                geometry = self.exchange(executor, "/pair/geometry", [
                    {**item, "tile_rows": [[0, 1]], "end_column": 24 if item["side"] == "original" else 30,
                     "max_end_column": 65} for item in data
                ])
                self.assertEqual(geometry, [{"tile_rows": [[0, 1]], "capture_right_column": 30}] * 2)
                uploads = [executor.submit(self.request, "/pair/image", {
                    **item, "base64": base64.b64encode(png()).decode()
                }) for item in data]
                if sheet == 1:
                    self.assertTrue(received_image.wait(timeout=10))
                    self.assertTrue(all(not upload.done() for upload in uploads))
                    acknowledge_image.set()
                for upload in uploads:
                    self.assertTrue(upload.result(timeout=10)["ok"])
                self.exchange(executor, "/pair/sheet", data)
            self.exchange(executor, "/result", [
                {"run_id": self.run_id, "side": side} for side in ("original", "shrunk")
            ])
            consumer.result(timeout=10)
        result = json.loads((self.output / "result.json").read_text())
        self.assertNotIn("error", result)

    @unittest.skipUnless(os.name == "nt", "Windows named-pipe protocol")
    def test_pair_rejects_invalid_boundaries_duplicates_and_out_of_order_tiles(self):
        self.start(mode="paired")
        original = {**self.data(), "tile_rows": [[0, 1]], "end_column": 24, "max_end_column": 65}
        for end, maximum in ((-1, 65), (66, 65), (65, 16384)):
            self.assert_http(400, "/pair/geometry", {
                **original, "end_column": end, "max_end_column": maximum,
            })

        def consume_end():
            with self.open_pipe() as pipe:
                self.assertEqual(self.read_exact(pipe, 4), b"\0" * 4)

        with self.workers(4) as executor:
            consumer = executor.submit(consume_end)
            requests = [original, {**original, "side": "shrunk", "end_column": 30}]
            # C# accepted a third geometry request after completing the pair.
            # The Rust exchange explicitly rejects that duplicate submission.
            rejects_duplicate = OPTIONS.host.suffix != ".dll"
            if rejects_duplicate:
                requests.insert(1, original)
            results = [executor.submit(self.request, "/pair/geometry", data) for data in requests]
            successes, rejected = 0, 0
            for future in results:
                try:
                    self.assertEqual(future.result(timeout=10)["capture_right_column"], 30)
                    successes += 1
                except urllib.error.HTTPError as error:
                    self.assertEqual(error.code, 400)
                    rejected += 1
            self.assertEqual((successes, rejected), (2, int(rejects_duplicate)))
            image = {**self.data(), "base64": base64.b64encode(png()).decode()}
            self.assert_http(400, "/pair/image", {**image, "tile_index": 1})
            self.assert_http(400, "/pair/image", {**image, "sheet_index": 2})
            self.assert_http(400, "/pair/image", {**image, "run_id": "retired"})
            self.request("/result", {"run_id": self.run_id, "side": "original",
                                     "error": {"code": "TEST_STOP", "message": "End negative protocol test."}})
            consumer.result(timeout=10)

    @unittest.skipUnless(os.name == "nt", "Windows named-pipe protocol")
    def test_consumer_rejection_fails_both_uploads(self):
        self.start(mode="paired")

        def reject_image():
            with self.open_pipe() as pipe:
                self.read_frame(pipe)
                pipe.write(b"\0")
                self.assertEqual(self.read_exact(pipe, 4), b"\0" * 4)

        with self.workers(3) as executor:
            consumer = executor.submit(reject_image)
            data = [self.data(side) for side in ("original", "shrunk")]
            self.exchange(executor, "/pair/geometry", [
                {**item, "tile_rows": [[0, 1]], "end_column": 1, "max_end_column": 65}
                for item in data
            ])
            uploads = [executor.submit(self.request, "/pair/image", {
                **item, "base64": base64.b64encode(png()).decode()
            }) for item in data]
            for upload in uploads:
                with self.assertRaises(urllib.error.HTTPError) as failure:
                    upload.result(timeout=10)
                self.assertEqual(failure.exception.code, 400)
            self.request("/result", {"run_id": self.run_id, "side": "original",
                                     "error": {"code": "CAPTURE_FAILED", "message": "Pixel consumer rejected evidence."}})
            consumer.result(timeout=10)

    @unittest.skipUnless(os.name == "nt", "Windows named-pipe protocol")
    def test_taskpane_restart_after_data_fails_session_and_releases_waiting_peer(self):
        self.start(mode="paired")

        def consume_end():
            with self.open_pipe() as pipe:
                self.assertEqual(self.read_exact(pipe, 4), b"\0" * 4)

        with self.workers(3) as executor:
            consumer = executor.submit(consume_end)
            waiting = executor.submit(self.request, "/pair/geometry", {
                **self.data(), "end_column": 1, "max_end_column": 65, "tile_rows": [[0, 1]],
            })
            # The pending first geometry is actual capture work. Wait until the
            # host has accepted it before requesting the replacement document.
            time.sleep(.15)
            self.assert_http(409, "/config?side=original", None)
            with self.assertRaises(urllib.error.HTTPError) as failure:
                waiting.result(timeout=10)
            self.assertEqual(failure.exception.code, 400)
            consumer.result(timeout=10)
        result = json.loads((self.output / "result.json").read_text())
        self.assertEqual(result["error"]["code"], "TASKPANE_RESTARTED")


if __name__ == "__main__":
    unittest.main(argv=[__file__, *REMAINING])
