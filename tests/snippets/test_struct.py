
import struct

data = struct.pack('IH', 14, 12)
assert data == bytes([14, 0, 0, 0, 12, 0])

v1, v2 = struct.unpack('IH', data)
assert v1 == 14
assert v2 == 12

