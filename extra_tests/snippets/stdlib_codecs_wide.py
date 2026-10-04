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
