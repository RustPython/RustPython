"""Check Windows event-log conversions without invoking host operations."""

import sys

from testutils import assert_raises

if sys.platform == "win32":
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
    assert_raises(
        TypeError, _winapi.ReportEvent, "invalid handle", 0, 0, 0, "message\0"
    )
    assert_raises(ValueError, _winapi.ReportEvent, 0, 0, 0, 0, "message\0")
    assert_raises(TypeError, _winapi.ReportEvent, 0, 0, 0, 0, None)
    for index, bits in ((1, 16), (2, 16), (3, 32)):
        for invalid in (-1, 1 << bits):
            arguments = [0, 0, 0, 0, "message\0"]
            arguments[index] = invalid
            assert_raises(OverflowError, _winapi.ReportEvent, *arguments)
        arguments = [0, 0, 0, 0, "message\0"]
        arguments[index] = "invalid integer"
        assert_raises(TypeError, _winapi.ReportEvent, *arguments)
