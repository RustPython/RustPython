import sys
import time

from testutils import assert_raises

# The watchdog's thread cap is keyword-only and uses Py_ssize_t conversion.
# Its output must distinguish an exact cap from a truncated thread dump.
if sys.platform not in ("wasi", "emscripten"):
    import faulthandler

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
