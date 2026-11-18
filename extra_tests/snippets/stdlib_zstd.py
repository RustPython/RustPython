from compression.zstd import ZstdCompressor, ZstdDecompressor

# basic roundtrip
payload = b"hello world" * 100
c = ZstdCompressor()
frame = c.compress(payload) + c.flush()
d = ZstdDecompressor()
out = d.decompress(frame)
assert out == payload
assert d.eof
assert not d.needs_input

# max_length=0 must not drop output bytes: probing with a zero cap has to
# leave every byte available to later drain calls (regression: the probe
# used to emit one byte per call into the void).
d = ZstdDecompressor()
out = d.decompress(frame, 0)
assert out == b""
assert not d.eof
assert not d.needs_input
rest = d.decompress(b"", len(payload))
assert rest == payload
assert d.eof

# two zero-cap probes in a row, then drain; still lossless
d = ZstdDecompressor()
assert d.decompress(frame, 0) == b""
assert d.decompress(b"", 0) == b""
assert d.decompress(b"", len(payload)) == payload
assert d.eof

# zero-cap probe on a truncated frame: no output, not at end
d = ZstdDecompressor()
out = d.decompress(frame[:-5], 0)
assert out == b""
assert not d.eof
assert not d.needs_input

# zero-cap probe on a skippable frame completes it
skippable = b"\x50\x2a\x4d\x18\x04\x00\x00\x00abcd"
d = ZstdDecompressor()
assert d.decompress(skippable, 0) == b""
assert d.eof
assert not d.needs_input
assert d.unused_data == b""

# skippable frame followed by trailing bytes lands in unused_data
d = ZstdDecompressor()
assert d.decompress(skippable + b"trail", 0) == b""
assert d.eof
assert d.unused_data == b"trail"
