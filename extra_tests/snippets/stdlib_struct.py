
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

assert struct.calcsize("B") == 1
# assert struct.calcsize("<L4B") == 12

assert struct.Struct('3B').pack(65, 66, 67) == bytes([65, 66, 67])

class Indexable(object):
    def __init__(self, value):
        self._value = value

    def __index__(self):
        return self._value

data = struct.pack('B', Indexable(65))
assert data == bytes([65])

data = struct.pack('5s', b"test1")
assert data == b"test1"

data = struct.pack('3s', b"test2")
assert data == b"tes"

data = struct.pack('7s', b"test3")
assert data == b"test3\0\0"

data = struct.pack('?', True)
assert data == b'\1'

data = struct.pack('?', [])
assert data == b'\0'