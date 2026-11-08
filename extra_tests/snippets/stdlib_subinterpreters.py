import subprocess
import sys
import tempfile
from pathlib import Path

import _interpchannels
import _interpreters


def roundtrip_buffer(view):
    cid = _interpchannels.create(3)
    try:
        _interpchannels.send_buffer(cid, view, blocking=False)
        received, unbound = _interpchannels.recv(cid)
        assert unbound is None
        return received
    finally:
        _interpchannels.destroy(cid)


data = bytearray(range(12))
for view in (memoryview(data)[2:10:2], memoryview(data).cast("I")):
    received = roundtrip_buffer(view)
    assert received.tolist() == view.tolist()
    assert received.shape == view.shape
    assert received.strides == view.strides
    assert received.format == view.format
    assert received.itemsize == view.itemsize


# A subinterpreter's finalizing thread need not be the process main thread.
# Keep a regression bounded: the old eval-breaker parked this worker forever.
subprocess.run(
    [
        sys.executable,
        "-c",
        """
from concurrent import interpreters
from threading import Thread

completed = []

def worker():
    interp = interpreters.create()
    interp.exec('''
import gc, weakref
gc.disable()
class C:
    pass
item = C()
item.cycle = item
ref = weakref.ref(item, lambda _: None)
del item
''')
    interp.close()
    completed.append(True)

thread = Thread(target=worker)
thread.start()
thread.join()
assert completed == [True]
""",
    ],
    check=True,
    timeout=30,
)


# Script globals persist in the isolated copy, without replacing worker __main__.
with tempfile.TemporaryDirectory() as directory:
    script = Path(directory) / "pool_main.py"
    script.write_text(
        """
from concurrent.futures import InterpreterPoolExecutor

counter = 0

def increment():
    global counter
    import __main__
    counter += 1
    return counter, hasattr(__main__, 'increment')

def inspect_state():
    import __main__
    return counter, hasattr(__main__, 'increment')

def consume(queue):
    from threading import Barrier, Thread
    barrier = Barrier(2)
    results = []
    def receive():
        barrier.wait()
        results.append(queue.get()())
    threads = [Thread(target=receive) for _ in range(2)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    return tuple(results)

if __name__ == '__main__':
    from concurrent import interpreters
    queue = interpreters.create_queue()
    queue.put(inspect_state)
    queue.put(inspect_state)
    interp = interpreters.create()
    try:
        assert interp.call(consume, queue) == ((0, False), (0, False))
    finally:
        interp.close()
    with InterpreterPoolExecutor(max_workers=1) as pool:
        assert pool.submit(increment).result() == (1, False)
        assert pool.submit(increment).result() == (2, False)
    assert counter == 0
""",
        encoding="utf-8",
    )
    subprocess.run([sys.executable, str(script)], check=True, timeout=30)


# Releasing queued buffers can run a finalizer which reenters the channel API.
subprocess.run(
    [
        sys.executable,
        "-c",
        """
import _interpchannels as channels
import _interpreters
import sys

released = []

class Exporter(bytearray):
    def __del__(self):
        channels.list_all()
        released.append(True)

for operation in ("destroy", "close", "drop", "timeout"):
    # CPython 3.14.6 also deadlocks when close/drop releases a reentrant exporter.
    if sys.implementation.name == "cpython" and operation in ("close", "drop"):
        continue
    channel = channels.create(3)
    released.clear()
    if operation == "timeout":
        try:
            channels.send_buffer(channel, Exporter(b"data"), timeout=0)
        except TimeoutError:
            pass
        else:
            raise AssertionError("send must time out without a receiver")
    else:
        channels.send_buffer(channel, Exporter(b"data"), blocking=False)
        assert not released
        if operation == "destroy":
            channels.destroy(channel)
        elif operation == "close":
            channels.close(channel, force=True)
        else:
            del channel
    assert released == [True], operation
""",
    ],
    check=True,
    timeout=30,
)


def check_preserve_exc_conversion():
    trace = []

    class Flag:
        def __init__(self, name, raises=False):
            self.name = name
            self.raises = raises

        def __bool__(self):
            trace.append(self.name)
            if self.raises:
                raise RuntimeError(self.name)
            return False

        def __del__(self):
            trace.append("del-" + self.name)

    class Id:
        def __index__(self):
            trace.append("id")
            return -1

    # Conversion is required even when preserve_exc's value is unused. It must
    # precede restrict conversion and the implementation's id lookup.
    try:
        _interpreters.call(
            Id(),
            None,
            preserve_exc=Flag("preserve", True),
            restrict=Flag("restrict", True),
        )
    except RuntimeError as exc:
        assert str(exc) == "preserve"
    else:
        raise AssertionError("preserve_exc was not converted")
    assert [event for event in trace if not event.startswith("del-")] == ["preserve"]

    trace.clear()
    try:
        _interpreters.call(
            Id(), None, preserve_exc=Flag("preserve"), restrict=Flag("restrict", True)
        )
    except RuntimeError as exc:
        assert str(exc) == "restrict"
    else:
        raise AssertionError("restrict was not converted")
    assert trace[:2] == ["preserve", "restrict"], trace

    # The original kwargs ownership must survive conversion. Both **dict
    # insertion orders retain the original inputs through id resolution.
    for reverse in (False, True):
        trace.clear()
        try:
            if reverse:
                _interpreters.call(
                    Id(),
                    None,
                    **{"restrict": Flag("restrict"), "preserve_exc": Flag("preserve")},
                )
            else:
                _interpreters.call(
                    Id(),
                    None,
                    **{"preserve_exc": Flag("preserve"), "restrict": Flag("restrict")},
                )
        except ValueError:
            pass
        else:
            raise AssertionError("negative id was accepted")
        cleanup = (
            ["del-restrict", "del-preserve"]
            if reverse
            else ["del-preserve", "del-restrict"]
        )
        assert trace == ["preserve", "restrict", "id"] + cleanup, trace

    # Existing tuple/dict conversions still take precedence over the flags.
    for bad in ({"args": None}, {"kwargs": None}):
        trace.clear()
        try:
            _interpreters.call(Id(), None, preserve_exc=Flag("preserve", True), **bad)
        except TypeError:
            pass
        else:
            raise AssertionError("invalid call arguments were accepted")
        assert [event for event in trace if not event.startswith("del-")] == [], trace


check_preserve_exc_conversion()
