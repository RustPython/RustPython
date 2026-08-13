# Regression tests for interpreter aborts, panics and unbounded allocations found
# by fuzzing and static review. Every case here used to kill the interpreter.
#
# Each test is labelled with the catalog id it came from.

import gc
import itertools
import sys
import weakref

from testutils import assert_raises

# ---------------------------------------------------------------------------
# unguarded native recursion (SIGSEGV -> RecursionError)
# ---------------------------------------------------------------------------

# RPYR-0013 / RPYR-0014: the hash slot dispatch recursed once per nesting level.
deep_tuple = ()
for _ in range(sys.getrecursionlimit() * 2):
    deep_tuple = (deep_tuple,)
with assert_raises(RecursionError):
    hash(deep_tuple)
# a dict key or set member hashes on insertion, same path
with assert_raises(RecursionError):
    {deep_tuple: 1}
with assert_raises(RecursionError):
    {deep_tuple}
# the guarded siblings still behave
with assert_raises(RecursionError):
    repr(deep_tuple)

deep_alias = int
for _ in range(sys.getrecursionlimit() * 2):
    deep_alias = list[deep_alias]
with assert_raises(RecursionError):
    hash(deep_alias)

# RPYR-0015: __parameters__ is computed eagerly at subscript and recursed into
# every list/tuple argument.
self_referential = []
self_referential.append(self_referential)
with assert_raises(RecursionError):
    list[self_referential]

nested = [0]
for _ in range(sys.getrecursionlimit() * 2):
    nested = [nested]
with assert_raises(RecursionError):
    list[nested]

# ---------------------------------------------------------------------------
# memory unsafety
# ---------------------------------------------------------------------------

# RPYR-0020 / RUSTPY-0018: the error message formatted its tasks with Rust's
# Debug, which walked a PyFunction's code through an unsound cast.
import _asyncio


def _task():
    pass


_asyncio._enter_task(0, _task)
with assert_raises(RuntimeError) as cm:
    _asyncio._enter_task(0, _task)
assert "Cannot enter into task" in str(cm.exception), cm.exception
assert "<function _task at " in str(cm.exception), cm.exception
_asyncio._leave_task(0, _task)

# RUSTPY-0008: a Match built through __new__ has no match state, and the
# mapping subscript read it anyway.
import re

match_type = type(re.match("a", "a"))
with assert_raises(TypeError):
    match_type.__new__(match_type)

# RUSTPY-0002: the field getters indexed a struct sequence that __new__ had
# left empty.
try:
    import pwd
except ImportError:
    pwd = None
if pwd is not None:
    with assert_raises(TypeError):
        pwd.struct_passwd()

# ---------------------------------------------------------------------------
# integer narrowing and unbounded allocation
# ---------------------------------------------------------------------------

# RPYR-0006 / RPYR-0007: the repeat count was multiplied and reserved without
# a guard.
from collections import deque

with assert_raises(MemoryError):
    deque([0]) * sys.maxsize
with assert_raises(MemoryError):
    (1,) * sys.maxsize
with assert_raises(MemoryError):
    [1] * sys.maxsize

# RPYR-0009: r was narrowed to usize with an unwrap.
with assert_raises(OverflowError):
    itertools.combinations(range(5), 2**64)
with assert_raises(OverflowError):
    itertools.combinations_with_replacement(range(5), 2**64)
with assert_raises(OverflowError):
    itertools.permutations(range(5), 2**64)
# and a representable r still has to fit in memory
with assert_raises(MemoryError):
    itertools.combinations(range(5), 2**44)
with assert_raises(MemoryError):
    itertools.combinations_with_replacement(range(5), 2**44)

# RPYR-0017 / RUSTPY-0017: ctypes narrowed a big int with an expect().
import ctypes

assert ctypes.c_char_p(2**64).value is None
assert ctypes.c_int(2**64 + 7).value == 7
buf = (ctypes.c_int * 1)()
ptr = ctypes.cast(buf, ctypes.POINTER(ctypes.c_int))
ptr[0] = 2**64 + 5
assert ptr[0] == 5

# ---------------------------------------------------------------------------
# eagerly collecting an unbounded iterable
# ---------------------------------------------------------------------------

# RPYR-0011: both operands were collected into vectors before being compared.
import math

with assert_raises(ValueError):
    math.sumprod(itertools.count(), [1, 2, 3])
with assert_raises(ValueError):
    math.sumprod([1, 2, 3], itertools.count())
assert math.sumprod(iter([1, 2, 3]), iter([4, 5, 6])) == 32

# RUSTPY-0012
import _suggestions

with assert_raises(TypeError):
    _suggestions._generate_suggestions(itertools.count(), "x")

# RUSTPY-0013
import lzma

with assert_raises(TypeError):
    lzma.LZMACompressor(
        format=lzma.FORMAT_RAW,
        filters=({"id": lzma.FILTER_LZMA2} for _ in itertools.count()),
    )

# RUSTPY-0014
with assert_raises(TypeError):
    ExceptionGroup("m", itertools.count())

# RUSTPY-0015: the right-hand side was materialized before its length was
# checked against the slice.
array3 = (ctypes.c_int * 3)()
with assert_raises(ValueError):
    array3[0:3] = itertools.count()
array3[0:3] = [7, 8, 9]
assert list(array3) == [7, 8, 9]

# RPYR-0019 / RUSTPY-0016
import os

if hasattr(os, "posix_spawn"):
    with assert_raises(TypeError):
        os.posix_spawn("/bin/true", map(str, itertools.count()), os.environ)
if hasattr(os, "setgroups"):
    with assert_raises(TypeError):
        os.setgroups(itertools.count())

# ---------------------------------------------------------------------------
# .unwrap()/.expect() on a value Python controls
# ---------------------------------------------------------------------------

# RPYR-0001: __reduce__ read args[0] of an exception with no args.
import pickle

assert pickle.loads(pickle.dumps(ImportError())).args == ()
restored = pickle.loads(pickle.dumps(ImportError("m", name="n", path="p")))
assert restored.args == ("m",)
assert restored.name == "n"
assert restored.path == "p"

# RPYR-0002: the module-level task registry can be rebound from Python.
saved_current_tasks = _asyncio._current_tasks
try:
    _asyncio._current_tasks = 42
    assert _asyncio.current_task(loop=object()) is None
finally:
    _asyncio._current_tasks = saved_current_tasks

# RPYR-0003: throw() downcast whatever __new__ returned.
future = _asyncio.Future(loop=object())


class BadException(Exception):
    def __new__(cls, *args):
        return 42


with assert_raises(TypeError):
    future.__await__().throw(BadException)

# RPYR-0004 / RPYR-0005
import mmap

mapping = mmap.mmap(-1, 10)
assert mapping.find(b"x", 5, 2) == -1
assert mapping.rfind(b"x", 5, 2) == -1
with assert_raises(ValueError):
    mapping.move(20, 0, 1)
with assert_raises(ValueError):
    mapping.move(0, 20, 1)
mapping.close()

# RPYR-0018: the second argument is keyword-only, and the positional form hit
# an unimplemented!().
import _imp

with assert_raises(TypeError):
    _imp.find_frozen("x", True)
assert _imp.find_frozen("_this_module_does_not_exist_") is None

# RUSTPY-0003: the hash types needed _hashlib to have initialized them.
import _md5
import _sha1

assert _md5.md5(b"").hexdigest() == "d41d8cd98f00b204e9800998ecf8427e"
assert _sha1.sha1(b"").hexdigest() == "da39a3ee5e6b4b0d3255bfef95601890afd80709"

# RUSTPY-0004: the reader unwrapped a dialect it never registered.
import _csv

with assert_raises(StopIteration):
    next(_csv.reader([]))

# RUSTPY-0005: the argument was read before its arity was checked.
import _typing

with assert_raises(TypeError):
    _typing._idfunc()

# RUSTPY-0006: the source was assumed to be valid UTF-8.
with assert_raises(UnicodeEncodeError):
    eval(chr(0xD800))
with assert_raises(UnicodeEncodeError):
    compile(chr(0xD800), "<test>", "eval")
with assert_raises(UnicodeEncodeError):
    exec(chr(0xD800))

# RUSTPY-0021: the warning raised under -W error and the hook unwrapped it.
import warnings

saved_breakpoint_env = os.environ.get("PYTHONBREAKPOINT")
os.environ["PYTHONBREAKPOINT"] = "nonexistent_xyz.foo"
try:
    with warnings.catch_warnings():
        warnings.simplefilter("error")
        with assert_raises(RuntimeWarning):
            sys.breakpointhook()
finally:
    if saved_breakpoint_env is None:
        del os.environ["PYTHONBREAKPOINT"]
    else:
        os.environ["PYTHONBREAKPOINT"] = saved_breakpoint_env

# RUSTPY-0024: the implicit ctypes conversion accepted a float and passed its
# truncated value where a pointer was expected.
import ctypes.util

libc_name = ctypes.util.find_library("c")
if libc_name is not None and sys.platform != "win32":
    libc = ctypes.CDLL(libc_name)
    for bad in (1.5, 0.0, 1e300):
        with assert_raises(TypeError):
            libc.abs(bad)
    assert libc.abs(-3) == 3
    assert libc.abs(True) == 1

# ---------------------------------------------------------------------------
# garbage collection
# ---------------------------------------------------------------------------


class Node:
    pass


def collects(wrap):
    """Build `node -> node.__dict__ -> wrap(container) -> container -> node`
    and report whether the collector breaks it."""

    def build():
        container = []
        node = Node()
        container.append(node)
        node.held = wrap(container)
        return weakref.ref(node)

    gc.collect()
    ref = build()
    gc.collect()
    return ref() is None


# RPYR-0010 / RPYR-0016: these types declared no traverse at all.
from collections import defaultdict

assert collects(lambda c: deque(c))
assert collects(lambda c: defaultdict(int, {"k": c}))
assert collects(lambda c: classmethod(lambda cls: c))

# RPYR-0012: a PyIter field reported the referents of the iterator it held
# instead of the iterator, so its own reference was never accounted for and
# every cycle through it was treated as reachable.
assert collects(iter)
assert collects(lambda c: map(str, c))
assert collects(lambda c: filter(None, c))
assert collects(lambda c: zip(c))
assert collects(enumerate)
assert collects(reversed)
assert collects(itertools.chain)
assert collects(itertools.cycle)
assert collects(lambda c: itertools.islice(c, 5))
assert collects(itertools.groupby)
assert collects(itertools.accumulate)
assert collects(lambda c: itertools.starmap(str, c))
assert collects(lambda c: itertools.takewhile(bool, c))
assert collects(lambda c: itertools.dropwhile(bool, c))
assert collects(lambda c: itertools.filterfalse(None, c))
assert collects(lambda c: itertools.compress(c, [1]))
assert collects(lambda c: itertools.product(c))
assert collects(lambda c: itertools.combinations(c, 1))

# ---------------------------------------------------------------------------
# struct sequences
# ---------------------------------------------------------------------------

# The optional second argument fills the hidden fields.
import time

fields = (2024, 1, 2, 3, 4, 5, 6, 7, 0)
assert time.struct_time(fields).tm_zone is None
assert time.struct_time(fields, {"tm_zone": "UTC"}).tm_zone == "UTC"
assert time.struct_time(fields, {"tm_gmtoff": 60}).tm_gmtoff == 60
with assert_raises(TypeError):
    time.struct_time(fields, ["tm_zone", "UTC"])

assert os.stat_result(tuple(range(10))).st_atime == 7
assert os.stat_result(tuple(range(10)), {"st_atime": 1.5}).st_atime == 1.5
with assert_raises(TypeError):
    os.stat_result(tuple(range(10)), ["st_atime"])

# ---------------------------------------------------------------------------
# races (these panicked a worker thread)
# ---------------------------------------------------------------------------

import threading

# RUSTPY-0020: repr took the first element with an expect() justified by a
# preceding non-empty check.
shared_set = {1, 2, 3, 4, 5}
stop = False


def mutate_set():
    while not stop:
        try:
            shared_set.clear()
            shared_set.update({1, 2, 3})
        except RuntimeError:  # changed size during iteration
            pass


def read_set():
    for _ in range(20000):
        try:
            repr(shared_set)
        except RuntimeError:  # changed size during iteration
            pass


threads = [threading.Thread(target=mutate_set) for _ in range(2)]
threads += [threading.Thread(target=read_set) for _ in range(2)]
for t in threads[:2]:
    t.start()
for t in threads[2:]:
    t.start()
for t in threads[2:]:
    t.join()
stop = True
for t in threads[:2]:
    t.join()

# RUSTPY-0022: the index was advanced and wrapped in two separate steps.
shared_cycle = itertools.cycle([1, 2, 3])


def spin_cycle():
    for _ in range(20000):
        next(shared_cycle)


threads = [threading.Thread(target=spin_cycle) for _ in range(4)]
for t in threads:
    t.start()
for t in threads:
    t.join()

print("crash regressions passed")
