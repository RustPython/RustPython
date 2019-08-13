import zlib
from testutils import assert_raises

# checksum functions
assert zlib.crc32(b"123") == 2286445522
assert zlib.crc32(b"123", 1) == 2307525093
assert zlib.crc32(b"123", 2) == 2345449404
assert zlib.crc32(b"123", 3) == 2316230027
assert zlib.crc32(b"123", 4) == 2403453710
assert zlib.crc32(b"123", 5) == 2390991161
assert zlib.crc32(b"123", 6) == 2361728864
assert zlib.crc32(b"123", -123) == 3515918521
assert zlib.crc32(b"123", -122) == 3554023136
assert zlib.crc32(b"123", -121) == 3524558039
assert zlib.crc32(b"123", -120) == 3645389802
assert zlib.crc32(b"123", -119) == 3632943581
assert zlib.crc32(b"123", -118) == 3670863748
assert zlib.crc32(b"123") == zlib.crc32(b"123", 0)

assert zlib.adler32(b"456") == 20906144
assert zlib.adler32(b"456", 1) == 20906144
assert zlib.adler32(b"456", 2) == 21102753
assert zlib.adler32(b"456", 3) == 21299362
assert zlib.adler32(b"456", 4) == 21495971
assert zlib.adler32(b"456", 5) == 21692580
assert zlib.adler32(b"456", 6) == 21889189
assert zlib.adler32(b"456", -123) == 393267
assert zlib.adler32(b"456", -122) == 589876
assert zlib.adler32(b"456", -121) == 786485
assert zlib.adler32(b"456", -120) == 983094
assert zlib.adler32(b"456", -119) == 1179703
assert zlib.adler32(b"456", -118) == 1376312
assert zlib.adler32(b"456") == zlib.adler32(b"456", 1)

# compression
lorem = bytes("Lorem ipsum dolor sit amet", "utf-8")

compressed_lorem_list = [
    b"x\x01\x01\x1a\x00\xe5\xffLorem ipsum dolor sit amet\x83\xd5\t\xc5",
    b"x\x01\xf3\xc9/J\xcdU\xc8,(.\xcdUH\xc9\xcf\xc9/R(\xce,QH\xccM-\x01\x00\x83\xd5\t\xc5",
    b"x^\xf3\xc9/J\xcdU\xc8,(.\xcdUH\xc9\xcf\xc9/R(\xce,QH\xccM-\x01\x00\x83\xd5\t\xc5",
    b"x^\xf3\xc9/J\xcdU\xc8,(.\xcdUH\xc9\xcf\xc9/R(\xce,QH\xccM-\x01\x00\x83\xd5\t\xc5",
    b"x^\xf3\xc9/J\xcdU\xc8,(.\xcdUH\xc9\xcf\xc9/R(\xce,QH\xccM-\x01\x00\x83\xd5\t\xc5",
    b"x^\xf3\xc9/J\xcdU\xc8,(.\xcdUH\xc9\xcf\xc9/R(\xce,QH\xccM-\x01\x00\x83\xd5\t\xc5",
    b"x\x9c\xf3\xc9/J\xcdU\xc8,(.\xcdUH\xc9\xcf\xc9/R(\xce,QH\xccM-\x01\x00\x83\xd5\t\xc5",
    b"x\xda\xf3\xc9/J\xcdU\xc8,(.\xcdUH\xc9\xcf\xc9/R(\xce,QH\xccM-\x01\x00\x83\xd5\t\xc5",
    b"x\xda\xf3\xc9/J\xcdU\xc8,(.\xcdUH\xc9\xcf\xc9/R(\xce,QH\xccM-\x01\x00\x83\xd5\t\xc5",
    b"x\xda\xf3\xc9/J\xcdU\xc8,(.\xcdUH\xc9\xcf\xc9/R(\xce,QH\xccM-\x01\x00\x83\xd5\t\xc5",
]

for level, text in enumerate(compressed_lorem_list):
    assert zlib.compress(lorem, level) == text

# default level
assert zlib.compress(lorem) == zlib.compress(lorem, -1) == zlib.compress(lorem, 6)

# decompression
for text in compressed_lorem_list:
    assert zlib.decompress(text) == lorem

assert_raises(zlib.error, lambda: zlib.compress(b"123", -40))
assert_raises(zlib.error, lambda: zlib.compress(b"123", 10))
