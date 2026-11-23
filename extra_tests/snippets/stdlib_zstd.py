import random
import sys
import types

import _zstd
from compression.zstd import (
    CompressionParameter,
    ZstdCompressor,
    ZstdDecompressor,
    ZstdDict,
    ZstdError,
    compress,
    decompress,
    get_frame_size,
)
from testutils import assert_raises


def raises(expected, message, function, *args, **kwargs):
    with assert_raises(expected) as caught:
        function(*args, **kwargs)
    assert str(caught.exception) == message, str(caught.exception)


class Index:
    def __init__(self, value):
        self.value = value

    def __index__(self):
        return self.value


class IntSubclass(int):
    def __index__(self):
        raise AssertionError("__index__ should not be called")

    def __int__(self):
        raise AssertionError("__int__ should not be called")


class TupleSubclass(tuple):
    pass


payload = b"hello world" * 100
frame = compress(payload)
skippable = b"\x50\x2a\x4d\x18\x04\x00\x00\x00abcd"
raw_dict = ZstdDict(b"0123456789abcdef" * 8, is_raw=True)

# max_length=0 must not drop output bytes: probing with a zero cap has to
# leave every byte available to later drain calls (regression: the probe
# used to emit one byte per call into the void).
d = ZstdDecompressor()
out = d.decompress(frame, 0)
assert out == b""
assert not d.eof
assert not d.needs_input
rest = d.decompress(b"", len(payload))
assert rest == payload
assert d.eof

# two zero-cap probes in a row, then drain; still lossless
d = ZstdDecompressor()
assert d.decompress(frame, 0) == b""
assert d.decompress(b"", 0) == b""
assert d.decompress(b"", len(payload)) == payload
assert d.eof

# zero-cap probe on a truncated frame: no output, not at end
d = ZstdDecompressor()
out = d.decompress(frame[:-5], 0)
assert out == b""
assert not d.eof
assert not d.needs_input

# skippable frame followed by trailing bytes lands in unused_data
d = ZstdDecompressor()
assert d.decompress(skippable + b"trail", 0) == b""
assert d.eof
assert d.unused_data == b"trail"

# Input a call could not consume is kept and used first by the next one,
# whether that one brings more data (even though needs_input is False) or
# none.
data = bytes(range(256)) * 2000
stream = compress(data, level=1) + b"trailing"
d = ZstdDecompressor()
out = b""
for piece in (stream[:100], stream[100:200], stream[200:]):
    out += d.decompress(piece, 1000)
assert not d.needs_input
while not d.eof:
    out += d.decompress(b"", 50000)
assert out == data
assert d.unused_data == b"trailing"

# compress() hands back everything that is ready: here two complete blocks
# of incompressible data, not just what fit some internal buffer.
blocks = random.Random(7).randbytes(2 * 128 * 1024)
c = ZstdCompressor()
out = c.compress(blocks)
assert len(out) >= len(blocks), len(out)
assert decompress(out + c.flush()) == blocks

# The classes and the exception are defined in _zstd but belong to the
# package they are public in.
for public in (ZstdCompressor, ZstdDecompressor, ZstdDict, ZstdError):
    assert public.__module__ == "compression.zstd", public
assert ZstdError.__doc__ == "An error occurred in the zstd library."

# level has to be an int; the entries of an options dict may use __index__
raises(TypeError, "invalid type for level, expected int", ZstdCompressor, Index(3))
raises(TypeError, "invalid type for level, expected int", ZstdCompressor, level=3.0)
ZstdCompressor(options={Index(_zstd.ZSTD_c_windowLog): Index(20)})
raises(
    TypeError,
    "ZstdCompressor() argument 'options' must be dict, not list",
    ZstdCompressor,
    options=[],
)
raises(
    TypeError,
    "ZstdDecompressor() argument 'options' must be dict, not list",
    ZstdDecompressor,
    options=[],
)

# the compression level gets the same error from either argument
low, high = CompressionParameter.compression_level.bounds()
message = f"illegal compression level {high + 1}; the valid range is [{low}, {high}]"
raises(ValueError, message, ZstdCompressor, high + 1)
raises(
    ValueError,
    message,
    ZstdCompressor,
    options={CompressionParameter.compression_level: high + 1},
)

# every other parameter is checked by libzstd
for strategy in (-1, 10, -(2**31), 2**31 - 1):
    with assert_raises(ValueError):
        ZstdCompressor(options={CompressionParameter.strategy: strategy})
for strategy in (0, 1, 9):
    ZstdCompressor(options={CompressionParameter.strategy: strategy})

# mode is an int, with no None stand-in for the default
c = ZstdCompressor()
with assert_raises(TypeError):
    c.compress(b"", None)
with assert_raises(TypeError):
    c.flush(None)
raises(
    ValueError,
    "mode argument wrong value, it should be one of "
    "ZstdCompressor.CONTINUE, ZstdCompressor.FLUSH_BLOCK, "
    "ZstdCompressor.FLUSH_FRAME.",
    c.compress,
    b"",
    3,
)
raises(
    ValueError,
    "mode argument wrong value, it should be "
    "ZstdCompressor.FLUSH_FRAME or ZstdCompressor.FLUSH_BLOCK.",
    c.flush,
    ZstdCompressor.CONTINUE,
)

# the pledged size has to be an int
raises(TypeError, "an integer is required", c.set_pledged_input_size, Index(5))
raises(TypeError, "an integer is required", c.set_pledged_input_size, 5.0)
c.set_pledged_input_size(True)

# zstd_dict is a ZstdDict or exactly the tuple one of its as_* properties
# returns
message = "zstd_dict argument should be a ZstdDict object."
for bad in (
    TupleSubclass(raw_dict.as_prefix),
    (raw_dict, Index(1)),
    (raw_dict, 1.0),
    (1, raw_dict),
):
    raises(TypeError, message, ZstdCompressor, zstd_dict=bad)
    raises(TypeError, message, ZstdDecompressor, zstd_dict=bad)
ZstdCompressor(zstd_dict=(raw_dict, True))

# a decompressor looks at its dictionary before its options
raises(TypeError, message, ZstdDecompressor, zstd_dict=123, options={12345: 1})

# ZstdDict
raises(ValueError, "invalid Zstandard dictionary", ZstdDict, b"12345678")
assert ZstdDict(b"12345678", is_raw="yes").dict_id == 0

# A digested dictionary is built once per compression level and then shared.
samples = [b"sample %d of some text to compress" % i for i in range(64)]
for level in (1, 5, 1, 5):
    for _ in range(2):
        out = compress(samples[0], level=level, zstd_dict=raw_dict.as_digested_dict)
        assert decompress(out, zstd_dict=raw_dict) == samples[0]

# errors from libzstd say what was being attempted
with assert_raises(ZstdError) as caught:
    ZstdDecompressor().decompress(b"not a zstd frame")
assert str(caught.exception).startswith("Unable to decompress Zstandard data: ")
with assert_raises(ZstdError) as caught:
    _zstd.get_param_bounds(12345, True)
assert str(caught.exception).startswith(
    "Unable to get zstd compression parameter bounds: "
)
with assert_raises(ZstdError) as caught:
    _zstd.get_param_bounds(12345, False)
assert str(caught.exception).startswith(
    "Unable to get zstd decompression parameter bounds: "
)
with assert_raises(ZstdError) as caught:
    _zstd.train_dict(b"", (), 100)
assert str(caught.exception).startswith("Unable to train the Zstandard dictionary: ")
with assert_raises(ZstdError) as caught:
    _zstd.finalize_dict(b"", b"", (), 100, 1)
assert str(caught.exception).startswith("Unable to finalize the Zstandard dictionary: ")

# module functions take their arguments by keyword too, and is_compress is
# any truth value
assert get_frame_size(frame_buffer=frame) == len(frame)
assert _zstd.get_frame_info(frame_buffer=frame) == (len(payload), 0)
assert _zstd.get_param_bounds(parameter=100, is_compress="yes") == (low, high)
assert _zstd.get_param_bounds(100, []) == _zstd.get_param_bounds(100, False)

# train_dict() and finalize_dict() check their arguments in a fixed order
message = "dict_size argument should be positive number."
raises(ValueError, message, _zstd.train_dict, b"", (1.5,), 0)
raises(ValueError, message, _zstd.finalize_dict, b"", b"", (1.5,), -1, 1)
with assert_raises(TypeError):
    _zstd.finalize_dict(b"", b"", (), 0, 1.5)

# The sample sizes are ints (subclasses included) that are used as they are,
# and they have to add up to the size of the samples.
message = "The samples size tuple doesn't match the concatenation's size."
for sizes in ((4, -1), (2,), (2**1000,), (-(2**1000),), (99, 1.5)):
    raises(ValueError, message, _zstd.train_dict, b"abc", sizes, 100)
    raises(ValueError, message, _zstd.finalize_dict, b"", b"abc", sizes, 300, 1)
for sizes in ((Index(3),), (1.5, 99), ("3",)):
    raises(TypeError, "an integer is required", _zstd.train_dict, b"abc", sizes, 100)
    raises(
        TypeError,
        "an integer is required",
        _zstd.finalize_dict,
        b"",
        b"abc",
        sizes,
        300,
        1,
    )
for size in (IntSubclass(1), True):
    # One byte is too little to train on, but the size itself is accepted.
    with assert_raises(ZstdError):
        _zstd.train_dict(b"x", (size,), 256)
    try:
        _zstd.finalize_dict(b"", b"x", (size,), 256, 1)
    except ZstdError:
        pass

# Module state does not leak into _zstd.__dict__, and mutating sys.modules or
# _zstd.ZstdError does not break raising ZstdError.
assert "_compression_parameter_type" not in dir(_zstd)
assert "_decompression_parameter_type" not in dir(_zstd)

saved_module = sys.modules["_zstd"]
saved_error = _zstd.ZstdError
try:
    sys.modules["_zstd"] = object()
    with assert_raises(saved_error):
        ZstdDecompressor().decompress(b"not a zstd frame")
    sys.modules["_zstd"] = saved_module
    _zstd.ZstdError = 123
    with assert_raises(saved_error):
        ZstdDecompressor().decompress(b"not a zstd frame")
finally:
    sys.modules["_zstd"] = saved_module
    _zstd.ZstdError = saved_error

# Member descriptors match CPython's read-only PyMemberDef attributes.
for desc in (
    ZstdDict.dict_id,
    ZstdCompressor.last_mode,
    ZstdDecompressor.eof,
    ZstdDecompressor.needs_input,
):
    assert isinstance(desc, types.MemberDescriptorType), type(desc)
raises(AttributeError, "readonly attribute", setattr, raw_dict, "dict_id", 1)
raises(AttributeError, "readonly attribute", setattr, ZstdCompressor(), "last_mode", 0)
raises(AttributeError, "readonly attribute", setattr, ZstdDecompressor(), "eof", True)
raises(
    AttributeError,
    "readonly attribute",
    setattr,
    ZstdDecompressor(),
    "needs_input",
    False,
)

# Negative nb_workers clamps to 0 (single-threaded), and a later key setting
# nb_workers=0 overrides an earlier non-zero entry cleanly.
for opts in (
    {CompressionParameter.nb_workers: -1},
    {Index(_zstd.ZSTD_c_nbWorkers): 2, Index(_zstd.ZSTD_c_nbWorkers): 0},
):
    c = ZstdCompressor(options=opts)
    assert decompress(c.compress(payload) + c.flush()) == payload

# unused_data is cached once EOF is reached, including with empty trailing data.
d = ZstdDecompressor()
assert d.decompress(frame + b"tail") == payload
assert d.eof
first_unused = d.unused_data
assert first_unused == b"tail"
assert d.unused_data is first_unused

d_empty = ZstdDecompressor()
assert d_empty.decompress(frame) == payload
assert d_empty.eof
assert d_empty.unused_data == b""
assert d_empty.unused_data is d_empty.unused_data
