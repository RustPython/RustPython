
from testutils import assert_raises
import struct

data = struct.pack('IH', 14, 12)
assert data == bytes([14, 0, 0, 0, 12, 0])

v1, v2 = struct.unpack('IH', data)
assert v1 == 14
assert v2 == 12

data = struct.pack('<IH', 14, 12)
assert data == bytes([14, 0, 0, 0, 12, 0])

v1, v2 = struct.unpack('<IH', data)
assert v1 == 14
assert v2 == 12

data = struct.pack('>IH', 14, 12)
assert data == bytes([0, 0, 0, 14, 0, 12])

v1, v2 = struct.unpack('>IH', data)
assert v1 == 14
assert v2 == 12

data = struct.pack('3B', 65, 66, 67)
assert data == bytes([65, 66, 67])

v1, v2, v3 = struct.unpack('3B', data)
assert v1 == 65
assert v2 == 66
assert v3 == 67

with assert_raises(Exception):
  data = struct.pack('B0B', 65, 66)

with assert_raises(Exception):
  data = struct.pack('B2B', 65, 66)

data = struct.pack('B1B', 65, 66)

with assert_raises(Exception):
  struct.pack('<IH', "14", 12)
