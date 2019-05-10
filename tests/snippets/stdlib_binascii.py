import binascii
from testutils import assertRaises


# hexlify tests
h = binascii.hexlify

assert h(b"abc") == b"616263"
assert h(1000 * b"x") == 1000 * b"78"
# bytearray not supported yet
# assert h(bytearray(b"a")) = b"61"

with assertRaises(TypeError):
    h("a")
