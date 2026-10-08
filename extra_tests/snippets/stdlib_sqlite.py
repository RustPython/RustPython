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


# Aggregate instances belong to a query, including its error and cancellation paths.
import gc
import weakref

from testutils import assert_raises


def check_aggregate_lifetime(failure):
    refs = []

    class TrackedAggregate:
        def __init__(self):
            refs.append(weakref.ref(self))

        def step(self, value):
            if failure == "step":
                raise ValueError("step")

        def finalize(self):
            if failure == "finalize":
                raise ValueError("finalize")
            return object() if failure == "result" else 1

    cx.create_aggregate("tracked", 1, TrackedAggregate)
    query = "SELECT tracked(key) FROM foo GROUP BY key % 2"
    if failure:
        with assert_raises(sqlite.OperationalError):
            cx.execute(query).fetchall()
    else:
        assert cx.execute(query).fetchall() == [(1,), (1,)]
    assert refs
    gc.collect()
    assert all(ref() is None for ref in refs), failure

    # xFinal may run without any preceding xStep.
    refs.clear()
    assert cx.execute("SELECT tracked(key) FROM foo WHERE 0").fetchall() == [(None,)]
    assert not refs


for failure in (None, "step", "finalize", "result"):
    check_aggregate_lifetime(failure)


# Window functions share aggregate finalization, including an unfinished cursor.
window_refs = []


class TrackedWindow:
    def __init__(self):
        window_refs.append(weakref.ref(self))

    def step(self, value):
        pass

    def inverse(self, value):
        pass

    def value(self):
        return 1

    def finalize(self):
        return 1


cx.create_window_function("tracked_window", 1, TrackedWindow)
window_query = """
    SELECT tracked_window(key) OVER (
        ORDER BY key ROWS BETWEEN 1 PRECEDING AND CURRENT ROW
    ) FROM foo
"""
assert cx.execute(window_query).fetchall() == [(1,)] * 4
assert window_refs
assert all(ref() is None for ref in window_refs)
window_refs.clear()
window_cursor = cx.execute(window_query)
assert window_cursor.fetchone() == (1,)
window_cursor.close()
gc.collect()
assert window_refs and all(ref() is None for ref in window_refs)


# Releasing the last reference can run Python that reads the cursor/connection.
def check_aggregate_destructor(operation):
    connection = sqlite.connect(":memory:")
    cursor = connection.cursor()
    cursor_ref = weakref.ref(cursor)
    released = []

    class Aggregate:
        def step(self, value):
            pass

        def value(self):
            return 1

        finalize = value

        def __del__(self):
            current = cursor_ref()
            if current is not None:
                try:
                    current.description
                except sqlite.ProgrammingError:
                    pass  # The cursor may already have been closed.
            released.append(connection.total_changes)

    connection.create_aggregate("tracked", 1, Aggregate)
    expected = 1
    if operation == "execute":
        cursor.execute("SELECT tracked(1)")
    elif operation == "script":
        cursor.executescript("SELECT tracked(1);")
    elif operation == "many":
        connection.execute("CREATE TABLE results(value)")
        cursor.executemany("INSERT INTO results SELECT tracked(?)", [(1,), (2,)])
        expected = 2
    else:
        connection.create_window_function("tracked_window", 1, Aggregate)
        cursor.execute("""
            SELECT tracked_window(x) OVER (ROWS UNBOUNDED PRECEDING)
            FROM (SELECT 1 AS x UNION ALL SELECT 2 UNION ALL SELECT 3)
        """)
        assert not released
        if operation == "fetch":
            assert cursor.fetchall() == [(1,), (1,), (1,)]
        elif operation == "close":
            cursor.close()
        elif operation == "replace":
            cursor.execute("SELECT 2")
        else:
            del cursor
            gc.collect()
    assert len(released) == expected, (operation, released)
    connection.close()


for operation in ("execute", "script", "many", "fetch", "close", "replace", "drop"):
    check_aggregate_destructor(operation)


# A nested query must not release its aggregate while an outer callback holds locks.
def check_nested_aggregate_destructor(operation):
    outer = sqlite.connect(":memory:", autocommit=False)
    nested = sqlite.connect(":memory:")
    released = []
    inspect = lambda: outer.total_changes

    class Aggregate:
        def step(self, value):
            pass

        def finalize(self):
            return 1

        def __del__(self):
            try:
                inspect()
            except sqlite.ProgrammingError:
                pass  # Closing the outer connection may finish before cleanup.
            released.append(True)

    nested.create_aggregate("tracked", 1, Aggregate)
    outer.set_trace_callback(lambda sql: nested.execute("SELECT tracked(1)"))
    if operation == "commit":
        outer.commit()
    elif operation == "rollback":
        outer.rollback()
    elif operation == "autocommit":
        outer.autocommit = True
    elif operation in ("blobopen", "blob_index"):
        outer.execute("CREATE TABLE blobs(data BLOB)")
        outer.execute("INSERT INTO blobs VALUES (zeroblob(1))")
        released.clear()

        def progress():
            nested.execute("SELECT tracked(1)")
            return 0

        outer.set_progress_handler(progress, 1)
        blob = outer.blobopen("blobs", "data", 1)
        outer.set_progress_handler(None, 0)
        if operation != "blobopen":
            released.clear()
            inspect = lambda: len(blob)

            class Index:
                def __index__(self):
                    nested.execute("SELECT tracked(1)")
                    return 0

            index = Index()
            assert blob[index] == 0
            blob[index] = 1
            assert len(released) == 2
        blob.close()
    elif operation == "replace_callback":

        class Callback:
            def __call__(self):
                return 1

            def __del__(self):
                nested.execute("SELECT tracked(1)")

        for register in (
            lambda cb: outer.create_function("hook", 0, cb),
            lambda cb: outer.create_aggregate("hook", 0, cb),
            lambda cb: outer.create_window_function("hook", 0, cb),
            lambda cb: outer.create_collation("hook", cb),
        ):
            released.clear()
            register(Callback())
            register(None)
            assert released
    else:
        outer.close()
    assert released, operation
    if operation != "close":
        outer.set_trace_callback(None)
    outer.close()
    nested.close()


for operation in (
    "commit",
    "rollback",
    "autocommit",
    "close",
    "blobopen",
    "blob_index",
    "replace_callback",
):
    check_nested_aggregate_destructor(operation)
