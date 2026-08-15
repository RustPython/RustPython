"""readinto() waits for a peer that may never answer.

The target buffer is exported for the whole call, so it cannot be resized
meanwhile, but everything else about it stays reachable: another thread can
read it, and the interpreter can still stop the world. A reader that holds the
buffer's storage for the duration of the wait takes all of that away, and a
thread parked on that storage never reaches a safepoint, so a collection that
wants every thread stopped waits for the peer too.
"""

import gc
import os
import socket
import threading
import time

# The peer answers after DELAY; the checks below have to finish well inside it.
DELAY = 1.5
SLACK = DELAY / 2


def check(start_read, feed_peer, result):
    buf = bytearray(16)
    got = []
    reading = threading.Event()

    def read():
        reading.set()
        got.append(start_read(buf))

    def feed():
        time.sleep(DELAY)
        feed_peer()

    reader = threading.Thread(target=read)
    feeder = threading.Thread(target=feed)
    reader.start()
    feeder.start()
    reading.wait()
    time.sleep(0.2)  # the reader is now waiting on its peer

    # None of this needs the peer, so none of it may wait for one.
    start = time.monotonic()
    assert len(buf) == 16, len(buf)
    assert isinstance(bytes(buf), bytes)
    gc.collect()
    elapsed = time.monotonic() - start
    assert elapsed < SLACK, "waited %.2fs on the peer" % elapsed

    # The export is still held, so the target still cannot be resized.
    try:
        buf.append(0)
    except BufferError:
        pass
    else:
        raise AssertionError("append during an export should raise BufferError")

    reader.join()
    feeder.join()
    assert got == [len(result)], got
    assert bytes(buf[: len(result)]) == result, bytes(buf)


# A pipe read goes through FileIO.readinto.
read_fd, write_fd = os.pipe()
pipe = open(read_fd, "rb", buffering=0)
try:
    check(pipe.readinto, lambda: os.write(write_fd, b"pipe"), b"pipe")
finally:
    pipe.close()
    os.close(write_fd)

# A socket read goes through socket.recv_into.
if hasattr(socket, "socketpair"):
    left, right = socket.socketpair()
    try:
        check(left.recv_into, lambda: right.send(b"socket"), b"socket")
    finally:
        left.close()
        right.close()

print("ok")
