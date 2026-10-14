# Shared wide / escape codecs from rustpython-common.

assert "hi".encode("utf-16-le") == b"h\x00i\x00"
assert "hi".encode("utf-16-be") == b"\x00h\x00i"
assert b"h\x00i\x00".decode("utf-16-le") == "hi"
assert b"\x00h\x00i".decode("utf-16-be") == "hi"

# BOM form: native utf-16 writes a BOM and decodes it back.
assert "hi".encode("utf-16").decode("utf-16") == "hi"
assert "hi".encode("utf-32").decode("utf-32") == "hi"
assert "hi".encode("utf-32-le") == b"h\x00\x00\x00i\x00\x00\x00"
assert b"h\x00\x00\x00i\x00\x00\x00".decode("utf-32-le") == "hi"

assert "café".encode("unicode-escape") == b"caf\\xe9"
assert b"caf\\xe9".decode("unicode-escape") == "café"
assert "A\u0100".encode("raw-unicode-escape") == b"A\\u0100"
assert b"A\\u0100".decode("raw-unicode-escape") == "A\u0100"

assert "+".encode("utf-7") == b"+-"
assert b"+-".decode("utf-7") == "+"

import _codecs

assert _codecs.escape_encode(b"a\tb") == (b"a\\tb", 3)
assert _codecs.escape_decode(rb"a\tb") == (b"a\tb", 4)
assert _codecs.utf_16_ex_decode(b"\xff\xfeh\x00i\x00", "strict", 0, True)[0] == "hi"
assert (
    _codecs.utf_32_ex_decode(b"\xff\xfe\x00\x00h\x00\x00\x00", "strict", 0, True)[0]
    == "h"
)


# Lookup errors preserve the requested spelling, while search hooks receive
# the Python 3.15 normalized registry key.
import codecs

seen_encoding_names = []


def record_unknown_encoding(name):
    seen_encoding_names.append(name)
    return None


codecs.register(record_unknown_encoding)
try:
    for requested, normalized in (
        ("NO Such  Codec!", "no-such--codec!"),
        ("x_ weird--Ω .Ab", "x_-weird--Ω-.ab"),
    ):
        for lookup in (
            codecs.lookup,
            lambda name: codecs.encode("value", name),
            lambda name: codecs.decode(b"value", name),
            lambda name: "value".encode(name),
            lambda name: b"value".decode(name),
        ):
            try:
                lookup(requested)
            except LookupError as error:
                assert str(error) == "unknown encoding: " + requested
            else:
                assert False, "unknown encoding was accepted"
            assert seen_encoding_names[-1] == normalized
finally:
    codecs.unregister(record_unknown_encoding)


# The encodings helper preserves case and dots, unlike registry lookup keys.
import encodings
import warnings


class EncodingName(str):
    pass


for encoding, expected in (
    ("", ""),
    ("UTF-8", "UTF_8"),
    ("  .UTF--8.  ", ".UTF_8."),
    ("---", ""),
    ("A__-- B", "A_B"),
    ("AéB", "A_B"),
    ("éUTF-8Ω", "UTF_8"),
    ("A😀B", "A_B"),
    ("UTF-8\x00ignored", "UTF_8"),
    ("\x00UTF-8", ""),
):
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        assert _codecs._normalize_encoding(encoding) == expected
        assert _codecs._normalize_encoding(encoding=EncodingName(encoding)) == expected
        assert not caught

for encoding in (b"UTF-8", bytearray(b"UTF-8"), None, 1):
    try:
        _codecs._normalize_encoding(encoding)
    except TypeError:
        pass
    else:
        assert False, "non-string encoding was accepted"

# UTF-8 validation precedes scanning, including a surrogate after NUL.
for encoding in ("A\ud800B", "A\x00\ud800B"):
    try:
        _codecs._normalize_encoding(encoding)
    except UnicodeEncodeError as error:
        assert error.encoding == "utf-8"
        assert error.object == encoding
        assert error.start == encoding.index("\ud800")
        assert error.end == error.start + 1
    else:
        assert False, "surrogate encoding was accepted"

with warnings.catch_warnings(record=True) as caught:
    warnings.simplefilter("always")
    assert encodings.normalize_encoding(b"UTF-8") == "UTF_8"
    assert not caught
    assert encodings.normalize_encoding("AéB") == "A_B"
    assert len(caught) == 1
    assert caught[0].category is DeprecationWarning

try:
    encodings.normalize_encoding(b"UTF-\xff")
except UnicodeDecodeError:
    pass
else:
    assert False, "non-ASCII bytes encoding was accepted"

for args, kwargs in (
    ((), {}),
    (("UTF-8", "ASCII"), {}),
    (("UTF-8",), {"encoding": "ASCII"}),
    ((), {"name": "UTF-8"}),
):
    try:
        _codecs._normalize_encoding(*args, **kwargs)
    except TypeError:
        pass
    else:
        assert False, "invalid normalization arguments were accepted"
