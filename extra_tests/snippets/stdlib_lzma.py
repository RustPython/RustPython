import itertools
import lzma

from testutils import assert_raises

# A raw-format compressor needs the filter chain's length before it can build
# it, so a filter argument that is not a sequence has to be rejected instead of
# being drained.
with assert_raises(TypeError):
    lzma.LZMACompressor(
        format=lzma.FORMAT_RAW,
        filters=({"id": lzma.FILTER_LZMA2} for _ in itertools.count()),
    )

# Length is checked before any specifier is parsed: five invalid ids report the
# chain length, not the id.
with assert_raises(ValueError) as raised:
    lzma.LZMACompressor(format=lzma.FORMAT_RAW, filters=[{"id": 999}] * 5)
assert type(raised.exception) is ValueError
assert str(raised.exception) == "Too many filters - liblzma supports a maximum of 4"

compressor = lzma.LZMACompressor(
    format=lzma.FORMAT_RAW, filters=[{"id": lzma.FILTER_LZMA2}]
)
compressed = compressor.compress(b"data") + compressor.flush()
decompressor = lzma.LZMADecompressor(
    format=lzma.FORMAT_RAW, filters=[{"id": lzma.FILTER_LZMA2}]
)
assert decompressor.decompress(compressed) == b"data"
