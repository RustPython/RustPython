"""Bounded CPython 3.15 base64 API regressions (no other binascii codecs)."""

import binascii

from testutils import assert_raises

encode = binascii.b2a_base64
decode = binascii.a2b_base64
alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
assert binascii.BASE64_ALPHABET == alphabet
assert binascii.URLSAFE_BASE64_ALPHABET == alphabet[:-2] + b"-_"

# RFC 4648 vectors and CPython's four-column wrapping normalization.
for raw, expected in (
    (b"", b""),
    (b"f", b"Zg=="),
    (b"fo", b"Zm8="),
    (b"foo", b"Zm9v"),
    (b"foob", b"Zm9vYg=="),
    (b"fooba", b"Zm9vYmE="),
    (b"foobar", b"Zm9vYmFy"),
    (b"www.python.org", b"d3d3LnB5dGhvbi5vcmc="),
):
    for factory in bytes, bytearray, memoryview:
        for padded in False, True:
            unwrapped = expected if padded else expected.rstrip(b"=")
            for width in 0, 1, 3, 4, 5, 7, 8, 11, 76:
                normalized = max(4, width // 4 * 4) if width else 1000
                wrapped = b"\n".join(
                    unwrapped[i : i + normalized]
                    for i in range(0, len(unwrapped), normalized)
                )
                for newline in False, True:
                    result = encode(
                        factory(raw), padded=padded, wrapcol=width, newline=newline
                    )
                    assert result == wrapped + (b"\n" if newline else b"")
                    assert (
                        decode(
                            factory(result),
                            padded=padded,
                            ignorechars=b"\n",
                            canonical=True,
                        )
                        == raw
                    )

# Input and custom alphabet asymmetry: decoding accepts ASCII str, but its
# alphabet must be bytes. Encoders accept any contiguous bytes-like alphabet.
raw = bytes(range(64))
standard = encode(raw, newline=False)
for custom in alphabet[::-1], alphabet[:-2] + b"-_", bytes(range(128, 192)):
    expected = standard.translate(bytes.maketrans(alphabet, custom))
    for factory in bytes, bytearray, memoryview:
        assert encode(raw, alphabet=factory(custom), newline=False) == expected
    assert decode(expected, alphabet=custom, canonical=True) == raw
assert decode("Zm9v", alphabet=alphabet) == b"foo"
assert encode(b"\0", alphabet=b"A" * 64, newline=False) == b"AA=="
assert decode(b"AA==", alphabet=b"A" * 64) == b"\xff"
assert encode(b"\0", alphabet=b"=" * 64, newline=False) == b"===="
assert decode(b"====", alphabet=b"=" * 64) == b""

# Supplied ignorechars enables strictness by default; alphabet characters are
# never ignored. Explicit non-strict mode still ignores arbitrary non-alphabet.
assert decode(b" Zm9v!", strict_mode=False, ignorechars=b"") == b"foo"
assert decode(b" Zm9v!", ignorechars=b" !") == b"foo"
assert decode(b"Zm9v", ignorechars=b"Zm9v") == b"foo"
for value in b" ", b"!", b"\xff":
    assert decode(value) == b""
    with assert_raises(binascii.Error):
        decode(value, ignorechars=b"")

for value, message in (
    (b"=", "Leading padding not allowed"),
    (b" =Zm9v", "Leading padding not allowed"),
    (b"Zm9v=", "Excess padding not allowed"),
    (b"Zg===", "Excess padding not allowed"),
    (b"Zg=a", "Discontinuous padding not allowed"),
    (b"Zg==a", "Excess data after padding"),
    (b"Zg", "Incorrect padding"),
    (b"!", "Only base64 data is allowed"),
):
    try:
        decode(value, ignorechars=b" ")
    except binascii.Error as error:
        assert str(error) == message
    else:
        assert False, (value, message)

for value, expected in (b"YQ==", b"a"), (b"YWI=", b"ab"), (b"YWJj", b"abc"):
    assert decode(value.replace(b"=", b""), padded=False, strict_mode=True) == expected
    assert decode(value, padded=False, ignorechars=b"=") == expected
    if b"=" in value:
        try:
            decode(value, padded=False, strict_mode=True)
        except binascii.Error as error:
            assert str(error) == "Padding not allowed"
        else:
            assert False
assert decode(b"Y=WJj", ignorechars=b"=") == b"abc"
assert decode(b"YQ==YQ==") == b"a\x06\x10"
for value in b"AB==", b"AP==", b"AAB=", b"AAD=":
    decode(value)
    for padded in False, True:
        try:
            decode(
                value if padded else value.rstrip(b"="), padded=padded, canonical=True
            )
        except binascii.Error as error:
            assert str(error) == "Non-zero padding bits"
        else:
            assert False
for value in b"A", b"A=", b"AAAAA":
    with assert_raises(binascii.Error):
        decode(value, padded=False)

for function in encode, decode:
    for invalid in b"", alphabet[:-1], alphabet + b"?":
        with assert_raises(ValueError):
            function(b"", alphabet=invalid)
    for invalid in None, "", []:
        with assert_raises(TypeError):
            function(b"", alphabet=invalid)
    with assert_raises(TypeError):
        function(data=b"")
    with assert_raises(TypeError):
        function(b"", False)
for invalid in bytearray(alphabet), memoryview(alphabet):
    with assert_raises(TypeError):
        decode(b"", alphabet=invalid)
for invalid in None, "", []:
    with assert_raises(TypeError):
        decode(b"", ignorechars=invalid)
with assert_raises(ValueError):
    encode(b"", wrapcol=-1)
with assert_raises(OverflowError):
    encode(b"", wrapcol=1 << 128)
with assert_raises(TypeError):
    encode(b"", wrapcol=4.0)
with assert_raises(ValueError):
    decode("é")


class Width:
    def __index__(self):
        return 4


assert encode(b"foobar", wrapcol=Width(), newline=False) == b"Zm9v\nYmFy"

# Conversion order is independent of keyword order and canonical runs after
# ignorechars acquisition, before alphabet length validation.
events = []


class Flag:
    def __init__(self, name, value):
        self.name = name
        self.value = value

    def __bool__(self):
        events.append(self.name)
        return self.value


assert (
    encode(b"f", newline=Flag("newline", False), padded=Flag("padded", False)) == b"Zg"
)
assert events == ["padded", "newline"]
events.clear()
assert (
    decode(
        b"Zg",
        canonical=Flag("canonical", True),
        padded=Flag("padded", False),
        strict_mode=Flag("strict", True),
    )
    == b"f"
)
assert events == ["strict", "padded", "canonical"]
events.clear()
with assert_raises(ValueError):
    decode(b"", alphabet=b"", canonical=Flag("canonical", False))
assert events == ["canonical"]


# The decoder cache uses the original bytes object, including subclass hooks.
class CachedAlphabet(bytes):
    def __hash__(self):
        events.append("hash")
        return 314159265

    def __eq__(self, other):
        events.append("equal")
        return bytes.__eq__(self, other)


custom = CachedAlphabet(alphabet[1:] + alphabet[:1])
encoded = encode(b"abc", alphabet=custom, newline=False)
events.clear()
assert decode(encoded, alphabet=custom) == b"abc"
assert events == ["hash", "hash"]
events.clear()
assert decode(encoded, alphabet=custom) == b"abc"
assert events == ["hash"]
events.clear()
assert decode(encoded, alphabet=CachedAlphabet(custom)) == b"abc"
assert events == ["hash", "equal"]


class UnhashableAlphabet(bytes):
    __hash__ = None


with assert_raises(TypeError):
    decode(b"", alphabet=UnhashableAlphabet(alphabet))
with assert_raises(ValueError):
    decode(b"", alphabet=UnhashableAlphabet(b""))
assert encode(b"abc", alphabet=UnhashableAlphabet(alphabet)) == b"YWJj\n"


class HashError(Exception):
    pass


class BrokenAlphabet(bytes):
    def __hash__(self):
        raise HashError


with assert_raises(HashError):
    decode(b"!", alphabet=BrokenAlphabet(alphabet), strict_mode=True)

# Keep the exact owning module alive through a saved builtin, independently of
# sys.modules. A fresh module's cache cycle must remain visible to the collector.
import gc
import importlib.util
import sys
import weakref

assert decode.__self__ is binascii
assert decode.__module__ == "binascii"
assert decode.__name__ == "a2b_base64"
original = sys.modules.pop("binascii")
try:
    assert decode(encoded, alphabet=custom) == b"abc"
finally:
    sys.modules["binascii"] = original

spec = importlib.util.find_spec("binascii")
original = sys.modules.pop("binascii")
try:
    isolated = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(isolated)
finally:
    sys.modules["binascii"] = original
assert isolated is not binascii
assert isolated.a2b_base64.__self__ is isolated
cycle_key = CachedAlphabet(alphabet)
cycle_key.owner = isolated
assert isolated.a2b_base64(b"YWJj", alphabet=cycle_key) == b"abc"
module_ref = weakref.ref(isolated)
del cycle_key, isolated
for _ in range(3):
    gc.collect()
assert module_ref() is None
assert not hasattr(binascii, "_reverse_table_cache")


# Hash callbacks can safely reenter the same module before decoding begins.
class ReentrantAlphabet(bytes):
    def __hash__(self):
        assert decode(b"Zm9v") == b"foo"
        assert encode(b"foo", wrapcol=4) == b"Zm9v\n"
        events.append("reenter")
        return 271828182


events.clear()
assert decode(b"YWJj", alphabet=ReentrantAlphabet(alphabet)) == b"abc"
assert events == ["reenter", "reenter"]
assert "ignorechars=<unrepresentable>" in decode.__text_signature__
assert "wrapcol=0" in encode.__text_signature__


# Acquired input and auxiliary exports survive conversions and cache callbacks,
# and are released in Clinic order, including a canonical conversion failure.
class Export:
    def __init__(self, name, data):
        self.name = name
        self.data = data

    def __buffer__(self, flags):
        events.append("acquire " + self.name)
        return memoryview(self.data)

    def __release_buffer__(self, view):
        events.append("release " + self.name)


class BrokenFlag:
    def __bool__(self):
        events.append("canonical error")
        raise HashError


events.clear()
assert (
    encode(Export("data", b"foo"), alphabet=Export("alphabet", alphabet)) == b"Zm9v\n"
)
assert events == [
    "acquire data",
    "acquire alphabet",
    "release data",
    "release alphabet",
]
events.clear()
assert decode(Export("data", b"Zm9v"), ignorechars=Export("ignored", b"")) == b"foo"
assert events == ["acquire data", "acquire ignored", "release data", "release ignored"]
events.clear()
with assert_raises(HashError):
    decode(
        Export("data", b""), ignorechars=Export("ignored", b""), canonical=BrokenFlag()
    )
assert events == [
    "acquire data",
    "acquire ignored",
    "canonical error",
    "release data",
    "release ignored",
]

# Aliased buffer exporters do not require nested backing-store locks.
shared = bytearray(alphabet)
assert encode(shared, alphabet=shared) == encode(bytes(shared), alphabet=bytes(shared))
shared = bytearray(b"Zm9v")
assert decode(shared, ignorechars=shared) == b"foo"

for length in (0, 1, 2, 3, 4, 5, 56, 57, 58, 63, 64, 65, 255, 256, 257):
    payload = (bytes(range(256)) * 2)[:length]
    for padded in False, True:
        for width in 0, 4, 76:
            encoded = encode(payload, padded=padded, wrapcol=width)
            assert (
                decode(encoded, padded=padded, ignorechars=b"\n", canonical=True)
                == payload
            )


# Bound module functions retain their metadata for module subclasses too.
class ModuleSubclass(type(binascii)):
    pass


original_class = binascii.__class__
try:
    binascii.__class__ = ModuleSubclass
    assert repr(decode) == "<built-in function a2b_base64>"
    assert decode.__qualname__ == decode.__name__ == "a2b_base64"
    assert decode.__self__ is binascii
    assert decode.__reduce__() == "a2b_base64"
finally:
    binascii.__class__ = original_class
