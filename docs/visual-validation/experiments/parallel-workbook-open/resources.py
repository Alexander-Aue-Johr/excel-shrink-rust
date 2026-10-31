"""Read Windows memory availability and CPU usage of experiment-owned processes."""
import ctypes
from ctypes import wintypes
import platform
import winreg

kernel = ctypes.WinDLL('kernel32', use_last_error=True)
kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
kernel.OpenProcess.restype = wintypes.HANDLE
kernel.CloseHandle.argtypes = [wintypes.HANDLE]
kernel.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4


class MemoryStatus(ctypes.Structure):
    _fields_ = [('length', wintypes.DWORD), ('load', wintypes.DWORD)] + [
        (name, ctypes.c_ulonglong) for name in (
            'total_physical', 'available_physical', 'total_page_file', 'available_page_file',
            'total_virtual', 'available_virtual', 'available_extended')]


def memory_status():
    status = MemoryStatus()
    status.length = ctypes.sizeof(status)
    if not kernel.GlobalMemoryStatusEx(ctypes.byref(status)):
        raise ctypes.WinError(ctypes.get_last_error())
    return status


def available_memory():
    return memory_status().available_physical


def machine_info():
    with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, r'HARDWARE\DESCRIPTION\System\CentralProcessor\0') as key:
        cpu = winreg.QueryValueEx(key, 'ProcessorNameString')[0].strip()
    return {'cpu': cpu, 'os': platform.platform(), 'ram_bytes': memory_status().total_physical}


def process_cpu_seconds(pid):
    """Query one owned PID. Return None when the process cannot be queried."""
    handle = kernel.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
    if not handle:
        return None
    try:
        times = [wintypes.FILETIME() for _ in range(4)]
        if not kernel.GetProcessTimes(handle, *(ctypes.byref(value) for value in times)):
            return None
        ticks = [(value.dwHighDateTime << 32) | value.dwLowDateTime for value in times]
        return (ticks[2] + ticks[3]) / 10_000_000
    finally:
        kernel.CloseHandle(handle)
