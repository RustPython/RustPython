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
