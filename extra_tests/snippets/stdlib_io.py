import os
import sys
from io import BufferedReader, BytesIO, FileIO, RawIOBase, StringIO, TextIOWrapper

from testutils import assert_raises

# Overwriting replaces characters, even when their encoded widths differ.
for initial, replacement in (
    ("abc", "é"),
    ("ébc", "a"),
    ("а😀в", "XY"),
    ("a\ud800c", "\udfffé"),
):
    stream = StringIO("prefix:" + initial)
    stream.seek(7)
    assert stream.write(replacement) == len(replacement)
    assert stream.getvalue() == "prefix:" + replacement + initial[len(replacement) :]
    assert stream.tell() == 7 + len(replacement)
    assert stream.read() == initial[len(replacement) :]
    assert stream.tell() == 7 + len(initial)

# Truncation keeps the character position, then pads in characters on writing.
stream = StringIO("é😀abc")
stream.seek(4)
stream.truncate(1)
assert stream.tell() == 4
assert stream.read() == ""
assert stream.tell() == 4
assert stream.write("") == 0
assert stream.getvalue() == "é"
state = stream.__getstate__()
stream = StringIO()
stream.__setstate__(state)
assert stream.tell() == 4
stream.write("Ж")
assert stream.getvalue() == "é\0\0\0Ж"
assert stream.tell() == 5


fi = FileIO("README.md")
assert fi.seekable()
bb = BufferedReader(fi)
assert bb.seekable()

result = bb.read()

assert len(result) <= 16 * 1024
assert len(result) >= 0
assert isinstance(result, bytes)

with FileIO("README.md") as fio:
    res = fio.read()
    assert len(res) <= 16 * 1024
    assert len(res) >= 0
    assert isinstance(res, bytes)

fd = os.open("README.md", os.O_RDONLY)

with FileIO(fd) as fio:
    res2 = fio.read()
    assert res == res2

fi = FileIO("README.md")
fi.read()
fi.close()
assert fi.closefd
assert fi.closed

with assert_raises(ValueError):
    fi.read()

with FileIO("README.md") as fio:
    nres = fio.read(1)
    assert len(nres) == 1
    nres = fio.read(2)
    assert len(nres) == 2


# Test that IOBase.isatty() raises ValueError when called on a closed file.
# Minimal subclass that inherits IOBase.isatty() without overriding it.
class MinimalRaw(RawIOBase):
    def readinto(self, b):
        return 0


f = MinimalRaw()
assert not f.closed
assert not f.isatty()  # open file: should return False

f.close()
assert f.closed

with assert_raises(ValueError):
    f.isatty()


class Gh6588:
    def __init__(self):
        self.textio = None
        self.closed = False

    def writable(self):
        return True

    def readable(self):
        return False

    def seekable(self):
        return False

    def write(self, data):
        self.textio.reconfigure(encoding="utf-8")
        return len(data)


raw = Gh6588()
textio = TextIOWrapper(raw, encoding="utf-8", write_through=True)
raw.textio = textio
with assert_raises(AttributeError):
    textio.writelines(["x"])


def test_textio_reconfigure():
    import struct
    from io import UnsupportedOperation

    # reconfigure distinguishes an explicitly supplied value from omission.
    with TextIOWrapper(BytesIO(), encoding="ascii", errors="replace") as stream:
        stream.reconfigure(encoding="ascii")
        assert stream.errors == "strict"
        with assert_raises(UnicodeEncodeError):
            stream.write("é")

    for options in ({"encoding": "ascii"}, {"errors": "replace"}, {"newline": None}):
        with TextIOWrapper(
            BytesIO(b"abc"), encoding="ascii", errors="replace"
        ) as stream:
            assert stream.read(1) == "a"
            with assert_raises(UnsupportedOperation):
                stream.reconfigure(**options)
            stream.reconfigure(encoding=None, errors=None, line_buffering=True)
            assert stream.line_buffering is True
            assert stream.read() == "bc"
            stream.reconfigure(**options)

    class RecordingFlush(TextIOWrapper):
        flush_count = 0

        def flush(self):
            self.flush_count += 1
            return super().flush()

    with RecordingFlush(BytesIO(), encoding="ascii") as stream:
        stream.reconfigure()
        assert stream.flush_count == 1

    class WideIndex:
        def __index__(self):
            return 1 << 32

    with TextIOWrapper(BytesIO(), encoding="ascii") as stream:
        for flag in ("line_buffering", "write_through"):
            if struct.calcsize("l") > 4:
                stream.reconfigure(**{flag: WideIndex()})
                assert getattr(stream, flag) is True
            else:
                with assert_raises(
                    OverflowError, _msg="Python int too large to convert to C long"
                ):
                    stream.reconfigure(**{flag: WideIndex()})
            with assert_raises(
                OverflowError, _msg="Python int too large to convert to C long"
            ):
                stream.reconfigure(**{flag: 1 << 80})


test_textio_reconfigure()

textio = TextIOWrapper(BytesIO())

for invalid_chunk_size in (0, -1, 2**100):
    try:
        textio._CHUNK_SIZE = invalid_chunk_size
    except ValueError:
        pass
    else:
        raise AssertionError(f"expected ValueError for {invalid_chunk_size!r}")

for invalid_chunk_size in (1.5, "4"):
    try:
        textio._CHUNK_SIZE = invalid_chunk_size
    except TypeError:
        pass
    else:
        raise AssertionError(f"expected TypeError for {invalid_chunk_size!r}")


class ChunkSize:
    def __index__(self):
        return 16


textio._CHUNK_SIZE = ChunkSize()
assert textio._CHUNK_SIZE == 16


class OversizedChunkSize:
    def __index__(self):
        return 2**100


try:
    textio._CHUNK_SIZE = OversizedChunkSize()
except ValueError as error:
    expected = "cannot fit 'OversizedChunkSize' into an index-sized integer"
    if str(error) != expected:
        raise AssertionError(f"unexpected error message: {error}") from error
else:
    raise AssertionError("expected ValueError for oversized indexable object")


def expect_value_error(expected, operation):
    try:
        operation()
    except ValueError as error:
        if str(error) != expected:
            raise AssertionError(f"unexpected error message: {error}") from error
    else:
        raise AssertionError(f"expected ValueError: {expected}")


class UninitializedChunkSize:
    def __init__(self):
        self.called = False

    def __index__(self):
        self.called = True
        return 16


uninitialized_textio = TextIOWrapper.__new__(TextIOWrapper)
uninitialized_chunk_size = UninitializedChunkSize()
expect_value_error(
    "I/O operation on uninitialized object",
    lambda: setattr(uninitialized_textio, "_CHUNK_SIZE", uninitialized_chunk_size),
)

if uninitialized_chunk_size.called:
    raise AssertionError(
        "__index__ should not be called for uninitialized TextIOWrapper"
    )


detached_textio = TextIOWrapper(BytesIO())
detached_textio.detach()
expect_value_error(
    "underlying buffer has been detached",
    lambda: setattr(detached_textio, "_CHUNK_SIZE", 16),
)
# CI also runs this snippet on CPython 3.15.0, whose setter checks
# attachment before deletion. RustPython targets the 3.15.2 Clinic contract.
legacy_chunk_size_deletion = sys.implementation.name == "cpython" and sys.version_info[
    :3
] == (3, 15, 0)
for wrapper, legacy_error, legacy_message in (
    (textio, AttributeError, "cannot delete attribute"),
    (uninitialized_textio, ValueError, "I/O operation on uninitialized object"),
    (detached_textio, ValueError, "underlying buffer has been detached"),
):
    error_type = legacy_error if legacy_chunk_size_deletion else AttributeError
    message = (
        legacy_message
        if legacy_chunk_size_deletion
        else "attribute '_CHUNK_SIZE' of '_io.TextIOWrapper' objects cannot be deleted"
    )
    with assert_raises(error_type) as error:
        del wrapper._CHUNK_SIZE
    assert str(error.exception) == message


for wrapper_name in ("ChunkWrapper", "€" * 34):
    wrapper_type = type(wrapper_name, (TextIOWrapper,), {})
    wrapper = wrapper_type(BytesIO())
    with assert_raises(AttributeError) as error:
        del wrapper._CHUNK_SIZE
    truncated_name = wrapper_name.encode("utf-8")[:100].decode("utf-8", "ignore")
    message = (
        "cannot delete attribute"
        if legacy_chunk_size_deletion
        else f"attribute '_CHUNK_SIZE' of '{truncated_name}' objects cannot be deleted"
    )
    assert str(error.exception) == message
    wrapper.close()


long_type_name = "X" * 250
LongNamedChunkSize = type(
    long_type_name,
    (),
    {"__index__": lambda self: 2**100},
)
expect_value_error(
    f"cannot fit '{long_type_name[:200]}' into an index-sized integer",
    lambda: setattr(textio, "_CHUNK_SIZE", LongNamedChunkSize()),
)


non_ascii_type_name = "é" * 250
NonAsciiNamedChunkSize = type(
    non_ascii_type_name,
    (),
    {"__index__": lambda self: 2**100},
)
truncated_non_ascii_type_name = non_ascii_type_name.encode("utf-8")[:200].decode(
    "utf-8"
)
expect_value_error(
    f"cannot fit '{truncated_non_ascii_type_name}' into an index-sized integer",
    lambda: setattr(textio, "_CHUNK_SIZE", NonAsciiNamedChunkSize()),
)


# A buffer size or read size that cannot be allocated is a MemoryError, not an
# aborted process.
assert_raises(MemoryError, lambda: BufferedReader(BytesIO(b"a"), buffer_size=2**62))
assert_raises(MemoryError, lambda: BufferedReader(BytesIO(b"a")).read(2**62))
assert_raises(MemoryError, lambda: BufferedReader(BytesIO(b"a")).read1(2**62))


def _text_cookie(
    start_pos=0,
    dec_flags=0,
    bytes_to_feed=0,
    chars_to_skip=0,
    need_eof=0,
    bytes_to_skip=0,
):
    packed = (
        start_pos.to_bytes(8, "little", signed=True)
        + dec_flags.to_bytes(4, "little", signed=True)
        + bytes_to_feed.to_bytes(4, "little", signed=True)
        + chars_to_skip.to_bytes(4, "little", signed=True)
        + bytes([need_eof])
        + bytes_to_skip.to_bytes(4, "little", signed=True)
    )
    return int.from_bytes(packed, "little")


# A cookie names a position both in characters and in bytes, and everything
# read back from it indexes what was decoded, so a position past the end is
# refused rather than stored.
for _bad in (
    _text_cookie(bytes_to_feed=10, chars_to_skip=1000, bytes_to_skip=0),
    _text_cookie(bytes_to_feed=10, chars_to_skip=100000, bytes_to_skip=3),
    _text_cookie(bytes_to_feed=10, chars_to_skip=1, bytes_to_skip=1000),
):
    _textio = TextIOWrapper(BytesIO(b"hello world " * 20), encoding="utf-8")
    _textio.read(1)
    try:
        _textio.seek(_bad)
    except (OSError, OverflowError):
        pass
    else:
        assert _textio.read(50) is not None
        _textio.tell()


# Reinitialization releases arguments after unlocking the stream.
for _type, _content in ((BytesIO, b"hello"), (StringIO, "hello")):
    _stream = _type(_content)
    _stream.close()
    _stream.__init__(_content)
    assert not _stream.closed
    assert _stream.getvalue() == _content

    class ClosingValue(type(_content)):
        def __del__(self):
            _stream.close()

    _stream.__init__(ClosingValue(_content))
    assert _stream.closed


_stream = BytesIO(b"old")
_released_values = []


class ClosingBuffer:
    def __buffer__(self, flags):
        return memoryview(b"new")

    def __release_buffer__(self, view):
        _released_values.append(_stream.getvalue())
        _stream.close()


_stream.__init__(ClosingBuffer())
assert _released_values == [b"new"]
assert _stream.closed


# OSError.filename is the original FileIO path argument, including path-like objects.
class PathLikeName:
    def __fspath__(self):
        return "/nonexistent/x"


path_like_name = PathLikeName()
try:
    FileIO(path_like_name, "r")
except OSError as error:
    assert error.filename is path_like_name
else:
    raise AssertionError("expected OSError")

for original in ("/nonexistent/x", b"/nonexistent/x"):
    try:
        FileIO(original, "r")
    except OSError as error:
        assert error.filename is original
    else:
        raise AssertionError("expected OSError")

with FileIO(__file__, "rbb") as rbb:
    assert rbb.mode == "rb"


def test_buffered_stream_capabilities():
    from _io import BufferedRandom, BufferedReader, BufferedRWPair, BufferedWriter

    events = []
    result = object()

    class Raw(BytesIO):
        def __init__(self, label):
            super().__init__(b"data")
            self.label = label
            self.can_read = True
            self.can_write = True

        def readable(self):
            events.append((self.label, "readable"))
            return self.can_read

        def writable(self):
            events.append((self.label, "writable"))
            return self.can_write

    for wrapper, forwards_read, forwards_write in (
        (BufferedReader, True, False),
        (BufferedWriter, False, True),
        (BufferedRandom, True, True),
        (BufferedRWPair, True, True),
    ):
        reader = Raw("reader")
        writer = Raw("writer") if wrapper is BufferedRWPair else reader
        args = (reader, writer) if wrapper is BufferedRWPair else (reader,)
        with wrapper(*args) as stream:
            # Change the raw capabilities after initialization.
            reader.can_read = result
            writer.can_write = 0
            events.clear()
            assert stream.readable() is (result if forwards_read else False)
            assert stream.writable() is (writer.can_write if forwards_write else False)
            assert events == [
                (label, method)
                for flag, label, method in (
                    (forwards_read, reader.label, "readable"),
                    (forwards_write, writer.label, "writable"),
                )
                if flag
            ]

        plain_raw = BytesIO()
        args = (plain_raw, BytesIO()) if wrapper is BufferedRWPair else (plain_raw,)
        closed = wrapper(*args)
        closed.close()
        for forwarded, operation in (
            (forwards_read, closed.readable),
            (forwards_write, closed.writable),
        ):
            if forwarded:
                assert_raises(ValueError, operation)
            else:
                assert operation() is False

    with BytesIO() as raw:
        detached = BufferedRandom(raw)
        assert detached.detach() is raw
        for operation in (detached.readable, detached.writable):
            with assert_raises(ValueError) as error:
                operation()
            assert str(error.exception) == "raw stream has been detached"


def test_buffered_pair_isatty_forwarding():
    from _io import BufferedRWPair

    events = []
    failure = LookupError("isatty failure")

    class NotABoolean:
        def __bool__(self):
            raise AssertionError("BufferedRWPair must not truth-test the result")

    class Raw(BytesIO):
        def __init__(self, label):
            super().__init__()
            self.label = label
            self.result = False

        def isatty(self):
            events.append(self.label)
            if self.result is failure:
                raise failure
            return self.result

    reader, writer = Raw("reader"), Raw("writer")
    with BufferedRWPair(reader, writer) as pair:
        reader.result = failure
        for value in (True, 0, NotABoolean()):
            events.clear()
            writer.result = value
            assert pair.isatty() is value
            assert events == ["writer"]

        reader.result = NotABoolean()
        writer.result = False
        events.clear()
        assert pair.isatty() is reader.result
        assert events == ["writer", "reader"]

        events.clear()
        writer.result = failure
        with assert_raises(LookupError) as error:
            pair.isatty()
        assert error.exception is failure
        assert events == ["writer"]


test_buffered_stream_capabilities()
test_buffered_pair_isatty_forwarding()


def test_fileio_warning_preserves_source():
    import tempfile
    import warnings
    from io import BufferedWriter
    from pathlib import Path

    with tempfile.TemporaryDirectory() as directory:
        with FileIO(Path(directory) / "resource", "wb") as raw:
            # Buffered I/O forwards the outer stream as the warning source.
            with BufferedWriter(raw) as buffered:
                for source in (raw, buffered):
                    with warnings.catch_warnings(record=True) as records:
                        warnings.simplefilter("always", ResourceWarning)
                        raw._dealloc_warn(source)
                    assert len(records) == 1
                    assert records[0].category is ResourceWarning
                    assert records[0].source is source
        with warnings.catch_warnings(record=True) as records:
            warnings.simplefilter("always", ResourceWarning)
            raw._dealloc_warn(raw)
        assert records == []


test_fileio_warning_preserves_source()
