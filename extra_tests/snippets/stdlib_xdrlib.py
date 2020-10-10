# This probably will be superceeded by the python unittests when that works.

import xdrlib

p = xdrlib.Packer()
p.pack_int(1337)

d = p.get_buffer()

print(d)

# assert d == b'\x00\x00\x059'
