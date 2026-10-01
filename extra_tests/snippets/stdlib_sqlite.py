import sqlite3 as sqlite
import unittest

rows = [(3,), (4,)]
cx = sqlite.connect(":memory:")
cx.execute(";")
cx.executescript(";")
cx.execute("CREATE TABLE foo(key INTEGER)")
cx.executemany("INSERT INTO foo(key) VALUES (?)", rows)

cur = cx.cursor()
fetchall = cur.execute("SELECT * FROM foo").fetchall()
assert fetchall == rows

cx.executescript("""
    /* CREATE TABLE foo(key INTEGER); */
    INSERT INTO foo(key) VALUES (10);
    INSERT INTO foo(key) VALUES (11);
""")


class AggrSum:
    def __init__(self):
        self.val = 0.0

    def step(self, val):
        self.val += val

    def finalize(self):
        return self.val


cx.create_aggregate("mysum", 1, AggrSum)
cur.execute("select mysum(key) from foo")
assert cur.fetchone()[0] == 28.0

# toobig = 2**64
# cur.execute("insert into foo(key) values (?)", (toobig,))


class AggrText:
    def __init__(self):
        self.txt = ""

    def step(self, txt):
        txt = str(txt)
        self.txt = self.txt + txt

    def finalize(self):
        return self.txt


cx.create_aggregate("aggtxt", 1, AggrText)
cur.execute("select aggtxt(key) from foo")
assert cur.fetchone()[0] == "341011"

# Blob extended-slice assignment with negative step
# Guard: CPython 3.11 has a SystemError bug with negative-step Blob slicing;
# this test only runs on RustPython where the fix is being validated.
# TODO: remove this once https://github.com/python/cpython/pull/150450 is released and RustPython CI uses it.
import sys

if sys.implementation.name == "rustpython":
    cx.execute("CREATE TABLE blobtest(b BLOB)")
    data = b"this blob data string is exactly fifty bytes long!"
    cx.execute("INSERT INTO blobtest(b) VALUES (?)", (data,))
    blob = cx.blobopen("blobtest", "b", 1)
    blob[9:0:-2] = b"12345"  # writes to indices 9, 7, 5, 3, 1
    actual = cx.execute("select b from blobtest").fetchone()[0]
    expected = b"t5i4 3l2b1" + data[10:]
    assert actual == expected, f"got {actual!r}, expected {expected!r}"
    blob.close()


def check_callback_lifetimes():
    import contextvars
    import gc
    import threading
    import weakref

    current = contextvars.ContextVar("sqlite_callback_context", default=0)
    instances = []
    errors = []
    connection = sqlite.connect(":memory:", check_same_thread=False)

    class Aggregate:
        def __init__(self):
            instances.append(weakref.ref(self))
            self.value = 0

        def step(self, value):
            self.value += value

        def finalize(self):
            return self.value + current.get()

    def register():
        try:
            connection.create_function("current_context", 0, current.get)
            connection.create_aggregate("total_context", 1, Aggregate)
        except BaseException as error:
            errors.append(error)

    worker = threading.Thread(target=register)
    worker.start()
    worker.join()
    assert not errors, errors

    current.set(40)
    assert connection.execute("SELECT current_context()").fetchone() == (40,)
    assert connection.execute("SELECT total_context(2)").fetchone() == (42,)
    gc.collect()
    assert len(instances) == 1 and instances[0]() is None

    class FailingAggregate(Aggregate):
        def finalize(self):
            raise ValueError("finalize")

    connection.create_aggregate("failing", 1, FailingAggregate)
    try:
        connection.execute("SELECT failing(2)").fetchone()
    except sqlite.OperationalError:
        pass
    else:
        raise AssertionError("aggregate failure did not propagate")
    gc.collect()
    assert len(instances) == 2 and instances[1]() is None
    connection.close()


check_callback_lifetimes()
