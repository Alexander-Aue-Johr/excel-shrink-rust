"""Measure one experimental child process without high-frequency disk logging.

Windows CPU totals come from GetProcessTimes. Memory observations use
GetProcessMemoryInfo, retaining OS peak counters and sampled private usage.
Measurements cover the direct child, not a descendant process tree.
"""

import ctypes
from ctypes import wintypes
import json
import os
import subprocess
import time


class MemoryCounters(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD)] + [
        (name, ctypes.c_size_t) for name in (
            "PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage",
            "QuotaPagedPoolUsage", "QuotaPeakNonPagedPoolUsage", "QuotaNonPagedPoolUsage",
            "PagefileUsage", "PeakPagefileUsage", "PrivateUsage",
        )
    ]


class WindowsProbe:
    """Hold the child's exact process object across exit and PID reuse."""

    def __init__(self, pid):
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        self.kernel.OpenProcess.restype = wintypes.HANDLE
        self.kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        self.kernel.CloseHandle.restype = wintypes.BOOL
        self.kernel.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
        self.kernel.GetProcessTimes.restype = wintypes.BOOL
        self.memory_info = ctypes.WinDLL("psapi", use_last_error=True).GetProcessMemoryInfo
        self.memory_info.argtypes = [wintypes.HANDLE, ctypes.POINTER(MemoryCounters), wintypes.DWORD]
        self.memory_info.restype = wintypes.BOOL
        self.handle = self.kernel.OpenProcess(0x0400 | 0x0010, False, pid)
        if not self.handle:
            raise ctypes.WinError(ctypes.get_last_error())
        self.samples = 0
        self.peak_working_set = 0
        self.peak_pagefile = 0
        self.sampled_peak_private = 0

    def sample(self):
        counters = MemoryCounters()
        counters.cb = ctypes.sizeof(counters)
        if not self.memory_info(self.handle, ctypes.byref(counters), counters.cb):
            return False
        self.samples += 1
        self.peak_working_set = max(self.peak_working_set, counters.PeakWorkingSetSize)
        self.peak_pagefile = max(self.peak_pagefile, counters.PeakPagefileUsage)
        self.sampled_peak_private = max(self.sampled_peak_private, counters.PrivateUsage)
        return True

    def finish(self):
        creation, exit_time, kernel, user = (wintypes.FILETIME() for _ in range(4))
        if not self.kernel.GetProcessTimes(self.handle, ctypes.byref(creation), ctypes.byref(exit_time),
                                           ctypes.byref(kernel), ctypes.byref(user)):
            raise ctypes.WinError(ctypes.get_last_error())
        sampled_after_exit = self.sample()
        return {
            "status": "measured", "scope": "direct_child_process",
            "cpu_user_seconds": filetime_seconds(user), "cpu_kernel_seconds": filetime_seconds(kernel),
            "observed_os_peak_working_set_bytes": self.peak_working_set if self.samples else None,
            "observed_os_peak_pagefile_usage_bytes": self.peak_pagefile if self.samples else None,
            "sampled_peak_private_bytes": self.sampled_peak_private if self.samples else None,
            "memory_samples": self.samples, "memory_query_after_exit_succeeded": sampled_after_exit,
            "sampling_interval_seconds": 0.01,
            "memory_limit": "Maximum observed OS peaks; an unsampled final peak may be missed. Private usage is sampled, not an exact lifetime maximum.",
        }

    def close(self):
        self.kernel.CloseHandle(self.handle)


def filetime_seconds(value):
    ticks = (value.dwHighDateTime << 32) | value.dwLowDateTime
    return ticks / 10_000_000


def run_measured(command, directory, timeout):
    """Retain a bounded trial receipt before propagating exit or timeout failure."""
    probe = None
    resources = {"status": "unavailable", "reason": "Native process counters require Windows"}
    started = time.perf_counter()
    timed_out = False
    with (directory / "stdout.log").open("wb") as stdout, (directory / "stderr.log").open("wb") as stderr:
        with subprocess.Popen(command, stdout=stdout, stderr=stderr) as process:
            try:
                if os.name == "nt":
                    try:
                        probe = WindowsProbe(process.pid)
                    except OSError as error:
                        resources = {"status": "unavailable", "reason": str(error)}
                while process.poll() is None:
                    if probe:
                        probe.sample()
                    if time.perf_counter() - started >= timeout:
                        timed_out = True
                        process.kill()
                        break
                    try:
                        process.wait(timeout=0.01)
                    except subprocess.TimeoutExpired:
                        pass
                process.wait()
                if probe:
                    try:
                        resources = probe.finish()
                    except OSError as error:
                        resources = {"status": "unavailable", "reason": str(error)}
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
                if probe:
                    probe.close()
    result = {"seconds": time.perf_counter() - started, "exit_code": process.returncode,
              "timed_out": timed_out, "resources": resources}
    (directory / "process-metrics.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    if timed_out:
        raise subprocess.TimeoutExpired(command, timeout)
    if process.returncode:
        raise subprocess.CalledProcessError(process.returncode, command)
    return result
