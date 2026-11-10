import binascii

from testutils import assert_equal, assert_raises

# hexlify tests
h = binascii.hexlify

assert h(b"abc") == b"616263"
assert h(1000 * b"x") == 1000 * b"78"
# bytearray not supported yet
# assert h(bytearray(b"a")) = b"61"
assert binascii.b2a_hex(b"aa") == b"6161"

with assert_raises(TypeError):
    h("a")


# unhexlify tests
uh = binascii.unhexlify

assert uh(b"616263") == b"abc"
assert uh(1000 * b"78") == 1000 * b"x"
x = 1000 * b"1234"
assert uh(h(x)) == x
assert uh(b"ABCDEF") == b"\xab\xcd\xef"
assert binascii.a2b_hex(b"6161") == b"aa"

assert uh("abcd") == b"\xab\xcd"

with assert_raises(ValueError):
    uh(b"a")  # Odd-length string

with assert_raises(ValueError):
    uh(b"nn")  # Non-hexadecimal digit found

assert binascii.crc32(b"hello world") == 222957957
assert binascii.crc32(b"hello world", 555555) == 1216827162
assert binascii.crc32(b"goodbye interesting world", 777777) == 1885538403

enc_b64 = binascii.b2a_base64

assert_equal(enc_b64(b"213"), b"MjEz\n")
assert_equal(enc_b64(b"213", newline=False), b"MjEz")

dec_b64 = binascii.a2b_base64

assert_equal(dec_b64(b"UnVzdFB5dGhvbg==\n"), b"RustPython")
assert_equal(
    dec_b64(b"4pii8J+QoyAg4ZaH8J2TpPCdlYrRguKTn/CdlZDwnZWl5Y2Ez4PwnZSrICDimazwn5Gj\n"),
    "☢🐣  ᖇ𝓤𝕊тⓟ𝕐𝕥卄σ𝔫  ♬👣".encode(),
)
assert_equal(dec_b64(b"3d=="), b"\xdd")

for exc, expected_name in [
    (binascii.Error, "Error"),
    (binascii.Incomplete, "Incomplete"),
]:
    assert exc.__module__ == "binascii"
    assert exc.__name__ == expected_name

# Module-bound functions retain their defining module when stored elsewhere.
# Both owned and borrowed Rust module receivers use the same Python binding.
for name, encoded, decoded in (
    ("a2b_base64", b"YQ==", b"a"),
    ("a2b_base32", b"ME======", b"a"),
    ("a2b_base85", b"VE", b"a"),
):
    function = getattr(binascii, name)
    assert function.__self__ is binascii
    assert function.__module__ == "binascii"
    assert function(encoded) == decoded
    assert function.__text_signature__.startswith("($module, data, /, *")

# A Python-visible module argument must not be mistaken for a bound receiver.
import _imp
import sys

assert _imp.exec_builtin(sys) == 0
with assert_raises(TypeError):
    _imp.exec_builtin()
