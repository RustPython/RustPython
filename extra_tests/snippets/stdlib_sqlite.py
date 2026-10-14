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


class BlobIndex:
    def __init__(self, value):
        self.value = value

    def __index__(self):
        return self.value


class BlobInt(int):
    pass


class BrokenBlobIndex:
    def __index__(self):
        raise OverflowError("index conversion failed")


cx.execute("CREATE TABLE blobindices(b BLOB)")
cx.execute("INSERT INTO blobindices(b) VALUES (?)", (b"abc",))
with cx.blobopen("blobindices", "b", 1) as blob:
    assert blob[BlobIndex(-1)] == ord("c")
    blob[BlobIndex(0)] = ord("A")
    assert blob[0] == ord("A")

    test = unittest.TestCase()
    for index in (sys.maxsize, -sys.maxsize - 1):
        with test.assertRaisesRegex(IndexError, "^Blob index out of range$"):
            blob[index]
        with test.assertRaisesRegex(IndexError, "^Blob index out of range$"):
            blob[index] = 0

    for value in (sys.maxsize + 1, -sys.maxsize - 2):
        for index in (value, BlobInt(value), BlobIndex(value)):
            message = (
                f"^cannot fit '{type(index).__name__}' into an index-sized integer$"
            )
            with test.assertRaisesRegex(IndexError, message):
                blob[index]
            with test.assertRaisesRegex(IndexError, message):
                blob[index] = 0

    for name, bounded_name in (("X" * 250, "X" * 200), ("€" * 67, "€" * 66)):
        long_index = type(name, (), {"__index__": lambda self: sys.maxsize + 1})()
        message = f"^cannot fit '{bounded_name}' into an index-sized integer$"
        with test.assertRaisesRegex(IndexError, message):
            blob[long_index]
        with test.assertRaisesRegex(IndexError, message):
            blob[long_index] = 0

    with test.assertRaisesRegex(OverflowError, "^index conversion failed$"):
        blob[BrokenBlobIndex()]
    with test.assertRaisesRegex(OverflowError, "^index conversion failed$"):
        blob[BrokenBlobIndex()] = 0
    assert blob[:] == b"Abc"


import sqlite3


def statement_cache_reuse():
    for capacity, expected in ((0, 3), (-1, 3), (2, 1), (None, 1)):
        options = {} if capacity is None else {"cached_statements": capacity}
        with sqlite3.connect(":memory:", **options) as connection:
            prepared = []

            def authorizer(action, *args):
                if action == sqlite3.SQLITE_SELECT:
                    prepared.append(action)
                return sqlite3.SQLITE_OK

            connection.set_authorizer(authorizer)
            for parameter in (1, 2, 3):
                assert connection.execute("SELECT ?", (parameter,)).fetchall() == [
                    (parameter,)
                ]
            assert len(prepared) == expected
            if capacity == 2:
                prepared.clear()
                for value in (1, 2, 1, 3, 2):
                    assert connection.execute(f"SELECT {value}").fetchall() == [
                        (value,)
                    ]
                assert len(prepared) == 4
        connection.close()


statement_cache_reuse()


def cache_reprepare_with_authorizer():
    connection = sqlite3.connect(":memory:")
    connection.execute("CREATE TABLE words(word TEXT COLLATE NOCASE)")
    connection.execute("CREATE INDEX word_index ON words(word)")
    connection.executemany("INSERT INTO words VALUES (?)", [("alpha",), ("beta",)])
    sql = "SELECT word FROM words WHERE word LIKE ?"
    assert connection.execute(sql, ("a%",)).fetchall() == [("alpha",)]
    authorizations = []

    def authorizer(action, *args):
        if action == sqlite3.SQLITE_SELECT and not authorizations:
            authorizations.append(True)
        return sqlite3.SQLITE_OK

    connection.set_authorizer(authorizer)
    for prefix, result in (("b%", "beta"), ("a%", "alpha")):
        authorizations.clear()
        assert connection.execute(sql, (prefix,)).fetchall() == [(result,)]
        assert authorizations == [True]
    connection.execute("ALTER TABLE words ADD COLUMN extra")
    authorizations.clear()
    assert connection.execute(sql, ("b%",)).fetchall() == [("beta",)]
    assert authorizations == [True]
    connection.close()


cache_reprepare_with_authorizer()


def statement_cache_connection_lifetime():
    # CPython 3.14 can crash when an adapter closes its connection.
    if sys.implementation.name != "rustpython":
        return
    test = unittest.TestCase()
    for phase in ("bind", "step", "fetch", "iterate"):
        connection = sqlite3.connect(":memory:")
        connection.execute("CREATE TABLE t(x)")

        def change_connection():
            connection.close()

        class Parameter:
            def __conform__(self, protocol):
                change_connection()
                return 1

        def change_on(value):
            if phase == "step" or value == 2:
                change_connection()
            return value

        def parameters():
            yield (1,)
            change_connection()
            yield (2,)

        connection.create_function("change_on", 1, change_on)
        with test.assertRaises(sqlite3.ProgrammingError):
            if phase == "bind":
                connection.execute("SELECT ?", (Parameter(),))
            elif phase == "step":
                connection.execute("SELECT change_on(1)")
            elif phase == "fetch":
                cursor = connection.execute(
                    "SELECT change_on(1) UNION ALL SELECT change_on(2)"
                )
                cursor.fetchone()
            else:
                connection.executemany("INSERT INTO t VALUES (?)", parameters())
        connection.close()


statement_cache_connection_lifetime()


def statement_cache_deferred_cleanup():
    # CPython can block when closing a populated cache from a callback's thread.
    if sys.implementation.name != "rustpython":
        return
    import threading

    test = unittest.TestCase()
    for action in ("close", "reinit"):
        for autocommit in (sqlite3.LEGACY_TRANSACTION_CONTROL, False):
            connection = sqlite3.connect(
                ":memory:", check_same_thread=False, autocommit=autocommit
            )
            connection.execute("SELECT 123").fetchall()
            traced = []
            connection.set_trace_callback(traced.append)
            completed = []
            workers = []
            failures = []

            def change_connection():
                try:
                    if action == "close":
                        connection.close()
                    else:
                        connection.__init__(":memory:", check_same_thread=False)
                except Exception as error:
                    failures.append(error)

            def callback():
                worker = threading.Thread(target=change_connection, daemon=True)
                workers.append(worker)
                worker.start()
                worker.join(5)
                completed.append(not worker.is_alive())
                return 1

            connection.create_function("callback", 0, callback)
            if action == "close":
                with test.assertRaises(sqlite3.ProgrammingError):
                    connection.execute("SELECT callback()")
            else:
                assert connection.execute("SELECT callback()").fetchall() == [(1,)]
                assert connection.execute("SELECT 456").fetchall() == [(456,)]
            for worker in workers:
                worker.join(5)
                assert not worker.is_alive()
            assert not failures, failures
            assert completed == [True], (action, autocommit)
            if autocommit is False:
                assert "ROLLBACK" in traced, traced
            connection.close()


def concurrent_sqlite_error_snapshots():
    # CPython also races when reading the connection's last error after a call.
    if sys.implementation.name != "rustpython":
        return
    import threading

    connection = sqlite3.connect(":memory:", check_same_thread=False, autocommit=True)
    for index in range(4):
        connection.execute(f"CREATE TABLE t{index}(x UNIQUE)")
        connection.execute(f"INSERT INTO t{index} VALUES (1)")

    for prepare in (True, False):
        barrier = threading.Barrier(4)
        failures = []
        completed = [0] * 4

        def worker(index):
            if prepare:
                sql = f"SELECT * FROM missing_table_{index}"
                expected = (
                    sqlite3.OperationalError,
                    f"no such table: missing_table_{index}",
                    sqlite3.SQLITE_ERROR,
                )
            else:
                sql = f"INSERT INTO t{index} VALUES (1)"
                expected = (
                    sqlite3.IntegrityError,
                    f"UNIQUE constraint failed: t{index}.x",
                    sqlite3.SQLITE_CONSTRAINT_UNIQUE,
                )
            barrier.wait()
            for _ in range(1000):
                try:
                    connection.execute(sql)
                except sqlite3.Error as error:
                    actual = (type(error), str(error), error.sqlite_errorcode)
                    if actual == expected:
                        completed[index] += 1
                        continue
                    failures.append((expected, actual))
                else:
                    failures.append((expected, "no error"))
                break

        workers = [
            threading.Thread(target=worker, args=(i,), daemon=True) for i in range(4)
        ]
        for thread in workers:
            thread.start()
        for thread in workers:
            thread.join(10)
            assert not thread.is_alive(), "concurrent SQLite call blocked"
        assert not failures, failures
        assert completed == [1000] * 4, completed
    connection.close()


statement_cache_deferred_cleanup()
concurrent_sqlite_error_snapshots()


def script_stops_after_close():
    # CPython can crash when its connection closes inside a callback.
    if sys.implementation.name != "rustpython":
        return
    test = unittest.TestCase()
    for phase in ("prepare", "step", "commit"):
        connection = sqlite3.connect(":memory:")
        continued = []
        connection.create_function("tail", 0, lambda: continued.append(True))
        connection.create_function("closeme", 0, connection.close)
        if phase == "prepare":

            def authorizer(*args):
                connection.close()
                return sqlite3.SQLITE_OK

            connection.set_authorizer(authorizer)
        elif phase == "commit":
            connection.execute("CREATE TABLE t(x)")
            connection.execute("INSERT INTO t VALUES (1)")
            connection.set_trace_callback(lambda sql: connection.close())
        script = (
            "SELECT closeme(); SELECT tail();" if phase == "step" else "SELECT tail();"
        )
        with test.assertRaises(sqlite3.ProgrammingError):
            connection.executescript(script)
        assert not continued, phase


def concurrent_sqlite_results():
    # CPython also races when reading connection-wide result metadata/errors.
    if sys.implementation.name != "rustpython":
        return
    import threading

    connection = sqlite3.connect(":memory:", check_same_thread=False, autocommit=True)
    connection.execute("CREATE TABLE t(id INTEGER PRIMARY KEY, value)")
    connection.executemany("INSERT INTO t VALUES (?, 0)", [(i,) for i in range(4)])
    for operation in ("insert", "update", "many", "returning", "backup"):
        barrier = threading.Barrier(4, timeout=10)
        failures = []
        completed = [0] * 4

        def worker(index):
            source = sqlite3.connect(":memory:") if operation == "backup" else None
            try:
                barrier.wait()
                for iteration in range(500):
                    expected = index + 1
                    if source is not None:
                        try:
                            source.backup(connection, name=f"missing_{index}")
                        except sqlite3.OperationalError as error:
                            assert str(error) == f"unknown database missing_{index}", (
                                str(error)
                            )
                            assert error.sqlite_errorcode == sqlite3.SQLITE_ERROR
                        else:
                            raise AssertionError("backup accepted an unknown schema")
                    elif operation == "insert":
                        expected = 1000 + index * 10000 + iteration
                        cursor = connection.execute(
                            "INSERT INTO t VALUES (?, 0)", (expected,)
                        )
                        assert cursor.lastrowid == expected, (
                            expected,
                            cursor.lastrowid,
                        )
                        assert cursor.rowcount == 1
                    else:
                        sql = "UPDATE t SET value=value+1 WHERE id < ?"
                        if operation == "many":
                            cursor = connection.executemany(
                                sql, [(expected,), (expected,)]
                            )
                            expected *= 2
                        else:
                            if operation == "returning":
                                sql += " RETURNING id"
                            cursor = connection.execute(sql, (expected,))
                            cursor.fetchall()
                        assert cursor.rowcount == expected, (
                            operation,
                            expected,
                            cursor.rowcount,
                        )
                    assert not connection.in_transaction
                    assert connection.total_changes >= 4
                    completed[index] += 1
            except Exception as error:
                failures.append(error)
            finally:
                if source is not None:
                    source.close()

        workers = [
            threading.Thread(target=worker, args=(i,), daemon=True) for i in range(4)
        ]
        for worker_thread in workers:
            worker_thread.start()
        for worker_thread in workers:
            worker_thread.join(15)
            assert not worker_thread.is_alive(), operation
        assert not failures, failures
        assert completed == [500] * 4, (operation, completed)
    connection.close()


def sqlite_callbacks_use_calling_thread():
    import threading

    connection = sqlite3.connect(":memory:", check_same_thread=False)
    connection.create_function("exception", 0, lambda: str(sys.exception()))
    traced = []
    connection.set_trace_callback(lambda sql: traced.append(str(sys.exception())))
    results = []

    def worker():
        try:
            raise ValueError("worker")
        except ValueError:
            results.append(connection.execute("SELECT exception()").fetchone()[0])

    try:
        raise RuntimeError("main")
    except RuntimeError:
        thread = threading.Thread(target=worker, daemon=True)
        thread.start()
        thread.join(5)
        assert not thread.is_alive()
        assert str(sys.exception()) == "main"
    assert results == ["worker"], results
    assert traced == ["worker"], traced
    connection.close()


sqlite_callbacks_use_calling_thread()


def concurrent_sqlite_transactions():
    if sys.implementation.name != "rustpython":
        return
    import threading
    import time

    connection = sqlite3.connect(":memory:", check_same_thread=False)
    connection.execute("CREATE TABLE t(x)")
    queries = ("INSERT INTO t VALUES (1)", "INSERT INTO t VALUES (2)")
    for query in queries:
        connection.execute(query)
    connection.rollback()
    entered = threading.Event()
    started = threading.Event()
    failures = []

    def trace(sql):
        if sql.startswith("BEGIN") and not entered.is_set():
            entered.set()
            started.wait(5)
            # Let the competing cached execute reach its implicit BEGIN check.
            time.sleep(0.05)

    def insert():
        try:
            assert entered.wait(5)
            started.set()
            connection.execute(queries[1])
        except Exception as error:
            failures.append(error)

    connection.set_trace_callback(trace)
    worker = threading.Thread(target=insert, daemon=True)
    worker.start()
    connection.execute(queries[0])
    worker.join(5)
    assert not worker.is_alive()
    assert not failures, failures
    assert connection.execute("SELECT count(*) FROM t").fetchone() == (2,)
    connection.set_trace_callback(None)
    connection.rollback()

    # Either ordering of legacy DML and autocommit=False must be rollbackable.
    barrier = threading.Barrier(3, timeout=10)
    rounds = 500

    def run(setter):
        for _ in range(rounds):
            barrier.wait()
            try:
                if setter:
                    connection.autocommit = False
                else:
                    connection.execute(queries[0])
            except Exception as error:
                failures.append(error)
            barrier.wait()

    workers = [
        threading.Thread(target=run, args=(setter,), daemon=True)
        for setter in (False, True)
    ]
    for worker in workers:
        worker.start()
    remaining = []
    for _ in range(rounds):
        connection.autocommit = True
        connection.autocommit = sqlite3.LEGACY_TRANSACTION_CONTROL
        barrier.wait()
        barrier.wait()
        connection.rollback()
        remaining.append(connection.execute("SELECT count(*) FROM t").fetchone()[0])
    for worker in workers:
        worker.join(5)
        assert not worker.is_alive()
    assert not failures, failures
    assert not any(remaining), remaining
    connection.close()


script_stops_after_close()
concurrent_sqlite_results()
concurrent_sqlite_transactions()
