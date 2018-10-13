
import struct

data = struct.pack('IH', 14, 12)
assert data == bytes([14,0,0,0,12,0])

