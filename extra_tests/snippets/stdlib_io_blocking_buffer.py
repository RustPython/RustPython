"""Transfers that wait for a peer must not hold the buffer they were given.

A pipe or a socket answers when the other end does, which may be never. The
buffer is exported for the whole call, so it cannot be resized meanwhile, but
nothing else about it changes: another thread can still read it, write to it,
and the interpreter can still stop the world. An implementation that holds the
buffer's storage for the duration of the wait takes all of that away, and a
thread parked on that storage never reaches a safepoint, so a collection that
wants every thread stopped ends up waiting for the peer too.
"""

import gc
import os
import socket
import threading
import time

# The peer acts after DELAY; the checks below have to finish well inside it.
DELAY = 1.0
SLACK = DELAY / 2


def measure(buf, writable):
    """Time the operations on `buf` that do not need the peer."""
    start = time.monotonic()
    assert len(buf) == len(buf), len(buf)
    assert isinstance(bytes(buf), bytes)
    if writable:
        buf[0] = buf[0]
    gc.collect()
    return time.monotonic() - start


def run(buf, blocking_call, release_peer, writable):
    started = threading.Event()
    result = []

    def transfer():
        started.set()
        result.append(blocking_call(buf))

    def peer():
        time.sleep(DELAY)
        release_peer()

    threads = [threading.Thread(target=transfer), threading.Thread(target=peer)]
    for t in threads:
        t.start()
    started.wait()
    time.sleep(0.2)  # the transfer is now waiting on its peer

    elapsed = measure(buf, writable)
    assert elapsed < SLACK, "waited %.2fs on the peer" % elapsed

    # The export is still held either way, so the buffer cannot be resized.
    try:
        buf.append(0)
    except BufferError:
        pass
    else:
        raise AssertionError("append during an export should raise BufferError")

    for t in threads:
        t.join()
    return result[0]


# --- reading: the buffer is written into, so nothing else may touch it at all


read_fd, write_fd = os.pipe()
pipe = open(read_fd, "rb", buffering=0)
try:
    target = bytearray(16)
    n = run(target, pipe.readinto, lambda: os.write(write_fd, b"pipe"), writable=False)
    assert n == 4, n
    assert bytes(target[:4]) == b"pipe", bytes(target)
finally:
    pipe.close()
    os.close(write_fd)

if hasattr(socket, "socketpair"):
    left, right = socket.socketpair()
    try:
        target = bytearray(16)
        n = run(target, left.recv_into, lambda: right.send(b"socket"), writable=False)
        assert n == 6, n
        assert bytes(target[:6]) == b"socket", bytes(target)
    finally:
        left.close()
        right.close()


# --- writing: the buffer is only read, so it stays writable meanwhile


read_fd, write_fd = os.pipe()
sink = open(write_fd, "wb", buffering=0)
try:
    # More than any pipe will hold, so the write cannot finish on its own.
    source = bytearray(4 * 1024 * 1024)
    drained = []

    def drain():
        with open(read_fd, "rb", buffering=0) as f:
            while True:
                chunk = f.read(1 << 16)
                if not chunk:
                    break
                drained.append(len(chunk))

    reader = threading.Thread(target=drain, daemon=True)
    run(source, sink.write, reader.start, writable=True)
    sink.close()
    reader.join()
    assert sum(drained) == len(source), (sum(drained), len(source))
finally:
    if not sink.closed:
        sink.close()

if hasattr(socket, "socketpair"):
    left, right = socket.socketpair()
    try:
        left.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, 4096)
        source = bytearray(4 * 1024 * 1024)
        received = []

        def receive():
            while sum(received) < len(source):
                chunk = right.recv(1 << 16)
                if not chunk:
                    break
                received.append(len(chunk))

        reader = threading.Thread(target=receive, daemon=True)
        run(source, left.sendall, reader.start, writable=True)
        reader.join()
        assert sum(received) == len(source), (sum(received), len(source))
    finally:
        left.close()
        right.close()

print("ok")
