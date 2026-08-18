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

compressor = lzma.LZMACompressor(
    format=lzma.FORMAT_RAW, filters=[{"id": lzma.FILTER_LZMA2}]
)
compressed = compressor.compress(b"data") + compressor.flush()
decompressor = lzma.LZMADecompressor(
    format=lzma.FORMAT_RAW, filters=[{"id": lzma.FILTER_LZMA2}]
)
assert decompressor.decompress(compressed) == b"data"
