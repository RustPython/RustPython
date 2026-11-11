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


# Clinic binding must finish before module-specific id/config interpretation.
def check_interpreter_clinic():
    import inspect

    signatures = {
        "list_all": "(*, require_ready=False)",
        "create": "(config='isolated', *, reqrefs=False)",
        "destroy": "(id, *, restrict=False)",
        "is_running": "(id, *, restrict=False)",
        "whence": "(id)",
        "get_config": "(id, *, restrict=False)",
        "is_shareable": "(obj)",
        "exec": "(id, code, shared={}, *, restrict=False)",
        "run_string": "(id, script, shared={}, *, restrict=False)",
        "run_func": "(id, func, shared={}, *, restrict=False)",
        "call": "(id, callable, args=(), kwargs={}, *, preserve_exc=False, restrict=False)",
        "set___main___attrs": "(id, updates, *, restrict=False)",
        "incref": "(id, *, implieslink=False, restrict=False)",
        "decref": "(id, *, restrict=False)",
        "capture_exception": "(exc=None)",
    }
    for name, expected in signatures.items():
        func = getattr(_interpreters, name)
        assert str(inspect.signature(func)) == expected, name
        raw = " ".join(func.__text_signature__.split())
        assert raw == "($module, /, " + expected[1:], (name, raw)

    trace = []

    class Flag:
        def __init__(self, label, raises=False):
            self.label = label
            self.raises = raises

        def __bool__(self):
            trace.append(self.label)
            if self.raises:
                raise RuntimeError(self.label)
            return False

    class Id:
        def __index__(self):
            trace.append("id")
            return -1

        def __repr__(self):
            return "negative-id"

    def check_error(func, args, kwargs, error, message, expected_trace=()):
        trace.clear()
        try:
            func(*args, **kwargs)
        except error as exc:
            assert str(exc) == message, (func.__name__, str(exc), message)
        else:
            raise AssertionError(func.__name__)
        assert trace == list(expected_trace), (func.__name__, trace, expected_trace)

    for name, missing in [
        ("exec", "code"),
        ("run_string", "script"),
        ("run_func", "func"),
        ("call", "callable"),
        ("set___main___attrs", "updates"),
    ]:
        check_error(
            getattr(_interpreters, name),
            (),
            {"id": -1},
            TypeError,
            f"{name}() missing required argument '{missing}' (pos 2)",
        )

    # The unused preserve_exc flag still runs its Clinic bool converter, before id.
    check_error(
        _interpreters.call,
        (Id(), None),
        {"preserve_exc": Flag("preserve", True)},
        RuntimeError,
        "preserve",
        ["preserve"],
    )
    check_error(
        _interpreters.call,
        (Id(), None),
        {"preserve_exc": Flag("preserve"), "restrict": Flag("restrict", True)},
        RuntimeError,
        "restrict",
        ["preserve", "restrict"],
    )
    check_error(
        _interpreters.incref,
        (Id(),),
        {"implieslink": Flag("implieslink"), "restrict": Flag("restrict")},
        ValueError,
        "interpreter ID must be a non-negative int, got negative-id",
        ["implieslink", "restrict", "id"],
    )

    # Real typed arguments are checked in Clinic order, before later flags and id.
    for name, args, kwargs, parameter, expected_type in [
        ("exec", (Id(), None), {"shared": None}, "shared", "dict"),
        ("run_func", (Id(), None), {"shared": None}, "shared", "dict"),
        ("run_string", (Id(), None), {"shared": None}, "script", "str"),
        ("call", (Id(), None), {"args": None, "kwargs": None}, "args", "tuple"),
        ("call", (Id(), None), {"kwargs": None}, "kwargs", "dict"),
        ("set___main___attrs", (Id(), None), {}, "updates", "dict"),
    ]:
        kwargs["restrict"] = Flag("restrict", True)
        check_error(
            getattr(_interpreters, name),
            args,
            kwargs,
            TypeError,
            f"{name}() argument '{parameter}' must be {expected_type}, not None",
        )

    # Full keyword binding rejects duplicates/unknown names before conversion.
    check_error(
        _interpreters.call,
        (-1, None),
        {"id": 2, "preserve_exc": Flag("preserve", True)},
        TypeError,
        "argument for call() given by name ('id') and position (1)",
    )
    check_error(
        _interpreters.call,
        (-1, None),
        {"unexpected": 1, "preserve_exc": Flag("preserve", True)},
        TypeError,
        "call() got an unexpected keyword argument 'unexpected'",
    )

    # O! accepts subclasses without invoking user conversion protocols.
    class TupleSubclass(tuple):
        pass

    class DictSubclass(dict):
        pass

    class StrSubclass(str):
        pass

    for name, args, kwargs in [
        ("call", (-1, None), {"args": TupleSubclass(), "kwargs": DictSubclass()}),
        ("run_string", (-1, StrSubclass("pass")), {"shared": DictSubclass()}),
        ("exec", (-1, None), {"shared": DictSubclass()}),
        ("set___main___attrs", (-1, DictSubclass()), {}),
    ]:
        check_error(
            getattr(_interpreters, name),
            args,
            kwargs,
            ValueError,
            "interpreter ID must be a non-negative int, got -1",
        )
    assert _interpreters.capture_exception() is None
    assert _interpreters.capture_exception(None) is None
    assert _interpreters.is_shareable(obj=None)

    # The native wrapper keeps original owners, including failed conversions.
    class TemporaryFlag(Flag):
        def __del__(self):
            trace.append("del-" + self.label)

    for raises in (False, True):
        for mode in range(4):
            trace.clear()
            try:
                if mode == 0:
                    _interpreters.call(
                        Id(),
                        None,
                        preserve_exc=TemporaryFlag("preserve"),
                        restrict=TemporaryFlag("restrict", raises),
                    )
                elif mode == 1:
                    _interpreters.call(
                        Id(),
                        None,
                        restrict=TemporaryFlag("restrict", raises),
                        preserve_exc=TemporaryFlag("preserve"),
                    )
                elif mode == 2:
                    _interpreters.call(
                        Id(),
                        None,
                        **{
                            "preserve_exc": TemporaryFlag("preserve"),
                            "restrict": TemporaryFlag("restrict", raises),
                        },
                    )
                else:
                    _interpreters.call(
                        Id(),
                        None,
                        **{
                            "restrict": TemporaryFlag("restrict", raises),
                            "preserve_exc": TemporaryFlag("preserve"),
                        },
                    )
            except (ValueError, RuntimeError):
                pass
            else:
                raise AssertionError(mode)
            expected = ["preserve", "restrict"] + ([] if raises else ["id"])
            # Direct calls release the evaluation stack in reverse, whereas
            # expanded calls release the keyword mapping in insertion order.
            reverse = mode % 2 == 1
            if mode < 2:
                reverse = not reverse
            if raises:
                # The __bool__ traceback owns restrict until exception cleanup.
                reverse = False
            expected += (
                ["del-restrict", "del-preserve"]
                if reverse
                else ["del-preserve", "del-restrict"]
            )
            assert trace == expected, (mode, raises, trace)

    class TemporaryId(Id):
        def __del__(self):
            trace.append("del-id")

    class TemporaryValue:
        def __init__(self, name):
            self.name = name

        def __del__(self):
            trace.append("del-" + self.name)

    class TemporaryTuple(tuple):
        def __del__(self):
            trace.append("del-args")

    class TemporaryDict(dict):
        def __del__(self):
            trace.append("del-kwargs")

    # Positional values and borrowed payloads have the same owners as flags.
    # Binding failures must also release those owners without any conversion.
    for invoke, converted, destroyed in (
        (
            lambda: _interpreters.call(
                TemporaryId(),
                TemporaryValue("callable"),
                TemporaryTuple(),
                TemporaryDict(),
                preserve_exc=TemporaryFlag("preserve"),
                restrict=TemporaryFlag("restrict"),
            ),
            ["preserve", "restrict", "id"],
            ["restrict", "preserve", "kwargs", "args", "callable", "id"],
        ),
        (
            lambda: _interpreters.call(
                restrict=TemporaryFlag("restrict"),
                callable=TemporaryValue("callable"),
                id=TemporaryId(),
                kwargs=TemporaryDict(),
                preserve_exc=TemporaryFlag("preserve"),
                args=TemporaryTuple(),
            ),
            ["preserve", "restrict", "id"],
            ["args", "preserve", "kwargs", "id", "callable", "restrict"],
        ),
        (
            lambda: _interpreters.call(
                TemporaryId(),
                TemporaryValue("callable"),
                TemporaryTuple(),
                TemporaryDict(),
                **dict(
                    restrict=TemporaryFlag("restrict"),
                    preserve_exc=TemporaryFlag("preserve"),
                ),
            ),
            ["preserve", "restrict", "id"],
            ["restrict", "preserve", "kwargs", "args", "callable", "id"],
        ),
        (
            lambda: _interpreters.call(
                **dict(
                    id=TemporaryId(),
                    callable=TemporaryValue("callable"),
                    args=TemporaryTuple(),
                    kwargs=TemporaryDict(),
                    preserve_exc=TemporaryFlag("preserve"),
                    restrict=TemporaryFlag("restrict"),
                )
            ),
            ["preserve", "restrict", "id"],
            ["id", "callable", "args", "kwargs", "preserve", "restrict"],
        ),
        (
            lambda: _interpreters.call(
                TemporaryId(),
                TemporaryValue("callable"),
                preserve_exc=TemporaryFlag("preserve"),
                restrict=TemporaryFlag("restrict"),
                unexpected=TemporaryValue("unexpected"),
            ),
            [],
            ["unexpected", "restrict", "preserve", "callable", "id"],
        ),
        (
            lambda: _interpreters.call(
                TemporaryId(),
                TemporaryValue("callable"),
                **dict(
                    preserve_exc=TemporaryFlag("preserve"),
                    restrict=TemporaryFlag("restrict"),
                    unexpected=TemporaryValue("unexpected"),
                ),
            ),
            [],
            ["preserve", "restrict", "unexpected", "callable", "id"],
        ),
    ):
        for _ in range(32):
            trace.clear()
            try:
                invoke()
            except (TypeError, ValueError):
                pass
            else:
                raise AssertionError("invalid call succeeded")
            assert trace == converted + ["del-" + name for name in destroyed], trace

    class TemporaryCurrentId(TemporaryId):
        def __index__(self):
            trace.append("id")
            return _interpreters.get_current()[0]

    for invoke, destroyed in (
        (
            lambda: _interpreters.is_running(
                TemporaryCurrentId(), restrict=TemporaryFlag("restrict")
            ),
            ["restrict", "id"],
        ),
        (
            lambda: _interpreters.is_running(
                **dict(id=TemporaryCurrentId(), restrict=TemporaryFlag("restrict"))
            ),
            ["id", "restrict"],
        ),
    ):
        trace.clear()
        assert isinstance(invoke(), bool)
        assert trace == ["restrict", "id"] + ["del-" + name for name in destroyed]

    check_error(
        _interpreters.call,
        (-1, None),
        {"args": None, "unexpected": 1},
        TypeError,
        "call() got an unexpected keyword argument 'unexpected'",
    )


check_interpreter_clinic()
