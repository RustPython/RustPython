import bz2


def test_compressor_returns_streaming_output():
    payload = bytes(range(256)) * 4096
    compressor = bz2.BZ2Compressor()
    partial = compressor.compress(payload)
    final = compressor.flush()

    assert partial
    assert bz2.decompress(partial + final) == payload


test_compressor_returns_streaming_output()
