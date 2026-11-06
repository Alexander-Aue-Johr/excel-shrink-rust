"""Loopback control API checks for the Rust CLI or a legacy C# host DLL."""
import argparse
import concurrent.futures
import ctypes
import http.client
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.error
import urllib.request
import zipfile


HERE = Path(__file__).resolve().parent
parser = argparse.ArgumentParser(add_help=False)
parser.add_argument("--host", type=Path,
                    default=HERE.parents[2] / "target/release/excel-shrink-cli.exe")
OPTIONS, REMAINING = parser.parse_known_args()


def current_process_started_ms(pid=None):
    from ctypes import wintypes
    kernel = ctypes.windll.kernel32
    kernel.GetCurrentProcess.restype = wintypes.HANDLE
    kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel.OpenProcess.restype = wintypes.HANDLE
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
    handle = kernel.GetCurrentProcess() if pid is None else kernel.OpenProcess(0x1000, False, pid)
    if not handle:
        raise OSError("Cannot open the test process")
    stamps = [wintypes.FILETIME() for _ in range(4)]
    try:
        if not kernel.GetProcessTimes(handle, *(ctypes.byref(stamp) for stamp in stamps)):
            raise OSError("Cannot inspect the test process identity")
    finally:
        if pid is not None:
            kernel.CloseHandle(handle)
    ticks = (stamps[0].dwHighDateTime << 32) | stamps[0].dwLowDateTime
    return (ticks - 116444736000000000) // 10000


class ControlRoomProtocol(unittest.TestCase):
    def setUp(self):
        (HERE / "obj").mkdir(exist_ok=True)
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
        host = OPTIONS.host.resolve()
        command = (["dotnet", str(host), "--control-room", str(settings)]
                   if host.suffix == ".dll" else
                   [str(host), "capture-host", "control-room", str(settings)])
        self.process = subprocess.Popen(
            command,
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

    def test_live_state_is_bounded_while_workbook_history_remains_pageable(self):
        groups = [{"id": f"book-{index}", "run_id": "current", "active": False,
                   "status": "completed", "started_unix_ms": index} for index in range(9000)]
        groups[1]["active"] = True
        groups[1]["status"] = "comparing"
        self.write("queue.json", {"run_id": "current", "groups": groups, "open_groups": 1})
        self.write("workbook-index.json", {"version": 1, "run_id": "current", "workbooks": groups})
        state = self.request("/api/state?group_scope=live&selected=book-3")
        self.assertEqual(state["groups_total"], 9000)
        self.assertTrue(state["groups_truncated"])
        self.assertEqual(len(state["groups"]), 10)
        self.assertEqual({row["id"] for row in state["groups"]},
                         {"book-1", "book-3", *(f"book-{index}" for index in range(8992, 9000))})
        self.assertEqual(len(self.request("/api/state")["groups"]), 9000,
                         "Legacy callers still receive the full queue")
        history = self.request("/api/workbooks?offset=500&limit=100")
        self.assertEqual(history["total"], 9000)
        self.assertEqual(history["workbooks"][0]["id"], "book-500")
        self.assert_status(400, "/api/state?group_scope=unknown")
        self.assert_status(400, "/api/state?group_scope=live&selected=../outside")

    def test_inspection_profiles_and_skipped_reason_follow_current_attempt(self):
        self.write("queue.json", {"groups": [{"id": "book", "status": "skipped",
            "attempt_id": "current", "active": False}]})
        self.write("../records/book.json", {"status": "skipped", "error": "Excel opening timed out",
            "control_room_attempt": {"id": "current"}})
        prepared = self.data.parent / "prepared"
        prepared.mkdir()
        with zipfile.ZipFile(prepared / "book.zip", "w") as bundle:
            bundle.writestr("original.xlsx", b"original")
            bundle.writestr("maximum-cleaning.xlsx", b"maximum")
        detail = self.request("/api/groups/book")
        self.assertEqual(detail["inspection_profiles"], ["original", "maximum-cleaning"])
        self.assertEqual(detail["outcome_error"], "Excel opening timed out")
        self.assert_status(400, "/api/groups/book/inspect", {"profile": "../../outside"})
        self.assert_status(400, "/api/groups/book/inspect", {"profile": "original", "path": "outside"})
        self.assert_status(404, "/api/groups/unknown/inspect", {"profile": "original"})
        self.write("queue.json", {"groups": [{"id": "book", "status": "opening",
            "attempt_id": "retry", "active": True}]})
        self.assertIsNone(self.request("/api/groups/book").get("outcome_error"))

    def test_corrupt_inspection_bundle_keeps_result_and_reason_visible(self):
        self.write("queue.json", {"groups": [{"id": "book", "status": "skipped", "active": False}]})
        self.write("../records/book.json", {"status": "skipped", "error": "Malformed workbook"})
        prepared = self.data.parent / "prepared"
        prepared.mkdir()
        (prepared / "book.zip").write_bytes(b"not a ZIP")
        detail = self.request("/api/groups/book")
        self.assertEqual(detail["outcome_error"], "Malformed workbook")
        self.assertEqual(detail["inspection_profiles"], [])
        self.assertTrue(detail["diagnostics"])

    def test_stop_is_scoped_to_verified_runner_and_cannot_be_unpaused(self):
        self.write("queue.json", {"running": True, "runner_pid": os.getpid(),
            "runner_started_unix_ms": current_process_started_ms() - 1000, "stop_supported": True})
        self.assert_status(409, "/api/run/stop", b"")
        self.assertFalse((self.data / "stop-request.json").exists())
        queue = {"running": True, "runner_pid": os.getpid(),
            "runner_started_unix_ms": current_process_started_ms(), "stop_supported": True}
        self.write("queue.json", queue)
        self.request("/api/control", {"paused": True})
        self.assertTrue(self.request("/api/run/stop", b"")["ok"])
        self.assertTrue(self.request("/api/state")["stop_requested"])
        controls = self.request("/api/control", {"target_groups": 4, "paused": True})["controls"]
        self.assertEqual(controls["target_groups"], 0)
        self.assertFalse(controls["paused"])
        queue["runner_started_unix_ms"] += 1
        self.write("queue.json", queue)
        self.assertFalse(self.request("/api/state")["stop_requested"])

    def test_immediate_stop_cancels_admission_for_verified_runner(self):
        dummy = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"],
                                 creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        try:
            started = current_process_started_ms(dummy.pid)
            queue = {"running": True, "runner_pid": dummy.pid,
                     "runner_started_unix_ms": started - 1000,
                     "terminate_supported": True}
            self.write("queue.json", queue)
            self.assert_status(409, "/api/run/terminate", b"")
            self.assertFalse((self.data / "terminate-request.json").exists())
            queue["runner_started_unix_ms"] = started
            self.write("queue.json", queue)
            self.assertTrue(self.request("/api/run/terminate", b"")["ok"])
            self.assertTrue(self.request("/api/state")["terminate_requested"])
            controls = self.request("/api/control", {"target_groups": 4, "paused": True})["controls"]
            self.assertEqual(controls["target_groups"], 0)
            self.assertFalse(controls["paused"])
            self.assertEqual(json.loads((self.data / "terminate-request.json").read_text()),
                             {"runner_pid": dummy.pid, "runner_started_unix_ms": started})
        finally:
            dummy.terminate()
            dummy.wait(timeout=10)

    def test_completed_stop_request_allows_saving_controls_without_starting_runner(self):
        queue = {"running": False, "phase": "stopped", "runner_pid": os.getpid(),
                 "runner_started_unix_ms": current_process_started_ms(), "stop_supported": True}
        self.write("queue.json", queue)
        self.write("stop-request.json", {"runner_pid": queue["runner_pid"],
                   "runner_started_unix_ms": queue["runner_started_unix_ms"]})
        state = self.request("/api/state")
        self.assertTrue(state["runner_alive"], "The saved stopped state must win even while the owner process exists")
        self.assertFalse(state["stop_requested"])
        controls = self.request("/api/control", {"target_groups": 4})["controls"]
        self.assertEqual(controls["target_groups"], 4)
        state = self.request("/api/state")
        self.assertEqual(state["controls"]["target_groups"], 4)
        self.assertFalse(state["running"])
        self.assertEqual(state["phase"], "stopped")
        self.assertFalse(state["stop_requested"])
        self.assertEqual(json.loads((self.data / "queue.json").read_text()), queue)

    def test_dead_or_reused_runner_stop_request_does_not_override_saved_controls(self):
        for label, pid, started in [
            ("dead", 2147483647, 1),
            ("reused", os.getpid(), current_process_started_ms() - 1000),
        ]:
            with self.subTest(label=label):
                queue = {"running": True, "phase": "comparing", "runner_pid": pid,
                         "runner_started_unix_ms": started, "stop_supported": True}
                self.write("queue.json", queue)
                self.write("stop-request.json", {"runner_pid": pid, "runner_started_unix_ms": started})
                state = self.request("/api/state")
                self.assertFalse(state["runner_alive"])
                self.assertFalse(state["stop_requested"])
                controls = self.request("/api/control", {"target_groups": 3, "paused": True})["controls"]
                self.assertEqual(controls["target_groups"], 3)
                self.assertTrue(controls["paused"])
                state = self.request("/api/state")
                self.assertEqual(state["controls"], controls)
                self.assertFalse(state["running"])
                self.assertFalse(state["stop_requested"])
                self.assertEqual(json.loads((self.data / "queue.json").read_text()), queue)

    def test_unverified_runner_keeps_matching_stop_request_in_force(self):
        # Missing creation identity leaves the process potentially alive. Keep
        # the orderly stop in force until the runner is known to have stopped.
        queue = {"running": True, "phase": "comparing", "runner_pid": os.getpid()}
        self.write("queue.json", queue)
        self.write("stop-request.json", {"runner_pid": os.getpid()})
        state = self.request("/api/state")
        self.assertIsNone(state["runner_alive"])
        self.assertEqual(state["phase"], "owner_unverified")
        self.assertTrue(state["stop_requested"])
        controls = self.request("/api/control", {"target_groups": 4, "paused": True})["controls"]
        self.assertEqual(controls["target_groups"], 0)
        self.assertFalse(controls["paused"])
        self.assertTrue(self.request("/api/state")["stop_requested"])

    def test_update_ready_requires_a_released_output_lock(self):
        import msvcrt
        self.write("queue.json", {"running": False, "runner_pid": os.getpid(),
            "runner_started_unix_ms": current_process_started_ms() - 1000})
        lock_path = self.directory / ".run.lock"
        with lock_path.open("w+b") as lock:
            lock.write(b"0")
            lock.flush()
            lock.seek(0)
            msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
            try:
                self.assertFalse(self.request("/api/state")["update_ready"])
            finally:
                lock.seek(0)
                msvcrt.locking(lock.fileno(), msvcrt.LK_UNLCK, 1)
        self.assertTrue(self.request("/api/state")["update_ready"])

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
        self.assertEqual(state["controls"]["tall_sheet_threshold"], 40)
        self.assertEqual(state["controls"]["tall_sheet_edge_tiles"], 2)
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

    def test_replacement_does_not_show_processes_from_its_previous_attempt(self):
        self.write("queue.json", {"groups": [{"id": "one", "attempt_id": "replacement", "status": "opening"}]})
        self.write("groups/one/launcher.json", {"attempt_id": "previous", "clients": {"original": {"pid": 99}}})
        pending = self.request("/api/groups/one")
        self.assertTrue(pending["launcher_pending"])
        self.assertNotIn("launcher", pending)
        self.write("groups/one/launcher.json", {"clients": {"original": {"pid": 99}}})
        self.assertTrue(self.request("/api/groups/one")["launcher_pending"], "Legacy launcher was attributed to a new attempt")
        self.write("groups/one/state.json", {"attempt_id": "stale-detail"})
        self.write("groups/one/launcher.json", {"attempt_id": "replacement", "clients": {}})
        self.assertEqual(self.request("/api/groups/one")["launcher"]["attempt_id"], "replacement")
        self.assertEqual(self.request("/api/groups/one")["attempt_id"], "replacement")
        self.assertFalse(self.request("/api/groups/one")["available"])
        self.write("groups/one/state.json", {"sheets": [{"name": "Old captures"}]})
        self.assertNotIn("sheets", self.request("/api/groups/one"), "Legacy captures were attributed to the replacement attempt")

    def test_progress_only_belongs_to_the_current_run(self):
        self.write("queue.json", {"run_id": "new", "groups": []})
        self.write("progress.json", {"run_id": "old", "counts": {"total": 900}})
        self.assertNotIn("progress", self.request("/api/state"))
        progress = {"run_id": "new", "counts": {"total": 100, "completed": 10, "skipped": 2},
                    "rates": {"completed_per_hour": 5}, "eta": {"status": "preparation_pending"}}
        self.write("progress.json", progress)
        self.assertEqual(self.request("/api/state")["progress"], progress)
        self.write("progress.json", {"run_id": "new", "oversized": "x" * 65536})
        state = self.request("/api/state")
        self.assertNotIn("progress", state)
        self.assertTrue(state["diagnostics"])

    def test_restart_preparation_overrides_previous_capture_completion(self):
        self.write("groups/one/state.json", {"status": "complete", "primary_complete": True})
        for status in ("preparing", "restart_pending", "restart_waiting_cleanup"):
            with self.subTest(status=status):
                self.write("queue.json", {"groups": [{"id": "one", "status": status}]})
                detail = self.request("/api/groups/one")
                self.assertEqual(detail["status"], status)
                self.assertEqual(detail["capture_status"], "complete")

    def test_controls_have_no_eight_group_cap_and_persist_atomically(self):
        result = self.request("/api/control", {"target_groups": 128, "paused": True, "equal_preview_width": 0,
                                              "capture_tile_megapixels": 2, "capture_batch_tiles": 4, "hide_excel": True,
                                              "tall_sheet_threshold": 75, "tall_sheet_edge_tiles": 3})
        self.assertEqual(result["controls"]["target_groups"], 128)
        self.assertTrue(result["controls"]["paused"])
        self.assertTrue(result["controls"]["hide_excel"])
        self.assertEqual(result["controls"]["capture_tile_megapixels"], 2)
        self.assertEqual(result["controls"]["capture_batch_tiles"], 4)
        self.assertEqual(result["controls"]["tall_sheet_threshold"], 75)
        self.assertEqual(result["controls"]["tall_sheet_edge_tiles"], 3)
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

    def test_ram_reserve_changes_persist_and_leave_other_controls_unchanged(self):
        before = self.request("/api/state")["controls"]
        self.assertEqual(before["system_reserve_mib"], 0)
        for reserve in (4096, 512, 2048, 1, 65536, 0):
            with self.subTest(reserve=reserve):
                result = self.request("/api/control", {"system_reserve_mib": reserve})
                self.assertTrue(result["ok"])
                expected = {**before, "system_reserve_mib": reserve}
                self.assertEqual(result["controls"], expected)
                self.assertEqual(self.request("/api/state")["controls"], expected)
                self.assertEqual(json.loads((self.data / "control.json").read_text()), expected)

    def test_capture_transport_is_persisted_without_changing_active_group_state(self):
        before = self.request("/api/state")["controls"]
        self.assertEqual(before["capture_transport"], "live-images")
        group = {"id": "one", "status": "capturing",
                 "capture_pipeline": {"transport": "live-images"}}
        self.write("groups/one/state.json", group)
        for mode in ("hash-first", "live-images"):
            with self.subTest(mode=mode):
                expected = {**before, "capture_transport": mode}
                result = self.request("/api/control", {"capture_transport": mode})
                self.assertEqual(result["controls"], expected)
                self.assertEqual(self.request("/api/state")["controls"], expected)
                self.assertEqual(json.loads((self.data / "control.json").read_text()), expected)
                self.assertEqual(json.loads((self.data / "groups/one/state.json").read_text()), group)

    def test_legacy_ram_reserve_defaults_without_overriding_an_explicit_value(self):
        legacy = self.request("/api/state")["controls"]
        legacy.pop("system_reserve_mib")
        self.write("control.json", legacy)
        self.assertEqual(self.request("/api/state")["controls"]["system_reserve_mib"], 0)
        self.write("control.json", {**legacy, "system_reserve_mib": 4096})
        self.assertEqual(self.request("/api/state")["controls"]["system_reserve_mib"], 4096)
        result = self.request("/api/control", {"job_timeout_seconds": 1200})["controls"]
        self.assertEqual(result["system_reserve_mib"], 4096)
        self.assertEqual(result["job_timeout_seconds"], 1200)
        self.assertEqual(result["target_groups"], legacy["target_groups"])
        self.assertEqual(json.loads((self.data / "control.json").read_text()), result)

    def test_invalid_controls_cannot_partially_change_state(self):
        before = (self.data / "control.json").read_bytes()
        for patch in [
            {"target_groups": -1}, {"target_groups": 1.5}, {"paused": "true"},
            {"job_timeout_seconds": -1}, {"equal_preview_width": 513}, {"typo": 2},
            {"target_groups": 24, "equal_preview_width": None},
            {"hide_excel": "false"}, {"capture_tile_megapixels": 0}, {"capture_tile_megapixels": 65},
            {"capture_batch_tiles": 0}, {"capture_batch_tiles": 17}, {"capture_batch_tiles": 1.5},
            {"tall_sheet_threshold": 0}, {"tall_sheet_threshold": 1.5}, {"tall_sheet_threshold": 2147483648},
            {"tall_sheet_edge_tiles": 0}, {"tall_sheet_edge_tiles": -1}, {"tall_sheet_edge_tiles": "2"},
            {"system_reserve_mib": -1}, {"system_reserve_mib": 1.5},
            {"system_reserve_mib": 65537}, {"system_reserve_mib": 4294967296},
            {"system_reserve_mib": "512"}, {"system_reserve_mib": None}, {"system_reserve_mib": True},
            {"target_groups": 24, "system_reserve_mib": 65537},
            {"capture_transport": "hash-only"}, {"capture_transport": ""},
            {"capture_transport": True}, {"capture_transport": 1}, {"capture_transport": None},
            {"target_groups": 24, "capture_transport": "unknown"},
        ]:
            with self.subTest(patch=patch):
                self.assert_status(400, "/api/control", patch)
                self.assertEqual((self.data / "control.json").read_bytes(), before)
        self.assert_status(415, "/api/control", b"target_groups=24", {"Content-Type": "application/x-www-form-urlencoded"})
        self.assert_status(400, "/api/control", b"{", {"Content-Type": "application/json"})

    def test_zero_groups_drains_without_a_conflicting_capture_pause(self):
        self.request("/api/control", {"paused": True})
        drained = self.request("/api/control", {"target_groups": 0})["controls"]
        self.assertEqual(drained["target_groups"], 0)
        self.assertFalse(drained["paused"])
        self.assertEqual(self.request("/api/state")["controls"], drained)
        self.assert_status(400, "/api/control", {"paused": True})
        self.assert_status(400, "/api/control", {"target_groups": 0, "paused": True})
        self.assertFalse(self.request("/api/state")["controls"]["paused"])
        resumed = self.request("/api/control", {"target_groups": 3, "paused": True})["controls"]
        self.assertEqual(resumed["target_groups"], 3)
        self.assertTrue(resumed["paused"])

    def test_worker_overview_reads_compact_active_snapshots_and_separates_progress(self):
        self.write("queue.json", {"run_id": "run", "groups": [
            {"id": "active", "attempt_id": "new", "workbook": "active.xlsx", "active": True, "status": "waiting_cleanup"},
            {"id": "old", "run_id": "run", "workbook": "old.xlsx", "active": False, "status": "differences"},
        ]})
        # The central monitor must not parse a workbook's potentially huge plan.
        (self.data / "groups/active").mkdir(parents=True)
        (self.data / "groups/active/state.json").write_text("not valid JSON", encoding="utf-8")
        monitor = {"state": "capturing", "state_since": "2026-01-01T10:00:00Z",
                   "last_progress_at": "2026-01-01T10:01:00Z", "wait_reason": None,
                   "last_success": {"at": "2026-01-01T10:01:00Z", "kind": "image_received"},
                   "events": [{"sequence": n} for n in range(30)],
                   "active_job": {"id": "capture-1", "kind": "capture_images"}}
        self.write("groups/active/launcher.json", {"attempt_id": "new", "clients": {
            "original": {"pid": 42, "heartbeat_count": 999, "last_heartbeat_at": "2026-01-01T11:00:00Z",
                         "stage": "capturing", "monitor": monitor}}})
        self.write("groups/active/coordinator.json", {"attempt_id": "new", "phase": "compare", "pid": 12,
            "state_since_unix_ms": 123000, "last_progress_unix_ms": 124000,
            "last_event": {"at_unix_ms": 125000, "kind": "waiting"}})
        response = self.request("/api/workers")
        self.assertEqual(response["active_groups"], 1)
        self.assertEqual(response["total_groups"], 1)
        self.assertEqual(response["diagnostics"], [])
        group = response["groups"][0]
        self.assertEqual(group["coordinator"]["phase"], "compare")
        self.assertEqual(group["coordinator"]["state_since_unix_ms"], 123000)
        self.assertEqual(group["coordinator"]["last_progress_unix_ms"], 124000)
        worker = group["clients"][0]
        self.assertEqual(worker["monitor"]["last_progress_at"], monitor["last_progress_at"])
        self.assertEqual(worker["heartbeat_count"], 999)
        self.assertEqual(len(worker["monitor"]["events"]), 24)
        self.assertIn("active/new/original/42", worker["worker_id"])
        self.assertEqual(self.request("/api/workers?include_recent=true")["total_groups"], 2)
        self.write("groups/active/launcher.json", {"attempt_id": "old", "clients": {"original": {"pid": 42}}})
        self.assertEqual(self.request("/api/workers")["groups"][0]["clients"], [])

    def test_classic_workers_use_only_the_matching_attempt_and_not_resident_processes(self):
        self.write("queue.json", {"groups": [{"id": "pair", "attempt_id": "current", "active": True,
            "capture_mode": "classic", "status": "retrying_classic"}]})
        self.write("groups/pair/launcher.json", {"attempt_id": "current", "clients": {"stale": {"pid": 99}}})
        observation = {"attempt_id": "current", "client_reset_supported": False, "clients": {
            "original": {"pid": 42, "stage": "capturing", "heartbeat_count": 3,
                         "monitor": {"state": "capturing", "state_since": "2026-01-01T00:00:00Z",
                                     "last_progress_at": "2026-01-01T00:00:01Z", "events": []}}}}
        self.write("groups/pair/classic.json", observation)
        group = self.request("/api/workers")["groups"][0]
        self.assertEqual([client["client_id"] for client in group["clients"]], ["original"])
        self.assertEqual(group["clients"][0]["heartbeat_count"], 3)
        detail = self.request("/api/groups/pair")
        self.assertEqual(list(detail["launcher"]["clients"]), ["original"])
        self.assertFalse(detail["launcher"]["client_reset_supported"])
        observation["attempt_id"] = "previous"
        self.write("groups/pair/classic.json", observation)
        self.assertEqual(self.request("/api/workers")["groups"][0]["clients"], [])
        self.assertNotIn("launcher", self.request("/api/groups/pair"))

    def test_completed_worker_is_not_presented_as_waiting_for_cleanup(self):
        self.write("queue.json", {"run_id": "run", "open_groups": 1, "groups": [
            {"id": "done", "run_id": "run", "attempt_id": "new", "active": True,
             "status": "waiting_cleanup"}]})
        monitor = {"attempt_id": "new", "phase": "finished", "pid": 2147483647, "process_started_unix_ms": 1,
                   "last_event": {"kind": "record_saved", "details": {"status": "unchanged"}}}
        self.write("groups/done/coordinator.json", monitor)
        self.assertEqual(self.request("/api/workers")["active_groups"], 0)
        state = self.request("/api/state")
        self.assertEqual(state["open_groups"], 0)
        self.assertEqual(state["scheduler_open_groups"], 1)
        self.assertEqual(state["groups"][0]["status"], "unchanged")
        self.assertFalse(state["groups"][0]["active"])
        self.assertTrue(state["diagnostics"])
        self.assertEqual(self.request("/api/groups/done")["status"], "unchanged")
        # A previous attempt and a bare process exit prove no completed result.
        monitor["attempt_id"] = "old"
        self.write("groups/done/coordinator.json", monitor)
        self.assertEqual(self.request("/api/workers")["active_groups"], 1)
        monitor["attempt_id"] = "new"
        monitor["last_event"] = {"kind": "cleanup_wait"}
        self.write("groups/done/coordinator.json", monitor)
        self.assertEqual(self.request("/api/workers")["active_groups"], 1)

    def test_worker_pages_do_not_read_unbounded_group_history(self):
        self.write("queue.json", {"groups": [{"id": f"g{n}", "active": True} for n in range(70)] +
                                           [{"id": f"history{n}", "active": False} for n in range(1000)]})
        first = self.request("/api/workers")
        self.assertEqual(len(first["groups"]), 64)
        self.assertEqual(first["next_offset"], 64)
        last = self.request("/api/workers?offset=64")
        self.assertEqual(len(last["groups"]), 6)
        self.assertIsNone(last["next_offset"])
        self.assertEqual(last["total_groups"], 70)
        self.assert_status(400, "/api/workers?offset=-1")

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
        started = current_process_started_ms()
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

    @unittest.skipUnless(os.name == "nt", "Native Windows process creation identity")
    def test_group_measurements_include_baseline_and_available_profile_values(self):
        self.write('queue.json', {'groups':[{'id':'one'}]})
        path = self.data.parent / 'records' / 'one.json'
        path.parent.mkdir()
        path.write_text(json.dumps({'results':[
            {'profile':'default','original_bytes':1000,'shrunk_bytes':600,'original_open_seconds':10,
             'shrunk_open_seconds':4,'original_memory_bytes':500,'shrunk_memory_bytes':250,'shrink_seconds':2},
            {'profile':'maximum-cleaning','original_bytes':1000,'shrunk_bytes':400}]}))
        measurements = self.request('/api/groups/one')['measurements']
        self.assertEqual(measurements['original']['open_seconds'], 10)
        self.assertEqual(measurements['default']['bytes'], 600)
        self.assertEqual(measurements['default']['memory_bytes'], 250)
        self.assertIsNone(measurements['maximum-cleaning']['open_seconds'])

    def test_native_startup_samples_are_attempt_bound_and_preserve_com_measurements(self):
        self.write('queue.json', {'groups':[{'id':'one','attempt_id':'new'}]})
        path = self.data.parent / 'records' / 'one.json'
        path.parent.mkdir()
        path.write_text(json.dumps({'results':[{'profile':'default','original_bytes':1000,'shrunk_bytes':600,
            'original_open_seconds':10,'shrunk_open_seconds':4,'native_startup':{'method':'launch-to-addin-ready-v1',
                'attempt_id':'old','original':{'seconds':99},'shrunk':{'seconds':88}}}]}))
        sample = {'startup_measurement_method':'launch-to-addin-ready-v1','startup_seconds':3,
            'startup_memory_private_bytes':200,'startup_memory_working_set_bytes':100,
            'startup_sampled_unix_ms':2000,'ready_at':'2026-10-25T10:00:00Z',
            'pid':123,'process_started_unix_ms':1000}
        self.write('groups/one/state.json', {'attempt_id':'new','aliases':{'default':'original'}})
        self.write('groups/one/launcher.json', {'attempt_id':'new','clients':{'original':sample}})
        measured = self.request('/api/groups/one')['measurements']
        self.assertEqual(measured['default']['open_seconds'],4)
        self.assertEqual(measured['default']['startup']['seconds'],3)
        self.assertEqual(measured['default']['startup']['client_id'],'original')
        self.assertEqual(measured['original']['startup']['private_bytes'],200)
        self.write('groups/one/launcher.json', {'attempt_id':'old','clients':{'original':sample}})
        self.assertNotIn('startup', self.request('/api/groups/one')['measurements']['default'])

    def test_hard_reset_snapshots_visibility_and_target_process_or_requeues_closed_group(self):
        self.write('queue.json', {'run_id':'current','runner_pid':os.getpid(),
            'runner_started_unix_ms':current_process_started_ms(),'running':True,
            'groups':[{'id':'one','run_id':'current','attempt_id':'attempt','active':True,'status':'active'}]})
        self.write('groups/one/launcher.json', {'attempt_id':'attempt','status':'running','client_reset_supported':True,
            'clients':{'original':{'pid':123,'process_started_unix_ms':456,'closed':False}}})
        self.request('/api/control', {'hide_excel':False})
        self.assert_status(409,'/api/groups/one/restart', {'client_id':'original','hard_reset':True,'expected_attempt_id':'old'})
        result = self.request('/api/groups/one/restart', {'client_id':'original','hard_reset':True})
        self.assertEqual(result['restart_scope'], 'client')
        command = result['command']
        self.assertFalse(command['hide_excel'])
        self.assertEqual(command['expected_pid'], 123)
        self.assertEqual(command['expected_started_unix_ms'], 456)
        again = self.request('/api/groups/one/restart', {'client_id':'original','hard_reset':True})
        self.assertEqual(again['command']['id'], command['id'])
        self.assertEqual(len(list((self.data/'commands').glob('*.json'))),0)
        self.write('groups/one/launcher.json', {'attempt_id':'attempt','status':'complete',
            'clients':{'original':{'pid':123,'closed':True}}})
        self.assertEqual(self.request('/api/groups/one/restart', {'client_id':'original','hard_reset':True})['restart_scope'],'group')
        self.assert_status(400,'/api/groups/one/restart', {'hard_reset':True})

    def test_restart_is_durable_idempotent_and_scoped_to_the_running_group(self):
        self.write("queue.json", {
            "run_id": "run-current", "runner_pid": os.getpid(), "runner_started_unix_ms": current_process_started_ms(),
            "running": True, "groups": [
                {"id": "one", "run_id": "run-current", "attempt_id": "attempt-one", "status": "active", "active": True},
                {"id": "historical", "run_id": "run-old", "status": "complete"},
                {"id": "excluded", "run_id": "run-current", "restart_eligible": False, "status": "complete"},
            ],
        })
        self.write("groups/one/launcher.json", {"clients": {"original": {"status": "ready"}}})
        result = self.request("/api/groups/one/restart", {"client_id": "original", "reason": "Accidental edit"})
        command = result["command"]
        self.assertEqual(result["restart_scope"], "group")
        self.assertEqual(command["run_id"], "run-current")
        self.assertEqual(command["attempt_id"], "attempt-one")
        self.assertEqual(command["status"], "pending")
        self.assertEqual(command["client_id"], "original")
        self.assertEqual(self.request("/api/commands/" + command["id"]), command)
        self.assertEqual(self.request("/api/groups/one/restart", {})["command"]["id"], command["id"])
        self.assertEqual(self.request("/api/groups/one")["restart_commands"], [command])
        self.assertEqual(len(list((self.data / "commands").glob("*.json"))), 1)
        self.assert_status(404, "/api/groups/missing/restart", {})
        self.assert_status(404, "/api/groups/one/restart", {"client_id": "missing"})
        self.assert_status(409, "/api/groups/historical/restart", {})
        self.assert_status(409, "/api/groups/excluded/restart", {})
        self.assert_status(400, "/api/groups/one/restart", {"client_id": "../original"})
        self.assert_status(400, "/api/groups/one/restart", {"reason": "x" * 1001})
        self.assert_status(400, "/api/groups/one/restart", {"unknown": True})
        self.assert_status(403, "/api/groups/one/restart", {}, {"Origin": "https://example.com"})
        self.assert_status(415, "/api/groups/one/restart", b"restart=1", {"Content-Type": "text/plain"})
        command["status"] = "completed"
        self.write("commands/" + command["id"] + ".json", command)
        self.assertNotEqual(self.request("/api/groups/one/restart", {})["command"]["id"], command["id"])
        self.write("queue.json", {"running": False, "groups": [{"id": "one"}]})
        self.assert_status(409, "/api/groups/one/restart", {})

    @unittest.skipUnless(os.name == "nt", "Native Windows process creation identity")
    def test_process_liveness_is_separate_from_quiet_heartbeats(self):
        self.write("queue.json", {"groups": [{"id": "one", "active": True}]})
        clients = {
            "original": {"pid": os.getpid(), "process_started_unix_ms": current_process_started_ms(),
                         "heartbeat_count": 7, "last_heartbeat_at": "2000-01-01T00:00:00Z", "stage": "blocked_edit_mode"},
            "reused-pid": {"pid": os.getpid(), "process_started_unix_ms": current_process_started_ms() - 1000},
            "unverified": {"pid": os.getpid(), "process_alive": True},
        }
        self.write("groups/one/launcher.json", {"clients": clients})
        result = self.request("/api/groups/one")["launcher"]["clients"]
        self.assertTrue(result["original"]["process_alive"], "Quiet heartbeat falsely marked a living process dead")
        self.assertEqual(result["original"]["heartbeat_count"], 7)
        self.assertEqual(result["original"]["stage"], "blocked_edit_mode")
        self.assertFalse(result["reused-pid"]["process_alive"])
        self.assertIsNone(result["unverified"]["process_alive"])
        self.write("groups/one/coordinator.json", {"pid": os.getpid(), "phase": "shrink",
            "process_started_unix_ms": current_process_started_ms()})
        self.assertTrue(self.request("/api/workers")["groups"][0]["coordinator"]["process_alive"])
        self.write("groups/one/coordinator.json", {"pid": os.getpid(), "phase": "shrink",
            "process_started_unix_ms": current_process_started_ms() - 1000})
        self.assertFalse(self.request("/api/workers")["groups"][0]["coordinator"]["process_alive"])


if __name__ == "__main__":
    unittest.main(argv=[__file__, *REMAINING])
