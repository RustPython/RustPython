import binascii
from testutils import assertRaises


# hexlify tests
h = binascii.hexlify

assert h(b"abc") == b"616263"
assert h(1000 * b"x") == 1000 * b"78"
# bytearray not supported yet
# assert h(bytearray(b"a")) = b"61"
assert binascii.b2a_hex(b"aa") == b"6161"

with assertRaises(TypeError):
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

with assertRaises(ValueError):
    uh(b"a")  # Odd-length string

with assertRaises(ValueError):
    uh(b"nn")  # Non-hexadecimal digit found
