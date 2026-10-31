from testutils import assert_raises

try:
    b"   \xff".decode("ascii")
except UnicodeDecodeError as e:
    assert e.start == 3
    assert e.end == 4
else:
    assert False, "should have thrown UnicodeDecodeError"

assert_raises(UnicodeEncodeError, "¿como estás?".encode, "ascii")


def round_trip(s, encoding="utf-8"):
    encoded = s.encode(encoding)
    decoded = encoded.decode(encoding)
    assert s == decoded


round_trip("👺♦  𝐚Şđƒ  ☆☝")
round_trip("☢🐣  ᖇ𝓤𝕊тⓟ𝕐𝕥卄σ𝔫  ♬👣")
round_trip("💀👌  ק𝔂tℍⓞ𝓷 ３  🔥👤")

# Bytes should not assume an encoding for isupper/islower
assert "Æ".isupper()
assert not "Æ".encode().isupper()
assert "æ".islower()
assert not "æ".encode().islower()

# Invalid Unicode
assert not b"\x80\x80".islower()
assert not b"\x80\x80".isupper()
assert b"\x80cat\x80".islower()
assert b"\x80CAT\x80".isupper()

# A lone surrogate gets an escape of its own wherever it sits, and a high one
# followed by a low one stays two escapes rather than being folded into one.
assert "\ud800".encode("unicode_escape") == b"\\ud800"
assert "a\ud800".encode("unicode_escape") == b"a\\ud800"
assert "\ud800b".encode("unicode_escape") == b"\\ud800b"
assert "\ud800\ud800".encode("unicode_escape") == b"\\ud800\\ud800"
assert "\ud800\udc00".encode("unicode_escape") == b"\\ud800\\udc00"
assert "\U00010000".encode("unicode_escape") == b"\\U00010000"

# Common encoding spellings pick the same codec, with its canonical name in errors
for spelling in ["utf-8", "utf8", "UTF-8", "Utf-8", "utf_8"]:
    assert "é€".encode(spelling) == b"\xc3\xa9\xe2\x82\xac"
    with assert_raises(UnicodeDecodeError, _msg=spelling) as cm:
        b"\xff".decode(spelling)
    assert str(cm.exception).startswith("'utf-8' codec"), str(cm.exception)
for spelling in ["latin-1", "latin1", "iso-8859-1", "l1"]:
    assert b"\xe9".decode(spelling) == "é"
    with assert_raises(UnicodeEncodeError) as cm:
        "€".encode(spelling)
    assert str(cm.exception).startswith("'latin-1' codec"), str(cm.exception)
with assert_raises(UnicodeEncodeError) as cm:
    "é".encode("ascii")
assert str(cm.exception).startswith("'ascii' codec"), str(cm.exception)
