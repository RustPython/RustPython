import _interpchannels
import _interpreters


def roundtrip_buffer(view):
    cid = _interpchannels.create(3)
    try:
        _interpchannels.send_buffer(cid, view, blocking=False)
        received, unbound = _interpchannels.recv(cid)
        assert unbound is None
        return received
    finally:
        _interpchannels.destroy(cid)


data = bytearray(range(12))
for view in (memoryview(data)[2:10:2], memoryview(data).cast("I")):
    received = roundtrip_buffer(view)
    assert received.tolist() == view.tolist()
    assert received.shape == view.shape
    assert received.strides == view.strides
    assert received.format == view.format
    assert received.itemsize == view.itemsize
