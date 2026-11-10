from io import BytesIO


def test_01():
    bytes_string = b"Test String 1"

    f = BytesIO()
    f.write(bytes_string)

    assert f.tell() == len(bytes_string)
    assert f.getvalue() == bytes_string


def test_02():
    bytes_string = b"Test String 2"
    f = BytesIO(bytes_string)

    assert f.read() == bytes_string
    assert f.read() == b""


def test_03():
    """
    Tests that the read method (integer arg)
    returns the expected value
    """
    string = b"Test String 3"
    f = BytesIO(string)

    assert f.read(1) == b"T"
    assert f.read(1) == b"e"
    assert f.read(1) == b"s"
    assert f.read(1) == b"t"


def test_04():
    """
    Tests that the read method increments the
    cursor position and the seek method moves
    the cursor to the appropriate position
    """
    string = b"Test String 4"
    f = BytesIO(string)

    assert f.read(4) == b"Test"
    assert f.tell() == 4
    assert f.seek(0) == 0
    assert f.read(4) == b"Test"


def test_05():
    """
    Tests that the write method accepts bytearray
    """
    bytes_string = b"Test String 5"

    f = BytesIO()
    f.write(bytearray(bytes_string))

    assert f.getvalue() == bytes_string


def test_06():
    """
    Tests readline
    """
    bytes_string = b"Test String 6\nnew line is here\nfinished"

    f = BytesIO(bytes_string)

    assert f.readline() == b"Test String 6\n"
    assert f.readline() == b"new line is here\n"
    assert f.readline() == b"finished"
    assert f.readline() == b""


def test_07():
    """
    Tests that flush() returns None when the file is open,
    and raises ValueError when the file is closed.
    CPython reference: Modules/_io/bytesio.c:325-335
    """
    f = BytesIO(b"Test String 7")

    # flush() on an open BytesIO returns None
    assert f.flush() is None

    # flush() is defined directly on BytesIO (not just inherited)
    assert "flush" in BytesIO.__dict__

    f.close()

    # flush() on a closed BytesIO raises ValueError
    try:
        f.flush()
        assert False, "Expected ValueError not raised"
    except ValueError:
        pass


if __name__ == "__main__":
    test_01()
    test_02()
    test_03()
    test_04()
    test_05()
    test_06()
    test_07()


# Reading into a buffer that views this same object locks it twice unless the
# read finishes first.
_bio = BytesIO(b"x" * 60)
assert _bio.readinto(_bio.getbuffer()) == 60
_bio = BytesIO(b"x" * 60)
assert _bio.readinto(memoryview(_bio.getbuffer())) == 60


def test_readlines_bytesio_semantics():
    import inspect
    import sys

    assert BytesIO.readlines.__text_signature__ == "($self, size=None, /)"
    assert str(inspect.signature(BytesIO.readlines)) == "(self, size=None, /)"

    class IndexOnly:
        def __index__(self):
            raise AssertionError("BytesIO.readlines must not call __index__")

    class Integer(int):
        def __index__(self):
            raise AssertionError("must use the stored integer value")

        def __int__(self):
            raise AssertionError("must use the stored integer value")

    # The positive limit is inclusive, unlike IOBase.readlines' hint.
    for size, expected in (
        (None, [b"a\n", b"bb\n", b"tail"]),
        (-1, [b"a\n", b"bb\n", b"tail"]),
        (0, [b"a\n", b"bb\n", b"tail"]),
        (False, [b"a\n", b"bb\n", b"tail"]),
        (True, [b"a\n"]),
        (2, [b"a\n"]),
        (Integer(2), [b"a\n"]),
        (3, [b"a\n", b"bb\n"]),
        (5, [b"a\n", b"bb\n"]),
        (sys.maxsize, [b"a\n", b"bb\n", b"tail"]),
    ):
        stream = BytesIO(b"a\nbb\ntail")
        assert stream.readlines(size) == expected, size
        assert stream.tell() == sum(map(len, expected)), size
    assert BytesIO(b"a\nb").readlines() == [b"a\n", b"b"]

    for value in (IndexOnly(), 1.5, "2"):
        stream = BytesIO(b"a\nb")
        try:
            stream.readlines(value)
        except TypeError as exc:
            assert str(exc) == (
                f"integer argument expected, got '{type(value).__name__}'"
            )
        else:
            raise AssertionError("non-int size accepted")
        assert stream.tell() == 0

    for value in (sys.maxsize + 1, -sys.maxsize - 2):
        stream = BytesIO(b"a\nb")
        try:
            stream.readlines(value)
        except OverflowError as exc:
            assert str(exc) == "Python int too large to convert to C ssize_t"
        else:
            raise AssertionError("overflowing size accepted")
        assert stream.tell() == 0

    # Closed state beats type/range conversion, but call-shape errors beat it.
    stream = BytesIO()
    stream.close()
    for value in (None, IndexOnly(), "2", sys.maxsize + 1):
        try:
            stream.readlines(value)
        except ValueError as exc:
            assert str(exc) == "I/O operation on closed file."
        else:
            raise AssertionError("closed stream accepted")
    for args, kwargs in (((1, 2), {}), ((), {"size": None})):
        try:
            stream.readlines(*args, **kwargs)
        except TypeError:
            pass
        else:
            raise AssertionError("invalid call shape accepted")

    class Overrides(BytesIO):
        def readline(self, *args):
            raise AssertionError("readline override called")

        def read(self, *args):
            raise AssertionError("read override called")

        def __iter__(self):
            raise AssertionError("iterator override called")

        def __next__(self):
            raise AssertionError("next override called")

    stream = Overrides(b"a\nbb\ntail")
    view = stream.getbuffer()
    assert stream.readlines(2) == [b"a\n"]
    assert stream.readlines() == [b"bb\n", b"tail"]
    assert bytes(view) == b"a\nbb\ntail"
    view.release()
    stream.seek(1)
    assert stream.readlines(1) == [b"\n"]
    assert stream.tell() == 2
    stream.seek(100)
    assert stream.readlines() == []
    assert stream.tell() == 100
    assert BytesIO().readlines() == []

    # Retain an int subclass argument through the read. Its finalizer may
    # reenter this stream and must not run with the storage lock held.
    positions = []
    stream = BytesIO(b"a\nb")

    class TemporarySize(int):
        def __del__(self):
            positions.append(stream.tell())

    assert stream.readlines(TemporarySize(2)) == [b"a\n"]
    assert positions == [2]


test_readlines_bytesio_semantics()


# BytesIO's non-int error uses tp_name, including static-type module names,
# without the truncation used by some other IO diagnostics.
import datetime

for value, expected_name in (
    (datetime.date(2026, 1, 1), "datetime.date"),
    (type("L" * 300, (), {})(), "L" * 300),
):
    try:
        BytesIO().readlines(value)
    except TypeError as exc:
        assert str(exc) == f"integer argument expected, got '{expected_name}'"
    else:
        raise AssertionError("non-int size accepted")
