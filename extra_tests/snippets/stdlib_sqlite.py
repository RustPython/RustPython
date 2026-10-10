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

    with test.assertRaisesRegex(OverflowError, "^index conversion failed$"):
        blob[BrokenBlobIndex()]
    with test.assertRaisesRegex(OverflowError, "^index conversion failed$"):
        blob[BrokenBlobIndex()] = 0
    assert blob[:] == b"Abc"
