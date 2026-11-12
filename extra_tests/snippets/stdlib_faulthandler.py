import faulthandler
import signal
import struct
import sys
import time

from testutils import assert_raises

# Faulthandler watchdogs use the same signed-nanosecond conversion as sleep.


def rejected_timeout(expected, message, timeout, **kwargs):
    try:
        with assert_raises(expected) as caught:
            faulthandler.dump_traceback_later(timeout, **kwargs)
        assert str(caught.exception) == message
    finally:
        faulthandler.cancel_dump_traceback_later()


# Keyword binding must reach timeout validation before inspecting the file.
try:
    with assert_raises(ValueError) as caught:
        faulthandler.dump_traceback_later(timeout=0, file=-1)
    assert str(caught.exception) == "timeout must be greater than 0"
    assert_raises(TypeError, faulthandler.dump_traceback_later)
    assert_raises(TypeError, faulthandler.dump_traceback_later, 60, timeout=60)
    assert_raises(TypeError, faulthandler.dump_traceback_later, 60, False, -1, False, 1)
finally:
    faulthandler.cancel_dump_traceback_later()


# All probes below reject the file before arming a watchdog, including tiny
# positive durations that could expire before a subsequent cancellation.
file_error = "file is not a valid file descriptor"
for timeout in (1e-20, 5e-324, 1e-9):
    rejected_timeout(ValueError, file_error, timeout, file=-1)
for timeout in (0, -0.0, -1, -1e-20, -5e-324):
    rejected_timeout(ValueError, "timeout must be greater than 0", timeout, file=-1)
rejected_timeout(ValueError, "Invalid value NaN (not a number)", float("nan"), file=-1)
for timeout in (float("inf"), -float("inf"), 10**100, -(10**100), 9223372037):
    rejected_timeout(
        OverflowError, "timestamp out of range for C PyTime_t", timeout, file=-1
    )
for timeout in (None, "60", object()):
    rejected_timeout(
        TypeError,
        "must be real number, not " + type(timeout).__name__,
        timeout,
        file=-1,
    )


calls = []


class TruthValue:
    def __init__(self, name, error=None):
        self.name = name
        self.error = error

    def __bool__(self):
        calls.append(self.name)
        if self.error is not None:
            raise self.error
        return False


class IndexedTimeout:
    def __index__(self):
        calls.append("index")
        return 60

    def __float__(self):
        raise AssertionError("timeout must prefer __index__")


class FloatTimeout:
    def __float__(self):
        calls.append("float")
        return 60.0


class InvalidFile:
    def fileno(self):
        calls.append("fileno")
        return -1

    def flush(self):
        raise AssertionError("invalid descriptor must not be flushed")


for timeout, conversion in ((IndexedTimeout(), "index"), (FloatTimeout(), "float")):
    calls.clear()
    rejected_timeout(
        RuntimeError,
        "file.fileno() is not a valid file descriptor",
        timeout,
        repeat=TruthValue("repeat"),
        exit=TruthValue("exit"),
        file=InvalidFile(),
    )
    assert calls == ["repeat", "exit", conversion, "fileno"]

for failing_argument in ("repeat", "exit"):
    calls.clear()
    failure = RuntimeError(failing_argument + " failed")
    rejected_timeout(
        RuntimeError,
        str(failure),
        None,
        repeat=TruthValue("repeat", failure if failing_argument == "repeat" else None),
        exit=TruthValue("exit", failure if failing_argument == "exit" else None),
        file=InvalidFile(),
    )
    assert calls == (["repeat"] if failing_argument == "repeat" else ["repeat", "exit"])

for timeout, expected, message in (
    (0, ValueError, "timeout must be greater than 0"),
    (float("nan"), ValueError, "Invalid value NaN (not a number)"),
    (float("inf"), OverflowError, "timestamp out of range for C PyTime_t"),
):
    calls.clear()
    rejected_timeout(
        expected,
        message,
        timeout,
        repeat=TruthValue("repeat"),
        exit=TruthValue("exit"),
        file=InvalidFile(),
    )
    assert calls == ["repeat", "exit"]


class FailingIndex:
    def __index__(self):
        raise RuntimeError("index failed")

    def __float__(self):
        raise AssertionError("failed index must not fall back to float")


class FailingFloat:
    def __float__(self):
        raise RuntimeError("float failed")


class InvalidIndex:
    def __index__(self):
        return 60.0

    def __float__(self):
        raise AssertionError("invalid index must not fall back to float")


rejected_timeout(RuntimeError, "index failed", FailingIndex(), file=-1)
rejected_timeout(RuntimeError, "float failed", FailingFloat(), file=-1)
try:
    assert_raises(TypeError, faulthandler.dump_traceback_later, InvalidIndex(), file=-1)
finally:
    faulthandler.cancel_dump_traceback_later()


class FloatSubclass(float):
    def __float__(self):
        raise AssertionError("float subclass override must be bypassed")


class IndexedFloatSubclass(FloatSubclass):
    def __index__(self):
        return -1


rejected_timeout(ValueError, file_error, FloatSubclass(60), file=-1)
rejected_timeout(
    ValueError, "timeout must be greater than 0", IndexedFloatSubclass(60), file=-1
)

# The platform limit and the signed C long seconds limit precede file lookup.
# This is below the signed-nanosecond limit on every tested platform.
if sys.platform == "win32":
    rejected_timeout(OverflowError, "timeout value is too large", 4_294_968, file=-1)
elif struct.calcsize("l") == 4:
    rejected_timeout(OverflowError, "timeout value is too large", 2**31, file=-1)
else:
    rejected_timeout(ValueError, file_error, 9223372036, file=-1)

# Exercise the successful keyword path only with a long timeout, immediately
# cancelled. Wasm targets do not provide the native watchdog thread.
if sys.platform not in ("wasi", "emscripten"):
    try:
        assert faulthandler.dump_traceback_later(timeout=60) is None
    finally:
        faulthandler.cancel_dump_traceback_later()


# An invalid file proves register's signum keyword binds without installing
# any signal handler. This API is absent on Windows.
if hasattr(faulthandler, "register") and hasattr(signal, "SIGUSR1"):
    with assert_raises(ValueError) as caught:
        faulthandler.register(signum=signal.SIGUSR1, file=-1)
    assert str(caught.exception) == "file is not a valid file descriptor"


# The watchdog's thread cap is keyword-only and uses Py_ssize_t conversion.
# Its output must distinguish an exact cap from a truncated thread dump.
if sys.platform not in ("wasi", "emscripten"):
    for cap in (None, 1.5):
        assert_raises(TypeError, faulthandler.dump_traceback_later, 60, max_threads=cap)
    for cap in (sys.maxsize + 1, -sys.maxsize - 2):
        assert_raises(
            OverflowError, faulthandler.dump_traceback_later, 60, max_threads=cap
        )
    assert_raises(
        TypeError, faulthandler.dump_traceback_later, 60, False, None, False, 1
    )

    try:
        import _thread
    except ModuleNotFoundError as exc:
        if exc.name != "_thread":
            raise
    else:
        import tempfile
        import threading

        ready = threading.Event()
        stop = threading.Event()

        def watchdog_worker():
            ready.set()
            stop.wait()

        watchdog_thread = threading.Thread(target=watchdog_worker)
        watchdog_thread.start()
        ready.wait()
        try:
            for cap, expected_threads, truncated in (
                (-1, 0, True),
                (0, 2, False),
                (1, 1, True),
                (2, 2, False),
                (3, 2, False),
            ):
                with tempfile.TemporaryFile() as output:
                    faulthandler.dump_traceback_later(
                        0.01, file=output, max_threads=cap
                    )
                    time.sleep(0.5)
                    faulthandler.cancel_dump_traceback_later()
                    output.seek(0)
                    traceback = output.read()
                    assert traceback.count(b"Thread 0x") == expected_threads, traceback
                    assert (b"\n...\n" in traceback) == truncated, traceback
        finally:
            faulthandler.cancel_dump_traceback_later()
            stop.set()
            watchdog_thread.join()
