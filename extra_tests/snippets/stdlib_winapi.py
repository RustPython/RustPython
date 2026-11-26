"""Check Windows API bindings with owned handles and temporary files."""

import sys

from testutils import assert_raises

if sys.platform != "win32":
    sys.exit(0)

import _winapi

# Every call fails before a host operation. ReportEvent overflow cases
# also retain an embedded NUL as an independent safety barrier.
assert _winapi.EVENTLOG_SUCCESS == 0
assert _winapi.EVENTLOG_ERROR_TYPE == 1
assert _winapi.EVENTLOG_WARNING_TYPE == 2
assert _winapi.EVENTLOG_INFORMATION_TYPE == 4
assert _winapi.EVENTLOG_AUDIT_SUCCESS == 8
assert _winapi.EVENTLOG_AUDIT_FAILURE == 16

assert_raises(TypeError, _winapi.RegisterEventSource, 1, "source\0")
assert_raises(TypeError, _winapi.RegisterEventSource, None, 1)
assert_raises(TypeError, _winapi.RegisterEventSource, None, None)
assert_raises(ValueError, _winapi.RegisterEventSource, None, "source\0")
assert_raises(ValueError, _winapi.RegisterEventSource, "server\0", "source\0")
assert_raises(ValueError, _winapi.RegisterEventSource, "", "source\0")
assert_raises(TypeError, _winapi.DeregisterEventSource, "invalid handle")
assert_raises(TypeError, _winapi.DeregisterEventSource, None)
assert_raises(TypeError, _winapi.ReportEvent, "invalid handle", 0, 0, 0, "message\0")
assert_raises(ValueError, _winapi.ReportEvent, 0, 0, 0, 0, "message\0")
assert_raises(TypeError, _winapi.ReportEvent, 0, 0, 0, 0, None)
for index, bits in ((1, 16), (2, 16), (3, 32)):
    for invalid, exception in ((-1, ValueError), (1 << bits, OverflowError)):
        arguments = [0, 0, 0, 0, "message\0"]
        arguments[index] = invalid
        assert_raises(exception, _winapi.ReportEvent, *arguments)
    arguments = [0, 0, 0, 0, "message\0"]
    arguments[index] = "invalid integer"
    assert_raises(TypeError, _winapi.ReportEvent, *arguments)


import subprocess


class PartialStartupInfo:
    dwFlags = 0

    def __getattr__(self, name):
        raise ValueError(name)


for startup in (None, PartialStartupInfo()):
    process, thread, _, _ = _winapi.CreateProcess(
        sys.executable,
        subprocess.list2cmdline([sys.executable, "-c", "pass"]),
        None,
        None,
        False,
        _winapi.CREATE_NO_WINDOW,
        None,
        None,
        startup,
    )
    try:
        status = _winapi.WaitForSingleObject(process, 10000)
        if status != _winapi.WAIT_OBJECT_0:
            _winapi.TerminateProcess(process, 1)
            _winapi.WaitForSingleObject(process, 10000)
        assert status == _winapi.WAIT_OBJECT_0
        assert _winapi.GetExitCodeProcess(process) == 0
    finally:
        _winapi.CloseHandle(thread)
        _winapi.CloseHandle(process)

startup = subprocess.STARTUPINFO()
startup.dwFlags = "invalid"
assert_raises(
    TypeError,
    _winapi.CreateProcess,
    None,
    "invalid\0",
    None,
    None,
    False,
    0,
    None,
    None,
    startup,
)


# These calls must bind keywords to the real native operations, not just
# advertise a matching signature. Use only resources created in this test.
import msvcrt
import tempfile

event = _winapi.CreateEventW(0, 1, 0, None)
try:
    assert _winapi.WaitForSingleObject(event, 0) == _winapi.WAIT_TIMEOUT
    assert _winapi.SetEvent(event=event) is None
    assert _winapi.WaitForSingleObject(event, 0) == _winapi.WAIT_OBJECT_0
    assert _winapi.BatchedWaitForMultipleObjects(
        handle_seq=[event], wait_all=False, milliseconds=100
    ) == [0]
    assert _winapi.ResetEvent(event=event) is None
    assert _winapi.WaitForSingleObject(event, 0) == _winapi.WAIT_TIMEOUT
finally:
    _winapi.CloseHandle(event)

mutex = _winapi.CreateMutexW(0, 1, None)
try:
    assert _winapi.ReleaseMutex(mutex=mutex) is None
finally:
    _winapi.CloseHandle(mutex)

with tempfile.TemporaryFile() as stream:
    handle = msvcrt.get_osfhandle(stream.fileno())
    assert _winapi.GetFileType(handle=handle) == _winapi.GetFileType(handle)
