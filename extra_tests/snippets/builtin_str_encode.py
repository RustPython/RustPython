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
