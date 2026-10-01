import subprocess
import sys
import tempfile
from pathlib import Path

import _interpchannels
import _interpreters

# Releasing an exported buffer can reenter the channel directory. Keep deadlock
# regressions in a subprocess so a locked directory cannot stall the whole suite.
operations = ["destroy", "timeout"]
if sys.implementation.name == "rustpython":
    # CPython 3.14 also deadlocks on these two release paths.
    operations += ["close", "drop"]
for operation in operations:
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

channel = channels.create(3)
operation = sys.argv[1]
if operation == 'timeout':
    try:
        channels.send_buffer(channel, Exporter(b'data'), timeout=0)
    except TimeoutError:
        pass
    else:
        raise AssertionError('send should time out')
else:
    channels.send_buffer(channel, Exporter(b'data'), blocking=False)
    assert not released
    if operation == 'destroy':
        channels.destroy(channel)
    elif operation == 'close':
        channels.close(channel, force=True)
    elif operation == 'drop':
        del channel
assert released == [True], released
""",
            operation,
        ],
        check=True,
        timeout=15,
    )


def roundtrip_buffer(view):
    cid = _interpchannels.create(3)
    try:
        _interpchannels.send_buffer(cid, view, blocking=False)
        received, unbound = _interpchannels.recv(cid)
        assert unbound is None
        return received
    finally:
        _interpchannels.destroy(cid)


if sys.implementation.name == "rustpython":
    # CPython also retains this cycle. Returning an export to its owner lets
    # RustPython expose the local exporter edge to the ordinary cycle collector.
    import gc
    import weakref

    class BufferOwner(bytearray):
        pass

    owner = BufferOwner(b"abcd")
    owner_ref = weakref.ref(owner)
    owner.view = roundtrip_buffer(memoryview(owner))
    del owner
    gc.collect()
    assert owner_ref() is None


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


# Native caches and weakref registries belong to an interpreter even when
# their Rust definitions or immutable referents are shared.
import _json
import _thread
import abc
import csv
import io
import os
import struct
import time
import weakref
from collections.abc import Sequence
from concurrent import interpreters

# A receiving view shares writable native storage without keeping a foreign
# Python payload usable. The bytes also outlive the sending interpreter.
interp = interpreters.create()
channel = _interpchannels.create(3)
try:
    interp.exec(f"""
import array, io, mmap, _interpchannels
backings = [bytearray(b'abcd'), array.array('B', b'abcd'),
            io.BytesIO(b'abcd'), mmap.mmap(-1, 4)]
backings[-1][:] = b'abcd'
views = [memoryview(backings[0]), memoryview(backings[1]),
         backings[2].getbuffer(), memoryview(backings[3])]
for view in views:
    _interpchannels.send_buffer({int(channel)}, view, blocking=False)
""")
    received_views = [_interpchannels.recv(channel)[0] for _ in range(4)]
    for received in received_views:
        received[1] = ord("Z")
    interp.exec("assert all(view.tobytes() == b'aZcd' for view in views)")
finally:
    interp.close()
    _interpchannels.destroy(channel)
for received in received_views:
    assert received.tobytes() == b"aZcd"
    alias = roundtrip_buffer(received)
    assert alias == received
    alias[1:] = received[:-1]
    assert received.tobytes() == b"aaZc"
    alias[:] = b"aZcd"
    alias.release()
    received[2] = ord("Y")
    assert received.tobytes() == b"aZYd"
    received.release()


def isolation_callback(ref):
    pass


isolation_callback._gc_owner_probe = True
owned_ref = weakref.ref(object, isolation_callback)
basic_ref = weakref.ref(object)
private_types = (
    csv.Error,
    io.UnsupportedOperation,
    struct.error,
    ExceptionGroup,
    _json.make_scanner,
    _json.make_encoder,
    _thread._ExceptHookArgs,
    time.struct_time,
    os.stat_result,
)
for typ in private_types:
    typ._gc_owner_probe = []


class PrivateSubclass:
    _gc_owner_probe = True


# Even immutable native classes contain mutable Python values. In particular,
# the bound __new__ function permits arbitrary values in its __module__ field.
saved_new_module = list.__new__.__module__
list.__new__.__module__ = ["main"]


interp = interpreters.create()
try:
    saved_abc_token = abc.get_cache_token()
    saved_int_flags = int.__flags__
    saved_int_sequence = issubclass(int, Sequence)
    saved_object_flags = object.__flags__
    saved_csv_limit = csv.field_size_limit()
    csv.register_dialect("_gc_main_dialect", delimiter="|")
    interp.exec("""
import csv, io, struct, weakref, _json, _thread, os, time
assert '_gc_main_dialect' not in csv.list_dialects()
csv.register_dialect('_gc_worker_dialect', delimiter=';')
csv.field_size_limit(17)
from collections.abc import Sequence
class SequenceInt(int):
    pass
Sequence.register(SequenceInt)
assert SequenceInt.__flags__ & (1 << 5)
old_int_flags = int.__flags__
Sequence.register(int)
assert int.__flags__ == old_int_flags
assert issubclass(int, Sequence)
type.__dict__['__abstractmethods__'].__set__(object, frozenset({'foreign_method'}))
assert object.__flags__ & (1 << 20)
assert list.__new__.__module__ != ['main']
list.__new__.__module__ = ['worker']
for typ in (csv.Error, io.UnsupportedOperation, struct.error, ExceptionGroup,
            _json.make_scanner, _json.make_encoder, _thread._ExceptHookArgs,
            time.struct_time, os.stat_result):
    assert not hasattr(typ, '_gc_owner_probe'), typ
    typ._gc_other_owner_probe = []
assert all(not getattr(ref.__callback__, '_gc_owner_probe', False)
           for ref in weakref.getweakrefs(object))
assert all(not getattr(cls, '_gc_owner_probe', False)
           for cls in object.__subclasses__())
basic_ref = weakref.ref(object)
assert weakref.ref(object) is basic_ref
""")
    assert abc.get_cache_token() == saved_abc_token
    assert int.__flags__ == saved_int_flags
    assert issubclass(int, Sequence) == saved_int_sequence
    # CPython still changes this shared static flag across interpreters.
    if sys.implementation.name == "rustpython":
        assert object.__flags__ == saved_object_flags
    assert csv.field_size_limit() == saved_csv_limit
    assert "_gc_worker_dialect" not in csv.list_dialects()
    assert weakref.ref(object) is basic_ref
    assert any(ref is owned_ref for ref in weakref.getweakrefs(object))
    assert PrivateSubclass in object.__subclasses__()
    assert list.__new__.__module__ == ["main"]
    assert list.__dict__["__new__"] is list.__new__
    for typ in private_types:
        assert not hasattr(typ, "_gc_other_owner_probe"), typ
finally:
    interp.exec("type.__dict__['__abstractmethods__'].__set__(object, frozenset())")
    interp.close()
    list.__new__.__module__ = saved_new_module
    csv.unregister_dialect("_gc_main_dialect")
    for typ in private_types:
        del typ._gc_owner_probe


# AST classes have mutable namespaces, slots and MROs; each interpreter must
# own the complete class, including references stored in _field_types.
import ast

ast.Constant._gc_owner_probe = []
saved_ast_bases = ast.Constant.__bases__
saved_ast_repr = ast.Constant.__dict__.get("__repr__")
ast.Constant.__bases__ = (ast.stmt,)
ast.Constant.__repr__ = lambda self: "main constant"
interp = interpreters.create()
try:
    interp.exec("""
import ast, gc
assert not hasattr(ast.Constant, '_gc_owner_probe')
assert ast.Constant.__bases__ == (ast.expr,)
assert repr(ast.Constant(value=1)) != 'main constant'
assert ast.Constant is type(ast.parse('1').body[0].value)
assert ast.Assign._field_types['value'] is ast.expr
ast.Constant.__repr__ = lambda self: 'worker constant'
assert repr(ast.Constant(value=1)) == 'worker constant'
assert all(not hasattr(obj, '_gc_owner_probe')
           for obj in gc.get_objects() if type(obj) is type)
""")
    assert repr(ast.Constant(value=1)) == "main constant"
    assert ast.Constant.__bases__ == (ast.stmt,)
finally:
    interp.close()
    ast.Constant.__bases__ = saved_ast_bases
    if saved_ast_repr is None:
        del ast.Constant.__repr__
    else:
        ast.Constant.__repr__ = saved_ast_repr
    del ast.Constant._gc_owner_probe


try:
    import sqlite3
except ImportError:
    pass  # Optional native module.
else:
    sqlite3.register_converter("_GC_OWNER_PROBE", bytes)
    interp = interpreters.create()
    try:
        interp.exec(
            "import sqlite3; assert '_GC_OWNER_PROBE' not in sqlite3.converters"
        )
    finally:
        interp.close()
        del sqlite3.converters["_GC_OWNER_PROBE"]


# RustPython implements these codecs by importing Python functions. Warming
# that cache must not reuse a function (and its globals) from another VM.
if sys.implementation.name == "rustpython":
    import _codecs

    import _pycodecs

    original_charmap_encode = _pycodecs.charmap_encode
    _pycodecs.charmap_encode = lambda *args: (b"private", 1)
    try:
        assert _codecs.charmap_encode("x") == (b"private", 1)
        interp = interpreters.create()
        try:
            interp.exec(
                "import _codecs; assert _codecs.charmap_encode('x') == (b'x', 1)"
            )
        finally:
            interp.close()
    finally:
        _pycodecs.charmap_encode = original_charmap_encode


# Entering a subinterpreter must start its own frame chain and restore the
# caller's chain afterwards.
caller_frame = sys._getframe()
interp = interpreters.create()
try:
    interp.exec("import sys; assert sys._getframe().f_back is None")
finally:
    interp.close()
assert sys._getframe() is caller_frame
del caller_frame
