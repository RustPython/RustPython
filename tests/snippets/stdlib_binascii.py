import binascii
from testutils import assert_raises


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

# unhexlify on strings not supported yet
# assert uh("abcd") == b"\xab\xcd"

with assert_raises(ValueError):
    uh(b"a")  # Odd-length string

with assert_raises(ValueError):
    uh(b"nn")  # Non-hexadecimal digit found

assert binascii.crc32(b"hello world") == 222957957
assert binascii.crc32(b"hello world", 555555) == 1216827162
assert binascii.crc32(b"goodbye interesting world",777777) == 1885538403
